//! Notification-area (tray) icon on its own thread with its own Win32 message loop, so it
//! keeps working while the main window is hidden (eframe stops running frames then).
//! Left-click shows Spindle; right-click opens a menu to launch builds, check for new
//! builds, open the library or quit. Balloon notifications announce new Blender builds.
//! A second Spindle finds this window by class name and asks it to show the first one.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const CLASS: &str = "SpindleTrayWindow";
const WM_TRAY: u32 = WM_APP + 1;
const WM_REFRESH: u32 = WM_APP + 2;
/// Posted by a second Spindle instance: show the main window.
pub const WM_SHOW_MAIN: u32 = WM_APP + 3;
/// Posted by `spindle_app --quit` (installers, updaters): exit cleanly.
pub const WM_QUIT_APP: u32 = WM_APP + 4;
const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;

const ID_SHOW: usize = 1;
const ID_DOWNLOADS: usize = 2;
const ID_CHECK: usize = 3;
const ID_LIBRARY: usize = 4;
const ID_QUIT: usize = 5;
const ID_BUILD0: usize = 100;

/// What the tray asks of the app.
#[derive(Debug, Clone)]
pub enum TrayEvent {
    /// The main window was shown from the tray.
    Shown,
    ShowDownloads,
    /// Launch a build (the app checks profile compatibility first).
    Launch(String),
    CheckUpdates,
    Quit,
}

/// Shared with the tray thread; changes are applied when `Tray::refresh` posts a message.
#[derive(Default)]
struct State {
    /// (build name, menu label), newest first.
    builds: Vec<(String, String)>,
    tooltip: String,
    balloon: Option<(String, String)>,
    /// Where the last balloon should lead when clicked.
    balloon_downloads: bool,
}

struct Ctx {
    main: HWND,
    egui: egui::Context,
    tx: Sender<TrayEvent>,
    state: Arc<Mutex<State>>,
    taskbar_created: u32,
}

thread_local! {
    static CTX: std::cell::RefCell<Option<Ctx>> = const { std::cell::RefCell::new(None) };
}

/// Handle to the running tray icon (cheap to clone, usable from any thread).
#[derive(Clone)]
pub struct Tray {
    hwnd: isize,
    state: Arc<Mutex<State>>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_into(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

impl Tray {
    /// Start the tray thread. `main` is the app window's HWND.
    pub fn start(main: isize, egui: egui::Context, tx: Sender<TrayEvent>) -> Option<Tray> {
        let state = Arc::new(Mutex::new(State { tooltip: "Spindle".into(), ..Default::default() }));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let st = state.clone();
        std::thread::Builder::new()
            .name("spindle-tray".into())
            .spawn(move || unsafe { run(main as HWND, egui, tx, st, ready_tx) })
            .ok()?;
        let hwnd = ready_rx.recv().ok()?;
        (hwnd != 0).then_some(Tray { hwnd, state })
    }

    fn post(&self, msg: u32) {
        // SAFETY: posting to our own window; harmless if it's gone.
        unsafe { PostMessageW(self.hwnd as HWND, msg, 0, 0) };
    }

    pub fn set_builds(&self, builds: Vec<(String, String)>) {
        if let Ok(mut s) = self.state.lock() {
            s.builds = builds;
        }
    }

    pub fn set_tooltip(&self, tip: &str) {
        let changed = self.state.lock().map(|mut s| {
            let c = s.tooltip != tip;
            s.tooltip = tip.to_owned();
            c
        });
        if changed.unwrap_or(false) {
            self.post(WM_REFRESH);
        }
    }

    /// Show a balloon notification. `downloads`: clicking it opens the Downloads view.
    pub fn notify(&self, title: &str, body: &str, downloads: bool) {
        if let Ok(mut s) = self.state.lock() {
            s.balloon = Some((title.to_owned(), body.to_owned()));
            s.balloon_downloads = downloads;
        }
        self.post(WM_REFRESH);
    }

    /// Remove the icon and end the tray thread. Waits until the icon is gone, so no
    /// ghost icon is left behind when the process exits right after.
    pub fn shutdown(&self) {
        // SAFETY: SendMessageW to our own window; returns once WM_CLOSE was handled.
        unsafe { SendMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
    }
}

/// Bring the main window back (also restores it if minimised).
pub fn show_main(main: HWND) {
    // SAFETY: plain window calls on the app's own window handle.
    unsafe {
        ShowWindow(main, SW_SHOW);
        if IsIconic(main) != 0 {
            ShowWindow(main, SW_RESTORE);
        }
        SetForegroundWindow(main);
    }
}

pub fn hide_main(main: isize) {
    // SAFETY: as above.
    unsafe { ShowWindow(main as HWND, SW_HIDE) };
}

/// If another Spindle is running, post it `msg` (WM_SHOW_MAIN or WM_QUIT_APP) and return true.
pub fn signal_existing(msg: u32) -> bool {
    let class = wide(CLASS);
    // SAFETY: FindWindowW with a NUL-terminated class name; PostMessageW to whatever it found.
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
        if hwnd.is_null() {
            return false;
        }
        PostMessageW(hwnd, msg, 0, 0);
        true
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    // SAFETY: zeroed is a valid NOTIFYICONDATAW (all integers, pointers and arrays).
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

unsafe fn add_icon(hwnd: HWND, tooltip: &str) {
    let mut nid = icon_data(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON);
        // Icon resource 1 is the program icon that build.rs embeds.
        nid.hIcon = LoadImageW(GetModuleHandleW(std::ptr::null()), 1 as _, IMAGE_ICON, size, size, 0) as HICON;
        if nid.hIcon.is_null() {
            nid.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
        }
        copy_into(&mut nid.szTip, tooltip);
        Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

unsafe fn run(main: HWND, egui: egui::Context, tx: Sender<TrayEvent>, state: Arc<Mutex<State>>, ready: Sender<isize>) {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        // A hidden top-level window (not message-only) so it receives TaskbarCreated.
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());
        if hwnd.is_null() {
            let _ = ready.send(0);
            return;
        }
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let tip = state.lock().map(|s| s.tooltip.clone()).unwrap_or_default();
        CTX.with(|c| *c.borrow_mut() = Some(Ctx { main, egui, tx, state, taskbar_created }));
        add_icon(hwnd, &tip);
        let _ = ready.send(hwnd as isize);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn with_ctx<R>(f: impl FnOnce(&Ctx) -> R) -> Option<R> {
    CTX.with(|c| c.borrow().as_ref().map(f))
}

fn send(ev: TrayEvent) {
    with_ctx(|c| {
        let _ = c.tx.send(ev);
        c.egui.request_repaint();
    });
}

fn show() {
    with_ctx(|c| show_main(c.main));
    send(TrayEvent::Shown);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if with_ctx(|c| c.taskbar_created == msg && msg != 0).unwrap_or(false) {
            // Explorer restarted: add the icon again.
            let tip = with_ctx(|c| c.state.lock().map(|s| s.tooltip.clone()).unwrap_or_default()).unwrap_or_default();
            add_icon(hwnd, &tip);
            return 0;
        }
        match msg {
            WM_TRAY => {
                match lparam as u32 {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => show(),
                    WM_RBUTTONUP | WM_CONTEXTMENU => menu(hwnd),
                    NIN_BALLOONUSERCLICK => {
                        let downloads = with_ctx(|c| c.state.lock().map(|s| s.balloon_downloads).unwrap_or(false)).unwrap_or(false);
                        show();
                        if downloads {
                            send(TrayEvent::ShowDownloads);
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_SHOW_MAIN => {
                show();
                0
            }
            WM_QUIT_APP => {
                // Frames only run while the window is visible, so show it to let the app close.
                show();
                send(TrayEvent::Quit);
                0
            }
            WM_REFRESH => {
                let (tip, balloon) = with_ctx(|c| {
                    c.state.lock().map(|mut s| (s.tooltip.clone(), s.balloon.take())).unwrap_or_default()
                })
                .unwrap_or_default();
                let mut nid = icon_data(hwnd);
                nid.uFlags = NIF_TIP;
                copy_into(&mut nid.szTip, &tip);
                if let Some((title, body)) = balloon {
                    nid.uFlags |= NIF_INFO;
                    nid.dwInfoFlags = NIIF_INFO;
                    copy_into(&mut nid.szInfoTitle, &title);
                    copy_into(&mut nid.szInfo, &body);
                }
                Shell_NotifyIconW(NIM_MODIFY, &nid);
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn menu(hwnd: HWND) {
    let builds = with_ctx(|c| c.state.lock().map(|s| s.builds.clone()).unwrap_or_default()).unwrap_or_default();
    unsafe {
        let m = CreatePopupMenu();
        let add = |id: usize, text: &str, flags: u32| {
            AppendMenuW(m, MF_STRING | flags, id, wide(text).as_ptr());
        };
        add(ID_SHOW, "Show Spindle", 0);
        SetMenuDefaultItem(m, ID_SHOW as u32, 0);
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        if builds.is_empty() {
            add(0, "No builds in the library", MF_GRAYED);
        }
        for (i, (_, label)) in builds.iter().take(10).enumerate() {
            add(ID_BUILD0 + i, &format!("Launch {label}"), 0);
        }
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        add(ID_DOWNLOADS, "Download Builds…", 0);
        add(ID_CHECK, "Check for New Builds", 0);
        add(ID_LIBRARY, "Open Library Folder", 0);
        AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
        add(ID_QUIT, "Quit Spindle", 0);

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Required so the menu closes when the user clicks elsewhere.
        SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, std::ptr::null()) as usize;
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);

        match id {
            ID_SHOW => show(),
            ID_DOWNLOADS => {
                show();
                send(TrayEvent::ShowDownloads);
            }
            ID_CHECK => send(TrayEvent::CheckUpdates),
            ID_LIBRARY => {
                if let Ok(sp) = spindle_core::Spindle::load() {
                    crate::platform::open_folder(&sp.library);
                }
            }
            ID_QUIT => {
                // The app runs frames only while visible: show it so it can shut down cleanly.
                show();
                send(TrayEvent::Quit);
            }
            id if id >= ID_BUILD0 => {
                if let Some((name, _)) = builds.get(id - ID_BUILD0) {
                    launch_from_tray(hwnd, name);
                }
            }
            _ => {}
        }
    }
}

/// Launch straight from the tray when the profile is compatible; otherwise bring the app
/// up so it can show the compatibility warning.
fn launch_from_tray(hwnd: HWND, name: &str) {
    let result = (|| -> Result<Option<String>, String> {
        let err = |e: anyhow::Error| format!("{e:#}");
        let sp = spindle_core::Spindle::load().map_err(err)?;
        let build = spindle_core::builds::find(&sp, name).map_err(err)?;
        let profile = spindle_core::profile::Profile::open(&sp, &crate::app::launch_profile(&sp)).map_err(err)?;
        if spindle_core::compat::check(&build, &profile.name, &profile.root).needs_attention() {
            return Ok(None);
        }
        spindle_core::launch::launch(&sp, &build, &profile, &[]).map_err(err)?;
        Ok(Some(format!("Blender {} with profile '{}'", build.version.unwrap_or(build.name), profile.name)))
    })();
    let balloon = |title: &str, body: String| {
        with_ctx(|c| {
            if let Ok(mut s) = c.state.lock() {
                s.balloon = Some((title.to_owned(), body));
                s.balloon_downloads = false;
            }
        });
        // SAFETY: posting to our own window.
        unsafe { PostMessageW(hwnd, WM_REFRESH, 0, 0) };
    };
    match result {
        Ok(Some(what)) => balloon("Launched", what),
        Ok(None) => {
            show();
            send(TrayEvent::Launch(name.to_owned()));
        }
        Err(e) => balloon("Couldn't launch", e),
    }
}

/// Wait (up to `secs`) until no other Spindle's tray window exists, e.g. the one that
/// just installed an update and is quitting.
pub fn wait_for_exit(secs: u64) {
    let class = wide(CLASS);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    // SAFETY: FindWindowW with a NUL-terminated class name.
    while unsafe { !FindWindowW(class.as_ptr(), std::ptr::null()).is_null() } && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

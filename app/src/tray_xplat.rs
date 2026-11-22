//! Tray icon on Linux (StatusNotifierItem over D-Bus, through tray-icon's KSNI backend)
//! and macOS (menu bar extra). Same API as the Windows tray in `tray.rs`.
//!
//! tray-icon objects belong to the main thread on macOS, so the icon lives in a
//! thread-local there; `Tray` handles only queue changes, which the app applies each frame
//! (`pump`). Menu clicks arrive on tray-icon's handler thread and are acted on directly
//! where they don't need the window (launching a build), else forwarded to the app.
//! These platforms have no tray balloons, so `notify` only updates the tooltip.

use std::cell::RefCell;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

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

#[derive(Default)]
struct Pending {
    builds: Option<Vec<(String, String)>>,
    tooltip: Option<String>,
}

/// Handle to the tray icon (cheap to clone, usable from any thread).
#[derive(Clone)]
pub struct Tray {
    pending: Arc<Mutex<Pending>>,
    egui: egui::Context,
}

thread_local! {
    static ICON: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
}

const SHOW: &str = "show";
const DOWNLOADS: &str = "downloads";
const CHECK: &str = "check";
const LIBRARY: &str = "library";
const QUIT: &str = "quit";
const BUILD: &str = "build:";

fn menu(builds: &[(String, String)]) -> Menu {
    let m = Menu::new();
    let _ = m.append(&MenuItem::with_id(SHOW, "Show Spindle", true, None));
    let _ = m.append(&PredefinedMenuItem::separator());
    if builds.is_empty() {
        let _ = m.append(&MenuItem::new("No builds yet", false, None));
    }
    for (name, label) in builds.iter().take(10) {
        let _ = m.append(&MenuItem::with_id(format!("{BUILD}{name}"), format!("Launch {label}"), true, None));
    }
    let _ = m.append(&PredefinedMenuItem::separator());
    let _ = m.append(&MenuItem::with_id(DOWNLOADS, "Download Builds…", true, None));
    let _ = m.append(&MenuItem::with_id(CHECK, "Check for New Builds", true, None));
    let _ = m.append(&MenuItem::with_id(LIBRARY, "Open Library Folder", true, None));
    let _ = m.append(&PredefinedMenuItem::separator());
    let _ = m.append(&MenuItem::with_id(QUIT, "Quit Spindle", true, None));
    m
}

fn show(egui: &egui::Context) {
    egui.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    egui.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    egui.send_viewport_cmd(egui::ViewportCommand::Focus);
    egui.request_repaint();
}

impl Tray {
    /// Create the icon (on the main thread). `_main` is unused here (Windows passes its HWND).
    pub fn start(_main: isize, egui: egui::Context, tx: Sender<TrayEvent>) -> Option<Tray> {
        let n = 64u32;
        let icon = Icon::from_rgba(crate::logo::rasterize(n), n, n).ok()?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("Spindle")
            .with_menu(Box::new(menu(&[])))
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .build()
            .map_err(|e| eprintln!("tray icon unavailable: {e}"))
            .ok()?;
        ICON.with(|i| *i.borrow_mut() = Some(tray));

        let (ctx, sender) = (egui.clone(), tx.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let id = e.id.0.as_str();
            let send = |ev: TrayEvent| {
                let _ = sender.send(ev);
                ctx.request_repaint();
            };
            match id {
                SHOW => {
                    show(&ctx);
                    send(TrayEvent::Shown);
                }
                DOWNLOADS => {
                    show(&ctx);
                    send(TrayEvent::ShowDownloads);
                }
                CHECK => send(TrayEvent::CheckUpdates),
                LIBRARY => {
                    if let Ok(sp) = spindle_core::Spindle::load() {
                        crate::platform::open_folder(&sp.library);
                    }
                }
                QUIT => {
                    send(TrayEvent::Quit);
                    // The app may not be drawing frames while minimised; don't hang around.
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_secs(3));
                        std::process::exit(0);
                    });
                }
                _ => {
                    if let Some(name) = id.strip_prefix(BUILD) {
                        if !crate::app::launch_quietly(name) {
                            show(&ctx);
                            send(TrayEvent::Launch(name.to_owned()));
                        }
                    }
                }
            }
        }));
        let (ctx, sender) = (egui.clone(), tx);
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, .. } = e {
                if !cfg!(target_os = "macos") {
                    show(&ctx);
                    let _ = sender.send(TrayEvent::Shown);
                }
            }
        }));
        Some(Tray { pending: Arc::new(Mutex::new(Pending::default())), egui })
    }

    pub fn set_builds(&self, builds: Vec<(String, String)>) {
        if let Ok(mut p) = self.pending.lock() {
            p.builds = Some(builds);
        }
        self.egui.request_repaint();
    }

    pub fn set_tooltip(&self, tip: &str) {
        if let Ok(mut p) = self.pending.lock() {
            p.tooltip = Some(tip.to_owned());
        }
    }

    /// No balloons on these platforms: the title goes into the tooltip until the next update.
    pub fn notify(&self, title: &str, _body: &str, _downloads: bool) {
        self.set_tooltip(&format!("Spindle · {title}"));
        self.egui.request_repaint();
    }

    /// Apply queued changes (call from the main thread, once per frame).
    pub fn pump(&self) {
        let (builds, tooltip) = match self.pending.lock() {
            Ok(mut p) => (p.builds.take(), p.tooltip.take()),
            Err(_) => return,
        };
        ICON.with(|i| {
            if let Some(icon) = i.borrow().as_ref() {
                if let Some(b) = builds {
                    icon.set_menu(Some(Box::new(menu(&b))));
                }
                if let Some(t) = tooltip {
                    let _ = icon.set_tooltip(Some(t));
                }
            }
        });
    }

    pub fn shutdown(&self) {
        ICON.with(|i| i.borrow_mut().take());
    }
}

/// Minimise instead of hiding: a hidden window stops drawing frames, a minimised one can
/// still be brought back by the tray menu.
pub fn hide_main(egui: &egui::Context) {
    egui.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod icons;
mod logo;
mod open_file;
mod platform;
mod theme;
#[cfg(windows)]
mod tray;
#[cfg(not(windows))]
#[path = "tray_xplat.rs"]
mod tray;
mod ui_center;
mod ui_dialogs;
mod ui_panels;
mod ui_shell;
mod widgets;

use std::path::PathBuf;

/// Window and taskbar icon: the same spindle mark as the title bar.
fn app_icon() -> egui::IconData {
    let n = 256u32;
    egui::IconData { rgba: logo::rasterize(n), width: n, height: n }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let mut screenshot: Option<PathBuf> = None;
    // Per-run overrides (not saved), so checks can picture every view and theme.
    let mut view: Option<app::View> = None;
    let mut theme: Option<theme::ThemeMode> = None;
    // Start hidden in the notification area (for starting with Windows).
    let start_in_tray = args.iter().any(|a| a == "--tray");
    // Open the profile-compatibility dialog for a build (for screenshots).
    let mut show_compat: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        let next = args.get(i + 1).map(|s| s.to_lowercase());
        match (args[i].as_str(), next.as_deref()) {
            ("--screenshot", Some(_)) => screenshot = Some(PathBuf::from(&args[i + 1])),
            ("--view", Some(v)) => {
                view = match v {
                    "builds" => Some(app::View::Builds),
                    "running" => Some(app::View::Running),
                    "addons" => Some(app::View::Addons),
                    "profiles" => Some(app::View::Profiles),
                    "downloads" => Some(app::View::Downloads),
                    _ => None,
                }
            }
            ("--show-compat", Some(_)) => show_compat = Some(args[i + 1].clone()),
            ("--theme", Some(t)) => theme = if t == "light" { Some(theme::ThemeMode::Light) } else { Some(theme::ThemeMode::Dark) },
            _ => {
                i += 1;
                continue;
            }
        }
        i += 2;
    }

    // Opened with a .blend file (file manager, "Open with"): launch it and exit.
    if screenshot.is_none() {
        if let Some(file) = open_file::blend_arg(&args) {
            open_file::open(&file);
            return Ok(());
        }
    }

    // One Spindle at a time: a second start brings the running one forward.
    // `--quit` asks a running Spindle to exit (for installers and updaters).
    #[cfg(windows)]
    if args.iter().any(|a| a == "--quit") {
        tray::signal_existing(tray::WM_QUIT_APP);
        return Ok(());
    }
    // Started by an update that's still quitting: let it go first.
    #[cfg(windows)]
    if args.iter().any(|a| a == "--after-update") {
        tray::wait_for_exit(20);
    }
    #[cfg(windows)]
    if screenshot.is_none() && tray::signal_existing(tray::WM_SHOW_MAIN) {
        return Ok(());
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Spindle")
        .with_app_id("spindle")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([960.0, 600.0])
        .with_decorations(false)
        .with_icon(app_icon());
    if start_in_tray && screenshot.is_none() {
        viewport = viewport.with_visible(false);
    }
    if screenshot.is_some() {
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }

    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    #[cfg(target_os = "macos")]
    if screenshot.is_some() {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            b.with_activate_ignoring_other_apps(false);
        }));
    }

    eframe::run_native(
        "Spindle",
        options,
        Box::new(move |cc| {
            let mut a = app::App::new(cc, screenshot.clone());
            if let Some(v) = view {
                a.prefs.view = v;
                a.select_first_in_view();
            }
            if let Some(t) = theme {
                a.set_theme(&cc.egui_ctx, t);
                a.prefs_dirty = false;
            }
            if let Some(q) = &show_compat {
                if let Ok(b) = spindle_core::builds::find(&a.sp, q) {
                    if let Some(report) = a.compat.get(&b.name).cloned() {
                        a.selection = app::Selection::Build(b.name.clone());
                        a.dlg.compat = Some(app::CompatDialog { build: b.name, report });
                    }
                }
            }
            Ok(Box::new(a))
        }),
    )
}

impl eframe::App for app::App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frame_no += 1;

        self.poll(&ctx);
        // Closing (title bar, Alt+F4, taskbar) hides to the tray unless quitting.
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quitting
            && self.prefs.close_to_tray
            && self.screenshot.is_none()
            && self.hide_to_tray(&ctx)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        ui_shell::shortcuts(self, &ctx);
        ui_shell::title_bar(self, ui);
        ui_shell::context_bar(self, ui);
        ui_shell::status_bar(self, ui);
        if self.prefs.show_left {
            ui_panels::left_panel(self, ui);
        }
        if self.prefs.show_right {
            ui_panels::right_panel(self, ui);
        }
        let fill = self.pal.canvas;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(fill)).show(ui, |ui| ui_center::center(self, ui));
        ui_dialogs::dialogs(self, &ctx);
        ui_shell::resize_edges(&ctx);

        if self.prefs_dirty && self.screenshot.is_none() {
            self.prefs_dirty = false;
            self.save_prefs();
        }
        self.screenshot_step(&ctx);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if self.screenshot.is_none() {
            self.save_prefs();
        }
    }
}

impl app::App {
    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        ctx.request_repaint();
        if !self.shot_requested && self.frame_no >= 120 {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if !self.shot_requested {
            return;
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let [w, h] = img.size;
            let mut buf = Vec::with_capacity(w * h * 4);
            for c in &img.pixels {
                buf.extend_from_slice(&c.to_srgba_unmultiplied());
            }
            if let Some(rgba) = image::RgbaImage::from_raw(w as u32, h as u32, buf) {
                let _ = rgba.save(&path);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

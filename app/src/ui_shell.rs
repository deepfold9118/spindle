//! Window chrome: title bar with the menus, the context bar (search, channel chips,
//! active profile), the status bar, window resize edges and keyboard shortcuts.

use egui::{Align, Align2, Color32, CornerRadius, Frame, Key, Layout, Margin, Panel, Sense, Stroke, Vec2};

use crate::app::*;
use crate::icons as I;
use crate::platform;
use crate::theme::{bold, icon_font, ThemeMode};
use crate::widgets::*;

/// Draw the Spindle logo (a white mask texture from logo.rs) tinted with `tint`,
/// as a square of `rect`'s height centred in `rect`.
pub fn paint_mark(p: &egui::Painter, rect: egui::Rect, tint: Color32, tex: &egui::TextureHandle) {
    // Snap to device pixels so the exact-size texture maps 1:1.
    let ppp = p.ctx().pixels_per_point();
    let side = tex.size()[1] as f32 / ppp;
    let min = ((rect.center() - Vec2::splat(side / 2.0)) * ppp).round() / ppp;
    let sq = egui::Rect::from_min_size(min, Vec2::splat(side));
    p.image(tex.id(), sq, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), tint);
}

fn window_button(ui: &mut egui::Ui, pal: &crate::theme::Palette, icon: &str, close: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(40.0, 30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), if close { pal.red } else { pal.hover });
    }
    let col = if resp.hovered() && close { Color32::WHITE } else { pal.muted };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, icon_font(14.0), col);
    resp
}

fn item(ui: &mut egui::Ui, icon: &str, label: &str, shortcut: &str, enabled: bool) -> bool {
    let text = if icon.is_empty() { format!("      {label}") } else { format!("{icon}  {label}") };
    let mut b = egui::Button::new(text);
    if !shortcut.is_empty() {
        b = b.shortcut_text(shortcut);
    }
    let r = ui.add_enabled(enabled, b);
    if r.clicked() {
        ui.close();
        true
    } else {
        false
    }
}

pub fn title_bar(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    Panel::top("titlebar")
        .exact_size(36.0)
        .frame(Frame::new().fill(pal.title).inner_margin(Margin::symmetric(8, 0)))
        .show_separator_line(false)
        .show(ui, |ui| {
            let bar_rect = ui.max_rect();
            let bg = ui.interact(bar_rect, ui.id().with("title_drag"), Sense::click_and_drag());
            if bg.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if bg.double_clicked() {
                let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            ui.horizontal_centered(|ui| {
                ui.add_space(2.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(17.0, 20.0), Sense::hover());
                let tex = app.mark_texture(ui.ctx(), r.height());
                paint_mark(ui.painter(), r, pal.accent, &tex);
                ui.add_space(8.0);
                egui::MenuBar::new().ui(ui, |ui| menus(app, ui));
            });
            ui.painter().text(bar_rect.center(), Align2::CENTER_CENTER, "Spindle", bold(12.5), pal.muted);
            let right = egui::Rect::from_min_max(egui::pos2(bar_rect.right() - 230.0, bar_rect.top()), bar_rect.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if window_button(ui, &pal, I::X, true).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                if window_button(ui, &pal, if max { I::MINIMIZE_2 } else { I::SQUARE }, false).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                if window_button(ui, &pal, I::MINUS, false).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                ui.add_space(8.0);
                let (icon, tip, next) = if pal.dark { (I::SUN, "Switch to light theme", ThemeMode::Light) } else { (I::MOON, "Switch to dark theme", ThemeMode::Dark) };
                if icon_button(ui, &pal, icon, tip, false).clicked() {
                    let ctx = ui.ctx().clone();
                    app.set_theme(&ctx, next);
                }
                if icon_button(ui, &pal, I::REFRESH_CW, "Refresh library (F5)", false).clicked() {
                    app.refresh();
                }
            });
        });
}

fn menus(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    ui.menu_button("File", |ui| {
        if item(ui, I::PLUS, "Add Build…", "", true) {
            app.add_external_build(&ctx);
        }
        if item(ui, I::USER, "New Profile…", "", true) {
            app.dlg.new_profile = Some(String::new());
        }
        ui.separator();
        if item(ui, I::FOLDER_OPEN, "Open Library Folder", "", true) {
            platform::open_folder(&app.sp.library);
        }
        if item(ui, I::FOLDER_OPEN, "Open Profiles Folder", "", true) {
            platform::open_folder(&app.sp.profiles_dir());
        }
        ui.separator();
        if item(ui, I::X, "Exit", "", true) {
            app.quit(&ctx);
        }
    });
    ui.menu_button("View", |ui| {
        for (v, icon, key) in [
            (View::Builds, I::HARD_DRIVE, "Ctrl+1"),
            (View::Running, I::ACTIVITY, "Ctrl+2"),
            (View::Addons, I::PACKAGE, "Ctrl+3"),
            (View::Profiles, I::USER, "Ctrl+4"),
            (View::Downloads, I::DOWNLOAD, "Ctrl+5"),
        ] {
            if item(ui, icon, v.title(), key, true) {
                app.set_view(v);
            }
        }
        ui.separator();
        if item(ui, I::PANEL_LEFT, if app.prefs.show_left { "Hide Library Panel" } else { "Show Library Panel" }, "", true) {
            app.prefs.show_left = !app.prefs.show_left;
            app.prefs_dirty = true;
        }
        if item(ui, I::PANEL_RIGHT, if app.prefs.show_right { "Hide Task Panel" } else { "Show Task Panel" }, "", true) {
            app.prefs.show_right = !app.prefs.show_right;
            app.prefs_dirty = true;
        }
        ui.separator();
        let (icon, label, next) = if app.pal.dark { (I::SUN, "Light Theme", ThemeMode::Light) } else { (I::MOON, "Dark Theme", ThemeMode::Dark) };
        if item(ui, icon, label, "", true) {
            app.set_theme(&ctx, next);
        }
    });
    ui.menu_button("Library", |ui| {
        let newest = app.default_label();
        if item(ui, I::PLAY, &format!("Launch Default ({})", newest.as_deref().unwrap_or("none")), "Ctrl+L", newest.is_some()) {
            app.launch_default(&ctx);
        }
        if item(ui, I::REFRESH_CW, "Refresh", "F5", true) {
            app.refresh();
        }
        ui.separator();
        if item(ui, I::DOWNLOAD, "Download Builds…", "Ctrl+5", true) {
            app.set_view(View::Downloads);
        }
        if item(ui, I::REFRESH_CW, "Check for New Builds", "", !app.checking) {
            app.check_updates(&ctx, true);
        }
        ui.separator();
        if item(ui, I::GIT_COMPARE, "Add Add-on from GitHub…", "", true) {
            app.open_github(&ctx, None);
        }
        if item(ui, I::FILE_BOX, "Add Add-on from File…", "", !app.busy("Adding add-ons")) {
            app.pick_addon_files(&ctx);
        }
        if item(ui, I::DOWNLOAD, "Sync Superhive Purchases", "", !app.busy("Syncing Superhive")) {
            app.sync_superhive(&ctx);
        }
        if item(ui, I::SHIELD_CHECK, "Superhive Token…", "", true) {
            app.dlg.superhive_login = true;
        }
    });
    ui.menu_button("Agents", |ui| {
        if item(ui, I::COPY, "Copy MCP Client Config", "", true) {
            ctx.copy_text(app.mcp_config_snippet());
            app.toast(&ctx, "MCP client config copied: point your agent at spindle-mcp");
        }
        if item(ui, I::ACTIVITY, "Show Running Blenders", "", true) {
            app.set_view(View::Running);
        }
    });
    ui.menu_button("Help", |ui| {
        let label = match &app.spindle_release {
            Some(r) => format!("Update to Spindle {}…", r.version),
            None => "Spindle Updates…".to_string(),
        };
        if item(ui, I::ARROW_UP_CIRCLE, &label, "", true) {
            app.dlg.self_update = true;
            if app.spindle_release.is_none() {
                app.check_spindle_release();
            }
        }
        if item(ui, I::INFO, "About Spindle", "", true) {
            app.dlg.about = true;
        }
    });
}

pub fn context_bar(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    Panel::top("contextbar")
        .exact_size(40.0)
        .frame(Frame::new().fill(pal.panel).inner_margin(Margin::symmetric(10, 0)).stroke(Stroke::NONE))
        .show_separator_line(true)
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new(I::SEARCH).font(icon_font(14.0)).color(pal.muted));
                let hint = match app.prefs.view {
                    View::Builds => "Find builds…",
                    View::Running => "Find running Blenders…",
                    View::Addons => "Find add-ons…",
                    View::Profiles => "Find profiles…",
                    View::Downloads => "Find downloads (version, branch, PR)…",
                };
                ui.add(egui::TextEdit::singleline(&mut app.quick_find).hint_text(hint).desired_width(220.0));
                if !app.quick_find.is_empty() && icon_button(ui, &pal, I::X, "Clear", false).clicked() {
                    app.quick_find.clear();
                }
                vsep(ui, &pal);
                // Channel chips only filter builds.
                if matches!(app.prefs.view, View::Builds | View::Downloads) {
                    ui.label(egui::RichText::new("Show").color(pal.muted).size(12.0));
                    let before = (app.prefs.show_stable, app.prefs.show_lts, app.prefs.show_daily, app.prefs.show_experimental);
                    dot_chip(ui, &pal, pal.green, "Stable", &mut app.prefs.show_stable);
                    dot_chip(ui, &pal, pal.accent_text, "LTS", &mut app.prefs.show_lts);
                    dot_chip(ui, &pal, pal.blue, "Daily", &mut app.prefs.show_daily);
                    dot_chip(ui, &pal, pal.grey, "Experimental", &mut app.prefs.show_experimental);
                    if before != (app.prefs.show_stable, app.prefs.show_lts, app.prefs.show_daily, app.prefs.show_experimental) {
                        app.prefs_dirty = true;
                    }
                    vsep(ui, &pal);
                }
                let icon = match app.prefs.view {
                    View::Builds => I::HARD_DRIVE,
                    View::Running => I::ACTIVITY,
                    View::Addons => I::PACKAGE,
                    View::Profiles => I::USER,
                    View::Downloads => I::DOWNLOAD,
                };
                ui.label(egui::RichText::new(icon).font(icon_font(14.0)).color(pal.accent_text));
                ui.add(egui::Label::new(egui::RichText::new(app.prefs.view.title()).color(pal.text).strong()).truncate());

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if icon_button(ui, &pal, I::PANEL_RIGHT, "Task panel", app.prefs.show_right).clicked() {
                        app.prefs.show_right = !app.prefs.show_right;
                        app.prefs_dirty = true;
                    }
                    if icon_button(ui, &pal, I::PANEL_LEFT, "Library panel", app.prefs.show_left).clicked() {
                        app.prefs.show_left = !app.prefs.show_left;
                        app.prefs_dirty = true;
                    }
                    ui.label(egui::RichText::new("Workspace").color(pal.muted).size(12.0));
                    vsep(ui, &pal);
                    let profiles = app.profiles.clone();
                    egui::ComboBox::from_id_salt("active_profile")
                        .selected_text(format!("{}  {}", I::USER, app.active_profile))
                        .width(140.0)
                        .show_ui(ui, |ui| {
                            for p in &profiles {
                                ui.selectable_value(&mut app.active_profile, p.clone(), p);
                            }
                        })
                        .response
                        .on_hover_text("Profile used when launching builds");
                    ui.label(egui::RichText::new("Launch with").color(pal.muted).size(12.0));
                });
            });
        });
}

pub fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    Panel::bottom("statusbar").exact_size(26.0).frame(Frame::new().fill(pal.title).inner_margin(Margin::symmetric(10, 0))).show_separator_line(false).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let now = ui.ctx().input(|i| i.time);
            if let Some(job) = app.jobs.first() {
                ui.add(egui::Spinner::new().size(12.0).color(pal.accent_text));
                let pct = job.progress.as_ref().and_then(|p| p.fraction()).map(|f| format!(" {:.0}%", f * 100.0)).unwrap_or_default();
                let more = if app.jobs.len() > 1 { format!(" (+{} more)", app.jobs.len() - 1) } else { String::new() };
                ui.label(egui::RichText::new(format!("{}{pct}…{more}", job.label)).size(11.5).color(pal.text));
            }
            let msg = match &app.status {
                Some((s, until)) if *until > now => s.clone(),
                _ => match app.prefs.view {
                    View::Builds => "Double-click a build to launch it with the selected profile · Agents see running Blenders through spindle-mcp".to_string(),
                    View::Running => "Blenders started by Spindle report here every few seconds, with their MCP add-on port".to_string(),
                    View::Addons => "Add-ons downloaded into Spindle's library · drop a .zip here to add one · Install puts one into a profile, shared by every build".to_string(),
                    View::Profiles => "A profile holds preferences, add-ons and extensions shared by every build launched with it".to_string(),
                    View::Downloads => "Official releases come from download.blender.org; Daily and Experimental from builder.blender.org · every download is SHA-256 checked".to_string(),
                },
            };
            let col = if msg.starts_with("Error") { pal.red } else { pal.muted };
            ui.label(egui::RichText::new(I::INFO).font(icon_font(12.0)).color(col));
            ui.add(egui::Label::new(egui::RichText::new(msg).size(11.5).color(col)).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(r) = &app.spindle_release {
                    let chip = ui
                        .add(egui::Label::new(egui::RichText::new(format!("{}  Spindle {} available", I::ARROW_UP_CIRCLE, r.version)).size(11.5).color(pal.accent)).sense(Sense::click()))
                        .on_hover_text("Install the new Spindle");
                    if chip.clicked() {
                        app.dlg.self_update = true;
                    }
                    ui.add_space(12.0);
                }
                let running = app.instances.len();
                ui.label(
                    egui::RichText::new(format!(
                        "Builds: {}   Running: {}   Add-ons: {}   Profiles: {}",
                        app.builds.len(),
                        running,
                        app.latest_addons().len(),
                        app.profiles.len()
                    ))
                    .size(11.5)
                    .color(pal.text),
                );
                if running > 0 {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    dot(ui, r.center(), 3.5, pal.green);
                }
            });
        });
    });
}

pub fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let rect = ctx.content_rect();
    let m = 5.0;
    let l = pos.x - rect.left() < m;
    let r = rect.right() - pos.x < m;
    let t = pos.y - rect.top() < m;
    let b = rect.bottom() - pos.y < m;
    use egui::viewport::ResizeDirection as D;
    let dir = match (l, r, t, b) {
        (true, _, true, _) => Some(D::NorthWest),
        (_, true, true, _) => Some(D::NorthEast),
        (true, _, _, true) => Some(D::SouthWest),
        (_, true, _, true) => Some(D::SouthEast),
        (true, ..) => Some(D::West),
        (_, true, ..) => Some(D::East),
        (_, _, true, _) => Some(D::North),
        (.., true) => Some(D::South),
        _ => None,
    };
    if let Some(d) = dir {
        let icon = match d {
            D::North | D::South => egui::CursorIcon::ResizeVertical,
            D::East | D::West => egui::CursorIcon::ResizeHorizontal,
            D::NorthWest | D::SouthEast => egui::CursorIcon::ResizeNwSe,
            _ => egui::CursorIcon::ResizeNeSw,
        };
        ctx.set_cursor_icon(icon);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(d));
        }
    }
}

pub fn shortcuts(app: &mut App, ctx: &egui::Context) {
    if ctx.memory(|m| m.focused().is_some()) {
        return;
    }
    let (f5, ctrl, k1, k2, k3, k4, k5, l) = ctx.input(|i| {
        (
            i.key_pressed(Key::F5),
            i.modifiers.command,
            i.key_pressed(Key::Num1),
            i.key_pressed(Key::Num2),
            i.key_pressed(Key::Num3),
            i.key_pressed(Key::Num4),
            i.key_pressed(Key::Num5),
            i.key_pressed(Key::L),
        )
    });
    if f5 {
        app.refresh();
    }
    if ctrl {
        if k1 {
            app.set_view(View::Builds);
        }
        if k2 {
            app.set_view(View::Running);
        }
        if k3 {
            app.set_view(View::Addons);
        }
        if k4 {
            app.set_view(View::Profiles);
        }
        if k5 {
            app.set_view(View::Downloads);
        }
        if l {
            app.launch_default(ctx);
        }
    }
}

//! Modal dialogs: Superhive token, install add-on into a profile, new profile, about.

use crate::app::*;
use crate::icons as I;
use crate::theme::bold;
use crate::widgets::*;

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    superhive_login(app, ctx);
    install(app, ctx);
    new_profile(app, ctx);
    about(app, ctx);
    compat(app, ctx);
    remove(app, ctx);
    self_update(app, ctx);
}

/// Help > Spindle Updates: the newest release, and Install and Restart.
fn self_update(app: &mut App, ctx: &egui::Context) {
    if !app.dlg.self_update {
        return;
    }
    let pal = app.pal;
    let mut close = false;
    let mut install = false;
    let mut check = false;
    let checking = app.busy("Checking for Spindle updates");
    let updating = app.busy("Updating Spindle");
    let current = spindle_core::selfupdate::CURRENT;
    let latest = spindle_core::selfupdate::cached(&app.sp);
    let dir = spindle_core::selfupdate::install_dir();
    egui::Modal::new(egui::Id::new("m_selfupdate")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(egui::RichText::new("Spindle Updates").font(bold(15.0)));
        ui.add_space(6.0);
        kv_line(ui, &pal, "This version", current);
        match &latest {
            Some(r) => kv_line(ui, &pal, "Newest release", &r.version),
            None if checking => kv_line(ui, &pal, "Newest release", "checking…"),
            None => kv_line(ui, &pal, "Newest release", "not checked yet"),
        }
        ui.add_space(6.0);
        let newer = app.spindle_release.clone();
        if let Some(r) = &newer {
            if !r.notes.trim().is_empty() {
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    ui.label(egui::RichText::new(r.notes.trim()).size(12.0).color(pal.muted));
                });
                ui.add_space(6.0);
            }
            if let Err(e) = &dir {
                ui.label(egui::RichText::new(format!("{e:#}")).size(12.0).color(pal.orange));
            }
        } else if latest.is_some() {
            ui.label(egui::RichText::new(format!("{}  You have the newest Spindle.", I::CIRCLE_CHECK)).color(pal.green));
        }
        if updating {
            ui.label(egui::RichText::new("Downloading and installing…").size(12.0).color(pal.muted));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if let Some(r) = &newer {
                let can = dir.is_ok() && !updating;
                if ui.add_enabled(can, egui::Button::new(format!("{}  Install Spindle {} and Restart", I::ARROW_UP_CIRCLE, r.version))).clicked() {
                    install = true;
                }
                if ui.button("Release Notes").clicked() {
                    crate::platform::open_url(&r.url);
                }
            } else if ui.add_enabled(!checking, egui::Button::new(format!("{}  Check Now", I::REFRESH_CW))).clicked() {
                check = true;
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    });
    if install {
        app.install_spindle_update(ctx);
    }
    if check {
        app.check_spindle_release();
    }
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.self_update = false;
    }
}

fn kv_line(ui: &mut egui::Ui, pal: &crate::theme::Palette, k: &str, v: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([120.0, 18.0], egui::Label::new(egui::RichText::new(k).size(12.0).color(pal.muted)));
        ui.label(egui::RichText::new(v).size(12.0).color(pal.text));
    });
}

/// Profile compatibility report for a build, with Launch Anyway.
fn compat(app: &mut App, ctx: &egui::Context) {
    let Some(d) = app.dlg.compat.clone() else { return };
    let pal = app.pal;
    let mut launch = false;
    let mut close = false;
    let blocker = d.report.worst() == Some(spindle_core::compat::Severity::Blocker);
    egui::Modal::new(egui::Id::new("m_compat")).show(ctx, |ui| {
        ui.set_width(520.0);
        let series = d.report.build_series.clone().unwrap_or_else(|| "?".into());
        ui.label(egui::RichText::new(format!("Profile '{}' with Blender {series}", d.report.profile)).font(bold(15.0)));
        ui.add_space(2.0);
        ui.label(egui::RichText::new(&d.build).size(11.5).color(pal.faint));
        ui.add_space(8.0);
        if d.report.issues.is_empty() {
            ui.label(egui::RichText::new(format!("{}  No problems found", I::CIRCLE_CHECK)).color(pal.green));
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for i in &d.report.issues {
                let (icon, col) = match i.severity {
                    spindle_core::compat::Severity::Blocker => (I::CIRCLE_ALERT, pal.red),
                    spindle_core::compat::Severity::Warning => (I::TRIANGLE_ALERT, pal.orange),
                    spindle_core::compat::Severity::Info => (I::INFO, pal.muted),
                };
                egui::Frame::new().fill(pal.deep).corner_radius(6).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icon).font(crate::theme::icon_font(14.0)).color(col));
                        ui.label(egui::RichText::new(&i.title).font(bold(13.0)).color(pal.text));
                    });
                    ui.label(egui::RichText::new(&i.detail).size(12.0).color(pal.muted));
                });
                ui.add_space(6.0);
            }
        });
        if blocker {
            ui.label(egui::RichText::new("Tip: launch older builds with a separate profile (Launch with, in the toolbar) to keep your main profile safe.").size(11.5).color(pal.faint));
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            if icon_text_button(ui, &pal, I::X, "Cancel", blocker).clicked() {
                close = true;
            }
            let label = if blocker { "Launch Anyway" } else { "Launch" };
            if icon_text_button(ui, &pal, I::PLAY, label, !blocker).clicked() {
                launch = true;
            }
        });
    });
    if launch {
        app.dlg.compat = None;
        app.launch_build_now(ctx, &d.build);
    } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.compat = None;
    }
}

/// Confirm removing a build.
fn remove(app: &mut App, ctx: &egui::Context) {
    let Some(name) = app.dlg.remove.clone() else { return };
    let pal = app.pal;
    let Some(b) = app.builds.iter().find(|b| b.name == name).cloned() else {
        app.dlg.remove = None;
        return;
    };
    let running = app.running_of(&b).len();
    let mut go = false;
    let mut close = false;
    egui::Modal::new(egui::Id::new("m_remove")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.label(egui::RichText::new(format!("Remove Blender {}?", b.version.as_deref().unwrap_or(&b.name))).font(bold(15.0)));
        ui.add_space(4.0);
        let body = if b.external {
            format!("Spindle forgets this build. Its files stay where they are:\n{}", b.root().display())
        } else {
            format!("The build folder moves to the Recycle Bin, so you can restore it from there:\n{}", b.root().display())
        };
        ui.label(egui::RichText::new(body).size(12.0).color(pal.muted));
        ui.label(egui::RichText::new("Profiles, add-ons and preferences are not touched.").size(12.0).color(pal.muted));
        if running > 0 {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(format!("{running} Blender(s) from this build are running; close them first.")).size(12.0).color(pal.red));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let label = if b.external { "Remove from Spindle" } else { "Move to Recycle Bin" };
            if ui.add_enabled(running == 0, egui::Button::new(format!("{}  {label}", I::TRASH_2))).clicked() {
                go = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if go {
        app.dlg.remove = None;
        app.remove_build(ctx, &name);
    } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.remove = None;
    }
}

fn superhive_login(app: &mut App, ctx: &egui::Context) {
    if !app.dlg.superhive_login {
        return;
    }
    let pal = app.pal;
    let mut close = false;
    let mut save = false;
    egui::Modal::new(egui::Id::new("m_superhive")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(egui::RichText::new("Superhive Token").font(bold(15.0)));
        ui.add_space(4.0);
        let state = if app.token_stored { "A token is saved. Enter a new one to replace it." } else { "No token saved yet." };
        ui.label(egui::RichText::new(format!(
            "Spindle uses your Superhive API token to download purchased add-ons into its library. \
             It is kept in the system credential store and never written into Blender's preferences.\n\n{state}"
        )).size(12.0).color(pal.muted));
        ui.add_space(8.0);
        let r = ui.add(egui::TextEdit::singleline(&mut app.dlg.token_input).password(true).hint_text("API token").desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            save = true;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can = !app.dlg.token_input.trim().is_empty();
            if ui.add_enabled(can, egui::Button::new(format!("{}  Save Token", I::SHIELD_CHECK))).clicked() {
                save = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if save && !app.dlg.token_input.trim().is_empty() {
        app.save_token(ctx);
    } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.token_input.clear();
        app.dlg.superhive_login = false;
    }
}

fn install(app: &mut App, ctx: &egui::Context) {
    let Some((id, mut build, mut profile)) = app.dlg.install.clone() else { return };
    let pal = app.pal;
    let mut go = false;
    let mut close = false;
    let builds: Vec<(String, String)> = app.builds.iter().map(|b| (b.name.clone(), b.version.clone().unwrap_or(b.name.clone()))).collect();
    let profiles = app.profiles.clone();
    let rec = app.latest_addons().into_iter().find(|a| a.id == id).cloned();
    egui::Modal::new(egui::Id::new("m_install")).show(ctx, |ui| {
        ui.set_width(380.0);
        let title = rec.as_ref().map(|r| format!("Install {} {}", r.name, r.version)).unwrap_or_else(|| format!("Install {id}"));
        ui.label(egui::RichText::new(title).font(bold(15.0)));
        ui.add_space(4.0);
        ui.label(egui::RichText::new("The add-on goes into the profile's own repository and is enabled there. Every build launched with that profile sees it.").size(12.0).color(pal.muted));
        ui.add_space(8.0);
        egui::Grid::new("install_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label(egui::RichText::new("Profile").color(pal.muted));
            egui::ComboBox::from_id_salt("install_profile").selected_text(&profile).width(220.0).show_ui(ui, |ui| {
                for p in &profiles {
                    ui.selectable_value(&mut profile, p.clone(), p);
                }
            });
            ui.end_row();
            ui.label(egui::RichText::new("Install with").color(pal.muted));
            let shown = builds.iter().find(|(n, _)| *n == build).map(|(_, v)| format!("Blender {v}")).unwrap_or_else(|| "Choose a build".into());
            egui::ComboBox::from_id_salt("install_build").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                for (n, v) in &builds {
                    ui.selectable_value(&mut build, n.clone(), format!("Blender {v}"));
                }
            });
            ui.end_row();
        });
        if let Some(r) = &rec {
            if let Some(min) = &r.blender_version_min {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("Needs Blender {min} or newer")).size(11.5).color(pal.faint));
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let can = !build.is_empty() && !profile.is_empty();
            if ui.add_enabled(can, egui::Button::new(format!("{}  Install", I::DOWNLOAD))).clicked() {
                go = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if go {
        app.dlg.install = None;
        app.install_addon(ctx, id, build, profile);
    } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.install = None;
    } else {
        app.dlg.install = Some((id, build, profile));
    }
}

fn new_profile(app: &mut App, ctx: &egui::Context) {
    let Some(mut name) = app.dlg.new_profile.clone() else { return };
    let pal = app.pal;
    let mut create = false;
    let mut close = false;
    egui::Modal::new(egui::Id::new("m_newprofile")).show(ctx, |ui| {
        ui.set_width(320.0);
        ui.label(egui::RichText::new("New Profile").font(bold(15.0)));
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Starts with Blender's default preferences.").size(12.0).color(pal.muted));
        ui.add_space(6.0);
        let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text("Profile name").desired_width(f32::INFINITY));
        r.request_focus();
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            create = true;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Create")).clicked() {
                create = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if create && !name.trim().is_empty() {
        app.dlg.new_profile = None;
        app.create_profile(ctx, &name);
    } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.new_profile = None;
    } else {
        app.dlg.new_profile = Some(name);
    }
}

fn about(app: &mut App, ctx: &egui::Context) {
    if !app.dlg.about {
        return;
    }
    let pal = app.pal;
    let mut close = false;
    egui::Modal::new(egui::Id::new("m_about")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::Vec2::new(28.0, 32.0), egui::Sense::hover());
            let tex = app.mark_texture(ui.ctx(), r.height());
            crate::ui_shell::paint_mark(ui.painter(), r, pal.accent, &tex);
            ui.label(egui::RichText::new(format!("Spindle {}", env!("CARGO_PKG_VERSION"))).font(bold(18.0)));
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(
            "A build, profile and add-on manager for Blender. Every build shares one profile, \
             agents see the Blenders that are really running, and Superhive purchases download into the library.",
        ).size(12.0).color(pal.muted));
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Inter (SIL OFL) · Lucide icons (ISC)").size(11.0).color(pal.faint));
        ui.add_space(8.0);
        if icon_text_button(ui, &pal, I::CHECK, "Close", true).clicked() {
            close = true;
        }
    });
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.about = false;
    }
}

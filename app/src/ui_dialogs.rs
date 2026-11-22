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

//! Modal dialogs: Superhive token, add from GitHub, install add-on into a profile, new
//! profile, about.

use crate::app::*;
use crate::icons as I;
use crate::theme::bold;
use crate::widgets::*;

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    superhive_login(app, ctx);
    github(app, ctx);
    install(app, ctx);
    new_profile(app, ctx);
    about(app, ctx);
    compat(app, ctx);
    remove(app, ctx);
    self_update(app, ctx);
    backups(app, ctx);
}

/// A profile's backups: restore one (after confirming) or move it to the Recycle Bin.
fn backups(app: &mut App, ctx: &egui::Context) {
    let Some(profile) = app.dlg.backups.clone() else { return };
    let pal = app.pal;
    let list: Vec<spindle_core::backup::Backup> = app.backups_of(&profile).into_iter().cloned().collect();
    let running = app.instances.iter().filter(|i| i.profile.as_deref() == Some(profile.as_str())).count();
    let restoring = app.jobs.iter().any(|j| j.label.starts_with("Restoring "));
    let mut close = false;
    let mut restore: Option<String> = None;
    let mut remove: Option<String> = None;
    egui::Modal::new(egui::Id::new("m_backups")).show(ctx, |ui| {
        ui.set_width(560.0);
        ui.label(egui::RichText::new(format!("Backups of '{profile}'")).font(bold(15.0)));
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Restoring replaces the profile's preferences (a whole-profile backup replaces its add-ons and \
                 extensions too). What it replaces is backed up first, so a restore can be undone.",
            )
            .size(12.0)
            .color(pal.muted),
        );
        if running > 0 {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(if running == 1 {
                    "A Blender is running with this profile. Close it before restoring, or it saves its preferences over the restored ones.".to_owned()
                } else {
                    format!("{running} Blenders are running with this profile. Close them before restoring, or they save their preferences over the restored ones.")
                })
                .size(12.0)
                .color(pal.red),
            );
        }
        ui.add_space(8.0);
        if list.is_empty() {
            ui.label(egui::RichText::new("No backups yet.").size(12.0).color(pal.muted));
        }
        egui::ScrollArea::vertical().max_height(340.0).auto_shrink([false, true]).show(ui, |ui| {
            for b in &list {
                let confirming = app.dlg.restore.as_deref() == Some(b.id.as_str());
                egui::Frame::new()
                    .fill(if confirming { pal.accent_soft } else { pal.card })
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(b.when()).font(bold(13.0)).color(pal.text));
                                    let what = if b.meta.full { "WHOLE PROFILE" } else { "PREFERENCES" };
                                    ui.label(egui::RichText::new(what).size(10.0).color(pal.muted));
                                    if b.meta.kind == spindle_core::backup::Kind::Automatic {
                                        ui.label(egui::RichText::new("AUTOMATIC").size(10.0).color(pal.accent_text));
                                    }
                                });
                                let mut detail = Vec::new();
                                if !b.meta.note.is_empty() {
                                    detail.push(b.meta.note.clone());
                                }
                                if let Some(v) = &b.meta.saved_by {
                                    detail.push(format!("saved by Blender {v}"));
                                }
                                detail.push(crate::ui_center::fmt_size(Some(b.size)));
                                ui.label(egui::RichText::new(detail.join(" · ")).size(11.0).color(pal.muted));
                                if confirming {
                                    let what = if b.meta.full { "the whole profile" } else { "the current preferences" };
                                    ui.label(egui::RichText::new(format!("Replace {what} with this backup?")).size(12.0).color(pal.text));
                                }
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if confirming {
                                    if ui.button("Cancel").clicked() {
                                        app.dlg.restore = None;
                                    }
                                    if ui.add_enabled_ui(running == 0 && !restoring, |ui| icon_text_button(ui, &pal, I::HISTORY, "Restore", true)).inner.clicked() {
                                        restore = Some(b.id.clone());
                                    }
                                } else {
                                    if icon_button(ui, &pal, I::TRASH_2, "Move this backup to the Recycle Bin", false).clicked() {
                                        remove = Some(b.id.clone());
                                    }
                                    if ui.add_enabled(running == 0 && !restoring, egui::Button::new("Restore…")).clicked() {
                                        app.dlg.restore = Some(b.id.clone());
                                    }
                                }
                            });
                        });
                    });
                ui.add_space(6.0);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if icon_text_button(ui, &pal, I::ARCHIVE, "Back Up Now", false).clicked() {
                app.back_up_profile(&profile, false);
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    });
    if let Some(id) = restore {
        app.restore_backup(&id);
        app.dlg.restore = None;
        app.dlg.backups = None;
    } else if let Some(id) = remove {
        app.remove_backup(ctx, &id);
    } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.backups = None;
        app.dlg.restore = None;
    }
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

/// Add from GitHub: a link, then the offer to build from the source when no release has the
/// add-on. Stays open (with a spinner) while the job runs, and shows errors in full.
fn github(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.dlg.github.clone() else { return };
    let pal = app.pal;
    let (mut lookup, mut build, mut back, mut close) = (false, false, false, false);
    egui::Modal::new(egui::Id::new("m_github")).show(ctx, |ui| {
        ui.set_width(460.0);
        match &d.offer {
            Some(o) => {
                ui.label(egui::RichText::new(format!("Build {} from Source?", o.repo)).font(bold(15.0)));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("{}.", o.reason)).size(12.0).color(pal.text));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Spindle can download {} and build an installable add-on from it, the way a release is made: \
                         an extension with Blender's own extension builder (this needs a Blender 4.2 or newer build), \
                         a legacy add-on as a zip of its folder. Nothing from the repository is run.",
                        o.describe()
                    ))
                    .size(12.0)
                    .color(pal.muted),
                );
            }
            None => {
                ui.label(egui::RichText::new("Add from GitHub").font(bold(15.0)));
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Paste a repository link. Spindle downloads the add-on from the newest release, \
                         or offers to build it from the source when there is no release to download.",
                    )
                    .size(12.0)
                    .color(pal.muted),
                );
                ui.add_space(8.0);
                let r = ui.add_enabled(
                    !d.busy,
                    egui::TextEdit::singleline(&mut d.link).hint_text("https://github.com/owner/repo").desired_width(f32::INFINITY),
                );
                if std::mem::take(&mut d.focus) {
                    r.request_focus();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    lookup = true;
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("A release link (…/releases/tag/v1.2) or a branch and folder (…/tree/main/my_addon) works too.")
                        .size(11.0)
                        .color(pal.faint),
                );
            }
        }
        if let Some(e) = &d.error {
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(e).size(11.5).color(pal.red)).selectable(true));
            });
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if d.busy {
                ui.add(egui::Spinner::new().size(14.0).color(pal.accent_text));
                let doing = if d.offer.is_some() { "Downloading and building…" } else { "Looking for the add-on…" };
                ui.label(egui::RichText::new(doing).size(12.0).color(pal.muted));
                if ui.button("Close").on_hover_text("It carries on in the background").clicked() {
                    close = true;
                }
            } else if d.offer.is_some() {
                if icon_text_button(ui, &pal, I::DOWNLOAD, "Download and Build", true).clicked() {
                    build = true;
                }
                if ui.button("Back").clicked() {
                    back = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            } else {
                // A link that doesn't parse explains itself in the dialog.
                if icon_text_button(ui, &pal, I::DOWNLOAD, "Add", true).clicked() && !d.link.trim().is_empty() {
                    lookup = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            }
        });
    });
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.github = None;
        return;
    }
    if back {
        (d.offer, d.error) = (None, None);
    }
    app.dlg.github = Some(d);
    if build {
        app.start_github_build(ctx);
    } else if lookup {
        app.start_github_lookup(ctx);
    }
}

/// Install an add-on into a profile. Builds don't come into it: every build launched with
/// the profile sees the add-on, and Spindle picks the build that runs Blender's installer.
fn install(app: &mut App, ctx: &egui::Context) {
    let Some((id, mut profile)) = app.dlg.install.clone() else { return };
    let pal = app.pal;
    let mut go = false;
    let mut close = false;
    let profiles = app.profiles.clone();
    let rec = app.latest_addons().into_iter().find(|a| a.id == id).cloned();
    // What each profile has now, for the list and the status line.
    let has: Vec<(String, spindle_core::addons::InstalledAddon)> =
        app.installs_of(&id).into_iter().map(|(p, i)| (p.to_owned(), i.clone())).collect();
    let status_of = |p: &str| -> String {
        let Some(r) = &rec else { return String::new() };
        match has.iter().find(|(name, _)| name == p).map(|(_, i)| i) {
            None => format!("Not in '{p}' yet."),
            Some(i) if i.version.as_deref() == Some(r.version.as_str()) => {
                format!("'{p}' already has {}; installing replaces it and switches it on.", r.version)
            }
            Some(i) => format!("'{p}' has {}; this replaces it with {}.", i.version.as_deref().unwrap_or("another version"), r.version),
        }
    };
    let running: Vec<String> = app.running_with(&profile).iter().map(|i| format!("Blender {}", i.blender_version)).collect();
    let prefs = app.profile_state.get(&profile).and_then(|s| s.prefs_by);
    let no_build = rec.as_ref().and_then(|r| spindle_core::addons::pick_installer(&app.builds, prefs, r).err().map(|e| format!("{e:#}")));
    egui::Modal::new(egui::Id::new("m_install")).show(ctx, |ui| {
        ui.set_width(420.0);
        let title = rec.as_ref().map(|r| format!("Install {} {}", r.name, r.version)).unwrap_or_else(|| format!("Install {id}"));
        ui.label(egui::RichText::new(title).font(bold(15.0)));
        ui.add_space(4.0);
        let into = match rec.as_ref().map(|r| r.format) {
            Some(spindle_core::addons::Format::Legacy) => "This legacy add-on goes into the profile's add-ons folder",
            _ => "The add-on goes into the profile's own repository",
        };
        ui.label(egui::RichText::new(format!("{into} and is switched on there. Every build launched with that profile sees it.")).size(12.0).color(pal.muted));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Profile").color(pal.muted));
            egui::ComboBox::from_id_salt("install_profile").selected_text(&profile).width(300.0).show_ui(ui, |ui| {
                for p in &profiles {
                    let note = match has.iter().find(|(name, _)| name == p).map(|(_, i)| i) {
                        Some(i) => format!("  ·  has {}", i.version.as_deref().unwrap_or("it")),
                        None => String::new(),
                    };
                    ui.selectable_value(&mut profile, p.clone(), format!("{p}{note}"));
                }
            });
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(status_of(&profile)).size(12.0).color(pal.text));
        if let Some(r) = &rec {
            if let Some(min) = &r.blender_version_min {
                ui.label(egui::RichText::new(format!("Works with Blender {min} and newer.")).size(11.5).color(pal.faint));
            }
        }
        if !running.is_empty() {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "{} is running with '{profile}'. Close it first: Blender saves its preferences when it quits, which would switch the add-on off again.",
                    running.join(" and ")
                ))
                .size(12.0)
                .color(pal.orange),
            );
        }
        if let Some(why) = &no_build {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(why).size(12.0).color(pal.red));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let can = !profile.is_empty() && running.is_empty() && no_build.is_none() && rec.is_some();
            ui.add_enabled_ui(can, |ui| {
                if icon_text_button(ui, &pal, I::DOWNLOAD, "Install", true).clicked() {
                    go = true;
                }
            });
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if go {
        app.dlg.install = None;
        app.install_addon(ctx, id, profile);
    } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.dlg.install = None;
    } else {
        app.dlg.install = Some((id, profile));
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
             agents see the Blenders that are really running, and add-ons from GitHub and Superhive download into the library.",
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

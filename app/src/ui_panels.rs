//! Left dock (library tree) and right dock (Task Panel / Properties).

use egui::{Frame, Panel, Vec2};

use crate::app::*;
use crate::icons as I;
use crate::platform;
use crate::theme::{bold, ThemeMode};
use crate::widgets::*;

// --------------------------------------------------------------------------------------------
// Left: library tree

pub fn left_panel(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    Panel::left("leftdock")
        .default_size(280.0)
        .size_range(220.0..=480.0)
        .resizable(true)
        .frame(Frame::new().fill(pal.panel))
        .show(ui, |ui| {
            let mut tab = 0u8;
            dock_header(ui, &pal, |ui| {
                tab_strip(ui, &pal, &mut tab, &[(0u8, "Library")]);
            });
            let inner = ui.available_rect_before_wrap().shrink2(Vec2::new(6.0, 6.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| library_tree(app, ui));
        });
}

fn library_tree(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    // Room for the three buttons below the tree.
    let bottom_h = 124.0;
    let tree_h = (ui.available_height() - bottom_h).max(60.0);
    egui::ScrollArea::vertical().id_salt("library_tree").max_height(tree_h).auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        let view = app.prefs.view;

        let open_id = ui.make_persistent_id("tree_builds_open");
        let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(open_id, true));
        let (r, toggled) = tree_row(ui, &pal, 0, Some(open), I::HARD_DRIVE, "Builds", Some(app.builds.len()), view == View::Builds);
        if toggled {
            open = !open;
            ui.data_mut(|d| d.insert_persisted(open_id, open));
        } else if r.clicked() {
            app.set_view(View::Builds);
        }
        if open {
            let builds: Vec<(String, String, usize)> = app
                .builds
                .iter()
                .map(|b| (b.name.clone(), b.version.clone().unwrap_or_else(|| b.name.clone()), app.running_of(b).len()))
                .collect();
            if builds.is_empty() {
                ui.label(egui::RichText::new("        No builds in the library yet").size(11.5).color(pal.faint));
            }
            for (name, version, running) in builds {
                let selected = view == View::Builds && app.selection == Selection::Build(name.clone());
                let channel = app.builds.iter().find(|b| b.name == name).map(|b| b.channel).unwrap_or(Channel::Custom);
                let label = format!("Blender {version} · {}", channel.label());
                let r = tree_row(ui, &pal, 1, None, I::BOX, &label, if running > 0 { Some(running) } else { None }, selected).0;
                if r.clicked() {
                    app.set_view(View::Builds);
                    app.selection = Selection::Build(name.clone());
                }
                if r.double_clicked() {
                    app.launch_build(&ctx, &name);
                }
                r.on_hover_text(&name);
            }
        }

        let running = app.instances.len();
        if tree_row(ui, &pal, 0, None, I::ACTIVITY, "Running", Some(running), view == View::Running).0.clicked() {
            app.set_view(View::Running);
        }

        let ups = app.updates.len();
        let label = if ups > 0 { format!("Download Builds · {ups} new") } else { "Download Builds".to_string() };
        let n = app.catalog.as_ref().map(|c| c.builds.len());
        if tree_row(ui, &pal, 0, None, I::DOWNLOAD, &label, n, view == View::Downloads).0.clicked() {
            app.set_view(View::Downloads);
        }

        let addons = app.latest_addons().len();
        let open_id = ui.make_persistent_id("tree_addons_open");
        let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(open_id, true));
        let (r, toggled) = tree_row(ui, &pal, 0, Some(open), I::PACKAGE, "Add-ons", Some(addons), view == View::Addons);
        if toggled {
            open = !open;
            ui.data_mut(|d| d.insert_persisted(open_id, open));
        } else if r.clicked() {
            app.set_view(View::Addons);
        }
        if open {
            let mut sources: Vec<(String, usize)> = Vec::new();
            for a in app.latest_addons() {
                match sources.iter_mut().find(|(s, _)| s == &a.source) {
                    Some((_, n)) => *n += 1,
                    None => sources.push((a.source.clone(), 1)),
                }
            }
            if sources.is_empty() {
                ui.label(egui::RichText::new("        Library is empty").size(11.5).color(pal.faint));
            }
            for (source, n) in sources {
                let icon = if source == spindle_core::github::SOURCE { I::GIT_COMPARE } else { I::SHIELD_CHECK };
                if tree_row(ui, &pal, 1, None, icon, crate::ui_center::source_label(&source), Some(n), false).0.clicked() {
                    app.set_view(View::Addons);
                }
            }
        }

        let open_id = ui.make_persistent_id("tree_profiles_open");
        let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(open_id, true));
        let (r, toggled) = tree_row(ui, &pal, 0, Some(open), I::USER, "Profiles", Some(app.profiles.len()), view == View::Profiles);
        if toggled {
            open = !open;
            ui.data_mut(|d| d.insert_persisted(open_id, open));
        } else if r.clicked() {
            app.set_view(View::Profiles);
        }
        if open {
            for p in app.profiles.clone() {
                let selected = view == View::Profiles && app.selection == Selection::Profile(p.clone());
                let icon = if p == app.active_profile { I::CIRCLE_CHECK } else { I::CIRCLE };
                if tree_row(ui, &pal, 1, None, icon, &p, None, selected).0.clicked() {
                    app.set_view(View::Profiles);
                    app.selection = Selection::Profile(p.clone());
                }
            }
        }
    });

    ui.add_space(4.0);
    if wide_button(ui, &pal, I::PLUS, "Add Build…").clicked() {
        app.add_external_build(&ctx);
    }
    ui.add_space(4.0);
    let syncing = app.busy("Syncing Superhive");
    if wide_button(ui, &pal, I::DOWNLOAD, if syncing { "Syncing Superhive…" } else { "Sync Superhive Purchases" }).clicked() && !syncing {
        app.sync_superhive(&ctx);
    }
    ui.add_space(4.0);
    if wide_button(ui, &pal, I::GIT_COMPARE, "Add Add-on from GitHub…").clicked() {
        app.open_github(&ctx, None);
    }
}

// --------------------------------------------------------------------------------------------
// Right: task panel / properties

pub fn right_panel(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    Panel::right("rightdock")
        .default_size(300.0)
        .size_range(240.0..=520.0)
        .resizable(true)
        .frame(Frame::new().fill(pal.panel))
        .show(ui, |ui| {
            dock_header(ui, &pal, |ui| {
                tab_strip(ui, &pal, &mut app.right_tab, &[(RightTab::Tasks, "Task Panel"), (RightTab::Properties, "Properties")]);
            });
            let inner = ui.available_rect_before_wrap().shrink2(Vec2::new(6.0, 6.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                egui::ScrollArea::vertical().id_salt(("right", app.right_tab as u8)).auto_shrink([false, false]).show(ui, |ui| match app.right_tab {
                    RightTab::Tasks => tasks(app, ui),
                    RightTab::Properties => properties(app, ui),
                });
            });
        });
}

fn tasks(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    ui.spacing_mut().item_spacing.y = 1.0;
    section(ui, &pal, "sec_common", "Common Tasks", true, |ui| {
        let newest = app.default_label();
        let label = match &newest {
            Some(v) => format!("Launch {v}"),
            None => "Launch Default Build".into(),
        };
        if task_row(ui, &pal, I::PLAY, Some(if newest.is_some() { pal.green } else { pal.faint }), &label, newest.is_some()).clicked() {
            app.launch_default(&ctx);
        }
        if task_row(ui, &pal, I::ACTIVITY, None, "Show Running Blenders", true).clicked() {
            app.set_view(View::Running);
        }
        let syncing = app.busy("Syncing Superhive");
        if task_row(ui, &pal, I::DOWNLOAD, None, if syncing { "Syncing Superhive…" } else { "Sync Superhive Purchases" }, !syncing).clicked() {
            app.sync_superhive(&ctx);
        }
        if task_row(ui, &pal, I::GIT_COMPARE, None, "Add Add-on from GitHub…", true).clicked() {
            app.open_github(&ctx, None);
        }
        let ups = app.updates.len();
        let label = if ups > 0 { format!("Download Builds ({ups} new)") } else { "Download Builds".to_string() };
        if task_row(ui, &pal, I::DOWNLOAD, Some(if ups > 0 { pal.accent_text } else { pal.muted }), &label, true).clicked() {
            app.set_view(View::Downloads);
        }
        if task_row(ui, &pal, I::REFRESH_CW, None, if app.checking { "Checking for New Builds…" } else { "Check for New Builds" }, !app.checking).clicked() {
            app.check_updates(&ctx, true);
        }
        if task_row(ui, &pal, I::PLUS, None, "Add Build…", true).clicked() {
            app.add_external_build(&ctx);
        }
        if task_row(ui, &pal, I::FOLDER_OPEN, None, "Open Library Folder", true).clicked() {
            platform::open_folder(&app.sp.library);
        }
    });

    match app.selection.clone() {
        Selection::Build(name) => {
            let running = app.selected_build().map(|b| app.running_of(b).len()).unwrap_or(0);
            let exe_dir = app.selected_build().and_then(|b| b.executable.parent().map(|p| p.to_path_buf()));
            section(ui, &pal, "sec_build", "Selected Build", true, |ui| {
                let label = format!("Launch with '{}'", app.active_profile);
                if task_row(ui, &pal, I::PLAY, Some(pal.green), &label, true).clicked() {
                    app.launch_build(&ctx, &name);
                }
                if task_row(ui, &pal, I::ACTIVITY, None, &format!("Show Running Instances ({running})"), running > 0).clicked() {
                    app.set_view(View::Running);
                }
                let fav = app.selected_build().is_some_and(|b| b.favourite);
                let fav_label = if fav { "Remove from Favourites" } else { "Add to Favourites" };
                if task_row(ui, &pal, I::STAR, Some(if fav { pal.accent } else { pal.muted }), fav_label, true).clicked() {
                    app.toggle_favourite(&name);
                }
                if task_row(ui, &pal, I::FOLDER_OPEN, None, "Open Build Folder", exe_dir.is_some()).clicked() {
                    if let Some(d) = &exe_dir {
                        platform::open_folder(d);
                    }
                }
                let report = app.compat.get(&name).cloned();
                let (icon, col, label) = match crate::ui_center::compat_marker(app, &name) {
                    Some((icon, col, _)) => (icon, Some(col), "Profile Compatibility Warnings…".to_string()),
                    None => (I::CIRCLE_CHECK, Some(pal.green), format!("Works with '{}'", app.active_profile)),
                };
                if task_row(ui, &pal, icon, col, &label, report.is_some()).clicked() {
                    if let Some(report) = report {
                        app.dlg.compat = Some(CompatDialog { build: name.clone(), report });
                    }
                }
                if task_row(ui, &pal, I::TRASH_2, None, "Remove Build…", running == 0).clicked() {
                    app.dlg.remove = Some(name.clone());
                }
            });
        }
        Selection::Instance(_) => {
            let endpoint = app.selected_instance().and_then(|i| i.mcp.iter().find(|m| m.port.is_some()).cloned());
            let file = app.selected_instance().map(|i| i.file.clone()).unwrap_or_default();
            section(ui, &pal, "sec_inst", "Selected Blender", true, |ui| {
                let label = match &endpoint {
                    Some(m) => format!("Copy MCP Address ({}:{})", m.host.as_deref().unwrap_or("localhost"), m.port.unwrap_or(0)),
                    None => "Copy MCP Address (no MCP add-on)".into(),
                };
                if task_row(ui, &pal, I::BOT, None, &label, endpoint.is_some()).clicked() {
                    if let Some(m) = &endpoint {
                        ctx.copy_text(format!("{}:{}", m.host.as_deref().unwrap_or("localhost"), m.port.unwrap_or(0)));
                        app.toast(&ctx, "MCP address copied");
                    }
                }
                let folder = std::path::Path::new(&file).parent().map(|p| p.to_path_buf());
                if task_row(ui, &pal, I::FOLDER_OPEN, None, "Open File's Folder", folder.is_some() && !file.is_empty()).clicked() {
                    if let Some(d) = &folder {
                        platform::open_folder(d);
                    }
                }
            });
        }
        Selection::Addon { id, .. } => {
            let can = !app.builds.is_empty();
            let origin = app.selected_addon().and_then(|a| a.origin.clone());
            section(ui, &pal, "sec_addon", "Selected Add-on", true, |ui| {
                if task_row(ui, &pal, I::DOWNLOAD, Some(if can { pal.green } else { pal.faint }), "Install into Profile…", can).clicked() {
                    app.dlg.install = Some((id.clone(), app.active_profile.clone()));
                }
                if let Some(o) = &origin {
                    // Adding the same link again picks up a newer release or commit.
                    let busy = app.busy(&format!("GitHub {}", o.repo));
                    if task_row(ui, &pal, I::REFRESH_CW, None, if busy { "Checking GitHub…" } else { "Check GitHub for a Newer Version" }, !busy).clicked() {
                        app.open_github(&ctx, Some(o.url.clone()));
                    }
                    if task_row(ui, &pal, I::EXTERNAL_LINK, None, "Open on GitHub", true).clicked() {
                        platform::open_url(&o.url);
                    }
                }
            });
        }
        Selection::Available(_) => {
            let a = app.selected_available().cloned();
            section(ui, &pal, "sec_avail", "Selected Download", true, |ui| {
                if let Some(a) = a {
                    let installed = app.installed_as(&a).map(|b| b.name.clone());
                    let busy = app.download_progress(&a.name).is_some();
                    match installed {
                        Some(b) => {
                            if task_row(ui, &pal, I::PLAY, Some(pal.green), "Launch (installed)", true).clicked() {
                                app.launch_build(&ctx, &b);
                            }
                        }
                        None => {
                            if task_row(ui, &pal, I::DOWNLOAD, Some(pal.green), if busy { "Downloading…" } else { "Download and Install" }, !busy).clicked() {
                                app.start_download(&ctx, a.clone());
                            }
                        }
                    }
                    if busy && task_row(ui, &pal, I::X, None, "Cancel Download", true).clicked() {
                        app.cancel_download(&a.name);
                    }
                    if task_row(ui, &pal, I::COPY, None, "Copy Download Link", true).clicked() {
                        ctx.copy_text(a.url.clone());
                        app.toast(&ctx, "Download link copied");
                    }
                } else {
                    ui.label(egui::RichText::new("No longer listed").size(12.0).color(pal.muted));
                }
            });
        }
        Selection::Profile(name) => {
            let is_default = app.sp.profile_name(None) == name;
            section(ui, &pal, "sec_profile", "Selected Profile", true, |ui| {
                if task_row(ui, &pal, I::CIRCLE_CHECK, None, "Use for Launching", app.active_profile != name).clicked() {
                    app.active_profile = name.clone();
                }
                if task_row(ui, &pal, I::STAR, None, "Make Default Profile", !is_default).clicked() {
                    app.set_default_profile(&ctx, &name);
                }
                if task_row(ui, &pal, I::FOLDER_OPEN, None, "Open Profile Folder", true).clicked() {
                    platform::open_folder(&app.profile_dir(&name));
                }
                let backing_up = app.busy(&format!("Backing up {name}"));
                if task_row(ui, &pal, I::ARCHIVE, None, "Back Up Preferences", !backing_up)
                    .on_hover_text("Save a copy of the preferences: userpref.blend, the startup file, bookmarks and recent files")
                    .clicked()
                {
                    app.back_up_profile(&name, false);
                }
                if task_row(ui, &pal, I::FILE_BOX, None, "Back Up Whole Profile", !backing_up)
                    .on_hover_text("The preferences plus every add-on and extension in the profile (larger)")
                    .clicked()
                {
                    app.back_up_profile(&name, true);
                }
                let n = app.backups_of(&name).len();
                let label = if n == 0 { "Restore Preferences…".to_owned() } else { format!("Restore Preferences… ({n})") };
                if task_row(ui, &pal, I::HISTORY, None, &label, n > 0).on_hover_text("Pick a backup to restore").clicked() {
                    app.dlg.backups = Some(name.clone());
                    app.dlg.restore = None;
                }
            });
        }
        Selection::None => {}
    }

    section(ui, &pal, "sec_agents", "Agents", true, |ui| {
        if task_row(ui, &pal, I::COPY, None, "Copy MCP Client Config", true).clicked() {
            ctx.copy_text(app.mcp_config_snippet());
            app.toast(&ctx, "MCP client config copied: point your agent at spindle-mcp");
        }
        let with_mcp = app.instances.iter().filter(|i| i.mcp.iter().any(|m| m.port.is_some())).count();
        let label = format!("{with_mcp} running Blender(s) reachable over MCP");
        task_row(ui, &pal, I::BOT, Some(if with_mcp > 0 { pal.green } else { pal.faint }), &label, false);
    });

    section(ui, &pal, "sec_customize", "Customize Program", true, |ui| {
        let token_label = if app.token_stored { "Superhive Token (saved)" } else { "Set Superhive Token…" };
        if task_row(ui, &pal, I::SHIELD_CHECK, None, token_label, true).clicked() {
            app.dlg.superhive_login = true;
        }
        if task_row(ui, &pal, I::USER, None, "New Profile…", true).clicked() {
            app.dlg.new_profile = Some(String::new());
        }
        let (icon, label, next) = if pal.dark { (I::SUN, "Switch to Light Theme", ThemeMode::Light) } else { (I::MOON, "Switch to Dark Theme", ThemeMode::Dark) };
        if task_row(ui, &pal, icon, None, label, true).clicked() {
            app.set_theme(&ctx, next);
        }
        let tray = if app.prefs.close_to_tray { "Closing Keeps Spindle in the Tray" } else { "Closing Exits Spindle" };
        if task_row(ui, &pal, if app.prefs.close_to_tray { I::CIRCLE_CHECK } else { I::CIRCLE }, None, tray, true)
            .on_hover_text("When on, the close button hides Spindle to the notification area, where it keeps watching for new builds")
            .clicked()
        {
            app.prefs.close_to_tray = !app.prefs.close_to_tray;
            app.prefs_dirty = true;
        }
        let check = |on: bool| if on { I::CIRCLE_CHECK } else { I::CIRCLE };
        if task_row(ui, &pal, check(app.autostart), None, "Start Spindle When I Sign In", true)
            .on_hover_text("Starts Spindle hidden in the tray, where it launches builds and watches for new ones")
            .clicked()
        {
            app.toggle_autostart(&ctx);
        }
        let can_associate = !cfg!(target_os = "macos");
        if task_row(ui, &pal, check(app.opens_blend), None, "Open .blend Files with Spindle", can_associate)
            .on_hover_text(if can_associate {
                "Spindle opens each file in the build that saved it (or the default build), after checking the profile"
            } else {
                "On macOS, file types come from app bundles; use `spindle open <file>` instead"
            })
            .clicked()
        {
            app.toggle_blend_association(&ctx);
        }
        if task_row(ui, &pal, check(app.on_path), None, "Add Spindle to PATH", true)
            .on_hover_text("Run spindle and spindle-mcp by name in a terminal")
            .clicked()
        {
            app.toggle_path(&ctx);
        }
        if cfg!(windows) && app.opens_blend {
            if task_row(ui, &pal, I::FILE_BOX, None, "Choose Default App for .blend…", true)
                .on_hover_text("Windows asks you to pick the default app yourself: choose Spindle for .blend")
                .clicked()
            {
                spindle_core::desktop::open_default_apps();
            }
        }
    });

    section(ui, &pal, "sec_details", "Details", true, |ui| {
        Frame::new().fill(pal.deep).corner_radius(6).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new("Library").font(bold(12.5)).color(pal.text));
            ui.add_space(2.0);
            kv(ui, &pal, "Builds", &app.builds.len().to_string());
            kv(ui, &pal, "Running", &app.instances.len().to_string());
            kv(ui, &pal, "Add-ons", &app.latest_addons().len().to_string());
            kv(ui, &pal, "Profiles", &app.profiles.len().to_string());
            kv(ui, &pal, "Superhive token", if app.token_stored { "Saved" } else { "Not set" });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(app.sp.library.display().to_string()).size(11.0).color(pal.muted));
        });
    });
}

pub fn kv(ui: &mut egui::Ui, pal: &crate::theme::Palette, k: &str, v: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(k).size(12.0).color(pal.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(v).size(12.0).color(pal.text)).truncate());
        });
    });
}

fn properties(app: &mut App, ui: &mut egui::Ui) {
    let pal = app.pal;
    let title = |ui: &mut egui::Ui, t: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(t).font(bold(13.0)).color(pal.accent_text));
        ui.add_space(2.0);
    };
    match &app.selection {
        Selection::Build(_) => {
            if let Some(b) = app.selected_build() {
                title(ui, &format!("Blender {}", b.version.as_deref().unwrap_or("?")));
                kv(ui, &pal, "Name", &b.name);
                kv(ui, &pal, "Channel", b.channel.label());
                kv(ui, &pal, "Location", if b.external { "External" } else { "Library" });
                kv(ui, &pal, "Running", &app.running_of(b).len().to_string());
                if let Some(py) = spindle_core::compat::build_python(b) {
                    kv(ui, &pal, "Python", &format!("{}.{}", py.0, py.1));
                }
                if let Some(m) = &b.meta {
                    if !m.branch.is_empty() {
                        kv(ui, &pal, "Branch", &m.branch);
                    }
                    if !m.hash.is_empty() {
                        kv(ui, &pal, "Commit", &m.hash);
                    }
                    if m.published.is_some() {
                        kv(ui, &pal, "Built", &crate::ui_center::ago(m.published));
                    }
                }
                if let Some(r) = app.compat.get(&b.name) {
                    let n = r.issues.iter().filter(|i| i.severity >= spindle_core::compat::Severity::Warning).count();
                    kv(ui, &pal, "Profile check", &if n == 0 { "Compatible".to_string() } else { format!("{n} issue(s)") });
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new(b.executable.display().to_string()).size(11.0).color(pal.muted));
            }
        }
        Selection::Instance(_) => {
            if let Some(i) = app.selected_instance() {
                title(ui, &format!("Blender {} · pid {}", i.blender_version, i.pid));
                kv(ui, &pal, "Profile", i.profile.as_deref().unwrap_or("-"));
                kv(ui, &pal, "File", if i.file.is_empty() { "(unsaved)" } else { &i.file });
                for m in &i.mcp {
                    let addr = match m.port {
                        Some(p) => format!("{}:{p}", m.host.as_deref().unwrap_or("localhost")),
                        None => "no port".into(),
                    };
                    kv(ui, &pal, "MCP", &addr);
                    ui.label(egui::RichText::new(&m.addon).size(11.0).color(pal.muted));
                }
                if i.mcp.is_empty() {
                    kv(ui, &pal, "MCP", "No MCP add-on enabled");
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&i.binary).size(11.0).color(pal.muted));
            }
        }
        Selection::Addon { .. } => {
            if let Some(a) = app.selected_addon() {
                title(ui, &a.name);
                kv(ui, &pal, "Id", &a.id);
                kv(ui, &pal, "Version", &a.version);
                kv(ui, &pal, "Type", &a.kind);
                kv(ui, &pal, "Format", a.format.label());
                kv(ui, &pal, "Source", crate::ui_center::source_label(&a.source));
                let installs = app.installs_of(&a.id);
                if installs.is_empty() {
                    kv(ui, &pal, "Installed in", "No profile");
                }
                for (n, (p, i)) in installs.iter().enumerate() {
                    let state = match i.enabled {
                        Some(true) => " · on",
                        Some(false) => " · off",
                        None => "",
                    };
                    kv(ui, &pal, if n == 0 { "Installed in" } else { "" }, &format!("{p} · {}{state}", i.version.as_deref().unwrap_or("?")));
                }
                if let Some(o) = &a.origin {
                    kv(ui, &pal, "Repository", &o.repo);
                    kv(ui, &pal, if o.built { "Built from" } else { "Release" }, &o.git_ref);
                    if !o.commit.is_empty() {
                        kv(ui, &pal, "Commit", &o.commit[..o.commit.len().min(12)]);
                    }
                    if !o.path.is_empty() {
                        kv(ui, &pal, "Folder", &o.path);
                    }
                }
                kv(ui, &pal, "Blender", &format!("{} – {}", a.blender_version_min.as_deref().unwrap_or("any"), a.blender_version_max.as_deref().unwrap_or("")));
                kv(ui, &pal, "Platforms", &if a.platforms.is_empty() { "All".to_string() } else { a.platforms.join(", ") });
                kv(ui, &pal, "Archive", &a.archive);
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("sha256 {}", a.sha256)).size(10.5).color(pal.faint));
            }
        }
        Selection::Available(_) => {
            if let Some(a) = app.selected_available() {
                title(ui, &format!("Blender {}", a.version));
                kv(ui, &pal, "Channel", a.channel.label());
                kv(ui, &pal, "Cycle", &a.cycle);
                kv(ui, &pal, if matches!(a.channel, Channel::Stable | Channel::Lts) { "Series" } else { "Branch" }, &a.branch);
                if !a.hash.is_empty() {
                    kv(ui, &pal, "Commit", &a.hash);
                }
                if let Some(s) = a.size {
                    kv(ui, &pal, "Size", &format!("{} MB", s / 1_000_000));
                }
                kv(ui, &pal, "Published", &crate::ui_center::ago(a.published));
                kv(ui, &pal, "Checksum", if a.sha256_url.is_some() { "SHA-256 verified" } else { "None published" });
                kv(ui, &pal, "Installs as", &a.name);
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&a.url).size(11.0).color(pal.muted));
            }
        }
        Selection::Profile(p) => {
            title(ui, p);
            let dir = app.profile_dir(p);
            kv(ui, &pal, "Extensions", &crate::ui_center::count_extensions(&dir).to_string());
            kv(ui, &pal, "Preferences", if dir.join("config").join("userpref.blend").is_file() { "Saved" } else { "Defaults" });
            let backups = app.backups_of(p);
            let summary = match backups.first() {
                Some(b) => format!("{} (newest {})", backups.len(), b.when()),
                None => "None yet".to_owned(),
            };
            kv(ui, &pal, "Backups", &summary);
            kv(ui, &pal, "Bridge", if dir.join("scripts").join("startup").join("spindle_bridge.py").is_file() { "Installed" } else { "Added on first launch" });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(dir.display().to_string()).size(11.0).color(pal.muted));
        }
        Selection::None => {
            ui.label(egui::RichText::new("Select a build, Blender, add-on or profile to see its properties.").color(pal.muted).size(12.0));
        }
    }
}

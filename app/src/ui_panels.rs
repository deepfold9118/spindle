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
    let bottom_h = 84.0;
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
                let label = format!("Blender {version} · {}", Channel::of(&name).label());
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
                ui.label(egui::RichText::new("        Library is empty: sync Superhive").size(11.5).color(pal.faint));
            }
            for (source, n) in sources {
                let label = if source == "superhive" { "Superhive".to_string() } else { source.clone() };
                if tree_row(ui, &pal, 1, None, I::SHIELD_CHECK, &label, Some(n), false).0.clicked() {
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
        let newest = app.builds.first().map(|b| b.version.clone().unwrap_or(b.name.clone()));
        let label = match &newest {
            Some(v) => format!("Launch Blender {v}"),
            None => "Launch Newest Build".into(),
        };
        if task_row(ui, &pal, I::PLAY, Some(if newest.is_some() { pal.green } else { pal.faint }), &label, newest.is_some()).clicked() {
            app.launch_newest(&ctx);
        }
        if task_row(ui, &pal, I::ACTIVITY, None, "Show Running Blenders", true).clicked() {
            app.set_view(View::Running);
        }
        let syncing = app.busy("Syncing Superhive");
        if task_row(ui, &pal, I::DOWNLOAD, None, if syncing { "Syncing Superhive…" } else { "Sync Superhive Purchases" }, !syncing).clicked() {
            app.sync_superhive(&ctx);
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
                if task_row(ui, &pal, I::FOLDER_OPEN, None, "Open Build Folder", exe_dir.is_some()).clicked() {
                    if let Some(d) = &exe_dir {
                        platform::open_folder(d);
                    }
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
            section(ui, &pal, "sec_addon", "Selected Add-on", true, |ui| {
                if task_row(ui, &pal, I::DOWNLOAD, Some(if can { pal.green } else { pal.faint }), "Install into Profile…", can).clicked() {
                    let build = app.builds.first().map(|b| b.name.clone()).unwrap_or_default();
                    app.dlg.install = Some((id.clone(), build, app.active_profile.clone()));
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
                kv(ui, &pal, "Channel", Channel::of(&b.name).label());
                kv(ui, &pal, "Location", if b.external { "External" } else { "Library" });
                kv(ui, &pal, "Running", &app.running_of(b).len().to_string());
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
                kv(ui, &pal, "Source", &a.source);
                kv(ui, &pal, "Blender", &format!("{} – {}", a.blender_version_min.as_deref().unwrap_or("any"), a.blender_version_max.as_deref().unwrap_or("")));
                kv(ui, &pal, "Platforms", &if a.platforms.is_empty() { "All".to_string() } else { a.platforms.join(", ") });
                kv(ui, &pal, "Archive", &a.archive);
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("sha256 {}", a.sha256)).size(10.5).color(pal.faint));
            }
        }
        Selection::Profile(p) => {
            title(ui, p);
            let dir = app.profile_dir(p);
            kv(ui, &pal, "Extensions", &crate::ui_center::count_extensions(&dir).to_string());
            kv(ui, &pal, "Preferences", if dir.join("config").join("userpref.blend").is_file() { "Saved" } else { "Defaults" });
            kv(ui, &pal, "Bridge", if dir.join("scripts").join("startup").join("spindle_bridge.py").is_file() { "Installed" } else { "Added on first launch" });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(dir.display().to_string()).size(11.0).color(pal.muted));
        }
        Selection::None => {
            ui.label(egui::RichText::new("Select a build, Blender, add-on or profile to see its properties.").color(pal.muted).size(12.0));
        }
    }
}

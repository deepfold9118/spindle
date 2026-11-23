//! Centre area: build cards, running-instance cards, the add-on library list and
//! profile cards. Card backgrounds take their click before their buttons are added,
//! so the buttons on top still win their own clicks.

use std::path::Path;

use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::app::*;
use crate::icons as I;
use crate::platform;
use crate::theme::{bold, icon_font, Palette};
use crate::widgets::*;

pub fn center(app: &mut App, ui: &mut Ui) {
    let inner = ui.available_rect_before_wrap().shrink(10.0);
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        egui::ScrollArea::vertical().id_salt(("center", app.prefs.view as u8)).auto_shrink([false, false]).show(ui, |ui| match app.prefs.view {
            View::Builds => builds_view(app, ui),
            View::Running => running_view(app, ui),
            View::Addons => addons_view(app, ui),
            View::Profiles => profiles_view(app, ui),
            View::Downloads => downloads_view(app, ui),
        });
    });
}

/// Lay out `n` cards of at least `min_w` wide in rows; `draw` gets each card's rect.
fn card_grid(ui: &mut Ui, n: usize, min_w: f32, h: f32, mut draw: impl FnMut(&mut Ui, usize, Rect)) {
    let gap = 10.0;
    let avail = ui.available_width();
    let cols = ((avail + gap) / (min_w + gap)).floor().max(1.0) as usize;
    let w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
    let mut i = 0;
    while i < n {
        let (row, _) = ui.allocate_exact_size(Vec2::new(avail, h), Sense::hover());
        for c in 0..cols {
            if i >= n {
                break;
            }
            let rect = Rect::from_min_size(row.min + Vec2::new(c as f32 * (w + gap), 0.0), Vec2::new(w, h));
            draw(ui, i, rect);
            i += 1;
        }
        ui.add_space(gap);
    }
}

fn card_bg(ui: &Ui, pal: &Palette, rect: Rect, selected: bool, hovered: bool) {
    ui.painter().rect_filled(rect, CornerRadius::same(6), pal.card);
    let stroke = if selected {
        Stroke::new(1.5, pal.accent)
    } else if hovered {
        Stroke::new(1.0, pal.hover)
    } else {
        Stroke::new(1.0, pal.border)
    };
    ui.painter().rect_stroke(rect, CornerRadius::same(6), stroke, StrokeKind::Inside);
}

/// Truncated single-line text painted at a position.
fn text_clip(ui: &Ui, pos: egui::Pos2, max_w: f32, text: &str, font: egui::FontId, color: Color32) {
    let galley = ui.painter().layout(text.to_string(), font, color, f32::INFINITY);
    let clip = Rect::from_min_size(egui::pos2(pos.x, pos.y - galley.size().y / 2.0), Vec2::new(max_w.max(1.0), galley.size().y));
    ui.painter().with_clip_rect(clip).galley(clip.min, galley, color);
}

fn empty_state(ui: &mut Ui, pal: &Palette, icon: &str, title: &str, body: &str) -> bool {
    let mut clicked = false;
    ui.add_space(80.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(icon).font(icon_font(40.0)).color(pal.faint));
        ui.add_space(8.0);
        ui.label(egui::RichText::new(title).font(bold(16.0)).color(pal.text));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(body).size(12.5).color(pal.muted));
        ui.add_space(12.0);
        clicked = true;
    });
    clicked
}

// --------------------------------------------------------------------------------------------
// Builds

fn channel_color(pal: &Palette, c: Channel) -> Color32 {
    match c {
        Channel::Lts => pal.accent,
        Channel::Stable => pal.green,
        Channel::Daily => pal.blue,
        Channel::Experimental | Channel::Custom => pal.grey,
    }
}

/// Text colour on a channel badge.
fn channel_fg(pal: &Palette, c: Channel) -> Color32 {
    let col = channel_color(pal, c);
    if col == pal.accent || col == pal.orange { pal.on_accent } else { Color32::WHITE }
}

/// Compatibility marker for a build: (icon, colour, tooltip) when there's something to say.
pub fn compat_marker(app: &App, build: &str) -> Option<(&'static str, Color32, String)> {
    let r = app.compat.get(build)?;
    let n = r.issues.iter().filter(|i| i.severity >= spindle_core::compat::Severity::Warning).count();
    match r.worst()? {
        spindle_core::compat::Severity::Blocker => Some((I::CIRCLE_ALERT, app.pal.red, format!("Can't use profile '{}' safely: {}", r.profile, r.issues[0].title))),
        spindle_core::compat::Severity::Warning => Some((I::TRIANGLE_ALERT, app.pal.orange, format!("{n} compatibility warning(s) with profile '{}'", r.profile))),
        spindle_core::compat::Severity::Info => None,
    }
}

fn builds_view(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    let builds: Vec<_> = app.visible_builds().into_iter().cloned().collect();
    if builds.is_empty() {
        empty_state(
            ui,
            &pal,
            I::HARD_DRIVE,
            if app.builds.is_empty() { "No builds yet" } else { "No builds match" },
            if app.builds.is_empty() { "Download Blender from blender.org, or add a build you already have" } else { "Clear the search or turn on more channels" },
        );
        if app.builds.is_empty() {
            ui.vertical_centered(|ui| {
                if icon_text_button(ui, &pal, I::DOWNLOAD, "Download Builds…", true).clicked() {
                    app.set_view(View::Downloads);
                }
                ui.add_space(4.0);
                if icon_text_button(ui, &pal, I::PLUS, "Add Build…", false).clicked() {
                    app.add_external_build(&ctx);
                }
            });
        }
        return;
    }
    card_grid(ui, builds.len(), 340.0, 156.0, |ui, i, rect| {
        let b = &builds[i];
        let channel = b.channel;
        let running = app.running_of(b).len();
        let selected = app.selection == Selection::Build(b.name.clone());
        let resp = ui.interact(rect, ui.id().with(("build", &b.name)), Sense::click());
        card_bg(ui, &pal, rect, selected, resp.hovered());
        if resp.clicked() {
            app.selection = Selection::Build(b.name.clone());
        }
        if resp.double_clicked() {
            app.launch_build(&ctx, &b.name);
        }

        let p = ui.painter();
        let x = rect.left() + 12.0;
        let br = badge(ui, &pal, egui::pos2(x, rect.top() + 12.0), channel.label(), channel_color(&pal, channel), channel_fg(&pal, channel));
        let marker = compat_marker(app, &b.name);
        let name_w = rect.right() - br.right() - if marker.is_some() { 72.0 } else { 48.0 };
        // Favourite star, top right (left of the compatibility marker).
        let sx = rect.right() - if marker.is_some() { 46.0 } else { 22.0 };
        let sr = Rect::from_center_size(egui::pos2(sx, br.center().y), Vec2::splat(22.0));
        let tip = match (b.favourite, i == 0) {
            (true, true) => "Default build (first favourite) · click to unstar",
            (true, false) => "Favourite · click to unstar",
            _ => "Star: favourites are listed first, and the first one is the default build",
        };
        let star = ui.interact(sr, ui.id().with(("fav", &b.name)), Sense::click()).on_hover_text(tip);
        let star_col = if b.favourite {
            pal.accent
        } else if star.hovered() {
            pal.text
        } else if resp.hovered() || selected {
            pal.faint
        } else {
            Color32::TRANSPARENT
        };
        if b.favourite {
            ui.painter().circle_filled(sr.center(), 4.0, pal.accent);
        }
        ui.painter().text(sr.center(), Align2::CENTER_CENTER, I::STAR, icon_font(15.0), star_col);
        if star.clicked() {
            app.toggle_favourite(&b.name);
        }
        text_clip(ui, egui::pos2(br.right() + 8.0, br.center().y), name_w, &b.name, egui::FontId::proportional(11.5), pal.muted);
        if let Some((icon, col, tip)) = marker {
            let mr = Rect::from_center_size(egui::pos2(rect.right() - 22.0, br.center().y), Vec2::splat(22.0));
            let r = ui.interact(mr, ui.id().with(("compat", &b.name)), Sense::click()).on_hover_text(format!("{tip}\nClick for details"));
            ui.painter().text(mr.center(), Align2::CENTER_CENTER, icon, icon_font(14.0), if r.hovered() { pal.text } else { col });
            if r.clicked() {
                if let Some(report) = app.compat.get(&b.name).cloned() {
                    app.dlg.compat = Some(CompatDialog { build: b.name.clone(), report });
                }
            }
        }
        let version = b.version.clone().unwrap_or_else(|| b.name.clone());
        p.text(egui::pos2(x, rect.top() + 52.0), Align2::LEFT_CENTER, format!("Blender {version}"), bold(24.0), pal.text);
        text_clip(ui, egui::pos2(x, rect.top() + 80.0), rect.width() - 24.0, &b.executable.display().to_string(), egui::FontId::proportional(11.0), pal.faint);

        let row = Rect::from_min_max(egui::pos2(x, rect.bottom() - 42.0), egui::pos2(rect.right() - 12.0, rect.bottom() - 12.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(Layout::left_to_right(Align::Center)), |ui| {
            if icon_text_button(ui, &pal, I::PLAY, "Launch", true).on_hover_text(format!("Launch with profile '{}'", app.active_profile)).clicked() {
                app.selection = Selection::Build(b.name.clone());
                app.launch_build(&ctx, &b.name);
            }
            if icon_button(ui, &pal, I::FOLDER_OPEN, "Open build folder", false).clicked() {
                if let Some(d) = b.executable.parent() {
                    platform::open_folder(d);
                }
            }
            let removing = app.busy(&format!("Removing {}", b.name));
            let tip = if b.external { "Remove from Spindle (files stay)" } else { "Remove build (to the Recycle Bin)" };
            if !removing && icon_button(ui, &pal, I::TRASH_2, tip, false).clicked() {
                app.dlg.remove = Some(b.name.clone());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if running > 0 {
                    let r = ui.label(egui::RichText::new(format!("{running} running")).size(12.0).color(pal.green));
                    dot(ui, egui::pos2(r.rect.left() - 8.0, r.rect.center().y), 4.0, pal.green);
                    if r.interact(Sense::click()).on_hover_text("Show running instances").clicked() {
                        app.set_view(View::Running);
                    }
                } else {
                    ui.label(egui::RichText::new("Not running").size(12.0).color(pal.faint));
                }
            });
        });
    });
}

// --------------------------------------------------------------------------------------------
// Running instances

fn running_view(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    let list: Vec<_> = app
        .instances
        .iter()
        .filter(|i| app.matches_find(&format!("{} {} {} {}", i.blender_version, i.file, i.profile.as_deref().unwrap_or(""), i.pid)))
        .cloned()
        .collect();
    if list.is_empty() {
        empty_state(
            ui,
            &pal,
            I::ACTIVITY,
            "No Blender running",
            "Blenders launched from Spindle appear here within a few seconds, along with their MCP add-on address",
        );
        if !app.builds.is_empty() {
            ui.vertical_centered(|ui| {
                if icon_text_button(ui, &pal, I::PLAY, "Launch Default Build", true).clicked() {
                    app.launch_default(&ctx);
                }
            });
        }
        return;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    card_grid(ui, list.len(), 380.0, 150.0, |ui, i, rect| {
        let inst = &list[i];
        let selected = app.selection == Selection::Instance(inst.pid);
        let resp = ui.interact(rect, ui.id().with(("inst", inst.pid)), Sense::click());
        card_bg(ui, &pal, rect, selected, resp.hovered());
        if resp.clicked() {
            app.selection = Selection::Instance(inst.pid);
        }
        let p = ui.painter();
        let x = rect.left() + 12.0;
        let br = badge(ui, &pal, egui::pos2(x, rect.top() + 12.0), "RUNNING", pal.green, Color32::WHITE);
        let up = (now - inst.started).max(0.0) as u64;
        let uptime = if up >= 3600 { format!("{}h {}m", up / 3600, up % 3600 / 60) } else { format!("{}m {}s", up / 60, up % 60) };
        text_clip(ui, egui::pos2(br.right() + 8.0, br.center().y), rect.width() - 120.0, &format!("pid {} · up {uptime} · profile {}", inst.pid, inst.profile.as_deref().unwrap_or("-")), egui::FontId::proportional(11.5), pal.muted);
        p.text(egui::pos2(x, rect.top() + 50.0), Align2::LEFT_CENTER, format!("Blender {}", inst.blender_version), bold(22.0), pal.text);
        let file = if inst.file.is_empty() { "(unsaved file)".to_string() } else { inst.file.clone() };
        text_clip(ui, egui::pos2(x + 18.0, rect.top() + 78.0), rect.width() - 42.0, &file, egui::FontId::proportional(12.0), pal.text);
        p.text(egui::pos2(x + 6.0, rect.top() + 78.0), Align2::CENTER_CENTER, I::FILE, icon_font(12.0), pal.muted);

        // MCP endpoint line: what agents need.
        let y = rect.bottom() - 30.0;
        match inst.mcp.iter().find(|m| m.port.is_some()) {
            Some(m) => {
                let addr = format!("{}:{}", m.host.as_deref().unwrap_or("localhost"), m.port.unwrap_or(0));
                p.text(egui::pos2(x + 6.0, y), Align2::CENTER_CENTER, I::BOT, icon_font(13.0), pal.accent_text);
                let shown = p.text(egui::pos2(x + 20.0, y), Align2::LEFT_CENTER, format!("MCP  {addr}"), bold(12.5), pal.accent_text);
                let name_x = shown.right() + 10.0;
                text_clip(ui, egui::pos2(name_x, y), rect.right() - 44.0 - name_x, &m.addon, egui::FontId::proportional(11.0), pal.muted);
                let br = Rect::from_center_size(egui::pos2(rect.right() - 25.0, y), Vec2::splat(26.0));
                let r = ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| icon_button(ui, &pal, I::COPY, "Copy MCP address", false)).inner;
                if r.clicked() {
                    ctx.copy_text(addr.clone());
                    app.toast(&ctx, format!("Copied {addr}"));
                }
            }
            None => {
                p.text(egui::pos2(x + 6.0, y), Align2::CENTER_CENTER, I::BOT, icon_font(13.0), pal.faint);
                p.text(egui::pos2(x + 20.0, y), Align2::LEFT_CENTER, "No MCP add-on enabled in this profile", egui::FontId::proportional(12.0), pal.faint);
            }
        }
    });
}

// --------------------------------------------------------------------------------------------
// Add-on library

/// Display name of an add-on source.
pub fn source_label(source: &str) -> &str {
    match source {
        "superhive" => "Superhive",
        "github" => "GitHub",
        other => other,
    }
}

fn addons_view(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    let list: Vec<_> = app
        .latest_addons()
        .into_iter()
        .filter(|a| app.matches_find(&format!("{} {} {}", a.name, a.id, a.source)))
        .cloned()
        .collect();
    if list.is_empty() {
        empty_state(
            ui,
            &pal,
            I::PACKAGE,
            if app.addons.is_empty() { "The add-on library is empty" } else { "No add-ons match" },
            if app.addons.is_empty() { "Add one from a GitHub repository, or sync your Superhive purchases" } else { "Clear the search to see every add-on" },
        );
        if app.addons.is_empty() {
            ui.vertical_centered(|ui| {
                if icon_text_button(ui, &pal, I::GIT_COMPARE, "Add from GitHub…", true).clicked() {
                    app.open_github(&ctx, None);
                }
                ui.add_space(6.0);
                if icon_text_button(ui, &pal, I::DOWNLOAD, "Sync Superhive Purchases", false).clicked() {
                    app.sync_superhive(&ctx);
                }
            });
        }
        return;
    }
    let versions = |id: &str| app.addons.iter().filter(|a| a.id.eq_ignore_ascii_case(id)).count();
    let counts: Vec<usize> = list.iter().map(|a| versions(&a.id)).collect();
    ui.spacing_mut().item_spacing.y = 6.0;
    for (a, n_versions) in list.iter().zip(counts) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
        let sel = Selection::Addon { source: a.source.clone(), id: a.id.clone(), version: a.version.clone() };
        let selected = app.selection == sel;
        let resp = ui.interact(rect, ui.id().with(("addon", &a.source, &a.id)), Sense::click());
        card_bg(ui, &pal, rect, selected, resp.hovered());
        if resp.clicked() {
            app.selection = sel.clone();
        }
        let p = ui.painter();
        let x = rect.left() + 14.0;
        let icon_rect = Rect::from_center_size(egui::pos2(x + 14.0, rect.center().y), Vec2::splat(32.0));
        p.rect_filled(icon_rect, CornerRadius::same(6), pal.raised);
        p.text(icon_rect.center(), Align2::CENTER_CENTER, if a.kind == "theme" { I::PALETTE } else { I::PACKAGE }, icon_font(16.0), pal.accent_text);
        let tx = icon_rect.right() + 12.0;
        let text_w = rect.width() * 0.45;
        text_clip(ui, egui::pos2(tx, rect.center().y - 9.0), text_w, &a.name, bold(14.0), pal.text);
        let sub = if n_versions > 1 { format!("{} · {} versions stored", a.id, n_versions) } else { a.id.clone() };
        text_clip(ui, egui::pos2(tx, rect.center().y + 10.0), text_w, &sub, egui::FontId::proportional(11.5), pal.muted);

        let mid = rect.left() + rect.width() * 0.55;
        let top = rect.center().y - 17.0;
        let vb = badge(ui, &pal, egui::pos2(mid, top), &a.version, pal.raised, pal.text);
        let kb = badge(ui, &pal, egui::pos2(vb.right() + 6.0, top), &a.kind, Color32::TRANSPARENT, pal.muted);
        let source = match &a.origin {
            Some(o) if o.built => format!("{} · built", source_label(&a.source)),
            _ => source_label(&a.source).to_owned(),
        };
        ui.painter().text(egui::pos2(kb.right() + 12.0, vb.center().y), Align2::LEFT_CENTER, source, egui::FontId::proportional(12.0), pal.muted);
        // Which profiles have it.
        let (icon, colour, text) = installed_summary(app, a);
        let y = rect.center().y + 11.0;
        let ir = ui.painter().text(egui::pos2(mid, y), Align2::LEFT_CENTER, icon, icon_font(12.0), colour);
        text_clip(ui, egui::pos2(ir.right() + 5.0, y), rect.right() - 162.0 - ir.right(), &text, egui::FontId::proportional(12.0), colour);

        let row = Rect::from_min_max(egui::pos2(rect.right() - 150.0, rect.top()), egui::pos2(rect.right() - 12.0, rect.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(Layout::right_to_left(Align::Center)), |ui| {
            let installing = app.busy(&format!("Installing {}", a.id));
            if installing {
                ui.add(egui::Spinner::new().size(14.0).color(pal.accent_text));
            } else if icon_text_button(ui, &pal, I::DOWNLOAD, "Install…", false).on_hover_text("Install into a profile").clicked() {
                app.selection = sel.clone();
                app.dlg.install = Some((a.id.clone(), app.active_profile.clone()));
            }
        });
    }
}

/// One line on where an add-on is installed: "Installed in default, studio (0.26.0)", with
/// profiles that have another version or have it switched off marked as such.
pub fn installed_summary(app: &App, a: &spindle_core::addons::AddonRecord) -> (&'static str, Color32, String) {
    let installs = app.installs_of(&a.id);
    if installs.is_empty() {
        return (I::CIRCLE_DASHED, app.pal.faint, "Not installed in a profile".into());
    }
    let mut all_on = true;
    let parts: Vec<String> = installs
        .iter()
        .map(|(p, i)| {
            let mut notes = Vec::new();
            if let Some(v) = i.version.as_deref().filter(|v| *v != a.version) {
                notes.push(v.to_owned());
            }
            if i.enabled == Some(false) {
                notes.push("off".to_owned());
                all_on = false;
            }
            if notes.is_empty() { (*p).to_owned() } else { format!("{p} ({})", notes.join(", ")) }
        })
        .collect();
    let colour = if all_on { app.pal.green } else { app.pal.orange };
    (I::CIRCLE_CHECK, colour, format!("Installed in {}", parts.join(", ")))
}

// --------------------------------------------------------------------------------------------
// Profiles

/// Installed extensions in a profile (all repositories, hidden folders skipped).
pub fn count_extensions(profile_dir: &Path) -> usize {
    let Ok(repos) = std::fs::read_dir(profile_dir.join("extensions")) else { return 0 };
    repos
        .flatten()
        .filter(|r| r.path().is_dir() && !r.file_name().to_string_lossy().starts_with('.'))
        .map(|r| {
            std::fs::read_dir(r.path())
                .map(|it| it.flatten().filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.')).count())
                .unwrap_or(0)
        })
        .sum()
}

fn profiles_view(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();
    let list: Vec<String> = app.profiles.iter().filter(|p| app.matches_find(p)).cloned().collect();
    if list.is_empty() {
        empty_state(ui, &pal, I::USER, "No profiles", "A profile is created the first time a build is launched with it");
        ui.vertical_centered(|ui| {
            if icon_text_button(ui, &pal, I::PLUS, "New Profile…", true).clicked() {
                app.dlg.new_profile = Some(String::new());
            }
        });
        return;
    }
    let default = app.sp.profile_name(None);
    card_grid(ui, list.len(), 320.0, 140.0, |ui, i, rect| {
        let name = &list[i];
        let dir = app.profile_dir(name);
        let selected = app.selection == Selection::Profile(name.clone());
        let resp = ui.interact(rect, ui.id().with(("profile", name)), Sense::click());
        card_bg(ui, &pal, rect, selected, resp.hovered());
        if resp.clicked() {
            app.selection = Selection::Profile(name.clone());
        }
        let x = rect.left() + 12.0;
        let mut bx = x;
        if *name == default {
            bx = badge(ui, &pal, egui::pos2(bx, rect.top() + 12.0), "DEFAULT", pal.accent, pal.on_accent).right() + 6.0;
        }
        if *name == app.active_profile {
            badge(ui, &pal, egui::pos2(bx, rect.top() + 12.0), "LAUNCHING", pal.green, Color32::WHITE);
        }
        let p = ui.painter();
        p.text(egui::pos2(x, rect.top() + 50.0), Align2::LEFT_CENTER, name, bold(22.0), pal.text);
        let ext = count_extensions(&dir);
        let prefs = if dir.join("config").join("userpref.blend").is_file() { "saved preferences" } else { "default preferences" };
        let backups = match app.backups_of(name).len() {
            0 => String::new(),
            1 => " · 1 backup".to_owned(),
            n => format!(" · {n} backups"),
        };
        p.text(egui::pos2(x, rect.top() + 76.0), Align2::LEFT_CENTER, format!("{ext} extensions · {prefs}{backups}"), egui::FontId::proportional(12.0), pal.muted);
        let row = Rect::from_min_max(egui::pos2(x, rect.bottom() - 42.0), egui::pos2(rect.right() - 12.0, rect.bottom() - 12.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(Layout::left_to_right(Align::Center)), |ui| {
            let active = *name == app.active_profile;
            if icon_text_button(ui, &pal, if active { I::CIRCLE_CHECK } else { I::CIRCLE }, if active { "Launching with this" } else { "Use for Launching" }, !active).clicked() {
                app.active_profile = name.clone();
            }
            if icon_button(ui, &pal, I::FOLDER_OPEN, "Open profile folder", false).clicked() {
                platform::open_folder(&dir);
            }
            if *name != default && icon_button(ui, &pal, I::STAR, "Make default profile", false).clicked() {
                app.set_default_profile(&ctx, name);
            }
        });
    });
}

// --------------------------------------------------------------------------------------------
// Download builds

pub fn fmt_size(bytes: Option<u64>) -> String {
    match bytes {
        Some(b) if b >= 1_000_000 => format!("{} MB", b / 1_000_000),
        Some(b) => format!("{} KB", b.div_ceil(1000)),
        None => String::new(),
    }
}

/// "3 days ago" style age of a unix timestamp.
pub fn ago(ts: Option<i64>) -> String {
    let Some(ts) = ts else { return String::new() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let s = (now - ts).max(0);
    match s {
        0..=3599 => format!("{} min ago", (s / 60).max(1)),
        3600..=86_399 => format!("{} h ago", s / 3600),
        86_400..=172_799 => "1 day ago".into(),
        172_800..=2_591_999 => format!("{} days ago", s / 86_400),
        2_592_000..=5_183_999 => "1 month ago".into(),
        _ => format!("{} months ago", s / 2_592_000),
    }
}

fn downloads_view(app: &mut App, ui: &mut Ui) {
    let pal = app.pal;
    let ctx = ui.ctx().clone();

    // Header: catalog age, refresh, and updates for what's installed.
    ui.horizontal(|ui| {
        let fetched = app.catalog.as_ref().map(|c| c.fetched);
        let label = match fetched {
            Some(t) => format!("Listings from blender.org, checked {}", ago(Some(t))),
            None => "Builds from download.blender.org and builder.blender.org".into(),
        };
        ui.label(egui::RichText::new(label).size(12.0).color(pal.muted));
        if app.checking {
            ui.add(egui::Spinner::new().size(12.0).color(pal.accent_text));
        } else if icon_text_button(ui, &pal, I::REFRESH_CW, "Check Now", false).clicked() {
            app.check_updates(&ctx, true);
        }
    });
    if let Some(errs) = app.catalog.as_ref().map(|c| c.errors.clone()).filter(|e| !e.is_empty()) {
        ui.label(egui::RichText::new(format!("Some sources failed: {}", errs.join("; "))).size(11.5).color(pal.red));
    }
    // Say what the channel chips hide, so a switched-off channel doesn't look like missing builds.
    let hidden = app.hidden_available();
    if !hidden.is_empty() {
        ui.horizontal(|ui| {
            let what: Vec<String> = hidden.iter().map(|(c, n)| format!("{n} {}", c.label())).collect();
            let text = format!("Not shown: {} (switched off under Show)", what.join(", "));
            ui.label(egui::RichText::new(text).size(12.0).color(pal.muted));
            if icon_text_button(ui, &pal, I::EYE, "Show All", false).clicked() {
                app.show_all_channels();
            }
        });
    }
    ui.add_space(6.0);

    let updates = app.updates.clone();
    if !updates.is_empty() {
        egui::Frame::new().fill(pal.accent_soft).corner_radius(6).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(format!("{}  Newer builds available", I::ZAP)).font(bold(13.5)).color(pal.text));
            ui.add_space(4.0);
            for u in &updates {
                ui.horizontal(|ui| {
                    let what = match &u.installed {
                        Some(i) => format!("{i}  →  Blender {} ({})", u.available.version, u.available.channel.label()),
                        None => format!("New {} release: Blender {}", u.available.channel.label(), u.available.version),
                    };
                    ui.label(egui::RichText::new(what).size(12.5).color(pal.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| download_control(app, ui, &ctx, &u.available, true));
                });
            }
        });
        ui.add_space(8.0);
    }

    let list: Vec<_> = app.visible_available().into_iter().cloned().collect();
    if list.is_empty() {
        let none = app.catalog.as_ref().is_none_or(|c| c.builds.is_empty());
        empty_state(
            ui,
            &pal,
            I::DOWNLOAD,
            if none { "No listings yet" } else { "No builds match" },
            if none { "Check blender.org for Stable, LTS, Daily and Experimental builds" } else { "Clear the search or turn on more channels" },
        );
        if none && !app.checking {
            ui.vertical_centered(|ui| {
                if icon_text_button(ui, &pal, I::REFRESH_CW, "Check for Builds", true).clicked() {
                    app.check_updates(&ctx, true);
                }
            });
        }
        return;
    }
    ui.spacing_mut().item_spacing.y = 6.0;
    let groups: [(&str, &[Channel]); 3] = [
        ("Official releases", &[Channel::Stable, Channel::Lts]),
        ("Daily builds", &[Channel::Daily]),
        ("Experimental and patch builds", &[Channel::Experimental]),
    ];
    for (title, channels) in groups {
        let rows: Vec<&spindle_core::downloads::Available> = list.iter().filter(|a| channels.contains(&a.channel)).collect();
        if rows.is_empty() {
            continue;
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new(format!("{title}  ·  {}", rows.len())).font(bold(13.0)).color(pal.accent_text));
        for a in rows {
            available_row(app, ui, &ctx, a);
        }
        ui.add_space(6.0);
    }
}

fn available_row(app: &mut App, ui: &mut Ui, ctx: &egui::Context, a: &spindle_core::downloads::Available) {
    let pal = app.pal;
    {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let sel = Selection::Available(a.name.clone());
        let resp = ui.interact(rect, ui.id().with(("avail", &a.name)), Sense::click());
        card_bg(ui, &pal, rect, app.selection == sel, resp.hovered());
        if resp.clicked() {
            app.selection = sel.clone();
        }
        let x = rect.left() + 12.0;
        let br = badge(ui, &pal, egui::pos2(x, rect.center().y - 9.0), a.channel.label(), channel_color(&pal, a.channel), channel_fg(&pal, a.channel));
        let tx = x + 104.0;
        let _ = br;
        text_clip(ui, egui::pos2(tx, rect.center().y - 8.0), 170.0, &format!("Blender {}", a.version), bold(14.0), pal.text);
        let sub = if matches!(a.channel, Channel::Stable | Channel::Lts) {
            format!("{} release", if a.channel == Channel::Lts { "Long-term support" } else { "Official" })
        } else {
            format!("{} · {}", a.cycle, a.branch)
        };
        text_clip(ui, egui::pos2(tx, rect.center().y + 10.0), rect.width() * 0.42, &sub, egui::FontId::proportional(11.5), pal.muted);
        let mid = rect.left() + rect.width() * 0.55;
        ui.painter().text(egui::pos2(mid, rect.center().y), Align2::LEFT_CENTER, format!("{}   {}", fmt_size(a.size), ago(a.published)), egui::FontId::proportional(12.0), pal.muted);

        let row = Rect::from_min_max(egui::pos2(rect.right() - 230.0, rect.top()), egui::pos2(rect.right() - 12.0, rect.bottom()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(Layout::right_to_left(Align::Center)), |ui| download_control(app, ui, ctx, a, false));
    }
}

/// Download button, progress bar with cancel, or "Installed".
fn download_control(app: &mut App, ui: &mut Ui, ctx: &egui::Context, a: &spindle_core::downloads::Available, primary: bool) {
    let pal = app.pal;
    if let Some(p) = app.download_progress(&a.name) {
        if icon_button(ui, &pal, I::X, "Cancel download", false).clicked() {
            app.cancel_download(&a.name);
        }
        let (text, frac) = match (p.phase(), p.fraction()) {
            (spindle_core::downloads::Phase::Downloading, f) => (format!("{:.0}%", f.unwrap_or(0.0) * 100.0), f.unwrap_or(0.0)),
            (spindle_core::downloads::Phase::Verifying, _) => ("Verifying".into(), 1.0),
            (spindle_core::downloads::Phase::Extracting, f) => (format!("Unpacking {:.0}%", f.unwrap_or(0.0) * 100.0), f.unwrap_or(0.0)),
        };
        ui.add(egui::ProgressBar::new(frac).desired_width(150.0).text(egui::RichText::new(text).size(11.0)).fill(pal.accent));
    } else if let Some(b) = app.installed_as(a) {
        let name = b.name.clone();
        if icon_text_button(ui, &pal, I::PLAY, "Launch", false).on_hover_text(format!("Installed as {name}")).clicked() {
            app.launch_build(ctx, &name);
        }
        ui.label(egui::RichText::new("Installed").size(12.0).color(pal.green));
        ui.label(egui::RichText::new(I::CIRCLE_CHECK).font(icon_font(12.0)).color(pal.green));
    } else if icon_text_button(ui, &pal, I::DOWNLOAD, "Download", primary)
        .on_hover_text(format!("{} from {}", fmt_size(a.size), a.url.split('/').nth(2).unwrap_or("blender.org")))
        .clicked()
    {
        app.start_download(ctx, a.clone());
    }
}

#![allow(dead_code)]
//! Small ArtCraft-style building blocks (shared with Fontabulous): icon buttons, tab strips, chips and badges.

use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::theme::{bold, icon_font, Palette};

/// Square icon button; `active` shows the accent-tinted selected state.
pub fn icon_button(ui: &mut Ui, pal: &Palette, icon: &str, tip: &str, active: bool) -> Response {
    icon_button_sized(ui, pal, icon, tip, active, true)
}

pub fn icon_button_sized(ui: &mut Ui, pal: &Palette, icon: &str, tip: &str, active: bool, enabled: bool) -> Response {
    let size = Vec2::splat(26.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let bg = if active {
            pal.accent_soft
        } else if resp.hovered() && enabled {
            pal.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
        let col = if !enabled {
            pal.faint
        } else if active {
            pal.accent_text
        } else if resp.hovered() {
            pal.text
        } else {
            pal.muted
        };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, icon_font(15.0), col);
    }
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// Button with a Lucide icon and a label.
pub fn icon_text_button(ui: &mut Ui, pal: &Palette, icon: &str, label: &str, primary: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), pal.text);
    let w = galley.size().x + 40.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let (bg, fg) = if primary {
            (if resp.hovered() { pal.accent_text } else { pal.accent }, pal.on_accent)
        } else {
            (if resp.hovered() { pal.hover } else { pal.raised }, pal.text)
        };
        ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
        ui.painter().text(rect.left_center() + Vec2::new(16.0, 0.0), Align2::CENTER_CENTER, icon, icon_font(14.0), fg);
        ui.painter().text(rect.left_center() + Vec2::new(30.0, 0.0), Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), fg);
    }
    resp
}

/// Wide flat button used at the bottom of the Fonts tree.
pub fn wide_button(ui: &mut Ui, pal: &Palette, icon: &str, label: &str) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::click());
    let bg = if resp.hovered() { pal.hover } else { pal.raised };
    ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), pal.text);
    let total = galley.size().x + 22.0;
    let x0 = rect.center().x - total / 2.0;
    ui.painter().text(egui::pos2(x0 + 7.0, rect.center().y), Align2::CENTER_CENTER, icon, icon_font(14.0), pal.muted);
    ui.painter().galley(egui::pos2(x0 + 22.0, rect.center().y - galley.size().y / 2.0), galley, pal.text);
    resp
}

/// ArtCraft tab strip: plain labels, the selected one on a raised chip.
pub fn tab_strip<T: PartialEq + Copy>(ui: &mut Ui, pal: &Palette, current: &mut T, tabs: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (t, label) in tabs {
            let sel = *current == *t;
            let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), pal.text);
            let size = Vec2::new(galley.size().x + 20.0, 26.0);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
            let bg = if sel {
                pal.raised
            } else if resp.hovered() {
                pal.hover.gamma_multiply(0.6)
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
            let col = if sel { pal.text } else { pal.muted };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, *label, egui::FontId::proportional(12.5), col);
            if resp.clicked() && !sel {
                *current = *t;
                changed = true;
            }
        }
    });
    changed
}

/// Header strip for a dock panel: tabs on the left, extra controls on the right.
pub fn dock_header(ui: &mut Ui, pal: &Palette, add: impl FnOnce(&mut Ui)) {
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 34.0));
    ui.painter().rect_filled(rect, 0.0, pal.deep);
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, pal.border));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(6.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    add(&mut child);
    ui.allocate_rect(rect, Sense::hover());
}

/// Coloured activation dot.
pub fn dot(ui: &Ui, center: egui::Pos2, r: f32, c: Color32) {
    ui.painter().circle_filled(center, r, c);
}

/// Small rounded badge with text, e.g. the file type "O"/"T" or weight "B".
pub fn badge(ui: &Ui, pal: &Palette, pos: egui::Pos2, text: &str, fill: Color32, fg: Color32) -> Rect {
    let galley = ui.painter().layout_no_wrap(text.to_string(), bold(10.0), fg);
    let size = Vec2::new((galley.size().x + 8.0).max(16.0), 15.0);
    let rect = Rect::from_min_size(pos, size);
    ui.painter().rect_filled(rect, CornerRadius::same(3), fill);
    if fill == Color32::TRANSPARENT {
        ui.painter().rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, pal.faint), StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    rect
}

/// Collapsible section header used in the task panel and properties grid.
pub fn section(ui: &mut Ui, pal: &Palette, id: &str, title: &str, default_open: bool, add: impl FnOnce(&mut Ui)) {
    let id = ui.make_persistent_id(id);
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, default_open));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), pal.hover.gamma_multiply(0.5));
    }
    let chev = if open { crate::icons::CHEVRON_DOWN } else { crate::icons::CHEVRON_RIGHT };
    ui.painter().text(rect.left_center() + Vec2::new(9.0, 0.0), Align2::CENTER_CENTER, chev, icon_font(12.0), pal.muted);
    ui.painter().text(rect.left_center() + Vec2::new(20.0, 0.0), Align2::LEFT_CENTER, title, bold(12.0), pal.muted);
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(id, open));
    }
    if open {
        ui.add_space(2.0);
        add(ui);
        ui.add_space(6.0);
    }
}

/// Row with icon + label acting like a link (task panel).
pub fn task_row(ui: &mut Ui, pal: &Palette, icon: &str, icon_color: Option<Color32>, label: &str, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), if enabled { Sense::click() } else { Sense::hover() });
    if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, CornerRadius::same(4), pal.hover.gamma_multiply(0.7));
    }
    let ic = icon_color.unwrap_or(if enabled { pal.muted } else { pal.faint });
    ui.painter().text(rect.left_center() + Vec2::new(28.0, 0.0), Align2::CENTER_CENTER, icon, icon_font(14.0), ic);
    let tc = if !enabled {
        pal.faint
    } else if resp.hovered() {
        pal.accent_text
    } else {
        pal.text
    };
    ui.painter().text(rect.left_center() + Vec2::new(42.0, 0.0), Align2::LEFT_CENTER, label, egui::FontId::proportional(12.5), tc);
    resp
}

/// Tree row: indentation, optional expander, icon, label and right-aligned count.
#[allow(clippy::too_many_arguments)]
pub fn tree_row(
    ui: &mut Ui,
    pal: &Palette,
    depth: usize,
    expander: Option<bool>,
    icon: &str,
    label: &str,
    count: Option<usize>,
    selected: bool,
) -> (Response, bool) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
    let mut toggled = false;
    if ui.is_rect_visible(rect) {
        if selected {
            ui.painter().rect_filled(rect, CornerRadius::same(4), pal.accent_soft);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(4), pal.hover.gamma_multiply(0.6));
        }
        let x = rect.left() + 6.0 + depth as f32 * 16.0;
        if let Some(open) = expander {
            let r = Rect::from_center_size(egui::pos2(x + 6.0, rect.center().y), Vec2::splat(16.0));
            let chev = if open { crate::icons::CHEVRON_DOWN } else { crate::icons::CHEVRON_RIGHT };
            ui.painter().text(r.center(), Align2::CENTER_CENTER, chev, icon_font(12.0), pal.muted);
            if let Some(p) = resp.interact_pointer_pos() {
                if resp.clicked() && r.expand(3.0).contains(p) {
                    toggled = true;
                }
            }
        }
        let ic = if selected { pal.accent_text } else { pal.muted };
        ui.painter().text(egui::pos2(x + 22.0, rect.center().y), Align2::CENTER_CENTER, icon, icon_font(14.0), ic);
        let right_pad = if count.is_some() { 44.0 } else { 6.0 };
        let max_w = (rect.right() - right_pad - (x + 34.0)).max(10.0);
        let galley = ui.painter().layout(label.to_string(), egui::FontId::proportional(12.5), pal.text, f32::INFINITY);
        let clip = Rect::from_min_max(egui::pos2(x + 34.0, rect.top()), egui::pos2(x + 34.0 + max_w, rect.bottom()));
        ui.painter().with_clip_rect(clip).galley(egui::pos2(x + 34.0, rect.center().y - galley.size().y / 2.0), galley, pal.text);
        if let Some(c) = count {
            ui.painter().text(
                rect.right_center() - Vec2::new(8.0, 0.0),
                Align2::RIGHT_CENTER,
                c.to_string(),
                egui::FontId::proportional(11.5),
                pal.muted,
            );
        }
    }
    (resp, toggled)
}

/// A pill toggle with a coloured dot (activation filters).
pub fn dot_chip(ui: &mut Ui, pal: &Palette, color: Color32, label: &str, on: &mut bool) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), pal.text);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 30.0, 24.0), Sense::click());
    let bg = if *on { pal.raised } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0, if *on { pal.raised } else { pal.border }), StrokeKind::Inside);
    let c = if *on { color } else { pal.faint };
    ui.painter().circle_filled(rect.left_center() + Vec2::new(12.0, 0.0), 4.0, c);
    ui.painter().galley(rect.left_center() + Vec2::new(22.0, -galley.size().y / 2.0), galley, if *on { pal.text } else { pal.muted });
    if resp.clicked() {
        *on = !*on;
    }
    resp
}

/// Thin vertical divider between toolbar groups.
pub fn vsep(ui: &mut Ui, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 20.0), Sense::hover());
    ui.painter().line_segment([rect.center_top(), rect.center_bottom()], Stroke::new(1.0, pal.border));
}

/// Five-star rating control.
pub fn stars(ui: &mut Ui, pal: &Palette, rating: &mut u8) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for i in 1..=5u8 {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
            let on = i <= *rating;
            let col = if on { pal.orange } else if resp.hovered() { pal.muted } else { pal.faint };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, crate::icons::STAR, icon_font(16.0), col);
            if resp.clicked() {
                *rating = if *rating == i { 0 } else { i };
                changed = true;
            }
        }
    });
    changed
}


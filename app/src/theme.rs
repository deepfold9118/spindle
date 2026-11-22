//! ArtCraft-style look: deep blue-grey panels, Spindle's amber accent (the logo colour), Inter for the
//! interface and Lucide line icons. Light and dark palettes share every role.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub title: Color32,
    pub panel: Color32,
    pub canvas: Color32,
    pub deep: Color32,
    pub border: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    /// Text and icons drawn on a filled accent (amber is too light for white).
    pub on_accent: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub card: Color32,
    pub green: Color32,
    pub orange: Color32,
    pub blue: Color32,
    pub grey: Color32,
    pub red: Color32,
}

pub fn palette(mode: ThemeMode) -> Palette {
    match mode {
        ThemeMode::Dark => Palette {
            dark: true,
            title: Color32::from_rgb(0x0b, 0x0d, 0x12),
            panel: Color32::from_rgb(0x18, 0x1b, 0x22),
            canvas: Color32::from_rgb(0x13, 0x16, 0x1c),
            deep: Color32::from_rgb(0x0f, 0x11, 0x16),
            border: Color32::from_rgb(0x23, 0x28, 0x32),
            raised: Color32::from_rgb(0x28, 0x2d, 0x37),
            hover: Color32::from_rgb(0x31, 0x36, 0x41),
            accent: Color32::from_rgb(0xfb, 0x9e, 0x35),
            accent_soft: Color32::from_rgb(0x5c, 0x42, 0x28),
            accent_text: Color32::from_rgb(0xfd, 0xb5, 0x62),
            on_accent: Color32::from_rgb(0x1a, 0x12, 0x06),
            text: Color32::from_rgb(0xe4, 0xe6, 0xea),
            muted: Color32::from_rgb(0x91, 0x96, 0x9f),
            faint: Color32::from_rgb(0x5a, 0x60, 0x6b),
            card: Color32::from_rgb(0x1a, 0x1e, 0x26),
            green: Color32::from_rgb(0x3f, 0xb9, 0x6a),
            orange: Color32::from_rgb(0xf0, 0xa2, 0x2e),
            blue: Color32::from_rgb(0x4c, 0x9b, 0xe8),
            grey: Color32::from_rgb(0x6b, 0x71, 0x7c),
            red: Color32::from_rgb(0xe5, 0x48, 0x4d),
        },
        ThemeMode::Light => Palette {
            dark: false,
            title: Color32::from_rgb(0xe4, 0xe7, 0xec),
            panel: Color32::from_rgb(0xf3, 0xf4, 0xf6),
            canvas: Color32::from_rgb(0xfa, 0xfa, 0xfb),
            deep: Color32::from_rgb(0xe9, 0xeb, 0xef),
            border: Color32::from_rgb(0xd5, 0xd8, 0xde),
            raised: Color32::from_rgb(0xe4, 0xe7, 0xeb),
            hover: Color32::from_rgb(0xd8, 0xdc, 0xe2),
            accent: Color32::from_rgb(0xfb, 0x9e, 0x35),
            accent_soft: Color32::from_rgb(0xf8, 0xe1, 0xc6),
            accent_text: Color32::from_rgb(0xa8, 0x5c, 0x08),
            on_accent: Color32::from_rgb(0x1a, 0x12, 0x06),
            text: Color32::from_rgb(0x1c, 0x1f, 0x26),
            muted: Color32::from_rgb(0x5a, 0x60, 0x6b),
            faint: Color32::from_rgb(0xa0, 0xa6, 0xb0),
            card: Color32::from_rgb(0xff, 0xff, 0xff),
            green: Color32::from_rgb(0x1f, 0x9d, 0x55),
            orange: Color32::from_rgb(0xdb, 0x8a, 0x10),
            blue: Color32::from_rgb(0x2f, 0x7f, 0xd0),
            grey: Color32::from_rgb(0x9a, 0xa0, 0xaa),
            red: Color32::from_rgb(0xd9, 0x36, 0x3c),
        },
    }
}

pub const ICON_FAMILY: &str = "lucide";

pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    defs.font_data.insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../assets/Inter-Regular.ttf"))));
    defs.font_data.insert("inter-semibold".into(), Arc::new(FontData::from_static(include_bytes!("../assets/Inter-SemiBold.ttf"))));
    let mut lucide = FontData::from_static(include_bytes!("../assets/lucide.ttf"));
    lucide.tweak.y_offset_factor = 0.0;
    defs.font_data.insert("lucide".into(), Arc::new(lucide));

    let prop = defs.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "inter".into());
    // Lucide right after Inter so its icons win over egui's bundled emoji icon font.
    prop.insert(1, "lucide".into());
    defs.families.entry(FontFamily::Monospace).or_default().push("lucide".into());
    let mut bold = vec!["inter-semibold".to_string()];
    bold.extend(defs.families[&FontFamily::Proportional].iter().skip(1).cloned());
    defs.families.insert(FontFamily::Name("bold".into()), bold);
    defs.families.insert(FontFamily::Name(ICON_FAMILY.into()), vec!["lucide".into(), "inter".into()]);
    ctx.set_fonts(defs);
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICON_FAMILY.into()))
}

pub fn apply(ctx: &egui::Context, mode: ThemeMode) {
    let p = palette(mode);
    let mut visuals = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = p.panel;
    visuals.window_fill = p.panel;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.extreme_bg_color = p.deep;
    visuals.faint_bg_color = if p.dark { Color32::from_rgb(0x1c, 0x20, 0x28) } else { Color32::from_rgb(0xee, 0xf0, 0xf3) };
    visuals.code_bg_color = p.deep;
    visuals.text_edit_bg_color = Some(if p.dark { p.deep } else { Color32::WHITE });
    visuals.hyperlink_color = p.accent_text;
    visuals.override_text_color = None;
    visuals.selection.bg_fill = p.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, p.accent_text);
    visuals.window_shadow = egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(if p.dark { 110 } else { 40 }) };
    visuals.popup_shadow = egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(if p.dark { 100 } else { 35 }) };
    visuals.slider_trailing_fill = true;
    visuals.striped = false;

    let r = CornerRadius::same(5);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.raised;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.accent_soft;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.raised;
    w.open.weak_bg_fill = p.raised;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = r;

    ctx.set_visuals_of(if p.dark { egui::Theme::Dark } else { egui::Theme::Light }, visuals);
    ctx.set_theme(if p.dark { egui::Theme::Dark } else { egui::Theme::Light });

    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(16.0, FontFamily::Name("bold".into()))),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
            (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(6.0, 4.0);
        style.spacing.button_padding = Vec2::new(8.0, 3.0);
        style.spacing.interact_size = Vec2::new(24.0, 24.0);
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.window_margin = Margin::same(12);
        style.spacing.slider_width = 110.0;
        style.spacing.combo_width = 120.0;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.floating = true;
        style.interaction.selectable_labels = false;
    });
}

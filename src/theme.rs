//! Look of the settings window: fonts, icons and the dark/light palettes.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Vec2};

use crate::config::Theme;

// Font Awesome 6 Free (solid) code points.
pub const I_CROSSHAIRS: &str = "\u{f05b}";
pub const I_EYE: &str = "\u{f06e}";
pub const I_EYE_SLASH: &str = "\u{f070}";
pub const I_UP: &str = "\u{f062}";
pub const I_DOWN: &str = "\u{f063}";
pub const I_TRASH: &str = "\u{f2ed}";
pub const I_PLUS: &str = "\u{f067}";
pub const I_IMAGE: &str = "\u{f03e}";
pub const I_SUN: &str = "\u{f185}";
pub const I_MOON: &str = "\u{f186}";
pub const I_SAVE: &str = "\u{f0c7}";
pub const I_RESET: &str = "\u{f2ea}";
pub const I_PLAY: &str = "\u{f04b}";
pub const I_WARN: &str = "\u{f071}";
pub const I_FOLDER: &str = "\u{f07c}";
pub const I_DOT: &str = "\u{f111}";
pub const I_RING: &str = "\u{f1ce}";
pub const I_CHEVRON: &str = "\u{f077}";
pub const I_REFRESH: &str = "\u{f021}";
pub const I_GAMEPAD: &str = "\u{f11b}";
pub const I_GEAR: &str = "\u{f013}";
pub const I_BOOKMARK: &str = "\u{f02e}";
pub const I_XMARK: &str = "\u{f00d}";
pub const I_COPY: &str = "\u{f0c5}";
pub const I_CHECK: &str = "\u{f00c}";
pub const I_PASTE: &str = "\u{f0ea}";

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub panel_hi: Color32,
    pub line: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_dim: Color32,
    pub danger: Color32,
}

pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette {
            bg: Color32::from_rgb(0x13, 0x14, 0x17),
            panel: Color32::from_rgb(0x1B, 0x1C, 0x20),
            panel_hi: Color32::from_rgb(0x25, 0x27, 0x2C),
            line: Color32::from_rgb(0x31, 0x34, 0x3A),
            text: Color32::from_rgb(0xE6, 0xE7, 0xEA),
            muted: Color32::from_rgb(0x8B, 0x8F, 0x98),
            accent: Color32::from_rgb(0x3D, 0xDC, 0x84),
            accent_dim: Color32::from_rgb(0x1F, 0x4D, 0x35),
            danger: Color32::from_rgb(0xF0, 0x5A, 0x5A),
        },
        Theme::Light => Palette {
            bg: Color32::from_rgb(0xF3, 0xF4, 0xF6),
            panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            panel_hi: Color32::from_rgb(0xEB, 0xED, 0xF0),
            line: Color32::from_rgb(0xD8, 0xDB, 0xE0),
            text: Color32::from_rgb(0x1C, 0x1E, 0x22),
            muted: Color32::from_rgb(0x69, 0x6E, 0x77),
            accent: Color32::from_rgb(0x12, 0x9E, 0x50),
            accent_dim: Color32::from_rgb(0xCF, 0xF0, 0xDC),
            danger: Color32::from_rgb(0xD0, 0x33, 0x33),
        },
    }
}

pub fn heading() -> FontFamily {
    FontFamily::Name("heading".into())
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut f = FontDefinitions::empty();
    let mut add = |name: &str, bytes: &'static [u8]| {
        f.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("poppins", include_bytes!("../assets/fonts/poppins-regular.ttf"));
    add("poppins-semi", include_bytes!("../assets/fonts/poppins-semibold.ttf"));
    add("fa", include_bytes!("../assets/fonts/fa-solid-900.ttf"));
    f.families.insert(FontFamily::Proportional, vec!["poppins".into(), "fa".into()]);
    f.families.insert(FontFamily::Monospace, vec!["poppins".into(), "fa".into()]);
    f.families.insert(heading(), vec!["poppins-semi".into(), "fa".into()]);
    ctx.set_fonts(f);
}

pub fn apply(ctx: &egui::Context, theme: Theme) {
    let p = palette(theme);
    let mut style = (*ctx.global_style()).clone();
    style.visuals = match theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };
    let v = &mut style.visuals;
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.panel_hi;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.accent_dim;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.widgets.inactive.weak_bg_fill = p.panel_hi;
    v.widgets.inactive.bg_fill = p.panel_hi;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.line);
    v.widgets.hovered.weak_bg_fill = p.line;
    v.widgets.hovered.bg_fill = p.line;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.muted);
    v.widgets.active.weak_bg_fill = p.accent_dim;
    v.widgets.active.bg_fill = p.accent_dim;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    v.slider_trailing_fill = true;
    // Compact: small text, tight spacing.
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(10.5)),
        (TextStyle::Body, FontId::proportional(12.0)),
        (TextStyle::Button, FontId::proportional(12.0)),
        (TextStyle::Monospace, FontId::proportional(12.0)),
        (TextStyle::Heading, FontId::new(13.5, heading())),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    style.spacing.button_padding = Vec2::new(7.0, 2.0);
    style.spacing.slider_width = 104.0;
    style.spacing.interact_size = Vec2::new(36.0, 19.0);
    style.spacing.combo_width = 120.0;
    style.spacing.scroll.bar_width = 6.0;
    ctx.set_global_style(style);
}

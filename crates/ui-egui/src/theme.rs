//! Colours and sizes. The palettes follow the Crafting Apps' look (SolidCraft's Graphite theme for
//! dark, its Cupertino theme for light); Light / Dark / System is egui's theme preference.

use commandercraft_core::settings::{Appearance, ThemeChoice};
use egui::{Color32, Stroke, Theme};

/// Colours the file panels use beyond egui's visuals.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub accent: Color32,
    pub accent_soft: Color32,
    pub hover: Color32,
    /// Background of marked (selected) rows.
    pub marked: Color32,
    pub marked_text: Color32,
    pub border: Color32,
    pub field: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            accent: hex(0x4aa3d8),
            accent_soft: hex(0x2d4a5e),
            hover: hex(0x34373d),
            marked: hex(0x5a4420),
            marked_text: hex(0xffd88a),
            border: hex(0x484d55),
            field: hex(0x1c1e21),
        }
    } else {
        Palette {
            accent: hex(0x007aff),
            accent_soft: hex(0xcfe3ff),
            hover: hex(0xe6e6e9),
            marked: hex(0xffe2a8),
            marked_text: hex(0x6b4200),
            border: hex(0xc6c6c8),
            field: hex(0xffffff),
        }
    }
}

/// The palette for the theme egui currently shows.
pub fn current(ctx: &egui::Context) -> Palette {
    palette(ctx.theme() == Theme::Dark)
}

fn visuals(dark: bool) -> egui::Visuals {
    let p = palette(dark);
    let (panel, header, text) =
        if dark { (hex(0x25272b), hex(0x303338), hex(0xe0e2e5)) } else { (hex(0xececec), hex(0xe2e2e2), hex(0x262628)) };
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = panel;
    v.window_fill = panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.extreme_bg_color = p.field;
    v.faint_bg_color = if dark { hex(0x2a2c31) } else { hex(0xf4f4f5) };
    v.override_text_color = Some(text);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
    v.widgets.inactive.weak_bg_fill = header;
    v.widgets.inactive.bg_fill = header;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.active.weak_bg_fill = p.accent_soft;
    v.widgets.open.weak_bg_fill = header;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_corner_radius = egui::CornerRadius::same(6);
    v.menu_corner_radius = egui::CornerRadius::same(6);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = egui::CornerRadius::same(3);
    }
    v
}

/// Apply theme choice and text sizes.
pub fn apply(ctx: &egui::Context, a: &Appearance) {
    ctx.set_visuals_of(Theme::Dark, visuals(true));
    ctx.set_visuals_of(Theme::Light, visuals(false));
    ctx.set_theme(match a.theme {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    });
    let size = font_size(a);
    ctx.all_styles_mut(|s| {
        use egui::{FontFamily, FontId, TextStyle};
        s.text_styles = [
            (TextStyle::Small, FontId::new(size * 0.82, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(size, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(size, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(size, FontFamily::Monospace)),
            (TextStyle::Heading, FontId::new(size * 1.35, FontFamily::Proportional)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 2.0);
        s.interaction.selectable_labels = false;
    });
    ctx.request_repaint();
}

pub fn font_size(a: &Appearance) -> f32 {
    if a.font_size.is_finite() { a.font_size.clamp(8.0, 32.0) } else { 13.0 }
}

/// Height of a file list row for the text size.
pub fn row_height(a: &Appearance) -> f32 {
    (font_size(a) * 1.45).round()
}

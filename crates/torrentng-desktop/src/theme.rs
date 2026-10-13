//! WebUI design tokens mapped onto egui.
//!
//! The values are the default `tng` palette from `webui/src/themes.ts`, so the
//! desktop shell reads as the same product as the browser UI. Keep the two in
//! sync when the WebUI palette changes (see `docs/DESKTOP.md`).

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle, Theme};
use serde::{Deserialize, Serialize};

/// User-facing theme selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
        }
    }

    pub fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::Light => egui::ThemePreference::Light,
        }
    }
}

/// One resolved palette (the CSS custom properties of the WebUI).
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub surface: Color32,
    pub surface2: Color32,
    pub table_head: Color32,
    pub row: Color32,
    pub row_alt: Color32,
    pub selected: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const DARK: Palette = Palette {
    bg: hex(0x0b1220),
    panel: hex(0x111827),
    surface: hex(0x162033),
    surface2: hex(0x202b42),
    table_head: hex(0x1b2740),
    row: hex(0x101826),
    row_alt: hex(0x152033),
    selected: hex(0x17335f),
    border: hex(0x22324d),
    border_strong: hex(0x40516f),
    text: hex(0xe6edf7),
    muted: hex(0xb7c4d6),
    faint: hex(0x9aa8ba),
    accent: hex(0x4f8cff),
    accent_soft: hex(0x17335f),
    accent_text: hex(0xdbe8ff),
    success: hex(0x22c55e),
    warning: hex(0xf59e0b),
    danger: hex(0xf87171),
};

pub const LIGHT: Palette = Palette {
    bg: hex(0xeef5ff),
    panel: hex(0xfdfefe),
    surface: hex(0xe3edf9),
    surface2: hex(0xcfdef0),
    table_head: hex(0xd7e5f6),
    row: hex(0xffffff),
    row_alt: hex(0xedf4fc),
    selected: hex(0xcfe2ff),
    border: hex(0xbfd0e5),
    border_strong: hex(0x6f86a5),
    text: hex(0x142235),
    muted: hex(0x405168),
    faint: hex(0x687991),
    accent: hex(0x1f62d1),
    accent_soft: hex(0xcfe2ff),
    accent_text: hex(0x0b1f44),
    success: hex(0x15803d),
    warning: hex(0xb45309),
    danger: hex(0xb91c1c),
};

/// Palette for the theme egui currently resolves to.
pub fn palette(ctx: &egui::Context) -> Palette {
    match ctx.theme() {
        Theme::Dark => DARK,
        Theme::Light => LIGHT,
    }
}

fn visuals(p: &Palette, dark: bool) -> egui::Visuals {
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let radius = CornerRadius::same(5);
    v.override_text_color = Some(p.text);
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.row_alt;
    v.code_bg_color = p.surface;
    v.window_stroke = Stroke::new(1.0_f32, p.border);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = radius;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.selection.bg_fill = p.selected;
    // Selected rows keep normal text; egui paints selected text with this stroke.
    v.selection.stroke = Stroke::new(1.0_f32, p.text);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.muted);
    w.noninteractive.corner_radius = radius;
    w.inactive.bg_fill = p.surface;
    w.inactive.weak_bg_fill = p.surface;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.inactive.corner_radius = radius;
    w.hovered.bg_fill = p.surface2;
    w.hovered.weak_bg_fill = p.surface2;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.hovered.corner_radius = radius;
    w.active.bg_fill = p.accent_soft;
    w.active.weak_bg_fill = p.accent_soft;
    w.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.active.fg_stroke = Stroke::new(1.0_f32, p.accent_text);
    w.active.corner_radius = radius;
    w.open.bg_fill = p.surface2;
    w.open.weak_bg_fill = p.surface2;
    w.open.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
    w.open.corner_radius = radius;
    v
}

/// Install both visuals sets and the density/type scale. Call once at start
/// and again when the theme preference changes.
pub fn apply(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_visuals_of(Theme::Dark, visuals(&DARK, true));
    ctx.set_visuals_of(Theme::Light, visuals(&LIGHT, false));
    ctx.set_theme(choice.preference());
    ctx.all_styles_mut(|style| {
        // WebUI density: 12px base text, compact controls, 36px table rows.
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.interact_size.y = 24.0;
        style.spacing.scroll.bar_width = 8.0;
        // Labels must not capture clicks: table rows select on click. The detail
        // dock turns this back on so hashes and paths can be copied.
        style.interaction.selectable_labels = false;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(13.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(13.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(11.0));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(16.0));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(12.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_matches_webui_tokens() {
        assert_eq!(DARK.bg, Color32::from_rgb(0x0b, 0x12, 0x20));
        assert_eq!(DARK.accent, Color32::from_rgb(0x4f, 0x8c, 0xff));
        assert_eq!(LIGHT.accent, Color32::from_rgb(0x1f, 0x62, 0xd1));
        assert_eq!(LIGHT.success, Color32::from_rgb(0x15, 0x80, 0x3d));
    }

    #[test]
    fn dark_and_light_text_contrast_is_readable() {
        fn lum(c: Color32) -> f32 {
            let f = |v: u8| {
                let s = v as f32 / 255.0;
                if s <= 0.03928 {
                    s / 12.92
                } else {
                    ((s + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        }
        for p in [DARK, LIGHT] {
            let (a, b) = (lum(p.text), lum(p.row));
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(ratio >= 7.0, "text/row contrast {ratio}");
        }
    }

    #[test]
    fn theme_choice_roundtrips_through_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            t: ThemeChoice,
        }
        for c in ThemeChoice::ALL {
            let s = toml::to_string(&W { t: c }).unwrap();
            assert_eq!(toml::from_str::<W>(&s).unwrap().t, c);
        }
    }
}

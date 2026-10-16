//! Fonts, colours and small drawing helpers shared by the UI.

use crate::config::ThemePref;
use egui::{Color32, Context, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke, TextStyle, Theme};
use std::sync::Arc;

pub const ACCENT: Color32 = Color32::from_rgb(0x22, 0xB5, 0x63);
pub const ACCENT_STRONG: Color32 = Color32::from_rgb(0x1A, 0x9A, 0x52);
pub const WARN: Color32 = Color32::from_rgb(0xE6, 0x8A, 0x00);
pub const DANGER: Color32 = Color32::from_rgb(0xE0, 0x45, 0x3A);
pub const INFO: Color32 = Color32::from_rgb(0x2F, 0x8F, 0xD8);

pub const SEMIBOLD: &str = "semibold";

/// Segoe UI (sharp, includes Arabic and its joined forms) + Segoe UI Symbol (icons), read
/// from the Windows font folder at runtime so nothing licensed is embedded in the exe.
/// Falls back to egui's built-in fonts if a file is missing.
pub fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let load = |name: &str| std::fs::read(format!("{dir}\\Fonts\\{name}")).ok();
    let mut add = |key: &str, bytes: Option<Vec<u8>>| -> bool {
        match bytes {
            Some(b) => {
                fonts.font_data.insert(key.into(), Arc::new(FontData::from_owned(b)));
                true
            }
            None => false,
        }
    };
    let has_regular = add("segoe", load("segoeui.ttf"));
    let has_sb = add("segoe-sb", load("seguisb.ttf").or_else(|| load("segoeuib.ttf")));
    let has_sym = add("segoe-sym", load("seguisym.ttf"));
    let has_mono = add("consolas", load("consola.ttf"));

    let builtin = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut regular = Vec::new();
    if has_regular {
        regular.push("segoe".to_string());
    }
    if has_sym {
        regular.push("segoe-sym".to_string());
    }
    regular.extend(builtin.iter().cloned());
    let mut semibold = Vec::new();
    if has_sb {
        semibold.push("segoe-sb".to_string());
    }
    semibold.extend(regular.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, regular.clone());
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semibold);
    if has_mono {
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "consolas".into());
    }
    ctx.set_fonts(fonts);
}

fn dark_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(0x17, 0x1A, 0x21);
    v.window_fill = Color32::from_rgb(0x1D, 0x21, 0x2B);
    v.extreme_bg_color = Color32::from_rgb(0x10, 0x13, 0x19);
    v.faint_bg_color = Color32::from_rgb(0x20, 0x25, 0x30);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0xE6, 0xEA, 0xF0));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x2E, 0x35, 0x43));
    v.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(0x1F, 0x24, 0x2E);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x2B, 0x32, 0x40);
    v.widgets.inactive.bg_fill = Color32::from_rgb(0x2B, 0x32, 0x40);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0xE6, 0xEA, 0xF0));
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x37, 0x40, 0x52);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x37, 0x40, 0x52);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x40, 0x4A, 0x5E);
    v.hyperlink_color = Color32::from_rgb(0x6C, 0xB8, 0xF0);
    v.selection.bg_fill = Color32::from_rgb(0x1E, 0x6B, 0x45);
    v.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(0xE9, 0xFF, 0xF1));
    v
}

fn light_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.panel_fill = Color32::from_rgb(0xF3, 0xF5, 0xF8);
    v.window_fill = Color32::WHITE;
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x1A, 0x20, 0x2A));
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0xD9, 0xDE, 0xE6));
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xE6, 0xEA, 0xF0);
    v.widgets.inactive.bg_fill = Color32::from_rgb(0xE6, 0xEA, 0xF0);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x1A, 0x20, 0x2A));
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xDA, 0xDF, 0xE8);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0xDA, 0xDF, 0xE8);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0xCD, 0xD3, 0xDD);
    v.hyperlink_color = Color32::from_rgb(0x16, 0x6F, 0xC0);
    v.selection.bg_fill = Color32::from_rgb(0xC4, 0xEE, 0xD6);
    v.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x0E, 0x4D, 0x2B));
    v
}

/// Apply colours and sizes for both themes, then pick which one is active. With
/// `ThemePref::System` egui follows Windows' light/dark setting live.
pub fn apply_style(ctx: &Context, pref: ThemePref, size: f32, rtl: bool) {
    for (theme, mut visuals) in [(Theme::Dark, dark_visuals()), (Theme::Light, light_visuals())] {
        for w in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            w.corner_radius = 8.into();
        }
        visuals.window_corner_radius = 12.into();
        let mut style = (*ctx.style_of(theme)).clone();
        style.visuals = visuals;
        use FontFamily::{Monospace, Proportional};
        style.text_styles = [
            (TextStyle::Small, FontId::new(size - 2.0, Proportional)),
            (TextStyle::Body, FontId::new(size, Proportional)),
            (TextStyle::Button, FontId::new(size, Proportional)),
            (TextStyle::Heading, FontId::new(size + 6.0, FontFamily::Name(SEMIBOLD.into()))),
            (TextStyle::Monospace, FontId::new(size - 1.0, Monospace)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = size + 12.0;
        // Shaped Arabic must never be re-wrapped by egui (it would split the visual order);
        // long Arabic text is wrapped by `rtl::wrap` instead.
        style.wrap_mode = if rtl { Some(egui::TextWrapMode::Extend) } else { None };
        ctx.set_style_of(theme, style);
    }
    ctx.set_theme(match pref {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Light => egui::ThemePreference::Light,
        ThemePref::Dark => egui::ThemePreference::Dark,
    });
}

/// Secondary text with a fixed, readable contrast (egui's `weak()` fades too much).
pub fn muted_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(0xA0, 0xA9, 0xB7)
    } else {
        Color32::from_rgb(0x55, 0x60, 0x70)
    }
}

pub fn semibold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(FontFamily::Name(SEMIBOLD.into()))
}

/// Header/footer strip colour.
pub fn strip_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(0x12, 0x15, 0x1B)
    } else {
        Color32::WHITE
    }
}

/// Card background.
pub fn card_fill(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(0x1E, 0x23, 0x2D)
    } else {
        Color32::WHITE
    }
}

/// The ArkoClick logo, rendered from assets/logo.svg by `cargo run --example make_icons`.
pub const LOGO_64: &[u8] = include_bytes!("../../assets/logo-64.rgba");
const LOGO_128: &[u8] = include_bytes!("../../assets/logo-128.rgba");

/// Draw the logo (loaded into a texture once, then cached by egui).
pub fn paint_logo(ui: &mut egui::Ui, size: f32) {
    let tex = ui.ctx().data_mut(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new("arkoclick-logo")));
    let tex = tex.unwrap_or_else(|| {
        let img = egui::ColorImage::from_rgba_unmultiplied([128, 128], LOGO_128);
        let t = ui.ctx().load_texture("arkoclick-logo", img, egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("arkoclick-logo"), t.clone()));
        t
    });
    ui.add(egui::Image::new((tex.id(), egui::vec2(size, size))));
}

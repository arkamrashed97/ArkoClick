//! Language-aware building blocks: every piece of text goes through `Tr`, which picks the
//! English or Arabic string and shapes Arabic for display; rows and columns flip direction
//! in Arabic so the whole interface is mirrored.

use super::rtl;
use super::theme::{card_fill, muted_color, semibold, ACCENT, ACCENT_STRONG};
use egui::{Align, Color32, Layout, Response, RichText, Sense, Stroke, TextStyle, TextureHandle, Ui};

#[derive(Clone, Copy)]
pub struct Tr {
    pub ar: bool,
}

impl Tr {
    /// Pick the string for the current language.
    pub fn s<'a>(self, en: &'a str, ar: &'a str) -> &'a str {
        if self.ar {
            ar
        } else {
            en
        }
    }

    /// Display form of any string (shaped + reordered when it contains Arabic).
    pub fn v(self, s: &str) -> String {
        if self.ar || rtl::has_rtl(s) {
            rtl::visual(s, self.ar)
        } else {
            s.to_owned()
        }
    }

    /// Translated, display-ready text.
    pub fn t(self, en: &str, ar: &str) -> RichText {
        RichText::new(self.v(self.s(en, ar)))
    }

    /// Display-ready dynamic text.
    pub fn d(self, s: impl AsRef<str>) -> RichText {
        RichText::new(self.v(s.as_ref()))
    }

    pub fn muted(self, ui: &Ui, en: &str, ar: &str) -> RichText {
        self.t(en, ar).color(muted_color(ui))
    }

    pub fn muted_d(self, ui: &Ui, s: impl AsRef<str>) -> RichText {
        self.d(s).color(muted_color(ui))
    }

    fn row_layout(self) -> Layout {
        if self.ar {
            Layout::right_to_left(Align::Center)
        } else {
            Layout::left_to_right(Align::Center)
        }
    }

    /// A horizontal row in reading direction (right-to-left in Arabic).
    pub fn row<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        let size = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(size, self.row_layout(), add).inner
    }

    /// Like `row`, wrapping onto new lines when it runs out of width.
    pub fn row_wrap<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        let size = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(size, self.row_layout().with_main_wrap(true), add).inner
    }

    /// A vertical stack aligned to the reading side.
    pub fn column<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        ui.with_layout(Layout::top_down(if self.ar { Align::Max } else { Align::Min }), add).inner
    }

    /// Inside a row: put the following widgets at the far end.
    pub fn far<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        let layout = if self.ar { Layout::left_to_right(Align::Center) } else { Layout::right_to_left(Align::Center) };
        ui.with_layout(layout, add).inner
    }

    /// A paragraph that wraps correctly in both languages.
    pub fn para(self, ui: &mut Ui, text: &str, color: Option<Color32>, small: bool) {
        let style = if small { TextStyle::Small } else { TextStyle::Body };
        let color = color.unwrap_or_else(|| ui.visuals().text_color());
        if !self.ar && !rtl::has_rtl(text) {
            let mut rt = RichText::new(text).color(color);
            if small {
                rt = rt.small();
            }
            ui.add(egui::Label::new(rt).wrap());
            return;
        }
        let font = style.resolve(ui.style());
        let width = ui.available_width().max(80.0);
        let lines = rtl::wrap(text, self.ar, width, |s| {
            ui.fonts(|f| f.layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE).size().x)
        });
        let spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 2.0;
        for line in lines {
            let mut rt = RichText::new(line).color(color);
            if small {
                rt = rt.small();
            }
            ui.label(rt);
        }
        ui.spacing_mut().item_spacing.y = spacing;
    }

    pub fn para_t(self, ui: &mut Ui, en: &str, ar: &str, muted: bool) {
        let c = if muted { Some(muted_color(ui)) } else { None };
        self.para(ui, self.s(en, ar), c, false);
    }
}

/// Numbered card used for the step-by-step editors.
pub fn card(ui: &mut Ui, tr: Tr, number: Option<usize>, title: RichText, add: impl FnOnce(&mut Ui)) {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke.color;
    egui::Frame::new()
        .fill(card_fill(ui))
        .stroke(Stroke::new(1.0_f32, stroke))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            tr.column(ui, |ui| {
                tr.row(ui, |ui| {
                    if let Some(n) = number {
                        number_badge(ui, n);
                    }
                    ui.label(semibold(title.text()).size(ui.style().text_styles[&TextStyle::Body].size + 2.0));
                });
                ui.add_space(6.0);
                add(ui);
            });
        });
    ui.add_space(10.0);
}

pub fn number_badge(ui: &mut Ui, n: usize) {
    let size = 26.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, ACCENT);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        n.to_string(),
        egui::FontId::proportional(14.0),
        Color32::WHITE,
    );
}

/// Large, filled call-to-action button.
pub fn big_button(ui: &mut Ui, text: RichText, fill: Color32, min_width: f32) -> Response {
    let size = ui.style().text_styles[&TextStyle::Body].size + 2.0;
    ui.add(
        egui::Button::new(semibold(text.text()).size(size).color(Color32::WHITE))
            .fill(fill)
            .min_size(egui::vec2(min_width, 42.0))
            .corner_radius(10.0),
    )
}

/// A selectable option shown as a pill; filled when selected.
pub fn chip(ui: &mut Ui, selected: bool, text: RichText) -> Response {
    let (fill, color, stroke) = if selected {
        (ACCENT_STRONG, Color32::WHITE, Stroke::new(1.0_f32, ACCENT_STRONG))
    } else {
        (ui.visuals().widgets.inactive.weak_bg_fill, ui.visuals().text_color(), Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color))
    };
    ui.add(egui::Button::new(text.color(color)).fill(fill).stroke(stroke).min_size(egui::vec2(0.0, 34.0)).corner_radius(17.0))
}

/// On/off switch with a label, mirrored in Arabic. Returns true when toggled.
pub fn switch(ui: &mut Ui, tr: Tr, on: &mut bool, label: RichText) -> bool {
    let mut changed = false;
    tr.row(ui, |ui| {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(44.0, 24.0), Sense::click());
        if resp.clicked() {
            *on = !*on;
            changed = true;
        }
        let t = ui.ctx().animate_bool_responsive(resp.id, *on);
        let off_bg = if ui.visuals().dark_mode { Color32::from_rgb(0x46, 0x4F, 0x60) } else { Color32::from_rgb(0xC3, 0xCA, 0xD5) };
        let bg = lerp_color(off_bg, ACCENT, t);
        ui.painter().rect_filled(rect, 12.0, bg);
        let x = egui::lerp((rect.left() + 12.0)..=(rect.right() - 12.0), if tr.ar { 1.0 - t } else { t });
        ui.painter().circle_filled(egui::pos2(x, rect.center().y), 9.0, Color32::WHITE);
        let lr = ui.add(egui::Label::new(label).sense(Sense::click()));
        if lr.clicked() {
            *on = !*on;
            changed = true;
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    });
    changed
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Zoomed picture of the picked spot with a crosshair, or a placeholder.
pub fn spot_picture(ui: &mut Ui, tex: Option<&TextureHandle>, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let p = ui.painter();
    let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
    match tex {
        Some(t) => {
            p.image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            let c = rect.center();
            let red = Color32::from_rgb(0xFF, 0x3B, 0x30);
            p.circle_stroke(c, size * 0.12, Stroke::new(2.0_f32, red));
            p.line_segment([c - egui::vec2(size * 0.22, 0.0), c - egui::vec2(size * 0.12, 0.0)], Stroke::new(2.0_f32, red));
            p.line_segment([c + egui::vec2(size * 0.12, 0.0), c + egui::vec2(size * 0.22, 0.0)], Stroke::new(2.0_f32, red));
            p.line_segment([c - egui::vec2(0.0, size * 0.22), c - egui::vec2(0.0, size * 0.12)], Stroke::new(2.0_f32, red));
            p.line_segment([c + egui::vec2(0.0, size * 0.12), c + egui::vec2(0.0, size * 0.22)], Stroke::new(2.0_f32, red));
        }
        None => {
            p.rect_filled(rect, 8.0, ui.visuals().extreme_bg_color);
            p.text(rect.center(), egui::Align2::CENTER_CENTER, "⌖", egui::FontId::proportional(size * 0.4), muted_color(ui));
        }
    }
    p.rect_stroke(rect, 8.0, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
    resp
}

/// Small coloured dot + text, added into the current row (which sets the direction).
pub fn status_pill(ui: &mut Ui, text: RichText, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 6.0, color);
    ui.label(text.color(color));
}

/// Compact on/off switch without a label (for use inside a row). Returns true when toggled.
pub fn mini_switch(ui: &mut Ui, tr: Tr, on: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(36.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let off_bg = if ui.visuals().dark_mode { Color32::from_rgb(0x46, 0x4F, 0x60) } else { Color32::from_rgb(0xC3, 0xCA, 0xD5) };
    ui.painter().rect_filled(rect, 10.0, lerp_color(off_bg, ACCENT, t));
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), if tr.ar { 1.0 - t } else { t });
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 7.5, Color32::WHITE);
    resp.clicked()
}

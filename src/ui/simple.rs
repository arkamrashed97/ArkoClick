//! "Simple clicker" tab: one spot, clicked repeatedly. Four numbered cards:
//! where, how fast, which button, when to stop.

use super::*;

const PRESETS: [(&str, &str, f64); 4] =
    [("Slow", "بطيء", 1.0), ("Normal", "عادي", 5.0), ("Fast", "سريع", 10.0), ("Very fast", "سريع جدًا", 20.0)];

pub(crate) fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// "One click every 200 ms" in words a non-technical user understands.
pub(crate) fn every_text(tr: Tr, wait_ms: u64, what_en: &str, what_ar: &str) -> String {
    if wait_ms >= 1000 {
        let secs = fmt_num(wait_ms as f64 / 1000.0);
        if tr.ar {
            format!("{what_ar} كل {secs} ثانية")
        } else {
            format!("One {what_en} every {secs} seconds")
        }
    } else if tr.ar {
        format!("{what_ar} كل {wait_ms} ملي ثانية")
    } else {
        format!("One {what_en} every {wait_ms} ms")
    }
}

impl App {
    pub(crate) fn simple_editor(&mut self, ui: &mut egui::Ui, id: u64) {
        let tr = self.tr();
        let mut acts = Vec::new();
        let Some(idx) = self.index_of(id) else { return };
        // A simple task is exactly one click step.
        {
            let c = &mut self.cfg.clickers_mut()[idx];
            if c.steps.is_empty() || !matches!(c.steps[0].action, Action::Click { .. }) {
                c.steps = vec![Step::click(ClickPoint::default(), DEFAULT_WAIT_MS)];
            }
            c.steps.truncate(1);
            c.steps[0].enabled = true;
        }
        self.editor_header(ui, id, &mut acts);

        let thumb = self.cfg.clickers()[idx].steps[0].thumb.clone();
        let tex = self.thumb_tex(ui.ctx(), thumb.as_deref());
        let live = self.live_root(&self.cfg.clickers()[idx]);
        let ghosted = live.map_or(false, ghost::is_ghosted);
        let target = self.cfg.clickers()[idx].target.clone();
        let picked = target.is_set();
        let mut changed = false;

        // 1. Where
        card(ui, tr, Some(1), tr.t("Where should it click?", "أين يضغط؟"), |ui| {
            if !picked {
                tr.para_t(
                    ui,
                    "Press the button below, then click the exact spot on your screen. Your mouse stays free while ArkoClick works.",
                    "اضغط الزر بالأسفل، ثم انقر على المكان المطلوب في الشاشة. يبقى الماوس حرًا لك أثناء عمل ArkoClick.",
                    true,
                );
                ui.add_space(6.0);
                if big_button(ui, tr.t("⌖  Choose the spot on screen", "⌖  اختر المكان على الشاشة"), INFO, 280.0).clicked() {
                    acts.push(Act::Pick(id, PickPurpose::Target));
                }
            } else {
                App::spot_summary(ui, tr, tex.as_ref(), &target, live.is_some(), ghosted, |ui| {
                    if ui.button(tr.t("⌖  Change the spot", "⌖  تغيير المكان")).clicked() {
                        acts.push(Act::Pick(id, PickPurpose::Target));
                    }
                    if ui.button(tr.t("Try one click", "جرّب نقرة واحدة")).clicked() {
                        acts.push(Act::Test(id, 0));
                    }
                });
            }
        });

        {
            let c = &mut self.cfg.clickers_mut()[idx];
            // 2. Speed
            card(ui, tr, Some(2), tr.t("How fast?", "ما السرعة؟"), |ui| {
                let step = &mut c.steps[0];
                let cps = 1000.0 / step.wait_ms.max(1) as f64;
                tr.row_wrap(ui, |ui| {
                    for (en, ar, v) in PRESETS {
                        let selected = (cps - v).abs() < 0.01;
                        let label = if tr.ar { format!("{ar}  ·  {} في الثانية", fmt_num(v)) } else { format!("{en}  ·  {}/sec", fmt_num(v)) };
                        if chip(ui, selected, tr.d(label)).clicked() {
                            step.wait_ms = (1000.0 / v).round() as u64;
                            changed = true;
                        }
                    }
                });
                let mut exact = cps;
                tr.row(ui, |ui| {
                    ui.label(tr.t("Or exactly:", "أو بالضبط:"));
                    if ui.add(egui::DragValue::new(&mut exact).range(0.02..=200.0).speed(0.1).max_decimals(2)).changed() {
                        step.wait_ms = (1000.0 / exact).round().clamp(1.0, 3_600_000.0) as u64;
                        changed = true;
                    }
                    ui.label(tr.t("clicks per second", "نقرة في الثانية"));
                });
                ui.label(tr.muted_d(ui, every_text(tr, step.wait_ms, "click", "نقرة")));
            });

            // 3. Button
            card(ui, tr, Some(3), tr.t("Which mouse button?", "أي زر في الماوس؟"), |ui| {
                if let Action::Click { button, kind, .. } = &mut c.steps[0].action {
                    tr.row_wrap(ui, |ui| {
                        for (b, en, ar) in [
                            (crate::config::MouseButton::Left, "Left button", "الزر الأيسر"),
                            (crate::config::MouseButton::Right, "Right button", "الزر الأيمن"),
                            (crate::config::MouseButton::Middle, "Middle button", "الزر الأوسط"),
                        ] {
                            if chip(ui, *button == b, tr.t(en, ar)).clicked() {
                                *button = b;
                                changed = true;
                            }
                        }
                    });
                    tr.row_wrap(ui, |ui| {
                        use crate::config::ClickKind;
                        if chip(ui, *kind == ClickKind::Single, tr.t("Single click", "نقرة واحدة")).clicked() {
                            *kind = ClickKind::Single;
                            changed = true;
                        }
                        if chip(ui, *kind == ClickKind::Double, tr.t("Double click", "نقرة مزدوجة")).clicked() {
                            *kind = ClickKind::Double;
                            changed = true;
                        }
                    });
                }
            });
        }

        // 4. Stop
        changed |= self.stop_card(ui, idx, 4, "clicks", "نقرة");
        changed |= self.more_options(ui, id, &mut acts);

        if changed {
            self.mark_dirty();
        }
        self.apply_acts(acts);
    }
}

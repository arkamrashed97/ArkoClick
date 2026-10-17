//! "Keyboard" tab: press a key or type a text into a window, repeatedly.
//!
//! Stored as steps: an optional click on the text box (so the cursor is in the right
//! place) followed by one Key or Text step.

use super::simple::{every_text, fmt_num};
use super::*;
use crate::config::vk_label;

const EVERY: [(f64, &str, &str); 5] = [
    (0.5, "Every half second", "كل نصف ثانية"),
    (1.0, "Every second", "كل ثانية"),
    (5.0, "Every 5 seconds", "كل 5 ثوانٍ"),
    (30.0, "Every 30 seconds", "كل 30 ثانية"),
    (60.0, "Every minute", "كل دقيقة"),
];

impl App {
    pub(crate) fn keyboard_editor(&mut self, ui: &mut egui::Ui, id: u64) {
        let tr = self.tr();
        let mut acts = Vec::new();
        let Some(idx) = self.index_of(id) else { return };
        // Normalise: [optional Click] + exactly one Key/Text step at the end.
        {
            let c = &mut self.cfg.clickers_mut()[idx];
            let last_is_input = matches!(c.steps.last().map(|s| &s.action), Some(Action::Key { .. } | Action::Text { .. }));
            if !last_is_input {
                c.steps.push(Step::new(Action::Key { vk: 0x20 }, 1000));
            }
            let focus = c.steps.len() >= 2 && matches!(c.steps[0].action, Action::Click { .. });
            let keep_from = if focus { c.steps.len() - 2 } else { c.steps.len() - 1 };
            if keep_from > 0 {
                c.steps.drain(..keep_from);
            }
            let ai = c.steps.len() - 1;
            c.steps[ai].enabled = true;
        }
        self.editor_header(ui, id, &mut acts);

        let c_ro = &self.cfg.clickers()[idx];
        let has_focus = c_ro.steps.len() == 2;
        let ai = c_ro.steps.len() - 1;
        let thumb = if has_focus { c_ro.steps[0].thumb.clone() } else { None };
        let live = self.live_root(c_ro);
        let ghosted = live.map_or(false, ghost::is_ghosted);
        let target = c_ro.target.clone();
        let picked = target.is_set();
        let capture = self.capture;
        let tex = self.thumb_tex(ui.ctx(), thumb.as_deref());
        let mut changed = false;
        let mut trig = self.cfg.clickers()[idx].trigger.clone();

        let c = &mut self.cfg.clickers_mut()[idx];
        // 1. Where
        card(ui, tr, Some(1), tr.t("Where should it type?", "أين يكتب؟"), |ui| {
            if !picked {
                tr.para_t(
                    ui,
                    "Press the button, then click the box in the other program where the text should go (a chat box, a field, a document…).",
                    "اضغط الزر، ثم انقر على المكان في البرنامج الآخر الذي تريد الكتابة فيه (مربع محادثة، خانة، مستند…).",
                    true,
                );
                ui.add_space(6.0);
                if big_button(ui, tr.t("⌖  Choose where to type", "⌖  اختر مكان الكتابة"), INFO, 280.0).clicked() {
                    acts.push(Act::Pick(id, PickPurpose::KeyboardField));
                }
            } else {
                App::spot_summary(ui, tr, tex.as_ref(), &target, live.is_some(), ghosted, |ui| {
                    if ui.button(tr.t("⌖  Change", "⌖  تغيير")).clicked() {
                        acts.push(Act::Pick(id, PickPurpose::KeyboardField));
                    }
                });
                if has_focus {
                    changed |= switch(
                        ui,
                        tr,
                        &mut c.steps[0].enabled,
                        tr.t("Click that spot before typing (puts the cursor in the box)", "انقر على هذا المكان قبل الكتابة (لوضع المؤشر في الخانة)"),
                    );
                }
                if App::trigger_ui(ui, tr, id, &mut trig, &target, &mut acts) {
                    changed = true;
                }
            }
        });
        if trig != c.trigger {
            c.trigger = trig;
        }

        // 2. What
        card(ui, tr, Some(2), tr.t("What should it do?", "ماذا يفعل؟"), |ui| {
            let is_key = matches!(c.steps[ai].action, Action::Key { .. });
            tr.row_wrap(ui, |ui| {
                if chip(ui, is_key, tr.t("⌨  Press a key", "⌨  اضغط زرًا")).clicked() && !is_key {
                    c.steps[ai].action = Action::Key { vk: 0x20 };
                    changed = true;
                }
                if chip(ui, !is_key, tr.t("✎  Type a text", "✎  اكتب نصًا")).clicked() && is_key {
                    c.steps[ai].action = Action::Text { text: String::new() };
                    changed = true;
                }
            });
            ui.add_space(4.0);
            match &mut c.steps[ai].action {
                Action::Key { vk } => {
                    tr.row(ui, |ui| {
                        ui.label(tr.t("Key:", "الزر:"));
                        let recording = capture == Some(Capture::StepKey(id, ai));
                        let text = if recording {
                            tr.t("Press a key on your keyboard…  (Esc cancels)", "اضغط زرًا في لوحة المفاتيح…  (Esc للإلغاء)").color(WARN)
                        } else {
                            semibold(vk_label(*vk)).size(18.0)
                        };
                        if ui.add(egui::Button::new(text).min_size(egui::vec2(140.0, 38.0))).clicked() {
                            acts.push(Act::Capture(Capture::StepKey(id, ai)));
                        }
                    });
                    ui.label(tr.muted(ui, "Click the button above, then press the key you want.", "انقر على الزر بالأعلى، ثم اضغط الزر الذي تريده."));
                    ui.label(
                        tr.muted(
                            ui,
                            "Letters follow the keyboard language of the other program. To write exact words, use “Type a text”.",
                            "الحروف تتبع لغة لوحة المفاتيح في البرنامج الآخر. لكتابة كلمات محددة استخدم «اكتب نصًا».",
                        )
                        .small(),
                    );
                }
                Action::Text { text } => {
                    let enter = text.ends_with('\n');
                    let body = text.trim_end_matches('\n').to_string();
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            tr.column(ui, |ui| {
                                if body.is_empty() {
                                    ui.label(tr.muted(ui, "No text yet", "لا يوجد نص بعد"));
                                } else {
                                    tr.para(ui, &body, None, false);
                                }
                            });
                        });
                    tr.row(ui, |ui| {
                        if ui.button(tr.t("✎  Write the text", "✎  اكتب النص")).clicked() {
                            acts.push(Act::EditText(TextTarget::StepText(id, ai), body.clone(), true));
                        }
                    });
                    let mut e = enter;
                    if switch(ui, tr, &mut e, tr.t("Press Enter after the text (to send it)", "اضغط Enter بعد النص (لإرساله)")) {
                        *text = if e { format!("{body}\n") } else { body.clone() };
                        changed = true;
                    }
                }
                Action::Click { .. } => {}
            }
            tr.row(ui, |ui| {
                if ui.add_enabled(picked, egui::Button::new(tr.t("Try it once", "جرّبها مرة واحدة"))).clicked() {
                    if has_focus && c.steps[0].enabled {
                        acts.push(Act::Test(id, 0));
                    }
                    acts.push(Act::Test(id, ai));
                }
            });
        });

        // 3. How often
        card(ui, tr, Some(3), tr.t("How often?", "كل كم مرة؟"), |ui| {
            let step = &mut c.steps[ai];
            let secs = step.wait_ms as f64 / 1000.0;
            tr.row_wrap(ui, |ui| {
                for (v, en, ar) in EVERY {
                    if chip(ui, (secs - v).abs() < 0.001, tr.t(en, ar)).clicked() {
                        step.wait_ms = (v * 1000.0) as u64;
                        changed = true;
                    }
                }
            });
            let mut exact = secs;
            tr.row(ui, |ui| {
                ui.label(tr.t("Or every", "أو كل"));
                if ui.add(egui::DragValue::new(&mut exact).range(0.05..=3600.0).speed(0.1).max_decimals(2)).changed() {
                    step.wait_ms = (exact * 1000.0).round().max(50.0) as u64;
                    changed = true;
                }
                ui.label(tr.t("seconds", "ثانية"));
            });
            let what = if matches!(step.action, Action::Key { .. }) { ("key press", "ضغطة زر") } else { ("text", "كتابة") };
            ui.label(tr.muted_d(ui, every_text(tr, step.wait_ms, what.0, what.1)));
            let _ = fmt_num;
        });

        changed |= self.stop_card(ui, idx, 4, "times", "مرة");
        changed |= self.more_options(ui, id, &mut acts);
        if changed {
            self.mark_dirty();
        }
        self.apply_acts(acts);
    }
}

//! "Classic clicker" tab: clicks wherever the mouse cursor currently is on the screen,
//! while the user moves the mouse around normally. Controlled via a global hotkey.

use super::*;
use crate::ui::simple::{every_text, fmt_num};

const PRESETS: [(&str, &str, f64); 4] = [
    ("Slow", "بطيء", 2.0),
    ("Normal", "عادي", 10.0),
    ("Fast", "سريع", 20.0),
    ("Very fast", "سريع جدًا", 50.0),
];

impl App {
    pub(crate) fn classic_editor(&mut self, ui: &mut egui::Ui, id: u64) {
        let tr = self.tr();
        let mut acts = Vec::new();
        let Some(idx) = self.index_of(id) else { return };

        // Ensure classic clicker has exactly one click step
        {
            let c = &mut self.cfg.clickers_mut()[idx];
            if c.steps.is_empty() || !matches!(c.steps[0].action, Action::Click { .. }) {
                c.steps = vec![Step::click(ClickPoint::default(), DEFAULT_WAIT_MS)];
            }
            c.steps.truncate(1);
            c.steps[0].enabled = true;
            c.mode = ClickMode::Foreground;
        }

        self.editor_header(ui, id, &mut acts);

        let mut changed = false;
        let capture = self.capture;

        // 1. Mouse Cursor Location
        card(ui, tr, Some(1), tr.t("Mouse Cursor Location", "موقع مؤشر الماوس"), |ui| {
            tr.para_t(
                ui,
                "Clicks automatically at your mouse pointer's current position. You can move the mouse freely across any program, game, or window while it clicks.",
                "ينقر تلقائيًا في المكان الحالي لمؤشر الماوس أينما حركته على الشاشة. يمكنك تحريك الماوس بحرية عبر أي برنامج أو لعبة أو نافذة أثناء النقر.",
                true,
            );
            ui.add_space(6.0);
            tr.row(ui, |ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 5.0, ACCENT);
                ui.label(semibold(tr.v(tr.s("Follows active mouse pointer", "يتبع مؤشر الماوس النشط"))));
            });
            ui.label(tr.muted(
                ui,
                "It never clicks on ArkoClick itself. Started with the button, it waits 3 seconds so you can move the mouse.",
                "لا ينقر أبدًا على نافذة ArkoClick نفسها. عند التشغيل من الزر ينتظر 3 ثوانٍ لتحريك الماوس.",
            )
            .small());
        });

        // 2. Start / Stop Shortcut Key
        card(ui, tr, Some(2), tr.t("Start / Stop Shortcut", "اختصار التشغيل والإيقاف"), |ui| {
            tr.para_t(
                ui,
                "Press this shortcut key from anywhere on your computer to start or stop the autoclicker immediately.",
                "اضغط هذا الاختصار من أي مكان في جهازك لتشغيل الأوتوكليكر أو إيقافه فورًا أثناء تحريك الماوس.",
                true,
            );
            ui.add_space(8.0);
            let c = &mut self.cfg.clickers_mut()[idx];
            tr.row(ui, |ui| {
                ui.label(tr.t("Shortcut key:", "مفتاح الاختصار:"));
                let recording = capture == Some(Capture::Clicker(id));
                let btn_text = if recording {
                    tr.t("Press any key…  (Esc cancels)", "اضغط أي مفتاح…  (Esc للإلغاء)").color(WARN)
                } else if c.hotkey.is_set() {
                    semibold(tr.v(&c.hotkey.label())).color(Color32::WHITE)
                } else {
                    tr.t("None (click to set)", "لا يوجد (انقر للتعيين)")
                };
                let btn_fill = if recording {
                    Color32::TRANSPARENT
                } else if c.hotkey.is_set() {
                    ACCENT_STRONG
                } else {
                    ui.visuals().widgets.inactive.weak_bg_fill
                };
                let resp = ui.add(
                    egui::Button::new(btn_text)
                        .fill(btn_fill)
                        .min_size(egui::vec2(140.0, 34.0))
                        .corner_radius(8.0),
                );
                if resp.clicked() {
                    acts.push(Act::Capture(Capture::Clicker(id)));
                }

                if c.hotkey.is_set() {
                    if ui.button(tr.t("Clear", "مسح")).clicked() {
                        c.hotkey = Hotkey::NONE;
                        changed = true;
                    }
                    if c.hotkey.vk != 0x76 || c.hotkey.has_modifier() {
                        if ui.button(tr.t("Reset to F7", "استعادة F7")).clicked() {
                            c.hotkey = Hotkey::key(0x76);
                            changed = true;
                        }
                    }
                }
            });
        });

        // 3. Click Speed / Interval
        {
            let c = &mut self.cfg.clickers_mut()[idx];
            card(ui, tr, Some(3), tr.t("How fast?", "ما السرعة؟"), |ui| {
                let step = &mut c.steps[0];
                let cps = 1000.0 / step.wait_ms.max(1) as f64;
                tr.row_wrap(ui, |ui| {
                    for (en, ar, v) in PRESETS {
                        let selected = (cps - v).abs() < 0.01;
                        let label = if tr.ar {
                            format!("{ar}  ·  {} في الثانية", fmt_num(v))
                        } else {
                            format!("{en}  ·  {}/sec", fmt_num(v))
                        };
                        if chip(ui, selected, tr.d(label)).clicked() {
                            step.wait_ms = (1000.0 / v).round() as u64;
                            changed = true;
                        }
                    }
                });
                let mut exact_cps = cps;
                tr.row(ui, |ui| {
                    ui.label(tr.t("Or clicks per second:", "أو نقرات في الثانية:"));
                    if ui
                        .add(egui::DragValue::new(&mut exact_cps).range(0.02..=200.0).speed(0.5).max_decimals(1))
                        .changed()
                    {
                        step.wait_ms = (1000.0 / exact_cps).round().clamp(1.0, 3_600_000.0) as u64;
                        changed = true;
                    }
                    ui.label(tr.t("clicks/sec", "نقرة/ثانية"));
                });
                let mut exact_ms = step.wait_ms;
                tr.row(ui, |ui| {
                    ui.label(tr.t("Or exact interval:", "أو الفاصل الزمني بالملي ثانية:"));
                    if ui
                        .add(egui::DragValue::new(&mut exact_ms).range(1..=3_600_000).speed(5))
                        .changed()
                    {
                        step.wait_ms = exact_ms.max(1);
                        changed = true;
                    }
                    ui.label(tr.t("ms", "ملي ثانية"));
                });
                ui.label(tr.muted_d(ui, every_text(tr, step.wait_ms, "click", "نقرة")));
            });

            // 4. Which mouse button?
            card(ui, tr, Some(4), tr.t("Which mouse button?", "أي زر في الماوس؟"), |ui| {
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

        // 5. When should it stop?
        changed |= self.stop_card(ui, idx, 5, "clicks", "نقرة");

        // 6. More options
        changed |= self.classic_more_options(ui, id, &mut acts);

        if changed {
            self.mark_dirty();
        }
        self.apply_acts(acts);
    }

    fn classic_more_options(&mut self, ui: &mut egui::Ui, id: u64, acts: &mut Vec<Act>) -> bool {
        let tr = self.tr();
        let Some(idx) = self.index_of(id) else { return false };
        let open = self.expanded.contains(&id);
        let mut changed = false;
        let mut toggle = false;
        tr.row(ui, |ui| {
            let label = if open {
                tr.s("▾  Fewer options", "▾  خيارات أقل")
            } else {
                tr.s("▸  More options", "▸  خيارات أكثر")
            };
            if ui.add(egui::Button::new(tr.d(label)).frame(false)).clicked() {
                toggle = true;
            }
        });
        if toggle {
            if open {
                self.expanded.remove(&id);
            } else {
                self.expanded.insert(id);
            }
        }
        if !open {
            return false;
        }

        let c = &mut self.cfg.clickers_mut()[idx];
        // No "start automatically" here: clicking wherever the mouse happens to be at startup
        // would be unsafe.
        c.auto_run = false;
        card(ui, tr, None, tr.t("More options", "خيارات إضافية"), |ui| {
            tr.row(ui, |ui| {
                ui.label(tr.t("Random variation in timing ±", "تغيير عشوائي في التوقيت ±"));
                changed |= ui.add(egui::DragValue::new(&mut c.jitter_ms).range(0..=60_000)).changed();
                ui.label(tr.t("ms", "ملي ثانية"));
            });
            ui.add_space(6.0);
            tr.row_wrap(ui, |ui| {
                if ui.button(tr.t("🗑  Delete this task", "🗑  حذف هذه المهمة").color(DANGER)).clicked() {
                    acts.push(Act::Confirm(Confirm::DeleteClicker(id)));
                }
            });
        });
        changed
    }
}

//! "Advanced clicker" tab: any sequence of clicks, key presses and texts, run in order and
//! repeated, with every option.

use super::*;
use crate::config::{vk_label, ClickKind, MouseButton};
use widgets::mini_switch;

enum StepOp {
    Up(usize),
    Down(usize),
    Remove(usize),
    Add(Action),
}

impl App {
    pub(crate) fn advanced_editor(&mut self, ui: &mut egui::Ui, id: u64) {
        let tr = self.tr();
        let mut acts = Vec::new();
        let Some(idx) = self.index_of(id) else { return };
        if self.cfg.clickers()[idx].steps.is_empty() {
            self.cfg.clickers_mut()[idx].steps.push(Step::click(ClickPoint::default(), DEFAULT_WAIT_MS));
        }
        self.editor_header(ui, id, &mut acts);

        let live = self.live_root(&self.cfg.clickers()[idx]);
        let ghosted = live.map_or(false, ghost::is_ghosted);
        let target = self.cfg.clickers()[idx].target.clone();
        let picked = target.is_set();
        let capture = self.capture;
        let next_step = self.stats.get(&id).filter(|s| s.status.is_active()).map(|s| s.step);
        // Thumbnails for every click step, fetched before borrowing the config mutably.
        let thumbs: Vec<Option<String>> = self.cfg.clickers()[idx].steps.iter().map(|s| s.thumb.clone()).collect();
        let texs: Vec<Option<TextureHandle>> = thumbs.iter().map(|t| self.thumb_tex(ui.ctx(), t.as_deref())).collect();
        let windows = self.windows_cache.clone();
        let mut changed = false;
        let mut ops = Vec::new();

        let c = &mut self.cfg.clickers_mut()[idx];
        // 1. Window
        card(ui, tr, Some(1), tr.t("Which window?", "أي نافذة؟"), |ui| {
            if picked {
                tr.row(ui, |ui| {
                    ui.label(semibold(tr.v(&app_name(&target))));
                    if ghosted {
                        ui.label(tr.t("Hidden, still working", "مخفية وتعمل").color(INFO));
                    } else if live.is_some() {
                        ui.label(tr.t("✓ Window is open", "✓ النافذة مفتوحة").color(ACCENT));
                    } else {
                        ui.label(tr.t("Window is closed right now", "النافذة مغلقة الآن").color(WARN));
                    }
                });
                ui.label(tr.muted_d(ui, truncate(&target.title, 70)).small());
            } else {
                tr.para_t(
                    ui,
                    "Pick a spot inside the program (this also sets the first click), or choose the window from the list.",
                    "اختر مكانًا داخل البرنامج (وسيصبح أول نقرة)، أو اختر النافذة من القائمة.",
                    true,
                );
            }
            tr.row_wrap(ui, |ui| {
                if ui.button(tr.t("⌖  Pick on screen", "⌖  اختر من الشاشة")).clicked() {
                    acts.push(Act::Pick(id, PickPurpose::Target));
                }
                egui::ComboBox::from_id_salt(("win-list", id))
                    .selected_text(tr.t("Choose from open windows…", "اختر من النوافذ المفتوحة…"))
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for w in &windows {
                            if ui.selectable_label(false, tr.d(w.display())).clicked() {
                                c.target = TargetInfo {
                                    title: w.title.clone(),
                                    class: w.class.clone(),
                                    process: w.process.clone(),
                                    hwnd: Some(w.hwnd.0),
                                };
                                let (cw, ch) = window::client_size(w.hwnd).unwrap_or((0, 0));
                                for s in &mut c.steps {
                                    if let Action::Click { point, .. } = &mut s.action {
                                        if point.client_w == 0 {
                                            point.x = cw / 2;
                                            point.y = ch / 2;
                                        }
                                        point.client_w = cw;
                                        point.client_h = ch;
                                    }
                                }
                                changed = true;
                            }
                        }
                    });
            });
        });

        // 2. Steps
        card(ui, tr, Some(2), tr.t("Steps  (run in order, then repeat)", "الخطوات  (تُنفَّذ بالترتيب ثم تتكرر)"), |ui| {
            let n = c.steps.len();
            for i in 0..n {
                let is_next = next_step == Some(i);
                let stroke = if is_next { ACCENT } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
                egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .stroke(egui::Stroke::new(if is_next { 2.0_f32 } else { 1.0_f32 }, stroke))
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        tr.column(ui, |ui| {
                            let step = &mut c.steps[i];
                            // Row 1: on/off, number, kind, tools
                            tr.row_wrap(ui, |ui| {
                                if mini_switch(ui, tr, &mut step.enabled) {
                                    changed = true;
                                }
                                ui.label(semibold(format!("#{}", i + 1)));
                                let kind = step.action.kind_label();
                                for (k, en, ar) in [("Click", "🖱 Click", "🖱 نقرة"), ("Key", "⌨ Key", "⌨ زر"), ("Text", "✎ Text", "✎ نص")] {
                                    if chip(ui, kind == k, tr.t(en, ar)).clicked() && kind != k {
                                        step.action = match k {
                                            "Key" => Action::Key { vk: 0x20 },
                                            "Text" => Action::Text { text: String::new() },
                                            _ => Action::default(),
                                        };
                                        step.thumb = None;
                                        changed = true;
                                    }
                                }
                                if is_next {
                                    ui.label(tr.t("next", "التالية").color(ACCENT).small());
                                }
                                ui.add_space(8.0);
                                if ui.small_button(tr.t("Test", "تجربة")).clicked() {
                                    acts.push(Act::Test(id, i));
                                }
                                if ui.add_enabled(i > 0, egui::Button::new("↑").small()).clicked() {
                                    ops.push(StepOp::Up(i));
                                }
                                if ui.add_enabled(i + 1 < n, egui::Button::new("↓").small()).clicked() {
                                    ops.push(StepOp::Down(i));
                                }
                                if ui.small_button(RichText::new("×").color(DANGER)).clicked() {
                                    ops.push(StepOp::Remove(i));
                                }
                            });
                            // Row 2: details
                            match &mut step.action {
                                Action::Click { point, button, kind } => {
                                    tr.row_wrap(ui, |ui| {
                                        spot_picture(ui, texs.get(i).and_then(|t| t.as_ref()), 56.0);
                                        if ui.button(tr.t("⌖  Pick", "⌖  اختيار")).clicked() {
                                            acts.push(Act::Pick(id, PickPurpose::Step(i)));
                                        }
                                        ui.label("X");
                                        changed |= ui.add(egui::DragValue::new(&mut point.x)).changed();
                                        ui.label("Y");
                                        changed |= ui.add(egui::DragValue::new(&mut point.y)).changed();
                                        for (b, en, ar) in
                                            [(MouseButton::Left, "Left", "أيسر"), (MouseButton::Right, "Right", "أيمن"), (MouseButton::Middle, "Middle", "أوسط")]
                                        {
                                            if chip(ui, *button == b, tr.t(en, ar)).clicked() {
                                                *button = b;
                                                changed = true;
                                            }
                                        }
                                        let double = *kind == ClickKind::Double;
                                        if chip(ui, double, tr.t("Double", "مزدوجة")).clicked() {
                                            *kind = if double { ClickKind::Single } else { ClickKind::Double };
                                            changed = true;
                                        }
                                    });
                                }
                                Action::Key { vk } => {
                                    tr.row(ui, |ui| {
                                        ui.label(tr.t("Key:", "الزر:"));
                                        let recording = capture == Some(Capture::StepKey(id, i));
                                        let text = if recording {
                                            tr.t("Press a key…  (Esc cancels)", "اضغط زرًا…  (Esc للإلغاء)").color(WARN)
                                        } else {
                                            semibold(vk_label(*vk))
                                        };
                                        if ui.add(egui::Button::new(text).min_size(egui::vec2(110.0, 30.0))).clicked() {
                                            acts.push(Act::Capture(Capture::StepKey(id, i)));
                                        }
                                    });
                                }
                                Action::Text { text } => {
                                    tr.row(ui, |ui| {
                                        let preview = text.replace('\n', " ⏎ ");
                                        if preview.trim().is_empty() {
                                            ui.label(tr.muted(ui, "No text yet", "لا يوجد نص بعد"));
                                        } else {
                                            ui.label(tr.d(truncate(&preview, 60)));
                                        }
                                        if ui.button(tr.t("✎  Edit", "✎  تعديل")).clicked() {
                                            acts.push(Act::EditText(TextTarget::StepText(id, i), text.clone(), true));
                                        }
                                    });
                                }
                            }
                            // Row 3: wait
                            tr.row(ui, |ui| {
                                ui.label(tr.t("then wait", "ثم انتظر"));
                                changed |= ui.add(egui::DragValue::new(&mut step.wait_ms).range(1..=3_600_000).speed(5)).changed();
                                ui.label(tr.t("ms", "ملي ثانية"));
                            });
                        });
                    });
                ui.add_space(4.0);
            }
            tr.row_wrap(ui, |ui| {
                if ui.button(tr.t("+  Click step", "+  خطوة نقر")).clicked() {
                    ops.push(StepOp::Add(Action::default()));
                }
                if ui.button(tr.t("+  Key step", "+  خطوة زر")).clicked() {
                    ops.push(StepOp::Add(Action::Key { vk: 0x20 }));
                }
                if ui.button(tr.t("+  Text step", "+  خطوة نص")).clicked() {
                    ops.push(StepOp::Add(Action::Text { text: String::new() }));
                }
            });
        });

        // 3. Options
        card(ui, tr, Some(3), tr.t("Options", "خيارات"), |ui| {
            let mut relative = c.steps.iter().all(|s| match &s.action {
                Action::Click { point, .. } => point.relative,
                _ => true,
            });
            if switch(ui, tr, &mut relative, tr.t("Follow the window when it is resized", "اتبع النافذة عند تغيير حجمها")) {
                for s in &mut c.steps {
                    if let Action::Click { point, .. } = &mut s.action {
                        point.relative = relative;
                    }
                }
                changed = true;
            }
            changed |= switch(
                ui,
                tr,
                &mut c.post_to_root,
                tr.t("Send to the whole window (try if clicks don't register)", "أرسل إلى النافذة كاملة (جرّبه إذا لم تُسجَّل النقرات)"),
            );
            tr.row(ui, |ui| {
                ui.label(tr.t("Random position ±", "موضع عشوائي ±"));
                changed |= ui.add(egui::DragValue::new(&mut c.jitter_px).range(0..=200)).changed();
                ui.label(tr.t("px", "بكسل"));
            });
        });

        // Apply structural step edits.
        if !ops.is_empty() {
            for op in ops {
                match op {
                    StepOp::Up(i) if i > 0 => c.steps.swap(i, i - 1),
                    StepOp::Down(i) if i + 1 < c.steps.len() => c.steps.swap(i, i + 1),
                    StepOp::Remove(i) if c.steps.len() > 1 => {
                        c.steps.remove(i);
                    }
                    StepOp::Remove(_) => acts.push(Act::Toast("A task needs at least one step.", "تحتاج المهمة إلى خطوة واحدة على الأقل.", true)),
                    StepOp::Add(action) => {
                        let wait = c.steps.last().map_or(DEFAULT_WAIT_MS, |s| s.wait_ms);
                        let action = match action {
                            Action::Click { button, kind, .. } => {
                                let base = c
                                    .steps
                                    .iter()
                                    .find_map(|s| match &s.action {
                                        Action::Click { point, .. } => Some(*point),
                                        _ => None,
                                    })
                                    .unwrap_or_default();
                                Action::Click { point: ClickPoint { x: 0, y: 0, ..base }, button, kind }
                            }
                            other => other,
                        };
                        c.steps.push(Step::new(action, wait));
                    }
                    _ => {}
                }
            }
            changed = true;
        }

        changed |= self.stop_card(ui, idx, 4, "times (whole sequence)", "مرة (السلسلة كاملة)");
        changed |= self.more_options(ui, id, &mut acts);
        if changed {
            self.mark_dirty();
        }
        self.apply_acts(acts);
    }
}

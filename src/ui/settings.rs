//! Settings window: language, appearance, startup, profiles, shortcuts, about.

use super::*;
use crate::config::ThemePref;
use crate::win::autostart;

impl App {
    pub(crate) fn settings_window(&mut self, ctx: &Context) {
        let tr = self.tr();
        let mut open = self.show_settings;
        let mut changed = false;
        let mut restyle = false;
        let mut new_lang = None;
        let mut acts = Vec::new();
        let mut quit = false;
        let mut use_profile = None;
        let mut copy_profile = None;
        let capture = self.capture;
        let max_h = (ctx.screen_rect().height() - 80.0).max(260.0);
        let notice = self.engine.notice();

        egui::Window::new(semibold(tr.v(tr.s("Settings", "الإعدادات"))))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(540.0)
            .max_height(max_h)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| {
                    tr.column(ui, |ui| {
                        // Language
                        card(ui, tr, None, tr.t("Language", "اللغة"), |ui| {
                            tr.row(ui, |ui| {
                                if chip(ui, !tr.ar, RichText::new("English")).clicked() && tr.ar {
                                    new_lang = Some(Lang::En);
                                }
                                if chip(ui, tr.ar, tr.d("العربية")).clicked() && !tr.ar {
                                    new_lang = Some(Lang::Ar);
                                }
                            });
                        });

                        // Appearance
                        card(ui, tr, None, tr.t("Appearance", "المظهر"), |ui| {
                            tr.row_wrap(ui, |ui| {
                                for (p, en, ar) in [
                                    (ThemePref::System, "Follow Windows", "حسب ويندوز"),
                                    (ThemePref::Light, "☀  Day", "☀  نهاري"),
                                    (ThemePref::Dark, "☾  Night", "☾  ليلي"),
                                ] {
                                    if chip(ui, self.cfg.theme == p, tr.t(en, ar)).clicked() {
                                        self.cfg.theme = p;
                                        restyle = true;
                                    }
                                }
                            });
                            tr.row(ui, |ui| {
                                ui.label(tr.t("Text size", "حجم الخط"));
                                if ui.add(egui::Slider::new(&mut self.cfg.font_size, 12.0..=20.0).step_by(0.5).fixed_decimals(1)).changed() {
                                    restyle = true;
                                }
                            });
                        });

                        // Startup
                        card(ui, tr, None, tr.t("Startup", "بدء التشغيل"), |ui| {
                            let mut auto = self.cfg.autostart;
                            if switch(ui, tr, &mut auto, tr.t("Start ArkoClick when Windows starts", "شغّل ArkoClick مع بدء تشغيل ويندوز")) {
                                match autostart::set_enabled(auto) {
                                    Ok(()) => {
                                        self.cfg.autostart = auto;
                                        changed = true;
                                    }
                                    Err(e) => {
                                        log::warn!("autostart: {e}");
                                        acts.push(Act::Toast("Couldn't change the startup setting.", "تعذر تغيير إعداد بدء التشغيل.", true));
                                    }
                                }
                            }
                            changed |= switch(
                                ui,
                                tr,
                                &mut self.cfg.start_minimized,
                                tr.t("Start hidden next to the clock (system tray)", "ابدأ مخفيًا بجوار الساعة (شريط النظام)"),
                            );
                            changed |= switch(
                                ui,
                                tr,
                                &mut self.cfg.close_to_tray,
                                tr.t("The close button keeps ArkoClick running near the clock", "زر الإغلاق يبقي ArkoClick يعمل بجوار الساعة"),
                            );
                        });

                        // Profiles
                        card(ui, tr, None, tr.t("Profiles", "الملفات"), |ui| {
                            tr.para_t(
                                ui,
                                "A profile is a separate set of tasks, for example one for work and one for games.",
                                "الملف مجموعة مستقلة من المهام، مثلًا ملف للعمل وآخر للألعاب.",
                                true,
                            );
                            let n = self.cfg.profiles.len();
                            for (i, p) in self.cfg.profiles.iter().enumerate() {
                                let current = i == self.cfg.active_profile;
                                tr.row_wrap(ui, |ui| {
                                    let name = if current { format!("✓  {}", p.name) } else { p.name.clone() };
                                    let mut rt = tr.d(name);
                                    if current {
                                        rt = rt.color(ACCENT);
                                    }
                                    ui.label(semibold(rt.text()).color(if current { ACCENT } else { ui.visuals().text_color() }));
                                    ui.label(tr.muted_d(ui, if tr.ar { format!("({} مهام)", p.clickers.len()) } else { format!("({} tasks)", p.clickers.len()) }).small());
                                    if !current && ui.small_button(tr.t("Use", "استخدام")).clicked() {
                                        use_profile = Some(i);
                                    }
                                    if ui.small_button(tr.t("Rename", "تغيير الاسم")).clicked() {
                                        acts.push(Act::EditText(TextTarget::ProfileName(i), p.name.clone(), false));
                                    }
                                    if ui.small_button(tr.t("Copy", "نسخ")).clicked() {
                                        copy_profile = Some(i);
                                    }
                                    if n > 1 && ui.small_button(tr.t("Delete", "حذف").color(DANGER)).clicked() {
                                        acts.push(Act::Confirm(Confirm::DeleteProfile(i)));
                                    }
                                });
                            }
                            tr.row(ui, |ui| {
                                if ui.button(tr.t("+  New profile", "+  ملف جديد")).clicked() {
                                    acts.push(Act::EditText(TextTarget::NewProfile, String::new(), false));
                                }
                            });
                        });

                        // Shortcuts
                        card(ui, tr, None, tr.t("Keyboard shortcuts", "اختصارات لوحة المفاتيح"), |ui| {
                            for (cap, en, ar, hk) in [
                                (Capture::ToggleAll, "Start / stop everything", "تشغيل / إيقاف الكل", self.cfg.hotkey_toggle_all),
                                (Capture::StopAll, "Emergency stop", "إيقاف طارئ", self.cfg.hotkey_stop_all),
                            ] {
                                tr.row(ui, |ui| {
                                    ui.label(tr.t(en, ar));
                                    tr.far(ui, |ui| {
                                        let text = if capture == Some(cap) {
                                            tr.t("Press the keys…", "اضغط الأزرار…").color(WARN)
                                        } else if hk.is_set() {
                                            RichText::new(hk.label())
                                        } else {
                                            tr.t("None", "لا يوجد")
                                        };
                                        if ui.add(egui::Button::new(text).min_size(egui::vec2(120.0, 30.0))).clicked() {
                                            acts.push(Act::Capture(cap));
                                        }
                                    });
                                });
                            }
                            ui.label(
                                tr.muted(
                                    ui,
                                    "Click a button, then press the shortcut. Backspace removes it. They work even when ArkoClick is hidden.",
                                    "انقر على الزر ثم اضغط الاختصار. Backspace يحذفه. تعمل حتى لو كان ArkoClick مخفيًا.",
                                )
                                .small(),
                            );
                            if !notice.is_empty() {
                                ui.label(tr.t("One of the shortcuts is already used by another program; choose a different one.", "أحد الاختصارات مستخدم في برنامج آخر؛ اختر اختصارًا مختلفًا.").color(DANGER));
                                ui.label(tr.muted_d(ui, &notice).small());
                            }
                        });

                        // Hidden windows
                        let hidden = ghost::count();
                        if hidden > 0 {
                            card(ui, tr, None, tr.t("Hidden windows", "النوافذ المخفية"), |ui| {
                                tr.row(ui, |ui| {
                                    ui.label(tr.d(if tr.ar { format!("{hidden} نافذة مخفية وتعمل") } else { format!("{hidden} hidden and working") }));
                                    if ui.button(tr.t("Show them all", "إظهارها كلها")).clicked() {
                                        ghost::show_all();
                                    }
                                });
                            });
                        }

                        // About
                        card(ui, tr, None, tr.t("About", "حول البرنامج"), |ui| {
                            ui.label(format!("{APP_TITLE} v{APP_VERSION}"));
                            tr.row(ui, |ui| {
                                ui.label(tr.muted(ui, "Developer:", "المطور:"));
                                ui.label(DEVELOPER);
                            });
                            tr.row(ui, |ui| {
                                ui.label(tr.muted(ui, "Email:", "البريد:"));
                                ui.hyperlink_to(DEVELOPER_EMAIL, format!("mailto:{DEVELOPER_EMAIL}"));
                            });
                            tr.row(ui, |ui| {
                                ui.label(tr.muted(ui, "Phone / WhatsApp:", "الهاتف / واتساب:"));
                                ui.hyperlink_to(DEVELOPER_PHONE, format!("https://wa.me/{}", DEVELOPER_PHONE.trim_start_matches('+')));
                            });
                        });

                        tr.row(ui, |ui| {
                            if ui.button(tr.t("Exit ArkoClick", "الخروج من ArkoClick").color(DANGER)).clicked() {
                                quit = true;
                            }
                        });
                    });
                });
            });

        self.show_settings = open;
        if let Some(l) = new_lang {
            self.set_language(ctx, l);
        }
        if restyle {
            theme::apply_style(ctx, self.cfg.theme, self.cfg.font_size, self.tr().ar);
            changed = true;
        }
        if let Some(i) = use_profile {
            self.switch_profile(i);
        }
        if let Some(i) = copy_profile {
            let j = self.cfg.duplicate_profile(i);
            self.switch_profile(j);
        }
        if changed {
            self.mark_dirty();
        }
        self.apply_acts(acts);
        if quit {
            self.quit(ctx);
        }
    }
}

use super::*;

impl DemagogApp {
    pub(super) fn show_top_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        theme: ThemeMode,
    ) {
        let compact = ui.available_width() < 1200.0 || self.settings.ui_font_size >= 20.0;
        if compact {
            self.show_compact_top_toolbar(ui, ctx, theme);
        } else {
            self.show_full_top_toolbar(ui, ctx, theme);
        }
    }

    pub(super) fn show_full_top_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        theme: ThemeMode,
    ) {
        egui::Frame::side_top_panel(ui.style())
            .fill(ui.style().visuals.panel_fill)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(!self.is_busy, egui::Button::new("Открыть книгу"))
                        .on_hover_text("Открыть TXT, FB2, FB2.ZIP, EPUB, DOCX, ZIP или MD (Ctrl+O)")
                        .clicked()
                    {
                        self.open_book_dialog(ctx.clone());
                    }

                    let enabled_dictionaries =
                        self.dictionaries.iter().filter(|d| d.meta.enabled).count();
                    let can_run = !self.is_busy && enabled_dictionaries > 0;
                    let run_disabled_hint = if enabled_dictionaries == 0 {
                        "Включите хотя бы один словарь"
                    } else {
                        "Дождитесь завершения текущей операции"
                    };

                    let (primary_fill, primary_text) = palette::primary_button(theme);
                    let run_btn = egui::Button::new(
                        RichText::new("⚡ Обработать").strong().color(primary_text),
                    )
                    .fill(primary_fill);

                    let run_response = ui
                        .add_enabled(can_run, run_btn)
                        .on_hover_text("Обработать исходный текст (Ctrl+Enter)")
                        .on_disabled_hover_text(run_disabled_hint);
                    if run_response.clicked() {
                        self.capture_diff_on_run = false;
                        self.start_processing(ctx.clone(), false);
                    }

                    if ui
                        .add_enabled(can_run, egui::Button::new("🔬 С журналом замен"))
                        .on_hover_text("Собрать журнал замен (Ctrl+Shift+Enter)")
                        .on_disabled_hover_text(run_disabled_hint)
                        .clicked()
                    {
                        self.capture_diff_on_run = true;
                        self.start_processing(ctx.clone(), true);
                    }

                    ui.separator();
                    let prev_fmt = self.settings.stress_format;
                    ui.add_enabled_ui(!self.is_busy, |ui| {
                        let format_label =
                            format!("Формат: {} ▼", self.settings.stress_format.short_label());
                        ui.menu_button(format_label, |ui| {
                            for fmt in [
                                StressFormat::PlusBeforeVowel,
                                StressFormat::AcuteAccent,
                                StressFormat::CleanYoOnly,
                            ] {
                                if ui
                                    .selectable_value(
                                        &mut self.settings.stress_format,
                                        fmt,
                                        fmt.label(),
                                    )
                                    .clicked()
                                {
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Формат вывода ударений");
                    });
                    if self.settings.stress_format != prev_fmt {
                        if !self.reformat_result() {
                            self.settings.stress_format = prev_fmt;
                        }
                        self.save_settings();
                    }

                    ui.separator();
                    if ui
                        .add_enabled(
                            !self.document.processed_text.is_empty() && !self.is_busy,
                            egui::Button::new("💾 Сохранить результат"),
                        )
                        .on_hover_text("Сохранить результат в TXT (Ctrl+S)")
                        .on_disabled_hover_text("Сначала обработайте текст")
                        .clicked()
                    {
                        self.save_processed_txt();
                    }
                    let can_export =
                        !self.document.processed_text.trim().is_empty() && !self.is_busy;
                    if ui
                        .add_enabled(can_export, egui::Button::new("📦 Экспорт чанков"))
                        .on_hover_text("Экспортировать чанки в JSON (Ctrl+E)")
                        .on_disabled_hover_text("Результат пуст — экспортировать нечего")
                        .clicked()
                    {
                        self.export_chunks_json();
                    }
                    ui.separator();
                    let theme_menu = ui.menu_button(theme_icon(theme), |ui| {
                        for candidate in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Warm] {
                            let label = format!("{} {}", theme_icon(candidate), candidate.label());
                            if ui.selectable_label(candidate == theme, label).clicked() {
                                self.settings.set_theme(candidate);
                                apply_style_settings(ui.ctx(), &self.settings);
                                self.save_settings();
                                ui.close();
                            }
                        }
                    });
                    theme_menu.response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("Тема: {}", theme.label()),
                        )
                    });
                    theme_menu.response.on_hover_text("Тема оформления");
                });
                ui.add_space(4.0);
            });
    }

    pub(super) fn show_compact_top_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        theme: ThemeMode,
    ) {
        egui::Frame::side_top_panel(ui.style())
            .fill(ui.style().visuals.panel_fill)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    let open_response = ui
                        .add_enabled(!self.is_busy, egui::Button::new("Открыть"))
                        .on_hover_text(
                            "Открыть TXT, FB2, FB2.ZIP, EPUB, DOCX, ZIP или MD (Ctrl+O)",
                        );
                    open_response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            !self.is_busy,
                            "Открыть книгу",
                        )
                    });
                    if open_response.clicked() {
                        self.open_book_dialog(ctx.clone());
                    }
                    let enabled_dictionaries =
                        self.dictionaries.iter().filter(|d| d.meta.enabled).count();
                    let can_run = !self.is_busy && enabled_dictionaries > 0;
                    let run_disabled_hint = if enabled_dictionaries == 0 {
                        "Включите хотя бы один словарь"
                    } else {
                        "Дождитесь завершения текущей операции"
                    };
                    let (primary_fill, primary_text) = palette::primary_button(theme);
                    let run_btn =
                        egui::Button::new(RichText::new("Обработать").strong().color(primary_text))
                            .fill(primary_fill);
                    let run_response = ui
                        .add_enabled(can_run, run_btn)
                        .on_hover_text("Обработать исходный текст (Ctrl+Enter)")
                        .on_disabled_hover_text(run_disabled_hint);
                    if run_response.clicked() {
                        self.capture_diff_on_run = false;
                        self.start_processing(ctx.clone(), false);
                    }

                    let prev_fmt = self.settings.stress_format;
                    ui.menu_button("Ещё", |ui| {
                        if ui.button("Открыть книгу").clicked() {
                            self.open_book_dialog(ctx.clone());
                            ui.close();
                        }
                        if ui
                            .add_enabled(can_run, egui::Button::new("С журналом замен"))
                            .on_disabled_hover_text(run_disabled_hint)
                            .clicked()
                        {
                            self.capture_diff_on_run = true;
                            self.start_processing(ctx.clone(), true);
                            ui.close();
                        }
                        ui.separator();
                        ui.add_enabled_ui(!self.is_busy, |ui| {
                            for fmt in [
                                StressFormat::PlusBeforeVowel,
                                StressFormat::AcuteAccent,
                                StressFormat::CleanYoOnly,
                            ] {
                                if ui
                                    .selectable_value(
                                        &mut self.settings.stress_format,
                                        fmt,
                                        fmt.label(),
                                    )
                                    .clicked()
                                {
                                    ui.close();
                                }
                            }
                        });
                        ui.separator();
                        if ui
                            .add_enabled(
                                !self.document.processed_text.is_empty() && !self.is_busy,
                                egui::Button::new("Сохранить результат"),
                            )
                            .on_disabled_hover_text("Сначала обработайте текст")
                            .clicked()
                        {
                            self.save_processed_txt();
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                !self.document.processed_text.trim().is_empty() && !self.is_busy,
                                egui::Button::new("Экспорт чанков"),
                            )
                            .on_disabled_hover_text("Результат пуст")
                            .clicked()
                        {
                            self.export_chunks_json();
                            ui.close();
                        }
                        ui.separator();
                        for candidate in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Warm] {
                            let label = format!("{} {}", theme_icon(candidate), candidate.label());
                            if ui.selectable_label(candidate == theme, label).clicked() {
                                self.settings.set_theme(candidate);
                                apply_style_settings(ui.ctx(), &self.settings);
                                self.save_settings();
                                ui.close();
                            }
                        }
                    });
                    if self.settings.stress_format != prev_fmt {
                        if !self.reformat_result() {
                            self.settings.stress_format = prev_fmt;
                        } else {
                            self.save_settings();
                        }
                    }
                });
                ui.add_space(4.0);
            });
    }

    pub(super) fn show_status_bar(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        egui::Panel::bottom("bottom_status").show_inside(ui, |ui| {
            let active_count = self.dictionaries.iter().filter(|d| d.meta.enabled).count();
            let active_rules: usize = self
                .dictionaries
                .iter()
                .filter(|d| d.meta.enabled)
                .map(|d| d.meta.rule_count)
                .sum();
            let counters = format!(
                "Активно: {}/{} словарей ({} правил)",
                active_count,
                self.dictionaries.len(),
                format_num(active_rules)
            );

            let font = self.settings.ui_font_size;
            let estimate = counters.chars().count() as f32 * font * 0.6
                + self.status_line.chars().count() as f32 * font * 0.6
                + if self.dic_load_errors.is_empty() {
                    0.0
                } else {
                    140.0
                }
                + if self.is_busy { 300.0 } else { 40.0 };
            // Wrap only when the row is actually tight; otherwise keep the
            // status and counters on a single line.
            if ui.available_width() < estimate {
                ui.vertical(|ui| {
                    self.show_status_left(ui, theme);
                    self.show_status_counters(ui, theme, &counters);
                });
            } else {
                ui.horizontal(|ui| {
                    self.show_status_left(ui, theme);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.show_status_counters(ui, theme, &counters);
                    });
                });
            }
        });
    }

    pub(super) fn show_sidebar(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        theme: ThemeMode,
    ) {
        // Left sidebar: Dictionary list with semantic tags & descriptions
        egui::Panel::left("dictionaries_sidebar")
            .default_size(450.0)
            .min_size(340.0)
            .show_inside(ui, |ui| {
                ui.add_space(6.0);
                ui.heading("📚 Словари");
                ui.add_space(6.0);
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(!self.is_busy, egui::Button::new("🔄 Обновить"))
                            .on_hover_text("Перезагрузить словари с диска")
                            .clicked()
                        {
                            self.reload_dictionaries_async(ctx.clone());
                        }
                        if ui
                            .add_enabled(!self.is_busy, egui::Button::new("Папка..."))
                            .clicked()
                        {
                            self.choose_dictionary_folder(ctx.clone());
                        }
                        ui.separator();
                        let selected = ui
                            .add_enabled_ui(!self.is_busy, presets_menu)
                            .inner;
                        if let Some(preset) = selected {
                            self.apply_preset(preset);
                        }

                        if ui
                            .checkbox(&mut self.settings.group_dictionaries_by_type, "Группы")
                            .on_hover_text(
                                "Группировать словари по типу. Порядок обработки остаётся по нумерации файлов конвейера",
                            )
                            .changed()
                        {
                            self.save_settings();
                        }
                    });
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label = ui.label("Фильтр:");
                    let width = (ui.available_width() - 24.0).max(80.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.dic_search_filter)
                            .hint_text("Имя, описание или тип")
                            .desired_width(width),
                    )
                    .labelled_by(label.id);
                    if !self.dic_search_filter.is_empty() {
                        let clear = ui.small_button("✖").on_hover_text("Сбросить фильтр");
                        clear.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                "Сбросить фильтр словарей",
                            )
                        });
                        if clear.clicked() {
                            self.dic_search_filter.clear();
                        }
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                let filter_low = self.dic_search_filter.to_lowercase();
                let mut changed_any = false;
                let visible: Vec<usize> = self
                    .dictionaries
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| {
                        filter_low.is_empty()
                            || d.meta.name.to_lowercase().contains(&filter_low)
                            || d.meta.description.to_lowercase().contains(&filter_low)
                            || d.meta.tag.label().to_lowercase().contains(&filter_low)
                    })
                    .map(|(idx, _)| idx)
                    .collect();

                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if visible.is_empty() {
                            ui.add_space(8.0);
                            if !self.dic_search_filter.trim().is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "Ничего не найдено по запросу «{}».",
                                        self.dic_search_filter.trim()
                                    ))
                                    .weak(),
                                );
                                if ui.button("Сбросить фильтр").clicked() {
                                    self.dic_search_filter.clear();
                                }
                            } else if self.dictionaries.is_empty() {
                                ui.label(
                                    RichText::new(if self.is_busy {
                                        "Загрузка словарей..."
                                    } else {
                                        "Словари не загружены. Укажите папку со словарями."
                                    })
                                    .weak(),
                                );
                                if !self.is_busy && ui.button("Указать папку...").clicked() {
                                    self.choose_dictionary_folder(ctx.clone());
                                }
                                if !self.dic_load_errors.is_empty() {
                                    ui.label(
                                        RichText::new(format!(
                                            "Ошибок загрузки: {}",
                                            self.dic_load_errors.len()
                                        ))
                                        .color(palette::warning_text(theme))
                                        .strong(),
                                    )
                                    .on_hover_text(self.dic_load_errors.join("\n"));
                                }
                            } else {
                                ui.label(RichText::new("Все словари скрыты фильтром.").weak());
                            }
                        } else if self.settings.group_dictionaries_by_type {
                            for tag in DictionaryTag::ALL {
                                let mut indices: Vec<usize> = visible
                                    .iter()
                                    .copied()
                                    .filter(|&i| self.dictionaries[i].meta.tag == tag)
                                    .collect();
                                if indices.is_empty() {
                                    continue;
                                }
                                indices.sort_by_cached_key(|&i| {
                                    dictionary_display_key(&self.dictionaries[i].meta.name)
                                });

                                let total = indices.len();
                                let active = indices
                                    .iter()
                                    .filter(|&&i| self.dictionaries[i].meta.enabled)
                                    .count();
                                let group_key = tag.label();
                                let collapsed = self
                                    .settings
                                    .collapsed_dictionary_groups
                                    .iter()
                                    .any(|g| g == group_key);
                                let desired_open = !filter_low.is_empty() || !collapsed;
                                let group_id = ui.make_persistent_id(("dic_group", group_key));
                                let mut state =
                                    egui::collapsing_header::CollapsingState::load_with_default_open(
                                        ui.ctx(),
                                        group_id,
                                        desired_open,
                                    );
                                state.set_open(desired_open);

                                let header = state.show_header(ui, |ui| {
                                    let (state_label, state_color) = if active == 0 {
                                        ("нет", palette::weak_text(theme))
                                    } else if active == total {
                                        ("все", palette::success_text(theme))
                                    } else {
                                        ("часть", palette::warning_text(theme))
                                    };
                                    let mut state_response = ui
                                        .add_enabled_ui(!self.is_busy, |ui| {
                                            ui.small_button(
                                                RichText::new(state_label).color(state_color),
                                            )
                                            .on_hover_text("Включить или отключить всю группу")
                                        })
                                        .inner;
                                    state_response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            !self.is_busy,
                                            format!(
                                                "Группа {group_key}: {state_label}. Включить или отключить всю группу"
                                            ),
                                        )
                                    });
                                    ui.add_space(4.0);
                                    let tag_response = ui.label(
                                        RichText::new(format!("[{}]", group_key))
                                            .color(palette::tag_color(tag, theme))
                                            .strong(),
                                    );
                                    state_response = state_response.labelled_by(tag_response.id);
                                    if state_response.clicked() {
                                        let enable = active < total;
                                        for &i in &indices {
                                            self.dictionaries[i].meta.enabled = enable;
                                        }
                                        changed_any = true;
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(format!("{active}/{total}"))
                                                    .small()
                                                    .weak(),
                                            );
                                        },
                                    );
                                });

                                let (toggle_response, _header, _body) =
                                    header.body_unindented(|ui| {
                                        ui.add_space(4.0);
                                        for idx in indices {
                                            dictionary_card(
                                                ui,
                                                &mut self.dictionaries[idx].meta,
                                                &mut changed_any,
                                                false,
                                                theme,
                                                !self.is_busy,
                                            );
                                        }
                                        ui.add_space(6.0);
                                    });

                                if filter_low.is_empty() && toggle_response.clicked() {
                                    if desired_open {
                                        if !self
                                            .settings
                                            .collapsed_dictionary_groups
                                            .iter()
                                            .any(|g| g == group_key)
                                        {
                                            self.settings
                                                .collapsed_dictionary_groups
                                                .push(group_key.to_string());
                                        }
                                    } else {
                                        self.settings
                                            .collapsed_dictionary_groups
                                            .retain(|g| g != group_key);
                                    }
                                    self.save_settings();
                                }
                            }
                        } else {
                            for idx in &visible {
                                dictionary_card(
                                    ui,
                                    &mut self.dictionaries[*idx].meta,
                                    &mut changed_any,
                                    true,
                                    theme,
                                    !self.is_busy,
                                );
                            }
                        }
                    });

                if changed_any {
                    self.sync_disabled_dictionaries();
                    self.mark_result_stale();
                    self.save_settings();
                }
            });
    }
    pub(super) fn show_central_workspace(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        theme: ThemeMode,
    ) {
        // Central workspace
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::RawText,
                    format!("📝 Исходный текст ({})", format_num(self.raw_char_count)),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::ProcessedText,
                    format!("✅ Результат ({})", format_num(self.processed_char_count)),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::Chunks,
                    format!("✂ Чанки ({})", self.document.chunks.len()),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::DiffAndStats,
                    format!(
                        "🔬 Журнал замен ({})",
                        format_num(self.report.total_replacements)
                    ),
                );
                ui.selectable_value(&mut self.active_tab, ActiveTab::Settings, "⚙ Настройки");
            });
            ui.separator();
            if self.is_busy {
                ui.label(
                    RichText::new("Идёт обработка — текст временно доступен только для чтения.")
                        .weak(),
                );
                ui.add_space(4.0);
            }

            match self.active_tab {
                ActiveTab::RawText => self.show_raw_text_tab(ui),
                ActiveTab::ProcessedText => self.show_processed_text_tab(ui, theme),
                ActiveTab::Chunks => self.show_chunks_tab(ui, theme),
                ActiveTab::DiffAndStats => self.show_diff_tab(ui, theme),
                ActiveTab::Settings => self.show_settings_tab(ui, ctx),
            }
        });
    }

    fn show_raw_text_tab(&mut self, ui: &mut egui::Ui) {
        let editor_font = FontId::new(self.settings.editor_font_size, FontFamily::Monospace);
        let line_height = editor_line_height(self.settings.editor_font_size);
        ui.add(egui::Label::new(RichText::new(&self.document.title).strong()).truncate());
        ui.horizontal_wrapped(|ui| {
                        let can_mutate = text_mutations_enabled(self.is_busy);
                        if ui
                            .button("💾 Сохранить исходный текст")
                            .on_hover_text("Сохранить исходный текст в TXT (Ctrl+Shift+S)")
                            .clicked()
                        {
                            self.save_source_txt();
                        }
                        let replace = ui
                            .add_enabled(
                                can_mutate,
                                egui::Button::new("📋 Заменить из буфера"),
                            )
                            .on_disabled_hover_text("Дождитесь завершения обработки");
                        if replace.clicked() {
                            match arboard::Clipboard::new().and_then(|mut cb| cb.get_text()) {
                                Ok(txt) if !txt.is_empty() => {
                                    if !self.has_unsaved_work()
                                        || confirm_destructive(
                                            "Заменить исходный текст содержимым буфера?\nНесохранённые изменения будут потеряны.",
                                        )
                                    {
                                        self.document.raw_text = txt;
                                        self.document.title = "Из буфера обмена".to_string();
                                        self.document.source_path = None;
                                        self.refresh_raw_char_count();
                                        self.source_dirty = true;
                                        self.mark_result_stale();
                                        self.find_dirty = true;
                                        self.find_last_edit = Instant::now();
                                    }
                                }
                                Ok(_) => {
                                    self.set_status(StatusLevel::Warning, "Буфер обмена пуст");
                                }
                                Err(err) => {
                                    self.set_status(
                                        StatusLevel::Error,
                                        format!("Не удалось прочитать буфер обмена: {err}"),
                                    );
                                }
                            }
                        }
                        let clear = ui
                            .add_enabled(can_mutate, egui::Button::new("🗑 Очистить"))
                            .on_disabled_hover_text("Дождитесь завершения обработки");
                        if clear.clicked() {
                            let had_text = text_would_be_lost(&self.document.raw_text);
                            let needs_confirm =
                                self.source_dirty || self.document.source_path.is_some();
                            if !had_text
                                || !needs_confirm
                                || confirm_destructive(
                                    "Очистить исходный текст?\nНесохранённые изменения будут потеряны.",
                                )
                            {
                                self.document.raw_text.clear();
                                self.raw_char_count = 0;
                                self.document.source_path = None;
                                self.source_dirty = false;
                                self.mark_result_stale();
                                self.find_dirty = true;
                                self.find_last_edit = Instant::now();
                            }
                        }
                    });
        if self.find_open {
            self.show_find_bar(ui);
        }
        ui.add_space(4.0);
        let find_ranges = if self.find_open {
            self.find_matches.ranges.clone()
        } else {
            Vec::new()
        };
        let find_current = if self.find_open {
            Some(self.find_current)
        } else {
            None
        };
        let mut layouter = editor_layouter(
            editor_font.clone(),
            ui.visuals().text_color(),
            line_height,
            &find_ranges,
            find_current,
        );
        ScrollArea::vertical().show(ui, |ui| {
            let output = ui
                .add_enabled_ui(text_mutations_enabled(self.is_busy), |ui| {
                    egui::TextEdit::multiline(&mut self.document.raw_text)
                        .desired_width(f32::INFINITY)
                        .desired_rows(28)
                        .margin(egui::Margin::symmetric(14, 12))
                        .layouter(&mut layouter)
                        .show(ui)
                })
                .inner;
            if output.response.changed() {
                self.source_dirty = true;
                self.refresh_raw_char_count();
                self.mark_result_stale();
                self.find_dirty = true;
                self.find_last_edit = Instant::now();
            }
            self.scroll_to_find_match(ui, &output);
        });
    }

    fn show_processed_text_tab(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        let editor_font = FontId::new(self.settings.editor_font_size, FontFamily::Monospace);
        let line_height = editor_line_height(self.settings.editor_font_size);
        ui.horizontal_wrapped(|ui| {
                        let has_result = !self.document.processed_text.is_empty();
                        if ui
                            .add_enabled(
                                has_result,
                                egui::Button::new("📋 Копировать весь результат"),
                            )
                            .clicked()
                        {
                            let text = self.document.processed_text.clone();
                            self.copy_to_clipboard(
                                &text,
                                "Весь обработанный текст скопирован в буфер обмена!".to_string(),
                                None,
                            );
                        }
                        if ui
                            .add_enabled(
                                has_result && text_mutations_enabled(self.is_busy),
                                egui::Button::new("🔄 Заменить исходный текст"),
                            )
                            .on_hover_text(
                                "Результат останется на месте, текущий исходный текст будет заменён",
                            )
                            .on_disabled_hover_text("Дождитесь завершения обработки")
                            .clicked()
                        {
                            let had_text = text_would_be_lost(&self.document.raw_text);
                            if !had_text
                                || confirm_destructive(
                                    "Заменить исходный текст результатом?\nТекущий исходный текст будет потерян.",
                                )
                            {
                                self.document.raw_text = self.document.processed_text.clone();
                                self.raw_char_count = self.processed_char_count;
                                self.raw_pipeline_output.clear();
                                self.source_dirty = true;
                                self.mark_result_stale();
                                self.find_dirty = true;
                                self.find_last_edit = Instant::now();
                            }
                        }
                    });
        self.show_stale_banner(ui, theme);
        if self.document.processed_text.is_empty() {
            ui.label(
                            RichText::new(
                                "Результат появится после обработки. Если не знаете, что выбрать, начните с пресета «Все (полный +)» в меню «Пресеты».",
                            )
                            .weak(),
                        );
            ui.add_space(4.0);
        }
        if self.find_open {
            self.show_find_bar(ui);
        }
        ui.add_space(4.0);
        let find_ranges = if self.find_open {
            self.find_matches.ranges.clone()
        } else {
            Vec::new()
        };
        let find_current = if self.find_open {
            Some(self.find_current)
        } else {
            None
        };
        let mut layouter = editor_layouter(
            editor_font.clone(),
            ui.visuals().text_color(),
            line_height,
            &find_ranges,
            find_current,
        );
        ScrollArea::vertical().show(ui, |ui| {
            let output = ui
                .add_enabled_ui(text_mutations_enabled(self.is_busy), |ui| {
                    egui::TextEdit::multiline(&mut self.document.processed_text)
                        .desired_width(f32::INFINITY)
                        .desired_rows(28)
                        .margin(egui::Margin::symmetric(14, 12))
                        .layouter(&mut layouter)
                        .show(ui)
                })
                .inner;
            if output.response.changed() {
                self.result_state.mark_edited();
                if self.document.processed_text.trim().is_empty() {
                    self.rechunk_result();
                }
                self.refresh_processed_char_count();
                self.find_dirty = true;
                self.find_last_edit = Instant::now();
            }
            self.scroll_to_find_match(ui, &output);
        });
    }

    fn show_chunks_tab(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        let editor_font = FontId::new(self.settings.editor_font_size, FontFamily::Monospace);
        let line_height = editor_line_height(self.settings.editor_font_size);
        ui.horizontal_wrapped(|ui| {
            let label = ui.label("Макс. длина чанка (символов):");
            let slider = ui
                .add_enabled(
                    !self.is_busy,
                    egui::Slider::new(&mut self.settings.chunk_config.max_chars, 150..=3000)
                        .step_by(50.0),
                )
                .labelled_by(label.id);
            self.apply_chunk_config_change(ui, slider.changed());
            let paragraphs_changed = ui
                .add_enabled_ui(!self.is_busy, |ui| {
                    ui.checkbox(
                        &mut self.settings.chunk_config.preserve_paragraphs,
                        "Не склеивать абзацы",
                    )
                })
                .inner
                .changed();
            if paragraphs_changed {
                self.rechunk_result();
                self.save_settings();
            }
        });
        self.show_stale_banner(ui, theme);
        if self.result_state.is_chunks_stale() {
            ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(
                                    "⚠ Результат изменён вручную: чанки пока не учитывают правки. Перед экспортом они будут пересобраны.",
                                )
                                .color(palette::warning_text(theme))
                                .strong(),
                            );
                            if ui.button("🔄 Пересобрать чанки").clicked() {
                                self.rechunk_result();
                            }
                        });
            ui.add_space(4.0);
        }
        ui.separator();

        let chunk_total = self.document.chunks.len();
        let text_width = (ui.available_width() - 44.0).max(120.0);
        let chars_per_line = (text_width / (editor_font.size * 0.65)).max(8.0);
        let estimated_lines =
            ((self.settings.chunk_config.max_chars as f32 / chars_per_line).ceil()).max(1.0);
        let row_height = line_height * estimated_lines
            + 64.0
            + ui.spacing().interact_size.y
            + ui.spacing().item_spacing.y * 3.0;

        if chunk_total == 0 {
            ui.label("Нажмите «Обработать», чтобы нарезать текст на чанки по границам предложений. Чанк можно скопировать отдельно, а все — экспортировать в JSON для ComfyUI.");
        } else {
            let mut copied_status: Option<(StatusLevel, String)> = None;
            ScrollArea::vertical().id_salt("chunks_scroll").show_rows(
                ui,
                row_height,
                chunk_total,
                |ui, rows| {
                    for idx in rows {
                        let chunk = &self.document.chunks[idx];
                        let is_copied = self.last_copied_chunk == Some(idx);
                        let mut frame = egui::Frame::group(ui.style());
                        if is_copied {
                            frame = frame
                                .stroke(egui::Stroke::new(2.0_f32, palette::success_text(theme)));
                        }

                        frame.inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "Чанк #{}/{} ({} симв.)",
                                        idx + 1,
                                        chunk_total,
                                        chunk.chars().count()
                                    ))
                                    .strong(),
                                );
                                if is_copied {
                                    ui.label(
                                        RichText::new("✓ Скопировано")
                                            .color(palette::success_text(theme))
                                            .strong(),
                                    );
                                }
                                if ui.button("📋 Копировать чанк").clicked() {
                                    match arboard::Clipboard::new()
                                        .and_then(|mut cb| cb.set_text(chunk.clone()))
                                    {
                                        Ok(()) => {
                                            self.last_copied_chunk = Some(idx);
                                            copied_status = Some((
                                                StatusLevel::Success,
                                                format!(
                                                    "Чанк #{} скопирован в буфер обмена",
                                                    idx + 1
                                                ),
                                            ));
                                        }
                                        Err(err) => {
                                            copied_status = Some((
                                                StatusLevel::Error,
                                                format!("Не удалось скопировать чанк: {err}"),
                                            ));
                                        }
                                    }
                                }
                            });
                            ui.add_space(4.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(chunk)
                                        .font(editor_font.clone())
                                        .line_height(Some(line_height)),
                                )
                                .selectable(true),
                            );
                        });
                        ui.add_space(6.0);
                    }
                },
            );
            if let Some((level, text)) = copied_status {
                self.set_status(level, text);
            }
        }
    }

    fn show_diff_tab(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        let editor_font = FontId::new(self.settings.editor_font_size, FontFamily::Monospace);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "Общее время: {} мс | Всего срабатываний: {}",
                    self.report.total_ms,
                    format_num(self.report.total_replacements)
                ))
                .strong(),
            );
            ui.separator();
            let filter_label = ui.label("Поиск по заменам:");
            ui.add(egui::TextEdit::singleline(&mut self.diff_filter).desired_width(240.0))
                .labelled_by(filter_label.id);
        });
        self.show_stale_banner(ui, theme);
        ui.separator();

        if !self.diff_cache_valid || self.diff_filter.to_lowercase() != self.diff_filter_cache {
            self.refresh_diff_visible();
        }
        let diff_total = self.diff_visible.len();
        egui::CollapsingHeader::new("⏱ Тайминги по словарям")
            .default_open(false)
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt("diff_timings_scroll")
                    .max_height(180.0)
                    .show(ui, |ui| {
                        egui::Grid::new("timings_grid")
                            .striped(true)
                            .min_col_width(90.0)
                            .show(ui, |ui| {
                                ui.label(RichText::new("Словарь").strong());
                                ui.label(RichText::new("Тип").strong());
                                ui.label(RichText::new("Правил").strong());
                                ui.label(RichText::new("Время (мс)").strong());
                                ui.label(RichText::new("Замен").strong());
                                ui.end_row();

                                for s in &self.report.steps {
                                    ui.label(&s.dictionary_name);
                                    ui.label(s.kind.badge());
                                    ui.label(format_num(s.rule_count));
                                    ui.label(format!("{} мс", s.elapsed_ms));
                                    ui.label(format_num(s.replacements_count));
                                    ui.end_row();
                                }
                            });
                    });
            });

        ui.add_space(14.0);
        ui.heading("🔍 Журнал сработавших правил");
        if self.report.events.is_empty() {
            ui.label("Для записи детальных примеров каждого сработавшего правила нажмите кнопку «🔬 С журналом замен» на верхней панели.");
        } else if self.diff_visible.is_empty() {
            ui.label("Ничего не найдено по фильтру замен.");
        } else {
            ScrollArea::both().id_salt("diff_scroll").show_rows(
                ui,
                (ui.text_style_height(&TextStyle::Body)
                    + editor_font.size * 1.45
                    + ui.spacing().item_spacing.y)
                    .max(ui.spacing().interact_size.y + ui.spacing().item_spacing.y),
                diff_total,
                |ui, rows| {
                    egui::Grid::new("diff_events_grid")
                        .start_row(0)
                        .striped(true)
                        .show(ui, |ui| {
                            for &event_index in &self.diff_visible[rows.clone()] {
                                let ev = &self.report.events[event_index];
                                ui.label(&ev.dictionary_name);
                                ui.add(
                                    egui::Label::new(RichText::new(&ev.rule_summary).monospace())
                                        .wrap_mode(egui::TextWrapMode::Extend)
                                        .selectable(true),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&ev.before_snippet)
                                            .color(palette::diff_before(theme))
                                            .font(editor_font.clone()),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Extend)
                                    .selectable(true),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&ev.after_snippet)
                                            .color(palette::diff_after(theme))
                                            .font(editor_font.clone()),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Extend)
                                    .selectable(true),
                                );
                                ui.end_row();
                            }
                        });
                },
            );
        }
    }

    fn show_settings_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ScrollArea::vertical().id_salt("settings_scroll").show(ui, |ui| {
            ui.heading("⚙ Настройки интерфейса и обработки");
            ui.add_space(10.0);

            let mut style_changed = false;

            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([24.0, 14.0])
                .show(ui, |ui| {
                    let ui_label = ui.label("Размер шрифта интерфейса (UI):");
                    if ui
                        .add(
                            egui::Slider::new(&mut self.settings.ui_font_size, 12.0..=28.0)
                                .suffix(" пт"),
                        )
                        .labelled_by(ui_label.id)
                        .changed()
                    {
                        style_changed = true;
                    }
                    ui.end_row();

                    let editor_label = ui.label("Размер шрифта текста книги / чанков:");
                    if ui
                        .add(
                            egui::Slider::new(&mut self.settings.editor_font_size, 12.0..=36.0)
                                .suffix(" пт"),
                        )
                        .labelled_by(editor_label.id)
                        .changed()
                    {
                        style_changed = true;
                    }
                    ui.end_row();

                    let chunk_label = ui.label("Макс. размер чанка:");
                    let chunk_response = ui
                        .add_enabled(
                            !self.is_busy,
                            egui::Slider::new(&mut self.settings.chunk_config.max_chars, 150..=3000)
                                .step_by(50.0)
                                .suffix(" симв."),
                        )
                        .labelled_by(chunk_label.id);
                    self.apply_chunk_config_change(ui, chunk_response.changed());
                    ui.end_row();

                    ui.label("Папка словарей:");
                    ui.horizontal_wrapped(|ui| {
                        ui.label(self.settings.dic_folder.display().to_string());
                        if ui
                            .add_enabled(!self.is_busy, egui::Button::new("Изменить..."))
                            .on_disabled_hover_text("Доступно после завершения обработки")
                            .clicked()
                        {
                            self.choose_dictionary_folder(ctx.clone());
                        }
                    });
                    ui.end_row();

                    ui.label("Проверка словарей:");
                    ui.horizontal_wrapped(|ui| {
                        let response = ui
                            .add_enabled(!self.is_busy, egui::Button::new("🔎 Проверить словари"))
                            .on_hover_text(
                                "Порядок этапов, 22 эталонные фразы и аудит фонетических кандидатов",
                            )
                            .on_disabled_hover_text("Доступно после завершения обработки");
                        if response.clicked() {
                            self.check_dictionaries_async(ctx.clone());
                        }
                        ui.label(
                            RichText::new("Порядок, эталоны и аудит фонетики")
                                .small()
                                .weak(),
                        );
                    });
                    ui.end_row();
                });

            if style_changed {
                apply_style_settings(ctx, &self.settings);
                self.save_settings();
            }

            ui.add_space(12.0);
            ui.separator();
            let reset = ui.add_enabled(!self.is_busy, egui::Button::new("🔄 Сбросить настройки..."));
            if reset
                .on_disabled_hover_text("Доступно после завершения обработки")
                .clicked()
                && confirm_destructive(
                    "Сбросить настройки интерфейса и обработки?\nТекст останется, выбор словарей вернётся к стандартному.",
                )
            {
                self.settings = reset_settings_preserving_folder(&self.settings);
                for d in &mut self.dictionaries {
                    d.meta.enabled = true;
                }
                self.sync_disabled_dictionaries();
                self.save_settings();
                self.mark_result_stale();
                apply_style_settings(ctx, &self.settings);
                self.set_status(StatusLevel::Success, "Настройки сброшены");
            }

            ui.add_space(18.0);
            ui.separator();

            ui.horizontal_wrapped(|ui| {
                let theme_label = ui.label("Тема:");
                let theme = self.settings.theme();
                for candidate in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Warm] {
                    if ui
                        .selectable_label(theme == candidate, candidate.label())
                        .labelled_by(theme_label.id)
                        .clicked()
                    {
                        self.settings.set_theme(candidate);
                        apply_style_settings(ui.ctx(), &self.settings);
                        self.save_settings();
                    }
                }
            });

            ui.add_space(10.0);
            egui::CollapsingHeader::new("⌨ Горячие клавиши")
                .default_open(true)
                .show(ui, |ui| {
                    egui::Grid::new("hotkeys_grid")
                        .striped(true)
                        .min_col_width(180.0)
                        .show(ui, |ui| {
                            for keys in [
                                ("Ctrl+O", "Открыть книгу"),
                                ("Ctrl+Enter", "Обработать"),
                                ("Ctrl+Shift+Enter", "Обработать с журналом замен"),
                                ("Ctrl+S", "Сохранить результат"),
                                ("Ctrl+Shift+S", "Сохранить исходник"),
                                ("Ctrl+E", "Экспорт чанков"),
                                ("Ctrl+F", "Открыть поиск"),
                                ("F3", "Следующее совпадение"),
                                ("Shift+F3", "Предыдущее совпадение"),
                                ("Esc", "Отмена или закрыть поиск"),
                            ] {
                                ui.label(keys.0);
                                ui.label(keys.1);
                                ui.end_row();
                            }
                        });
                });

            ui.add_space(8.0);
            egui::CollapsingHeader::new("ℹ О программе")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(format!("Demagog-RS {APP_VERSION}"));
                    ui.label("Процессор текста и словарей для TTS и ComfyUI.");
                    ui.label("Лицензия проекта — MIT. Словари сохраняют лицензии исходных источников.");
                });

            ui.add_space(8.0);
            ui.add_space(18.0);
            ui.separator();
            egui::CollapsingHeader::new("ℹ Справка по категориям словарей")
                .default_open(false)
                .show(ui, |ui| {
                    for text in [
                        "• [ЧИСЛА И ДАТЫ] — переводит числа и даты в слова (1995 → тысяча девятьсот девяносто пятом).",
                        "• [Ё-ФИКАЦИЯ] — расставляет букву «ё» (все/всё и 337 тыс. слов из edge_Yo_.dic). Знаков «+» не ставит.",
                        "• [ОМОГРАФЫ (загл.)] — находит омографы по контексту и выделяет ударную гласную заглавной буквой (зАмок / замОк, зЕмли).",
                        "• [ЗАГЛАВНЫЕ → '+'] (словари 69 и 70) — переводит заглавные ударные буквы омографов в знак «+» (зЕмли → з+емли).",
                        "• [УДАРЕНИЯ '+'] и [ОРФОЭПИЯ '+' (2.9М)] (словари 35, 67, 71..75, 95.NEW) — сами напрямую ставят знак «+» в обычных словах. Именно из-за 95.NEW и 67 в тексте остаются «+», если выключить только словарь 69.",
                        "💡 Совет: если не нужны знаки «+», выберите в меню «Пресеты» вариант «Числа + Ё (без ударений)» или формат выхода «Без ударений — только Ё» сверху.",
                    ] {
                        ui.add(egui::Label::new(text).selectable(true));
                    }
                });
        });
    }
}
fn dictionary_card(
    ui: &mut egui::Ui,
    d: &mut DictionaryMeta,
    changed_any: &mut bool,
    show_tag: bool,
    theme: ThemeMode,
    interactive: bool,
) {
    let mut details = d.description.clone();
    details.push_str(&format!(
        "\n{} • {} правил • {}",
        d.kind.badge(),
        format_num(d.rule_count),
        d.encoding
    ));
    if let (Some(ms), Some(repl)) = (d.last_ms, d.last_replacements) {
        details.push_str(&format!("\nПоследний запуск: {ms} мс, замен: {repl}"));
    }

    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            // Force equal card width: otherwise the frame shrinks
            // to the longest wrapped line and the right edge is ragged.
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let mut checkbox_response =
                    ui.add_enabled(interactive, egui::Checkbox::new(&mut d.enabled, ""));

                if show_tag {
                    ui.label(
                        RichText::new(format!("[{}]", d.tag.label()))
                            .color(palette::tag_color(d.tag, theme))
                            .strong(),
                    );
                }

                if d.outputs_plus_stress {
                    ui.label(
                        RichText::new("[ставит +]")
                            .color(palette::warning_text(theme))
                            .strong(),
                    );
                }

                let name_response = ui
                    .label(RichText::new(&d.name).strong())
                    .on_hover_text(&details);
                checkbox_response = checkbox_response.labelled_by(name_response.id);
                if checkbox_response.changed() {
                    *changed_any = true;
                }

                ui.label(
                    RichText::new(format!("{} правил", format_num(d.rule_count)))
                        .small()
                        .weak(),
                );

                if !d.warnings.is_empty() {
                    ui.label(
                        RichText::new(format!("⚠ {}", d.warnings.len()))
                            .small()
                            .color(palette::warning_text(theme))
                            .strong(),
                    )
                    .on_hover_text(format!(
                        "Пропущено правил: {}\n{}",
                        d.warnings.len(),
                        d.warnings.join("\n")
                    ));
                }

                if let (Some(ms), Some(repl)) = (d.last_ms, d.last_replacements) {
                    let col = if repl > 0 {
                        palette::success_text(theme)
                    } else {
                        palette::weak_text(theme)
                    };
                    ui.label(
                        RichText::new(format!("{ms} мс"))
                            .small()
                            .color(col)
                            .strong(),
                    )
                    .on_hover_text(format!("Замен: {repl}"));
                }
            });
        });
    ui.add_space(3.0);
}

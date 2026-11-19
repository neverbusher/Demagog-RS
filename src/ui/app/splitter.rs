use super::*;
use crate::services::book_splitter::{self, BookPart};

#[derive(Debug, Clone, Default)]
pub(super) struct SplitterState {
    pub preview: Vec<BookPart>,
    pub names: Vec<String>,
    pub has_preview: bool,
    pub stale: bool,
    pub save_pending: bool,
}

impl DemagogApp {
    pub(super) fn show_splitter_tab(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        if self.document.raw_text.trim().is_empty() {
            ui.label("Откройте книгу (Ctrl+O), чтобы нарезать её на части.");
            return;
        }
        if !self.splitter.has_preview && !self.splitter.stale {
            self.run_split_preview();
        }

        let mut changed = false;
        ui.add_enabled_ui(!self.is_busy, |ui| {
            egui::Grid::new("splitter_settings")
                .num_columns(2)
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    changed |= ui
                        .checkbox(&mut self.settings.split.by_size, "По размеру")
                        .changed();
                    ui.add_enabled_ui(self.settings.split.by_size, |ui| {
                        changed |= ui
                            .add(
                                egui::Slider::new(
                                    &mut self.settings.split.max_chars,
                                    1_000..=500_000,
                                )
                                .step_by(1000.0)
                                .text("символов"),
                            )
                            .changed();
                    });
                    ui.end_row();

                    changed |= ui
                        .checkbox(&mut self.settings.split.by_keywords, "По ключевым словам")
                        .changed();
                    ui.add_enabled_ui(self.settings.split.by_keywords, |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut self.settings.split.keywords)
                                    .hint_text("Глава, Часть, Пролог, Эпилог")
                                    .desired_width(320.0),
                            )
                            .changed();
                    });
                    ui.end_row();

                    changed |= ui
                        .checkbox(
                            &mut self.settings.split.by_blank_lines,
                            "По двум пустым строкам",
                        )
                        .changed();
                    ui.label("");
                    ui.end_row();

                    changed |= ui
                        .checkbox(
                            &mut self.settings.split.by_all_caps,
                            "По строкам ВСЕ ЗАГЛАВНЫЕ",
                        )
                        .changed();
                    ui.label("");
                    ui.end_row();

                    let toc_available = !self.document.headings.is_empty();
                    changed |= ui
                        .add_enabled_ui(toc_available, |ui| {
                            ui.checkbox(&mut self.settings.split.by_toc, "По оглавлению (FB2/EPUB)")
                        })
                        .inner
                        .changed();
                    if toc_available {
                        ui.label("");
                    } else {
                        ui.label(RichText::new("В этой книге нет структуры глав").weak());
                    }
                    ui.end_row();

                    let min_label = ui.label("Мин. размер части (0 = выкл):");
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut self.settings.split.min_chars, 0..=10_000)
                                .step_by(100.0),
                        )
                        .labelled_by(min_label.id)
                        .changed();
                    ui.end_row();

                    changed |= ui
                        .checkbox(
                            &mut self.settings.split.remove_heading,
                            "Удалять строку-заголовок из части",
                        )
                        .changed();
                    ui.label("");
                    ui.end_row();

                    let tpl_label = ui.label("Шаблон имени файла:");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.settings.split.template)
                                .desired_width(320.0),
                        )
                        .labelled_by(tpl_label.id)
                        .changed();
                    ui.end_row();

                    ui.label("Переменные: %Number%, %FileName%, %FirstLine%, %Header%");
                    ui.end_row();

                    let start_label = ui.label("Стартовый номер:");
                    changed |= ui
                        .add(egui::Slider::new(
                            &mut self.settings.split.start_number,
                            1..=999,
                        ))
                        .labelled_by(start_label.id)
                        .changed();
                    ui.end_row();

                    changed |= ui
                        .checkbox(
                            &mut self.settings.split.add_number,
                            "Добавлять порядковый номер",
                        )
                        .changed();
                    ui.label("");
                    ui.end_row();

                    ui.label("Папка назначения:");
                    ui.horizontal(|ui| {
                        let dir_text = self
                            .settings
                            .split
                            .output_dir
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "не выбрана".to_string());
                        ui.label(RichText::new(dir_text).weak());
                        if ui.button("📂 Выбрать…").clicked() {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                self.settings.split.output_dir = Some(dir);
                                changed = true;
                            }
                        }
                    });
                    ui.end_row();

                    changed |= ui
                        .checkbox(
                            &mut self.settings.split.subfolder,
                            "Подпапка с именем книги",
                        )
                        .changed();
                    ui.end_row();

                    changed |= ui
                        .checkbox(
                            &mut self.settings.split.overwrite,
                            "Перезаписывать существующие файлы",
                        )
                        .changed();
                    ui.label("");
                    ui.end_row();
                });
        });
        self.persist_split_change(ui, changed);

        let example_part = self.splitter.preview.first().cloned().unwrap_or(BookPart {
            title: None,
            text: "Глава первая\nТекст части…".to_string(),
        });
        let example = book_splitter::part_file_name(
            &self.settings.split,
            &example_part,
            0,
            self.splitter.preview.len().max(1),
            &self.split_source_name(),
        );
        ui.label(RichText::new(format!("Пример имени: {example}.txt")).weak());
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.is_busy, egui::Button::new("🔍 Тест"))
                .clicked()
            {
                self.run_split_preview();
            }
            let can_export = !self.is_busy
                && self.settings.split.output_dir.is_some()
                && book_splitter::validate_config(&self.settings.split).is_ok();
            if ui
                .add_enabled(can_export, egui::Button::new("💾 Нарезать"))
                .clicked()
            {
                self.export_split_parts();
            }
        });
        ui.separator();

        if self.splitter.stale && self.splitter.has_preview {
            ui.label(
                RichText::new(
                    "⚠ Настройки или текст изменены — нажмите «Тест», чтобы обновить превью.",
                )
                .color(palette::warning_text(theme))
                .strong(),
            );
        }
        if self.splitter.has_preview {
            let total = self.splitter.preview.len();
            ui.label(RichText::new(format!("Частей: {total}")).strong());
            if total <= 1 {
                ui.label(
                    RichText::new(
                        "Текст не разделился — проверьте способы деления и ключевые слова.",
                    )
                    .color(palette::warning_text(theme)),
                );
            }
            let editor_font = FontId::new(self.settings.editor_font_size, FontFamily::Monospace);
            ScrollArea::vertical()
                .id_salt("splitter_preview")
                .show(ui, |ui| {
                    for (idx, part) in self.splitter.preview.iter().enumerate() {
                        let header = format!(
                            "{} — {} симв.",
                            self.splitter.names[idx],
                            format_num(part.text.chars().count())
                        );
                        egui::CollapsingHeader::new(RichText::new(header).strong())
                            .id_salt(idx)
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&part.text).font(editor_font.clone()),
                                    )
                                    .selectable(true),
                                );
                            });
                    }
                });
        }
    }

    fn persist_split_change(&mut self, ui: &egui::Ui, changed: bool) {
        if changed {
            self.splitter.stale = true;
            self.splitter.save_pending = true;
        }
        if self.splitter.save_pending && !ui.ctx().input(|i| i.pointer.primary_down()) {
            self.splitter.save_pending = false;
            self.save_settings();
        }
    }

    fn split_source_name(&self) -> String {
        match &self.document.source_path {
            Some(path) => crate::services::batch::output_stem(path),
            None => self.document.title.clone(),
        }
    }

    fn run_split_preview(&mut self) {
        if let Err(err) = book_splitter::validate_config(&self.settings.split) {
            self.set_status(StatusLevel::Warning, err);
            return;
        }
        let parts = book_splitter::split_book(
            &self.document.raw_text,
            &self.document.headings,
            &self.settings.split,
        );
        self.store_split_preview(parts);
    }

    fn store_split_preview(&mut self, parts: Vec<BookPart>) {
        let source_name = self.split_source_name();
        self.splitter.names = parts
            .iter()
            .enumerate()
            .map(|(index, part)| {
                book_splitter::part_file_name(
                    &self.settings.split,
                    part,
                    index,
                    parts.len(),
                    &source_name,
                )
            })
            .collect();
        self.splitter.preview = parts;
        self.splitter.has_preview = true;
        self.splitter.stale = false;
    }

    fn export_split_parts(&mut self) {
        if let Err(err) = book_splitter::validate_config(&self.settings.split) {
            self.set_status(StatusLevel::Error, err);
            return;
        }
        let parts = book_splitter::split_book(
            &self.document.raw_text,
            &self.document.headings,
            &self.settings.split,
        );
        if parts.is_empty() {
            self.set_status(StatusLevel::Warning, "Нечего сохранять: текст пуст");
            return;
        }
        if parts.len() > 200
            && !confirm_destructive(&format!(
                "Будет создано {} файлов в папке назначения. Продолжить?",
                parts.len()
            ))
        {
            return;
        }
        let source_name = self.split_source_name();
        match book_splitter::export_parts(&parts, &self.settings.split, &source_name) {
            Ok(summary) => {
                self.store_split_preview(parts);
                if summary.errors.is_empty() {
                    self.set_status(
                        StatusLevel::Success,
                        format!(
                            "Сохранено {} частей в {}",
                            summary.saved,
                            summary.dir.display()
                        ),
                    );
                } else {
                    self.set_status(
                        StatusLevel::Warning,
                        format!(
                            "Сохранено {} частей, ошибок: {}. {}",
                            summary.saved,
                            summary.errors.len(),
                            summary.errors.join("; ")
                        ),
                    );
                }
            }
            Err(err) => self.set_status(StatusLevel::Error, err),
        }
    }
}

use super::*;
use crate::domain::{AppError, AppResult};
use crate::io::book_reader::can_read_book;
use crate::services::batch::{BatchInput, BatchOutcome};
use crate::services::book_splitter::{self, BookPart};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SplitterMode {
    #[default]
    Single,
    Batch,
}

#[derive(Debug, Clone)]
pub(super) enum BatchSplitStatus {
    Queued,
    Running,
    Done(usize),
    Partial { saved: usize, error: String },
}

#[derive(Debug, Clone, Default)]
pub(super) struct BatchQueueState {
    pub files: Vec<BatchInput>,
    pub statuses: Vec<BatchSplitStatus>,
    pub running: bool,
    pub summary: Option<BatchOutcome>,
    pub cancel_requested: bool,
    pub error: Option<String>,
    pub warning: Option<String>,
}

impl BatchQueueState {
    pub(super) fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if !can_read_book(&path) || self.files.iter().any(|file| file.path == path) {
                continue;
            }
            let base_dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
            self.files.push(BatchInput { path, base_dir });
            self.statuses.push(BatchSplitStatus::Queued);
        }
    }

    pub(super) fn clear(&mut self) {
        self.files.clear();
        self.statuses.clear();
        self.running = false;
        self.summary = None;
        self.cancel_requested = false;
        self.error = None;
        self.warning = None;
    }

    pub(super) fn can_start(&self, is_busy: bool, output_dir_set: bool) -> bool {
        !is_busy && !self.files.is_empty() && output_dir_set
    }

    pub(super) fn display_name(&self, index: usize) -> String {
        self.files[index]
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub(super) fn done_count(&self) -> usize {
        self.statuses
            .iter()
            .filter(|status| matches!(status, BatchSplitStatus::Done(_)))
            .count()
    }

    pub(super) fn failed_count(&self) -> usize {
        self.statuses
            .iter()
            .filter(|status| matches!(status, BatchSplitStatus::Partial { .. }))
            .count()
    }

    pub(super) fn saved_count(&self) -> usize {
        self.statuses
            .iter()
            .map(|status| match status {
                BatchSplitStatus::Done(saved) | BatchSplitStatus::Partial { saved, .. } => *saved,
                _ => 0,
            })
            .sum()
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct SplitterState {
    pub mode: SplitterMode,
    pub preview: Vec<BookPart>,
    pub names: Vec<String>,
    pub has_preview: bool,
    pub stale: bool,
    pub save_pending: bool,
    pub batch_queue: BatchQueueState,
}

impl SplitterState {
    pub(super) fn add_batch_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        self.batch_queue.add_files(paths);
        self.mode = SplitterMode::Batch;
    }

    pub(super) fn reset_preview(&mut self) {
        self.preview.clear();
        self.names.clear();
        self.has_preview = false;
        self.stale = false;
    }
}

impl DemagogApp {
    pub(super) fn show_splitter_tab(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        self.show_splitter_mode_switch(ui);
        match self.splitter.mode {
            SplitterMode::Single => self.show_splitter_single(ui, theme),
            SplitterMode::Batch => self.show_splitter_batch_menu(ui, theme),
        }
    }

    fn show_splitter_mode_switch(&mut self, ui: &mut egui::Ui) {
        let mut selected = self.splitter.mode;
        ui.horizontal(|ui| {
            ui.label("Режим:");
            ui.add_enabled_ui(!self.worker.is_busy, |ui| {
                ui.radio_value(&mut selected, SplitterMode::Single, "Одна книга");
                ui.radio_value(&mut selected, SplitterMode::Batch, "Пакет");
            });
        });
        if self.worker.is_busy {
            ui.weak("Переключение режима доступно после завершения работы.");
        }
        if selected != self.splitter.mode {
            self.splitter.mode = selected;
        }
    }

    fn show_splitter_single(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        if self.editor.document.raw_text.trim().is_empty() {
            ui.label("Откройте книгу (Ctrl+O), чтобы нарезать её на части.");
            return;
        }
        if !self.splitter.has_preview && !self.splitter.stale {
            self.run_split_preview(ui.ctx().clone());
        }

        let changed = self.show_splitter_settings(ui);
        self.persist_split_change(ui, changed);

        let example = self.splitter.names.first().cloned().unwrap_or_else(|| {
            let example_part = self.splitter.preview.first().cloned().unwrap_or(BookPart {
                title: None,
                text: "Глава первая\nТекст части…".to_string(),
            });
            book_splitter::part_file_name(
                &self.settings.split,
                &example_part,
                0,
                self.splitter.preview.len().max(1),
                &self.split_source_name(),
            )
        });
        ui.label(RichText::new(format!("Пример имени: {example}.txt")).weak());
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.worker.is_busy, egui::Button::new("🔍 Тест"))
                .clicked()
            {
                self.run_split_preview(ui.ctx().clone());
            }
            let can_export = !self.worker.is_busy
                && self.settings.split.output_dir.is_some()
                && self.validate_split_document().is_ok();
            if ui
                .add_enabled(can_export, egui::Button::new("💾 Нарезать"))
                .clicked()
            {
                self.export_split_parts(ui.ctx().clone());
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

    pub(super) fn show_splitter_settings(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.add_enabled_ui(!self.worker.is_busy, |ui| {
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

                    let toc_available = !self.editor.document.headings.is_empty();
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
                        if ui.button("📂 Выбрать…").clicked()
                            && let Some(dir) = rfd::FileDialog::new().pick_folder()
                        {
                            self.settings.split.output_dir = Some(dir);
                            changed = true;
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
        changed
    }

    pub(super) fn persist_split_change(&mut self, ui: &egui::Ui, changed: bool) {
        if changed {
            self.splitter.stale = true;
            self.splitter.save_pending = true;
        }
        if self.splitter.save_pending && !ui.ctx().input(|i| i.pointer.primary_down()) {
            self.splitter.save_pending = false;
            self.save_settings();
        }
    }

    fn validate_split_document(&self) -> AppResult<()> {
        book_splitter::validate_config(&self.settings.split)?;
        let config = &self.settings.split;
        let has_non_toc_method =
            config.by_size || config.by_keywords || config.by_blank_lines || config.by_all_caps;
        if config.by_toc && !has_non_toc_method && self.editor.document.headings.is_empty() {
            return Err(AppError::invalid_data("В этой книге нет структуры глав"));
        }
        Ok(())
    }

    fn split_source_name(&self) -> String {
        match &self.editor.document.source_path {
            Some(path) => crate::services::batch::output_stem(path),
            None => self.editor.document.title.clone(),
        }
    }

    fn run_split_preview(&mut self, ctx: egui::Context) {
        if self.worker.is_busy {
            return;
        }
        if let Err(err) = self.validate_split_document() {
            self.set_status(StatusLevel::Warning, err.to_string());
            self.splitter.stale = true;
            return;
        }
        self.splitter.stale = true;
        self.worker.is_busy = true;
        self.worker.progress_ratio = 0.0;
        self.set_status(StatusLevel::Info, "Подготовка превью…");
        let text = self.editor.document.raw_text.clone();
        let headings = self.editor.document.headings.clone();
        let config = self.settings.split.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.worker.cancel_flag = Some(Arc::clone(&cancel));
        let (tx, rx) = mpsc::channel();
        self.worker.rx = Some(rx);
        std::thread::spawn(move || {
            let result =
                book_splitter::split_book_with_cancel(&text, &headings, &config, Some(&cancel));
            let message = match classify_worker_result(result, Some(cancel.as_ref())) {
                WorkerOutcome::Completed(parts) => WorkerMessage::SplitPreview(Ok(parts)),
                WorkerOutcome::Cancelled => {
                    WorkerMessage::SplitPreview(Err(AppError::cancelled("Превью отменено")))
                }
                WorkerOutcome::Failed(error) => WorkerMessage::SplitPreview(Err(error)),
            };
            let _ = tx.send(message);
            ctx.request_repaint();
        });
    }

    pub(super) fn store_split_preview(&mut self, parts: Vec<BookPart>) {
        let source_name = self.split_source_name();
        let names = book_splitter::planned_part_names(&parts, &self.settings.split, &source_name);
        self.store_split_preview_with_names(parts, names);
    }

    fn store_split_preview_with_names(&mut self, parts: Vec<BookPart>, names: Vec<String>) {
        self.splitter.names = names;
        self.splitter.preview = parts;
        self.splitter.has_preview = true;
        self.splitter.stale = false;
    }

    fn export_split_parts(&mut self, ctx: egui::Context) {
        if self.worker.is_busy {
            return;
        }
        if let Err(err) = self.validate_split_document() {
            self.set_status(StatusLevel::Error, err.to_string());
            return;
        }
        self.splitter.stale = true;
        self.worker.is_busy = true;
        self.worker.progress_ratio = 0.0;
        self.set_status(StatusLevel::Info, "Подготовка частей к записи…");
        let text = self.editor.document.raw_text.clone();
        let headings = self.editor.document.headings.clone();
        let config = self.settings.split.clone();
        let source_name = self.split_source_name();
        let cancel = Arc::new(AtomicBool::new(false));
        self.worker.cancel_flag = Some(Arc::clone(&cancel));
        let (tx, rx) = mpsc::channel();
        self.worker.rx = Some(rx);
        let (confirm_tx, confirm_rx) = mpsc::channel();
        self.worker.split_confirm_tx = Some(confirm_tx);
        std::thread::spawn(move || {
            let confirm_tx = tx.clone();
            let confirm = move |parts: usize| {
                if confirm_tx
                    .send(WorkerMessage::BookSplit(
                        book_splitter::BatchSplitProgress::ConfirmLargeExport { parts },
                    ))
                    .is_err()
                {
                    return false;
                }
                confirm_rx.recv().unwrap_or(false)
            };
            let result = (|| {
                let parts = book_splitter::split_book_with_cancel(
                    &text,
                    &headings,
                    &config,
                    Some(cancel.as_ref()),
                )?;
                if parts.is_empty() {
                    return Err(AppError::invalid_data("Нечего сохранять: текст пуст"));
                }
                if parts.len() > 200 && !confirm(parts.len()) {
                    return Err(AppError::cancelled("Экспорт отменён"));
                }
                let summary = book_splitter::export_parts_with_cancel(
                    &parts,
                    &config,
                    &source_name,
                    Some(cancel.as_ref()),
                )?;
                Ok(super::worker::SingleSplitExport { parts, summary })
            })();
            let _ = tx.send(WorkerMessage::SplitExport(result));
            ctx.request_repaint();
        });
    }

    pub(super) fn apply_split_export(
        &mut self,
        mut parts: Vec<BookPart>,
        summary: book_splitter::ExportSummary,
        pending_status: &mut Option<(StatusLevel, String)>,
    ) -> bool {
        let attempted = summary.targets.len().min(parts.len());
        parts.truncate(attempted);
        let names = summary
            .targets
            .iter()
            .take(attempted)
            .map(|target| {
                target
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .and_then(|name| name.strip_suffix(".txt").map(str::to_string))
                    .unwrap_or_else(|| "часть".to_string())
            })
            .collect();
        self.store_split_preview_with_names(parts, names);
        if summary.errors.is_empty() {
            *pending_status = Some((
                StatusLevel::Success,
                format!(
                    "Сохранено {} частей в {}",
                    summary.saved,
                    summary.dir.display()
                ),
            ));
        } else {
            *pending_status = Some((
                StatusLevel::Warning,
                format!(
                    "Сохранено {} частей, ошибок: {}. {}",
                    summary.saved,
                    summary.errors.len(),
                    summary
                        .errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            ));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app(text: &str, output_dir: Option<PathBuf>) -> DemagogApp {
        let mut settings = AppSettings::default();
        settings.split.by_size = false;
        settings.split.by_keywords = true;
        settings.split.output_dir = output_dir;
        let document = BookDocument {
            raw_text: text.to_string(),
            source_path: Some(PathBuf::from("book.txt")),
            ..Default::default()
        };
        DemagogApp::from_parts(settings, document)
    }

    #[test]
    fn single_preview_starts_a_worker() {
        let mut app = test_app("Глава 1\nТекст.", None);

        app.run_split_preview(egui::Context::default());

        assert!(app.worker.is_busy);
        assert!(app.worker.rx.is_some());
        assert!(!app.splitter.has_preview);
    }

    #[test]
    fn single_preview_names_include_collision_suffixes() {
        let mut app = test_app("Глава 1\nОдинаково.\nГлава 2\nОдинаково.", None);
        app.settings.split.remove_heading = true;
        app.settings.split.template = "%FirstLine%".to_string();
        app.settings.split.add_number = false;

        app.run_split_preview(egui::Context::default());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.worker.is_busy && std::time::Instant::now() < deadline {
            app.poll_worker();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(
            app.splitter.names,
            vec!["Одинаково".to_string(), "Одинаково (2)".to_string()]
        );
    }

    #[test]
    fn single_export_starts_a_worker() {
        let dir = std::env::temp_dir().join(format!(
            "demagog-single-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = test_app("Глава 1\nТекст.", Some(dir.clone()));

        app.export_split_parts(egui::Context::default());
        let started_async = app.worker.rx.is_some();
        if started_async {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while app.worker.is_busy && std::time::Instant::now() < deadline {
                app.poll_worker();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);

        assert!(started_async);
        assert!(!app.worker.is_busy);
        assert!(app.splitter.has_preview);
    }

    #[test]
    fn cancelled_single_export_preview_contains_only_attempted_parts() {
        let dir = std::env::temp_dir().join(format!(
            "demagog-single-cancel-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut app = test_app("Глава 1\nПервый.\nГлава 2\nВторой.", Some(dir.clone()));
        let parts = vec![
            BookPart {
                title: Some("Глава 1".to_string()),
                text: "Глава 1\nПервый.".to_string(),
            },
            BookPart {
                title: Some("Глава 2".to_string()),
                text: "Глава 2\nВторой.".to_string(),
            },
        ];
        let summary = book_splitter::ExportSummary {
            saved: 1,
            dir: dir.clone(),
            targets: vec![dir.join("01 Глава 1.txt")],
            errors: vec![AppError::cancelled("Экспорт частей отменён")],
        };
        let mut pending_status = None;

        app.apply_split_export(parts, summary, &mut pending_status);

        assert_eq!(app.splitter.preview.len(), 1);
        assert_eq!(app.splitter.names, vec!["01 Глава 1".to_string()]);
        assert_eq!(app.splitter.preview[0].text, "Глава 1\nПервый.");
        assert!(matches!(pending_status, Some((StatusLevel::Warning, _))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

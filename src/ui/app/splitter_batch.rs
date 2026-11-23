//! Пакетный режим вкладки «Книгорез»: очередь, статусы, запуск нарезки.

use super::splitter::BatchSplitStatus;
use super::*;
use crate::services::book_splitter::{self, run_split_batch, validate_config};

impl DemagogApp {
    /// Пакетное меню — очередь книг, статусы, запуск пакетной нарезки.
    pub(super) fn show_splitter_batch_menu(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        let changed = self.show_splitter_settings(ui);
        self.persist_split_change(ui, changed);
        self.show_batch_queue_section(ui, theme);
    }

    fn show_batch_queue_section(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        ui.label("Пакетная нарезка");
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!self.is_busy, egui::Button::new("➕ Добавить файлы…"))
                .on_hover_text("Выбрать несколько книг (TXT, FB2, EPUB, DOCX, ZIP)")
                .clicked()
                && let Some(paths) = rfd::FileDialog::new()
                    .add_filter(
                        "Книги и текст",
                        &["txt", "fb2", "epub", "docx", "zip", "md", "log"],
                    )
                    .pick_files()
            {
                self.splitter.batch_queue.add_files(paths);
            }
            if ui
                .add_enabled(
                    !self.splitter.batch_queue.files.is_empty(),
                    egui::Button::new("🗑 Очистить"),
                )
                .clicked()
            {
                self.splitter.batch_queue.clear();
            }
        });
        if self.splitter.batch_queue.files.is_empty() {
            ui.label("Добавьте книги — каждая будет нарезана в папке «ИмяКниги».");
        }
        ScrollArea::vertical()
            .id_salt("splitter_batch_queue")
            .max_height(180.0)
            .show(ui, |ui| {
                for (index, status) in self.splitter.batch_queue.statuses.iter().enumerate() {
                    let (icon, detail) = match status {
                        BatchSplitStatus::Queued => ("⏳", String::new()),
                        BatchSplitStatus::Running => ("⚙", "нарезка…".to_string()),
                        BatchSplitStatus::Done(parts) => ("✅", format!("{parts} частей")),
                        BatchSplitStatus::Failed(error) => ("❌", error.clone()),
                    };
                    let name = self.splitter.batch_queue.display_name(index);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(icon);
                        ui.label(name);
                        if !detail.is_empty() {
                            ui.label(RichText::new(detail).small().weak());
                        }
                    });
                }
            });
        let total = self.splitter.batch_queue.statuses.len();
        if self.splitter.batch_queue.running && total > 0 {
            let done =
                self.splitter.batch_queue.done_count() + self.splitter.batch_queue.failed_count();
            ui.add(
                egui::ProgressBar::new(done as f32 / total as f32).text(format!("{done}/{total}")),
            );
        }
        ui.horizontal_wrapped(|ui| {
            if self.splitter.batch_queue.running {
                if ui.button("⏹ Отмена").clicked()
                    && let Some(flag) = &self.cancel_flag
                {
                    flag.store(true, Ordering::Relaxed);
                }
            } else if ui
                .add_enabled(
                    self.splitter
                        .batch_queue
                        .can_start(self.is_busy, self.settings.split.output_dir.is_some()),
                    egui::Button::new("▶ Нарезать пакет"),
                )
                .on_disabled_hover_text("Добавьте файлы и выберите папку назначения")
                .clicked()
            {
                self.start_batch_split(ui.ctx().clone());
            }
            if let Some(outcome) = self.splitter.batch_queue.summary {
                let counts = outcome.counts();
                let text = if outcome.is_cancelled() {
                    format!(
                        "Отменено: нарезано {}, ошибок {}",
                        counts.succeeded, counts.failed
                    )
                } else {
                    format!("Готово: {}, ошибок: {}", counts.succeeded, counts.failed)
                };
                ui.label(RichText::new(text).strong());
            }
        });
        if let Some(warning) = &self.splitter.batch_queue.warning {
            ui.label(RichText::new(warning.clone()).color(palette::warning_text(theme)));
        }
        if let Some(error) = &self.splitter.batch_queue.error {
            ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
        }
    }

    /// Папки «ИмяКниги» для всех в очереди пакетной нарезки, уже существующие на диске.
    fn batch_existing_dirs(&self) -> Vec<String> {
        let Some(base) = self.settings.split.output_dir.as_ref() else {
            return Vec::new();
        };
        self.splitter
            .batch_queue
            .files
            .iter()
            .map(|file| {
                base.join(book_splitter::sanitize_file_name(
                    &crate::services::batch::output_stem(&file.path),
                ))
            })
            .filter(|dir| dir.exists())
            .map(|dir| dir.display().to_string())
            .collect()
    }

    fn start_batch_split(&mut self, ctx: egui::Context) {
        if !self
            .splitter
            .batch_queue
            .can_start(self.is_busy, self.settings.split.output_dir.is_some())
        {
            return;
        }
        if let Err(err) = validate_config(&self.settings.split) {
            self.splitter.batch_queue.error = Some(err);
            return;
        }
        self.splitter.batch_queue.warning = None;
        let existing = self.batch_existing_dirs();
        if !existing.is_empty() {
            let warning = format!(
                "Папки уже существуют: {} — файлы внутри могут затереться. Продолжить?",
                existing.join(", ")
            );
            if !confirm_destructive(&warning) {
                return;
            }
            self.splitter.batch_queue.warning = Some(warning);
        }
        self.splitter
            .batch_queue
            .statuses
            .fill(BatchSplitStatus::Queued);
        self.splitter.batch_queue.summary = None;
        self.splitter.batch_queue.error = None;
        self.splitter.batch_queue.running = true;
        self.is_busy = true;
        self.progress_ratio = 0.0;

        let files = self.splitter.batch_queue.files.clone();
        let config = self.settings.split.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(Arc::clone(&cancel));
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

        std::thread::spawn(move || {
            let (btx, brx) = mpsc::channel();
            let tx_forward = tx.clone();
            let ctx_forward = ctx.clone();
            let forwarder = std::thread::spawn(move || {
                while let Ok(msg) = brx.recv() {
                    let _ = tx_forward.send(WorkerMessage::BookSplit(msg));
                    ctx_forward.request_repaint();
                }
            });
            let reader = CompositeBookReader::new();
            run_split_batch(&files, &reader, &config, Some(cancel.as_ref()), Some(&btx));
            drop(btx);
            let _ = forwarder.join();
        });
    }
}

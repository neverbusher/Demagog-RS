use super::*;
use crate::domain::{BatchQueueEntry, BatchSettings};
use crate::services::batch::{BatchInput, BatchOptions, BatchOutcome, BatchOutputs, run_batch};

#[derive(Debug, Clone)]
pub(super) enum BatchItemStatus {
    Queued,
    Running,
    Done(BatchOutputs),
    Failed(String),
}

#[derive(Debug, Clone)]
pub(super) struct BatchItem {
    pub path: PathBuf,
    pub base_dir: PathBuf,
    pub status: BatchItemStatus,
}

#[derive(Debug, Clone)]
pub(super) struct BatchState {
    pub open: bool,
    pub items: Vec<BatchItem>,
    pub output_dir: Option<PathBuf>,
    pub export_txt: bool,
    pub export_chunks: bool,
    pub overwrite: bool,
    pub running: bool,
    pub summary: Option<BatchOutcome>,
    pub error: Option<String>,
}

impl Default for BatchState {
    fn default() -> Self {
        Self {
            open: false,
            items: Vec::new(),
            output_dir: None,
            export_txt: true,
            export_chunks: true,
            overwrite: true,
            running: false,
            summary: None,
            error: None,
        }
    }
}

impl BatchState {
    pub fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            let base_dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
            if self.try_push(path.clone(), base_dir) && self.output_dir.is_none() {
                self.output_dir = path.parent().map(|parent| parent.join("processed"));
            }
        }
    }

    pub fn add_folder_files(&mut self, paths: impl IntoIterator<Item = PathBuf>, base: &Path) {
        for path in paths {
            if self.try_push(path, base.to_path_buf()) && self.output_dir.is_none() {
                self.output_dir = Some(base.join("processed"));
            }
        }
    }

    fn try_push(&mut self, path: PathBuf, base_dir: PathBuf) -> bool {
        if !can_read_book(&path) || self.items.iter().any(|item| item.path == path) {
            return false;
        }
        self.items.push(BatchItem {
            path,
            base_dir,
            status: BatchItemStatus::Queued,
        });
        true
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.output_dir = None;
        self.summary = None;
        self.error = None;
    }

    pub fn can_start(&self, is_busy: bool) -> bool {
        !is_busy
            && !self.items.is_empty()
            && (self.export_txt || self.export_chunks)
            && self.output_dir.is_some()
    }

    fn start_disabled_hint(&self, is_busy: bool) -> &'static str {
        if is_busy {
            "Дождитесь завершения текущей операции"
        } else if self.items.is_empty() {
            "Добавьте файлы в очередь"
        } else if !self.export_txt && !self.export_chunks {
            "Выберите хотя бы один формат вывода"
        } else {
            "Выберите папку результатов"
        }
    }
}

fn batch_setup_error(result: Result<BatchOutcome, AppError>) -> Option<String> {
    match result {
        Err(error @ AppError::Failed(_)) => Some(error.to_string()),
        Err(AppError::Cancelled(_)) | Ok(_) => None,
    }
}

pub(super) fn restore_queue(entries: &[BatchQueueEntry]) -> (Vec<BatchItem>, usize) {
    let mut items = Vec::new();
    let mut dropped = 0;
    for entry in entries {
        if entry.path.is_file() {
            items.push(BatchItem {
                path: entry.path.clone(),
                base_dir: entry.base_dir.clone(),
                status: BatchItemStatus::Queued,
            });
        } else {
            dropped += 1;
        }
    }
    (items, dropped)
}

pub(super) fn collect_books_recursive(dir: &Path, exclude: Option<&Path>) -> Vec<PathBuf> {
    let mut books = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        if exclude.is_some_and(|ex| current.starts_with(ex)) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if can_read_book(&path) {
                books.push(path);
            }
        }
    }
    books.sort();
    books
}

impl DemagogApp {
    pub(super) fn sync_batch_settings(&mut self) {
        self.settings.batch = BatchSettings {
            export_txt: self.batch.export_txt,
            export_chunks: self.batch.export_chunks,
            overwrite: self.batch.overwrite,
            output_dir: self.batch.output_dir.clone(),
            queue: self
                .batch
                .items
                .iter()
                .map(|item| BatchQueueEntry {
                    path: item.path.clone(),
                    base_dir: item.base_dir.clone(),
                })
                .collect(),
        };
    }

    pub(super) fn persist_batch(&mut self) {
        self.sync_batch_settings();
        self.save_settings();
    }

    pub(super) fn show_batch_window(&mut self, ctx: &egui::Context) {
        if !self.batch.open {
            return;
        }

        let mut open = true;
        egui::Window::new("📚 Пакетная обработка")
            .open(&mut open)
            .resizable(true)
            .default_width(760.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                let editable = !self.batch.running;
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(editable, egui::Button::new("➕ Добавить файлы"))
                        .on_hover_text("Выбрать несколько книг (TXT, FB2, EPUB, DOCX, ZIP, MD)")
                        .clicked()
                        && let Some(paths) = rfd::FileDialog::new()
                            .add_filter(
                                "Книги и текст",
                                &["txt", "fb2", "epub", "docx", "zip", "md", "log"],
                            )
                            .pick_files()
                    {
                        self.batch.add_files(paths);
                        self.persist_batch();
                    }
                    if ui
                        .add_enabled(editable, egui::Button::new("📁 Добавить папку"))
                        .on_hover_text("Добавить все поддерживаемые книги из папки и подпапок")
                        .clicked()
                        && let Some(dir) = rfd::FileDialog::new().pick_folder()
                    {
                        let books =
                            collect_books_recursive(&dir, self.batch.output_dir.as_deref());
                        self.batch.add_folder_files(books, &dir);
                        self.persist_batch();
                    }
                    if ui
                        .add_enabled(
                            editable && !self.batch.items.is_empty(),
                            egui::Button::new("🗑 Очистить"),
                        )
                        .clicked()
                    {
                        self.batch.clear();
                        self.persist_batch();
                    }
                });
                ui.add_space(6.0);

                ui.horizontal_wrapped(|ui| {
                    ui.label("Папка результатов:");
                    let dir_text = self
                        .batch
                        .output_dir
                        .as_ref()
                        .map(|dir| dir.display().to_string())
                        .unwrap_or_else(|| "не выбрана".to_string());
                    ui.label(RichText::new(dir_text).weak());
                    if ui
                        .add_enabled(editable, egui::Button::new("Выбрать…"))
                        .clicked()
                        && let Some(dir) = rfd::FileDialog::new().pick_folder()
                    {
                        self.batch.output_dir = Some(dir);
                        self.persist_batch();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let formats_changed = ui
                        .add_enabled_ui(editable, |ui| {
                            let txt = ui
                                .checkbox(&mut self.batch.export_txt, "TXT результата")
                                .on_hover_text("Один обработанный TXT на каждый входной файл")
                                .changed();
                            let chunks = ui
                                .checkbox(&mut self.batch.export_chunks, "JSON чанков")
                                .on_hover_text("JSON с чанками для ComfyUI на каждый входной файл")
                                .changed();
                            let overwrite = ui
                                .checkbox(&mut self.batch.overwrite, "Перезаписывать существующие")
                                .on_hover_text(
                                    "Выкл: одноимённые файлы сохраняются с суффиксом (2), (3)…",
                                )
                                .changed();
                            txt || chunks || overwrite
                        })
                        .inner;
                    if formats_changed {
                        self.persist_batch();
                    }
                });
                ui.separator();

                ScrollArea::vertical()
                    .id_salt("batch_files")
                    .max_height(260.0)
                    .show(ui, |ui| {
                        if self.batch.items.is_empty() {
                            ui.label(
                                "Добавьте файлы или папку — можно перетащить несколько книг в это окно.",
                            );
                        }
                        for item in &self.batch.items {
                            let (icon, detail) = match &item.status {
                                BatchItemStatus::Queued => ("⏳", String::new()),
                                BatchItemStatus::Running => ("⚙", "обработка…".to_string()),
                                BatchItemStatus::Done(outputs) => {
                                    ("✅", format!("чанков: {}", outputs.chunk_count))
                                }
                                BatchItemStatus::Failed(error) => ("❌", error.clone()),
                            };
                            let name = item
                                .path
                                .strip_prefix(&item.base_dir)
                                .unwrap_or(&item.path)
                                .display()
                                .to_string();
                            ui.horizontal_wrapped(|ui| {
                                ui.label(icon);
                                ui.label(name);
                                if !detail.is_empty() {
                                    ui.label(RichText::new(detail).small().weak());
                                }
                            });
                        }
                    });
                ui.separator();

                let total = self.batch.items.len();
                if self.batch.running && total > 0 {
                    let done = self
                        .batch
                        .items
                        .iter()
                        .filter(|item| {
                            matches!(
                                item.status,
                                BatchItemStatus::Done(_) | BatchItemStatus::Failed(_)
                            )
                        })
                        .count();
                    ui.add(
                        egui::ProgressBar::new(done as f32 / total as f32)
                            .text(format!("{done}/{total}")),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    if self.batch.running {
                        if ui.button("⏹ Отмена").clicked()
                            && let Some(flag) = &self.cancel_flag
                        {
                            flag.store(true, Ordering::Relaxed);
                        }
                    } else if ui
                        .add_enabled(
                            self.batch.can_start(self.is_busy),
                            egui::Button::new("▶ Старт"),
                        )
                        .on_disabled_hover_text(self.batch.start_disabled_hint(self.is_busy))
                        .clicked()
                    {
                        self.start_batch(ctx.clone());
                    }
                    if let Some(outcome) = self.batch.summary {
                        let counts = outcome.counts();
                        let text = if outcome.is_cancelled() {
                            format!(
                                "Отменено: обработано {}, ошибок {}",
                                counts.succeeded, counts.failed
                            )
                        } else {
                            format!("Готово: {} | Ошибок: {}", counts.succeeded, counts.failed)
                        };
                        ui.label(RichText::new(text).strong());
                    }
                    if !self.batch.running
                        && self
                            .batch
                            .output_dir
                            .as_ref()
                            .is_some_and(|dir| dir.is_dir())
                        && ui.button("📂 Открыть папку").clicked()
                        && let Some(dir) = &self.batch.output_dir
                    {
                        let _ = std::process::Command::new("explorer").arg(dir).spawn();
                    }
                });
                if let Some(error) = &self.batch.error {
                    ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                }
            });
        self.batch.open = open;
    }

    pub(super) fn start_batch(&mut self, ctx: egui::Context) {
        if !self.batch.can_start(self.is_busy) {
            return;
        }
        let Some(output_dir) = self.batch.output_dir.clone() else {
            return;
        };

        for item in &mut self.batch.items {
            item.status = BatchItemStatus::Queued;
        }
        self.batch.summary = None;
        self.batch.error = None;
        self.batch.running = true;
        self.is_busy = true;
        self.progress_ratio = 0.0;

        let files: Vec<BatchInput> = self
            .batch
            .items
            .iter()
            .map(|item| BatchInput {
                path: item.path.clone(),
                base_dir: item.base_dir.clone(),
            })
            .collect();
        let processors: Vec<Arc<dyn TextProcessor>> = self
            .dictionaries
            .iter()
            .filter(|d| d.meta.enabled)
            .map(|d| d.processor.clone())
            .collect();
        let options = BatchOptions {
            output_dir,
            export_txt: self.batch.export_txt,
            export_chunks: self.batch.export_chunks,
            overwrite: self.batch.overwrite,
            pipeline: PipelineOptions {
                stress_format: self.settings.stress_format,
                chunk_config: self.settings.chunk_config.clone(),
                capture_diff: false,
                cleanup: self.settings.cleanup,
            },
        };

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
                    let _ = tx_forward.send(WorkerMessage::Batch(msg));
                    ctx_forward.request_repaint();
                }
            });

            let reader = CompositeBookReader::new();
            let pipeline = ProcessingPipeline::new(
                Arc::new(RussianStressFormatter),
                Arc::new(SentenceAwareChunker),
            );
            let result = run_batch(
                &files,
                &reader,
                &processors,
                &pipeline,
                &options,
                Some(cancel.as_ref()),
                Some(&btx),
            );
            drop(btx);
            let _ = forwarder.join();
            if let Some(error) = batch_setup_error(result) {
                let _ = tx.send(WorkerMessage::BatchSetupError(error));
                ctx.request_repaint();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_setup_error_only_reports_failed_results() {
        assert_eq!(
            batch_setup_error(Err(AppError::io("  Ошибка пакета\n"))),
            Some("  Ошибка пакета\n".to_string())
        );
        assert_eq!(
            batch_setup_error(Err(AppError::cancelled("Пакет отменён"))),
            None
        );
        assert_eq!(
            batch_setup_error(Ok(BatchOutcome::Completed(Default::default()))),
            None
        );
    }
}

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, Sender};

use super::splitter::BatchSplitStatus;
use super::{
    ActiveTab, DemagogApp, LoadedDictionary, PipelineReport, StatusLevel, batch, format_num,
    splitter,
};
use crate::domain::{AppError, BookDocument, PipelineProgress};
use crate::engine::loader::CompiledDictionary;
use crate::ports::is_cancelled;
use crate::services::batch::BatchProgress;
use crate::services::book_splitter::{BatchSplitProgress, BookPart, ExportSummary};
use crate::services::dictionary_check::{self, CheckReport};

pub(super) struct SingleSplitExport {
    pub(super) parts: Vec<BookPart>,
    pub(super) summary: ExportSummary,
}

pub(super) enum WorkerMessage {
    DictionariesCompiled {
        compiled: Vec<CompiledDictionary>,
        errors: Vec<String>,
        elapsed_ms: u128,
    },
    BookLoaded(Result<BookDocument, String>),
    Pipeline(PipelineProgress),
    DictionaryCheckFinished(Result<CheckReport, String>),
    Batch(BatchProgress),
    BookSplit(BatchSplitProgress),
    SplitPreview(Result<Vec<BookPart>, AppError>),
    SplitExport(Result<SingleSplitExport, AppError>),
    BatchSetupError(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum WorkerOutcome<T> {
    Cancelled,
    Completed(T),
    Failed(AppError),
}

pub(super) fn classify_worker_result<T>(
    result: Result<T, AppError>,
    cancel: Option<&AtomicBool>,
) -> WorkerOutcome<T> {
    if is_cancelled(cancel) {
        return WorkerOutcome::Cancelled;
    }
    match result {
        Ok(value) => WorkerOutcome::Completed(value),
        Err(AppError::Cancelled(_)) => WorkerOutcome::Cancelled,
        Err(error) => WorkerOutcome::Failed(error),
    }
}

#[derive(Default)]
pub(super) struct WorkerState {
    pub(super) is_busy: bool,
    pub(super) progress_ratio: f32,
    pub(super) rx: Option<Receiver<WorkerMessage>>,
    pub(super) cancel_flag: Option<Arc<AtomicBool>>,
    pub(super) split_confirm_tx: Option<Sender<bool>>,
    pub(super) dic_load_errors: Vec<String>,
}

type PendingStatus = Option<(StatusLevel, String)>;

impl DemagogApp {
    pub(super) fn poll_worker(&mut self) {
        let rx = self.worker.rx.take();
        let mut finished = false;
        let mut disconnected = false;
        let mut pending_status: PendingStatus = None;

        if let Some(rx) = &rx {
            loop {
                match rx.try_recv() {
                    Ok(msg) => {
                        finished = self.handle_worker_message(msg, &mut pending_status);
                        if finished {
                            break;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        if let Some((level, text)) = pending_status {
            self.set_status(level, text);
        }

        if finished || disconnected {
            self.processing.processing_snapshot = None;
            if disconnected && !finished {
                self.set_status(
                    StatusLevel::Error,
                    "Рабочий поток завершился неожиданно — попробуйте ещё раз",
                );
                self.worker.cancel_flag = None;
                self.worker.split_confirm_tx = None;
                self.worker.is_busy = false;
                self.worker.progress_ratio = 0.0;
                if self.splitter.batch_queue.running {
                    self.splitter.batch_queue.running = false;
                    self.splitter.batch_queue.cancel_requested = false;
                    self.splitter.batch_queue.warning = None;
                    self.splitter.batch_queue.error = None;
                    for status in &mut self.splitter.batch_queue.statuses {
                        if matches!(status, BatchSplitStatus::Running) {
                            *status = BatchSplitStatus::Queued;
                        }
                    }
                }
            }
            drop(rx);
        } else {
            self.worker.rx = rx;
        }
    }

    fn handle_worker_message(
        &mut self,
        msg: WorkerMessage,
        pending_status: &mut PendingStatus,
    ) -> bool {
        match msg {
            WorkerMessage::DictionariesCompiled {
                compiled,
                errors,
                elapsed_ms,
            } => self.handle_dictionaries_compiled(compiled, errors, elapsed_ms, pending_status),
            WorkerMessage::DictionaryCheckFinished(result) => {
                self.handle_dictionary_check_finished(result, pending_status)
            }
            WorkerMessage::BookLoaded(result) => self.handle_book_loaded(result, pending_status),
            WorkerMessage::Pipeline(progress) => self.handle_pipeline(progress, pending_status),
            WorkerMessage::Batch(progress) => self.handle_batch(progress, pending_status),
            WorkerMessage::BookSplit(progress) => self.handle_book_split(progress, pending_status),
            WorkerMessage::SplitPreview(result) => {
                self.handle_split_preview(result, pending_status)
            }
            WorkerMessage::SplitExport(result) => self.handle_split_export(result, pending_status),
            WorkerMessage::BatchSetupError(error) => {
                self.handle_batch_setup_error(error, pending_status)
            }
        }
    }

    fn handle_dictionaries_compiled(
        &mut self,
        compiled: Vec<CompiledDictionary>,
        errors: Vec<String>,
        elapsed_ms: u128,
        pending_status: &mut PendingStatus,
    ) -> bool {
        let total_rules: usize = compiled.iter().map(|c| c.meta.rule_count).sum();
        let settings = &self.settings;
        self.dictionaries = compiled
            .into_iter()
            .map(|compiled| {
                let mut meta = compiled.meta;
                meta.enabled = settings.is_dic_enabled(&meta.name);
                LoadedDictionary {
                    meta,
                    processor: compiled.processor,
                }
            })
            .collect();
        let mut status = format!(
            "Готово: загружено {} словарей ({} правил) за {} мс",
            self.dictionaries.len(),
            format_num(total_rules),
            elapsed_ms
        );
        if !errors.is_empty() {
            status.push_str(&format!(" | пропущено с ошибкой: {}", errors.len()));
        }
        let level = if errors.is_empty() {
            StatusLevel::Success
        } else {
            StatusLevel::Warning
        };
        *pending_status = Some((level, status));
        self.worker.dic_load_errors = errors;
        if !self.editor.document.processed_text.is_empty()
            || !self.editor.document.chunks.is_empty()
        {
            self.editor.result_state.mark_stale();
        }
        self.worker.cancel_flag = None;
        self.worker.is_busy = false;
        self.worker.progress_ratio = 1.0;
        true
    }

    fn handle_dictionary_check_finished(
        &mut self,
        result: Result<CheckReport, String>,
        pending_status: &mut PendingStatus,
    ) -> bool {
        self.worker.is_busy = false;
        self.worker.progress_ratio = 1.0;
        match result {
            Ok(report) => {
                let failures = report.order.failures.len();
                let warnings = report.order.warnings.len();
                let total = dictionary_check::GOLDEN_CASES.len();
                let mut status = if failures > 0 {
                    format!(
                        "Проверка словарей: ошибок {failures} (эталонных фраз {}/{total})",
                        report.passed
                    )
                } else if warnings > 0 {
                    format!(
                        "Проверка словарей: OK с предупреждениями (эталонных фраз {}/{total})",
                        report.passed
                    )
                } else {
                    format!(
                        "Проверка словарей: OK (эталонных фраз {}/{total})",
                        report.passed
                    )
                };
                if report.audit.total_candidates > 0 {
                    status.push_str(&format!(
                        " | фонетических кандидатов: {}",
                        report.audit.total_candidates
                    ));
                }
                let level = if failures > 0 {
                    StatusLevel::Error
                } else if warnings > 0 {
                    StatusLevel::Warning
                } else {
                    StatusLevel::Success
                };
                *pending_status = Some((level, status));
                self.view.check_report = Some(report);
                self.view.show_check_window = true;
            }
            Err(error) => {
                *pending_status = Some((StatusLevel::Error, format!("Проверка словарей: {error}")));
            }
        }
        self.worker.cancel_flag = None;
        true
    }

    fn handle_book_loaded(
        &mut self,
        result: Result<BookDocument, String>,
        pending_status: &mut PendingStatus,
    ) -> bool {
        match result {
            Ok(doc) => {
                let chars = doc.raw_text.chars().count();
                *pending_status = Some((
                    StatusLevel::Success,
                    format!(
                        "Открыта книга «{}» ({} символов)",
                        doc.title,
                        format_num(chars)
                    ),
                ));
                self.editor.document = doc;
                self.editor.raw_char_count = chars;
                self.editor.processed_char_count = 0;
                self.diff.diff_cache_valid = false;
                self.editor.raw_pipeline_output.clear();
                self.view.active_tab = ActiveTab::RawText;
                self.view.last_copied_chunk = None;
                self.processing.report = PipelineReport::default();
                for d in &mut self.dictionaries {
                    d.meta.last_ms = None;
                    d.meta.last_replacements = None;
                }
                self.editor.source_dirty = false;
                self.editor.result_state.reset();
                self.splitter = splitter::SplitterState::default();
                self.search.find_dirty = true;
            }
            Err(err) => {
                *pending_status = Some((StatusLevel::Error, err));
            }
        }
        self.worker.cancel_flag = None;
        self.worker.is_busy = false;
        self.worker.progress_ratio = 0.0;
        true
    }

    fn handle_pipeline(
        &mut self,
        progress: PipelineProgress,
        pending_status: &mut PendingStatus,
    ) -> bool {
        match progress {
            PipelineProgress::StepStarted { index, total, name } => {
                self.worker.progress_ratio = index as f32 / total.max(1) as f32;
                *pending_status = Some((
                    StatusLevel::Info,
                    format!("[{index}/{total}] Обработка: {name}..."),
                ));
                false
            }
            PipelineProgress::StepFinished(step) => {
                if let Some(d) = self
                    .dictionaries
                    .iter_mut()
                    .find(|d| d.meta.name == step.dictionary_name)
                {
                    d.meta.last_ms = Some(step.elapsed_ms);
                    d.meta.last_replacements = Some(step.replacements_count);
                }
                false
            }
            PipelineProgress::Completed(result) => {
                let source_changed =
                    self.processing
                        .processing_snapshot
                        .take()
                        .is_some_and(|snapshot| {
                            snapshot.source_changed(&self.editor.document.raw_text)
                        });
                let summary = format!(
                    "Обработано за {} мс | Замен: {} | Чанков: {}",
                    result.report.total_ms,
                    format_num(result.report.total_replacements),
                    result.chunks.len()
                );
                self.editor.raw_pipeline_output = result.raw_text;
                self.editor.document.processed_text = result.processed_text;
                self.editor.document.chunks = result.chunks;
                self.editor.processed_char_count =
                    self.editor.document.processed_text.chars().count();
                self.diff.diff_cache_valid = false;
                let had_events = !result.report.events.is_empty();
                self.processing.report = result.report;
                self.editor
                    .result_state
                    .mark_computed(self.settings.stress_format);
                if source_changed {
                    self.editor.result_state.mark_stale();
                    *pending_status = Some((
                        StatusLevel::Warning,
                        format!(
                            "{summary} | Исходный текст изменён во время обработки — результат устарел, обработайте заново"
                        ),
                    ));
                } else {
                    *pending_status = Some((StatusLevel::Success, summary));
                }
                self.view.last_copied_chunk = None;
                self.search.find_dirty = true;
                self.worker.cancel_flag = None;
                self.worker.is_busy = false;
                self.worker.progress_ratio = 1.0;
                if had_events && self.processing.capture_diff_on_run {
                    self.view.active_tab = ActiveTab::DiffAndStats;
                } else if self.view.active_tab == ActiveTab::RawText {
                    self.view.active_tab = ActiveTab::ProcessedText;
                }
                true
            }
            PipelineProgress::Cancelled => {
                *pending_status = Some((StatusLevel::Warning, "Операция отменена".to_string()));
                self.worker.cancel_flag = None;
                self.worker.is_busy = false;
                self.worker.progress_ratio = 0.0;
                true
            }
            PipelineProgress::Error(err) => {
                *pending_status = Some((StatusLevel::Error, format!("Ошибка: {err}")));
                self.worker.cancel_flag = None;
                self.worker.is_busy = false;
                self.worker.progress_ratio = 0.0;
                true
            }
        }
    }

    fn handle_batch(
        &mut self,
        progress: BatchProgress,
        pending_status: &mut PendingStatus,
    ) -> bool {
        match progress {
            BatchProgress::FileStarted { index, total, name } => {
                if let Some(item) = self.batch.items.get_mut(index) {
                    item.status = batch::BatchItemStatus::Running;
                }
                self.worker.progress_ratio = index as f32 / total.max(1) as f32;
                *pending_status = Some((
                    StatusLevel::Info,
                    format!("Пакет [{}/{}]: {}...", index + 1, total, name),
                ));
                false
            }
            BatchProgress::FileFinished { index, outcome } => {
                if let Some(item) = self.batch.items.get_mut(index) {
                    item.status = match outcome {
                        Ok(outputs) => batch::BatchItemStatus::Done(outputs),
                        Err(error) => batch::BatchItemStatus::Failed(error),
                    };
                }
                let total = self.batch.items.len().max(1);
                self.worker.progress_ratio = (index + 1) as f32 / total as f32;
                false
            }
            BatchProgress::Finished(outcome) => {
                self.batch.running = false;
                self.batch.summary = Some(outcome);
                let counts = outcome.counts();
                if outcome.is_cancelled() {
                    for item in &mut self.batch.items {
                        if matches!(item.status, batch::BatchItemStatus::Running) {
                            item.status = batch::BatchItemStatus::Queued;
                        }
                    }
                    self.worker.progress_ratio = 0.0;
                    *pending_status = Some((
                        StatusLevel::Warning,
                        "Пакетная обработка отменена".to_string(),
                    ));
                } else {
                    self.worker.progress_ratio = 1.0;
                    let level = if counts.failed > 0 {
                        StatusLevel::Warning
                    } else {
                        StatusLevel::Success
                    };
                    *pending_status = Some((
                        level,
                        format!(
                            "Пакет готов: обработано {}, ошибок {}",
                            counts.succeeded, counts.failed
                        ),
                    ));
                }
                self.worker.cancel_flag = None;
                self.worker.is_busy = false;
                true
            }
        }
    }

    fn handle_split_preview(
        &mut self,
        result: Result<Vec<BookPart>, AppError>,
        pending_status: &mut PendingStatus,
    ) -> bool {
        self.worker.cancel_flag = None;
        self.worker.is_busy = false;
        self.worker.progress_ratio = 1.0;
        match result {
            Ok(parts) => {
                let count = parts.len();
                self.store_split_preview(parts);
                *pending_status =
                    Some((StatusLevel::Info, format!("Превью готово: {count} частей")));
            }
            Err(error) => {
                let level = if error.is_cancelled() {
                    StatusLevel::Warning
                } else {
                    StatusLevel::Error
                };
                *pending_status = Some((level, error.to_string()));
            }
        }
        true
    }

    fn handle_split_export(
        &mut self,
        result: Result<SingleSplitExport, AppError>,
        pending_status: &mut PendingStatus,
    ) -> bool {
        self.worker.cancel_flag = None;
        self.worker.split_confirm_tx = None;
        self.worker.is_busy = false;
        self.worker.progress_ratio = 1.0;
        match result {
            Ok(result) => {
                self.apply_split_export(result.parts, result.summary, pending_status);
            }
            Err(error) => {
                let level = if error.is_cancelled() {
                    StatusLevel::Warning
                } else {
                    StatusLevel::Error
                };
                *pending_status = Some((level, error.to_string()));
            }
        }
        true
    }

    fn handle_book_split(
        &mut self,
        progress: BatchSplitProgress,
        pending_status: &mut PendingStatus,
    ) -> bool {
        match progress {
            BatchSplitProgress::FileStarted { index, total, name } => {
                if let Some(status) = self.splitter.batch_queue.statuses.get_mut(index) {
                    *status = BatchSplitStatus::Running;
                }
                self.worker.progress_ratio = index as f32 / total.max(1) as f32;
                *pending_status = Some((
                    StatusLevel::Info,
                    format!("Нарезка [{}/{}]: {}...", index + 1, total, name),
                ));
                false
            }
            BatchSplitProgress::ConfirmLargeExport { parts } => {
                let approved = super::confirm_destructive(&format!(
                    "Будет создано {parts} файлов в папке назначения. Продолжить?"
                ));
                if let Some(tx) = self.worker.split_confirm_tx.as_ref() {
                    let _ = tx.send(approved);
                }
                false
            }
            BatchSplitProgress::FileFinished { index, result } => {
                if let Some(status) = self.splitter.batch_queue.statuses.get_mut(index) {
                    *status = if result.errors.is_empty() {
                        BatchSplitStatus::Done(result.saved)
                    } else {
                        BatchSplitStatus::Partial {
                            saved: result.saved,
                            error: result
                                .errors
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("; "),
                        }
                    };
                }
                let total = self.splitter.batch_queue.statuses.len().max(1);
                self.worker.progress_ratio = (index + 1) as f32 / total as f32;
                false
            }
            BatchSplitProgress::Finished(outcome) => {
                self.splitter.batch_queue.running = false;
                self.splitter.batch_queue.cancel_requested = false;
                self.splitter.batch_queue.warning = None;
                self.splitter.batch_queue.error = None;
                for status in &mut self.splitter.batch_queue.statuses {
                    if matches!(status, BatchSplitStatus::Running) {
                        *status = BatchSplitStatus::Queued;
                    }
                }
                self.splitter.batch_queue.summary = Some(outcome);
                self.worker.cancel_flag = None;
                self.worker.split_confirm_tx = None;
                self.worker.is_busy = false;
                let counts = outcome.counts();
                let total = self.splitter.batch_queue.statuses.len().max(1);
                let finished = self.splitter.batch_queue.done_count()
                    + self.splitter.batch_queue.failed_count();
                self.worker.progress_ratio = if outcome.is_cancelled() {
                    finished as f32 / total as f32
                } else {
                    1.0
                };
                let level = if outcome.is_cancelled() || counts.failed > 0 {
                    StatusLevel::Warning
                } else {
                    StatusLevel::Success
                };
                let saved = self.splitter.batch_queue.saved_count();
                let text = if outcome.is_cancelled() {
                    format!(
                        "Отменено: книг {}, ошибок {}, сохранено частей {}",
                        counts.succeeded, counts.failed, saved
                    )
                } else {
                    format!(
                        "Пакетный книгорез: книг {}, ошибок {}, сохранено частей {}",
                        counts.succeeded, counts.failed, saved
                    )
                };
                *pending_status = Some((level, text));
                true
            }
        }
    }

    fn handle_batch_setup_error(
        &mut self,
        error: String,
        pending_status: &mut PendingStatus,
    ) -> bool {
        self.batch.running = false;
        self.batch.error = Some(error.clone());
        self.worker.cancel_flag = None;
        self.worker.is_busy = false;
        self.worker.progress_ratio = 0.0;
        *pending_status = Some((StatusLevel::Error, format!("Пакетная обработка: {error}")));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AppSettings;
    use crate::ui::app::ProcessingSnapshot;

    fn test_app() -> DemagogApp {
        DemagogApp::from_parts(AppSettings::default(), BookDocument::default())
    }

    #[test]
    fn empty_receiver_restores_pending_worker_state() {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut app = test_app();
        app.worker.rx = Some(rx);
        app.worker.is_busy = true;
        app.worker.cancel_flag = Some(Arc::clone(&cancel));
        app.worker.progress_ratio = 0.4;
        app.processing.processing_snapshot = Some(ProcessingSnapshot::capture("исходник"));
        app.set_status(StatusLevel::Info, "В работе".to_string());

        app.poll_worker();

        assert!(app.worker.rx.is_some());
        assert!(app.worker.is_busy);
        assert!(Arc::ptr_eq(
            app.worker.cancel_flag.as_ref().expect("cancel flag"),
            &cancel
        ));
        assert_eq!(app.worker.progress_ratio, 0.4);
        assert!(app.processing.processing_snapshot.is_some());
        assert_eq!(app.view.status_line, "В работе");
        assert_eq!(app.view.status_level, StatusLevel::Info);
        assert!(
            tx.send(WorkerMessage::Pipeline(PipelineProgress::Cancelled))
                .is_ok()
        );
    }

    #[test]
    fn disconnected_receiver_clears_worker_state_and_reports_error() {
        let (tx, rx) = mpsc::channel();
        drop(tx);
        let mut app = test_app();
        app.worker.rx = Some(rx);
        app.worker.is_busy = true;
        app.worker.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
        app.worker.progress_ratio = 0.7;
        app.processing.processing_snapshot = Some(ProcessingSnapshot::capture("исходник"));

        app.poll_worker();

        assert!(app.worker.rx.is_none());
        assert!(app.worker.cancel_flag.is_none());
        assert!(!app.worker.is_busy);
        assert_eq!(app.worker.progress_ratio, 0.0);
        assert!(app.processing.processing_snapshot.is_none());
        assert_eq!(app.view.status_level, StatusLevel::Error);
        assert_eq!(
            app.view.status_line,
            "Рабочий поток завершился неожиданно — попробуйте ещё раз"
        );
        assert_eq!(app.view.last_error, Some(app.view.status_line.clone()));
    }
}

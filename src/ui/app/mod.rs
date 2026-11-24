use egui::{self, Color32, FontFamily, FontId, RichText, ScrollArea, TextStyle};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::domain::{
    AppSettings, BookDocument, ChunkConfig, ChunksExportPayload, DictionaryMeta, DictionaryTag,
    PipelineProgress, PipelineReport, StressFormat, ThemeMode,
};
use crate::engine::loader::{DictionaryLoadReport, load_dictionaries_parallel_with_cancel};
use crate::io::book_reader::{CompositeBookReader, SUPPORTED_BOOK_FORMATS, can_read_book};
use crate::io::settings_store;
use crate::ports::{DocumentReader, StressFormatter, TextChunker, TextProcessor};
use crate::presentation;
use crate::services::chunker::SentenceAwareChunker;
use crate::services::dictionary_check::{self, CheckReport};
use crate::services::pipeline::{PipelineOptions, ProcessingPipeline};
use crate::services::stress::RussianStressFormatter;
use crate::ui::palette;

mod batch;
mod splitter;
mod splitter_batch;
mod state;
mod views;
mod worker;

#[cfg(test)]
use state::ResultState;
use state::{DiffState, EditorState, ProcessingSnapshot, ProcessingState, SearchState, ViewState};
use worker::{WorkerMessage, WorkerOutcome, classify_worker_result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ActiveTab {
    #[default]
    RawText,
    ProcessedText,
    Chunks,
    BookCutter,
    DiffAndStats,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum StatusLevel {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

fn text_mutations_enabled(is_busy: bool) -> bool {
    !is_busy
}

struct LoadedDictionary {
    meta: DictionaryMeta,
    processor: Arc<dyn TextProcessor>,
}

fn chunks_for_text(text: &str, config: &ChunkConfig) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    SentenceAwareChunker.split_into_chunks(text, config)
}

fn can_export_chunks(text: &str, chunks: &[String]) -> bool {
    !text.trim().is_empty() && !chunks.is_empty()
}

pub struct DemagogApp {
    settings: AppSettings,
    dictionaries: Vec<LoadedDictionary>,
    editor: EditorState,
    view: ViewState,
    processing: ProcessingState,
    worker: worker::WorkerState,
    search: SearchState,
    diff: DiffState,
    batch: batch::BatchState,
    splitter: splitter::SplitterState,
}

impl DemagogApp {
    #[cfg(not(feature = "sw"))]
    #[cfg(feature = "eframe-backend")]
    pub fn new(cc: &eframe::CreationContext<'_>, initial_dic_folder: PathBuf) -> Self {
        Self::init(cc.egui_ctx.clone(), initial_dic_folder)
    }

    fn sample_book_document() -> BookDocument {
        let sample_text = concat!(
            "В 1995 году в замке у ворот города стояли все свои. ",
            "Они завоевывать земли не собирались, но руки на груди скрестили все.\n\n",
            "Абаренов посмотрел на берег реки: стоЯщих на столе кубков было ровно 12 штук. ",
            "Веселые места и весеннего ветра шум напоминали о том, что все уже в прошлом."
        )
        .to_string();
        BookDocument {
            title: "Пример текста".to_string(),
            source_path: None,
            raw_text: sample_text,
            processed_text: String::new(),
            chunks: Vec::new(),
            headings: Vec::new(),
        }
    }
    pub fn init(egui_ctx: egui::Context, initial_dic_folder: PathBuf) -> Self {
        configure_cyrillic_fonts(&egui_ctx);
        let (settings, settings_warning) = settings_store::load(initial_dic_folder);
        apply_style_settings(&egui_ctx, &settings);

        let document = Self::sample_book_document();

        let mut app = Self::from_parts(settings, document);
        app.set_status(
            StatusLevel::Info,
            "Загрузка и компиляция словарей на всех ядрах CPU...",
        );
        app.reload_dictionaries_async(egui_ctx);
        if let Some(warning) = settings_warning {
            app.set_status(StatusLevel::Error, warning.to_string());
        }
        let missing = app
            .settings
            .batch
            .queue
            .len()
            .saturating_sub(app.batch.items.len());
        if missing > 0 {
            app.set_status(
                StatusLevel::Warning,
                format!(
                    "Очередь пакетной обработки восстановлена: {} файлов, {} пропущено (не найдены)",
                    app.batch.items.len(),
                    missing
                ),
            );
        }
        app
    }

    fn from_parts(settings: AppSettings, document: BookDocument) -> Self {
        let result_format = settings.stress_format;
        let editor = EditorState::new(document, result_format);
        let (restored_items, _) = batch::restore_queue(&settings.batch.queue);
        let batch_state = batch::BatchState {
            open: false,
            items: restored_items,
            output_dir: settings.batch.output_dir.clone(),
            export_txt: settings.batch.export_txt,
            export_chunks: settings.batch.export_chunks,
            overwrite: settings.batch.overwrite,
            running: false,
            summary: None,
            error: None,
        };
        Self {
            settings,
            dictionaries: Vec::new(),
            editor,
            view: ViewState::default(),
            processing: ProcessingState::default(),
            worker: worker::WorkerState::default(),
            search: SearchState::default(),
            diff: DiffState::default(),
            batch: batch_state,
            splitter: splitter::SplitterState::default(),
        }
    }

    /// Сброс приложения в состояние свежего запуска: пример текста, пустая очередь,
    /// все состояния на начальные значения.
    pub(crate) fn reset_app(&mut self, ctx: egui::Context) {
        self.editor.reset_document(Self::sample_book_document());
        self.view.reset_document_state();
        self.processing = ProcessingState::default();
        self.worker = worker::WorkerState::default();
        self.diff.reset();
        self.search.reset();
        self.splitter = splitter::SplitterState::default();
        self.batch = batch::BatchState::default();
        self.dictionaries.clear();
        self.set_status(
            StatusLevel::Info,
            "Загрузка и компиляция словарей на всех ядрах CPU...",
        );
        self.reload_dictionaries_async(ctx);
    }

    fn reload_dictionaries_async(&mut self, ctx: egui::Context) {
        if self.worker.is_busy {
            return;
        }
        self.worker.is_busy = true;
        self.worker.progress_ratio = 0.1;
        self.worker.dic_load_errors.clear();
        // Drop the old set so a failed reload cannot leave dictionaries from
        // the previous folder usable for processing.
        self.dictionaries.clear();
        let dic_folder = self.settings.dic_folder.display().to_string();
        self.set_status(
            StatusLevel::Info,
            format!("Параллельная компиляция словарей из {dic_folder}..."),
        );

        let folder = self.settings.dic_folder.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.worker.cancel_flag = Some(Arc::clone(&cancel));
        let (tx, rx) = mpsc::channel();
        self.worker.rx = Some(rx);

        std::thread::spawn(move || {
            let start = Instant::now();
            let result = load_dictionaries_parallel_with_cancel(&folder, Some(cancel.as_ref()));
            match classify_worker_result(result, Some(cancel.as_ref())) {
                WorkerOutcome::Cancelled => {
                    let _ = tx.send(WorkerMessage::Pipeline(PipelineProgress::Cancelled));
                }
                WorkerOutcome::Completed(report) => {
                    let DictionaryLoadReport { compiled, errors } = report;
                    let elapsed_ms = start.elapsed().as_millis();
                    let _ = tx.send(WorkerMessage::DictionariesCompiled {
                        compiled,
                        errors,
                        elapsed_ms,
                    });
                }
                WorkerOutcome::Failed(error) => {
                    let _ = tx.send(WorkerMessage::Pipeline(PipelineProgress::Error(
                        error.to_string(),
                    )));
                }
            }
            ctx.request_repaint();
        });
    }

    fn check_dictionaries_async(&mut self, ctx: egui::Context) {
        if self.worker.is_busy {
            return;
        }
        self.worker.is_busy = true;
        self.worker.progress_ratio = 0.25;
        self.view.check_report = None;
        self.view.show_check_window = false;
        self.set_status(
            StatusLevel::Info,
            "Проверка порядка словарей и эталонных фраз...",
        );
        let folder = self.settings.dic_folder.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.worker.cancel_flag = Some(Arc::clone(&cancel));
        let (tx, rx) = mpsc::channel();
        self.worker.rx = Some(rx);
        std::thread::spawn(move || {
            let result = dictionary_check::run_check_with_cancel(&folder, Some(cancel.as_ref()));
            match classify_worker_result(result, Some(cancel.as_ref())) {
                WorkerOutcome::Cancelled => {
                    let _ = tx.send(WorkerMessage::Pipeline(PipelineProgress::Cancelled));
                }
                WorkerOutcome::Completed(report) => {
                    let _ = tx.send(WorkerMessage::DictionaryCheckFinished(Ok(report)));
                }
                WorkerOutcome::Failed(error) => {
                    let _ = tx.send(WorkerMessage::DictionaryCheckFinished(Err(
                        error.to_string()
                    )));
                }
            }
            ctx.request_repaint();
        });
    }

    fn show_check_window(&mut self, ctx: &egui::Context) {
        if !self.view.show_check_window {
            return;
        }
        let Some(report) = self.view.check_report.as_ref() else {
            self.view.show_check_window = false;
            return;
        };

        let mut open = true;
        let mut close_requested = false;
        let mut copy_text: Option<String> = None;
        egui::Window::new("🔎 Проверка словарей")
            .open(&mut open)
            .resizable(true)
            .default_width(820.0)
            .default_height(640.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("Словарей: {}", report.loaded)).strong());
                    ui.label(format!("Правил: {}", report.total_rules));
                    ui.label(format!(
                        "Эталонных фраз: {}/{}",
                        report.passed,
                        dictionary_check::GOLDEN_CASES.len()
                    ));
                });

                let error_color = ui.visuals().error_fg_color;
                let warn_color = ui.visuals().warn_fg_color;
                ScrollArea::vertical()
                    .id_salt("dic_check_messages")
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for failure in &report.order.failures {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!(
                                        "• {}",
                                        presentation::check_diagnostic_text(failure)
                                    ))
                                    .color(error_color),
                                )
                                .selectable(true),
                            );
                        }
                        for warning in &report.order.warnings {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!(
                                        "• {}",
                                        presentation::check_diagnostic_text(warning)
                                    ))
                                    .color(warn_color),
                                )
                                .selectable(true),
                            );
                        }
                    });

                ui.add_space(6.0);
                ui.separator();
                ui.heading("Порядок обработки");
                ScrollArea::vertical()
                    .id_salt("dic_check_order")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for (index, dictionary) in report.order.ordered.iter().enumerate() {
                            ui.label(format!(
                                "{:2}. [{}] {}",
                                index + 1,
                                presentation::dictionary_stage_label(dictionary.stage),
                                dictionary.name
                            ));
                        }
                    });

                ui.add_space(6.0);
                ui.separator();
                ui.heading("Фонетические кандидаты");
                ui.label(
                    "Правила, меняющие буквы или границы слов. Это кандидаты, а не обязательно \
                     ошибки: словари с пометкой «по назначению» меняют текст намеренно.",
                );
                ui.label(format!(
                    "Кандидатов: {} из {} правил в обычных словарях.",
                    report.audit.total_candidates, report.audit.scanned_rules
                ));
                if !report.audit.skipped.is_empty() {
                    egui::CollapsingHeader::new(format!(
                        "Не аудировались по назначению: {} словарей",
                        report.audit.skipped.len()
                    ))
                    .default_open(false)
                    .show(ui, |ui| {
                        for (name, purpose) in &report.audit.skipped {
                            ui.label(format!(
                                "{name} [{}]",
                                presentation::dictionary_purpose_label(*purpose)
                            ));
                        }
                    });
                }

                let mut dictionaries: Vec<_> = report
                    .audit
                    .dictionaries
                    .iter()
                    .filter(|dictionary| !dictionary.candidates.is_empty())
                    .collect();
                dictionaries
                    .sort_by_key(|dictionary| std::cmp::Reverse(dictionary.candidates.len()));

                ScrollArea::vertical()
                    .id_salt("dic_check_audit")
                    .max_height(260.0)
                    .show(ui, |ui| {
                        if dictionaries.is_empty() {
                            ui.label("Кандидатов не найдено.");
                        }
                        for dictionary in dictionaries {
                            let title = format!(
                                "{} — {} из {} [{}]",
                                dictionary.name,
                                dictionary.candidates.len(),
                                dictionary.total_rules,
                                presentation::dictionary_purpose_label(dictionary.purpose)
                            );
                            egui::CollapsingHeader::new(title)
                                .default_open(false)
                                .show(ui, |ui| {
                                    for candidate in dictionary.candidates.iter().take(200) {
                                        ui.add(
                                            egui::Label::new(format!(
                                                "стр. {}: {} → {} [{}]",
                                                candidate.line,
                                                candidate.lhs,
                                                candidate.rhs,
                                                presentation::audit_kind_label(candidate.kind)
                                            ))
                                            .selectable(true),
                                        );
                                    }
                                    if dictionary.candidates.len() > 200 {
                                        ui.label(format!(
                                            "… ещё {}",
                                            dictionary.candidates.len() - 200
                                        ));
                                    }
                                });
                        }
                    });

                ui.add_space(6.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("📋 Скопировать аудит").clicked() {
                        copy_text = Some(presentation::audit_report_text(&report.audit));
                    }
                    if ui.button("Закрыть").clicked() {
                        close_requested = true;
                    }
                });
            });

        self.view.show_check_window = open && !close_requested;
        if let Some(text) = copy_text {
            self.copy_to_clipboard(&text, "Аудит словарей скопирован".to_string(), None);
        }
    }

    fn apply_preset(&mut self, preset: DictionaryPreset) {
        for d in &mut self.dictionaries {
            d.meta.enabled = preset_enables(preset, d.meta.tag, &d.meta.name);
        }
        let previous_format = self.settings.stress_format;
        self.settings.stress_format = match preset {
            DictionaryPreset::All | DictionaryPreset::HomographsOnly => {
                StressFormat::PlusBeforeVowel
            }
            DictionaryPreset::NumbersAndYo => StressFormat::CleanYoOnly,
            DictionaryPreset::Clear => self.settings.stress_format,
        };
        self.sync_disabled_dictionaries();
        self.mark_result_stale();
        if self.settings.stress_format != previous_format && !self.reformat_result() {
            self.settings.stress_format = previous_format;
        }
        self.save_settings();
    }

    fn start_processing(&mut self, ctx: egui::Context, capture_diff: bool) {
        if self.worker.is_busy {
            return;
        }
        let active_processors: Vec<Arc<dyn TextProcessor>> = self
            .dictionaries
            .iter()
            .filter(|d| d.meta.enabled)
            .map(|d| Arc::clone(&d.processor))
            .collect();

        if active_processors.is_empty() {
            self.set_status(StatusLevel::Warning, "Не выбран ни один словарь!");
            return;
        }
        if self.editor.result_state.is_edited()
            && !confirm_destructive(
                "Запустить обработку заново?\nРучные правки в результате будут потеряны.",
            )
        {
            return;
        }

        self.sync_disabled_dictionaries();
        self.save_settings();

        self.worker.is_busy = true;
        self.worker.progress_ratio = 0.0;
        self.processing.report = PipelineReport::default();
        self.reset_long_list_state();
        for d in &mut self.dictionaries {
            d.meta.last_ms = None;
            d.meta.last_replacements = None;
        }

        let input_text = self.editor.document.raw_text.clone();
        self.processing.processing_snapshot = Some(ProcessingSnapshot::capture(&input_text));
        let stress_format = self.settings.stress_format;
        let chunk_config = self.settings.chunk_config.clone();
        let cleanup = self.settings.cleanup;
        let cancel = Arc::new(AtomicBool::new(false));
        self.worker.cancel_flag = Some(Arc::clone(&cancel));
        let (tx, rx) = mpsc::channel();
        self.worker.rx = Some(rx);

        std::thread::spawn(move || {
            let (ptx, prx) = mpsc::channel();
            let tx_forward = tx.clone();
            let ctx_forward = ctx.clone();

            let forwarder = std::thread::spawn(move || {
                while let Ok(msg) = prx.recv() {
                    let _ = tx_forward.send(WorkerMessage::Pipeline(msg));
                    ctx_forward.request_repaint();
                }
            });

            let pipeline = ProcessingPipeline::new(
                Arc::new(RussianStressFormatter),
                Arc::new(SentenceAwareChunker),
            );
            let result = pipeline.execute(
                &input_text,
                &active_processors,
                PipelineOptions {
                    stress_format,
                    chunk_config,
                    capture_diff,
                    cleanup,
                },
                Some(cancel.as_ref()),
                Some(&ptx),
            );
            match classify_worker_result(result, Some(cancel.as_ref())) {
                WorkerOutcome::Cancelled => {
                    let _ = ptx.send(PipelineProgress::Cancelled);
                }
                WorkerOutcome::Completed(result) => {
                    let _ = ptx.send(PipelineProgress::Completed(result));
                }
                WorkerOutcome::Failed(error) => {
                    let _ = ptx.send(PipelineProgress::Error(error.to_string()));
                }
            }
            drop(ptx);
            let _ = forwarder.join();
        });
    }

    fn open_book_dialog(&mut self, ctx: egui::Context) {
        if self.worker.is_busy {
            return;
        }

        let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "Книги и текст",
                &["txt", "fb2", "epub", "docx", "zip", "md", "log"],
            )
            .pick_file()
        else {
            return;
        };

        self.load_book(path, ctx);
    }

    fn load_book(&mut self, path: PathBuf, ctx: egui::Context) {
        if self.worker.is_busy {
            return;
        }
        if !can_read_book(&path) {
            self.set_status(StatusLevel::Warning, SUPPORTED_BOOK_FORMATS.to_string());
            return;
        }
        if self.has_unsaved_work()
            && !confirm_destructive(
                "Открыть другую книгу?\nНесохранённые изменения будут потеряны.",
            )
        {
            return;
        }

        self.worker.is_busy = true;
        self.worker.progress_ratio = 0.0;
        self.set_status(StatusLevel::Info, format!("Чтение «{}»...", path.display()));

        let cancel = Arc::new(AtomicBool::new(false));
        self.worker.cancel_flag = Some(Arc::clone(&cancel));
        let (tx, rx) = mpsc::channel();
        self.worker.rx = Some(rx);
        std::thread::spawn(move || {
            let reader = CompositeBookReader::new();
            let result = reader.read_document_with_cancel(&path, Some(cancel.as_ref()));
            match classify_worker_result(result, Some(cancel.as_ref())) {
                WorkerOutcome::Cancelled => {
                    let _ = tx.send(WorkerMessage::Pipeline(PipelineProgress::Cancelled));
                }
                WorkerOutcome::Completed(document) => {
                    let _ = tx.send(WorkerMessage::BookLoaded(Ok(document)));
                }
                WorkerOutcome::Failed(error) => {
                    let _ = tx.send(WorkerMessage::BookLoaded(Err(error.to_string())));
                }
            }
            ctx.request_repaint();
        });
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};

        let open = KeyboardShortcut::new(Modifiers::CTRL, Key::O);
        let save = KeyboardShortcut::new(Modifiers::CTRL, Key::S);
        let save_source = KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::S);
        let export = KeyboardShortcut::new(Modifiers::CTRL, Key::E);
        let process = KeyboardShortcut::new(Modifiers::CTRL, Key::Enter);
        let process_diff = KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::Enter);

        if !self.worker.is_busy && ctx.input_mut(|i| i.consume_shortcut(&open)) {
            self.open_book_dialog(ctx.clone());
        }
        if !self.worker.is_busy && ctx.input_mut(|i| i.consume_shortcut(&process_diff)) {
            self.processing.capture_diff_on_run = true;
            self.start_processing(ctx.clone(), true);
        }
        if !self.worker.is_busy && ctx.input_mut(|i| i.consume_shortcut(&process)) {
            self.processing.capture_diff_on_run = false;
            self.start_processing(ctx.clone(), false);
        }
        if !self.worker.is_busy && ctx.input_mut(|i| i.consume_shortcut(&save_source)) {
            self.save_source_txt();
        }
        if !self.worker.is_busy && ctx.input_mut(|i| i.consume_shortcut(&save)) {
            self.save_processed_txt();
        }
        if !self.worker.is_busy && ctx.input_mut(|i| i.consume_shortcut(&export)) {
            self.export_chunks_json();
        }

        let find = KeyboardShortcut::new(Modifiers::CTRL, Key::F);
        if ctx.input_mut(|i| i.consume_shortcut(&find)) {
            if self.view.active_tab != ActiveTab::RawText
                && self.view.active_tab != ActiveTab::ProcessedText
            {
                self.view.active_tab = if self.editor.document.processed_text.is_empty() {
                    ActiveTab::RawText
                } else {
                    ActiveTab::ProcessedText
                };
            }
            self.search.find_open = true;
            self.view.focus_find = true;
            self.search.find_dirty = true;
            self.search.find_last_edit = Instant::now();
        }
        if self.search.find_open {
            let find_next = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
            let find_prev = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
            if ctx.input_mut(|i| i.consume_shortcut(&find_prev)) {
                self.step_find(-1);
            } else if ctx.input_mut(|i| i.consume_shortcut(&find_next)) {
                self.step_find(1);
            }
        }

        if let Some(cancel) = self.worker.cancel_flag.clone()
            && !cancel.load(Ordering::Relaxed)
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            cancel.store(true, Ordering::Relaxed);
            self.set_status(StatusLevel::Warning, "Отмена...");
        }
        // A second Esc (cancel already pending, or nothing running) closes find.
        if self.search.find_open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.search.find_open = false;
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = window_title(
            &self.editor.document.title,
            self.editor.document.source_path.is_some(),
        );
        if title != self.view.last_window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.view.last_window_title = title;
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        if self.batch.open {
            if self.batch.running {
                self.set_status(
                    StatusLevel::Warning,
                    "Дождитесь завершения пакетной обработки",
                );
                return;
            }
            let added = dropped.iter().filter(|path| can_read_book(path)).count();
            self.batch.add_files(dropped);
            self.persist_batch();
            if added > 0 {
                self.set_status(
                    StatusLevel::Info,
                    format!("В очередь добавлено файлов: {added}"),
                );
            } else {
                self.set_status(StatusLevel::Warning, SUPPORTED_BOOK_FORMATS.to_string());
            }
            return;
        }
        if self.worker.is_busy {
            self.set_status(
                StatusLevel::Warning,
                "Дождитесь завершения текущей операции",
            );
            return;
        }
        let Some(path) = dropped.into_iter().find(|path| can_read_book(path)) else {
            self.set_status(StatusLevel::Warning, SUPPORTED_BOOK_FORMATS.to_string());
            return;
        };
        self.load_book(path, ctx.clone());
    }

    fn refresh_raw_char_count(&mut self) {
        self.editor.raw_char_count = self.editor.document.raw_text.chars().count();
    }

    fn refresh_processed_char_count(&mut self) {
        self.editor.processed_char_count = self.editor.document.processed_text.chars().count();
    }

    fn reset_long_list_state(&mut self) {
        self.diff.diff_cache_valid = false;
    }

    fn refresh_diff_visible(&mut self) {
        let filter = self.diff.diff_filter.to_lowercase();
        self.diff.diff_visible.clear();
        for (index, event) in self.processing.report.events.iter().enumerate() {
            if filter.is_empty()
                || event.dictionary_name.to_lowercase().contains(&filter)
                || event.before_snippet.to_lowercase().contains(&filter)
                || event.after_snippet.to_lowercase().contains(&filter)
                || event.rule_summary.to_lowercase().contains(&filter)
            {
                self.diff.diff_visible.push(index);
            }
        }
        self.diff.diff_filter_cache = filter;
        self.diff.diff_cache_valid = true;
    }

    fn refresh_find_matches(&mut self) {
        let query = self.search.find_query.clone();
        let text = match self.view.active_tab {
            ActiveTab::RawText => self.editor.document.raw_text.as_str(),
            ActiveTab::ProcessedText => self.editor.document.processed_text.as_str(),
            _ => "",
        };
        let matches = find_matches(text, &query);
        self.search.find_matches = matches;
        self.search.find_current = 0;
        self.search.find_go_to = None;
        self.search.find_for_tab = Some(self.view.active_tab);
    }

    fn step_find(&mut self, delta: isize) {
        if self.search.find_dirty {
            self.refresh_find_matches();
            self.search.find_dirty = false;
            if !self.search.find_matches.ranges.is_empty() {
                self.search.find_current = 0;
                self.search.find_go_to = Some(0);
            }
            return;
        }
        let total = self.search.find_matches.ranges.len();
        if total == 0 {
            return;
        }
        let current = self.search.find_current.min(total - 1);
        self.search.find_current = (current as isize + delta).rem_euclid(total as isize) as usize;
        self.search.find_go_to = Some(self.search.find_current);
    }

    fn scroll_to_find_match(
        &mut self,
        ui: &egui::Ui,
        output: &egui::widgets::text_edit::TextEditOutput,
    ) {
        let Some(index) = self.search.find_go_to else {
            return;
        };
        let char_positions = {
            let text = match self.view.active_tab {
                ActiveTab::RawText => &self.editor.document.raw_text,
                ActiveTab::ProcessedText => &self.editor.document.processed_text,
                _ => "",
            };
            self.search
                .find_matches
                .ranges
                .get(index)
                .and_then(|&(start, end)| {
                    Some((
                        text.get(..start)?.chars().count(),
                        text.get(..end)?.chars().count(),
                    ))
                })
        };
        self.search.find_go_to = None;
        let Some((char_start, char_end)) = char_positions else {
            return;
        };
        let mut state = output.state.clone();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(char_start),
                egui::text::CCursor::new(char_end),
            )));
        state.store(ui.ctx(), output.response.id);
        let layout_cursor = output
            .galley
            .layout_from_cursor(egui::text::CCursor::new(char_start));
        let rect = output
            .galley
            .pos_from_layout_cursor(&layout_cursor)
            .translate(output.galley_pos.to_vec2());
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
    }

    fn show_find_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let label = ui.label("Найти:");
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut self.search.find_query)
                        .hint_text("Строка или фрагмент")
                        .desired_width(240.0),
                )
                .labelled_by(label.id);
            if self.view.focus_find {
                response.request_focus();
                self.view.focus_find = false;
            }
            if response.changed() {
                self.search.find_dirty = true;
                self.search.find_last_edit = Instant::now();
            }
            let debounce = Duration::from_millis(150);
            if self.search.find_dirty || self.search.find_for_tab != Some(self.view.active_tab) {
                if self.search.find_query.trim().is_empty()
                    || self.search.find_last_edit.elapsed() >= debounce
                {
                    self.refresh_find_matches();
                    self.search.find_dirty = false;
                } else {
                    ui.ctx().request_repaint_after(debounce);
                }
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let step = if ui.input(|i| i.modifiers.shift) {
                    -1
                } else {
                    1
                };
                self.step_find(step);
                self.view.focus_find = true;
            }

            let total = self.search.find_matches.ranges.len();
            let counter = if self.search.find_query.trim().is_empty() {
                "Введите строку".to_string()
            } else if total == 0 {
                "Совпадений не найдено".to_string()
            } else {
                format!(
                    "{} из {}",
                    format_num((self.search.find_current + 1).min(total)),
                    format_num(total)
                )
            };
            ui.label(RichText::new(counter).weak());

            let prev = ui
                .add_enabled(total > 0, egui::Button::new("◀"))
                .on_hover_text("Предыдущее совпадение (Shift+Enter)");
            prev.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    total > 0,
                    "Предыдущее совпадение",
                )
            });
            if prev.clicked() {
                self.step_find(-1);
            }
            let next = ui
                .add_enabled(total > 0, egui::Button::new("▶"))
                .on_hover_text("Следующее совпадение (Enter, F3)");
            next.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    total > 0,
                    "Следующее совпадение",
                )
            });
            if next.clicked() {
                self.step_find(1);
            }
            if self.search.find_matches.truncated {
                ui.label(
                    RichText::new(format!("показаны первые {}", format_num(MAX_FIND_MATCHES)))
                        .small()
                        .weak(),
                );
            }

            let close = ui.button("✖").on_hover_text("Закрыть поиск (Esc)");
            close.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Закрыть поиск")
            });
            if close.clicked() {
                self.search.find_open = false;
            }
        });
    }

    fn save_processed_txt(&mut self) {
        if self.editor.document.processed_text.is_empty() {
            self.set_status(StatusLevel::Warning, "Сначала выполните обработку текста!");
            return;
        }
        if !self.confirm_stale_result("Сохранить его всё равно") {
            return;
        }
        let default_name = format!("{}_processed.txt", self.editor.document.title);
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Текстовый файл UTF-8", &["txt"])
            .save_file()
        {
            match std::fs::write(&path, &self.editor.document.processed_text) {
                Ok(_) => {
                    self.editor.result_state.mark_saved();
                    self.set_status(
                        StatusLevel::Success,
                        format!("Сохранено в {}", path.display()),
                    );
                }
                Err(e) => {
                    self.set_status(StatusLevel::Error, format!("Ошибка сохранения: {e}"));
                }
            }
        }
    }

    fn save_source_txt(&mut self) {
        if self.editor.document.raw_text.trim().is_empty() {
            self.set_status(
                StatusLevel::Warning,
                "Исходный текст пуст — нечего сохранять",
            );
            return;
        }
        let default_name = format!("{}_source.txt", self.editor.document.title);
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Текстовый файл UTF-8", &["txt"])
            .save_file()
        {
            match std::fs::write(&path, &self.editor.document.raw_text) {
                Ok(_) => {
                    self.editor.document.source_path = Some(path.clone());
                    self.editor.source_dirty = false;
                    self.set_status(
                        StatusLevel::Success,
                        format!("Исходный текст сохранён в {}", path.display()),
                    );
                }
                Err(e) => self.set_status(StatusLevel::Error, format!("Ошибка сохранения: {e}")),
            }
        }
    }

    fn export_chunks_json(&mut self) {
        if self.editor.document.processed_text.trim().is_empty() {
            self.set_status(
                StatusLevel::Warning,
                "Результат пуст — экспортировать нечего",
            );
            return;
        }
        if !self.confirm_stale_result("Экспортировать его всё равно") {
            return;
        }
        if self.editor.result_state.is_chunks_stale() {
            self.rechunk_result();
        }
        if !can_export_chunks(
            &self.editor.document.processed_text,
            &self.editor.document.chunks,
        ) {
            self.set_status(
                StatusLevel::Warning,
                "Нет чанков для экспорта — запустите обработку!",
            );
            return;
        }
        let default_name = format!("{}_chunks.json", self.editor.document.title);
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("JSON для ComfyUI", &["json"])
            .save_file()
        {
            let payload = ChunksExportPayload {
                title: &self.editor.document.title,
                stress_format: self.editor.result_state.format().export_label(),
                chunk_count: self.editor.document.chunks.len(),
                chunks: &self.editor.document.chunks,
            };
            if let Ok(json) = serde_json::to_string_pretty(&payload) {
                match std::fs::write(&path, json) {
                    Ok(_) => {
                        // Chunks do not contain manual edits to processed_text,
                        // so an edited result stays dirty after exporting.
                        self.editor.result_state.mark_exported();
                        self.set_status(
                            StatusLevel::Success,
                            format!("Чанки экспортированы в {}", path.display()),
                        );
                    }
                    Err(e) => {
                        self.set_status(StatusLevel::Error, format!("Ошибка записи JSON: {e}"))
                    }
                }
            }
        }
    }

    /// Rebuild the result from the pipeline output using the current stress format.
    /// Returns false when the user declined or the text can no longer be reformatted.
    fn reformat_result(&mut self) -> bool {
        if self.editor.document.processed_text.is_empty()
            && self.editor.raw_pipeline_output.is_empty()
        {
            return true;
        }
        if self.editor.raw_pipeline_output.is_empty() {
            self.editor.result_state.mark_stale();
            self.set_status(
                StatusLevel::Warning,
                "Результат нельзя пересобрать: он уже снят с конвейера — обработайте книгу заново",
            );
            return false;
        }
        if self.editor.result_state.is_edited()
            && !confirm_destructive(
                "Пересобрать результат по текущему формату?\nРучные правки в результате будут потеряны.",
            )
        {
            return false;
        }
        let formatter = RussianStressFormatter;
        self.editor.document.processed_text = formatter.format_stress(
            &self.editor.raw_pipeline_output,
            self.settings.stress_format,
        );
        self.refresh_processed_char_count();
        self.editor
            .result_state
            .mark_reformatted(self.settings.stress_format);
        self.rechunk_result();
        true
    }

    /// Re-chunk the current result without touching its text.
    fn rechunk_result(&mut self) {
        self.editor.document.chunks = chunks_for_text(
            &self.editor.document.processed_text,
            &self.settings.chunk_config,
        );
        self.view.last_copied_chunk = None;
        self.editor.result_state.mark_rechunked();
        self.search.find_dirty = true;
    }

    /// Apply chunk settings when a drag ends instead of on every slider tick.
    fn apply_chunk_config_change(&mut self, ui: &egui::Ui, changed: bool) {
        if changed {
            self.editor.chunk_apply_pending = true;
        }
        if !self.editor.chunk_apply_pending {
            return;
        }
        if ui.ctx().input(|i| i.pointer.primary_down()) {
            return;
        }
        self.editor.chunk_apply_pending = false;
        self.rechunk_result();
        self.save_settings();
    }
}

#[cfg(feature = "eframe-backend")]
impl eframe::App for DemagogApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.update_ui(ui);
    }
}

impl DemagogApp {
    fn set_status(&mut self, level: StatusLevel, text: impl Into<String>) {
        self.view.status_line = text.into();
        self.view.status_level = level;
        if level == StatusLevel::Error {
            self.view.last_error = Some(self.view.status_line.clone());
        }
    }

    fn sync_disabled_dictionaries(&mut self) {
        // Keep disabled entries for dictionaries that are not currently loaded
        // (failed/partial/empty load); otherwise a bad load would silently
        // re-enable everything the user had turned off.
        let loaded: std::collections::HashMap<&str, bool> = self
            .dictionaries
            .iter()
            .map(|d| (d.meta.name.as_str(), d.meta.enabled))
            .collect();
        let mut disabled: Vec<String> = self
            .settings
            .disabled_dictionaries
            .iter()
            .filter(|name| !loaded.contains_key(name.as_str()) || !loaded[name.as_str()])
            .cloned()
            .collect();
        for d in &self.dictionaries {
            if !d.meta.enabled && !disabled.contains(&d.meta.name) {
                disabled.push(d.meta.name.clone());
            }
        }
        self.settings.disabled_dictionaries = disabled;
    }

    fn save_settings(&mut self) {
        if let Err(err) = settings_store::save(&self.settings) {
            self.set_status(
                StatusLevel::Error,
                format!("Не удалось сохранить настройки: {err}"),
            );
        }
    }

    fn choose_dictionary_folder(&mut self, ctx: egui::Context) {
        if self.worker.is_busy {
            return;
        }
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.settings.dic_folder = dir;
            self.save_settings();
            self.reload_dictionaries_async(ctx);
        }
    }

    fn has_unsaved_work(&self) -> bool {
        self.editor.source_dirty || self.editor.result_state.is_dirty()
    }

    fn mark_result_stale(&mut self) {
        if !self.editor.document.processed_text.is_empty()
            || !self.editor.document.chunks.is_empty()
        {
            self.editor.result_state.mark_stale();
        }
    }

    fn confirm_stale_result(&self, action: &str) -> bool {
        if !self.editor.result_state.is_stale() {
            return true;
        }
        confirm_destructive(&format!(
            "Результат устарел: исходный текст или набор словарей изменились.\n{action}?"
        ))
    }

    fn copy_to_clipboard(&mut self, text: &str, success: String, chunk: Option<usize>) {
        match arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text.to_string()))
        {
            Ok(()) => {
                if let Some(index) = chunk {
                    self.view.last_copied_chunk = Some(index);
                }
                self.set_status(StatusLevel::Success, success);
            }
            Err(err) => {
                self.set_status(
                    StatusLevel::Error,
                    format!("Не удалось скопировать в буфер обмена: {err}"),
                );
            }
        }
    }

    fn show_status_left(&mut self, ui: &mut egui::Ui, theme: ThemeMode) {
        if self.worker.is_busy {
            ui.spinner();
            ui.add(
                egui::ProgressBar::new(self.worker.progress_ratio)
                    .desired_width(180.0)
                    .show_percentage(),
            );
            if let Some(cancel) = self.worker.cancel_flag.clone() {
                let requested = cancel.load(Ordering::Relaxed);
                if ui
                    .add_enabled(!requested, egui::Button::new("Отмена"))
                    .on_hover_text("Отменить текущую операцию (Esc)")
                    .clicked()
                {
                    cancel.store(true, Ordering::Relaxed);
                    self.set_status(StatusLevel::Warning, "Отмена...");
                }
            }
        }
        let (icon, color) = match self.view.status_level {
            StatusLevel::Info => ("", ui.visuals().text_color()),
            StatusLevel::Success => ("✅ ", palette::success_text(theme)),
            StatusLevel::Warning => ("⚠️ ", palette::warning_text(theme)),
            StatusLevel::Error => ("❌ ", palette::diff_before(theme)),
        };
        let status = ui.add(
            egui::Label::new(
                RichText::new(format!("{icon}{}", self.view.status_line)).color(color),
            )
            .selectable(true),
        );
        status.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, self.view.status_line.clone())
        });
        ui.ctx().accesskit_node_builder(status.id, |node| {
            node.set_role(egui::accesskit::Role::Status);
            node.set_live(status_live(self.worker.is_busy, self.view.status_level));
            node.set_live_atomic();
            node.set_label(self.view.status_line.clone());
        });
        if !self.worker.dic_load_errors.is_empty() {
            status.on_hover_text(self.worker.dic_load_errors.join("\n"));
        }
        if self.view.status_level != StatusLevel::Error
            && let Some(error) = self.view.last_error.clone()
        {
            let chip = ui
                .add(
                    egui::Button::new(
                        RichText::new("⚠ Ошибка")
                            .color(palette::diff_before(theme))
                            .strong(),
                    )
                    .frame(false),
                )
                .on_hover_text(&error);
            if chip.clicked() {
                rfd::MessageDialog::new()
                    .set_title("Demagog-RS — ошибка")
                    .set_level(rfd::MessageLevel::Error)
                    .set_buttons(rfd::MessageButtons::Ok)
                    .set_description(&error)
                    .show();
            }
            if ui
                .small_button("✖")
                .on_hover_text("Скрыть ошибку")
                .clicked()
            {
                self.view.last_error = None;
            }
        }
    }

    fn show_status_counters(&self, ui: &mut egui::Ui, theme: ThemeMode, counters: &str) {
        ui.label(RichText::new(counters).weak());
        if !self.worker.dic_load_errors.is_empty() {
            let details = self.worker.dic_load_errors.join("\n");
            let errors = ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "Ошибки загрузки: {}",
                        self.worker.dic_load_errors.len()
                    ))
                    .color(palette::warning_text(theme))
                    .strong(),
                )
                .sense(egui::Sense::click()),
            );
            errors.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    "Ошибки загрузки словарей",
                )
            });
            if errors.clicked() {
                rfd::MessageDialog::new()
                    .set_title("Demagog-RS — ошибки загрузки словарей")
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::Ok)
                    .set_description(&details)
                    .show();
            }
            errors.on_hover_text(details);
        }
    }

    fn show_stale_banner(&self, ui: &mut egui::Ui, theme: ThemeMode) {
        if !self.editor.result_state.is_stale() {
            return;
        }
        ui.add(
            egui::Label::new(
                RichText::new(
                    "⚠ Результат устарел: исходный текст или набор словарей изменились. Обработайте текст заново.",
                )
                .color(palette::warning_text(theme))
                .strong(),
            )
            .selectable(true),
        );
        ui.add_space(4.0);
    }

    pub(crate) fn shutdown(&mut self) {
        self.sync_disabled_dictionaries();
        self.save_settings();
        // Releasing millions of dictionary strings takes seconds on Windows.
        // The process is exiting, so skip the destructor walk and let the OS
        // reclaim the heap during normal backend teardown.
        std::mem::forget(std::mem::take(&mut self.dictionaries));
    }

    pub(crate) fn update_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_worker();
        if !self.view.window_icon_applied {
            self.view.window_icon_applied = super::window::apply_window_icon();
        }
        let theme = self.settings.theme();

        if ctx.input(|i| i.viewport().close_requested()) && self.has_unsaved_work() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if confirm_destructive(
                "Есть несохранённые изменения исходного текста или результата.\nВыйти без сохранения?",
            ) {
                self.shutdown();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }

        self.update_window_title(&ctx);
        self.handle_shortcuts(&ctx);
        self.handle_dropped_files(&ctx);

        self.show_top_toolbar(ui, &ctx, theme);

        self.show_status_bar(ui, theme);

        self.show_sidebar(ui, &ctx, theme);

        self.show_central_workspace(ui, &ctx, theme);

        self.show_check_window(&ctx);
        self.show_batch_window(&ctx);
    }
}

fn warm_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::light();
    visuals.dark_mode = false;
    visuals.panel_fill = palette::WARM_PANEL;
    visuals.window_fill = palette::WARM_WINDOW;
    visuals.extreme_bg_color = palette::WARM_TEXT_EDIT;
    visuals.text_edit_bg_color = Some(palette::WARM_TEXT_EDIT);
    visuals.faint_bg_color = palette::WARM_FAINT;
    visuals.code_bg_color = palette::WARM_CODE;
    visuals.weak_text_color = Some(palette::weak_text(ThemeMode::Warm));
    visuals.selection.bg_fill = Color32::from_rgb(224, 198, 154);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(120, 88, 52));
    visuals.hyperlink_color = Color32::from_rgb(139, 90, 43);
    visuals.warn_fg_color = Color32::from_rgb(154, 90, 0);
    visuals.error_fg_color = Color32::from_rgb(161, 58, 46);

    visuals.widgets.noninteractive.weak_bg_fill = palette::WARM_PANEL;
    visuals.widgets.noninteractive.bg_fill = palette::WARM_PANEL;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, palette::WARM_BORDER);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, palette::WARM_TEXT);

    visuals.widgets.inactive.weak_bg_fill = palette::WARM_WIDGET;
    visuals.widgets.inactive.bg_fill = palette::WARM_WIDGET;
    visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(205, 188, 165));
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(72, 60, 46));

    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(226, 211, 188);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(226, 211, 188);
    visuals.widgets.hovered.bg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(160, 135, 105));
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5_f32, Color32::from_rgb(47, 39, 31));

    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(211, 191, 161);
    visuals.widgets.active.bg_fill = Color32::from_rgb(211, 191, 161);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, Color32::BLACK);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(2.0_f32, Color32::BLACK);

    visuals.widgets.open.weak_bg_fill = Color32::from_rgb(238, 229, 214);
    visuals.widgets.open.bg_fill = Color32::from_rgb(238, 229, 214);
    visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(200, 185, 165));
    visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0_f32, Color32::BLACK);
    visuals
}

fn apply_style_settings(ctx: &egui::Context, settings: &AppSettings) {
    let theme = settings.theme();
    let egui_theme = if theme == ThemeMode::Dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(egui_theme);

    let mut visuals = match theme {
        ThemeMode::Dark => {
            let mut visuals = egui::Visuals::dark();
            visuals.weak_text_color = Some(palette::weak_text(theme));
            visuals
        }
        ThemeMode::Light => {
            let mut visuals = egui::Visuals::light();
            visuals.weak_text_color = Some(palette::weak_text(theme));
            visuals
        }
        ThemeMode::Warm => warm_visuals(),
    };
    visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0_f32, palette::focus_ring(theme));
    visuals.widgets.active.expansion = 1.0;
    ctx.set_visuals(visuals);

    let base = settings.ui_font_size;
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(base + 4.0, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(base, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(base, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new((base - 2.5).max(11.0), FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(settings.editor_font_size, FontFamily::Monospace),
        );
    });
}

fn configure_cyrillic_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\arial.ttf",
        r"C:\Windows\Fonts\tahoma.ttf",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "sys_cyrillic".to_owned(),
                egui::FontData::from_owned(bytes).into(),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "sys_cyrillic".to_owned());
            break;
        }
    }
    let mono_candidates = [
        r"C:\Windows\Fonts\consola.ttf",
        r"C:\Windows\Fonts\cour.ttf",
    ];
    for path in mono_candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "sys_mono".to_owned(),
                egui::FontData::from_owned(bytes).into(),
            );
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "sys_mono".to_owned());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn dictionary_display_key(name: &str) -> (u32, String) {
    let leading_num: u32 = name
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(u32::MAX);
    (leading_num, name.to_lowercase())
}

fn format_num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (count, ch) in s.chars().rev().enumerate() {
        if count > 0 && count % 3 == 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub fn locate_default_dic_folder() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let exe_dic = dir.join("dic");
        if exe_dic.is_dir() {
            return exe_dic;
        }
        if let Some(parent) = dir.parent().and_then(|p| p.parent()) {
            let workspace_dic = parent.join("dic");
            if workspace_dic.is_dir() {
                return workspace_dic;
            }
        }
    }
    let local = Path::new("dic");
    if local.is_dir() {
        return local.to_path_buf();
    }
    PathBuf::from("dic")
}

fn text_would_be_lost(text: &str) -> bool {
    !text.trim().is_empty()
}

fn theme_icon(theme: ThemeMode) -> &'static str {
    match theme {
        ThemeMode::Dark => "🌙",
        ThemeMode::Light => "☀",
        ThemeMode::Warm => "📖",
    }
}

fn status_live(is_busy: bool, level: StatusLevel) -> egui::accesskit::Live {
    if is_busy || level == StatusLevel::Info {
        egui::accesskit::Live::Off
    } else {
        egui::accesskit::Live::Polite
    }
}

fn reset_settings_preserving_folder(settings: &AppSettings) -> AppSettings {
    AppSettings {
        dic_folder: settings.dic_folder.clone(),
        ..AppSettings::default()
    }
}

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn window_title(document_title: &str, has_source: bool) -> String {
    if has_source {
        format!("Demagog-RS — {document_title}")
    } else {
        "Demagog-RS".to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DictionaryPreset {
    All,
    HomographsOnly,
    NumbersAndYo,
    Clear,
}

fn presets_menu(ui: &mut egui::Ui) -> Option<DictionaryPreset> {
    let mut selected = None;
    ui.menu_button("Пресеты", |ui| {
        if ui
            .button("Все (полный +)")
            .on_hover_text("Включить все словари, включая полную орфоэпию со знаками '+'")
            .clicked()
        {
            selected = Some(DictionaryPreset::All);
            ui.close();
        }
        if ui
            .button("Только омографы с '+' (без 95.NEW)")
            .on_hover_text(
                "Ставит '+' только в омографах и трудных словах, не трогая простые слова из 95.NEW (2.9 млн)",
            )
            .clicked()
        {
            selected = Some(DictionaryPreset::HomographsOnly);
            ui.close();
        }
        if ui
            .button("Числа + Ё (без ударений)")
            .on_hover_text("Оставляет числа и даты, ё-фикацию и чистку текста")
            .clicked()
        {
            selected = Some(DictionaryPreset::NumbersAndYo);
            ui.close();
        }
        ui.separator();
        if ui.button("Снять все").clicked() {
            selected = Some(DictionaryPreset::Clear);
            ui.close();
        }
    });
    selected
}

fn preset_enables(preset: DictionaryPreset, tag: DictionaryTag, name: &str) -> bool {
    match preset {
        DictionaryPreset::All => true,
        DictionaryPreset::HomographsOnly => !name.starts_with("95."),
        DictionaryPreset::NumbersAndYo => matches!(
            tag,
            DictionaryTag::NumbersAndDates
                | DictionaryTag::YoFication
                | DictionaryTag::CleanupAndTranslit
        ),
        DictionaryPreset::Clear => false,
    }
}

fn editor_line_height(font_size: f32) -> f32 {
    font_size * 1.45
}

const MAX_FIND_MATCHES: usize = 5000;

/// Uniform row height for chunk virtualization must cover the longest chunk;
/// estimating from the config limit overshoots when actual chunks are shorter.
fn estimated_chunk_lines(chunks: &[String], max_chars: usize, chars_per_line: f32) -> f32 {
    let longest = chunks
        .iter()
        .map(|chunk| chunk.chars().count())
        .max()
        .unwrap_or(max_chars);
    ((longest as f32 / chars_per_line).ceil()).max(1.0)
}

/// Font sliders persist on release/keyboard change, not on every drag tick.
fn should_save_font_settings(
    changed: bool,
    dragged: bool,
    drag_stopped: bool,
    lost_focus: bool,
) -> bool {
    drag_stopped || lost_focus || (changed && !dragged)
}

#[derive(Debug, Default)]
struct FindMatches {
    ranges: Vec<(usize, usize)>,
    truncated: bool,
}

/// Build a search pattern that ignores stress marks in the processed text:
/// a word may contain `+` before a vowel or a combining acute after it.
fn search_regex(query: &str) -> Option<regex::Regex> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let mut pattern = String::new();
    for ch in query.chars() {
        if ch != '+' {
            pattern.push_str(r"\+?");
        }
        match ch {
            'е' | 'Е' | 'ё' | 'Ё' => pattern.push_str("[её]"),
            _ => pattern.push_str(&regex::escape(&ch.to_string())),
        }
        pattern.push_str(r"\u{0301}?");
    }
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .build()
        .ok()
}

fn find_matches(text: &str, query: &str) -> FindMatches {
    let Some(regex) = search_regex(query) else {
        return FindMatches::default();
    };

    let mut ranges = Vec::new();
    for found in regex.find_iter(text) {
        if ranges.len() == MAX_FIND_MATCHES {
            return FindMatches {
                ranges,
                truncated: true,
            };
        }
        ranges.push((found.start(), found.end()));
    }
    FindMatches {
        ranges,
        truncated: false,
    }
}

fn layout_editor_text(
    text: &str,
    font: FontId,
    color: Color32,
    line_height: f32,
    matches: &[(usize, usize)],
    current: Option<usize>,
    wrap_width: f32,
) -> egui::text::LayoutJob {
    let base = egui::TextFormat {
        font_id: font,
        color,
        line_height: Some(line_height),
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;

    let mut cursor = 0;
    for (index, &(start, end)) in matches.iter().enumerate() {
        if start < cursor
            || start >= end
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            continue;
        }
        if start > cursor {
            job.append(&text[cursor..start], 0.0, base.clone());
        }
        let mut highlight = base.clone();
        highlight.background = if Some(index) == current {
            palette::find_current_highlight()
        } else {
            palette::find_highlight()
        };
        highlight.color = Color32::BLACK;
        job.append(&text[start..end], 0.0, highlight);
        cursor = end;
    }
    if cursor < text.len() {
        job.append(&text[cursor..], 0.0, base.clone());
    }
    if job.sections.is_empty() {
        job.append("", 0.0, base);
    }
    job
}

fn editor_layouter<'a>(
    font: FontId,
    text_color: Color32,
    line_height: f32,
    matches: &'a [(usize, usize)],
    current: Option<usize>,
) -> impl FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> Arc<egui::Galley> + 'a {
    move |ui, text, wrap_width| {
        let job = layout_editor_text(
            text.as_str(),
            font.clone(),
            text_color,
            line_height,
            matches,
            current,
            wrap_width,
        );
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    }
}

// Нативное модальное окно в тестах ждёт клика и вешает прогон, поэтому тесты
// подтверждают действие сразу; сама логика «спросить перед потерей текста»
// проверяется чистыми функциями (`text_would_be_lost` и т. п.).
#[cfg(test)]
fn confirm_destructive(_description: &str) -> bool {
    true
}

#[cfg(not(test))]
fn confirm_destructive(description: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title("Demagog-RS")
        .set_level(rfd::MessageLevel::Warning)
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_description(description)
        .show()
        == rfd::MessageDialogResult::Yes
}

#[cfg(test)]
mod worker_outcome_tests {
    use super::{WorkerOutcome, classify_worker_result};
    use crate::domain::AppError;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn typed_cancellation_is_cancelled_when_flag_is_false() {
        let cancel = AtomicBool::new(false);
        let result = Err(AppError::Cancelled("Чтение книги отменено"));

        assert_eq!(
            classify_worker_result::<()>(result, Some(&cancel)),
            WorkerOutcome::Cancelled
        );
    }

    #[test]
    fn cancellation_flag_wins_over_a_real_failure() {
        let cancel = AtomicBool::new(true);
        let result = Err(AppError::failed("Ошибка чтения книги"));

        assert_eq!(
            classify_worker_result::<()>(result, Some(&cancel)),
            WorkerOutcome::Cancelled
        );
    }

    #[test]
    fn real_failure_is_preserved_when_flag_is_false() {
        let cancel = AtomicBool::new(false);
        let error = AppError::failed("Ошибка чтения книги");

        assert_eq!(
            classify_worker_result::<()>(Err(error.clone()), Some(&cancel)),
            WorkerOutcome::Failed(error)
        );
    }

    #[test]
    fn successful_value_is_completed() {
        let cancel = AtomicBool::new(false);

        assert_eq!(
            classify_worker_result(Ok(42), Some(&cancel)),
            WorkerOutcome::Completed(42)
        );
    }
}

#[cfg(test)]
mod tests;

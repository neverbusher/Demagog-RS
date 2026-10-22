use egui::{self, Color32, FontFamily, FontId, RichText, ScrollArea, TextStyle};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::domain::{
    AppSettings, BookDocument, ChunkConfig, DictionaryMeta, DictionaryTag, PipelineProgress,
    PipelineReport, StressFormat, ThemeMode,
};
use crate::engine::loader::{CompiledDictionary, DictionaryLoadReport, load_dictionaries_parallel};
use crate::io::book_reader::{CompositeBookReader, SUPPORTED_BOOK_FORMATS, can_read_book};
use crate::ports::{DocumentReader, StressFormatter, TextChunker, TextProcessor};
use crate::services::chunker::SentenceAwareChunker;
use crate::services::pipeline::{PipelineOptions, ProcessingPipeline};
use crate::services::stress::RussianStressFormatter;
use crate::ui::palette;

mod views;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    RawText,
    ProcessedText,
    Chunks,
    DiffAndStats,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
struct ResultState {
    dirty: bool,
    stale: bool,
    edited: bool,
    chunks_stale: bool,
    format: StressFormat,
}

impl ResultState {
    fn new(format: StressFormat) -> Self {
        Self {
            dirty: false,
            stale: false,
            edited: false,
            chunks_stale: false,
            format,
        }
    }

    fn reset(&mut self) {
        self.dirty = false;
        self.stale = false;
        self.edited = false;
        self.chunks_stale = false;
    }

    fn mark_computed(&mut self, format: StressFormat) {
        self.dirty = true;
        self.stale = false;
        self.edited = false;
        self.chunks_stale = false;
        self.format = format;
    }

    fn mark_reformatted(&mut self, format: StressFormat) {
        self.dirty = true;
        self.edited = false;
        self.format = format;
    }

    fn mark_edited(&mut self) {
        self.dirty = true;
        self.edited = true;
        self.chunks_stale = true;
    }

    fn mark_saved(&mut self) {
        self.dirty = false;
    }

    fn mark_exported(&mut self) {
        if !self.edited {
            self.dirty = false;
        }
        self.chunks_stale = false;
    }

    fn mark_rechunked(&mut self) {
        self.chunks_stale = false;
    }

    fn mark_stale(&mut self) {
        self.stale = true;
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn is_stale(&self) -> bool {
        self.stale
    }

    fn is_edited(&self) -> bool {
        self.edited
    }

    fn is_chunks_stale(&self) -> bool {
        self.chunks_stale
    }

    fn format(&self) -> StressFormat {
        self.format
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessingSnapshot {
    source_hash: u64,
}

impl ProcessingSnapshot {
    fn capture(text: &str) -> Self {
        Self {
            source_hash: text_fingerprint(text),
        }
    }

    fn source_changed(&self, text: &str) -> bool {
        self.source_hash != text_fingerprint(text)
    }
}

fn text_fingerprint(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
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
    document: BookDocument,
    raw_pipeline_output: String,
    active_tab: ActiveTab,
    report: PipelineReport,
    status_line: String,
    status_level: StatusLevel,
    last_error: Option<String>,
    is_busy: bool,
    processing_snapshot: Option<ProcessingSnapshot>,
    progress_ratio: f32,
    rx: Option<Receiver<WorkerMessage>>,
    diff_filter: String,
    dic_search_filter: String,
    last_copied_chunk: Option<usize>,
    capture_diff_on_run: bool,
    dic_load_errors: Vec<String>,
    source_dirty: bool,
    result_state: ResultState,
    chunk_apply_pending: bool,
    cancel_flag: Option<Arc<AtomicBool>>,
    find_open: bool,
    find_query: String,
    find_matches: FindMatches,
    find_for_tab: Option<ActiveTab>,
    find_dirty: bool,
    find_last_edit: Instant,
    find_current: usize,
    find_go_to: Option<usize>,
    raw_char_count: usize,
    processed_char_count: usize,
    chunk_render_limit: usize,
    diff_render_limit: usize,
    diff_filter_cache: String,
    diff_visible: Vec<usize>,
    diff_cache_valid: bool,
    focus_find: bool,
    last_window_title: String,
    window_icon_applied: bool,
}

enum WorkerMessage {
    DictionariesCompiled {
        compiled: Vec<CompiledDictionary>,
        errors: Vec<String>,
        elapsed_ms: u128,
    },
    BookLoaded(Result<BookDocument, String>),
    Pipeline(PipelineProgress),
}

impl DemagogApp {
    #[cfg(not(feature = "sw"))]
    #[cfg(feature = "eframe-backend")]
    pub fn new(cc: &eframe::CreationContext<'_>, initial_dic_folder: PathBuf) -> Self {
        Self::init(cc.egui_ctx.clone(), initial_dic_folder)
    }

    pub fn init(egui_ctx: egui::Context, initial_dic_folder: PathBuf) -> Self {
        configure_cyrillic_fonts(&egui_ctx);
        let (settings, settings_warning) = AppSettings::load(initial_dic_folder);
        apply_style_settings(&egui_ctx, &settings);

        let sample_text = concat!(
            "В 1995 году в замке у ворот города стояли все свои. ",
            "Они завоевывать земли не собирались, но руки на груди скрестили все.\n\n",
            "Абаренов посмотрел на берег реки: стоЯщих на столе кубков было ровно 12 штук. ",
            "Веселые места и весеннего ветра шум напоминали о том, что все уже в прошлом."
        )
        .to_string();
        let document = BookDocument {
            title: "Пример текста".to_string(),
            source_path: None,
            raw_text: sample_text,
            processed_text: String::new(),
            chunks: Vec::new(),
        };

        let mut app = Self::from_parts(settings, document);
        app.set_status(
            StatusLevel::Info,
            "Загрузка и компиляция словарей на всех ядрах CPU...",
        );
        app.reload_dictionaries_async(egui_ctx);
        if let Some(warning) = settings_warning {
            app.set_status(StatusLevel::Error, warning);
        }
        app
    }

    fn from_parts(settings: AppSettings, document: BookDocument) -> Self {
        let result_format = settings.stress_format;
        let raw_char_count = document.raw_text.chars().count();
        let processed_char_count = document.processed_text.chars().count();
        Self {
            settings,
            dictionaries: Vec::new(),
            document,
            raw_pipeline_output: String::new(),
            active_tab: ActiveTab::RawText,
            report: PipelineReport::default(),
            status_line: String::new(),
            status_level: StatusLevel::Info,
            last_error: None,
            is_busy: false,
            processing_snapshot: None,
            progress_ratio: 0.0,
            rx: None,
            diff_filter: String::new(),
            dic_search_filter: String::new(),
            last_copied_chunk: None,
            capture_diff_on_run: false,
            dic_load_errors: Vec::new(),
            source_dirty: false,
            result_state: ResultState::new(result_format),
            chunk_apply_pending: false,
            cancel_flag: None,
            find_open: false,
            find_query: String::new(),
            find_matches: FindMatches::default(),
            find_for_tab: None,
            find_dirty: false,
            find_last_edit: Instant::now(),
            find_current: 0,
            find_go_to: None,
            raw_char_count,
            processed_char_count,
            chunk_render_limit: CHUNK_BATCH,
            diff_render_limit: DIFF_BATCH,
            diff_filter_cache: String::new(),
            diff_visible: Vec::new(),
            diff_cache_valid: false,
            focus_find: false,
            last_window_title: "Demagog-RS".to_string(),
            window_icon_applied: false,
        }
    }

    fn reload_dictionaries_async(&mut self, ctx: egui::Context) {
        if self.is_busy {
            return;
        }
        self.is_busy = true;
        self.progress_ratio = 0.1;
        self.dic_load_errors.clear();
        let dic_folder = self.settings.dic_folder.display().to_string();
        self.set_status(
            StatusLevel::Info,
            format!("Параллельная компиляция словарей из {dic_folder}..."),
        );

        let folder = self.settings.dic_folder.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

        std::thread::spawn(move || {
            let start = Instant::now();
            match load_dictionaries_parallel(&folder) {
                Ok(report) => {
                    let DictionaryLoadReport { compiled, errors } = report;
                    let elapsed_ms = start.elapsed().as_millis();
                    let _ = tx.send(WorkerMessage::DictionariesCompiled {
                        compiled,
                        errors,
                        elapsed_ms,
                    });
                }
                Err(e) => {
                    let _ = tx.send(WorkerMessage::Pipeline(PipelineProgress::Error(e)));
                }
            }
            ctx.request_repaint();
        });
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
        if self.is_busy {
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
        if self.result_state.is_edited()
            && !confirm_destructive(
                "Запустить обработку заново?\nРучные правки в результате будут потеряны.",
            )
        {
            return;
        }

        self.sync_disabled_dictionaries();
        self.save_settings();

        self.is_busy = true;
        self.progress_ratio = 0.0;
        self.report = PipelineReport::default();
        self.reset_long_list_state();
        for d in &mut self.dictionaries {
            d.meta.last_ms = None;
            d.meta.last_replacements = None;
        }

        let input_text = self.document.raw_text.clone();
        self.processing_snapshot = Some(ProcessingSnapshot::capture(&input_text));
        let stress_format = self.settings.stress_format;
        let chunk_config = self.settings.chunk_config.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(Arc::clone(&cancel));
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

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
            if let Some(result) = pipeline.execute(
                &input_text,
                &active_processors,
                PipelineOptions {
                    stress_format,
                    chunk_config,
                    capture_diff,
                },
                Some(cancel.as_ref()),
                Some(&ptx),
            ) {
                let _ = ptx.send(PipelineProgress::Completed(result));
            }
            drop(ptx);
            let _ = forwarder.join();
        });
    }

    fn poll_worker(&mut self) {
        let mut finished = false;
        let mut disconnected = false;
        // The worker channel borrows `self.rx` for the whole loop, so status
        // updates are collected here and applied once the borrow ends.
        let mut pending_status: Option<(StatusLevel, String)> = None;

        if let Some(rx) = &self.rx {
            loop {
                match rx.try_recv() {
                    Ok(msg) => {
                        match msg {
                            WorkerMessage::DictionariesCompiled {
                                compiled,
                                errors,
                                elapsed_ms,
                            } => {
                                let total_rules: usize =
                                    compiled.iter().map(|c| c.meta.rule_count).sum();
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
                                    status.push_str(&format!(
                                        " | пропущено с ошибкой: {}",
                                        errors.len()
                                    ));
                                }
                                let level = if errors.is_empty() {
                                    StatusLevel::Success
                                } else {
                                    StatusLevel::Warning
                                };
                                pending_status = Some((level, status));
                                self.dic_load_errors = errors;
                                if !self.document.processed_text.is_empty()
                                    || !self.document.chunks.is_empty()
                                {
                                    self.result_state.mark_stale();
                                }
                                self.is_busy = false;
                                self.progress_ratio = 1.0;
                                finished = true;
                            }
                            WorkerMessage::BookLoaded(result) => {
                                match result {
                                    Ok(doc) => {
                                        let chars = doc.raw_text.chars().count();
                                        pending_status = Some((
                                            StatusLevel::Success,
                                            format!(
                                                "Открыта книга «{}» ({} символов)",
                                                doc.title,
                                                format_num(chars)
                                            ),
                                        ));
                                        self.document = doc;
                                        self.raw_char_count = chars;
                                        self.processed_char_count = 0;
                                        self.chunk_render_limit = CHUNK_BATCH;
                                        self.diff_render_limit = DIFF_BATCH;
                                        self.diff_cache_valid = false;
                                        self.raw_pipeline_output.clear();
                                        self.active_tab = ActiveTab::RawText;
                                        self.last_copied_chunk = None;
                                        self.report = PipelineReport::default();
                                        for d in &mut self.dictionaries {
                                            d.meta.last_ms = None;
                                            d.meta.last_replacements = None;
                                        }
                                        self.source_dirty = false;
                                        self.result_state.reset();
                                        self.find_dirty = true;
                                    }
                                    Err(err) => {
                                        pending_status = Some((StatusLevel::Error, err));
                                    }
                                }
                                self.is_busy = false;
                                self.progress_ratio = 0.0;
                                finished = true;
                            }
                            WorkerMessage::Pipeline(progress) => match progress {
                                PipelineProgress::StepStarted { index, total, name } => {
                                    self.progress_ratio = index as f32 / total.max(1) as f32;
                                    pending_status = Some((
                                        StatusLevel::Info,
                                        format!("[{index}/{total}] Обработка: {name}..."),
                                    ));
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
                                }
                                PipelineProgress::Completed(result) => {
                                    let source_changed =
                                        self.processing_snapshot.take().is_some_and(|snapshot| {
                                            snapshot.source_changed(&self.document.raw_text)
                                        });
                                    let summary = format!(
                                        "Обработано за {} мс | Замен: {} | Чанков: {}",
                                        result.report.total_ms,
                                        format_num(result.report.total_replacements),
                                        result.chunks.len()
                                    );
                                    self.raw_pipeline_output = result.raw_text;
                                    self.document.processed_text = result.processed_text;
                                    self.document.chunks = result.chunks;
                                    self.processed_char_count =
                                        self.document.processed_text.chars().count();
                                    self.chunk_render_limit = CHUNK_BATCH;
                                    self.diff_render_limit = DIFF_BATCH;
                                    self.diff_cache_valid = false;
                                    let had_events = !result.report.events.is_empty();
                                    self.report = result.report;
                                    self.result_state.mark_computed(self.settings.stress_format);
                                    if source_changed {
                                        self.result_state.mark_stale();
                                        pending_status = Some((
                                            StatusLevel::Warning,
                                            format!(
                                                "{summary} | Исходный текст изменён во время обработки — результат устарел, обработайте заново"
                                            ),
                                        ));
                                    } else {
                                        pending_status = Some((StatusLevel::Success, summary));
                                    }
                                    self.last_copied_chunk = None;
                                    self.find_dirty = true;
                                    self.cancel_flag = None;
                                    self.is_busy = false;
                                    self.progress_ratio = 1.0;
                                    if had_events && self.capture_diff_on_run {
                                        self.active_tab = ActiveTab::DiffAndStats;
                                    } else if self.active_tab == ActiveTab::RawText {
                                        self.active_tab = ActiveTab::ProcessedText;
                                    }
                                    finished = true;
                                }
                                PipelineProgress::Cancelled => {
                                    pending_status = Some((
                                        StatusLevel::Warning,
                                        "Обработка отменена".to_string(),
                                    ));
                                    self.cancel_flag = None;
                                    self.is_busy = false;
                                    self.progress_ratio = 0.0;
                                    finished = true;
                                }
                                PipelineProgress::Error(err) => {
                                    pending_status =
                                        Some((StatusLevel::Error, format!("Ошибка: {err}")));
                                    self.cancel_flag = None;
                                    self.is_busy = false;
                                    self.progress_ratio = 0.0;
                                    finished = true;
                                }
                            },
                        }

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
            self.processing_snapshot = None;
            if disconnected && !finished {
                self.set_status(
                    StatusLevel::Error,
                    "Рабочий поток завершился неожиданно — попробуйте ещё раз",
                );
                self.cancel_flag = None;
                self.is_busy = false;
                self.progress_ratio = 0.0;
            }
            self.rx = None;
        }
    }

    fn open_book_dialog(&mut self, ctx: egui::Context) {
        if self.is_busy {
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
        if self.is_busy {
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

        self.is_busy = true;
        self.progress_ratio = 0.0;
        self.set_status(StatusLevel::Info, format!("Чтение «{}»...", path.display()));

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let reader = CompositeBookReader::new();
            let result = reader.read_document(&path);
            let _ = tx.send(WorkerMessage::BookLoaded(result));
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

        if !self.is_busy && ctx.input_mut(|i| i.consume_shortcut(&open)) {
            self.open_book_dialog(ctx.clone());
        }
        if !self.is_busy && ctx.input_mut(|i| i.consume_shortcut(&process_diff)) {
            self.capture_diff_on_run = true;
            self.start_processing(ctx.clone(), true);
        }
        if !self.is_busy && ctx.input_mut(|i| i.consume_shortcut(&process)) {
            self.capture_diff_on_run = false;
            self.start_processing(ctx.clone(), false);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&save_source)) {
            self.save_source_txt();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&save)) {
            self.save_processed_txt();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&export)) {
            self.export_chunks_json();
        }

        let find = KeyboardShortcut::new(Modifiers::CTRL, Key::F);
        if ctx.input_mut(|i| i.consume_shortcut(&find)) {
            if self.active_tab != ActiveTab::RawText && self.active_tab != ActiveTab::ProcessedText
            {
                self.active_tab = if self.document.processed_text.is_empty() {
                    ActiveTab::RawText
                } else {
                    ActiveTab::ProcessedText
                };
            }
            self.find_open = true;
            self.focus_find = true;
            self.find_dirty = true;
            self.find_last_edit = Instant::now();
        }
        if self.find_open {
            let find_next = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
            let find_prev = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
            if ctx.input_mut(|i| i.consume_shortcut(&find_prev)) {
                self.step_find(-1);
            } else if ctx.input_mut(|i| i.consume_shortcut(&find_next)) {
                self.step_find(1);
            }
        }

        if let Some(cancel) = self.cancel_flag.clone() {
            if !cancel.load(Ordering::Relaxed)
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            {
                cancel.store(true, Ordering::Relaxed);
                self.set_status(StatusLevel::Warning, "Отмена...");
            }
        } else if self.find_open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.find_open = false;
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = window_title(&self.document.title, self.document.source_path.is_some());
        if title != self.last_window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_window_title = title;
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
        if self.is_busy {
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
        self.raw_char_count = self.document.raw_text.chars().count();
    }

    fn refresh_processed_char_count(&mut self) {
        self.processed_char_count = self.document.processed_text.chars().count();
    }

    fn reset_long_list_state(&mut self) {
        self.chunk_render_limit = CHUNK_BATCH;
        self.diff_render_limit = DIFF_BATCH;
        self.diff_cache_valid = false;
    }

    fn refresh_diff_visible(&mut self) {
        let filter = self.diff_filter.to_lowercase();
        self.diff_visible.clear();
        for (index, event) in self.report.events.iter().enumerate() {
            if filter.is_empty()
                || event.dictionary_name.to_lowercase().contains(&filter)
                || event.before_snippet.to_lowercase().contains(&filter)
                || event.after_snippet.to_lowercase().contains(&filter)
                || event.rule_summary.to_lowercase().contains(&filter)
            {
                self.diff_visible.push(index);
            }
        }
        self.diff_filter_cache = filter;
        self.diff_cache_valid = true;
        self.diff_render_limit = DIFF_BATCH;
    }

    fn refresh_find_matches(&mut self) {
        let query = self.find_query.clone();
        let text = match self.active_tab {
            ActiveTab::RawText => self.document.raw_text.as_str(),
            ActiveTab::ProcessedText => self.document.processed_text.as_str(),
            _ => "",
        };
        let matches = find_matches(text, &query);
        self.find_matches = matches;
        self.find_current = 0;
        self.find_go_to = None;
        self.find_for_tab = Some(self.active_tab);
    }

    fn step_find(&mut self, delta: isize) {
        if self.find_dirty {
            self.refresh_find_matches();
            self.find_dirty = false;
            if !self.find_matches.ranges.is_empty() {
                self.find_current = 0;
                self.find_go_to = Some(0);
            }
            return;
        }
        let total = self.find_matches.ranges.len();
        if total == 0 {
            return;
        }
        let current = self.find_current.min(total - 1);
        self.find_current = (current as isize + delta).rem_euclid(total as isize) as usize;
        self.find_go_to = Some(self.find_current);
    }

    fn scroll_to_find_match(
        &mut self,
        ui: &egui::Ui,
        output: &egui::widgets::text_edit::TextEditOutput,
    ) {
        let Some(index) = self.find_go_to else {
            return;
        };
        let char_positions = {
            let text = match self.active_tab {
                ActiveTab::RawText => &self.document.raw_text,
                ActiveTab::ProcessedText => &self.document.processed_text,
                _ => "",
            };
            self.find_matches
                .ranges
                .get(index)
                .and_then(|&(start, end)| {
                    Some((
                        text.get(..start)?.chars().count(),
                        text.get(..end)?.chars().count(),
                    ))
                })
        };
        self.find_go_to = None;
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
                    egui::TextEdit::singleline(&mut self.find_query)
                        .hint_text("Строка или фрагмент")
                        .desired_width(240.0),
                )
                .labelled_by(label.id);
            if self.focus_find {
                response.request_focus();
                self.focus_find = false;
            }
            if response.changed() {
                self.find_dirty = true;
                self.find_last_edit = Instant::now();
            }
            let debounce = Duration::from_millis(150);
            if self.find_dirty || self.find_for_tab != Some(self.active_tab) {
                if self.find_query.trim().is_empty() || self.find_last_edit.elapsed() >= debounce {
                    self.refresh_find_matches();
                    self.find_dirty = false;
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
                self.focus_find = true;
            }

            let total = self.find_matches.ranges.len();
            let counter = if self.find_query.trim().is_empty() {
                "Введите строку".to_string()
            } else if total == 0 {
                "Совпадений не найдено".to_string()
            } else {
                format!(
                    "{} из {}",
                    format_num(self.find_current + 1),
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
            if self.find_matches.truncated {
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
                self.find_open = false;
            }
        });
    }

    fn save_processed_txt(&mut self) {
        if self.document.processed_text.is_empty() {
            self.set_status(StatusLevel::Warning, "Сначала выполните обработку текста!");
            return;
        }
        if !self.confirm_stale_result("Сохранить его всё равно") {
            return;
        }
        let default_name = format!("{}_processed.txt", self.document.title);
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Текстовый файл UTF-8", &["txt"])
            .save_file()
        {
            match std::fs::write(&path, &self.document.processed_text) {
                Ok(_) => {
                    self.result_state.mark_saved();
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
        if self.document.raw_text.trim().is_empty() {
            self.set_status(
                StatusLevel::Warning,
                "Исходный текст пуст — нечего сохранять",
            );
            return;
        }
        let default_name = format!("{}_source.txt", self.document.title);
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Текстовый файл UTF-8", &["txt"])
            .save_file()
        {
            match std::fs::write(&path, &self.document.raw_text) {
                Ok(_) => {
                    self.document.source_path = Some(path.clone());
                    self.source_dirty = false;
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
        if self.document.processed_text.trim().is_empty() {
            self.set_status(
                StatusLevel::Warning,
                "Результат пуст — экспортировать нечего",
            );
            return;
        }
        if !self.confirm_stale_result("Экспортировать его всё равно") {
            return;
        }
        if self.result_state.is_chunks_stale() {
            self.rechunk_result();
        }
        if !can_export_chunks(&self.document.processed_text, &self.document.chunks) {
            self.set_status(
                StatusLevel::Warning,
                "Нет чанков для экспорта — запустите обработку!",
            );
            return;
        }
        let default_name = format!("{}_chunks.json", self.document.title);
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("JSON для ComfyUI", &["json"])
            .save_file()
        {
            #[derive(serde::Serialize)]
            struct ExportPayload<'a> {
                title: &'a str,
                stress_format: &'a str,
                chunk_count: usize,
                chunks: &'a [String],
            }
            let payload = ExportPayload {
                title: &self.document.title,
                stress_format: self.result_state.format().label(),
                chunk_count: self.document.chunks.len(),
                chunks: &self.document.chunks,
            };
            if let Ok(json) = serde_json::to_string_pretty(&payload) {
                match std::fs::write(&path, json) {
                    Ok(_) => {
                        // Chunks do not contain manual edits to processed_text,
                        // so an edited result stays dirty after exporting.
                        self.result_state.mark_exported();
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
        if self.document.processed_text.is_empty() && self.raw_pipeline_output.is_empty() {
            return true;
        }
        if self.raw_pipeline_output.is_empty() {
            self.result_state.mark_stale();
            self.set_status(
                StatusLevel::Warning,
                "Результат нельзя пересобрать: он уже снят с конвейера — обработайте книгу заново",
            );
            return false;
        }
        if self.result_state.is_edited()
            && !confirm_destructive(
                "Пересобрать результат по текущему формату?\nРучные правки в результате будут потеряны.",
            )
        {
            return false;
        }
        let formatter = RussianStressFormatter;
        self.document.processed_text =
            formatter.format_stress(&self.raw_pipeline_output, self.settings.stress_format);
        self.refresh_processed_char_count();
        self.result_state
            .mark_reformatted(self.settings.stress_format);
        self.rechunk_result();
        true
    }

    /// Re-chunk the current result without touching its text.
    fn rechunk_result(&mut self) {
        self.document.chunks =
            chunks_for_text(&self.document.processed_text, &self.settings.chunk_config);
        self.last_copied_chunk = None;
        self.chunk_render_limit = CHUNK_BATCH;
        self.result_state.mark_rechunked();
        self.find_dirty = true;
    }

    /// Apply chunk settings when a drag ends instead of on every slider tick.
    fn apply_chunk_config_change(&mut self, ui: &egui::Ui, changed: bool) {
        if changed {
            self.chunk_apply_pending = true;
        }
        if !self.chunk_apply_pending {
            return;
        }
        if ui.ctx().input(|i| i.pointer.primary_down()) {
            return;
        }
        self.chunk_apply_pending = false;
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
        self.status_line = text.into();
        self.status_level = level;
        if level == StatusLevel::Error {
            self.last_error = Some(self.status_line.clone());
        }
    }

    fn sync_disabled_dictionaries(&mut self) {
        self.settings.disabled_dictionaries = self
            .dictionaries
            .iter()
            .filter(|d| !d.meta.enabled)
            .map(|d| d.meta.name.clone())
            .collect();
    }

    fn save_settings(&mut self) {
        if let Err(err) = self.settings.save() {
            self.set_status(
                StatusLevel::Error,
                format!("Не удалось сохранить настройки: {err}"),
            );
        }
    }

    fn choose_dictionary_folder(&mut self, ctx: egui::Context) {
        if self.is_busy {
            return;
        }
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.settings.dic_folder = dir;
            self.save_settings();
            self.reload_dictionaries_async(ctx);
        }
    }

    fn has_unsaved_work(&self) -> bool {
        self.source_dirty || self.result_state.is_dirty()
    }

    fn mark_result_stale(&mut self) {
        if !self.document.processed_text.is_empty() || !self.document.chunks.is_empty() {
            self.result_state.mark_stale();
        }
    }

    fn confirm_stale_result(&self, action: &str) -> bool {
        if !self.result_state.is_stale() {
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
                    self.last_copied_chunk = Some(index);
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
        if self.is_busy {
            ui.spinner();
            ui.add(
                egui::ProgressBar::new(self.progress_ratio)
                    .desired_width(180.0)
                    .show_percentage(),
            );
            if let Some(cancel) = self.cancel_flag.clone() {
                let requested = cancel.load(Ordering::Relaxed);
                if ui
                    .add_enabled(!requested, egui::Button::new("Отмена"))
                    .on_hover_text("Остановить после текущего словаря (Esc)")
                    .clicked()
                {
                    cancel.store(true, Ordering::Relaxed);
                    self.set_status(StatusLevel::Warning, "Отмена...");
                }
            }
        }
        let (icon, color) = match self.status_level {
            StatusLevel::Info => ("", ui.visuals().text_color()),
            StatusLevel::Success => ("✅ ", palette::success_text(theme)),
            StatusLevel::Warning => ("⚠️ ", palette::warning_text(theme)),
            StatusLevel::Error => ("❌ ", palette::diff_before(theme)),
        };
        let status = ui.add(
            egui::Label::new(RichText::new(format!("{icon}{}", self.status_line)).color(color))
                .selectable(true),
        );
        status.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, self.status_line.clone())
        });
        ui.ctx().accesskit_node_builder(status.id, |node| {
            node.set_role(egui::accesskit::Role::Status);
            node.set_live(status_live(self.is_busy, self.status_level));
            node.set_live_atomic();
            node.set_label(self.status_line.clone());
        });
        if !self.dic_load_errors.is_empty() {
            status.on_hover_text(self.dic_load_errors.join("\n"));
        }
        if self.status_level != StatusLevel::Error
            && let Some(error) = self.last_error.clone()
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
                self.last_error = None;
            }
        }
    }

    fn show_status_counters(&self, ui: &mut egui::Ui, theme: ThemeMode, counters: &str) {
        ui.label(RichText::new(counters).weak());
        if !self.dic_load_errors.is_empty() {
            let details = self.dic_load_errors.join("\n");
            let errors = ui.add(
                egui::Label::new(
                    RichText::new(format!("Ошибки загрузки: {}", self.dic_load_errors.len()))
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
        if !self.result_state.is_stale() {
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
        // Exit immediately so Windows reclaims the ~6.6M heap-allocated dictionary strings
        // in 1ms instead of running sequential HeapFree destructors for 5 seconds.
        std::process::exit(0);
    }

    pub(crate) fn update_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_worker();
        if !self.window_icon_applied {
            self.window_icon_applied = super::window::apply_window_icon(&super::window::app_icon());
        }
        let theme = self.settings.theme();

        if ctx.input(|i| i.viewport().close_requested()) && self.has_unsaved_work() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if confirm_destructive(
                "Есть несохранённые изменения исходного текста или результата.\nВыйти без сохранения?",
            ) {
                self.shutdown();
            }
        }

        self.update_window_title(&ctx);
        self.handle_shortcuts(&ctx);
        self.handle_dropped_files(&ctx);

        self.show_top_toolbar(ui, &ctx, theme);

        self.show_status_bar(ui, theme);

        self.show_sidebar(ui, &ctx, theme);

        self.show_central_workspace(ui, &ctx, theme);
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
const CHUNK_BATCH: usize = 80;
const DIFF_BATCH: usize = 150;

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
mod tests;

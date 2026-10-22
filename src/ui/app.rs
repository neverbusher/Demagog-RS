use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;
use egui::{self, Color32, FontFamily, FontId, RichText, ScrollArea, TextStyle};

use crate::domain::{
    AppSettings, BookDocument, DictionaryMeta, DictionaryTag, PipelineProgress,
    PipelineReport, StressFormat, ThemeMode,
};
use crate::engine::loader::{load_dictionaries_parallel, CompiledDictionary, DictionaryLoadReport};
use crate::io::book_reader::CompositeBookReader;
use crate::ports::{DocumentReader, StressFormatter, TextChunker, TextProcessor};
use crate::services::chunker::SentenceAwareChunker;
use crate::services::pipeline::ProcessingPipeline;
use crate::services::stress::RussianStressFormatter;
use crate::ui::palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    RawText,
    ProcessedText,
    DramaChunks,
    DiffAndStats,
    Settings,
}

pub struct DemagogApp {
    settings: AppSettings,
    dictionaries: Vec<DictionaryMeta>,
    processors: Vec<Arc<dyn TextProcessor>>,
    document: BookDocument,
    raw_pipeline_output: String,
    active_tab: ActiveTab,
    report: PipelineReport,
    status_line: String,
    is_busy: bool,
    progress_ratio: f32,
    rx: Option<Receiver<WorkerMessage>>,
    diff_filter: String,
    dic_search_filter: String,
    last_copied_chunk: Option<usize>,
    capture_diff_on_run: bool,
    dic_load_errors: Vec<String>,
    unsaved_work: bool,
    cancel_flag: Option<Arc<AtomicBool>>,
    find_open: bool,
    find_query: String,
    find_matches: FindMatches,
    find_for_tab: Option<ActiveTab>,
    find_dirty: bool,
    focus_find: bool,
    last_window_title: String,
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
        let settings = AppSettings::load(initial_dic_folder);
        apply_style_settings(&egui_ctx, &settings);

        let sample_text = concat!(
            "В 1995 году в замке у ворот города стояли все свои. ",
            "Они завоевывать земли не собирались, но руки на груди скрестили все.\n\n",
            "Абаренов посмотрел на берег реки: стоЯщих на столе кубков было ровно 12 штук. ",
            "Веселые места и весеннего ветра шум напоминали о том, что все уже в прошлом."
        )
        .to_string();

        let mut app = Self {
            settings,
            dictionaries: Vec::new(),
            processors: Vec::new(),
            document: BookDocument {
                title: "Пример текста".to_string(),
                source_path: None,
                raw_text: sample_text,
                processed_text: String::new(),
                chunks: Vec::new(),
            },
            raw_pipeline_output: String::new(),
            active_tab: ActiveTab::RawText,
            report: PipelineReport::default(),
            status_line: "Загрузка и компиляция словарей на всех ядрах CPU...".to_string(),
            is_busy: false,
            progress_ratio: 0.0,
            rx: None,
            diff_filter: String::new(),
            dic_search_filter: String::new(),
            last_copied_chunk: None,
            capture_diff_on_run: false,
            dic_load_errors: Vec::new(),
            unsaved_work: false,
            cancel_flag: None,
            find_open: false,
            find_query: String::new(),
            find_matches: FindMatches::default(),
            find_for_tab: None,
            find_dirty: false,
            focus_find: false,
            last_window_title: "Demagog-RS".to_string(),
        };

        app.reload_dictionaries_async(egui_ctx);
        app
    }

    fn reload_dictionaries_async(&mut self, ctx: egui::Context) {
        if self.is_busy {
            return;
        }
        self.is_busy = true;
        self.progress_ratio = 0.1;
        self.dic_load_errors.clear();
        self.status_line = format!(
            "Параллельная компиляция словарей из {}...",
            self.settings.dic_folder.display()
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
            d.enabled = preset_enables(preset, d.tag, &d.name);
        }
        self.settings.stress_format = match preset {
            DictionaryPreset::All | DictionaryPreset::HomographsOnly => {
                StressFormat::PlusBeforeVowel
            }
            DictionaryPreset::NumbersAndYo => StressFormat::CleanYoOnly,
            DictionaryPreset::Clear => self.settings.stress_format,
        };
        self.settings.sync_disabled_from_metas(&self.dictionaries);
        self.settings.save();
    }

    fn start_processing(&mut self, ctx: egui::Context, capture_diff: bool) {
        if self.is_busy {
            return;
        }
        let active_processors: Vec<Arc<dyn TextProcessor>> = self
            .dictionaries
            .iter()
            .zip(self.processors.iter())
            .filter(|(meta, _)| meta.enabled)
            .map(|(_, proc)| Arc::clone(proc))
            .collect();

        if active_processors.is_empty() {
            self.status_line = "Не выбран ни один словарь!".to_string();
            return;
        }

        self.settings.sync_disabled_from_metas(&self.dictionaries);
        self.settings.save();

        self.is_busy = true;
        self.progress_ratio = 0.0;
        self.report = PipelineReport::default();
        for d in &mut self.dictionaries {
            d.last_ms = None;
            d.last_replacements = None;
        }

        let input_text = self.document.raw_text.clone();
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
            let _ = pipeline.execute(
                &input_text,
                &active_processors,
                stress_format,
                &chunk_config,
                capture_diff,
                Some(cancel.as_ref()),
                Some(&ptx),
            );
            drop(ptx);
            let _ = forwarder.join();
        });
    }

    fn poll_worker(&mut self) {
        let mut finished = false;
        let mut disconnected = false;

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
                                self.dictionaries = compiled
                                    .iter()
                                    .map(|c| {
                                        let mut m = c.meta.clone();
                                        m.enabled = self.settings.is_dic_enabled(&m.name);
                                        m
                                    })
                                    .collect();
                                self.processors =
                                    compiled.into_iter().map(|c| c.processor).collect();
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
                                self.status_line = status;
                                self.dic_load_errors = errors;
                                self.is_busy = false;
                                self.progress_ratio = 1.0;
                                finished = true;
                            }
                            WorkerMessage::BookLoaded(result) => {
                                match result {
                                    Ok(doc) => {
                                        let chars = doc.raw_text.chars().count();
                                        self.status_line = format!(
                                            "Открыта книга «{}» ({} символов)",
                                            doc.title,
                                            format_num(chars)
                                        );
                                        self.document = doc;
                                        self.raw_pipeline_output.clear();
                                        self.active_tab = ActiveTab::RawText;
                                        self.last_copied_chunk = None;
                                        self.report = PipelineReport::default();
                                        for d in &mut self.dictionaries {
                                            d.last_ms = None;
                                            d.last_replacements = None;
                                        }
                                        self.unsaved_work = false;
                                        self.find_dirty = true;
                                    }
                                    Err(err) => {
                                        self.status_line = err;
                                    }
                                }
                                self.is_busy = false;
                                self.progress_ratio = 0.0;
                                finished = true;
                            }
                            WorkerMessage::Pipeline(progress) => match progress {
                                PipelineProgress::StepStarted { index, total, name } => {
                                    self.progress_ratio =
                                        index as f32 / total.max(1) as f32;
                                    self.status_line =
                                        format!("[{index}/{total}] Обработка: {name}...");
                                }
                                PipelineProgress::StepFinished(step) => {
                                    if let Some(d) = self
                                        .dictionaries
                                        .iter_mut()
                                        .find(|d| d.name == step.dictionary_name)
                                    {
                                        d.last_ms = Some(step.elapsed_ms);
                                        d.last_replacements =
                                            Some(step.replacements_count);
                                    }
                                }
                                PipelineProgress::Completed(result) => {
                                    self.status_line = format!(
                                        "Обработано за {} мс | Замен: {} | Чанков: {}",
                                        result.report.total_ms,
                                        format_num(result.report.total_replacements),
                                        result.chunks.len()
                                    );
                                    self.raw_pipeline_output = result.raw_text;
                                    self.document.processed_text = result.processed_text;
                                    self.document.chunks = result.chunks;
                                    let had_events = !result.report.events.is_empty();
                                    self.report = result.report;
                                    self.unsaved_work = true;
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
                                    self.status_line = "Обработка отменена".to_string();
                                    self.cancel_flag = None;
                                    self.is_busy = false;
                                    self.progress_ratio = 0.0;
                                    finished = true;
                                }
                                PipelineProgress::Error(err) => {
                                    self.status_line = format!("Ошибка: {err}");
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

        if finished || disconnected {
            if disconnected && !finished {
                self.status_line =
                    "Рабочий поток завершился неожиданно — попробуйте ещё раз".to_string();
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
            .add_filter("Книги и текст", &["txt", "fb2", "epub", "docx", "zip", "md"])
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
        if self.unsaved_work
            && !confirm_destructive(
                "Открыть другую книгу?\nНесохранённая работа будет потеряна.",
            )
        {
            return;
        }

        self.is_busy = true;
        self.progress_ratio = 0.0;
        self.status_line = format!("Чтение «{}»...", path.display());

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
        let export = KeyboardShortcut::new(Modifiers::CTRL, Key::E);
        let process = KeyboardShortcut::new(Modifiers::CTRL, Key::Enter);
        let process_diff =
            KeyboardShortcut::new(Modifiers::CTRL | Modifiers::SHIFT, Key::Enter);

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
        if ctx.input_mut(|i| i.consume_shortcut(&save)) {
            self.save_processed_txt();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&export)) {
            self.export_chunks_json();
        }

        let find = KeyboardShortcut::new(Modifiers::CTRL, Key::F);
        if ctx.input_mut(|i| i.consume_shortcut(&find)) {
            if self.active_tab != ActiveTab::RawText
                && self.active_tab != ActiveTab::ProcessedText
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
        }

        if let Some(cancel) = self.cancel_flag.clone() {
            if !cancel.load(Ordering::Relaxed)
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            {
                cancel.store(true, Ordering::Relaxed);
                self.status_line = "Отмена...".to_string();
            }
        } else if self.find_open
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.find_open = false;
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = window_title(&self.document.title, self.unsaved_work);
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
            self.status_line = "Дождитесь завершения текущей операции".to_string();
            return;
        }
        let Some(path) = dropped
            .into_iter()
            .find(|path| is_supported_book_path(path))
        else {
            self.status_line =
                "Поддерживаются TXT, FB2, EPUB, DOCX, ZIP и MD".to_string();
            return;
        };
        self.load_book(path, ctx.clone());
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
        self.find_for_tab = Some(self.active_tab);
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
            }
            if self.find_dirty || self.find_for_tab != Some(self.active_tab) {
                self.refresh_find_matches();
                self.find_dirty = false;
            }

            let count = self.find_matches.ranges.len();
            let suffix = if self.find_matches.truncated { "+" } else { "" };
            ui.label(
                RichText::new(if self.find_query.trim().is_empty() {
                    "Введите строку".to_string()
                } else {
                    format!("Совпадений: {}{}", format_num(count), suffix)
                })
                .weak(),
            );

            if ui
                .button("✖")
                .on_hover_text("Закрыть поиск (Esc)")
                .clicked()
            {
                self.find_open = false;
            }
        });
    }

    fn save_processed_txt(&mut self) {
        if self.document.processed_text.is_empty() {
            self.status_line = "Сначала выполните обработку текста!".to_string();
            return;
        }
        let default_name = format!("{}_drama3.txt", self.document.title);
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("Текстовый файл UTF-8", &["txt"])
            .save_file()
        {
            match std::fs::write(&path, &self.document.processed_text) {
                Ok(_) => {
                    self.unsaved_work = false;
                    self.status_line = format!("Сохранено в {}", path.display());
                }
                Err(e) => {
                    self.status_line = format!("Ошибка сохранения: {e}");
                }
            }
        }
    }

    fn export_chunks_json(&mut self) {
        if self.document.chunks.is_empty() {
            self.status_line = "Нет чанков для экспорта — запустите обработку!".to_string();
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
                stress_format: self.settings.stress_format.label(),
                chunk_count: self.document.chunks.len(),
                chunks: &self.document.chunks,
            };
            if let Ok(json) = serde_json::to_string_pretty(&payload) {
                match std::fs::write(&path, json) {
                    Ok(_) => {
                        self.unsaved_work = false;
                        self.status_line = format!("Чанки экспортированы в {}", path.display());
                    }
                    Err(e) => self.status_line = format!("Ошибка записи JSON: {e}"),
                }
            }
        }
    }

    fn reformat_and_rechunk_existing(&mut self) {
        if !self.raw_pipeline_output.is_empty() {
            let formatter = RussianStressFormatter;
            self.document.processed_text =
                formatter.format_stress(&self.raw_pipeline_output, self.settings.stress_format);
            let chunker = SentenceAwareChunker;
            self.document.chunks = chunker
                .split_into_chunks(&self.document.processed_text, &self.settings.chunk_config);
        } else if !self.document.processed_text.is_empty() {
            let chunker = SentenceAwareChunker;
            self.document.chunks = chunker
                .split_into_chunks(&self.document.processed_text, &self.settings.chunk_config);
        }

        if !self.document.processed_text.is_empty() || !self.document.chunks.is_empty() {
            self.unsaved_work = true;
            self.find_dirty = true;
        }
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
    pub(crate) fn shutdown(&mut self) {
        self.settings.sync_disabled_from_metas(&self.dictionaries);
        self.settings.save();
        // Exit immediately so Windows reclaims the ~6.6M heap-allocated dictionary strings
        // in 1ms instead of running sequential HeapFree destructors for 5 seconds.
        std::process::exit(0);
    }

    pub(crate) fn update_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll_worker();
        let theme = self.settings.theme();

        if ctx.input(|i| i.viewport().close_requested()) && self.unsaved_work {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if confirm_destructive("Есть несохранённая работа.\nВыйти без сохранения?") {
                self.shutdown();
            }
        }

        self.update_window_title(&ctx);
        self.handle_shortcuts(&ctx);
        self.handle_dropped_files(&ctx);

        // Top toolbar
        egui::Panel::top("top_toolbar").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.is_busy,
                        egui::Button::new("Открыть книгу"),
                    )
                    .on_hover_text("Открыть TXT, FB2, FB2.ZIP, EPUB или DOCX (Ctrl+O)")
                    .clicked()
                {
                    self.open_book_dialog(ctx.clone());
                }

                let (primary_fill, primary_text) = palette::primary_button(theme);
                let run_btn = egui::Button::new(
                    RichText::new("⚡ Обработать книгу")
                        .strong()
                        .color(primary_text),
                )
                .fill(primary_fill);

                let run_response = ui
                    .add_enabled(!self.is_busy, run_btn)
                    .on_hover_text("Обработать книгу (Ctrl+Enter)");
                if run_response.clicked() {
                    self.capture_diff_on_run = false;
                    self.start_processing(ctx.clone(), false);
                }

                if ui
                    .add_enabled(
                        !self.is_busy,
                        egui::Button::new("🔬 С логом замен (Diff)"),
                    )
                    .on_hover_text("Собрать журнал замен (Ctrl+Shift+Enter)")
                    .clicked()
                {
                    self.capture_diff_on_run = true;
                    self.start_processing(ctx.clone(), true);
                }

                ui.separator();

                ui.label("Формат выхода:");
                let prev_fmt = self.settings.stress_format;
                egui::ComboBox::from_id_salt("stress_format_combo")
                    .selected_text(self.settings.stress_format.short_label())
                    .show_ui(ui, |ui| {
                        for fmt in [
                            StressFormat::PlusBeforeVowel,
                            StressFormat::AcuteAccent,
                            StressFormat::CleanYoOnly,
                        ] {
                            ui.selectable_value(&mut self.settings.stress_format, fmt, fmt.label());
                        }
                    });
                if self.settings.stress_format != prev_fmt {
                    self.reformat_and_rechunk_existing();
                    self.settings.save();
                }

                ui.separator();

                if ui
                    .add_enabled(
                        !self.document.processed_text.is_empty(),
                        egui::Button::new("💾 Сохранить TXT"),
                    )
                    .on_hover_text("Сохранить результат в TXT (Ctrl+S)")
                    .on_disabled_hover_text("Сначала обработайте текст")
                    .clicked()
                {
                    self.save_processed_txt();
                }
                if ui
                    .add_enabled(
                        !self.document.chunks.is_empty(),
                        egui::Button::new("📦 Экспорт JSON"),
                    )
                    .on_hover_text("Экспортировать чанки в JSON (Ctrl+E)")
                    .on_disabled_hover_text("Сначала обработайте текст")
                    .clicked()
                {
                    self.export_chunks_json();
                }

                ui.separator();

                ui.menu_button(theme_icon(theme), |ui| {
                    for candidate in [ThemeMode::Dark, ThemeMode::Light, ThemeMode::Warm] {
                        let label = format!("{} {}", theme_icon(candidate), candidate.label());
                        if ui.selectable_label(candidate == theme, label).clicked() {
                            self.settings.set_theme(candidate);
                            apply_style_settings(ui.ctx(), &self.settings);
                            self.settings.save();
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Тема оформления");
            });
            ui.add_space(4.0);
        });

        // Bottom status bar
        egui::Panel::bottom("bottom_status").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
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
                            self.status_line = "Отмена...".to_string();
                        }
                    }
                }
                let status = ui.label(&self.status_line);
                if !self.dic_load_errors.is_empty() {
                    status.on_hover_text(self.dic_load_errors.join("\n"));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let active_count = self.dictionaries.iter().filter(|d| d.enabled).count();
                    let active_rules: usize = self
                        .dictionaries
                        .iter()
                        .filter(|d| d.enabled)
                        .map(|d| d.rule_count)
                        .sum();
                    ui.label(
                        RichText::new(format!(
                            "Активно: {}/{} словарей ({} правил)",
                            active_count,
                            self.dictionaries.len(),
                            format_num(active_rules)
                        ))
                        .weak(),
                    );
                    if !self.dic_load_errors.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "Ошибки загрузки: {}",
                                self.dic_load_errors.len()
                            ))
                            .color(palette::warning_text(theme))
                            .strong(),
                        )
                        .on_hover_text(self.dic_load_errors.join("\n"));
                    }
                });
            });
        });

        // Left sidebar: Dictionary list with semantic tags & descriptions
        egui::Panel::left("dictionaries_sidebar")
            .default_size(450.0)
            .min_size(340.0)
            .show_inside(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.heading("📚 Словари");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                self.settings.dic_folder = dir;
                                self.settings.save();
                                self.reload_dictionaries_async(ctx.clone());
                            }
                        }
                    });
                });

                ui.horizontal(|ui| {
                    let mut selected = None;
                    ui.menu_button("Пресеты", |ui| {
                        if ui
                            .button("Все (полный +)")
                            .on_hover_text(
                                "Включить все словари, включая полную орфоэпию со знаками '+'",
                            )
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
                            .on_hover_text(
                                "Оставляет числа и даты, ё-фикацию и чистку текста",
                            )
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
                        self.settings.save();
                    }
                });

                ui.horizontal(|ui| {
                    let label = ui.label("Фильтр:");
                    let width = (ui.available_width() - 24.0).max(80.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.dic_search_filter)
                            .hint_text("Имя, описание или тип")
                            .desired_width(width),
                    )
                    .labelled_by(label.id);
                    if !self.dic_search_filter.is_empty()
                        && ui
                            .small_button("✖")
                            .on_hover_text("Сбросить фильтр")
                            .clicked()
                    {
                        self.dic_search_filter.clear();
                    }
                });

                ui.separator();

                let filter_low = self.dic_search_filter.to_lowercase();
                let mut changed_any = false;
                let visible: Vec<usize> = self
                    .dictionaries
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| {
                        filter_low.is_empty()
                            || d.name.to_lowercase().contains(&filter_low)
                            || d.description.to_lowercase().contains(&filter_low)
                            || d.tag.label().to_lowercase().contains(&filter_low)
                    })
                    .map(|(idx, _)| idx)
                    .collect();

                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.settings.group_dictionaries_by_type {
                            for tag in DictionaryTag::ALL {
                                let mut indices: Vec<usize> = visible
                                    .iter()
                                    .copied()
                                    .filter(|&i| self.dictionaries[i].tag == tag)
                                    .collect();
                                if indices.is_empty() {
                                    continue;
                                }
                                indices.sort_by_cached_key(|&i| {
                                    dictionary_display_key(&self.dictionaries[i].name)
                                });

                                let total = indices.len();
                                let active = indices
                                    .iter()
                                    .filter(|&&i| self.dictionaries[i].enabled)
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
                                        .small_button(RichText::new(state_label).color(state_color))
                                        .on_hover_text("Включить или отключить всю группу");
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
                                            self.dictionaries[i].enabled = enable;
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
                                        ui.add_space(2.0);
                                        for idx in indices {
                                            dictionary_card(
                                                ui,
                                                &mut self.dictionaries[idx],
                                                &mut changed_any,
                                                false,
                                                theme,
                                            );
                                        }
                                        ui.add_space(4.0);
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
                                    self.settings.save();
                                }
                            }
                        } else {
                            for idx in &visible {
                                dictionary_card(
                                    ui,
                                    &mut self.dictionaries[*idx],
                                    &mut changed_any,
                                    true,
                                    theme,
                                );
                            }
                        }
                    });

                if changed_any {
                    self.settings.sync_disabled_from_metas(&self.dictionaries);
                    self.settings.save();
                }
            });

        // Central workspace
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::RawText,
                    format!(
                        "📝 Исходный текст ({})",
                        format_num(self.document.raw_text.chars().count())
                    ),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::ProcessedText,
                    format!(
                        "✅ Результат ({})",
                        format_num(self.document.processed_text.chars().count())
                    ),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::DramaChunks,
                    format!("✂ Чанки ({})", self.document.chunks.len()),
                );
                ui.selectable_value(
                    &mut self.active_tab,
                    ActiveTab::DiffAndStats,
                    format!(
                        "🔬 Отладка замен ({})",
                        format_num(self.report.total_replacements)
                    ),
                );
                ui.selectable_value(&mut self.active_tab, ActiveTab::Settings, "⚙ Настройки");
            });
            ui.separator();

            let editor_font = FontId::new(self.settings.editor_font_size, FontFamily::Monospace);
            let line_height = editor_line_height(self.settings.editor_font_size);

            match self.active_tab {
                ActiveTab::RawText => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&self.document.title).strong());
                        if ui.button("📋 Заменить из буфера").clicked() {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                if let Ok(txt) = cb.get_text() {
                                    if !txt.is_empty()
                                        && (!self.unsaved_work
                                            || confirm_destructive(
                                                "Заменить исходный текст содержимым буфера?\nНесохранённые изменения будут потеряны.",
                                            ))
                                    {
                                        self.document.raw_text = txt;
                                        self.document.title = "Из буфера обмена".to_string();
                                        self.unsaved_work = true;
                                        self.find_dirty = true;
                                    }
                                }
                            }
                        }
                        if ui.button("🗑 Очистить").clicked() {
                            let had_text = text_would_be_lost(&self.document.raw_text);
                            if !had_text
                                || confirm_destructive(
                                    "Очистить исходный текст?\nТекст будет удалён без возможности отмены.",
                                )
                            {
                                self.document.raw_text.clear();
                                self.find_dirty = true;
                                if had_text {
                                    self.unsaved_work = true;
                                }
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
                    let mut layouter = editor_layouter(
                        editor_font.clone(),
                        ui.visuals().text_color(),
                        line_height,
                        &find_ranges,
                    );
                    ScrollArea::vertical().show(ui, |ui| {
                        let response = ui.add(
                            egui::TextEdit::multiline(&mut self.document.raw_text)
                                .desired_width(f32::INFINITY)
                                .desired_rows(28)
                                .margin(egui::Margin::symmetric(14, 12))
                                .layouter(&mut layouter),
                        );
                        if response.changed() {
                            self.unsaved_work = true;
                            self.find_dirty = true;
                        }
                    });
                }

                ActiveTab::ProcessedText => {
                    ui.horizontal(|ui| {
                        let has_result = !self.document.processed_text.is_empty();
                        if ui
                            .add_enabled(
                                has_result,
                                egui::Button::new("📋 Копировать весь результат"),
                            )
                            .clicked()
                        {
                            if let Ok(mut cb) = arboard::Clipboard::new() {
                                let _ = cb.set_text(self.document.processed_text.clone());
                                self.status_line =
                                    "Весь обработанный текст скопирован в буфер обмена!".to_string();
                            }
                        }
                        if ui
                            .add_enabled(
                                has_result,
                                egui::Button::new("🔄 Перенести результат в исходник"),
                            )
                            .clicked()
                        {
                            let had_text = text_would_be_lost(&self.document.raw_text);
                            if !had_text
                                || confirm_destructive(
                                    "Заменить исходный текст результатом?\nТекущий исходный текст будет потерян.",
                                )
                            {
                                self.document.raw_text = self.document.processed_text.clone();
                                self.raw_pipeline_output.clear();
                                self.unsaved_work = true;
                                self.find_dirty = true;
                            }
                        }
                    });
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
                    let mut layouter = editor_layouter(
                        editor_font.clone(),
                        ui.visuals().text_color(),
                        line_height,
                        &find_ranges,
                    );
                    ScrollArea::vertical().show(ui, |ui| {
                        let response = ui.add(
                            egui::TextEdit::multiline(&mut self.document.processed_text)
                                .desired_width(f32::INFINITY)
                                .desired_rows(28)
                                .margin(egui::Margin::symmetric(14, 12))
                                .layouter(&mut layouter),
                        );
                        if response.changed() {
                            self.unsaved_work = true;
                            self.find_dirty = true;
                        }
                    });
                }

                ActiveTab::DramaChunks => {
                    ui.horizontal(|ui| {
                        let label = ui.label("Макс. длина чанка (символов):");
                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut self.settings.chunk_config.max_chars,
                                    150..=3000,
                                )
                                .step_by(50.0),
                            )
                            .labelled_by(label.id)
                            .changed()
                        {
                            self.reformat_and_rechunk_existing();
                            self.settings.save();
                        }
                        if ui
                            .checkbox(
                                &mut self.settings.chunk_config.preserve_paragraphs,
                                "Не склеивать абзацы",
                            )
                            .changed()
                        {
                            self.reformat_and_rechunk_existing();
                            self.settings.save();
                        }
                    });
                    ui.separator();

                    ScrollArea::vertical().show(ui, |ui| {
                        if self.document.chunks.is_empty() {
                            ui.label("Нажмите «Обработать книгу», чтобы нарезать текст на чанки по границам предложений. Чанк можно скопировать отдельно, а все — экспортировать в JSON для ComfyUI.");
                        }
                        for (idx, chunk) in self.document.chunks.iter().enumerate() {
                            let is_copied = self.last_copied_chunk == Some(idx);
                            let mut frame = egui::Frame::group(ui.style());
                            if is_copied {
                                frame = frame.stroke(egui::Stroke::new(
                                    2.0_f32,
                                    palette::success_text(theme),
                                ));
                            }

                            frame
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "Чанк #{}/{} ({} симв.)",
                                                idx + 1,
                                                self.document.chunks.len(),
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
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .button("📋 Копировать чанк")
                                                    .clicked()
                                                {
                                                    if let Ok(mut cb) = arboard::Clipboard::new() {
                                                        let _ = cb.set_text(chunk.clone());
                                                        self.last_copied_chunk = Some(idx);
                                                        self.status_line = format!(
                                                            "Чанк #{} скопирован в буфер обмена",
                                                            idx + 1
                                                        );
                                                    }
                                                }
                                            },
                                        );
                                    });
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new(chunk)
                                            .font(editor_font.clone())
                                            .line_height(Some(line_height)),
                                    );
                                });
                            ui.add_space(6.0);
                        }
                    });
                }

                ActiveTab::DiffAndStats => {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Общее время: {} мс | Всего срабатываний: {}",
                                self.report.total_ms,
                                format_num(self.report.total_replacements)
                            ))
                            .strong(),
                        );
                        ui.separator();
                        ui.label("Поиск по заменам:");
                        ui.text_edit_singleline(&mut self.diff_filter);
                    });
                    ui.separator();

                    ScrollArea::vertical().show(ui, |ui| {
                        ui.heading("⏱ Тайминги по словарям");
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

                        ui.add_space(14.0);
                        ui.heading("🔍 Журнал сработавших правил (Diff)");
                        if self.report.events.is_empty() {
                            ui.label(
                                "Для записи детальных примеров каждого сработавшего правила нажмите кнопку «🔬 С логом замен (Diff)» на верхней панели.",
                            );
                        } else {
                            let filter_low = self.diff_filter.to_lowercase();
                            egui::Grid::new("diff_events_grid")
                                .striped(true)
                                .max_col_width(420.0)
                                .show(ui, |ui| {
                                    ui.label(RichText::new("Словарь").strong());
                                    ui.label(RichText::new("Правило").strong());
                                    ui.label(RichText::new("Было").strong());
                                    ui.label(RichText::new("Стало").strong());
                                    ui.end_row();

                                    for ev in &self.report.events {
                                        if !filter_low.is_empty()
                                            && !ev
                                                .dictionary_name
                                                .to_lowercase()
                                                .contains(&filter_low)
                                            && !ev
                                                .before_snippet
                                                .to_lowercase()
                                                .contains(&filter_low)
                                            && !ev
                                                .after_snippet
                                                .to_lowercase()
                                                .contains(&filter_low)
                                            && !ev
                                                .rule_summary
                                                .to_lowercase()
                                                .contains(&filter_low)
                                        {
                                            continue;
                                        }
                                        ui.label(&ev.dictionary_name);
                                        ui.label(RichText::new(&ev.rule_summary).monospace());
                                        ui.label(
                                            RichText::new(&ev.before_snippet)
                                                .color(palette::diff_before(theme))
                                                .font(editor_font.clone()),
                                        );
                                        ui.label(
                                            RichText::new(&ev.after_snippet)
                                                .color(palette::diff_after(theme))
                                                .font(editor_font.clone()),
                                        );
                                        ui.end_row();
                                    }
                                });
                        }
                    });
                }

                ActiveTab::Settings => {
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

                            let editor_label =
                                ui.label("Размер шрифта текста книги / чанков:");
                            if ui
                                .add(
                                    egui::Slider::new(
                                        &mut self.settings.editor_font_size,
                                        12.0..=36.0,
                                    )
                                    .suffix(" пт"),
                                )
                                .labelled_by(editor_label.id)
                                .changed()
                            {
                                style_changed = true;
                            }
                            ui.end_row();

                            let chunk_label = ui.label("Макс. размер чанка:");
                            if ui
                                .add(
                                    egui::Slider::new(
                                        &mut self.settings.chunk_config.max_chars,
                                        150..=3000,
                                    )
                                    .step_by(50.0)
                                    .suffix(" симв."),
                                )
                                .labelled_by(chunk_label.id)
                                .changed()
                            {
                                self.reformat_and_rechunk_existing();
                                self.settings.save();
                            }
                            ui.end_row();

                            ui.label("Папка словарей:");
                            ui.horizontal(|ui| {
                                ui.label(self.settings.dic_folder.display().to_string());
                                if ui.button("Изменить...").clicked() {
                                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                        self.settings.dic_folder = dir;
                                        self.settings.save();
                                        self.reload_dictionaries_async(ctx.clone());
                                    }
                                }
                            });
                            ui.end_row();
                        });

                    if style_changed {
                        apply_style_settings(&ctx, &self.settings);
                        self.settings.save();
                    }

                    ui.add_space(18.0);
                    ui.separator();
                    egui::CollapsingHeader::new("ℹ Справка по категориям словарей")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.label("• [ЧИСЛА И ДАТЫ] — переводит числа и даты в слова (1995 → тысяча девятьсот девяносто пятом).");
                            ui.label("• [Ё-ФИКАЦИЯ] — расставляет букву «ё» (все/всё и 337 тыс. слов из edge_Yo_.dic). Знаков «+» не ставит.");
                            ui.label("• [ОМОГРАФЫ (загл.)] — находит омографы по контексту и выделяет ударную гласную заглавной буквой (зАмок / замОк, зЕмли).");
                            ui.label("• [ЗАГЛАВНЫЕ → '+'] (словари 69 и 70) — переводит заглавные ударные буквы омографов в знак «+» (зЕмли → з+емли).");
                            ui.label("• [УДАРЕНИЯ '+'] и [ОРФОЭПИЯ '+' (2.9М)] (словари 35, 67, 71..75, 95.NEW) — сами напрямую ставят знак «+» в обычных словах. Именно из-за 95.NEW и 67 в тексте остаются «+», если выключить только словарь 69.");
                            ui.label("💡 Совет: если не нужны знаки «+», выберите в меню «Пресеты» вариант «Числа + Ё (без ударений)» или формат выхода «Без ударений — только Ё» сверху.");
                        });
                }
            }
        });
    }
}

fn dictionary_card(
    ui: &mut egui::Ui,
    d: &mut DictionaryMeta,
    changed_any: &mut bool,
    show_tag: bool,
    theme: ThemeMode,
) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            // Force equal card width: otherwise the frame shrinks
            // to the longest wrapped line and the right edge is ragged.
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let mut checkbox_response = ui.checkbox(&mut d.enabled, "");

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
                    .on_hover_text(&d.description);
                checkbox_response = checkbox_response.labelled_by(name_response.id);
                if checkbox_response.changed() {
                    *changed_any = true;
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} • {} правил • {}",
                        d.kind.badge(),
                        format_num(d.rule_count),
                        d.encoding
                    ))
                    .small()
                    .weak(),
                );
                if !d.warnings.is_empty() {
                    ui.label(
                        RichText::new(format!("⚠ Пропущено правил: {}", d.warnings.len()))
                            .small()
                            .color(palette::warning_text(theme))
                            .strong(),
                    )
                    .on_hover_text(d.warnings.join("\n"));
                }
                if let (Some(ms), Some(repl)) = (d.last_ms, d.last_replacements) {
                    let col = if repl > 0 {
                        palette::success_text(theme)
                    } else {
                        palette::weak_text(theme)
                    };
                    ui.label(
                        RichText::new(format!("• {ms} мс ({repl} замен)"))
                            .small()
                            .color(col)
                            .strong(),
                    );
                }
            });
        });
    ui.add_space(4.0);
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
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(205, 188, 165));
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(72, 60, 46));

    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(226, 211, 188);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(226, 211, 188);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(160, 135, 105));
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

    let visuals = match theme {
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
    ctx.set_visuals(visuals);

    let base = settings.ui_font_size;
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(base + 4.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(base, FontFamily::Proportional),
        );
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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
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

fn window_title(document_title: &str, unsaved: bool) -> String {
    let marker = if unsaved { " *" } else { "" };
    format!("Demagog-RS — {document_title}{marker}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DictionaryPreset {
    All,
    HomographsOnly,
    NumbersAndYo,
    Clear,
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

fn is_supported_book_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "txt" | "fb2" | "epub" | "docx" | "zip" | "md"
            )
        })
}

fn editor_line_height(font_size: f32) -> f32 {
    font_size * 1.45
}

const MAX_FIND_MATCHES: usize = 5000;

#[derive(Debug, Default)]
struct FindMatches {
    ranges: Vec<(usize, usize)>,
    truncated: bool,
}

fn find_matches(text: &str, query: &str) -> FindMatches {
    let query = query.trim();
    if query.is_empty() {
        return FindMatches::default();
    }
    let Ok(regex) = regex::RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
    else {
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
    for &(start, end) in matches {
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
        highlight.background = palette::find_highlight();
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
) -> impl FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> Arc<egui::Galley> + 'a {
    move |ui, text, wrap_width| {
        let job = layout_editor_text(
            text.as_str(),
            font.clone(),
            text_color,
            line_height,
            matches,
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
mod tests {
    use super::*;

    #[test]
    fn text_would_be_lost_for_non_empty_text() {
        assert!(text_would_be_lost("Книга"));
    }

    #[test]
    fn blank_text_does_not_need_confirmation() {
        assert!(!text_would_be_lost(" \n\t"));
    }

    #[test]
    fn accepts_supported_book_extensions_case_insensitively() {
        for name in [
            "book.txt",
            "book.FB2",
            "book.epub",
            "book.docx",
            "book.zip",
            "book.MD",
        ] {
            assert!(is_supported_book_path(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn rejects_other_files() {
        for name in ["cover.png", "book", "book.rtf"] {
            assert!(!is_supported_book_path(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn find_matches_is_case_insensitive_and_handles_yo() {
        let text = "Всё всё ВСЁ";
        let found = find_matches(text, "всё");
        assert_eq!(found.ranges.len(), 3);
        assert!(!found.truncated);
        for &(start, end) in &found.ranges {
            assert_eq!(&text[start..end].to_lowercase(), "всё");
        }
    }

    #[test]
    fn find_treats_metacharacters_literally() {
        let text = "a+b aab";
        let found = find_matches(text, "a+b");
        assert_eq!(found.ranges, vec![(0, 3)]);
    }

    #[test]
    fn blank_query_returns_no_matches() {
        assert!(find_matches("текст", "   ").ranges.is_empty());
    }

    #[test]
    fn find_caps_the_number_of_highlights() {
        let text = "a".repeat(MAX_FIND_MATCHES + 10);
        let found = find_matches(&text, "a");
        assert_eq!(found.ranges.len(), MAX_FIND_MATCHES);
        assert!(found.truncated);
    }

    #[test]
    fn layout_marks_only_matching_sections() {
        let text = "abc DEF";
        let matches = [(4, 7)];
        let job = layout_editor_text(
            text,
            FontId::monospace(14.0),
            Color32::WHITE,
            20.0,
            &matches,
            200.0,
        );
        assert_eq!(job.text, text);
        assert_eq!(job.sections.len(), 2);
        assert_eq!(job.sections[0].byte_range, 0..4);
        assert_eq!(job.sections[1].byte_range, 4..7);
        assert_eq!(
            job.sections[1].format.background,
            palette::find_highlight()
        );
        assert_eq!(job.sections[1].format.color, Color32::BLACK);
        assert_ne!(
            job.sections[0].format.background,
            palette::find_highlight()
        );
        assert_eq!(job.sections[1].format.line_height, Some(20.0));
    }

    #[test]
    fn layout_keeps_whole_text_when_nothing_matches() {
        let job = layout_editor_text(
            "abc",
            FontId::monospace(14.0),
            Color32::WHITE,
            20.0,
            &[],
            200.0,
        );
        assert_eq!(job.sections.len(), 1);
        assert_eq!(job.text, "abc");
    }

    #[test]
    fn preset_all_enables_every_dictionary() {
        assert!(preset_enables(
            DictionaryPreset::All,
            DictionaryTag::PlusStressOrthoepy,
            "95.NEW_Орфоэпия(phoneme).dic"
        ));
    }

    #[test]
    fn homographs_preset_skips_the_95_orthoepy_dictionary() {
        assert!(!preset_enables(
            DictionaryPreset::HomographsOnly,
            DictionaryTag::PlusStressOrthoepy,
            "95.NEW_Орфоэпия(phoneme).dic"
        ));
        assert!(preset_enables(
            DictionaryPreset::HomographsOnly,
            DictionaryTag::CapToPlusConverter,
            "69_TTS_Offline+УДАРЕНИЯ@(phoneme).dic"
        ));
    }

    #[test]
    fn numbers_and_yo_preset_keeps_only_safe_tags() {
        assert!(preset_enables(
            DictionaryPreset::NumbersAndYo,
            DictionaryTag::NumbersAndDates,
            "10_REX_числа(chisla).rex"
        ));
        assert!(preset_enables(
            DictionaryPreset::NumbersAndYo,
            DictionaryTag::YoFication,
            "15_REX_всё-все(vse_vsyo).rex"
        ));
        assert!(!preset_enables(
            DictionaryPreset::NumbersAndYo,
            DictionaryTag::PlusStressWords,
            "35_мой словарь.dic"
        ));
    }

    #[test]
    fn clear_preset_disables_every_dictionary() {
        assert!(!preset_enables(
            DictionaryPreset::Clear,
            DictionaryTag::NumbersAndDates,
            "10_REX_числа(chisla).rex"
        ));
    }

    #[test]
    fn window_title_includes_document_and_unsaved_marker() {
        assert_eq!(window_title("Книга", false), "Demagog-RS — Книга");
        assert_eq!(window_title("Книга", true), "Demagog-RS — Книга *");
    }

    #[test]
    fn editor_line_height_stays_comfortable() {
        for font_size in [12.0, 16.0, 19.0, 28.0, 36.0] {
            let line_height = editor_line_height(font_size);
            assert!(
                line_height >= font_size * 1.4,
                "{line_height} is too tight for {font_size}"
            );
            assert!(
                line_height <= font_size * 1.6,
                "{line_height} is too loose for {font_size}"
            );
        }
    }
}

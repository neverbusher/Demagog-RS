#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod domain;
mod engine;
mod io;
mod ports;
mod services;
mod ui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::domain::{ChunkConfig, StressFormat};
use crate::engine::loader::{DictionaryLoadReport, load_dictionaries_parallel};
use crate::io::book_reader::CompositeBookReader;
use crate::ports::DocumentReader;
use crate::services::chunker::SentenceAwareChunker;
use crate::services::pipeline::{PipelineOptions, ProcessingPipeline};
use crate::services::stress::RussianStressFormatter;
#[cfg(not(any(feature = "eframe-backend", feature = "sw")))]
compile_error!("Enable either the `eframe-backend` or `sw` feature");

#[cfg(all(feature = "sw", feature = "eframe-backend"))]
compile_error!("Build either the default eframe backend or `--no-default-features --features sw`");

#[cfg(feature = "eframe-backend")]
use crate::ui::app::DemagogApp;
use crate::ui::app::locate_default_dic_folder;
#[cfg(feature = "eframe-backend")]
use crate::ui::window::initial_viewport;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--bench" || a == "--cli") {
        run_cli_or_bench(&args);
        std::process::exit(0);
    }

    let dic_folder = locate_default_dic_folder();

    #[cfg(feature = "sw")]
    {
        if let Err(err) = ui::sw::run(dic_folder) {
            show_startup_error(format!(
                "Не удалось открыть окно Demagog-RS (программный рендер).\n\n{err}"
            ));
        }
    }

    #[cfg(all(feature = "eframe-backend", not(feature = "sw")))]
    {
        if let Err(err) = run_eframe(dic_folder) {
            show_startup_error(format!(
                "Не удалось открыть окно Demagog-RS.\n\n{err}\n\n\
                 Обновите драйвер видеокарты или запустите Demagog-RS-cpu.exe: \
                 он использует программный рендер и не требует OpenGL."
            ));
        }
    }

    #[cfg(not(any(feature = "sw", feature = "eframe-backend")))]
    compile_error!("Enable either the `eframe-backend` or `sw` feature");
}

fn show_startup_error(description: String) {
    let _ = rfd::MessageDialog::new()
        .set_title("Demagog-RS")
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .set_description(description)
        .show();
}

#[cfg(feature = "eframe-backend")]
fn run_eframe(dic_folder: PathBuf) -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        viewport: initial_viewport(),
        ..Default::default()
    };

    eframe::run_native(
        "Demagog-RS",
        native_options,
        Box::new(move |cc| Ok(Box::new(DemagogApp::new(cc, dic_folder)))),
    )
}

fn run_cli_or_bench(args: &[String]) {
    let dic_folder = locate_default_dic_folder();
    println!("Загрузка словарей из {}...", dic_folder.display());
    let t0 = Instant::now();
    let report = load_dictionaries_parallel(&dic_folder).expect("Ошибка загрузки словарей");
    let DictionaryLoadReport { compiled, errors } = report;
    for err in &errors {
        eprintln!("Пропущен словарь: {err}");
    }
    let total_rules: usize = compiled.iter().map(|c| c.meta.rule_count).sum();
    println!(
        "Скомпилировано {} словарей ({} правил) за {} мс",
        compiled.len(),
        total_rules,
        t0.elapsed().as_millis()
    );

    let input_path = args
        .windows(2)
        .find(|w| w[0] == "--input" || w[0] == "-i")
        .map(|w| PathBuf::from(&w[1]))
        .expect("Укажите входной файл через --input <путь>");

    let reader = CompositeBookReader::new();
    let doc = reader
        .read_document(&input_path)
        .expect("Не удалось прочитать тестовую книгу");
    println!(
        "Книга: «{}» ({} байт / {} символов)",
        doc.title,
        doc.raw_text.len(),
        doc.raw_text.chars().count()
    );

    let processors: Vec<_> = compiled.into_iter().map(|c| c.processor).collect();
    let pipeline = ProcessingPipeline::new(
        Arc::new(RussianStressFormatter),
        Arc::new(SentenceAwareChunker),
    );

    let Some(result) = pipeline.execute(
        &doc.raw_text,
        &processors,
        PipelineOptions {
            stress_format: StressFormat::PlusBeforeVowel,
            chunk_config: ChunkConfig::default(),
            capture_diff: args.iter().any(|a| a == "--diff"),
        },
        None,
        None,
    ) else {
        eprintln!("Обработка отменена");
        return;
    };

    println!("------------------------------------------------------------");
    for s in &result.report.steps {
        println!(
            "{:42} | {:10} | {:8} правил | {:5} мс | {:6} замен",
            s.dictionary_name,
            s.kind.badge(),
            s.rule_count,
            s.elapsed_ms,
            s.replacements_count
        );
    }
    println!("------------------------------------------------------------");
    println!(
        "ИТОГО: {} мс | Всего замен: {} | Чанков для Drama3: {}",
        result.report.total_ms,
        result.report.total_replacements,
        result.chunks.len()
    );

    if !result.report.events.is_empty() {
        println!("--- Журнал замен ---");
        for ev in &result.report.events {
            println!(
                "{} | {} | {} -> {}",
                ev.dictionary_name, ev.rule_summary, ev.before_snippet, ev.after_snippet
            );
        }
    }

    if let Some(out_path) = args
        .windows(2)
        .find(|w| w[0] == "--output" || w[0] == "-o")
        .map(|w| PathBuf::from(&w[1]))
    {
        std::fs::write(&out_path, &result.processed_text)
            .expect("Не удалось записать выходной файл");
        println!("Результат сохранён в {}", out_path.display());
    }
}

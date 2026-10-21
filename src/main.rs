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
use crate::engine::loader::{load_dictionaries_parallel, DictionaryLoadReport};
use crate::io::book_reader::CompositeBookReader;
use crate::ports::DocumentReader;
use crate::services::chunker::SentenceAwareChunker;
use crate::services::pipeline::ProcessingPipeline;
use crate::services::stress::RussianStressFormatter;
use crate::ui::app::{locate_default_dic_folder, DemagogApp};

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--bench" || a == "--cli") {
        run_cli_or_bench(&args);
        std::process::exit(0);
    }

    let dic_folder = locate_default_dic_folder();
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Demagog-RS")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1000.0, 640.0]),
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
    let doc = reader.read_document(&input_path).expect("Не удалось прочитать тестовую книгу");
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

    let result = pipeline.execute(
        &doc.raw_text,
        &processors,
        StressFormat::PlusBeforeVowel,
        &ChunkConfig::default(),
        args.iter().any(|a| a == "--diff"),
        None,
    );

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
                ev.dictionary_name,
                ev.rule_summary,
                ev.before_snippet,
                ev.after_snippet
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

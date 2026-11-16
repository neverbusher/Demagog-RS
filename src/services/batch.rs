use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use crate::domain::ChunksExportPayload;
use crate::ports::{DocumentReader, TextProcessor};
use crate::services::pipeline::{PipelineOptions, ProcessingPipeline};

pub struct BatchOptions {
    pub output_dir: PathBuf,
    pub export_txt: bool,
    pub export_chunks: bool,
    pub overwrite: bool,
    pub pipeline: PipelineOptions,
}

#[derive(Debug, Clone)]
pub struct BatchOutputs {
    pub txt: Option<PathBuf>,
    pub json: Option<PathBuf>,
    pub chunk_count: usize,
}

#[derive(Debug, Clone)]
pub enum BatchProgress {
    FileStarted {
        index: usize,
        total: usize,
        name: String,
    },
    FileFinished {
        index: usize,
        outcome: Result<BatchOutputs, String>,
    },
    Completed {
        succeeded: usize,
        failed: usize,
    },
    Cancelled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchSummary {
    pub succeeded: usize,
    pub failed: usize,
    pub cancelled: bool,
}

pub fn run_batch(
    files: &[PathBuf],
    reader: &dyn DocumentReader,
    processors: &[Arc<dyn TextProcessor>],
    pipeline: &ProcessingPipeline,
    options: &BatchOptions,
    cancel: Option<&AtomicBool>,
    progress_tx: Option<&Sender<BatchProgress>>,
) -> Result<BatchSummary, String> {
    std::fs::create_dir_all(&options.output_dir).map_err(|e| {
        format!(
            "Не удалось создать папку {}: {e}",
            options.output_dir.display()
        )
    })?;

    let mut summary = BatchSummary::default();
    let total = files.len();
    for (index, path) in files.iter().enumerate() {
        if is_cancelled(cancel) {
            summary.cancelled = true;
            send(progress_tx, BatchProgress::Cancelled);
            return Ok(summary);
        }
        send(
            progress_tx,
            BatchProgress::FileStarted {
                index,
                total,
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            },
        );

        match process_one(path, reader, processors, pipeline, options, cancel) {
            Ok(outputs) => {
                summary.succeeded += 1;
                send(
                    progress_tx,
                    BatchProgress::FileFinished {
                        index,
                        outcome: Ok(outputs),
                    },
                );
            }
            Err(err) => {
                if is_cancelled(cancel) {
                    summary.cancelled = true;
                    send(progress_tx, BatchProgress::Cancelled);
                    return Ok(summary);
                }
                summary.failed += 1;
                send(
                    progress_tx,
                    BatchProgress::FileFinished {
                        index,
                        outcome: Err(err),
                    },
                );
            }
        }
    }

    send(
        progress_tx,
        BatchProgress::Completed {
            succeeded: summary.succeeded,
            failed: summary.failed,
        },
    );
    Ok(summary)
}

fn process_one(
    path: &Path,
    reader: &dyn DocumentReader,
    processors: &[Arc<dyn TextProcessor>],
    pipeline: &ProcessingPipeline,
    options: &BatchOptions,
    cancel: Option<&AtomicBool>,
) -> Result<BatchOutputs, String> {
    let doc = reader.read_document_with_cancel(path, cancel)?;
    let result = pipeline
        .execute(
            &doc.raw_text,
            processors,
            options.pipeline.clone(),
            cancel,
            None,
        )
        .ok_or_else(|| "Обработка отменена".to_string())?;
    if result.processed_text.trim().is_empty() {
        return Err("Пустой результат обработки".to_string());
    }

    let stem = output_stem(path);
    let mut outputs = BatchOutputs {
        txt: None,
        json: None,
        chunk_count: result.chunks.len(),
    };
    if options.export_txt {
        let target = unique_target(
            &options.output_dir,
            &format!("{stem}.txt"),
            options.overwrite,
        );
        std::fs::write(&target, &result.processed_text)
            .map_err(|e| format!("Не удалось записать {}: {e}", target.display()))?;
        outputs.txt = Some(target);
    }
    if options.export_chunks {
        let target = unique_target(
            &options.output_dir,
            &format!("{stem}_chunks.json"),
            options.overwrite,
        );
        let payload = ChunksExportPayload {
            title: &doc.title,
            stress_format: options.pipeline.stress_format.label(),
            chunk_count: result.chunks.len(),
            chunks: &result.chunks,
        };
        let json = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        std::fs::write(&target, json)
            .map_err(|e| format!("Не удалось записать {}: {e}", target.display()))?;
        outputs.json = Some(target);
    }
    Ok(outputs)
}

fn unique_target(dir: &Path, file_name: &str, overwrite: bool) -> PathBuf {
    let target = dir.join(file_name);
    if overwrite || !target.exists() {
        return target;
    }
    let (stem, ext) = match file_name.rfind('.') {
        Some(pos) => (&file_name[..pos], &file_name[pos..]),
        None => (file_name, ""),
    };
    let mut n = 2;
    loop {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

fn output_stem(path: &Path) -> String {
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if file_name.to_ascii_lowercase().ends_with(".fb2.zip") {
        return file_name[..file_name.len() - ".fb2.zip".len()].to_string();
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("book")
        .to_string()
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

fn send(tx: Option<&Sender<BatchProgress>>, msg: BatchProgress) {
    if let Some(tx) = tx {
        let _ = tx.send(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        ChunkConfig, DictionaryKind, ProcessOutput, StressFormat, TextCleanupConfig,
    };
    use crate::io::book_reader::CompositeBookReader;
    use crate::services::chunker::SentenceAwareChunker;
    use crate::services::stress::RussianStressFormatter;
    use std::sync::mpsc;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "demagog-batch-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_book(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write book");
        path
    }

    fn pipeline() -> ProcessingPipeline {
        ProcessingPipeline::new(
            Arc::new(RussianStressFormatter),
            Arc::new(SentenceAwareChunker),
        )
    }

    fn options(output_dir: PathBuf) -> BatchOptions {
        BatchOptions {
            output_dir,
            export_txt: true,
            export_chunks: true,
            overwrite: true,
            pipeline: PipelineOptions {
                stress_format: StressFormat::PlusBeforeVowel,
                chunk_config: ChunkConfig::default(),
                capture_diff: false,
                cleanup: TextCleanupConfig {
                    remove_empty_lines: true,
                    add_line_end_periods: true,
                },
            },
        }
    }

    fn collect(rx: mpsc::Receiver<BatchProgress>) -> Vec<BatchProgress> {
        rx.iter().collect()
    }

    struct CancelOnSecondFile {
        flag: Arc<AtomicBool>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl TextProcessor for CancelOnSecondFile {
        fn name(&self) -> &str {
            "cancel-on-second"
        }
        fn kind(&self) -> DictionaryKind {
            DictionaryKind::RegexStandard
        }
        fn rule_count(&self) -> usize {
            1
        }
        fn process(&self, text: &str, _capture_diff: bool) -> ProcessOutput {
            let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if call >= 2 {
                self.flag.store(true, Ordering::Relaxed);
            }
            ProcessOutput {
                text: text.to_string(),
                replacements_count: 0,
                events: Vec::new(),
            }
        }
    }

    struct EmptyingProcessor;

    impl TextProcessor for EmptyingProcessor {
        fn name(&self) -> &str {
            "emptying"
        }
        fn kind(&self) -> DictionaryKind {
            DictionaryKind::RegexStandard
        }
        fn rule_count(&self) -> usize {
            1
        }
        fn process(&self, _text: &str, _capture_diff: bool) -> ProcessOutput {
            ProcessOutput {
                text: String::new(),
                replacements_count: 0,
                events: Vec::new(),
            }
        }
    }

    #[test]
    fn writes_txt_and_chunks_json_for_each_file() {
        let root = temp_dir("both");
        let input = root.join("in");
        std::fs::create_dir_all(&input).unwrap();
        let alpha = write_book(&input, "alpha.txt", "Глава первая\n\nТекст.");
        let beta = write_book(&input, "beta.txt", "Другая книга.");
        let out = root.join("out");

        let (tx, rx) = mpsc::channel();
        let summary = run_batch(
            &[alpha, beta],
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            Some(&tx),
        )
        .expect("batch runs");
        drop(tx);

        assert_eq!(
            summary,
            BatchSummary {
                succeeded: 2,
                failed: 0,
                cancelled: false
            }
        );
        assert_eq!(
            std::fs::read_to_string(out.join("alpha.txt")).unwrap(),
            "Глава первая.\nТекст."
        );
        assert!(out.join("beta.txt").exists());
        assert!(out.join("beta_chunks.json").exists());

        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("alpha_chunks.json")).unwrap())
                .expect("valid JSON");
        assert_eq!(json["title"], "alpha");
        assert_eq!(json["stress_format"], StressFormat::PlusBeforeVowel.label());
        assert_eq!(
            json["chunk_count"],
            json["chunks"].as_array().unwrap().len()
        );
        assert!(json["chunk_count"].as_u64().unwrap() >= 1);
        assert_eq!(json["chunks"][0], "Глава первая.\nТекст.");

        let messages = collect(rx);
        let started = messages
            .iter()
            .filter(|m| matches!(m, BatchProgress::FileStarted { .. }))
            .count();
        let finished_ok = messages
            .iter()
            .filter(|m| matches!(m, BatchProgress::FileFinished { outcome: Ok(_), .. }))
            .count();
        assert_eq!(started, 2);
        assert_eq!(finished_ok, 2);
        assert!(matches!(
            messages.last(),
            Some(BatchProgress::Completed {
                succeeded: 2,
                failed: 0
            })
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn continues_after_missing_file() {
        let root = temp_dir("missing");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let missing = root.join("missing.txt");
        let beta = write_book(&root, "beta.txt", "Другой текст.");
        let out = root.join("out");

        let (tx, rx) = mpsc::channel();
        let summary = run_batch(
            &[alpha, missing, beta],
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            Some(&tx),
        )
        .expect("batch runs");
        drop(tx);

        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.failed, 1);
        assert!(out.join("alpha.txt").exists());
        assert!(out.join("beta.txt").exists());
        let messages = collect(rx);
        assert!(messages.iter().any(|m| matches!(
            m,
            BatchProgress::FileFinished {
                index: 1,
                outcome: Err(_)
            }
        )));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unsupported_extension_is_per_file_error() {
        let root = temp_dir("unsupported");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let image = write_book(&root, "image.png", "not a book");
        let out = root.join("out");

        let summary = run_batch(
            &[alpha, image],
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(summary.succeeded, 1);
        assert_eq!(summary.failed, 1);
        assert!(out.join("alpha.txt").exists());
        assert!(!out.join("image.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn preset_cancel_processes_nothing() {
        let root = temp_dir("precancel");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let out = root.join("out");
        let flag = AtomicBool::new(true);

        let (tx, rx) = mpsc::channel();
        let summary = run_batch(
            &[alpha],
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            Some(&flag),
            Some(&tx),
        )
        .expect("batch runs");
        drop(tx);

        assert!(summary.cancelled);
        assert_eq!(summary.succeeded, 0);
        assert!(!out.join("alpha.txt").exists());
        assert!(matches!(collect(rx).last(), Some(BatchProgress::Cancelled)));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cancel_mid_batch_stops_after_current_file() {
        let root = temp_dir("midcancel");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let beta = write_book(&root, "beta.txt", "Другой текст.");
        let out = root.join("out");
        let flag = Arc::new(AtomicBool::new(false));
        let processors: Vec<Arc<dyn TextProcessor>> = vec![Arc::new(CancelOnSecondFile {
            flag: flag.clone(),
            calls: std::sync::atomic::AtomicUsize::new(0),
        })];

        let summary = run_batch(
            &[alpha, beta],
            &CompositeBookReader::new(),
            &processors,
            &pipeline(),
            &options(out.clone()),
            Some(flag.as_ref()),
            None,
        )
        .expect("batch runs");

        assert!(summary.cancelled);
        assert_eq!(summary.succeeded, 1);
        assert!(out.join("alpha.txt").exists());
        assert!(!out.join("beta.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overwrite_off_keeps_existing_and_adds_suffix() {
        let root = temp_dir("suffix");
        let alpha = write_book(&root, "alpha.txt", "Новый текст.");
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("alpha.txt"), "старое").unwrap();

        let mut opts = options(out.clone());
        opts.overwrite = false;
        let summary = run_batch(
            &[alpha],
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &opts,
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(summary.succeeded, 1);
        assert_eq!(
            std::fs::read_to_string(out.join("alpha.txt")).unwrap(),
            "старое"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("alpha (2).txt")).unwrap(),
            "Новый текст."
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overwrite_on_replaces_existing() {
        let root = temp_dir("overwrite");
        let alpha = write_book(&root, "alpha.txt", "Новый текст.");
        let out = root.join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("alpha.txt"), "старое").unwrap();

        run_batch(
            &[alpha],
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            std::fs::read_to_string(out.join("alpha.txt")).unwrap(),
            "Новый текст."
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn txt_only_switch_skips_json() {
        let root = temp_dir("txtonly");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let out = root.join("out");

        let mut opts = options(out.clone());
        opts.export_chunks = false;
        run_batch(
            &[alpha],
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &opts,
            None,
            None,
        )
        .expect("batch runs");

        assert!(out.join("alpha.txt").exists());
        assert!(!out.join("alpha_chunks.json").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn chunks_only_switch_skips_txt() {
        let root = temp_dir("chunksonly");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let out = root.join("out");

        let mut opts = options(out.clone());
        opts.export_txt = false;
        run_batch(
            &[alpha],
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &opts,
            None,
            None,
        )
        .expect("batch runs");

        assert!(!out.join("alpha.txt").exists());
        assert!(out.join("alpha_chunks.json").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_result_is_per_file_error() {
        let root = temp_dir("empty");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let out = root.join("out");
        let processors: Vec<Arc<dyn TextProcessor>> = vec![Arc::new(EmptyingProcessor)];

        let summary = run_batch(
            &[alpha],
            &CompositeBookReader::new(),
            &processors,
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(summary.failed, 1);
        assert_eq!(summary.succeeded, 0);
        assert!(!out.join("alpha.txt").exists());
        assert!(!out.join("alpha_chunks.json").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn output_dir_setup_failure_returns_err() {
        let root = temp_dir("setupfail");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let blocker = root.join("blocker");
        std::fs::write(&blocker, "file, not a dir").unwrap();

        let result = run_batch(
            &[alpha],
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(blocker.join("out")),
            None,
            None,
        );

        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn output_stem_strips_compound_fb2_zip_extension() {
        assert_eq!(output_stem(Path::new("book.fb2.zip")), "book");
        assert_eq!(output_stem(Path::new("book.FB2.ZIP")), "book");
        assert_eq!(output_stem(Path::new("book.fb2")), "book");
        assert_eq!(output_stem(Path::new("book.txt")), "book");
        assert_eq!(output_stem(Path::new("dir/book.epub")), "book");
    }
}

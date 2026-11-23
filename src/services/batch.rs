use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use crate::domain::{AppError, AppResult, ChunksExportPayload};
use crate::ports::{DocumentReader, TextProcessor, is_cancelled};
use crate::services::pipeline::{PipelineOptions, ProcessingPipeline};

pub struct BatchOptions {
    pub output_dir: PathBuf,
    pub export_txt: bool,
    pub export_chunks: bool,
    pub overwrite: bool,
    pub pipeline: PipelineOptions,
}

/// A queued source file plus the root its output path is mirrored against.
/// Folder-added books keep their subfolder structure; individually picked
/// files use their own parent as base and land flat in the output dir.
#[derive(Debug, Clone)]
pub struct BatchInput {
    pub path: PathBuf,
    pub base_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct BatchOutputs {
    pub txt: Option<PathBuf>,
    pub json: Option<PathBuf>,
    pub chunk_count: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchCounts {
    pub succeeded: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchOutcome {
    Completed(BatchCounts),
    Cancelled(BatchCounts),
}

impl BatchOutcome {
    pub fn counts(self) -> BatchCounts {
        match self {
            Self::Completed(counts) | Self::Cancelled(counts) => counts,
        }
    }

    pub fn is_cancelled(self) -> bool {
        matches!(self, Self::Cancelled(_))
    }
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
    Finished(BatchOutcome),
}

pub fn run_batch(
    files: &[BatchInput],
    reader: &dyn DocumentReader,
    processors: &[Arc<dyn TextProcessor>],
    pipeline: &ProcessingPipeline,
    options: &BatchOptions,
    cancel: Option<&AtomicBool>,
    progress_tx: Option<&Sender<BatchProgress>>,
) -> AppResult<BatchOutcome> {
    std::fs::create_dir_all(&options.output_dir).map_err(|e| {
        AppError::failed(format!(
            "Не удалось создать папку {}: {e}",
            options.output_dir.display()
        ))
    })?;

    let mut counts = BatchCounts::default();
    let mut used: HashSet<String> = HashSet::new();
    let total = files.len();
    for (index, input) in files.iter().enumerate() {
        if is_cancelled(cancel) {
            let outcome = BatchOutcome::Cancelled(counts);
            send(progress_tx, BatchProgress::Finished(outcome));
            return Ok(outcome);
        }
        send(
            progress_tx,
            BatchProgress::FileStarted {
                index,
                total,
                name: input
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            },
        );

        match process_one(input, reader, processors, pipeline, options, cancel, &used) {
            Ok(outputs) => {
                counts.succeeded += 1;
                if let Some(path) = &outputs.txt {
                    used.insert(target_key(path));
                }
                if let Some(path) = &outputs.json {
                    used.insert(target_key(path));
                }
                send(
                    progress_tx,
                    BatchProgress::FileFinished {
                        index,
                        outcome: Ok(outputs),
                    },
                );
            }
            Err(err) => {
                if is_cancelled(cancel) || err.is_cancelled() {
                    let outcome = BatchOutcome::Cancelled(counts);
                    send(progress_tx, BatchProgress::Finished(outcome));
                    return Ok(outcome);
                }
                counts.failed += 1;
                send(
                    progress_tx,
                    BatchProgress::FileFinished {
                        index,
                        outcome: Err(err.to_string()),
                    },
                );
            }
        }
    }

    let outcome = BatchOutcome::Completed(counts);
    send(progress_tx, BatchProgress::Finished(outcome));
    Ok(outcome)
}

fn process_one(
    input: &BatchInput,
    reader: &dyn DocumentReader,
    processors: &[Arc<dyn TextProcessor>],
    pipeline: &ProcessingPipeline,
    options: &BatchOptions,
    cancel: Option<&AtomicBool>,
    used: &HashSet<String>,
) -> AppResult<BatchOutputs> {
    let doc = reader.read_document_with_cancel(&input.path, cancel)?;
    let result = pipeline.execute(
        &doc.raw_text,
        processors,
        options.pipeline.clone(),
        cancel,
        None,
    )?;
    if result.processed_text.trim().is_empty() {
        return Err(AppError::failed("Пустой результат обработки"));
    }

    let relative = match input.path.strip_prefix(&input.base_dir) {
        Ok(rel) => rel.to_path_buf(),
        Err(_) => PathBuf::from(input.path.file_name().unwrap_or_default()),
    };
    let target_dir = options
        .output_dir
        .join(relative.parent().unwrap_or(Path::new("")));
    std::fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Не удалось создать папку {}: {e}", target_dir.display()))?;

    let stem = output_stem(&input.path);
    let mut outputs = BatchOutputs {
        txt: None,
        json: None,
        chunk_count: result.chunks.len(),
    };
    if options.export_txt {
        let target = unique_target(&target_dir, &format!("{stem}.txt"), options.overwrite, used);
        std::fs::write(&target, &result.processed_text)
            .map_err(|e| format!("Не удалось записать {}: {e}", target.display()))?;
        outputs.txt = Some(target);
    }
    if options.export_chunks {
        let target = unique_target(
            &target_dir,
            &format!("{stem}_chunks.json"),
            options.overwrite,
            used,
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

// Утилиты именования живут в нейтральном `io::paths`; ре-экспорт для совместимости.
pub(crate) use crate::io::paths::{output_stem, target_key, unique_target};

fn send(tx: Option<&Sender<BatchProgress>>, msg: BatchProgress) {
    if let Some(tx) = tx {
        let _ = tx.send(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        BookDocument, ChunkConfig, DictionaryKind, FailureKind, ProcessOutput, StressFormat,
        TextCleanupConfig,
    };
    use crate::io::book_reader::CompositeBookReader;
    use crate::services::chunker::SentenceAwareChunker;
    use crate::services::stress::RussianStressFormatter;
    use std::sync::atomic::Ordering;
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

    struct CancelSpecificReader {
        inner: CompositeBookReader,
        path: PathBuf,
    }

    impl DocumentReader for CancelSpecificReader {
        fn can_read(&self, path: &Path) -> bool {
            self.inner.can_read(path)
        }

        fn read_document_with_cancel(
            &self,
            path: &Path,
            cancel: Option<&AtomicBool>,
        ) -> AppResult<BookDocument> {
            if path == self.path.as_path() {
                return Err(AppError::cancelled("Чтение книги отменено"));
            }
            self.inner.read_document_with_cancel(path, cancel)
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
        let outcome = run_batch(
            &flat(&[alpha, beta]),
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
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 2,
                failed: 0,
            })
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
            Some(BatchProgress::Finished(finished)) if *finished == outcome
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
        let outcome = run_batch(
            &flat(&[alpha, missing, beta]),
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
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 2,
                failed: 1,
            })
        );
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
        assert!(matches!(
            messages.last(),
            Some(BatchProgress::Finished(finished)) if *finished == outcome
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unsupported_extension_is_per_file_error() {
        let root = temp_dir("unsupported");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let image = write_book(&root, "image.png", "not a book");
        let out = root.join("out");

        let outcome = run_batch(
            &flat(&[alpha, image]),
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 1,
            })
        );
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
        let outcome = run_batch(
            &flat(&[alpha]),
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            Some(&flag),
            Some(&tx),
        )
        .expect("batch runs");
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Cancelled(BatchCounts {
                succeeded: 0,
                failed: 0,
            })
        );
        assert!(!out.join("alpha.txt").exists());
        let messages = collect(rx);
        assert!(matches!(
            messages.last(),
            Some(BatchProgress::Finished(finished)) if *finished == outcome
        ));

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

        let (tx, rx) = mpsc::channel();
        let outcome = run_batch(
            &flat(&[alpha, beta]),
            &CompositeBookReader::new(),
            &processors,
            &pipeline(),
            &options(out.clone()),
            Some(flag.as_ref()),
            Some(&tx),
        )
        .expect("batch runs");
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Cancelled(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
        assert!(out.join("alpha.txt").exists());
        assert!(!out.join("beta.txt").exists());
        let messages = collect(rx);
        assert!(matches!(
            messages.last(),
            Some(BatchProgress::Finished(finished)) if *finished == outcome
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn typed_cancellation_preserves_partial_counts_without_flag() {
        let root = temp_dir("typedcancel");
        let missing = root.join("missing.txt");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let beta = write_book(&root, "beta.txt", "Отменяемая книга.");
        let gamma = write_book(&root, "gamma.txt", "Недостигнутая книга.");
        let out = root.join("out");
        let flag = AtomicBool::new(false);
        let reader = CancelSpecificReader {
            inner: CompositeBookReader::new(),
            path: beta.clone(),
        };

        let (tx, rx) = mpsc::channel();
        let outcome = run_batch(
            &flat(&[missing, alpha, beta, gamma]),
            &reader,
            &[],
            &pipeline(),
            &options(out.clone()),
            Some(&flag),
            Some(&tx),
        )
        .expect("batch runs");
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Cancelled(BatchCounts {
                succeeded: 1,
                failed: 1,
            })
        );
        assert!(!flag.load(Ordering::Relaxed));
        assert!(out.join("alpha.txt").exists());
        assert!(!out.join("beta.txt").exists());
        assert!(!out.join("gamma.txt").exists());
        let messages = collect(rx);
        assert!(messages.iter().any(|m| matches!(
            m,
            BatchProgress::FileFinished {
                index: 0,
                outcome: Err(_)
            }
        )));
        assert!(messages.iter().any(|m| matches!(
            m,
            BatchProgress::FileFinished {
                index: 1,
                outcome: Ok(_)
            }
        )));
        assert!(
            !messages
                .iter()
                .any(|m| matches!(m, BatchProgress::FileFinished { index: 2, .. }))
        );
        assert!(matches!(
            messages.last(),
            Some(BatchProgress::Finished(finished)) if *finished == outcome
        ));

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
        let outcome = run_batch(
            &flat(&[alpha]),
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &opts,
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
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

        let outcome = run_batch(
            &flat(&[alpha]),
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
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
        let outcome = run_batch(
            &flat(&[alpha]),
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &opts,
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
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
        let outcome = run_batch(
            &flat(&[alpha]),
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &opts,
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
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
        let (tx, rx) = mpsc::channel();

        let outcome = run_batch(
            &flat(&[alpha]),
            &CompositeBookReader::new(),
            &processors,
            &pipeline(),
            &options(out.clone()),
            None,
            Some(&tx),
        )
        .expect("batch runs");
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 0,
                failed: 1,
            })
        );
        assert!(!out.join("alpha.txt").exists());
        assert!(!out.join("alpha_chunks.json").exists());
        let messages = collect(rx);
        assert!(messages.iter().any(|m| matches!(
            m,
            BatchProgress::FileFinished {
                index: 0,
                outcome: Err(_)
            }
        )));
        assert!(matches!(
            messages.last(),
            Some(BatchProgress::Finished(finished)) if *finished == outcome
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn output_dir_setup_failure_returns_err() {
        let root = temp_dir("setupfail");
        let alpha = write_book(&root, "alpha.txt", "Текст.");
        let blocker = root.join("blocker");
        std::fs::write(&blocker, "file, not a dir").unwrap();

        let result = run_batch(
            &flat(&[alpha]),
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(blocker.join("out")),
            None,
            None,
        );

        let expected_prefix = format!(
            "Не удалось создать папку {}: ",
            blocker.join("out").display()
        );
        match result {
            Err(AppError::Failed(failure)) => {
                assert_eq!(failure.kind, FailureKind::Other);
                assert!(
                    failure.message.starts_with(&expected_prefix),
                    "{}",
                    failure.message
                );
            }
            Err(error) => panic!("setup failure must be a genuine AppError: {error:?}"),
            Ok(outcome) => panic!("setup unexpectedly succeeded: {outcome:?}"),
        }

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

    fn flat(paths: &[PathBuf]) -> Vec<BatchInput> {
        paths
            .iter()
            .map(|path| BatchInput {
                path: path.clone(),
                base_dir: path.parent().unwrap_or(Path::new("")).to_path_buf(),
            })
            .collect()
    }

    #[test]
    fn same_name_books_from_different_subfolders_do_not_collide() {
        let root = temp_dir("nocollide");
        let input = root.join("in");
        let sub1 = input.join("sub1");
        let sub2 = input.join("sub2");
        std::fs::create_dir_all(&sub1).unwrap();
        std::fs::create_dir_all(&sub2).unwrap();
        let book1 = write_book(&sub1, "book.txt", "Первая книга.");
        let book2 = write_book(&sub2, "book.txt", "Вторая книга.");
        let out = root.join("out");

        let inputs = vec![
            BatchInput {
                path: book1,
                base_dir: input.clone(),
            },
            BatchInput {
                path: book2,
                base_dir: input.clone(),
            },
        ];
        let outcome = run_batch(
            &inputs,
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 2,
                failed: 0,
            })
        );
        assert_eq!(
            std::fs::read_to_string(out.join("sub1").join("book.txt")).unwrap(),
            "Первая книга."
        );
        assert_eq!(
            std::fs::read_to_string(out.join("sub2").join("book.txt")).unwrap(),
            "Вторая книга."
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn mirrors_subfolder_structure_for_txt_and_json() {
        let root = temp_dir("mirror");
        let input = root.join("in");
        let sub = input.join("Автор").join("Цикл");
        std::fs::create_dir_all(&sub).unwrap();
        let book = write_book(&sub, "book.txt", "Текст книги.");
        let out = root.join("out");

        let inputs = vec![BatchInput {
            path: book,
            base_dir: input.clone(),
        }];
        let outcome = run_batch(
            &inputs,
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
        let mirrored = out.join("Автор").join("Цикл");
        assert!(mirrored.join("book.txt").exists());
        assert!(mirrored.join("book_chunks.json").exists());
        assert!(!out.join("book.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn individual_file_input_stays_flat() {
        let root = temp_dir("flat");
        let book = write_book(&root, "book.txt", "Текст.");
        let out = root.join("out");

        let outcome = run_batch(
            &flat(&[book]),
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
        assert!(out.join("book.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn same_relative_path_from_different_bases_gets_suffix_when_overwrite_off() {
        let root = temp_dir("crossbase");
        let in1 = root.join("in1");
        let in2 = root.join("in2");
        std::fs::create_dir_all(&in1).unwrap();
        std::fs::create_dir_all(&in2).unwrap();
        let book1 = write_book(&in1, "book.txt", "Первая.");
        let book2 = write_book(&in2, "book.txt", "Вторая.");
        let out = root.join("out");

        let inputs = vec![
            BatchInput {
                path: book1,
                base_dir: in1,
            },
            BatchInput {
                path: book2,
                base_dir: in2,
            },
        ];
        let mut opts = options(out.clone());
        opts.overwrite = false;
        let outcome = run_batch(
            &inputs,
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &opts,
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 2,
                failed: 0,
            })
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book.txt")).unwrap(),
            "Первая."
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book (2).txt")).unwrap(),
            "Вторая."
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn same_run_collision_gets_suffix_even_with_overwrite_on() {
        let root = temp_dir("intrarun");
        let in1 = root.join("in1");
        let in2 = root.join("in2");
        std::fs::create_dir_all(&in1).unwrap();
        std::fs::create_dir_all(&in2).unwrap();
        let book1 = write_book(&in1, "book.txt", "Первая.");
        let book2 = write_book(&in2, "book.txt", "Вторая.");
        let out = root.join("out");

        let inputs = vec![
            BatchInput {
                path: book1,
                base_dir: in1,
            },
            BatchInput {
                path: book2,
                base_dir: in2,
            },
        ];
        let outcome = run_batch(
            &inputs,
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 2,
                failed: 0,
            })
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book.txt")).unwrap(),
            "Первая."
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book (2).txt")).unwrap(),
            "Вторая."
        );
        assert!(out.join("book_chunks.json").exists());
        assert!(out.join("book_chunks (2).json").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn same_run_case_differing_names_get_suffix() {
        let root = temp_dir("intrarun-case");
        let in1 = root.join("in1");
        let in2 = root.join("in2");
        std::fs::create_dir_all(&in1).unwrap();
        std::fs::create_dir_all(&in2).unwrap();
        let book1 = write_book(&in1, "Book.txt", "Первая.");
        let book2 = write_book(&in2, "book.txt", "Вторая.");
        let out = root.join("out");

        let inputs = vec![
            BatchInput {
                path: book1,
                base_dir: in1,
            },
            BatchInput {
                path: book2,
                base_dir: in2,
            },
        ];
        let outcome = run_batch(
            &inputs,
            &CompositeBookReader::new(),
            &[],
            &pipeline(),
            &options(out.clone()),
            None,
            None,
        )
        .expect("batch runs");

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 2,
                failed: 0,
            })
        );
        assert_eq!(
            std::fs::read_to_string(out.join("Book.txt")).unwrap(),
            "Первая."
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book (2).txt")).unwrap(),
            "Вторая."
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

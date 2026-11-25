//! Оркестрация пакетной нарезки: чтение книг, нарезка, экспорт, прогресс.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use crate::domain::{AppError, AppResult, SplitConfig};
use crate::ports::{DocumentReader, is_cancelled};
use crate::services::batch::{BatchCounts, BatchInput, BatchOutcome, output_stem, target_key};

use super::export::export_parts_to_with_cancel;
use super::naming::sanitize_file_name;
use super::split::split_book_with_cancel;

#[derive(Debug, Clone)]
pub struct BatchSplitFileResult {
    pub saved: usize,
    pub errors: Vec<AppError>,
}

#[derive(Debug, Clone)]
pub enum BatchSplitProgress {
    FileStarted {
        index: usize,
        total: usize,
        name: String,
    },
    FileFinished {
        index: usize,
        result: BatchSplitFileResult,
    },
    ConfirmLargeExport {
        parts: usize,
    },
    Finished(BatchOutcome),
}

/// Нарезка пачки книг: для каждого входного файла читает книгу,
/// нарезает её и экспортирует части согласно `subfolder` и `overwrite`.
/// Ошибка одной книги не останавливает остальные.
#[cfg(test)]
pub fn run_split_batch(
    inputs: &[BatchInput],
    reader: &dyn DocumentReader,
    config: &SplitConfig,
    cancel: Option<&AtomicBool>,
    progress_tx: Option<&Sender<BatchSplitProgress>>,
) -> BatchOutcome {
    run_split_batch_with_confirmation(inputs, reader, config, cancel, progress_tx, None)
}

pub(crate) fn run_split_batch_with_confirmation(
    inputs: &[BatchInput],
    reader: &dyn DocumentReader,
    config: &SplitConfig,
    cancel: Option<&AtomicBool>,
    progress_tx: Option<&Sender<BatchSplitProgress>>,
    confirm_large_export: Option<&dyn Fn(usize) -> bool>,
) -> BatchOutcome {
    let mut counts = BatchCounts::default();
    let mut used: HashSet<String> = HashSet::new();
    let mut used_dirs: HashSet<String> = HashSet::new();
    let total = inputs.len();
    for (index, input) in inputs.iter().enumerate() {
        if is_cancelled(cancel) {
            let outcome = BatchOutcome::Cancelled(counts);
            send(progress_tx, BatchSplitProgress::Finished(outcome));
            return outcome;
        }
        send(
            progress_tx,
            BatchSplitProgress::FileStarted {
                index,
                total,
                name: input
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            },
        );
        match split_one_book(
            input,
            reader,
            config,
            cancel,
            &mut used,
            &mut used_dirs,
            confirm_large_export,
        ) {
            Ok(result) => {
                let stop = account_book_result(&mut counts, &result, cancel);
                send(
                    progress_tx,
                    BatchSplitProgress::FileFinished { index, result },
                );
                if stop {
                    let outcome = BatchOutcome::Cancelled(counts);
                    send(progress_tx, BatchSplitProgress::Finished(outcome));
                    return outcome;
                }
            }
            Err(err) => {
                let result = BatchSplitFileResult {
                    saved: 0,
                    errors: vec![err.clone()],
                };
                send(
                    progress_tx,
                    BatchSplitProgress::FileFinished { index, result },
                );
                if is_cancelled(cancel) || err.is_cancelled() {
                    let outcome = BatchOutcome::Cancelled(counts);
                    send(progress_tx, BatchSplitProgress::Finished(outcome));
                    return outcome;
                }
                counts.failed += 1;
            }
        }
    }
    let outcome = BatchOutcome::Completed(counts);
    send(progress_tx, BatchSplitProgress::Finished(outcome));
    outcome
}

fn account_book_result(
    counts: &mut BatchCounts,
    result: &BatchSplitFileResult,
    cancel: Option<&AtomicBool>,
) -> bool {
    let cancelled_by_result = result.errors.iter().any(|error| error.is_cancelled());
    if result.errors.is_empty() {
        counts.succeeded += 1;
    } else if !cancelled_by_result {
        counts.failed += 1;
    }
    is_cancelled(cancel) || cancelled_by_result
}

fn split_one_book(
    input: &BatchInput,
    reader: &dyn DocumentReader,
    config: &SplitConfig,
    cancel: Option<&AtomicBool>,
    used: &mut HashSet<String>,
    used_dirs: &mut HashSet<String>,
    confirm_large_export: Option<&dyn Fn(usize) -> bool>,
) -> AppResult<BatchSplitFileResult> {
    let doc = reader.read_document_with_cancel(&input.path, cancel)?;
    let parts = split_book_with_cancel(&doc.raw_text, &doc.headings, config, cancel)?;
    if parts.is_empty() {
        return Err(AppError::invalid_data("Книга пуста"));
    }
    if parts.len() > 200 && confirm_large_export.is_some_and(|confirm| !confirm(parts.len())) {
        return Err(AppError::cancelled("Экспорт пакета отменён"));
    }
    let source_name = output_stem(&input.path);
    let base = config
        .output_dir
        .as_ref()
        .ok_or_else(|| AppError::invalid_data("Не выбрана папка назначения"))?;
    let dir = if config.subfolder {
        unique_book_dir(base, &source_name, used_dirs)
    } else {
        base.clone()
    };
    let summary = export_parts_to_with_cancel(&parts, config, &source_name, &dir, used, cancel)?;
    Ok(BatchSplitFileResult {
        saved: summary.saved,
        errors: summary.errors,
    })
}

pub(crate) fn unique_book_dir(
    base: &Path,
    source_name: &str,
    used_dirs: &mut HashSet<String>,
) -> PathBuf {
    let stem = sanitize_file_name(source_name);
    let mut candidate = base.join(&stem);
    let mut suffix = 2;
    while used_dirs.contains(&target_key(&candidate)) {
        candidate = base.join(format!("{stem} ({suffix})"));
        suffix += 1;
    }
    used_dirs.insert(target_key(&candidate));
    candidate
}

fn send(tx: Option<&Sender<BatchSplitProgress>>, msg: BatchSplitProgress) {
    if let Some(tx) = tx {
        let _ = tx.send(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AppError, AppFailure, FailureKind};
    use crate::io::book_reader::CompositeBookReader;
    use crate::services::batch::target_key;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "demagog-split-batch-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn config(output_dir: Option<PathBuf>) -> SplitConfig {
        SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            template: "%FirstLine%".to_string(),
            add_number: false,
            output_dir,
            subfolder: true,
            ..Default::default()
        }
    }

    fn input(root: &Path, text: &str) -> BatchInput {
        let path = root.join("book.txt");
        std::fs::write(&path, text).unwrap();
        BatchInput {
            path,
            base_dir: root.to_path_buf(),
        }
    }

    fn expect_failure(error: &AppError, kind: FailureKind) -> &AppFailure {
        let AppError::Failed(failure) = error else {
            panic!("expected {kind:?} failure, got {error:?}");
        };
        assert_eq!(failure.kind, kind);
        failure
    }

    #[test]
    fn empty_book_is_invalid_data() {
        let root = temp_dir("empty-category");
        let book = input(&root, "Глава 1\n");
        let mut split_config = config(Some(root.join("out")));
        split_config.remove_heading = true;

        let error = split_one_book(
            &book,
            &CompositeBookReader::new(),
            &split_config,
            None,
            &mut HashSet::new(),
            &mut HashSet::new(),
            None,
        )
        .expect_err("empty book must fail");

        let failure = expect_failure(&error, FailureKind::InvalidData);
        assert_eq!(failure.message, "Книга пуста");
        assert_eq!(error.to_string(), failure.message);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn completed_book_is_counted_when_cancel_arrives_after_export() {
        use std::sync::atomic::AtomicBool;

        let cancel = AtomicBool::new(true);
        let result = BatchSplitFileResult {
            saved: 2,
            errors: Vec::new(),
        };
        let mut counts = BatchCounts::default();

        let stop = account_book_result(&mut counts, &result, Some(&cancel));

        assert!(stop);
        assert_eq!(
            counts,
            BatchCounts {
                succeeded: 1,
                failed: 0,
            }
        );
    }

    #[test]
    fn missing_destination_is_invalid_data() {
        let root = temp_dir("destination-category");
        let book = input(&root, "Глава 1\n\nТекст.");

        let error = split_one_book(
            &book,
            &CompositeBookReader::new(),
            &config(None),
            None,
            &mut HashSet::new(),
            &mut HashSet::new(),
            None,
        )
        .expect_err("missing destination must fail");

        let failure = expect_failure(&error, FailureKind::InvalidData);
        assert_eq!(failure.message, "Не выбрана папка назначения");
        assert_eq!(error.to_string(), failure.message);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overwrite_replaces_existing_targets_when_enabled() {
        let root = temp_dir("overwrite-category");
        let book = input(&root, "Глава 1\n\nТекст.");
        let out = root.join("out");
        let existing = out.join("book").join("Глава 1.txt");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, "старое").unwrap();

        let saved = split_one_book(
            &book,
            &CompositeBookReader::new(),
            &config(Some(out)),
            None,
            &mut HashSet::new(),
            &mut HashSet::new(),
            None,
        )
        .expect("overwrite must succeed");

        assert_eq!(saved.saved, 1);
        assert!(saved.errors.is_empty());
        assert_eq!(
            std::fs::read_to_string(&existing).unwrap(),
            "Глава 1\n\nТекст."
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn joined_export_write_errors_are_io() {
        let root = temp_dir("joined-export-errors");
        let book = input(&root, "Глава 1\n\nТекст первый.\nГлава 2\n\nТекст второй.");
        let out = root.join("out");
        let dir = out.join("book");
        std::fs::create_dir_all(&dir).unwrap();
        let mut used = HashSet::new();
        let mut blocked = Vec::new();
        for stem in ["Глава 1", "Глава 2"] {
            for suffix in ["", " (2)"] {
                used.insert(target_key(&dir.join(format!("{stem}{suffix}.txt"))));
            }
            let target = dir.join(format!("{stem} (3).txt"));
            std::fs::create_dir_all(&target).unwrap();
            blocked.push(target);
        }
        let expected = blocked
            .iter()
            .map(|path| {
                let error = std::fs::write(path, "blocked").expect_err("directory write fails");
                format!("{}: {error}", path.display())
            })
            .collect::<Vec<_>>()
            .join("; ");

        let result = split_one_book(
            &book,
            &CompositeBookReader::new(),
            &config(Some(out)),
            None,
            &mut used,
            &mut HashSet::new(),
            None,
        )
        .expect("partial export completes");

        assert_eq!(result.saved, 0);
        assert_eq!(result.errors.len(), 2);
        let messages = result
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        assert_eq!(messages, expected);
        for error in &result.errors {
            let failure = expect_failure(error, FailureKind::Io);
            assert_eq!(error.to_string(), failure.message);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

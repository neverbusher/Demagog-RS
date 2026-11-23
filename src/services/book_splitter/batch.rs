//! Оркестрация пакетной нарезки: чтение книг, нарезка, экспорт, прогресс.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use crate::domain::{AppError, AppResult, SplitConfig};
use crate::ports::{DocumentReader, is_cancelled};
use crate::services::batch::{BatchCounts, BatchInput, BatchOutcome, output_stem};

use super::export::export_parts_to;
use super::naming::{part_target, sanitize_file_name};
use super::split::split_book;

#[derive(Debug, Clone)]
pub enum BatchSplitProgress {
    FileStarted {
        index: usize,
        total: usize,
        name: String,
    },
    FileFinished {
        index: usize,
        outcome: Result<usize, String>,
    },
    Finished(BatchOutcome),
}

/// Нарезка пачки книг: для каждого входного файла читает книгу,
/// нарезает её и экспортирует части в папку с именем книги.
/// Ошибка одной книги не останавливает остальные.
pub fn run_split_batch(
    inputs: &[BatchInput],
    reader: &dyn DocumentReader,
    config: &SplitConfig,
    cancel: Option<&AtomicBool>,
    progress_tx: Option<&Sender<BatchSplitProgress>>,
) -> BatchOutcome {
    let mut counts = BatchCounts::default();
    let mut used: HashSet<String> = HashSet::new();
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
        match split_one_book(input, reader, config, cancel, &mut used) {
            Ok(parts_written) => {
                counts.succeeded += 1;
                send(
                    progress_tx,
                    BatchSplitProgress::FileFinished {
                        index,
                        outcome: Ok(parts_written),
                    },
                );
            }
            Err(err) => {
                if is_cancelled(cancel) || err.is_cancelled() {
                    let outcome = BatchOutcome::Cancelled(counts);
                    send(progress_tx, BatchSplitProgress::Finished(outcome));
                    return outcome;
                }
                counts.failed += 1;
                send(
                    progress_tx,
                    BatchSplitProgress::FileFinished {
                        index,
                        outcome: Err(err.to_string()),
                    },
                );
            }
        }
    }
    let outcome = BatchOutcome::Completed(counts);
    send(progress_tx, BatchSplitProgress::Finished(outcome));
    outcome
}

fn split_one_book(
    input: &BatchInput,
    reader: &dyn DocumentReader,
    config: &SplitConfig,
    cancel: Option<&AtomicBool>,
    used: &mut HashSet<String>,
) -> AppResult<usize> {
    let doc = reader.read_document_with_cancel(&input.path, cancel)?;
    let parts = split_book(&doc.raw_text, &doc.headings, config);
    if parts.is_empty() {
        return Err(AppError::invalid_data("Книга пуста"));
    }
    let source_name = output_stem(&input.path);
    let base = config
        .output_dir
        .as_ref()
        .ok_or_else(|| AppError::invalid_data("Не выбрана папка назначения"))?;
    // Пакетный режим: части каждой книги идут в папку с её именем.
    let dir = base.join(sanitize_file_name(&source_name));
    if config.overwrite {
        let existing: Vec<String> = parts
            .iter()
            .enumerate()
            .filter_map(|(index, part)| {
                let target = part_target(&dir, config, part, index, parts.len(), &source_name);
                target.exists().then(|| target.display().to_string())
            })
            .collect();
        if !existing.is_empty() {
            return Err(AppError::failed(format!(
                "Файлы уже существуют и будут затёрты: {}",
                existing.join(", ")
            )));
        }
    }
    let summary = export_parts_to(&parts, config, &source_name, &dir, used)?;
    if !summary.errors.is_empty() {
        let message = summary
            .errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(AppError::io(message));
    }
    Ok(summary.saved)
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
        )
        .expect_err("empty book must fail");

        let failure = expect_failure(&error, FailureKind::InvalidData);
        assert_eq!(failure.message, "Книга пуста");
        assert_eq!(error.to_string(), failure.message);
        let _ = std::fs::remove_dir_all(&root);
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
        )
        .expect_err("missing destination must fail");

        let failure = expect_failure(&error, FailureKind::InvalidData);
        assert_eq!(failure.message, "Не выбрана папка назначения");
        assert_eq!(error.to_string(), failure.message);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn overwrite_conflict_remains_other_failure() {
        let root = temp_dir("overwrite-category");
        let book = input(&root, "Глава 1\n\nТекст.");
        let out = root.join("out");
        let existing = out.join("book").join("Глава 1.txt");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, "старое").unwrap();

        let error = split_one_book(
            &book,
            &CompositeBookReader::new(),
            &config(Some(out)),
            None,
            &mut HashSet::new(),
        )
        .expect_err("overwrite conflict must fail");

        let failure = expect_failure(&error, FailureKind::Other);
        assert_eq!(
            failure.message,
            format!(
                "Файлы уже существуют и будут затёрты: {}",
                existing.display()
            )
        );
        assert_eq!(error.to_string(), failure.message);
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "старое");
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

        let error = split_one_book(
            &book,
            &CompositeBookReader::new(),
            &config(Some(out)),
            None,
            &mut used,
        )
        .expect_err("export write errors must fail the book");

        let failure = expect_failure(&error, FailureKind::Io);
        assert_eq!(failure.message, expected);
        assert_eq!(error.to_string(), failure.message);
        let _ = std::fs::remove_dir_all(&root);
    }
}

//! Оркестрация пакетной нарезки: чтение книг, нарезка, экспорт, прогресс.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use crate::domain::{AppResult, SplitConfig};
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
        return Err("Книга пуста".into());
    }
    let source_name = output_stem(&input.path);
    let base = config
        .output_dir
        .as_ref()
        .ok_or("Не выбрана папка назначения")?;
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
            return Err(format!(
                "Файлы уже существуют и будут затёрты: {}",
                existing.join(", ")
            )
            .into());
        }
    }
    let summary = export_parts_to(&parts, config, &source_name, &dir, used)?;
    if !summary.errors.is_empty() {
        return Err(summary.errors.join("; ").into());
    }
    Ok(summary.saved)
}

fn send(tx: Option<&Sender<BatchSplitProgress>>, msg: BatchSplitProgress) {
    if let Some(tx) = tx {
        let _ = tx.send(msg);
    }
}

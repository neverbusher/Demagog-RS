//! Запись частей на диск (ввод-вывод), без логики нарезки.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::domain::{AppError, AppResult, SplitConfig};
use crate::ports::is_cancelled;
use crate::services::batch::{target_key, unique_target};

use super::naming::{part_target, sanitize_file_name};
use super::split::BookPart;

#[derive(Debug, Default)]
pub struct ExportSummary {
    pub saved: usize,
    pub dir: PathBuf,
    pub targets: Vec<PathBuf>,
    pub errors: Vec<AppError>,
}

pub fn planned_part_names(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
) -> Vec<String> {
    let dir = book_dir(config, source_name).unwrap_or_default();
    let overwrite = config.output_dir.is_none() || config.overwrite;
    let mut used = HashSet::new();
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let target = part_target(&dir, config, part, index, parts.len(), source_name);
            let file_name = target
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let target = unique_target(&dir, &file_name, overwrite, &used);
            used.insert(target_key(&target));
            target
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".txt"))
                .unwrap_or("часть")
                .to_string()
        })
        .collect()
}

/// Папка назначения для книги: база, либо база + подпапка с именем книги.
pub fn book_dir(config: &SplitConfig, source_name: &str) -> Option<PathBuf> {
    let base = config.output_dir.as_ref()?;
    Some(if config.subfolder {
        base.join(sanitize_file_name(source_name))
    } else {
        base.clone()
    })
}

#[cfg(test)]
pub fn export_parts(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
) -> AppResult<ExportSummary> {
    export_parts_into(parts, config, source_name, &mut HashSet::new())
}

/// То же, но `used` — общее на всю операцию, чтобы части разных книг
/// не затирали одни имена других (батч-нарезка).
#[cfg(test)]
pub fn export_parts_into(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
    used: &mut HashSet<String>,
) -> AppResult<ExportSummary> {
    export_parts_into_with_cancel(parts, config, source_name, used, None)
}

pub(crate) fn export_parts_with_cancel(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
    cancel: Option<&AtomicBool>,
) -> AppResult<ExportSummary> {
    export_parts_into_with_cancel(parts, config, source_name, &mut HashSet::new(), cancel)
}

fn export_parts_into_with_cancel(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
    used: &mut HashSet<String>,
    cancel: Option<&AtomicBool>,
) -> AppResult<ExportSummary> {
    let dir = book_dir(config, source_name)
        .ok_or_else(|| AppError::invalid_data("Не выбрана папка назначения"))?;
    export_parts_to_with_cancel(parts, config, source_name, &dir, used, cancel)
}

fn write_part(target: &Path, text: &str, overwrite: bool) -> std::io::Result<()> {
    if overwrite {
        return std::fs::write(target, text);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    file.write_all(text.as_bytes())
}

/// Запись частей в явно заданную папку `dir`.
pub(crate) fn export_parts_to_with_cancel(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
    dir: &Path,
    used: &mut HashSet<String>,
    cancel: Option<&AtomicBool>,
) -> AppResult<ExportSummary> {
    std::fs::create_dir_all(dir)
        .map_err(|e| AppError::io(format!("Не удалось создать папка {}: {e}", dir.display())))?;
    let mut summary = ExportSummary {
        dir: dir.to_path_buf(),
        ..Default::default()
    };
    for (index, part) in parts.iter().enumerate() {
        if is_cancelled(cancel) {
            summary
                .errors
                .push(AppError::cancelled("Экспорт частей отменён"));
            break;
        }
        let target_path = part_target(dir, config, part, index, parts.len(), source_name);
        let file_name = target_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut target = unique_target(dir, &file_name, config.overwrite, used);
        used.insert(target_key(&target));
        loop {
            match write_part(&target, &part.text, config.overwrite) {
                Ok(()) => {
                    summary.targets.push(target.clone());
                    summary.saved += 1;
                    break;
                }
                Err(error)
                    if !config.overwrite && error.kind() == std::io::ErrorKind::AlreadyExists =>
                {
                    target = unique_target(dir, &file_name, false, used);
                    used.insert(target_key(&target));
                }
                Err(error) => {
                    summary.targets.push(target.clone());
                    summary
                        .errors
                        .push(AppError::io(format!("{}: {error}", target.display())));
                    break;
                }
            }
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_overwrite_write_never_truncates_an_existing_target() {
        let path = std::env::temp_dir().join(format!(
            "demagog-atomic-write-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "старый текст").unwrap();

        let error = write_part(&path, "новый текст", false).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "старый текст");
        let _ = std::fs::remove_file(path);
    }
}

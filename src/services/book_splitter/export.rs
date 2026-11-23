//! Запись частей на диск (ввод-вывод), без логики нарезки.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::domain::SplitConfig;
use crate::services::batch::{target_key, unique_target};

use super::naming::{part_target, sanitize_file_name};
use super::split::BookPart;

#[derive(Debug, Default)]
pub struct ExportSummary {
    pub saved: usize,
    pub dir: PathBuf,
    pub errors: Vec<String>,
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

pub fn export_parts(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
) -> Result<ExportSummary, String> {
    export_parts_into(parts, config, source_name, &mut HashSet::new())
}

/// То же, но `used` — общее на всю операцию, чтобы части разных книг
/// не затирали одни имена других (батч-нарезка).
pub fn export_parts_into(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
    used: &mut HashSet<String>,
) -> Result<ExportSummary, String> {
    let dir = book_dir(config, source_name).ok_or("Не выбрана папка назначения")?;
    export_parts_to(parts, config, source_name, &dir, used)
}

/// Запись частей в явно заданную папку `dir`.
pub fn export_parts_to(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
    dir: &Path,
    used: &mut HashSet<String>,
) -> Result<ExportSummary, String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Не удалось создать папку {}: {e}", dir.display()))?;
    let mut summary = ExportSummary {
        dir: dir.to_path_buf(),
        ..Default::default()
    };
    for (index, part) in parts.iter().enumerate() {
        let target_path = part_target(dir, config, part, index, parts.len(), source_name);
        let file_name = target_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let target = unique_target(dir, &file_name, config.overwrite, used);
        used.insert(target_key(&target));
        match std::fs::write(&target, &part.text) {
            Ok(()) => summary.saved += 1,
            Err(e) => summary.errors.push(format!("{}: {e}", target.display())),
        }
    }
    Ok(summary)
}

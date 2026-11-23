//! Нейтральные утилиты именования выходных файлов (без привязки к батчу).
//! Используются сервисами batch/book_splitter и UI.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn target_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

pub fn unique_target(
    dir: &Path,
    file_name: &str,
    overwrite: bool,
    used: &HashSet<String>,
) -> PathBuf {
    // `used` guards against two inputs of the same run mapping to one target:
    // overwrite may replace previous runs' files, never a sibling's result.
    // Keys are lowercased because Windows paths are case-insensitive.
    let is_free = |path: &Path| !used.contains(&target_key(path)) && (overwrite || !path.exists());
    let target = dir.join(file_name);
    if is_free(&target) {
        return target;
    }
    let (stem, ext) = match file_name.rfind('.') {
        Some(pos) => (&file_name[..pos], &file_name[pos..]),
        None => (file_name, ""),
    };
    let mut n = 2;
    loop {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if is_free(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Book name without a compound `.fb2.zip` extension, or `None` when the
/// path has no usable stem.
pub fn book_stem(path: &Path) -> Option<String> {
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if file_name.to_ascii_lowercase().ends_with(".fb2.zip") {
        return Some(file_name[..file_name.len() - ".fb2.zip".len()].to_string());
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
}

pub fn output_stem(path: &Path) -> String {
    book_stem(path)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "book".to_string())
}

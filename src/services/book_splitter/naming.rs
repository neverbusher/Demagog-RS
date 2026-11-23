//! Именование файлов частей: шаблон, санитизация, целевой путь.

use std::path::{Path, PathBuf};

use crate::domain::SplitConfig;

use super::split::BookPart;

pub fn part_file_name(
    config: &SplitConfig,
    part: &BookPart,
    index: usize,
    total: usize,
    source_name: &str,
) -> String {
    let number = config.start_number + index;
    let width = (config.start_number + total.saturating_sub(1))
        .to_string()
        .len()
        .max(2);
    let padded = format!("{number:0width$}");
    let first_line: String = part
        .text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(60)
        .collect();
    let header = part.title.as_deref().unwrap_or(&first_line);
    let template = if config.template.trim().is_empty() {
        SplitConfig::default().template
    } else {
        config.template.clone()
    };
    let mut name = template
        .replace("%Number%", &padded)
        .replace("%FileName%", source_name)
        .replace("%FirstLine%", &first_line)
        .replace("%Header%", header);
    if config.add_number && !template.contains("%Number%") {
        name.push_str(&format!(" {padded}"));
    }
    sanitize_file_name(&name)
}

pub fn sanitize_file_name(name: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    for c in name.chars() {
        let c = if matches!(c, '/' | '\\' | '?' | '*' | ':' | '"' | '<' | '>' | '|')
            || c.is_control()
        {
            '_'
        } else {
            c
        };
        if c.is_whitespace() {
            if !last_space && !out.is_empty() {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    let trimmed = out.trim().trim_end_matches(['.', ' ']);
    let capped: String = trimmed.chars().take(120).collect();
    let capped = capped.trim_end_matches(['.', ' ']).to_string();
    if capped.is_empty() {
        "часть".to_string()
    } else {
        capped
    }
}

/// Полный путь файла части (`.txt`) внутри `dir`.
pub fn part_target(
    dir: &Path,
    config: &SplitConfig,
    part: &BookPart,
    index: usize,
    total: usize,
    source_name: &str,
) -> PathBuf {
    dir.join(format!(
        "{}.txt",
        part_file_name(config, part, index, total, source_name)
    ))
}

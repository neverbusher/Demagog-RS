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
    let number = config.start_number.saturating_add(index);
    let width = config
        .start_number
        .saturating_add(total.saturating_sub(1))
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
    let mut name = render_template(&template, &padded, source_name, &first_line, header);
    if config.add_number && !template.contains("%Number%") {
        name.push_str(&format!(" {padded}"));
    }
    sanitize_file_name(&name)
}

fn render_template(
    template: &str,
    number: &str,
    source: &str,
    first: &str,
    header: &str,
) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(relative_end) = tail[1..].find('%') else {
            out.push_str(tail);
            return out;
        };
        let end = relative_end + 2;
        let token = &tail[..end];
        match token {
            "%Number%" => out.push_str(number),
            "%FileName%" => out.push_str(source),
            "%FirstLine%" => out.push_str(first),
            "%Header%" => out.push_str(header),
            _ => out.push_str(token),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
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
    } else if is_windows_reserved_name(&capped) {
        let prefixed: String = format!("_{capped}").chars().take(120).collect();
        prefixed.trim_end_matches(['.', ' ']).to_string()
    } else {
        capped
    }
}

fn is_windows_reserved_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or("");
    let normalized: String = base
        .chars()
        .map(|ch| match ch {
            '¹' => '1',
            '²' => '2',
            '³' => '3',
            other => other,
        })
        .collect();
    let upper = normalized.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) || matches!(
        upper.as_str(),
        "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
    ) || matches!(
        upper.as_str(),
        "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    )
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

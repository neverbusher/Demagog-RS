//! Чистая нарезка текста книги на части (без ввода-вывода).

use crate::domain::{BookHeading, SplitConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookPart {
    pub title: Option<String>,
    pub text: String,
}

struct Boundary {
    offset: usize,
    title: Option<String>,
    rank: u8,
}

pub fn validate_config(config: &SplitConfig) -> Result<(), String> {
    if !(config.by_size
        || config.by_keywords
        || config.by_blank_lines
        || config.by_all_caps
        || config.by_toc)
    {
        return Err("Включите хотя бы один способ деления".to_string());
    }
    if config.by_keywords && parse_keywords(&config.keywords).is_empty() {
        return Err("Введите одно или несколько ключевых слов для поиска".to_string());
    }
    Ok(())
}

pub fn split_book(text: &str, headings: &[BookHeading], config: &SplitConfig) -> Vec<BookPart> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let boundaries = collect_boundaries(text, headings, config);
    let parts = form_parts(text, &boundaries, config);
    if config.by_size {
        apply_size_cap(parts, config.max_chars)
    } else {
        parts
    }
}

fn parse_keywords(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|k| k.trim().to_lowercase())
        .filter(|k| !k.is_empty())
        .collect()
}

fn is_all_caps(line: &str) -> bool {
    let mut has_cased = false;
    for c in line.chars() {
        if c.is_lowercase() {
            return false;
        }
        if c.is_uppercase() {
            has_cased = true;
        }
    }
    has_cased
}

fn collect_boundaries(text: &str, headings: &[BookHeading], config: &SplitConfig) -> Vec<Boundary> {
    let mut boundaries: Vec<Boundary> = Vec::new();
    let keywords = parse_keywords(&config.keywords);
    let mut blank_run = 0usize;
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let line_body = line.trim_end_matches(['\n', '\r']);
        let trimmed = line_body.trim();
        if trimmed.is_empty() {
            blank_run += 1;
        } else {
            let line_start = offset + (line_body.len() - line_body.trim_start().len());
            if config.by_blank_lines && blank_run >= 2 {
                boundaries.push(Boundary {
                    offset: line_start,
                    title: None,
                    rank: 0,
                });
            }
            let lower = trimmed.to_lowercase();
            if config.by_keywords && keywords.iter().any(|k| lower.starts_with(k.as_str())) {
                boundaries.push(Boundary {
                    offset: line_start,
                    title: Some(trimmed.to_string()),
                    rank: 1,
                });
            }
            if config.by_all_caps && is_all_caps(trimmed) {
                boundaries.push(Boundary {
                    offset: line_start,
                    title: Some(trimmed.to_string()),
                    rank: 1,
                });
            }
            blank_run = 0;
        }
        offset += line.len();
    }
    if config.by_toc {
        for heading in headings {
            if heading.offset <= text.len() && text.is_char_boundary(heading.offset) {
                boundaries.push(Boundary {
                    offset: heading.offset,
                    title: Some(heading.title.clone()),
                    rank: 2,
                });
            }
        }
    }
    boundaries.sort_by(|a, b| a.offset.cmp(&b.offset).then(b.rank.cmp(&a.rank)));
    boundaries.dedup_by_key(|b| b.offset);
    boundaries
}

fn part_text(piece: &str, strip_heading: bool) -> &str {
    let trimmed = piece.trim();
    if strip_heading {
        trimmed
            .split_once('\n')
            .map(|(_, rest)| rest.trim())
            .unwrap_or("")
    } else {
        trimmed
    }
}

fn form_parts(text: &str, boundaries: &[Boundary], config: &SplitConfig) -> Vec<BookPart> {
    let mut parts: Vec<BookPart> = Vec::new();
    let mut start = 0usize;
    let mut current_title: Option<String> = None;
    let mut strip_heading = false;
    for boundary in boundaries {
        if boundary.offset == 0 {
            current_title = boundary.title.clone();
            strip_heading = boundary.title.is_some();
            continue;
        }
        if boundary.offset <= start {
            continue;
        }
        let candidate = part_text(
            &text[start..boundary.offset],
            strip_heading && config.remove_heading,
        );
        if config.min_chars > 0 && candidate.chars().count() < config.min_chars {
            continue;
        }
        if !candidate.is_empty() {
            parts.push(BookPart {
                title: current_title.take(),
                text: candidate.to_string(),
            });
        }
        start = boundary.offset;
        current_title = boundary.title.clone();
        strip_heading = boundary.title.is_some();
    }
    let tail = part_text(&text[start..], strip_heading && config.remove_heading);
    if !tail.is_empty() {
        parts.push(BookPart {
            title: current_title,
            text: tail.to_string(),
        });
    }
    parts
}

fn apply_size_cap(parts: Vec<BookPart>, max_chars: usize) -> Vec<BookPart> {
    let max_chars = max_chars.max(1);
    let mut out = Vec::new();
    for part in parts {
        if part.text.chars().count() <= max_chars {
            out.push(part);
            continue;
        }
        for (index, sub) in split_text_by_size(&part.text, max_chars)
            .into_iter()
            .enumerate()
        {
            out.push(BookPart {
                title: if index == 0 { part.title.clone() } else { None },
                text: sub,
            });
        }
    }
    out
}

fn split_text_by_size(text: &str, max_chars: usize) -> Vec<String> {
    use crate::services::chunker::{split_into_sentences, split_long_sentence};
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            // Пустые строки не теряем: добавляем в текущий чанк, если места позволяет.
            if !current.is_empty() {
                if current.chars().count() + 1 > max_chars {
                    chunks.push(std::mem::take(&mut current));
                } else {
                    current.push('\n');
                }
            }
            continue;
        }
        if line.chars().count() > max_chars {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            for sentence in split_into_sentences(line) {
                if sentence.chars().count() > max_chars {
                    if !current.is_empty() {
                        chunks.push(std::mem::take(&mut current));
                    }
                    chunks.extend(split_long_sentence(&sentence, max_chars));
                } else {
                    push_line(&mut chunks, &mut current, &sentence, max_chars);
                }
            }
            continue;
        }
        push_line(&mut chunks, &mut current, line, max_chars);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn push_line(chunks: &mut Vec<String>, current: &mut String, line: &str, max_chars: usize) {
    let sep = if current.is_empty() { 0 } else { 1 };
    if !current.is_empty() && current.chars().count() + sep + line.chars().count() > max_chars {
        chunks.push(std::mem::take(current));
    }
    if !current.is_empty() {
        current.push('\n');
    }
    current.push_str(line);
}

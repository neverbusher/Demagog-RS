//! Чистая нарезка текста книги на части (без ввода-вывода).

use regex::Regex;
use std::sync::atomic::AtomicBool;

use crate::domain::{AppError, AppResult, BookHeading, SplitConfig};
use crate::ports::is_cancelled;

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

pub fn validate_config(config: &SplitConfig) -> AppResult<()> {
    if !(config.by_size
        || config.by_keywords
        || config.by_blank_lines
        || config.by_all_caps
        || config.by_toc)
    {
        return Err(AppError::invalid_data(
            "Включите хотя бы один способ деления",
        ));
    }
    if config.by_keywords && parse_keywords(&config.keywords).is_empty() {
        return Err(AppError::invalid_data(
            "Введите одно или несколько ключевых слов для поиска",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub fn split_book(text: &str, headings: &[BookHeading], config: &SplitConfig) -> Vec<BookPart> {
    split_book_with_cancel(text, headings, config, None).unwrap_or_default()
}

pub(crate) fn split_book_with_cancel(
    text: &str,
    headings: &[BookHeading],
    config: &SplitConfig,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<BookPart>> {
    if is_cancelled(cancel) {
        return Err(AppError::cancelled("Нарезка книги отменена"));
    }
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let boundaries = collect_boundaries(text, headings, config, cancel)?;
    let parts = if boundaries.is_empty() {
        vec![BookPart {
            title: None,
            text: text.to_string(),
        }]
    } else {
        form_parts(text, &boundaries, config, cancel)?
    };
    if config.by_size {
        apply_size_cap(parts, config.max_chars, cancel)
    } else {
        Ok(parts)
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

fn collect_boundaries(
    text: &str,
    headings: &[BookHeading],
    config: &SplitConfig,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<Boundary>> {
    if is_cancelled(cancel) {
        return Err(AppError::cancelled("Нарезка книги отменена"));
    }
    let mut boundaries: Vec<Boundary> = Vec::new();
    let keywords = parse_keywords(&config.keywords);
    let keyword_pattern = if config.by_keywords && !keywords.is_empty() {
        let alternatives = keywords
            .iter()
            .map(|keyword| regex::escape(keyword))
            .collect::<Vec<_>>()
            .join("|");
        Some(
            Regex::new(&format!(r"^(?:{alternatives})(?:$|\p{{N}}|\W)"))
                .map_err(|_| AppError::invalid_data("Слишком длинный список ключевых слов"))?,
        )
    } else {
        None
    };
    let mut blank_run = 0usize;
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        if is_cancelled(cancel) {
            return Err(AppError::cancelled("Нарезка книги отменена"));
        }
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
            if keyword_pattern
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(&lower))
            {
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
            if is_cancelled(cancel) {
                return Err(AppError::cancelled("Нарезка книги отменена"));
            }
            if heading.offset <= text.len()
                && text.is_char_boundary(heading.offset)
                && (heading.offset == 0 || text[..heading.offset].ends_with('\n'))
            {
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
    Ok(boundaries)
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

fn content_after_heading(text: &str, offset: usize) -> usize {
    text[offset..]
        .find('\n')
        .map(|relative| offset + relative + 1)
        .unwrap_or(text.len())
}

fn form_parts(
    text: &str,
    boundaries: &[Boundary],
    config: &SplitConfig,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<BookPart>> {
    let mut parts: Vec<BookPart> = Vec::new();
    let mut start = 0usize;
    let mut content_start = 0usize;
    let mut current_title: Option<String> = None;
    let mut strip_heading = false;
    for boundary in boundaries {
        if is_cancelled(cancel) {
            return Err(AppError::cancelled("Нарезка книги отменена"));
        }
        if boundary.offset == 0 {
            current_title = boundary.title.clone();
            strip_heading = boundary.title.is_some();
            content_start = if boundary.title.is_some() {
                content_after_heading(text, boundary.offset)
            } else {
                boundary.offset
            };
            continue;
        }
        if boundary.offset <= start {
            continue;
        }
        if text[content_start..boundary.offset].trim().is_empty() {
            if config.remove_heading {
                start = boundary.offset;
            }
            if config.remove_heading || boundary.title.is_some() {
                current_title = boundary.title.clone();
            }
            strip_heading = boundary.title.is_some();
            content_start = if boundary.title.is_some() {
                content_after_heading(text, boundary.offset)
            } else {
                boundary.offset
            };
            continue;
        }
        let candidate = part_text(
            &text[start..boundary.offset],
            strip_heading && config.remove_heading,
        );
        if candidate.is_empty() {
            start = boundary.offset;
            current_title = boundary.title.clone();
            strip_heading = boundary.title.is_some();
            content_start = if boundary.title.is_some() {
                content_after_heading(text, boundary.offset)
            } else {
                boundary.offset
            };
            continue;
        }
        if config.min_chars > 0 && candidate.chars().count() < config.min_chars {
            continue;
        }
        parts.push(BookPart {
            title: current_title.take(),
            text: candidate.to_string(),
        });
        start = boundary.offset;
        current_title = boundary.title.clone();
        strip_heading = boundary.title.is_some();
        content_start = if boundary.title.is_some() {
            content_after_heading(text, boundary.offset)
        } else {
            boundary.offset
        };
    }
    if is_cancelled(cancel) {
        return Err(AppError::cancelled("Нарезка книги отменена"));
    }
    let tail = part_text(&text[start..], strip_heading && config.remove_heading);
    if !tail.is_empty() {
        if text[content_start..].trim().is_empty()
            && !config.remove_heading
            && let Some(previous) = parts.last_mut()
        {
            let separator = if text[..start].ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            previous.text.push_str(separator);
            previous.text.push_str(tail);
        } else {
            parts.push(BookPart {
                title: current_title,
                text: tail.to_string(),
            });
        }
    }
    Ok(parts)
}

fn apply_size_cap(
    parts: Vec<BookPart>,
    max_chars: usize,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<BookPart>> {
    let max_chars = max_chars.max(1);
    let mut out = Vec::new();
    for part in parts {
        if is_cancelled(cancel) {
            return Err(AppError::cancelled("Нарезка книги отменена"));
        }
        if part.text.chars().count() <= max_chars {
            out.push(part);
            continue;
        }
        for (index, sub) in split_text_by_size(&part.text, max_chars, cancel)?
            .into_iter()
            .enumerate()
        {
            out.push(BookPart {
                title: if index == 0 { part.title.clone() } else { None },
                text: sub,
            });
        }
    }
    Ok(out)
}

fn split_text_by_size(
    text: &str,
    max_chars: usize,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<String>> {
    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0;
    // Preserve exact whitespace with adjacent content; only whitespace may exceed the cap.
    let mut pending = String::new();
    let mut pending_chars = 0;
    while start < text.len() {
        if is_cancelled(cancel) {
            return Err(AppError::cancelled("Нарезка книги отменена"));
        }
        let available = max_chars.saturating_sub(pending_chars).max(1);
        let end = text[start..]
            .char_indices()
            .nth(available)
            .map(|(offset, _)| start + offset)
            .unwrap_or(text.len());
        let mut cut = end;
        if end < text.len() {
            let window = &text[start..end];
            if let Some(paragraph) = last_paragraph_boundary(window) {
                cut = start + paragraph;
            } else if let Some(sentence) = last_sentence_boundary(window, Some(&text[end..])) {
                cut = start + sentence;
            } else if let Some(space) = window.rfind(char::is_whitespace) {
                let whitespace = window[space..].chars().next().unwrap();
                if whitespace == '\r' {
                    if window[space..].starts_with("\r\n") {
                        cut = start + space + 2;
                    } else if text[end..].starts_with('\n') {
                        cut = start + space;
                    } else {
                        cut = start + space + 1;
                    }
                } else {
                    cut = start + space + whitespace.len_utf8();
                }
            }
        }
        if cut <= start {
            cut = end;
        }
        if cut == end && end < text.len() {
            let window = &text[start..end];
            if let Some(safe) = sentence_unit_crossing_window(window, &text[end..]) {
                let candidate = start + safe;
                if candidate > start {
                    cut = candidate;
                } else {
                    let mut extended = end;
                    while let Some(ch) = text[extended..].chars().next() {
                        if !is_closing_quote(ch) {
                            break;
                        }
                        extended += ch.len_utf8();
                    }
                    cut = extended;
                }
            }
        }
        let piece = &text[start..cut];
        let mut combined = std::mem::take(&mut pending);
        combined.push_str(piece);
        if piece.trim().is_empty() {
            pending = combined;
            pending_chars += piece.chars().count();
        } else {
            let rebalanced = if combined.chars().count() > max_chars {
                chunks
                    .last()
                    .and_then(|last| try_split_balanced(last, &combined, max_chars))
            } else {
                None
            };
            if let Some((prefix, suffix)) = rebalanced {
                *chunks.last_mut().expect("last chunk exists") = prefix;
                chunks.push(suffix);
            } else {
                chunks.push(combined);
            }
            pending_chars = 0;
        }
        start = cut;
    }
    if !pending.is_empty() {
        let rebalanced = chunks
            .last()
            .and_then(|last| try_split_balanced(last, &pending, max_chars));
        if let Some((prefix, suffix)) = rebalanced {
            *chunks.last_mut().expect("last chunk exists") = prefix;
            chunks.push(suffix);
        } else if let Some(last) = chunks.last_mut() {
            last.push_str(&pending);
        } else {
            chunks.push(pending);
        }
    }
    Ok(chunks)
}

fn try_split_balanced(left: &str, right: &str, max_chars: usize) -> Option<(String, String)> {
    let mut combined = String::with_capacity(left.len().saturating_add(right.len()));
    combined.push_str(left);
    combined.push_str(right);
    let total_chars = combined.chars().count();
    if total_chars <= max_chars {
        return None;
    }
    let leading_whitespace = combined.chars().take_while(|ch| ch.is_whitespace()).count();
    let trailing_whitespace = combined
        .chars()
        .rev()
        .take_while(|ch| ch.is_whitespace())
        .count();
    if leading_whitespace == total_chars {
        return None;
    }
    let first_content = leading_whitespace;
    let last_content = total_chars - trailing_whitespace - 1;
    let min_split = total_chars.saturating_sub(max_chars);
    let max_split = max_chars.min(total_chars - 1);
    for (char_index, (split_at, next)) in combined.char_indices().enumerate() {
        if char_index < min_split {
            continue;
        }
        if char_index > max_split {
            break;
        }
        if char_index <= first_content || char_index > last_content {
            continue;
        }
        let previous = combined[..split_at].chars().next_back();
        if previous == Some('\r') && next == '\n' {
            continue;
        }
        if is_closing_quote(next)
            && previous.is_some_and(|ch| is_sentence_punctuation(ch) || is_closing_quote(ch))
        {
            continue;
        }
        return Some((
            combined[..split_at].to_string(),
            combined[split_at..].to_string(),
        ));
    }
    None
}

fn sentence_unit_crossing_window(window: &str, following: &str) -> Option<usize> {
    if !following.chars().next().is_some_and(is_closing_quote) {
        return None;
    }
    let mut end = window.len();
    while let Some(ch) = window[..end].chars().next_back() {
        if !is_closing_quote(ch) {
            break;
        }
        end -= ch.len_utf8();
    }
    let punctuation = window[..end].chars().next_back()?;
    is_sentence_punctuation(punctuation).then_some(end - punctuation.len_utf8())
}

fn is_sentence_punctuation(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?' | '…')
}

fn is_closing_quote(ch: char) -> bool {
    matches!(ch, '»' | '”' | '’' | '"' | '\'' | ')' | ']' | '}')
}

fn last_paragraph_boundary(text: &str) -> Option<usize> {
    [
        text.rfind("\n\n").map(|pos| pos + 2),
        text.rfind("\r\n\r\n").map(|pos| pos + 4),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn last_sentence_boundary(text: &str, following: Option<&str>) -> Option<usize> {
    text.char_indices().rev().find_map(|(offset, ch)| {
        if !is_sentence_punctuation(ch) {
            return None;
        }
        let mut end = offset + ch.len_utf8();
        let mut rest = &text[end..];
        while let Some(closing) = rest.chars().next() {
            if !is_closing_quote(closing) {
                break;
            }
            end += closing.len_utf8();
            rest = &text[end..];
        }
        if end == text.len() {
            return match following {
                None => Some(end),
                Some(next) => next
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
                    .then_some(end),
            };
        }
        rest.chars()
            .next()
            .is_some_and(char::is_whitespace)
            .then_some(end)
    })
}

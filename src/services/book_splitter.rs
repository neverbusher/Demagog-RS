use std::collections::HashSet;
use std::path::PathBuf;

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

#[derive(Debug, Default)]
pub struct ExportSummary {
    pub saved: usize,
    pub dir: PathBuf,
    pub errors: Vec<String>,
}

pub fn export_parts(
    parts: &[BookPart],
    config: &SplitConfig,
    source_name: &str,
) -> Result<ExportSummary, String> {
    let base = config
        .output_dir
        .as_ref()
        .ok_or("Не выбрана папка назначения")?;
    let dir = if config.subfolder {
        base.join(sanitize_file_name(source_name))
    } else {
        base.clone()
    };
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Не удалось создать папку {}: {e}", dir.display()))?;
    let mut used: HashSet<String> = HashSet::new();
    let mut summary = ExportSummary {
        dir: dir.clone(),
        ..Default::default()
    };
    for (index, part) in parts.iter().enumerate() {
        let name = part_file_name(config, part, index, parts.len(), source_name);
        let target = crate::services::batch::unique_target(
            &dir,
            &format!("{name}.txt"),
            config.overwrite,
            &used,
        );
        used.insert(crate::services::batch::target_key(&target));
        match std::fs::write(&target, &part.text) {
            Ok(()) => summary.saved += 1,
            Err(e) => summary.errors.push(format!("{}: {e}", target.display())),
        }
    }
    Ok(summary)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SplitConfig {
        SplitConfig {
            by_size: false,
            by_keywords: false,
            add_number: false,
            ..Default::default()
        }
    }

    #[test]
    fn keywords_split_case_insensitive_at_line_start() {
        let config = SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            ..cfg()
        };
        let text = "Вступление.\nглава 1\nТекст один.\nГЛАВА 2\nТекст два.";
        let parts = split_book(text, &[], &config);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].title, None);
        assert_eq!(parts[0].text, "Вступление.");
        assert_eq!(parts[1].title.as_deref(), Some("глава 1"));
        assert_eq!(parts[1].text, "глава 1\nТекст один.");
        assert_eq!(parts[2].text, "ГЛАВА 2\nТекст два.");
    }

    #[test]
    fn keyword_mid_line_is_not_a_boundary() {
        let config = SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            ..cfg()
        };
        let text = "Это была глава о войне.\nГлава 2\nТекст.";
        let parts = split_book(text, &[], &config);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].text, "Это была глава о войне.");
    }

    #[test]
    fn remove_heading_strips_first_line() {
        let config = SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            remove_heading: true,
            ..cfg()
        };
        let text = "Глава 1\nТекст один.\nГлава 2\nТекст два.";
        let parts = split_book(text, &[], &config);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].text, "Текст один.");
        assert_eq!(parts[0].title.as_deref(), Some("Глава 1"));
    }

    #[test]
    fn all_caps_lines_split_but_mixed_case_do_not() {
        let config = SplitConfig {
            by_all_caps: true,
            ..cfg()
        };
        let text = "ГЛАВА 1. НАЧАЛО\nтекст\nГлава обычная\nПРОДОЛЖЕНИЕ 2\nещё текст";
        let parts = split_book(text, &[], &config);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].text, "ГЛАВА 1. НАЧАЛО\nтекст\nГлава обычная");
        assert_eq!(parts[1].title.as_deref(), Some("ПРОДОЛЖЕНИЕ 2"));
    }

    #[test]
    fn two_blank_lines_split() {
        let config = SplitConfig {
            by_blank_lines: true,
            ..cfg()
        };
        let text = "Абзац.\n\n\nВторой.\n \n \nТретий.";
        let parts = split_book(text, &[], &config);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1].text, "Второй.");
        assert_eq!(parts[2].text, "Третий.");
    }

    #[test]
    fn toc_boundaries_use_headings() {
        let config = SplitConfig {
            by_toc: true,
            ..cfg()
        };
        let text = "Преамбула.\nГлава 1\nТекст один.\nГлава 2\nТекст два.";
        let off1 = text.find("Глава 1").unwrap();
        let off2 = text.find("Глава 2").unwrap();
        let headings = vec![
            BookHeading {
                offset: off1,
                title: "Глава 1".to_string(),
            },
            BookHeading {
                offset: off2,
                title: "Глава 2".to_string(),
            },
        ];
        let parts = split_book(text, &headings, &config);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].text, "Преамбула.");
        assert_eq!(parts[1].title.as_deref(), Some("Глава 1"));
        assert_eq!(parts[2].text, "Глава 2\nТекст два.");
    }

    #[test]
    fn methods_union_and_dedupe_boundaries() {
        let config = SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            by_all_caps: true,
            ..cfg()
        };
        // "ГЛАВА 1" matches both keywords (case-insensitive) and all-caps: one boundary.
        let text = "Вступление.\nГЛАВА 1\nТекст.";
        let parts = split_book(text, &[], &config);
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn empty_text_gives_no_parts() {
        assert!(split_book("  \n ", &[], &cfg()).is_empty());
    }

    #[test]
    fn no_methods_give_single_part() {
        let parts = split_book("Просто текст.", &[], &cfg());
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].text, "Просто текст.");
    }

    #[test]
    fn validate_config_rejects_no_methods_and_empty_keywords() {
        assert!(validate_config(&cfg()).is_err());
        let config = SplitConfig {
            by_keywords: true,
            keywords: " , ".to_string(),
            ..cfg()
        };
        assert!(validate_config(&config).is_err());
        let config = SplitConfig {
            by_keywords: true,
            ..cfg()
        };
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn min_size_merges_small_parts_into_previous() {
        let config = SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            min_chars: 100,
            ..cfg()
        };
        let text = "Глава 1\nКоротко.\nГлава 2\nДлинный текст второй главы.";
        let parts = split_book(text, &[], &config);
        assert_eq!(parts.len(), 1);
        assert!(parts[0].text.contains("Глава 2"));
        let config = SplitConfig {
            min_chars: 0,
            ..config
        };
        assert_eq!(split_book(text, &[], &config).len(), 2);
    }

    #[test]
    fn size_cap_splits_long_parts_at_paragraphs() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 50,
            ..cfg()
        };
        let para = "Абзац из примерно тридцати символов.";
        let text = format!("{para}\n{para}\n{para}\n{para}");
        let parts = split_book(&text, &[], &config);
        assert!(parts.len() >= 3);
        for part in &parts {
            assert!(part.text.chars().count() <= 50, "{}", part.text);
        }
        assert!(parts.iter().all(|p| !p.text.is_empty()));
    }

    #[test]
    fn size_only_mode_splits_plain_text() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 1_000,
            ..cfg()
        };
        let text = "Слово ".repeat(500);
        let parts = split_book(&text, &[], &config);
        assert!(parts.len() >= 3);
        for part in &parts {
            assert!(part.text.chars().count() <= 1_000);
        }
    }

    #[test]
    fn overlong_paragraph_is_split_by_sentences_and_words() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 80,
            ..cfg()
        };
        let sentence = "Предложение из нескольких слов, которое точно длиннее лимита. ".repeat(3);
        let parts = split_book(&sentence, &[], &config);
        assert!(parts.len() >= 3);
        for part in &parts {
            assert!(part.text.chars().count() <= 80);
        }
        let word = "я".repeat(300);
        let parts = split_book(&word, &[], &config);
        assert!(parts.iter().all(|p| p.text.chars().count() <= 80));
        assert_eq!(
            parts
                .iter()
                .map(|p| p.text.clone())
                .collect::<Vec<_>>()
                .concat()
                .chars()
                .count(),
            300
        );
    }

    #[test]
    fn size_cap_subparts_inherit_title_only_first() {
        let config = SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            by_size: true,
            max_chars: 60,
            ..cfg()
        };
        let text = format!("Глава 1\n{}", "Абзац книги. ".repeat(20));
        let parts = split_book(&text, &[], &config);
        assert!(parts.len() >= 2);
        assert_eq!(parts[0].title.as_deref(), Some("Глава 1"));
        assert!(parts[1..].iter().all(|p| p.title.is_none()));
    }

    #[test]
    fn toc_heading_with_invalid_offset_is_skipped() {
        let config = SplitConfig {
            by_toc: true,
            ..cfg()
        };
        let text = "Глава\nТекст.";
        // offset 1 lands inside the UTF-8 sequence of 'Г' — must be skipped, not panic
        let headings = vec![BookHeading {
            offset: 1,
            title: "X".to_string(),
        }];
        let parts = split_book(text, &headings, &config);
        assert_eq!(parts.len(), 1);
    }

    #[test]
    fn template_variables_and_number_padding() {
        let config = SplitConfig {
            template: "%Number% %FirstLine%".to_string(),
            ..cfg()
        };
        let part = BookPart {
            title: None,
            text: "Глава первая\nТекст.".to_string(),
        };
        assert_eq!(
            part_file_name(&config, &part, 0, 12, "book"),
            "01 Глава первая"
        );
        assert_eq!(
            part_file_name(&config, &part, 11, 12, "book"),
            "12 Глава первая"
        );
        assert_eq!(
            part_file_name(&config, &part, 0, 100, "book"),
            "001 Глава первая"
        );
    }

    #[test]
    fn header_falls_back_to_first_line_and_filename_variable() {
        let config = SplitConfig {
            template: "%FileName% - %Header%".to_string(),
            ..cfg()
        };
        let part = BookPart {
            title: None,
            text: "Первая строка\nвторая".to_string(),
        };
        assert_eq!(
            part_file_name(&config, &part, 0, 1, "Книга"),
            "Книга - Первая строка"
        );
        let part = BookPart {
            title: Some("Заголовок".to_string()),
            text: "Текст.".to_string(),
        };
        assert_eq!(
            part_file_name(&config, &part, 0, 1, "Книга"),
            "Книга - Заголовок"
        );
    }

    #[test]
    fn first_line_is_trimmed_and_capped_at_60_chars() {
        let config = SplitConfig {
            template: "%FirstLine%".to_string(),
            ..cfg()
        };
        let long = "я".repeat(100);
        let part = BookPart {
            title: None,
            text: format!("\n  {long}  \nвторая"),
        };
        let name = part_file_name(&config, &part, 0, 1, "book");
        assert_eq!(name.chars().count(), 60);
    }

    #[test]
    fn number_appended_when_template_has_no_number_variable() {
        let config = SplitConfig {
            template: "%FileName%".to_string(),
            add_number: true,
            ..cfg()
        };
        let part = BookPart {
            title: None,
            text: "Текст.".to_string(),
        };
        assert_eq!(part_file_name(&config, &part, 4, 10, "book"), "book 05");
        let config = SplitConfig {
            add_number: false,
            ..config
        };
        assert_eq!(part_file_name(&config, &part, 4, 10, "book"), "book");
    }

    #[test]
    fn empty_template_uses_default_and_unknown_variables_stay() {
        let config = SplitConfig {
            template: "  ".to_string(),
            ..cfg()
        };
        let part = BookPart {
            title: None,
            text: "Текст.".to_string(),
        };
        assert_eq!(part_file_name(&config, &part, 0, 1, "book"), "01 Текст");
        let config = SplitConfig {
            template: "%What% глава".to_string(),
            add_number: false,
            ..cfg()
        };
        assert_eq!(part_file_name(&config, &part, 0, 1, "book"), "%What% глава");
    }

    #[test]
    fn sanitize_replaces_forbidden_and_control_chars() {
        assert_eq!(
            sanitize_file_name("a/b\\c?d*e:f\"g<h>i|j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(sanitize_file_name("a\tb\nc"), "a_b_c");
    }

    #[test]
    fn sanitize_collapses_whitespace_and_trims_trailing_dots() {
        assert_eq!(
            sanitize_file_name("  много   пробелов...  "),
            "много пробелов"
        );
        assert_eq!(sanitize_file_name("..."), "часть");
        assert_eq!(sanitize_file_name(""), "часть");
    }

    #[test]
    fn sanitize_caps_length_at_120_chars() {
        let name = sanitize_file_name(&"я".repeat(300));
        assert_eq!(name.chars().count(), 120);
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "demagog-split-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn export_writes_all_parts_as_utf8_txt() {
        let dir = temp_dir("export");
        let config = SplitConfig {
            output_dir: Some(dir.clone()),
            ..cfg()
        };
        let parts = vec![
            BookPart {
                title: None,
                text: "Первая часть.".to_string(),
            },
            BookPart {
                title: None,
                text: "Вторая часть.".to_string(),
            },
        ];
        let summary = export_parts(&parts, &config, "book").expect("export");
        assert_eq!(summary.saved, 2);
        assert!(summary.errors.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.join("01 Первая часть.txt")).unwrap(),
            "Первая часть."
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("02 Вторая часть.txt")).unwrap(),
            "Вторая часть."
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_subfolder_uses_sanitized_book_name() {
        let dir = temp_dir("subfolder");
        let config = SplitConfig {
            output_dir: Some(dir.clone()),
            subfolder: true,
            ..cfg()
        };
        let parts = vec![BookPart {
            title: None,
            text: "Текст.".to_string(),
        }];
        let summary = export_parts(&parts, &config, "Моя: книга").expect("export");
        assert_eq!(summary.dir, dir.join("Моя_ книга"));
        assert!(summary.dir.join("01 Текст.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_overwrite_off_keeps_existing_and_suffixes() {
        let dir = temp_dir("nooverwrite");
        std::fs::write(dir.join("01 Текст.txt"), "старое").unwrap();
        let config = SplitConfig {
            output_dir: Some(dir.clone()),
            overwrite: false,
            ..cfg()
        };
        let parts = vec![BookPart {
            title: None,
            text: "Текст.".to_string(),
        }];
        export_parts(&parts, &config, "book").expect("export");
        assert_eq!(
            std::fs::read_to_string(dir.join("01 Текст.txt")).unwrap(),
            "старое"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("01 Текст (2).txt")).unwrap(),
            "Текст."
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_same_run_collision_gets_suffix() {
        let dir = temp_dir("collision");
        let config = SplitConfig {
            output_dir: Some(dir.clone()),
            template: "%FirstLine%".to_string(),
            add_number: false,
            ..cfg()
        };
        let parts = vec![
            BookPart {
                title: None,
                text: "Одинаково.".to_string(),
            },
            BookPart {
                title: None,
                text: "Одинаково.".to_string(),
            },
        ];
        let summary = export_parts(&parts, &config, "book").expect("export");
        assert_eq!(summary.saved, 2);
        assert!(dir.join("Одинаково.txt").exists());
        assert!(dir.join("Одинаково (2).txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_without_output_dir_errors() {
        let parts = vec![BookPart {
            title: None,
            text: "Текст.".to_string(),
        }];
        assert!(export_parts(&parts, &cfg(), "book").is_err());
    }

    #[test]
    fn export_blocked_dir_errors() {
        let dir = temp_dir("blocked");
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "file, not a dir").unwrap();
        let config = SplitConfig {
            output_dir: Some(blocker.join("out")),
            ..cfg()
        };
        let parts = vec![BookPart {
            title: None,
            text: "Текст.".to_string(),
        }];
        assert!(export_parts(&parts, &config, "book").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

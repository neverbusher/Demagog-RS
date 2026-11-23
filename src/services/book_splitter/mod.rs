//! Книгорез: нарезка книги на TXT-части.
//!
//! Модуль разделён по ответственности:
//! * [`split`] — чистая нарезка текста (без ввода-вывода);
//! * [`naming`] — именование и санитизация файлов частей;
//! * [`export`] — запись частей на диск;
//! * [`batch`] — оркестрация пакетной нарезки и прогресс.
//!
//! Публичный API ре-экспортируется здесь, чтобы вызывающий код
//! обращался к `book_splitter::…` независимо от внутренней раскладки.

mod batch;
mod export;
mod naming;
mod split;

pub use batch::{BatchSplitProgress, BatchSplitSummary, run_split_batch};
pub use export::export_parts;
pub use naming::{part_file_name, sanitize_file_name};
pub use split::{BookPart, split_book, validate_config};

// Вариант экспорта с общим `used`, используемый тестами.
#[cfg(test)]
pub use export::export_parts_into;

// Импорты ниже нужны только тестам (они делают `use super::*`).
#[cfg(test)]
use crate::domain::{BookHeading, SplitConfig};
#[cfg(test)]
use crate::services::batch::BatchInput;
#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::path::PathBuf;
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
    fn size_cap_preserves_blank_lines() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 30,
            ..cfg()
        };
        let text = "Абзац один.\n\nАбзац два.\n\nАбзац три.";
        let parts = split_book(text, &[], &config);
        assert!(parts[0].text.contains("\n\n"));
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

    fn split_batch_options(output_dir: PathBuf) -> SplitConfig {
        SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            template: "%FirstLine%".to_string(),
            add_number: false,
            output_dir: Some(output_dir),
            ..Default::default()
        }
    }

    fn write_book(dir: &std::path::Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write book");
        path
    }

    fn flat(paths: &[PathBuf]) -> Vec<BatchInput> {
        paths
            .iter()
            .map(|path| BatchInput {
                path: path.clone(),
                base_dir: path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(""))
                    .to_path_buf(),
            })
            .collect()
    }

    #[test]
    fn export_parts_into_shares_used_across_calls() {
        let dir = temp_dir("sharedused");
        let config = SplitConfig {
            output_dir: Some(dir.clone()),
            template: "%FirstLine%".to_string(),
            add_number: false,
            ..cfg()
        };
        let part = BookPart {
            title: None,
            text: "Одинаково.".to_string(),
        };
        let mut used = HashSet::new();
        export_parts_into(std::slice::from_ref(&part), &config, "book", &mut used)
            .expect("first export");
        let summary =
            export_parts_into(&[part], &config, "book", &mut used).expect("second export");
        assert_eq!(summary.saved, 1);
        assert!(dir.join("Одинаково.txt").exists());
        assert!(dir.join("Одинаково (2).txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_split_batch_exports_parts_per_book() {
        use crate::io::book_reader::CompositeBookReader;
        use std::sync::mpsc;

        let root = temp_dir("splitbatch");
        let input = root.join("in");
        std::fs::create_dir_all(&input).expect("create input dir");
        let book1 = write_book(&input, "book1.txt", "Глава 1\n\nТекст первый.");
        let book2 = write_book(&input, "book2.txt", "Глава 2\n\nТекст второй.");
        let out = root.join("out");

        let (tx, rx) = mpsc::channel();
        let summary = run_split_batch(
            &flat(&[book1, book2]),
            &CompositeBookReader::new(),
            &split_batch_options(out.clone()),
            None,
            Some(&tx),
        );
        drop(tx);

        assert_eq!(
            summary,
            BatchSplitSummary {
                succeeded: 2,
                failed: 0,
                cancelled: false
            }
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book1").join("Глава 1.txt")).unwrap(),
            "Глава 1\n\nТекст первый."
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book2").join("Глава 2.txt")).unwrap(),
            "Глава 2\n\nТекст второй."
        );

        let messages: Vec<BatchSplitProgress> = rx.iter().collect();
        assert_eq!(
            messages
                .iter()
                .filter(|m| matches!(m, BatchSplitProgress::FileStarted { .. }))
                .count(),
            2
        );
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, BatchSplitProgress::FileFinished { outcome: Ok(1), .. }))
        );
        assert!(matches!(
            messages.last(),
            Some(BatchSplitProgress::Completed {
                succeeded: 2,
                failed: 0,
                cancelled: false
            })
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_continues_after_error() {
        use crate::io::book_reader::CompositeBookReader;

        let root = temp_dir("splitcontinue");
        let input = root.join("in");
        std::fs::create_dir_all(&input).expect("create input dir");
        let alpha = write_book(&input, "alpha.txt", "Глава 1\n\nТекст первый.");
        let missing = input.join("missing.txt");
        let beta = write_book(&input, "beta.txt", "Глава 2\n\nТекст второй.");
        let out = root.join("out");

        let summary = run_split_batch(
            &flat(&[alpha, missing, beta]),
            &CompositeBookReader::new(),
            &split_batch_options(out.clone()),
            None,
            None,
        );

        assert_eq!(
            summary,
            BatchSplitSummary {
                succeeded: 2,
                failed: 1,
                cancelled: false
            }
        );
        assert!(out.join("alpha").join("Глава 1.txt").exists());
        assert!(out.join("beta").join("Глава 2.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_fails_when_target_files_exist_with_overwrite_on() {
        use crate::io::book_reader::CompositeBookReader;

        let root = temp_dir("splitblock");
        let input = root.join("in");
        std::fs::create_dir_all(&input).expect("create input dir");
        let book = write_book(&input, "book.txt", "Глава 1\n\nТекст.");
        let out = root.join("out");
        let existing = out.join("book").join("Глава 1.txt");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, "старое").unwrap();

        let summary = run_split_batch(
            &flat(&[book]),
            &CompositeBookReader::new(),
            &split_batch_options(out.clone()),
            None,
            None,
        );

        assert_eq!(
            summary,
            BatchSplitSummary {
                succeeded: 0,
                failed: 1,
                cancelled: false
            }
        );
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "старое");

        let _ = std::fs::remove_dir_all(&root);
    }
}

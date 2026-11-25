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

pub use batch::BatchSplitProgress;
#[cfg(test)]
pub use batch::{BatchSplitFileResult, run_split_batch};
pub(crate) use batch::{run_split_batch_with_confirmation, unique_book_dir};
pub(crate) use export::export_parts_with_cancel;
pub use export::{ExportSummary, planned_part_names};
pub use naming::part_file_name;
pub(crate) use split::split_book_with_cancel;
pub use split::{BookPart, validate_config};

#[cfg(test)]
pub use export::{export_parts, export_parts_into};
#[cfg(test)]
pub use split::split_book;

// Импорты ниже нужны только тестам (они делают `use super::*`).
#[cfg(test)]
use crate::domain::{BookHeading, SplitConfig};
#[cfg(test)]
use crate::services::batch::{BatchCounts, BatchInput, BatchOutcome};
#[cfg(test)]
use std::collections::HashSet;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
mod tests {
    use super::naming::sanitize_file_name;
    use super::*;
    use crate::domain::{AppError, AppFailure, FailureKind};

    fn expect_failure(error: &AppError, kind: FailureKind) -> &AppFailure {
        let AppError::Failed(failure) = error else {
            panic!("expected {kind:?} failure, got {error:?}");
        };
        assert_eq!(failure.kind, kind);
        failure
    }

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
    fn split_book_cancellation_stops_large_scan() {
        use super::split::split_book_with_cancel;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;

        let text = "строка\n".repeat(1_000_000);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            cancel_flag.store(true, Ordering::Relaxed);
        });

        let result = split_book_with_cancel(&text, &[], &cfg(), Some(cancel.as_ref()));

        assert!(
            matches!(result, Err(error) if error.is_cancelled()),
            "large scan must be cancellable"
        );
    }

    #[test]
    fn no_methods_give_single_part() {
        let parts = split_book("Просто текст.", &[], &cfg());
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].text, "Просто текст.");
    }

    #[test]
    fn validate_config_rejects_no_methods_and_empty_keywords() {
        let error = validate_config(&cfg()).expect_err("no split methods must fail");
        let failure = expect_failure(&error, FailureKind::InvalidData);
        assert_eq!(failure.message, "Включите хотя бы один способ деления");
        assert_eq!(error.to_string(), failure.message);

        let config = SplitConfig {
            by_keywords: true,
            keywords: " , ".to_string(),
            ..cfg()
        };
        let error = validate_config(&config).expect_err("empty keywords must fail");
        let failure = expect_failure(&error, FailureKind::InvalidData);
        assert_eq!(
            failure.message,
            "Введите одно или несколько ключевых слов для поиска"
        );
        assert_eq!(error.to_string(), failure.message);

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
    fn min_size_does_not_merge_empty_heading_into_next_chapter() {
        let config = SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            min_chars: 50,
            remove_heading: true,
            ..cfg()
        };
        let text = "Глава 1\nГлава 2\nТекст второй главы, который длиннее минимального размера.";

        let parts = split_book(text, &[], &config);

        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].title.as_deref(), Some("Глава 2"));
        assert_eq!(
            parts[0].text,
            "Текст второй главы, который длиннее минимального размера."
        );
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
    fn size_cap_preserves_exact_text_when_parts_are_joined() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 4,
            ..cfg()
        };
        let text = "  А  \n\n  Б  \n  В  ";

        let parts = split_book(text, &[], &config);

        assert!(parts.len() > 1);
        assert!(parts.iter().all(|part| !part.text.trim().is_empty()));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_does_not_emit_a_whitespace_only_leading_part() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 1_000,
            ..cfg()
        };
        let text = format!("\n\n{}", "a".repeat(1_001));

        let parts = split_book(&text, &[], &config);

        assert_eq!(parts.len(), 2);
        assert!(
            parts
                .iter()
                .all(|part| !part.text.trim().is_empty() && part.text.chars().count() <= 1_000)
        );
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_attaches_long_whitespace_runs_to_adjacent_content() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 1_000,
            ..cfg()
        };
        let text = format!("A{}B", " ".repeat(2_000));

        let parts = split_book(&text, &[], &config);

        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|part| !part.text.trim().is_empty()));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_rebalances_trailing_whitespace_when_the_cap_is_avoidable() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 4,
            ..cfg()
        };
        let text = "aaaa ";

        let parts = split_book(text, &[], &config);

        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|part| part.text.chars().count() <= 4));
        assert!(parts.iter().all(|part| !part.text.trim().is_empty()));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_rebalances_long_trailing_whitespace_within_the_cap() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 10,
            ..cfg()
        };
        let text = format!("{}      ", "b".repeat(10));

        let parts = split_book(&text, &[], &config);

        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|part| part.text.chars().count() <= 10));
        assert!(parts.iter().all(|part| !part.text.trim().is_empty()));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_rebalance_never_splits_a_crlf_pair() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 6,
            ..cfg()
        };
        let text = "zzzzzzabc\r\n!  ";

        let parts = split_book(text, &[], &config);

        assert!(parts.iter().all(|part| !part.text.ends_with('\r')));
        assert!(parts.iter().all(|part| !part.text.starts_with('\n')));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_does_not_split_a_closing_quote_at_the_window_edge() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 6,
            ..cfg()
        };
        let text = "ab hi!\" next";

        let parts = split_book(text, &[], &config);

        assert!(
            !parts
                .windows(2)
                .any(|pair| { pair[0].text.ends_with('!') && pair[1].text.starts_with('"') })
        );
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_rebalances_internal_pending_with_the_previous_part() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 4,
            ..cfg()
        };
        let text = "A.    BC";

        let parts = split_book(text, &[], &config);

        assert!(parts.iter().all(|part| part.text.chars().count() <= 4));
        assert!(parts.iter().all(|part| !part.text.trim().is_empty()));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_hard_cut_keeps_a_closing_quote_with_punctuation() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 7,
            ..cfg()
        };
        let text = "aaaaaa!\" next";

        let parts = split_book(text, &[], &config);

        assert!(
            !parts
                .windows(2)
                .any(|pair| { pair[0].text.ends_with('!') && pair[1].text.starts_with('"') })
        );
        assert!(parts.iter().all(|part| part.text.chars().count() <= 7));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_rebalance_keeps_a_closing_quote_with_punctuation() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 8,
            ..cfg()
        };
        let text = "aaaaaa!\"  ";

        let parts = split_book(text, &[], &config);

        assert!(
            !parts
                .windows(2)
                .any(|pair| { pair[0].text.ends_with('!') && pair[1].text.starts_with('"') })
        );
        assert!(parts.iter().all(|part| !part.text.trim().is_empty()));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_pending_window_keeps_a_closing_quote_with_punctuation() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 4,
            ..cfg()
        };
        let text = "    !\" next";

        let parts = split_book(text, &[], &config);

        assert!(
            !parts
                .windows(2)
                .any(|pair| { pair[0].text.ends_with('!') && pair[1].text.starts_with('"') })
        );
        assert!(parts.iter().all(|part| !part.text.trim().is_empty()));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_rebalance_handles_unicode_char_boundaries() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 7,
            ..cfg()
        };
        let text = "!.»B».».!B \" .!я\r\n\r\n»\"!\"!B!яa.\rя\rBa\n».!я»\n я .\r. я \"a.aB»\n B»\na\"a\n».».!.!\n\r";

        let parts = split_book(text, &[], &config);

        assert!(parts.iter().all(|part| part.text.chars().count() <= 7));
        assert!(parts.iter().all(|part| !part.text.trim().is_empty()));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_keeps_unicode_closing_quotes_with_punctuation() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 1,
            ..cfg()
        };

        for text in ["!”", "!’"] {
            let parts = split_book(text, &[], &config);

            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].text, text);
        }
    }

    #[test]
    fn size_cap_long_whitespace_run_completes_in_linear_time() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 1_000,
            ..cfg()
        };
        let text = format!("A{}B", " ".repeat(200_000));
        let started = std::time::Instant::now();

        let parts = split_book(&text, &[], &config);

        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(parts.len(), 2);
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
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
    fn size_cap_keeps_a_closing_quote_with_its_sentence() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 25,
            ..cfg()
        };
        let text = "Он сказал: «Все кончено!» Потом он ушёл.";

        let parts = split_book(text, &[], &config);

        assert_eq!(parts[0].text, "Он сказал: «Все кончено!»");
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
        );
    }

    #[test]
    fn size_cap_never_splits_a_crlf_pair_between_parts() {
        let config = SplitConfig {
            by_size: true,
            max_chars: 4,
            ..cfg()
        };
        let text = "абв\r\nвгд";

        let parts = split_book(text, &[], &config);

        assert!(parts.iter().all(|part| part.text.chars().count() <= 4));
        assert!(parts.iter().all(|part| !part.text.trim().is_empty()));
        assert!(parts.iter().all(|part| !part.text.ends_with('\r')));
        assert!(parts.iter().all(|part| !part.text.starts_with('\n')));
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            text
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
    fn part_number_uses_saturating_arithmetic() {
        let config = SplitConfig {
            template: "%Number%".to_string(),
            add_number: false,
            start_number: usize::MAX,
            ..cfg()
        };
        let part = BookPart {
            title: None,
            text: "Текст.".to_string(),
        };

        assert_eq!(
            part_file_name(&config, &part, 1, 2, "book"),
            usize::MAX.to_string()
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
    fn template_values_are_not_reinterpreted_as_variables() {
        let config = SplitConfig {
            template: "%FileName%".to_string(),
            add_number: false,
            ..cfg()
        };
        let part = BookPart {
            title: Some("Заголовок".to_string()),
            text: "Текст.".to_string(),
        };

        assert_eq!(part_file_name(&config, &part, 0, 1, "%Header%"), "%Header%");
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
    fn sanitize_prefixes_windows_reserved_names() {
        for (input, expected) in [
            ("NUL", "_NUL"),
            ("con", "_con"),
            ("COM1.txt", "_COM1.txt"),
            ("lpt9", "_lpt9"),
            ("aux.fb2", "_aux.fb2"),
        ] {
            assert_eq!(sanitize_file_name(input), expected, "{input}");
        }
    }

    #[test]
    fn sanitize_caps_length_at_120_chars() {
        let name = sanitize_file_name(&"я".repeat(300));
        assert_eq!(name.chars().count(), 120);
    }

    #[test]
    fn sanitize_caps_reserved_name_after_prefixing() {
        let name = sanitize_file_name(&format!("NUL.{}", "a".repeat(116)));

        assert!(name.starts_with("_NUL."));
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
            summary.targets,
            vec![
                dir.join("01 Первая часть.txt"),
                dir.join("02 Вторая часть.txt")
            ]
        );
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
    fn planned_names_include_same_run_collision_suffixes() {
        let dir = temp_dir("planned-collision");
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

        assert_eq!(
            planned_part_names(&parts, &config, "book"),
            vec!["Одинаково", "Одинаково (2)"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn planned_names_respect_existing_files_when_overwrite_is_disabled() {
        let dir = temp_dir("planned-existing");
        std::fs::write(dir.join("Текст.txt"), "старое").unwrap();
        let config = SplitConfig {
            output_dir: Some(dir.clone()),
            template: "%FirstLine%".to_string(),
            overwrite: false,
            ..cfg()
        };
        let parts = vec![BookPart {
            title: None,
            text: "Текст.".to_string(),
        }];

        assert_eq!(
            planned_part_names(&parts, &config, "book"),
            vec!["Текст (2)"]
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
    fn export_stops_between_files_when_cancelled() {
        use super::export::export_parts_to_with_cancel;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let dir = temp_dir("cancel-export");
        let config = SplitConfig {
            output_dir: Some(dir.clone()),
            template: "%Number%".to_string(),
            add_number: false,
            ..cfg()
        };
        let parts: Vec<BookPart> = (1..=1_000)
            .map(|index| BookPart {
                title: None,
                text: format!("Текст {index}."),
            })
            .collect();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::clone(&cancel);
        let first_target = dir.join("0001.txt");
        std::thread::spawn(move || {
            while !first_target.exists() {
                std::thread::yield_now();
            }
            cancel_flag.store(true, Ordering::Relaxed);
        });

        let summary = export_parts_to_with_cancel(
            &parts,
            &config,
            "book",
            &dir,
            &mut HashSet::new(),
            Some(cancel.as_ref()),
        )
        .expect("cancel returns partial summary");

        assert!(summary.saved < parts.len());
        assert!(summary.errors.iter().any(|error| error.is_cancelled()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_without_output_dir_errors() {
        let parts = vec![BookPart {
            title: None,
            text: "Текст.".to_string(),
        }];
        let error = export_parts(&parts, &cfg(), "book").expect_err("missing destination");
        let failure = expect_failure(&error, FailureKind::InvalidData);
        assert_eq!(failure.message, "Не выбрана папка назначения");
        assert_eq!(error.to_string(), failure.message);
    }

    #[test]
    fn export_blocked_dir_errors() {
        let dir = temp_dir("blocked");
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "file, not a dir").unwrap();
        let destination = blocker.join("out");
        let config = SplitConfig {
            output_dir: Some(destination.clone()),
            ..cfg()
        };
        let parts = vec![BookPart {
            title: None,
            text: "Текст.".to_string(),
        }];
        let error = export_parts(&parts, &config, "book").expect_err("blocked destination");
        let failure = expect_failure(&error, FailureKind::Io);
        let expected_prefix = format!("Не удалось создать папка {}: ", destination.display());
        assert!(
            failure.message.starts_with(&expected_prefix),
            "expected prefix {expected_prefix:?}, actual {:?}",
            failure.message
        );
        assert_eq!(error.to_string(), failure.message);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_write_errors_are_io_and_keep_partial_success() {
        let dir = temp_dir("partial-export");
        let blocked_target = dir.join("01 Текст.txt");
        std::fs::create_dir_all(&blocked_target).unwrap();
        let config = SplitConfig {
            output_dir: Some(dir.clone()),
            ..cfg()
        };
        let parts = vec![
            BookPart {
                title: None,
                text: "Текст.".to_string(),
            },
            BookPart {
                title: None,
                text: "Вторая часть.".to_string(),
            },
        ];

        let summary = export_parts(&parts, &config, "book").expect("partial export completes");

        assert_eq!(summary.saved, 1);
        assert_eq!(summary.errors.len(), 1);
        let failure = expect_failure(&summary.errors[0], FailureKind::Io);
        assert!(
            failure
                .message
                .starts_with(&format!("{}: ", blocked_target.display())),
            "{}",
            failure.message
        );
        assert_eq!(summary.errors[0].to_string(), failure.message);
        assert_eq!(
            std::fs::read_to_string(dir.join("02 Вторая часть.txt")).unwrap(),
            "Вторая часть."
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn split_batch_options(output_dir: PathBuf) -> SplitConfig {
        SplitConfig {
            by_keywords: true,
            keywords: "Глава".to_string(),
            template: "%FirstLine%".to_string(),
            add_number: false,
            output_dir: Some(output_dir),
            subfolder: true,
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
        let outcome = run_split_batch(
            &flat(&[book1, book2]),
            &CompositeBookReader::new(),
            &split_batch_options(out.clone()),
            None,
            Some(&tx),
        );
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 2,
                failed: 0,
            })
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
                .any(|m| matches!(m, BatchSplitProgress::FileFinished { result, .. } if result.saved == 1 && result.errors.is_empty()))
        );
        assert!(matches!(
            messages.last(),
            Some(BatchSplitProgress::Finished(finished)) if *finished == outcome
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_separates_same_stem_books_from_different_directories() {
        use crate::io::book_reader::CompositeBookReader;

        let root = temp_dir("split-same-stem");
        let first_dir = root.join("first");
        let second_dir = root.join("second");
        std::fs::create_dir_all(&first_dir).unwrap();
        std::fs::create_dir_all(&second_dir).unwrap();
        let first = write_book(&first_dir, "book.txt", "Глава 1\n\nПервый текст.");
        let second = write_book(&second_dir, "book.txt", "Глава 2\n\nВторой текст.");
        let out = root.join("out");
        let mut options = split_batch_options(out.clone());
        options.subfolder = true;

        let outcome = run_split_batch(
            &flat(&[first, second]),
            &CompositeBookReader::new(),
            &options,
            None,
            None,
        );

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 2,
                failed: 0,
            })
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book").join("Глава 1.txt")).unwrap(),
            "Глава 1\n\nПервый текст."
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book (2)").join("Глава 2.txt")).unwrap(),
            "Глава 2\n\nВторой текст."
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_respects_disabled_subfolder() {
        use crate::io::book_reader::CompositeBookReader;

        let root = temp_dir("split-flat");
        let book = write_book(&root, "book.txt", "Глава 1\n\nТекст.");
        let out = root.join("out");
        let mut options = split_batch_options(out.clone());
        options.subfolder = false;

        let outcome = run_split_batch(
            &flat(&[book]),
            &CompositeBookReader::new(),
            &options,
            None,
            None,
        );

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
        assert_eq!(
            std::fs::read_to_string(out.join("Глава 1.txt")).unwrap(),
            "Глава 1\n\nТекст."
        );
        assert!(!out.join("book").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_declined_large_export_writes_nothing() {
        use crate::io::book_reader::CompositeBookReader;

        let root = temp_dir("split-large-declined");
        let text = (1..=201)
            .map(|index| format!("Глава {index}\nТекст {index}."))
            .collect::<Vec<_>>()
            .join("\n");
        let book = write_book(&root, "book.txt", &text);
        let out = root.join("out");

        let outcome = run_split_batch_with_confirmation(
            &flat(&[book]),
            &CompositeBookReader::new(),
            &split_batch_options(out.clone()),
            None,
            None,
            Some(&|parts| {
                assert_eq!(parts, 201);
                false
            }),
        );

        assert_eq!(outcome, BatchOutcome::Cancelled(BatchCounts::default()));
        assert!(!out.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_pre_cancelled_processes_nothing() {
        use crate::io::book_reader::CompositeBookReader;
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc;

        let root = temp_dir("splitprecancel");
        let book = write_book(&root, "book.txt", "Глава 1\n\nТекст.");
        let out = root.join("out");
        let cancel = AtomicBool::new(true);

        let (tx, rx) = mpsc::channel();
        let outcome = run_split_batch(
            &flat(&[book]),
            &CompositeBookReader::new(),
            &split_batch_options(out.clone()),
            Some(&cancel),
            Some(&tx),
        );
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Cancelled(BatchCounts {
                succeeded: 0,
                failed: 0,
            })
        );

        let messages: Vec<BatchSplitProgress> = rx.iter().collect();
        assert!(
            messages
                .iter()
                .all(|message| !matches!(message, BatchSplitProgress::FileFinished { .. }))
        );
        assert!(matches!(
            messages.last(),
            Some(BatchSplitProgress::Finished(finished)) if *finished == outcome
        ));

        let output_is_empty = match std::fs::read_dir(&out) {
            Ok(mut entries) => entries.next().is_none(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => panic!("failed to inspect output directory: {error}"),
        };
        assert!(output_is_empty, "pre-cancelled batch created output");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_mid_run_cancellation_preserves_partial_counts() {
        use crate::domain::{AppResult, BookDocument};
        use crate::io::book_reader::CompositeBookReader;
        use crate::ports::DocumentReader;
        use std::path::Path;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::{Arc, mpsc};

        struct CancelAfterSecondRead {
            inner: CompositeBookReader,
            cancel: Arc<AtomicBool>,
            reads: AtomicUsize,
        }

        impl DocumentReader for CancelAfterSecondRead {
            fn can_read(&self, path: &Path) -> bool {
                self.inner.can_read(path)
            }

            fn read_document_with_cancel(
                &self,
                path: &Path,
                cancel: Option<&AtomicBool>,
            ) -> AppResult<BookDocument> {
                let document = self.inner.read_document_with_cancel(path, cancel)?;
                if self.reads.fetch_add(1, Ordering::Relaxed) == 1 {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                Ok(document)
            }
        }

        let root = temp_dir("splitmidcancel");
        let input = root.join("in");
        std::fs::create_dir_all(&input).expect("create input dir");
        let book1 = write_book(&input, "book1.txt", "Глава 1\n\nТекст первый.");
        let book2 = write_book(&input, "book2.txt", "Глава 2\n\nТекст второй.");
        let out = root.join("out");
        let cancel = Arc::new(AtomicBool::new(false));
        let reader = CancelAfterSecondRead {
            inner: CompositeBookReader::new(),
            cancel: cancel.clone(),
            reads: AtomicUsize::new(0),
        };

        let (tx, rx) = mpsc::channel();
        let outcome = run_split_batch(
            &flat(&[book1, book2]),
            &reader,
            &split_batch_options(out.clone()),
            Some(cancel.as_ref()),
            Some(&tx),
        );
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Cancelled(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
        assert!(out.join("book1").join("Глава 1.txt").exists());
        assert!(!out.join("book2").join("Глава 2.txt").exists());

        let messages: Vec<BatchSplitProgress> = rx.iter().collect();
        assert_eq!(
            messages
                .iter()
                .filter(|message| matches!(message, BatchSplitProgress::FileStarted { .. }))
                .count(),
            2
        );
        assert!(matches!(
            messages.last(),
            Some(BatchSplitProgress::Finished(finished)) if *finished == outcome
        ));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_preserves_typed_reader_cancellation() {
        use crate::domain::{AppError, AppResult, BookDocument};
        use crate::io::book_reader::CompositeBookReader;
        use crate::ports::DocumentReader;
        use std::path::Path;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc;

        struct CancelSpecificReader {
            inner: CompositeBookReader,
            path: PathBuf,
        }

        impl DocumentReader for CancelSpecificReader {
            fn can_read(&self, path: &Path) -> bool {
                self.inner.can_read(path)
            }

            fn read_document_with_cancel(
                &self,
                path: &Path,
                cancel: Option<&AtomicBool>,
            ) -> AppResult<BookDocument> {
                if path == self.path.as_path() {
                    return Err(AppError::cancelled("Чтение книги отменено"));
                }
                self.inner.read_document_with_cancel(path, cancel)
            }
        }

        let root = temp_dir("splittypedcancel");
        let input = root.join("in");
        std::fs::create_dir_all(&input).expect("create input dir");
        let book1 = write_book(&input, "book1.txt", "Глава 1\n\nТекст первый.");
        let book2 = write_book(&input, "book2.txt", "Глава 2\n\nТекст второй.");
        let book3 = write_book(&input, "book3.txt", "Глава 3\n\nТекст третий.");
        let out = root.join("out");
        let cancel = AtomicBool::new(false);
        let reader = CancelSpecificReader {
            inner: CompositeBookReader::new(),
            path: book2.clone(),
        };

        let (tx, rx) = mpsc::channel();
        let outcome = run_split_batch(
            &flat(&[book1, book2, book3]),
            &reader,
            &split_batch_options(out.clone()),
            Some(&cancel),
            Some(&tx),
        );
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Cancelled(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
        assert!(!cancel.load(Ordering::Relaxed));
        assert!(out.join("book1").join("Глава 1.txt").exists());
        assert!(!out.join("book2").join("Глава 2.txt").exists());
        assert!(!out.join("book3").join("Глава 3.txt").exists());

        let messages: Vec<BatchSplitProgress> = rx.iter().collect();
        assert!(messages.iter().any(|message| matches!(
            message,
            BatchSplitProgress::FileFinished { index: 0, result } if result.saved == 1 && result.errors.is_empty()
        )));
        assert!(messages.iter().any(|message| matches!(
            message,
            BatchSplitProgress::FileFinished { index: 1, result }
                if result.saved == 0 && result.errors.iter().any(|error| error.is_cancelled())
        )));
        assert!(
            !messages.iter().any(|message| matches!(
                message,
                BatchSplitProgress::FileFinished { index: 2.., .. }
            ))
        );
        assert!(matches!(
            messages.last(),
            Some(BatchSplitProgress::Finished(finished)) if *finished == outcome
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

        let outcome = run_split_batch(
            &flat(&[alpha, missing, beta]),
            &CompositeBookReader::new(),
            &split_batch_options(out.clone()),
            None,
            None,
        );

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 2,
                failed: 1,
            })
        );
        assert!(out.join("alpha").join("Глава 1.txt").exists());
        assert!(out.join("beta").join("Глава 2.txt").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_preserves_existing_targets_when_overwrite_is_disabled() {
        use crate::io::book_reader::CompositeBookReader;

        let root = temp_dir("split-no-overwrite");
        let book = write_book(&root, "book.txt", "Глава 1\n\nНовый текст.");
        let out = root.join("out");
        let existing = out.join("book").join("Глава 1.txt");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, "старый текст").unwrap();
        let mut options = split_batch_options(out.clone());
        options.overwrite = false;

        let outcome = run_split_batch(
            &flat(&[book]),
            &CompositeBookReader::new(),
            &options,
            None,
            None,
        );

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "старый текст");
        assert_eq!(
            std::fs::read_to_string(out.join("book").join("Глава 1 (2).txt")).unwrap(),
            "Глава 1\n\nНовый текст."
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_reports_partial_saved_count() {
        use crate::io::book_reader::CompositeBookReader;
        use std::sync::mpsc;

        let root = temp_dir("split-partial");
        let book = write_book(
            &root,
            "book.txt",
            "Глава 1\n\nПервый текст.\nГлава 2\n\nВторой текст.",
        );
        let out = root.join("out");
        std::fs::create_dir_all(out.join("book").join("Глава 1.txt")).unwrap();

        let (tx, rx) = mpsc::channel();
        let outcome = run_split_batch(
            &flat(&[book]),
            &CompositeBookReader::new(),
            &split_batch_options(out.clone()),
            None,
            Some(&tx),
        );
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 0,
                failed: 1,
            })
        );
        assert_eq!(
            std::fs::read_to_string(out.join("book").join("Глава 2.txt")).unwrap(),
            "Глава 2\n\nВторой текст."
        );
        let messages: Vec<BatchSplitProgress> = rx.iter().collect();
        assert!(messages.iter().any(|message| matches!(
            message,
            BatchSplitProgress::FileFinished { result, .. }
                if result.saved == 1 && !result.errors.is_empty()
        )));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn run_split_batch_overwrites_existing_targets_when_enabled() {
        use crate::io::book_reader::CompositeBookReader;
        use std::sync::mpsc;

        let root = temp_dir("splitblock");
        let input = root.join("in");
        std::fs::create_dir_all(&input).expect("create input dir");
        let book = write_book(&input, "book.txt", "Глава 1\n\nТекст.");
        let out = root.join("out");
        let existing = out.join("book").join("Глава 1.txt");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, "старое").unwrap();

        let (tx, rx) = mpsc::channel();
        let outcome = run_split_batch(
            &flat(&[book]),
            &CompositeBookReader::new(),
            &split_batch_options(out.clone()),
            None,
            Some(&tx),
        );
        drop(tx);

        assert_eq!(
            outcome,
            BatchOutcome::Completed(BatchCounts {
                succeeded: 1,
                failed: 0,
            })
        );
        assert_eq!(
            std::fs::read_to_string(&existing).unwrap(),
            "Глава 1\n\nТекст."
        );
        let messages: Vec<BatchSplitProgress> = rx.iter().collect();
        assert!(messages.iter().any(|message| matches!(
            message,
            BatchSplitProgress::FileFinished { index: 0, result }
                if result.saved == 1 && result.errors.is_empty()
        )));

        let _ = std::fs::remove_dir_all(&root);
    }
}

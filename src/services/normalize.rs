use crate::domain::{NormalizationStats, TextCleanupConfig};

pub fn normalize_text(text: &str, config: &TextCleanupConfig) -> (String, NormalizationStats) {
    let mut stats = NormalizationStats::default();
    if !config.remove_empty_lines && !config.add_line_end_periods {
        return (text.to_string(), stats);
    }

    let mut out = String::with_capacity(text.len());
    for segment in text.split_inclusive('\n') {
        let (content, ending) = split_line_ending(segment);
        if config.remove_empty_lines && content.trim().is_empty() {
            stats.removed_empty_lines += 1;
            continue;
        }
        if config.add_line_end_periods
            && let Some(punctuated) = punctuate_line(content)
        {
            stats.added_periods += 1;
            out.push_str(&punctuated);
        } else {
            out.push_str(content);
        }
        out.push_str(ending);
    }
    (out, stats)
}

fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(without_lf) = segment.strip_suffix('\n') {
        if let Some(without_crlf) = without_lf.strip_suffix('\r') {
            (without_crlf, "\r\n")
        } else {
            (without_lf, "\n")
        }
    } else {
        (segment, "")
    }
}

/// Returns the line with a trailing '.' when it lacks terminal punctuation.
/// Closing quotes/brackets are transparent: «Привет» -> «Привет». (Russian rule).
fn punctuate_line(content: &str) -> Option<String> {
    let trimmed = content.trim_end();
    let last = trimmed.chars().next_back()?;
    if is_terminal_punct(last) {
        return None;
    }
    if is_closing_mark(last) {
        let core = trimmed.trim_end_matches(is_closing_mark);
        match core.chars().next_back() {
            None => return None,
            Some(c) if is_terminal_punct(c) => return None,
            Some(_) => {}
        }
    }
    let mut punctuated = trimmed.to_string();
    punctuated.push('.');
    Some(punctuated)
}

fn is_terminal_punct(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…' | ':' | ';' | ',' | '—' | '-')
}

fn is_closing_mark(c: char) -> bool {
    matches!(c, '»' | '"' | '\'' | ')' | ']' | '”' | '’')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(remove_empty_lines: bool, add_line_end_periods: bool) -> TextCleanupConfig {
        TextCleanupConfig {
            remove_empty_lines,
            add_line_end_periods,
        }
    }

    #[test]
    fn heading_line_without_punctuation_gets_period() {
        let (out, stats) = normalize_text("Глава первая", &cfg(false, true));
        assert_eq!(out, "Глава первая.");
        assert_eq!(stats.added_periods, 1);
    }

    #[test]
    fn line_ending_with_digit_gets_period() {
        let (out, stats) = normalize_text("Глава 1\nОн пошёл.", &cfg(false, true));
        assert_eq!(out, "Глава 1.\nОн пошёл.");
        assert_eq!(stats.added_periods, 1);
    }

    #[test]
    fn lines_with_terminal_punctuation_are_untouched() {
        for line in [
            "Конец.",
            "Вопрос?",
            "Вот!",
            "Много…",
            "Двоеточие:",
            "Запятая,",
            "Точка с запятой;",
            "Тире —",
            "Дефис -",
            "Многоточие...",
        ] {
            let (out, stats) = normalize_text(line, &cfg(false, true));
            assert_eq!(out, line, "line changed: {line}");
            assert_eq!(stats.added_periods, 0, "period added to: {line}");
        }
    }

    #[test]
    fn period_is_added_after_closing_quotes() {
        let (out, stats) = normalize_text("«Привет»\n\"Здравствуйте\"", &cfg(false, true));
        assert_eq!(out, "«Привет».\n\"Здравствуйте\".");
        assert_eq!(stats.added_periods, 2);
    }

    #[test]
    fn closing_quotes_with_inner_punctuation_are_untouched() {
        for line in ["«Привет!»", "«Точно…»", "(Вот так!)", "«Да»."] {
            let (out, stats) = normalize_text(line, &cfg(false, true));
            assert_eq!(out, line, "line changed: {line}");
            assert_eq!(stats.added_periods, 0, "period added to: {line}");
        }
    }

    #[test]
    fn closing_bracket_without_inner_punctuation_gets_period() {
        let (out, stats) = normalize_text("(Смех за кадром)", &cfg(false, true));
        assert_eq!(out, "(Смех за кадром).");
        assert_eq!(stats.added_periods, 1);
    }

    #[test]
    fn line_of_only_closing_marks_gets_no_period() {
        let (out, stats) = normalize_text("»)", &cfg(false, true));
        assert_eq!(out, "»)");
        assert_eq!(stats.added_periods, 0);
    }

    #[test]
    fn empty_and_whitespace_lines_are_removed() {
        let (out, stats) = normalize_text("Абзац.\n\n   \n\t\nДругой.", &cfg(true, false));
        assert_eq!(out, "Абзац.\nДругой.");
        assert_eq!(stats.removed_empty_lines, 3);
    }

    #[test]
    fn crlf_line_endings_are_preserved() {
        let (out, stats) = normalize_text("Глава\r\n\r\nТекст\r\n", &cfg(true, true));
        assert_eq!(out, "Глава.\r\nТекст.\r\n");
        assert_eq!(stats.removed_empty_lines, 1);
        assert_eq!(stats.added_periods, 2);
    }

    #[test]
    fn trailing_spaces_are_trimmed_when_period_is_added() {
        let (out, _) = normalize_text("Глава первая   \nТекст", &cfg(false, true));
        assert_eq!(out, "Глава первая.\nТекст.");
    }

    #[test]
    fn whitespace_only_line_is_not_punctuated_when_removal_is_off() {
        let (out, stats) = normalize_text("   \nТекст", &cfg(false, true));
        assert_eq!(out, "   \nТекст.");
        assert_eq!(stats.added_periods, 1);
    }

    #[test]
    fn disabled_flags_leave_text_untouched() {
        let text = "Глава\n\n   \nТекст без точки";
        let (out, stats) = normalize_text(text, &cfg(false, false));
        assert_eq!(out, text);
        assert_eq!(stats, NormalizationStats::default());
    }

    #[test]
    fn empty_text_stays_empty() {
        let (out, stats) = normalize_text("", &cfg(true, true));
        assert_eq!(out, "");
        assert_eq!(stats, NormalizationStats::default());

        let (out, stats) = normalize_text("\n\n", &cfg(true, true));
        assert_eq!(out, "");
        assert_eq!(stats.removed_empty_lines, 2);
    }

    #[test]
    fn last_line_without_newline_is_processed() {
        let (out, stats) = normalize_text("Первая.\n\nВторая", &cfg(true, true));
        assert_eq!(out, "Первая.\nВторая.");
        assert_eq!(stats.removed_empty_lines, 1);
        assert_eq!(stats.added_periods, 1);
    }

    #[test]
    fn stress_plus_before_vowel_does_not_block_period() {
        let (out, _) = normalize_text("з+амок", &cfg(false, true));
        assert_eq!(out, "з+амок.");
    }
}

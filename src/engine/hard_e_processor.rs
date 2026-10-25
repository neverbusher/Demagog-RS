use rustc_hash::FxHashMap;

use crate::domain::{DictionaryKind, ProcessOutput, ReplacementEvent};
use crate::ports::TextProcessor;

pub struct HardEProcessor {
    name: String,
    exact: FxHashMap<String, String>,
    prefixes: FxHashMap<String, String>,
    min_prefix_chars: usize,
    max_prefix_chars: usize,
    rule_count: usize,
    compile_warnings: Vec<String>,
}

impl HardEProcessor {
    pub fn compile(name: String, content: &str) -> Self {
        let mut exact = FxHashMap::default();
        let mut prefixes = FxHashMap::default();
        let mut min_prefix_chars = usize::MAX;
        let mut max_prefix_chars = 0usize;
        let mut compile_warnings = Vec::new();

        for raw_line in content.lines() {
            let line = raw_line.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((raw_key, raw_value)) = line.split_once('=') else {
                continue;
            };
            let key = raw_key.trim();
            let value = raw_value.trim();
            if key.is_empty() || value.is_empty() {
                continue;
            }
            if key.split_whitespace().count() != 1 || value.split_whitespace().count() != 1 {
                push_warning(
                    &mut compile_warnings,
                    format!("{key}={value}: многословные правила не поддерживаются"),
                );
                continue;
            }
            if let Some(stem) = key.strip_suffix('*') {
                let Some(replacement) = value.strip_suffix('*') else {
                    push_warning(
                        &mut compile_warnings,
                        format!("{key}={value}: маска должна сохранять окончание"),
                    );
                    continue;
                };
                if stem.is_empty()
                    || stem.contains('*')
                    || replacement.contains('*')
                    || !stem.chars().any(char::is_alphabetic)
                    || !stem.chars().all(is_rule_char)
                    || !replacement.chars().all(is_rule_char)
                {
                    push_warning(
                        &mut compile_warnings,
                        format!("{key}={value}: плохая маска"),
                    );
                    continue;
                }
                if stem.chars().count() != replacement.chars().count() {
                    push_warning(
                        &mut compile_warnings,
                        format!("{key}={value}: замена меняет длину слова"),
                    );
                    continue;
                }
                let char_len = stem.chars().count();
                min_prefix_chars = min_prefix_chars.min(char_len);
                max_prefix_chars = max_prefix_chars.max(char_len);
                prefixes.insert(stem.to_lowercase(), replacement.to_lowercase());
                continue;
            }

            if key.contains('*')
                || !key.chars().any(char::is_alphabetic)
                || !key.chars().all(is_rule_char)
                || !value.chars().all(is_rule_char)
            {
                push_warning(
                    &mut compile_warnings,
                    format!("{key}={value}: плохое правило"),
                );
                continue;
            }
            if key.chars().count() != value.chars().count() {
                push_warning(
                    &mut compile_warnings,
                    format!("{key}={value}: замена меняет длину слова"),
                );
                continue;
            }
            exact.insert(key.to_lowercase(), value.to_lowercase());
        }

        let rule_count = exact.len() + prefixes.len();
        Self {
            name,
            exact,
            prefixes,
            min_prefix_chars,
            max_prefix_chars,
            rule_count,
            compile_warnings,
        }
    }

    pub fn compile_warnings(&self) -> &[String] {
        &self.compile_warnings
    }

    fn lookup(&self, word: &str) -> Option<(String, String)> {
        if let Some(replacement) = self.exact.get(word) {
            return Some((replacement.clone(), format!("{word}={replacement}")));
        }
        if self.prefixes.is_empty() {
            return None;
        }
        let char_indices: Vec<(usize, char)> = word.char_indices().collect();
        let total_chars = char_indices.len();
        if total_chars < self.min_prefix_chars {
            return None;
        }
        let hyphenated = word.contains('-');
        let max_chars = total_chars.min(self.max_prefix_chars);
        for char_len in (self.min_prefix_chars..=max_chars).rev() {
            let byte_end = if char_len == total_chars {
                word.len()
            } else {
                char_indices[char_len].0
            };
            let stem = &word[..byte_end];
            if hyphenated && !stem.contains('-') {
                continue;
            }
            if let Some(replacement) = self.prefixes.get(stem) {
                let suffix = &word[byte_end..];
                return Some((
                    format!("{replacement}{suffix}"),
                    format!("{stem}*={replacement}"),
                ));
            }
        }
        None
    }
}

impl TextProcessor for HardEProcessor {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> DictionaryKind {
        DictionaryKind::HardE
    }

    fn rule_count(&self) -> usize {
        self.rule_count
    }

    fn process(&self, text: &str, capture_diff: bool) -> ProcessOutput {
        let mut out = String::with_capacity(text.len());
        let mut replacements_count = 0usize;
        let mut events = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let c = chars[i];
            if !is_run_char(c) {
                out.push(c);
                i += 1;
                continue;
            }

            let start = i;
            while i < chars.len() && is_run_char(chars[i]) {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();

            if let Some((transformed, rule)) = self.transform_run(&run) {
                replacements_count += 1;
                push_event(
                    &mut events,
                    capture_diff,
                    &self.name,
                    rule,
                    &run,
                    &transformed,
                );
                out.push_str(&transformed);
                continue;
            }

            if !run.contains('-') {
                out.push_str(&run);
                continue;
            }

            let mut segment_start = 0;
            for (idx, ch) in run.char_indices() {
                if ch != '-' {
                    continue;
                }
                let segment = &run[segment_start..idx];
                if let Some((transformed, rule)) = self.transform_run(segment) {
                    replacements_count += 1;
                    push_event(
                        &mut events,
                        capture_diff,
                        &self.name,
                        rule,
                        segment,
                        &transformed,
                    );
                    out.push_str(&transformed);
                } else {
                    out.push_str(segment);
                }
                out.push('-');
                segment_start = idx + 1;
            }
            let tail = &run[segment_start..];
            if let Some((transformed, rule)) = self.transform_run(tail) {
                replacements_count += 1;
                push_event(
                    &mut events,
                    capture_diff,
                    &self.name,
                    rule,
                    tail,
                    &transformed,
                );
                out.push_str(&transformed);
            } else {
                out.push_str(tail);
            }
        }

        ProcessOutput {
            text: out,
            replacements_count,
            events,
        }
    }
}

impl HardEProcessor {
    fn transform_run(&self, run: &str) -> Option<(String, String)> {
        let bare: String = run.chars().filter(|&ch| ch != '+').collect();
        if bare.is_empty() {
            return None;
        }
        let (replacement, rule) = self.lookup(&bare.to_lowercase())?;
        if replacement.chars().count() != bare.chars().count() {
            return None;
        }

        let mut replacement_chars = replacement.chars();
        let mut transformed = String::with_capacity(run.len());
        for original in run.chars() {
            if original == '+' {
                transformed.push('+');
                continue;
            }
            let replacement_char = replacement_chars.next()?;
            if original.is_uppercase() {
                transformed.extend(replacement_char.to_uppercase());
            } else {
                transformed.push(replacement_char);
            }
        }
        if replacement_chars.next().is_some() || transformed == run {
            return None;
        }
        Some((transformed, rule))
    }
}

fn push_event(
    events: &mut Vec<ReplacementEvent>,
    capture_diff: bool,
    dictionary_name: &str,
    rule: String,
    before: &str,
    after: &str,
) {
    if capture_diff && events.len() < 200 {
        events.push(ReplacementEvent {
            dictionary_name: dictionary_name.to_string(),
            rule_summary: rule,
            before_snippet: before.to_string(),
            after_snippet: after.to_string(),
        });
    }
}

fn push_warning(warnings: &mut Vec<String>, message: String) {
    if warnings.len() < 50 {
        warnings.push(message);
    }
}

fn is_rule_char(c: char) -> bool {
    c.is_alphabetic() || c == '-'
}

fn is_run_char(c: char) -> bool {
    is_cyrillic_letter(c) || c == '+' || c == '-'
}

fn is_cyrillic_letter(c: char) -> bool {
    matches!(c, 'а'..='я' | 'ё' | 'А'..='Я' | 'Ё')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::TextProcessor;

    fn compile(content: &str) -> HardEProcessor {
        HardEProcessor::compile("test(hard-e).dic".to_string(), content)
    }

    #[test]
    fn replaces_exact_word() {
        let processor = compile("тест=тэст\n");
        let output = processor.process("тест", false);

        assert_eq!(output.text, "тэст");
        assert_eq!(output.replacements_count, 1);
    }

    #[test]
    fn keeps_stress_marks_in_place() {
        let processor = compile("тест=тэст\n");
        let output = processor.process("т+ест", false);

        assert_eq!(output.text, "т+эст");
    }

    #[test]
    fn keeps_letter_case() {
        let processor = compile("тест=тэст\n");
        let output = processor.process("Тест ТЕСТ", false);

        assert_eq!(output.text, "Тэст ТЭСТ");
    }

    #[test]
    fn applies_prefix_mask_and_keeps_ending() {
        let processor = compile("бизнес*=бизнэс*\n");
        let output = processor.process("бизнес-план", false);

        assert_eq!(output.text, "бизнэс-план");
    }

    #[test]
    fn applies_hyphenated_prefix_mask() {
        let processor = compile("альма-матер*=альма-матэр*\n");
        let output = processor.process("альма-матер", false);

        assert_eq!(output.text, "альма-матэр");
    }

    #[test]
    fn applies_exact_word_inside_hyphenated_compound() {
        let processor = compile("тест=тэст\n");
        let output = processor.process("интернет-тест", false);

        assert_eq!(output.text, "интернет-тэст");
    }

    #[test]
    fn applies_rules_to_each_part_of_hyphenated_word() {
        let processor = compile("интернет*=интэрнэт*\nтест=тэст\n");
        let output = processor.process("интерн+ет-т+ест", false);

        assert_eq!(output.text, "интэрн+эт-т+эст");
        assert_eq!(output.replacements_count, 2);
    }

    #[test]
    fn leaves_unknown_and_latin_words_untouched() {
        let processor = compile("тест=тэст\n");
        let output = processor.process("test тест", false);

        assert_eq!(output.text, "test тэст");
        assert_eq!(output.replacements_count, 1);
    }

    #[test]
    fn reports_captured_replacement() {
        let processor = compile("тест=тэст\n");
        let output = processor.process("т+ест", true);

        assert_eq!(output.events.len(), 1);
        assert_eq!(output.events[0].before_snippet, "т+ест");
        assert_eq!(output.events[0].after_snippet, "т+эст");
    }

    #[test]
    fn skips_multiword_rule_with_warning() {
        let processor = compile("а темпо=а тэмпо\n");

        assert_eq!(processor.rule_count(), 0);
        assert!(!processor.compile_warnings().is_empty());
        assert_eq!(processor.process("а темпо", false).text, "а темпо");
    }
}

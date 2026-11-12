use std::sync::atomic::AtomicBool;

use rayon::prelude::*;
use regex::{Regex, RegexBuilder};
use rustc_hash::FxHashSet;

use crate::domain::{DictionaryKind, ProcessOutput, ReplacementEvent};
use crate::engine::rex_normalizer::{normalize_rex_pattern, normalize_rex_replacement};
use crate::ports::TextProcessor;

#[derive(Debug, Clone)]
pub struct CompiledRexRule {
    pub regex: Regex,
    pub replacement: String,
    pub raw_rule: String,
    pub needs_digit: bool,
    pub required_sub: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IndexedRexBlock {
    pub keywords: Vec<String>,
    pub rules: Vec<CompiledRexRule>,
}

struct PendingRexRule {
    pattern: String,
    replacement: String,
    raw_rule: String,
    warning_line: String,
    case_sensitive: bool,
}

pub struct RexProcessor {
    name: String,
    kind: DictionaryKind,
    unindexed_rules: Vec<CompiledRexRule>,
    indexed_blocks: Vec<IndexedRexBlock>,
    rule_count: usize,
    collapse_spaces_at_end: bool,
    cleanup_yo_tags: bool,
    compile_warnings: Vec<String>,
}

impl RexProcessor {
    pub fn compile(name: String, content: &str, _encoding: &'static str) -> Self {
        let mut unindexed_pending = Vec::new();
        let mut blocks_pending: Vec<(Vec<String>, Vec<PendingRexRule>)> = Vec::new();
        let mut current_keywords: Option<Vec<String>> = None;
        let mut current_rules = Vec::new();
        let mut collapse_spaces_at_end = false;
        let cleanup_yo_tags = content.contains("<:yo:>") || content.contains("<:ye:>");

        for raw_line in content.lines() {
            let trimmed = raw_line.trim().trim_start_matches('\u{feff}');
            if trimmed.is_empty() || trimmed.starts_with("--") {
                continue;
            }

            if let Some(idx_header) = trimmed.strip_prefix("###") {
                let kws: Vec<String> = idx_header
                    .split(',')
                    .map(|k| k.trim().to_lowercase())
                    .filter(|k| !k.is_empty() && !k.starts_with('#') && !k.starts_with('-'))
                    .collect();

                if !kws.is_empty() {
                    if let Some(prev_kws) = current_keywords.take()
                        && !current_rules.is_empty()
                    {
                        blocks_pending.push((prev_kws, std::mem::take(&mut current_rules)));
                    }
                    current_keywords = Some(kws);
                }
                continue;
            }

            if trimmed.starts_with('#') {
                continue;
            }

            let Some((lhs, rhs)) = raw_line.split_once('=') else {
                continue;
            };

            if lhs.trim() == "[ ]+" && rhs == " " {
                collapse_spaces_at_end = true;
                continue;
            }

            let (pattern, case_sensitive) = normalize_rex_pattern(lhs);
            if pattern.is_empty() {
                continue;
            }

            let rule = PendingRexRule {
                pattern,
                replacement: normalize_rex_replacement(rhs),
                raw_rule: trimmed.chars().take(90).collect(),
                warning_line: trimmed.to_string(),
                case_sensitive,
            };
            if current_keywords.is_some() {
                current_rules.push(rule);
            } else {
                unindexed_pending.push(rule);
            }
        }

        if let Some(prev_kws) = current_keywords.take()
            && !current_rules.is_empty()
        {
            blocks_pending.push((prev_kws, current_rules));
        }

        let (mut unindexed_rules, mut compile_warnings) = compile_rex_rules(unindexed_pending);
        let compiled_blocks: Vec<(IndexedRexBlock, Vec<String>)> = blocks_pending
            .into_par_iter()
            .map(|(keywords, pending)| {
                let (rules, warnings) = compile_rex_rules(pending);
                (IndexedRexBlock { keywords, rules }, warnings)
            })
            .collect();

        let mut indexed_blocks = Vec::with_capacity(compiled_blocks.len());
        for (block, warnings) in compiled_blocks {
            indexed_blocks.push(block);
            for warning in warnings {
                if compile_warnings.len() < 50 {
                    compile_warnings.push(warning);
                }
            }
        }

        let is_indexed = indexed_blocks.len() > 5;
        if !is_indexed && !indexed_blocks.is_empty() {
            for b in indexed_blocks.drain(..) {
                unindexed_rules.extend(b.rules);
            }
        }

        let rule_count =
            unindexed_rules.len() + indexed_blocks.iter().map(|b| b.rules.len()).sum::<usize>();

        Self {
            name,
            kind: if is_indexed {
                DictionaryKind::RegexIndexed
            } else {
                DictionaryKind::RegexStandard
            },
            unindexed_rules,
            indexed_blocks,
            rule_count,
            collapse_spaces_at_end,
            cleanup_yo_tags,
            compile_warnings,
        }
    }

    pub fn compile_warnings(&self) -> &[String] {
        &self.compile_warnings
    }

    fn process_chunk(
        &self,
        chunk: &str,
        mut events: Option<&mut Vec<ReplacementEvent>>,
    ) -> (String, usize) {
        let mut words = FxHashSet::default();
        if !self.indexed_blocks.is_empty() {
            for w in chunk.split(|c: char| !c.is_alphabetic()) {
                if !w.is_empty() {
                    words.insert(w.to_lowercase());
                }
            }
        }

        let mut current = chunk.to_string();
        let mut current_lower = current.to_lowercase();
        let mut has_digit = current.bytes().any(|b| b.is_ascii_digit());
        let mut count = 0usize;

        for rule in &self.unindexed_rules {
            self.apply_rule(
                rule,
                &mut current,
                &mut current_lower,
                &mut has_digit,
                &mut count,
                &mut events,
            );
        }

        for block in &self.indexed_blocks {
            if block.keywords.iter().any(|kw| words.contains(kw)) {
                for rule in &block.rules {
                    self.apply_rule(
                        rule,
                        &mut current,
                        &mut current_lower,
                        &mut has_digit,
                        &mut count,
                        &mut events,
                    );
                }
            }
        }

        if self.collapse_spaces_at_end && current.contains("  ") {
            let mut collapsed = String::with_capacity(current.len());
            let mut prev_space = false;
            for ch in current.chars() {
                if ch == ' ' {
                    if !prev_space {
                        collapsed.push(ch);
                    }
                    prev_space = true;
                } else {
                    prev_space = false;
                    collapsed.push(ch);
                }
            }
            current = collapsed;
        }

        if self.cleanup_yo_tags {
            if current.contains("<:yo:>") {
                current = current.replace("<:yo:>", "ё");
            }
            if current.contains("<:ye:>") {
                current = current.replace("<:ye:>", "е");
            }
        }

        (current, count)
    }

    fn apply_rule(
        &self,
        rule: &CompiledRexRule,
        current: &mut String,
        current_lower: &mut String,
        has_digit: &mut bool,
        count: &mut usize,
        events: &mut Option<&mut Vec<ReplacementEvent>>,
    ) {
        if rule.needs_digit && !*has_digit {
            return;
        }
        if let Some(sub) = &rule.required_sub
            && !current_lower.contains(sub.as_str())
        {
            return;
        }
        if !rule.regex.is_match(current) {
            return;
        }

        let next = rule.regex.replace_all(current, rule.replacement.as_str());
        if next == *current {
            return;
        }

        if let Some(evs) = events.as_deref_mut()
            && evs.len() < 200
            && let Some(m) = rule.regex.find(current)
        {
            let before_snip = snippet_around(current, m.start(), m.end());
            let after_snip = snippet_around(
                next.as_ref(),
                m.start().min(next.len()),
                m.start().saturating_add(rule.replacement.len()),
            );
            evs.push(ReplacementEvent {
                dictionary_name: self.name.clone(),
                rule_summary: rule.raw_rule.clone(),
                before_snippet: before_snip,
                after_snippet: after_snip,
            });
        }

        *current = next.into_owned();
        *current_lower = current.to_lowercase();
        *has_digit = current.bytes().any(|b| b.is_ascii_digit());
        *count += 1;
    }
}

impl TextProcessor for RexProcessor {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> DictionaryKind {
        self.kind
    }

    fn rule_count(&self) -> usize {
        self.rule_count
    }

    fn process(&self, text: &str, capture_diff: bool) -> ProcessOutput {
        self.process_with_cancel(text, capture_diff, None)
    }

    fn process_with_cancel(
        &self,
        text: &str,
        capture_diff: bool,
        cancel: Option<&AtomicBool>,
    ) -> ProcessOutput {
        if is_cancelled(cancel) {
            return ProcessOutput {
                text: text.to_string(),
                replacements_count: 0,
                events: Vec::new(),
            };
        }
        if !capture_diff && text.len() > 1024 {
            let lines: Vec<&str> = text.split('\n').collect();
            let results: Vec<(String, usize)> = lines
                .par_iter()
                .map(|line| {
                    if is_cancelled(cancel) || line.trim().is_empty() {
                        (line.to_string(), 0)
                    } else {
                        self.process_chunk(line, None)
                    }
                })
                .collect();

            let mut out = String::with_capacity(text.len() + 256);
            let mut total_repl = 0usize;
            for (i, (line_out, c)) in results.into_iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(&line_out);
                total_repl += c;
            }

            return ProcessOutput {
                text: out,
                replacements_count: total_repl,
                events: Vec::new(),
            };
        }

        let mut out = String::with_capacity(text.len() + 256);
        let mut total_repl = 0usize;
        let mut events = Vec::new();

        for (i, line) in text.split('\n').enumerate() {
            if is_cancelled(cancel) {
                break;
            }
            if i > 0 {
                out.push('\n');
            }
            if line.trim().is_empty() {
                out.push_str(line);
                continue;
            }

            let line_events = if capture_diff {
                Some(&mut events)
            } else {
                None
            };
            let (line_out, c) = self.process_chunk(line, line_events);
            out.push_str(&line_out);
            total_repl += c;
        }

        ProcessOutput {
            text: out,
            replacements_count: total_repl,
            events,
        }
    }
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
}

fn compile_rex_rules(pending: Vec<PendingRexRule>) -> (Vec<CompiledRexRule>, Vec<String>) {
    let results: Vec<Result<CompiledRexRule, String>> = pending
        .into_par_iter()
        .map(|rule| {
            let compiled = RegexBuilder::new(&rule.pattern)
                .case_insensitive(!rule.case_sensitive)
                .unicode(true)
                .build();
            match compiled {
                Ok(regex) => {
                    let (needs_digit, required_sub) = analyze_pattern_prefilter(&rule.pattern);
                    Ok(CompiledRexRule {
                        regex,
                        replacement: rule.replacement,
                        raw_rule: rule.raw_rule,
                        needs_digit,
                        required_sub,
                    })
                }
                Err(err) => Err(format!("{} → {err}", rule.warning_line)),
            }
        })
        .collect();

    let mut rules = Vec::with_capacity(results.len());
    let mut warnings = Vec::new();
    for result in results {
        match result {
            Ok(rule) => rules.push(rule),
            Err(warning) => {
                if warnings.len() < 50 {
                    warnings.push(warning);
                }
            }
        }
    }
    (rules, warnings)
}

/// Extracts fast pre-filter conditions for a regex pattern:
/// 1) Whether it requires an ASCII digit (`\d` that every match must cover)
/// 2) Whether it requires a fixed substring like `"вс"`, `"<t"`, `"<:"`
fn analyze_pattern_prefilter(pat: &str) -> (bool, Option<String>) {
    // Alternations make both prefilters unsafe: the required literal or digit
    // may only belong to one branch.
    if pat.contains('|') {
        return (false, None);
    }

    let lower = pat.to_lowercase();
    let hints: [(&str, bool); 5] = [
        ("вс", lower.contains(r"\bвс") || lower.contains("(вс)")),
        ("<:ye:>", lower.contains("<:ye:>")),
        ("<:yo:>", lower.contains("<:yo:>")),
        ("<t", lower.contains("<t")),
        ("<num", lower.contains("<num")),
    ];
    for (literal, hinted) in hints {
        if hinted && literal_is_required(&lower, literal) {
            return (false, Some(literal.to_string()));
        }
    }
    if requires_digit(pat) {
        return (true, None);
    }
    (false, None)
}

/// Per-position structural facts: inside a character class, and how many
/// enclosing groups carry an optional quantifier (`?`, `*`, `{0..}`).
struct PatternStructure {
    in_class: Vec<bool>,
    optional_depth: Vec<usize>,
}

fn analyze_structure(pat: &str) -> PatternStructure {
    let chars: Vec<char> = pat.chars().collect();
    let n = chars.len();
    let mut in_class = vec![false; n];
    let mut optional_depth = vec![0usize; n];
    let mut group_stack: Vec<usize> = Vec::new();
    let mut optional_groups: Vec<(usize, usize)> = Vec::new();
    let mut class_start: Option<usize> = None;
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '\\' && i + 1 < n {
            i += 2;
            continue;
        }
        if let Some(start) = class_start {
            if c == ']' {
                for slot in in_class.iter_mut().take(i + 1).skip(start) {
                    *slot = true;
                }
                class_start = None;
            }
            i += 1;
            continue;
        }
        match c {
            '[' => class_start = Some(i),
            '(' => group_stack.push(i),
            ')' => {
                if let Some(start) = group_stack.pop()
                    && quantifier_makes_optional(&chars, i + 1)
                {
                    optional_groups.push((start, i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    for (start, end) in optional_groups {
        for slot in optional_depth.iter_mut().take(end + 1).skip(start) {
            *slot += 1;
        }
    }
    PatternStructure {
        in_class,
        optional_depth,
    }
}

fn quantifier_makes_optional(chars: &[char], after: usize) -> bool {
    matches!(chars.get(after), Some('*') | Some('?'))
        || (chars.get(after) == Some(&'{') && chars.get(after + 1) == Some(&'0'))
}

/// True when every match must contain the literal: at least one occurrence
/// sits outside classes and optional groups and its last character is not
/// quantified away. Alternations must be excluded by the caller.
fn literal_is_required(pat: &str, literal: &str) -> bool {
    let chars: Vec<char> = pat.chars().collect();
    let lit: Vec<char> = literal.chars().collect();
    if lit.is_empty() || chars.len() < lit.len() {
        return false;
    }
    let structure = analyze_structure(pat);
    for i in 0..=chars.len() - lit.len() {
        if chars[i..i + lit.len()] != lit[..] {
            continue;
        }
        let covered =
            (i..i + lit.len()).all(|p| !structure.in_class[p] && structure.optional_depth[p] == 0);
        if covered && !quantifier_makes_optional(&chars, i + lit.len()) {
            return true;
        }
    }
    false
}

/// True only when the pattern cannot match without at least one digit:
/// an unescaped `\d` outside character classes and optional groups,
/// without an optional quantifier.
fn requires_digit(pat: &str) -> bool {
    let chars: Vec<char> = pat.chars().collect();
    let structure = analyze_structure(pat);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            if chars[i + 1] == 'd'
                && !structure.in_class[i]
                && structure.optional_depth[i] == 0
                && !quantifier_makes_optional(&chars, i + 2)
            {
                return true;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    false
}

fn snippet_around(s: &str, start_byte: usize, end_byte: usize) -> String {
    let mut start = start_byte.min(s.len());
    while start > 0 && !s.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = end_byte.min(s.len()).max(start);
    while end < s.len() && !s.is_char_boundary(end) {
        end += 1;
    }

    let before = s[..start]
        .chars()
        .rev()
        .take(25)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    let matched = &s[start..end];
    let after: String = s[end..].chars().take(25).collect();
    format!("{before}[{matched}]{after}").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::TextProcessor;

    #[test]
    fn rex_processing_honors_cancel_between_lines() {
        let processor = RexProcessor::compile("t.rex".to_string(), "кот=к+от\n", "UTF-8");
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let text = "кот\n".repeat(5000);
        let out = processor.process_with_cancel(&text, false, Some(&cancel));
        assert_eq!(out.replacements_count, 0);
        assert_eq!(out.text, text);
    }

    #[test]
    fn optional_tag_literal_disables_substring_prefilter() {
        let processor = RexProcessor::compile("t.rex".to_string(), "<t?=X\n", "UTF-8");
        assert_eq!(processor.process("<", false).text, "X");
    }

    #[test]
    fn class_tag_literal_disables_substring_prefilter() {
        let processor = RexProcessor::compile("t.rex".to_string(), "[<t]=X\n", "UTF-8");
        assert_eq!(processor.process("t", false).text, "X");
    }

    #[test]
    fn optional_group_disables_digit_prefilter() {
        let processor = RexProcessor::compile("t.rex".to_string(), "(\\d+)?abc=X\n", "UTF-8");
        assert_eq!(processor.process("abc", false).text, "X");
    }

    #[test]
    fn optional_vs_group_disables_substring_prefilter() {
        let processor = RexProcessor::compile("t.rex".to_string(), "(вс)?е=X\n", "UTF-8");
        assert_eq!(processor.process("е", false).text, "X");
    }

    #[test]
    fn escaped_punctuation_in_rhs_is_literal() {
        let processor = RexProcessor::compile("t.rex".to_string(), "a=\\?\\!\n", "UTF-8");
        assert_eq!(processor.process("a", false).text, "?!");
    }

    #[test]
    fn snippet_handles_offsets_inside_multibyte_chars() {
        let snippet = snippet_around("я😀", 1, 3);
        assert!(snippet.contains('['));
    }

    #[test]
    fn alternation_disables_digit_prefilter() {
        assert_eq!(analyze_pattern_prefilter(r"\d+|один"), (false, None));
    }

    #[test]
    fn diff_and_fast_paths_produce_same_text() {
        let processor = RexProcessor::compile("test.rex".to_string(), "слово=СЛОВО\n", "UTF-8");
        let text = format!("{}\nСлово.", "Слово и слово.".repeat(100));
        assert!(text.len() > 1024);

        let fast = processor.process(&text, false);
        let diff = processor.process(&text, true);

        assert_eq!(fast.text, diff.text);
        assert_eq!(fast.replacements_count, diff.replacements_count);
    }

    #[test]
    fn negated_digit_class_still_runs_on_digit_free_text() {
        let processor = RexProcessor::compile("t.rex".to_string(), "[^\\d]=X\n", "UTF-8");
        let output = processor.process("абв", false);
        assert_eq!(output.text, "XXX");
    }

    #[test]
    fn plain_digit_class_is_skipped_on_digit_free_text() {
        let processor = RexProcessor::compile("t.rex".to_string(), "\\d=N\n", "UTF-8");
        assert_eq!(processor.process("абв", false).text, "абв");
        assert_eq!(processor.process("а1б", false).text, "аNб");
    }

    #[test]
    fn positive_digit_class_matches_without_digits() {
        let processor = RexProcessor::compile("t.rex".to_string(), "[a\\d]=X\n", "UTF-8");
        assert_eq!(processor.process("a", false).text, "X");
        assert_eq!(processor.process("1", false).text, "X");
    }

    #[test]
    fn letter_tag_rule_applies_without_digits() {
        let processor =
            RexProcessor::compile("t.rex".to_string(), "(<[a-z\\d]+>) млн=$1<mln>\n", "UTF-8");
        assert_eq!(processor.process("<usd> млн", false).text, "<usd><mln>");
        assert_eq!(processor.process("<t1> млн", false).text, "<t1><mln>");
    }

    #[test]
    fn parallel_compile_keeps_indexed_block_order() {
        let mut content = String::new();
        for i in 0..6 {
            content.push_str(&format!("###ключ{i}\nслово{i}=ЗАМЕНА{i}\n"));
        }

        let processor = RexProcessor::compile("test.rex".to_string(), &content, "UTF-8");
        assert_eq!(processor.kind(), DictionaryKind::RegexIndexed);
        assert_eq!(processor.indexed_blocks.len(), 6);
        for (i, block) in processor.indexed_blocks.iter().enumerate() {
            assert_eq!(block.keywords, vec![format!("ключ{i}")]);
            assert_eq!(block.rules.len(), 1);
            assert_eq!(block.rules[0].replacement, format!("ЗАМЕНА{i}"));
        }
    }
}

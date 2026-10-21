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
        let mut unindexed_rules = Vec::new();
        let mut indexed_blocks: Vec<IndexedRexBlock> = Vec::new();
        let mut current_keywords: Option<Vec<String>> = None;
        let mut current_block_rules: Vec<CompiledRexRule> = Vec::new();
        let mut collapse_spaces_at_end = false;
        let cleanup_yo_tags = content.contains("<:yo:>") || content.contains("<:ye:>");
        let mut compile_warnings = Vec::new();

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
                    if let Some(prev_kws) = current_keywords.take() {
                        if !current_block_rules.is_empty() {
                            indexed_blocks.push(IndexedRexBlock {
                                keywords: prev_kws,
                                rules: std::mem::take(&mut current_block_rules),
                            });
                        }
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

            let (pat_str, case_sensitive) = normalize_rex_pattern(lhs);
            if pat_str.is_empty() {
                continue;
            }

            let compiled = RegexBuilder::new(&pat_str)
                .case_insensitive(!case_sensitive)
                .unicode(true)
                .build();

            if let Err(err) = &compiled {
                if compile_warnings.len() < 50 {
                    compile_warnings.push(format!("{} → {err}", trimmed));
                }
            }

            if let Ok(regex) = compiled {
                let (needs_digit, required_sub) = analyze_pattern_prefilter(&pat_str);
                let rule = CompiledRexRule {
                    regex,
                    replacement: normalize_rex_replacement(rhs),
                    raw_rule: trimmed.chars().take(90).collect(),
                    needs_digit,
                    required_sub,
                };
                if current_keywords.is_some() {
                    current_block_rules.push(rule);
                } else {
                    unindexed_rules.push(rule);
                }
            }
        }

        if let Some(prev_kws) = current_keywords.take() {
            if !current_block_rules.is_empty() {
                indexed_blocks.push(IndexedRexBlock {
                    keywords: prev_kws,
                    rules: current_block_rules,
                });
            }
        }

        let is_indexed = indexed_blocks.len() > 5;
        if !is_indexed && !indexed_blocks.is_empty() {
            for b in indexed_blocks.drain(..) {
                unindexed_rules.extend(b.rules);
            }
        }

        let rule_count = unindexed_rules.len()
            + indexed_blocks.iter().map(|b| b.rules.len()).sum::<usize>();

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
        if let Some(sub) = &rule.required_sub {
            if !current_lower.contains(sub.as_str()) {
                return;
            }
        }
        if !rule.regex.is_match(current) {
            return;
        }

        let next = rule.regex.replace_all(current, rule.replacement.as_str());
        if next == *current {
            return;
        }

        if let Some(evs) = events.as_deref_mut() {
            if evs.len() < 200 {
                if let Some(m) = rule.regex.find(current) {
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
            }
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
        if !capture_diff && text.len() > 1024 {
            let lines: Vec<&str> = text.split('\n').collect();
            let results: Vec<(String, usize)> = lines
                .par_iter()
                .map(|line| {
                    if line.trim().is_empty() {
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

/// Extracts fast pre-filter conditions for a regex pattern:
/// 1) Whether it requires an ASCII digit (`\d` without `|` at top level)
/// 2) Whether it requires a fixed substring like `"вс"`, `"<t"`, `"<:"`
fn analyze_pattern_prefilter(pat: &str) -> (bool, Option<String>) {
    // Alternations make both prefilters unsafe: the required literal or digit
    // may only belong to one branch.
    if pat.contains('|') {
        return (false, None);
    }

    let lower = pat.to_lowercase();
    if lower.contains(r"\bвс") || lower.contains("(вс)") {
        return (false, Some("вс".to_string()));
    }
    if lower.contains("<:ye:>") || lower.contains(r"<\:ye\:>") {
        return (false, Some("<:ye:>".to_string()));
    }
    if lower.contains("<:yo:>") || lower.contains(r"<\:yo\:>") {
        return (false, Some("<:yo:>".to_string()));
    }
    if lower.contains("<t") {
        return (false, Some("<t".to_string()));
    }
    if lower.contains("<num") {
        return (false, Some("<num".to_string()));
    }
    if pat.contains(r"\d")
        && !pat.contains(r"\d*")
        && !pat.contains(r"\d?")
        && !pat.contains(r"\d{0")
    {
        return (true, None);
    }
    (false, None)
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
        let processor =
            RexProcessor::compile("test.rex".to_string(), "слово=СЛОВО\n", "UTF-8");
        let text = format!("{}\nСлово.", "Слово и слово.".repeat(100));
        assert!(text.len() > 1024);

        let fast = processor.process(&text, false);
        let diff = processor.process(&text, true);

        assert_eq!(fast.text, diff.text);
        assert_eq!(fast.replacements_count, diff.replacements_count);
    }
}

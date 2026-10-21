use rayon::prelude::*;
use regex::RegexBuilder;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::domain::{DictionaryKind, ProcessOutput, ReplacementEvent};
use crate::engine::rex_processor::CompiledRexRule;
use crate::ports::TextProcessor;

#[derive(Debug, Clone)]
struct ExactPhraseRule {
    lhs: String,
    lhs_lower: String,
    rhs: String,
    case_sensitive: bool,
    word_boundaries_start: bool,
    word_boundaries_end: bool,
    required_words: Vec<String>,
}

#[derive(Debug, Clone)]
struct AnchoredWildcardRule {
    required_words: Vec<String>,
    compiled: CompiledRexRule,
}

#[derive(Debug, Clone)]
struct SubstringWildcardRule {
    required_sub: String,
    compiled: CompiledRexRule,
}

pub struct DicProcessor {
    name: String,
    kind: DictionaryKind,
    rule_count: usize,
    is_stress_converter: bool,
    exact_cs: FxHashMap<String, String>,
    exact_ci: FxHashMap<String, String>,
    prefix_cs: FxHashMap<String, String>,
    prefix_ci: FxHashMap<String, String>,
    min_prefix_chars: usize,
    max_prefix_chars: usize,
    phrase_index: FxHashMap<String, Vec<ExactPhraseRule>>,
    unanchored_phrases: Vec<ExactPhraseRule>,
    wildcard_index: FxHashMap<String, Vec<AnchoredWildcardRule>>,
    unanchored_wildcards: Vec<SubstringWildcardRule>,
}

impl DicProcessor {
    pub fn compile(name: String, content: &str, _encoding: &'static str) -> Self {
        let is_stress_converter = name.contains("69_TTS_Offline+УДАРЕНИЯ");
        if is_stress_converter {
            let rule_count = content
                .lines()
                .filter(|l| {
                    let t = l.trim().trim_start_matches('\u{feff}');
                    !t.is_empty() && !t.starts_with('#') && t.contains('=')
                })
                .count();
            return Self {
                name,
                kind: DictionaryKind::DicStressConverter,
                rule_count,
                is_stress_converter: true,
                exact_cs: FxHashMap::default(),
                exact_ci: FxHashMap::default(),
                prefix_cs: FxHashMap::default(),
                prefix_ci: FxHashMap::default(),
                min_prefix_chars: usize::MAX,
                max_prefix_chars: 0,
                phrase_index: FxHashMap::default(),
                unanchored_phrases: Vec::new(),
                wildcard_index: FxHashMap::default(),
                unanchored_wildcards: Vec::new(),
            };
        }

        let mut exact_cs = FxHashMap::default();
        let mut exact_ci = FxHashMap::default();
        let mut prefix_cs = FxHashMap::default();
        let mut prefix_ci = FxHashMap::default();
        let mut min_prefix_chars = usize::MAX;
        let mut max_prefix_chars = 0usize;
        let mut phrase_index: FxHashMap<String, Vec<ExactPhraseRule>> = FxHashMap::default();
        let mut unanchored_phrases = Vec::new();
        let mut wildcard_index: FxHashMap<String, Vec<AnchoredWildcardRule>> = FxHashMap::default();
        let mut unanchored_wildcards = Vec::new();
        let mut rule_count = 0usize;

        for raw_line in content.lines() {
            let line = raw_line.trim_start_matches('\u{feff}');
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('#') && !trimmed.starts_with("##") {
                continue;
            }

            let Some((raw_lhs, raw_rhs)) = split_dic_rule(line) else {
                continue;
            };

            let (lhs, case_sensitive) = if let Some(rest) = raw_lhs.strip_prefix("$$") {
                (format!("${rest}"), false)
            } else if let Some(rest) = raw_lhs.strip_prefix("##") {
                (format!("#{rest}"), false)
            } else if let Some(rest) = raw_lhs.strip_prefix('$') {
                (rest.to_string(), true)
            } else {
                (raw_lhs.to_string(), false)
            };

            let lhs = lhs.trim();
            if lhs.is_empty() {
                continue;
            }

            let mut rhs = if !raw_lhs.ends_with(' ') && raw_rhs.ends_with(' ') && raw_rhs.trim().len() > 1 {
                raw_rhs.trim_end().to_string()
            } else {
                raw_rhs.to_string()
            };

            // Normalize case-insensitive rule where author capitalized first letter on both sides (e.g. `Ещё=Ещ+о`)
            if !case_sensitive {
                rhs = normalize_ci_rhs_initial_case(lhs, &rhs);
            }

            rule_count += 1;

            if !lhs.contains('*') {
                if is_single_word_token(lhs) {
                    if case_sensitive {
                        exact_cs.insert(lhs.to_string(), rhs);
                    } else {
                        exact_ci.insert(lhs.to_lowercase(), rhs);
                    }
                } else {
                    let words: Vec<String> = extract_alpha_words(lhs);
                    let first_char_alpha = lhs.chars().next().is_some_and(|c| c.is_alphabetic());
                    let last_char_alpha = lhs.chars().last().is_some_and(|c| c.is_alphabetic());
                    let rule = ExactPhraseRule {
                        lhs: lhs.to_string(),
                        lhs_lower: lhs.to_lowercase(),
                        rhs,
                        case_sensitive,
                        word_boundaries_start: first_char_alpha,
                        word_boundaries_end: last_char_alpha,
                        required_words: words.clone(),
                    };
                    if let Some(first_word) = words.first() {
                        phrase_index.entry(first_word.clone()).or_default().push(rule);
                    } else {
                        unanchored_phrases.push(rule);
                    }
                }
            } else {
                if let Some(stem) = lhs.strip_suffix('*') {
                    if !stem.is_empty() && !stem.contains('*') && is_single_word_token(stem) && !rhs.contains(' ') {
                        let char_len = stem.chars().count();
                        if char_len >= 2 {
                            min_prefix_chars = min_prefix_chars.min(char_len);
                            max_prefix_chars = max_prefix_chars.max(char_len);
                            if case_sensitive {
                                prefix_cs.insert(stem.to_string(), rhs);
                            } else {
                                prefix_ci.insert(stem.to_lowercase(), rhs);
                            }
                            continue;
                        }
                    }
                }

                if let Some((compiled, required_words, sub_filter)) =
                    compile_dic_wildcard_rule(lhs, &rhs, case_sensitive)
                {
                    if let Some(anchor) = required_words.iter().max_by_key(|w| w.len()).cloned() {
                        wildcard_index
                            .entry(anchor)
                            .or_default()
                            .push(AnchoredWildcardRule {
                                required_words,
                                compiled,
                            });
                    } else {
                        unanchored_wildcards.push(SubstringWildcardRule {
                            required_sub: sub_filter,
                            compiled,
                        });
                    }
                }
            }
        }

        for list in phrase_index.values_mut() {
            list.sort_by(|a, b| b.lhs.len().cmp(&a.lhs.len()));
        }
        unanchored_phrases.sort_by(|a, b| b.lhs.len().cmp(&a.lhs.len()));

        Self {
            name,
            kind: DictionaryKind::DicStandard,
            rule_count,
            is_stress_converter: false,
            exact_cs,
            exact_ci,
            prefix_cs,
            prefix_ci,
            min_prefix_chars,
            max_prefix_chars,
            phrase_index,
            unanchored_phrases,
            wildcard_index,
            unanchored_wildcards,
        }
    }

    fn process_stress_converter(&self, text: &str, capture_diff: bool) -> ProcessOutput {
        let mut out = String::with_capacity(text.len() + 256);
        let mut replacements_count = 0usize;
        let mut events = Vec::new();

        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if i + 1 < chars.len() && is_lower_cyrillic(c) && is_upper_cyrillic_vowel(chars[i + 1]) {
                let lower_v = chars[i + 1].to_lowercase().next().unwrap_or(chars[i + 1]);
                out.push(c);
                out.push('+');
                out.push(lower_v);
                replacements_count += 1;

                if capture_diff && events.len() < 150 {
                    let w_start = (0..=i)
                        .rev()
                        .take_while(|&idx| chars[idx].is_alphabetic() || chars[idx] == '+')
                        .last()
                        .unwrap_or(i);
                    let w_end = (i + 2..chars.len())
                        .take_while(|&idx| chars[idx].is_alphabetic() || chars[idx] == '+')
                        .last()
                        .map(|idx| idx + 1)
                        .unwrap_or(i + 2);
                    let before_w: String = chars[w_start..w_end].iter().collect();
                    let after_w = format!(
                        "{}{}+{}{}",
                        chars[w_start..i].iter().collect::<String>(),
                        c,
                        lower_v,
                        chars[i + 2..w_end].iter().collect::<String>()
                    );
                    events.push(ReplacementEvent {
                        dictionary_name: self.name.clone(),
                        rule_summary: format!("$*{}{}*={}+{}", c, chars[i + 1], c, lower_v),
                        before_snippet: before_w,
                        after_snippet: after_w,
                    });
                }

                i += 2;
            } else {
                out.push(c);
                i += 1;
            }
        }

        ProcessOutput {
            text: out,
            replacements_count,
            events,
        }
    }

    fn process_paragraph(
        &self,
        paragraph: &str,
        mut events: Option<&mut Vec<ReplacementEvent>>,
    ) -> (String, usize) {
        let mut current = paragraph.to_string();
        let mut count = 0usize;

        for rule in &self.unanchored_phrases {
            if current.contains(&rule.lhs) {
                let next = current.replace(&rule.lhs, &rule.rhs);
                if next != current {
                    count += 1;
                    if let Some(ev) = events.as_deref_mut() {
                        if ev.len() < 200 {
                            ev.push(ReplacementEvent {
                                dictionary_name: self.name.clone(),
                                rule_summary: format!("{}={}", rule.lhs, rule.rhs),
                                before_snippet: rule.lhs.clone(),
                                after_snippet: rule.rhs.clone(),
                            });
                        }
                    }
                    current = next;
                }
            }
        }

        if !self.phrase_index.is_empty() || !self.wildcard_index.is_empty() {
            let mut word_set = FxHashSet::default();
            let mut word_list = Vec::new();
            for w in current.split(|c: char| !c.is_alphabetic() && c != '+') {
                if !w.is_empty() {
                    let clean_w = if w.contains('+') {
                        w.replace('+', "").to_lowercase()
                    } else {
                        w.to_lowercase()
                    };
                    if word_set.insert(clean_w.clone()) {
                        word_list.push(clean_w);
                    }
                }
            }

            if !self.phrase_index.is_empty() {
                let mut current_lower = String::new();
                for w in &word_list {
                    if let Some(rules) = self.phrase_index.get(w) {
                        for rule in rules {
                            if rule.required_words.iter().all(|rw| word_set.contains(rw)) {
                                if current_lower.is_empty() {
                                    current_lower = current.to_lowercase();
                                }
                                if current_lower.contains(&rule.lhs_lower) {
                                    if let Some(replaced) = apply_exact_phrase(&current, rule) {
                                        count += 1;
                                        if let Some(ev) = events.as_deref_mut() {
                                            if ev.len() < 200 {
                                                ev.push(ReplacementEvent {
                                                    dictionary_name: self.name.clone(),
                                                    rule_summary: format!("{}={}", rule.lhs, rule.rhs),
                                                    before_snippet: rule.lhs.clone(),
                                                    after_snippet: rule.rhs.clone(),
                                                });
                                            }
                                        }
                                        current = replaced;
                                        current_lower = current.to_lowercase();
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !self.wildcard_index.is_empty() {
                for w in &word_list {
                    if let Some(w_rules) = self.wildcard_index.get(w) {
                        for ar in w_rules {
                            if ar.required_words.iter().all(|rw| word_set.contains(rw)) {
                                if ar.compiled.regex.is_match(&current) {
                                    let next = ar
                                        .compiled
                                        .regex
                                        .replace_all(&current, ar.compiled.replacement.as_str());
                                    if next != current {
                                        count += 1;
                                        if let Some(ev) = events.as_deref_mut() {
                                            if ev.len() < 200 {
                                                ev.push(ReplacementEvent {
                                                    dictionary_name: self.name.clone(),
                                                    rule_summary: ar.compiled.raw_rule.clone(),
                                                    before_snippet: ar.compiled.raw_rule.clone(),
                                                    after_snippet: ar.compiled.replacement.clone(),
                                                });
                                            }
                                        }
                                        current = next.into_owned();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if !self.unanchored_wildcards.is_empty() {
            let mut current_lower = current.to_lowercase();
            for sw in &self.unanchored_wildcards {
                if !sw.required_sub.is_empty() && !current_lower.contains(&sw.required_sub) {
                    continue;
                }
                if sw.compiled.regex.is_match(&current) {
                    let next = sw
                        .compiled
                        .regex
                        .replace_all(&current, sw.compiled.replacement.as_str());
                    if next != current {
                        count += 1;
                        if let Some(ev) = events.as_deref_mut() {
                            if ev.len() < 200 {
                                ev.push(ReplacementEvent {
                                    dictionary_name: self.name.clone(),
                                    rule_summary: sw.compiled.raw_rule.clone(),
                                    before_snippet: sw.compiled.raw_rule.clone(),
                                    after_snippet: sw.compiled.replacement.clone(),
                                });
                            }
                        }
                        current = next.into_owned();
                        current_lower = current.to_lowercase();
                    }
                }
            }
        }

        if !self.exact_cs.is_empty()
            || !self.exact_ci.is_empty()
            || !self.prefix_cs.is_empty()
            || !self.prefix_ci.is_empty()
        {
            let mut out = String::with_capacity(current.len() + 64);
            let mut idx = 0;
            let bytes = current.as_bytes();

            while idx < bytes.len() {
                let ch = current[idx..].chars().next().unwrap();
                if is_word_token_char(ch) {
                    let start = idx;
                    idx += ch.len_utf8();
                    while idx < bytes.len() {
                        let nc = current[idx..].chars().next().unwrap();
                        if is_word_token_char(nc) {
                            idx += nc.len_utf8();
                        } else {
                            break;
                        }
                    }
                    let word = &current[start..idx];
                    if let Some((repl, rule_repr)) = self.lookup_single_word(word) {
                        if repl != word {
                            count += 1;
                            if let Some(ev) = events.as_deref_mut() {
                                if ev.len() < 200 {
                                    ev.push(ReplacementEvent {
                                        dictionary_name: self.name.clone(),
                                        rule_summary: rule_repr,
                                        before_snippet: word.to_string(),
                                        after_snippet: repl.clone(),
                                    });
                                }
                            }
                            out.push_str(&repl);
                        } else {
                            out.push_str(word);
                        }
                    } else {
                        out.push_str(word);
                    }
                } else {
                    out.push(ch);
                    idx += ch.len_utf8();
                }
            }
            current = out;
        }

        (current, count)
    }

    #[inline]
    fn lookup_single_word(&self, word: &str) -> Option<(String, String)> {
        let already_stressed = word.contains('+');

        if let Some(rhs) = self.exact_cs.get(word) {
            return Some((rhs.clone(), format!("${word}={rhs}")));
        }

        let lower = word.to_lowercase();
        if !already_stressed {
            if let Some(rhs) = self.exact_ci.get(&lower) {
                let adapted = preserve_initial_capital(word, rhs);
                return Some((adapted, format!("{lower}={rhs}")));
            }
        }

        if (!self.prefix_cs.is_empty() || !self.prefix_ci.is_empty()) && !already_stressed {
            let char_indices: Vec<(usize, char)> = word.char_indices().collect();
            let total_chars = char_indices.len();
            if total_chars >= self.min_prefix_chars {
                let max_c = total_chars.min(self.max_prefix_chars);
                for c_len in (self.min_prefix_chars..=max_c).rev() {
                    let byte_end = if c_len == total_chars {
                        word.len()
                    } else {
                        char_indices[c_len].0
                    };
                    let stem = &word[..byte_end];
                    let suffix = &word[byte_end..];

                    if let Some(rhs_stem) = self.prefix_cs.get(stem) {
                        let full = format!("{rhs_stem}{suffix}");
                        return Some((full, format!("${stem}*={rhs_stem}")));
                    }
                    let stem_lower = stem.to_lowercase();
                    if let Some(rhs_stem) = self.prefix_ci.get(&stem_lower) {
                        let adapted_stem = preserve_initial_capital(stem, rhs_stem);
                        let full = format!("{adapted_stem}{suffix}");
                        return Some((full, format!("{stem_lower}*={rhs_stem}")));
                    }
                }
            }
        }

        None
    }
}

impl TextProcessor for DicProcessor {
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
        if self.is_stress_converter {
            return self.process_stress_converter(text, capture_diff);
        }

        if !capture_diff && text.len() > 2048 {
            let lines: Vec<&str> = text.split('\n').collect();
            let results: Vec<(String, usize)> = lines
                .par_iter()
                .map(|line| {
                    if line.trim().is_empty() {
                        (line.to_string(), 0)
                    } else {
                        self.process_paragraph(line, None)
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

        let mut events = Vec::new();
        let (out, count) = self.process_paragraph(
            text,
            if capture_diff { Some(&mut events) } else { None },
        );
        ProcessOutput {
            text: out,
            replacements_count: count,
            events,
        }
    }
}

fn split_dic_rule(line: &str) -> Option<(&str, &str)> {
    if let Some(pos) = line.find('\x06') {
        Some((&line[..pos], &line[pos + 1..]))
    } else if let Some(pos) = line.find("::") {
        Some((&line[..pos], &line[pos + 2..]))
    } else {
        line.split_once('=')
    }
}

#[inline]
fn is_word_token_char(c: char) -> bool {
    c.is_alphabetic() || c == '+' || c == '\''
}

#[inline]
fn is_single_word_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphabetic() || c == '+')
}

#[inline]
fn is_lower_cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'ё')
}

#[inline]
fn is_upper_cyrillic_vowel(c: char) -> bool {
    matches!(c, 'А' | 'Е' | 'Ё' | 'И' | 'О' | 'У' | 'Ы' | 'Э' | 'Ю' | 'Я')
}

fn extract_alpha_words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphabetic())
        .filter(|w| w.chars().count() >= 2)
        .map(|w| w.to_lowercase())
        .collect()
}

/// When a case-insensitive rule has both LHS and RHS starting with the same uppercase letter
/// AND RHS already contains `+` (e.g. `Ещё=Ещ+о` in 67_TTS_Offline), lowercase the initial letter
/// of RHS so lowercase text (`ещё`) stays lowercase (`ещ+о`).
fn normalize_ci_rhs_initial_case(lhs: &str, rhs: &str) -> String {
    let Some(first_l) = lhs.chars().next() else {
        return rhs.to_string();
    };
    let mut rhs_chars = rhs.chars();
    let Some(first_r) = rhs_chars.next() else {
        return rhs.to_string();
    };
    if first_l.is_uppercase() && first_r.is_uppercase() && rhs.contains('+') {
        let lower: String = first_r.to_lowercase().collect();
        let rest: String = rhs_chars.collect();
        return format!("{lower}{rest}");
    }
    rhs.to_string()
}

fn preserve_initial_capital(original: &str, rhs: &str) -> String {
    let mut orig_chars = original.chars();
    let Some(first_orig) = orig_chars.next() else {
        return rhs.to_string();
    };
    if !first_orig.is_uppercase() {
        return rhs.to_string();
    }
    let rhs_has_internal_upper = rhs
        .chars()
        .skip(1)
        .any(|c| c.is_uppercase() && !rhs.starts_with('+'));
    if rhs_has_internal_upper {
        return rhs.to_string();
    }

    let mut rhs_chars = rhs.chars();
    let Some(first_rhs) = rhs_chars.next() else {
        return rhs.to_string();
    };
    if first_rhs == '+' {
        if let Some(second_rhs) = rhs_chars.next() {
            let upper: String = second_rhs.to_uppercase().collect();
            let rest: String = rhs_chars.collect();
            return format!("+{upper}{rest}");
        }
        return rhs.to_string();
    }
    let upper: String = first_rhs.to_uppercase().collect();
    let rest: String = rhs_chars.collect();
    format!("{upper}{rest}")
}

fn apply_exact_phrase(text: &str, rule: &ExactPhraseRule) -> Option<String> {
    let haystack = if rule.case_sensitive {
        text.to_string()
    } else {
        text.to_lowercase()
    };
    let needle = if rule.case_sensitive {
        &rule.lhs
    } else {
        &rule.lhs_lower
    };

    if needle.is_empty() || (haystack.len() != text.len() && !rule.case_sensitive) {
        return apply_exact_phrase_regex(text, rule);
    }

    let mut result = String::new();
    let mut last_idx = 0usize;
    let mut search_from = 0usize;
    let mut changed = false;

    while let Some(rel_pos) = haystack[search_from..].find(needle) {
        let start = search_from + rel_pos;
        let end = start + needle.len();

        let boundary_ok_start = if rule.word_boundaries_start && start > 0 {
            !text[..start]
                .chars()
                .last()
                .is_some_and(|c| c.is_alphabetic() || c == '+')
        } else {
            true
        };
        let boundary_ok_end = if rule.word_boundaries_end && end < text.len() {
            !text[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '+')
        } else {
            true
        };

        if boundary_ok_start && boundary_ok_end {
            result.push_str(&text[last_idx..start]);
            let matched_slice = &text[start..end];
            result.push_str(&preserve_initial_capital(matched_slice, &rule.rhs));
            last_idx = end;
            search_from = end;
            changed = true;
        } else {
            let step = text[start..].chars().next().map_or(1, |c| c.len_utf8());
            search_from = start + step;
        }
    }

    if changed {
        result.push_str(&text[last_idx..]);
        Some(result)
    } else {
        None
    }
}

fn apply_exact_phrase_regex(text: &str, rule: &ExactPhraseRule) -> Option<String> {
    let escaped = regex::escape(&rule.lhs);
    let pat = format!(
        "{}{}{}",
        if rule.word_boundaries_start { r"\b" } else { "" },
        escaped,
        if rule.word_boundaries_end { r"\b" } else { "" }
    );
    let re = RegexBuilder::new(&pat)
        .case_insensitive(!rule.case_sensitive)
        .unicode(true)
        .build()
        .ok()?;
    let next = re.replace_all(text, regex::NoExpand(&rule.rhs));
    if next != text {
        Some(next.into_owned())
    } else {
        None
    }
}

fn compile_dic_wildcard_rule(
    lhs: &str,
    rhs: &str,
    case_sensitive: bool,
) -> Option<(CompiledRexRule, Vec<String>, String)> {
    let lhs_tokens: Vec<&str> = lhs.split_whitespace().collect();
    let rhs_tokens: Vec<&str> = rhs.split_whitespace().collect();
    if lhs_tokens.is_empty() {
        return None;
    }

    let mut required_words = Vec::new();
    let mut longest_core = String::new();

    for &lt in &lhs_tokens {
        if !lt.contains('*') {
            for w in extract_alpha_words(lt) {
                if w.chars().count() >= 2 {
                    required_words.push(w);
                }
            }
        } else {
            let core_clean = lt.trim_matches('*').trim_matches('.').to_lowercase();
            if core_clean.len() > longest_core.len() {
                longest_core = core_clean;
            }
        }
    }

    let non_standalone_count = lhs_tokens.iter().filter(|&&t| t != "*").count();
    let standalone_keeps_word =
        rhs_tokens.len() == non_standalone_count && non_standalone_count < lhs_tokens.len();

    let mut pattern = String::new();
    let mut replacement = String::new();
    let mut group_idx = 1usize;
    let mut rhs_idx = 0usize;

    for (ti, &lt) in lhs_tokens.iter().enumerate() {
        if ti > 0 {
            pattern.push_str(r"\s+");
            replacement.push(' ');
        }

        if lt == "*" {
            pattern.push_str(r"(\S+)");
            if standalone_keeps_word {
                replacement.push_str(&format!("${{{group_idx}}}"));
            } else if let Some(&rt) = rhs_tokens.get(rhs_idx) {
                replacement.push_str(rt);
                rhs_idx += 1;
            } else {
                replacement.push_str(&format!("${{{group_idx}}}"));
            }
            group_idx += 1;
            continue;
        }

        let rt = rhs_tokens.get(rhs_idx).copied().unwrap_or("");
        rhs_idx += 1;

        let starts_star = lt.starts_with('*');
        let ends_star = lt.ends_with('*') && lt.len() > 1;
        let core = lt.trim_matches('*');

        if starts_star {
            pattern.push_str(r"([\w+\-]*)");
            replacement.push_str(&format!("${{{group_idx}}}"));
            group_idx += 1;
        } else if core.chars().next().is_some_and(|c| c.is_alphabetic()) {
            pattern.push_str(r"\b");
        }

        pattern.push_str(&regex::escape(core));
        replacement.push_str(rt);

        if ends_star {
            pattern.push_str(r"([\w+\-]*)");
            replacement.push_str(&format!("${{{group_idx}}}"));
            group_idx += 1;
        } else if core.chars().last().is_some_and(|c| c.is_alphabetic()) {
            pattern.push_str(r"\b");
        }
    }

    while rhs_idx < rhs_tokens.len() {
        replacement.push(' ');
        replacement.push_str(rhs_tokens[rhs_idx]);
        rhs_idx += 1;
    }

    let regex = RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .unicode(true)
        .build()
        .ok()?;

    Some((
        CompiledRexRule {
            regex,
            replacement,
            raw_rule: format!("{lhs}={rhs}").chars().take(90).collect(),
            needs_digit: false,
            required_sub: None,
        },
        required_words,
        longest_core,
    ))
}

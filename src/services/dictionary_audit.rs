use std::path::Path;

use crate::engine::loader::sorted_dictionary_paths;
use crate::io::encoding::decode_text_bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditKind {
    Letters,
    Boundary,
    Punctuation,
    Mask,
}

impl AuditKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Letters => "буквы",
            Self::Boundary => "границы слов",
            Self::Punctuation => "пунктуация",
            Self::Mask => "маска",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditCandidate {
    pub line: usize,
    pub lhs: String,
    pub rhs: String,
    pub kind: AuditKind,
}

#[derive(Debug, Clone)]
pub struct DictionaryAudit {
    pub name: String,
    pub purpose: &'static str,
    pub total_rules: usize,
    pub candidates: Vec<AuditCandidate>,
}

#[derive(Debug, Clone, Default)]
pub struct AuditReport {
    pub dictionaries: Vec<DictionaryAudit>,
    pub skipped: Vec<(String, &'static str)>,
    pub scanned_rules: usize,
    pub total_candidates: usize,
}

impl AuditReport {
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "Аудит фонетических кандидатов: {} из {} правил\n",
            self.total_candidates, self.scanned_rules
        );
        if !self.skipped.is_empty() {
            out.push_str("\nНе аудировались по назначению:\n");
            for (name, purpose) in &self.skipped {
                out.push_str(&format!("  {name} [{purpose}]\n"));
            }
        }
        for dictionary in &self.dictionaries {
            if dictionary.candidates.is_empty() {
                continue;
            }
            out.push_str(&format!(
                "\n== {} [{}] — {} из {} правил\n",
                dictionary.name,
                dictionary.purpose,
                dictionary.candidates.len(),
                dictionary.total_rules
            ));
            for candidate in &dictionary.candidates {
                out.push_str(&format!(
                    "  стр. {}: {} → {} [{}]\n",
                    candidate.line,
                    candidate.lhs,
                    candidate.rhs,
                    candidate.kind.label()
                ));
            }
        }
        out
    }
}

pub fn dictionary_purpose(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("edge_yo") || lower.starts_with("33_") {
        "ё-фикация"
    } else if lower.starts_with("41_1")
        || lower.starts_with("41_2")
        || lower.starts_with("42_1")
        || lower.starts_with("42_2")
    {
        "все/всё: звёзды"
    } else if lower.starts_with("41_") || lower.starts_with("42_") {
        "все/всё"
    } else if lower.starts_with("40.") {
        "омографы: звёзды"
    } else if lower.starts_with("50_")
        || lower.starts_with("51_")
        || lower.starts_with("52_")
        || lower.starts_with("53_")
    {
        "омографы: фразы"
    } else if lower.starts_with("60_") {
        "пост-обработка"
    } else if lower.starts_with("65_") {
        "транслитерация"
    } else if lower.starts_with("67_") {
        "автор"
    } else if lower.starts_with("69_") || lower.starts_with("70_") {
        "конвертеры"
    } else if lower.starts_with("71_") || lower.starts_with("72_") {
        "ударения"
    } else if lower.starts_with("73_") {
        "аббревиатуры"
    } else if lower.starts_with("74_") {
        "мат"
    } else if lower.starts_with("75_") || lower.starts_with("77_") {
        "имена"
    } else if lower.starts_with("76_") {
        "импорт: фразы"
    } else if lower.starts_with("95.") {
        "база"
    } else if lower.contains("(hard-e)") || lower.starts_with("96_") {
        "твёрдое э"
    } else if lower.starts_with("35_") {
        "пользовательский"
    } else {
        "прочее"
    }
}

pub fn expected_by_design(purpose: &str) -> bool {
    matches!(
        purpose,
        "ё-фикация"
            | "база"
            | "омографы: звёзды"
            | "все/всё: звёзды"
            | "транслитерация"
            | "конвертеры"
            | "аббревиатуры"
            | "мат"
            | "имена"
            | "твёрдое э"
    )
}

pub fn audit_dictionaries(dir: &Path) -> Result<AuditReport, String> {
    let paths = sorted_dictionary_paths(dir)?;
    let mut report = AuditReport::default();

    for path in paths {
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dic"))
        {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unknown>")
            .to_string();
        let purpose = dictionary_purpose(&name);
        if expected_by_design(purpose) {
            report.skipped.push((name, purpose));
            continue;
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("Не удалось прочитать {}: {error}", path.display()))?;
        let (content, _) = decode_text_bytes(&bytes);
        let mut total_rules = 0usize;
        let mut candidates = Vec::new();

        for (index, raw_line) in content.lines().enumerate() {
            let line = raw_line.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || (line.starts_with('#') && !line.starts_with("##")) {
                continue;
            }
            let Some((lhs, rhs)) = split_rule(line) else {
                continue;
            };
            let lhs = lhs.trim();
            let rhs = rhs.trim();
            if lhs.is_empty() {
                continue;
            }
            total_rules += 1;
            if let Some(kind) = classify_rule(lhs, rhs) {
                candidates.push(AuditCandidate {
                    line: index + 1,
                    lhs: lhs.to_string(),
                    rhs: rhs.to_string(),
                    kind,
                });
            }
        }

        report.scanned_rules += total_rules;
        report.total_candidates += candidates.len();
        report.dictionaries.push(DictionaryAudit {
            name,
            purpose,
            total_rules,
            candidates,
        });
    }

    Ok(report)
}

fn split_rule(line: &str) -> Option<(&str, &str)> {
    if let Some(position) = line.find('\x06') {
        return Some((&line[..position], &line[position + 1..]));
    }
    if let Some(position) = line.find("::") {
        return Some((&line[..position], &line[position + 2..]));
    }
    line.split_once('=')
}

fn strip_markers(lhs: &str) -> &str {
    if let Some(rest) = lhs.strip_prefix("$$") {
        rest
    } else if let Some(rest) = lhs.strip_prefix("##") {
        rest
    } else if let Some(rest) = lhs.strip_prefix('$') {
        rest
    } else {
        lhs
    }
}

fn normalized_chars(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars()
        .filter(|character| *character != '+')
        .flat_map(|character| character.to_lowercase())
        .map(|character| if character == 'ё' { 'е' } else { character })
}

fn normalized_eq(left: &str, right: &str) -> bool {
    normalized_chars(left).eq(normalized_chars(right))
}

fn normalize_text(text: &str) -> String {
    text.to_lowercase()
        .replace('+', "")
        .replace('ё', "е")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_letters(text: &str) -> String {
    normalize_text(text)
        .chars()
        .filter(|character| character.is_alphabetic())
        .collect()
}

fn classify_rule(lhs: &str, rhs: &str) -> Option<AuditKind> {
    if lhs.contains('*') || rhs.contains('*') {
        return Some(AuditKind::Mask);
    }
    let lhs = strip_markers(lhs);
    if normalized_eq(lhs, rhs) {
        return None;
    }
    if lhs.split_whitespace().count() != rhs.split_whitespace().count() {
        return Some(AuditKind::Boundary);
    }
    if normalize_letters(lhs) != normalize_letters(rhs) {
        return Some(AuditKind::Letters);
    }
    Some(AuditKind::Punctuation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> std::path::PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "demagog-audit-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp audit dir");
        dir
    }

    #[test]
    fn classifies_phonetic_candidates() {
        let dir = temp_dir();
        std::fs::write(
            dir.join("60_пост обработка@.dic"),
            concat!(
                "дренажом=дренажем\n",
                "не было=н+ебыло\n",
                "белье, обувь=бельё обувь\n",
                "все=всё\n",
                "тест=т+ест\n",
                "*кно на=кно, на\n",
            ),
        )
        .expect("write post-processing dictionary");
        std::fs::write(dir.join("65_ЛАТИНИЦА@.dic"), "finita=финита\n")
            .expect("write transliteration dictionary");
        std::fs::write(dir.join("10_REX.rex"), "# ignored\n").expect("write rex");
        std::fs::write(dir.join("notes.txt"), "ignored\n").expect("write notes");

        let report = audit_dictionaries(&dir).expect("audit dictionaries");
        assert_eq!(report.scanned_rules, 6);
        assert_eq!(report.total_candidates, 4);
        assert_eq!(report.dictionaries.len(), 1);
        assert!(
            report
                .skipped
                .iter()
                .any(|(name, purpose)| name.starts_with("65_") && *purpose == "транслитерация")
        );

        let post = &report.dictionaries[0];
        assert_eq!(post.purpose, "пост-обработка");
        assert_eq!(post.total_rules, 6);
        assert_eq!(post.candidates.len(), 4);
        assert_eq!(post.candidates[0].line, 1);
        assert_eq!(post.candidates[0].kind, AuditKind::Letters);
        assert_eq!(post.candidates[1].kind, AuditKind::Boundary);
        assert_eq!(post.candidates[2].kind, AuditKind::Punctuation);
        assert_eq!(post.candidates[3].kind, AuditKind::Mask);

        let text = report.to_text();
        assert!(text.contains("Аудит фонетических кандидатов"));
        assert!(text.contains("дренажом"));
        assert!(text.contains("65_ЛАТИНИЦА@.dic"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_stress_and_yo_only_rules() {
        let dir = temp_dir();
        std::fs::write(
            dir.join("95.NEW_Орфоэпия(phoneme).dic"),
            concat!("человек=челов+ек\n", "все=всё\n", "зеленый=зелён+ый\n",),
        )
        .expect("write orthoepy dictionary");

        let report = audit_dictionaries(&dir).expect("audit dictionaries");
        assert_eq!(report.scanned_rules, 0);
        assert_eq!(report.total_candidates, 0);
        assert!(report.dictionaries.is_empty());
        assert!(
            report
                .skipped
                .iter()
                .any(|(name, purpose)| name.starts_with("95.") && *purpose == "база")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sorts_candidates_by_line_and_keeps_dictionary_order() {
        let dir = temp_dir();
        std::fs::write(
            dir.join("67_TTS_Offline АВТОР(phoneme).dic"),
            concat!(
                "свои дома=свои дом+а\n",
                "Всьо пошло=всё пошло\n",
                "не было=н+ебыло\n",
            ),
        )
        .expect("write author dictionary");
        std::fs::write(dir.join("60_пост обработка@.dic"), "дренажом=дренажем\n")
            .expect("write post-processing dictionary");

        let report = audit_dictionaries(&dir).expect("audit dictionaries");
        assert_eq!(report.dictionaries[0].name, "60_пост обработка@.dic");
        assert_eq!(report.scanned_rules, 4);
        assert_eq!(report.total_candidates, 3);
        let author = &report.dictionaries[1];
        assert_eq!(author.candidates.len(), 2);
        assert_eq!(author.candidates[0].line, 2);
        assert_eq!(author.candidates[1].line, 3);
        assert_eq!(author.candidates[0].kind, AuditKind::Letters);
        assert_eq!(author.candidates[1].kind, AuditKind::Boundary);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

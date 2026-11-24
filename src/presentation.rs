use crate::domain::{DictionaryKind, DictionaryTag, StressFormat, ThemeMode};
use crate::services::dictionary_audit::{AuditKind, AuditReport};

pub fn theme_label(theme: ThemeMode) -> &'static str {
    match theme {
        ThemeMode::Dark => "Тёмная",
        ThemeMode::Light => "Светлая",
        ThemeMode::Warm => "Тёплая",
    }
}

pub fn stress_format_label(format: StressFormat) -> &'static str {
    format.export_label()
}

pub fn stress_format_short_label(format: StressFormat) -> &'static str {
    match format {
        StressFormat::PlusBeforeVowel => "+ перед гласной",
        StressFormat::AcuteAccent => "Ударение Юникода",
        StressFormat::CleanYoOnly => "Только Ё",
    }
}

pub fn dictionary_kind_badge(kind: DictionaryKind) -> &'static str {
    match kind {
        DictionaryKind::RegexStandard => "REX",
        DictionaryKind::RegexIndexed => "REX-IDX",
        DictionaryKind::DicStandard => "DIC",
        DictionaryKind::DicStressConverter => "DIC-STRESS",
        DictionaryKind::HardE => "HARD-E",
    }
}

pub fn dictionary_tag_label(tag: DictionaryTag) -> &'static str {
    match tag {
        DictionaryTag::NumbersAndDates => "ЧИСЛА И ДАТЫ",
        DictionaryTag::YoFication => "Ё-ФИКАЦИЯ",
        DictionaryTag::HomographsCap => "ОМОГРАФЫ (загл.)",
        DictionaryTag::CapToPlusConverter => "ЗАГЛАВНЫЕ -> '+'",
        DictionaryTag::PlusStressWords => "УДАРЕНИЯ '+'",
        DictionaryTag::PlusStressOrthoepy => "ОРФОЭПИЯ '+' (2.9М)",
        DictionaryTag::HardPronunciation => "ТВЁРДОЕ «Э»",
        DictionaryTag::CleanupAndTranslit => "ЧИСТКА / ТРАНСЛИТ",
    }
}

pub fn audit_kind_label(kind: AuditKind) -> &'static str {
    match kind {
        AuditKind::Letters => "буквы",
        AuditKind::Boundary => "границы слов",
        AuditKind::Punctuation => "пунктуация",
        AuditKind::Mask => "маска",
    }
}

pub fn audit_report_text(report: &AuditReport) -> String {
    let mut out = format!(
        "Аудит фонетических кандидатов: {} из {} правил\n",
        report.total_candidates, report.scanned_rules
    );
    if !report.skipped.is_empty() {
        out.push_str("\nНе аудировались по назначению:\n");
        for (name, purpose) in &report.skipped {
            out.push_str(&format!("  {name} [{purpose}]\n"));
        }
    }
    for dictionary in &report.dictionaries {
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
                audit_kind_label(candidate.kind)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::dictionary_audit::{AuditCandidate, DictionaryAudit};

    #[test]
    fn theme_labels_match_the_interface_contract() {
        let cases = [
            (ThemeMode::Dark, "Тёмная"),
            (ThemeMode::Light, "Светлая"),
            (ThemeMode::Warm, "Тёплая"),
        ];

        for (theme, expected) in cases {
            assert_eq!(theme_label(theme), expected, "{theme:?}");
        }
    }

    #[test]
    fn stress_format_labels_match_the_interface_contract() {
        let cases = [
            (
                StressFormat::PlusBeforeVowel,
                "+ перед гласной (з+амок)",
                "+ перед гласной",
            ),
            (
                StressFormat::AcuteAccent,
                "Ударение Юникода (за́мок) [U+0301]",
                "Ударение Юникода",
            ),
            (
                StressFormat::CleanYoOnly,
                "Без ударений — только Ё (замок)",
                "Только Ё",
            ),
        ];

        for (format, expected_label, expected_short_label) in cases {
            assert_eq!(stress_format_label(format), expected_label, "{format:?}");
            assert_eq!(
                stress_format_short_label(format),
                expected_short_label,
                "{format:?}"
            );
        }
    }

    #[test]
    fn dictionary_kind_badges_match_the_interface_contract() {
        let cases = [
            (DictionaryKind::RegexStandard, "REX"),
            (DictionaryKind::RegexIndexed, "REX-IDX"),
            (DictionaryKind::DicStandard, "DIC"),
            (DictionaryKind::DicStressConverter, "DIC-STRESS"),
            (DictionaryKind::HardE, "HARD-E"),
        ];

        for (kind, expected) in cases {
            assert_eq!(dictionary_kind_badge(kind), expected, "{kind:?}");
        }
    }

    #[test]
    fn dictionary_tag_labels_match_the_interface_contract() {
        let cases = [
            (DictionaryTag::NumbersAndDates, "ЧИСЛА И ДАТЫ"),
            (DictionaryTag::YoFication, "Ё-ФИКАЦИЯ"),
            (DictionaryTag::HomographsCap, "ОМОГРАФЫ (загл.)"),
            (DictionaryTag::CapToPlusConverter, "ЗАГЛАВНЫЕ -> '+'"),
            (DictionaryTag::PlusStressWords, "УДАРЕНИЯ '+'"),
            (DictionaryTag::PlusStressOrthoepy, "ОРФОЭПИЯ '+' (2.9М)"),
            (DictionaryTag::HardPronunciation, "ТВЁРДОЕ «Э»"),
            (DictionaryTag::CleanupAndTranslit, "ЧИСТКА / ТРАНСЛИТ"),
        ];

        for (tag, expected) in cases {
            assert_eq!(dictionary_tag_label(tag), expected, "{tag:?}");
        }
    }

    #[test]
    fn audit_kind_labels_match_the_report_contract() {
        let cases = [
            (AuditKind::Letters, "буквы"),
            (AuditKind::Boundary, "границы слов"),
            (AuditKind::Punctuation, "пунктуация"),
            (AuditKind::Mask, "маска"),
        ];

        for (kind, expected) in cases {
            assert_eq!(audit_kind_label(kind), expected, "{kind:?}");
        }
    }

    #[test]
    fn audit_report_text_preserves_the_complete_russian_output() {
        let report = AuditReport {
            dictionaries: vec![
                DictionaryAudit {
                    name: "60_пост обработка@.dic".to_string(),
                    purpose: "пост-обработка",
                    total_rules: 6,
                    candidates: vec![
                        AuditCandidate {
                            line: 1,
                            lhs: "дренажом".to_string(),
                            rhs: "дренажем".to_string(),
                            kind: AuditKind::Letters,
                        },
                        AuditCandidate {
                            line: 2,
                            lhs: "не было".to_string(),
                            rhs: "н+ебыло".to_string(),
                            kind: AuditKind::Boundary,
                        },
                        AuditCandidate {
                            line: 3,
                            lhs: "белье, обувь".to_string(),
                            rhs: "бельё обувь".to_string(),
                            kind: AuditKind::Punctuation,
                        },
                        AuditCandidate {
                            line: 6,
                            lhs: "*кно на".to_string(),
                            rhs: "кно, на".to_string(),
                            kind: AuditKind::Mask,
                        },
                    ],
                },
                DictionaryAudit {
                    name: "пусто.dic".to_string(),
                    purpose: "прочее",
                    total_rules: 1,
                    candidates: Vec::new(),
                },
            ],
            skipped: vec![
                ("65_ЛАТИНИЦА@.dic".to_string(), "транслитерация"),
                ("95.NEW_Орфоэпия(phoneme).dic".to_string(), "база"),
            ],
            scanned_rules: 7,
            total_candidates: 4,
        };
        let expected = concat!(
            "Аудит фонетических кандидатов: 4 из 7 правил\n",
            "\nНе аудировались по назначению:\n",
            "  65_ЛАТИНИЦА@.dic [транслитерация]\n",
            "  95.NEW_Орфоэпия(phoneme).dic [база]\n",
            "\n== 60_пост обработка@.dic [пост-обработка] — 4 из 6 правил\n",
            "  стр. 1: дренажом → дренажем [буквы]\n",
            "  стр. 2: не было → н+ебыло [границы слов]\n",
            "  стр. 3: белье, обувь → бельё обувь [пунктуация]\n",
            "  стр. 6: *кно на → кно, на [маска]\n",
        );

        assert_eq!(audit_report_text(&report), expected);
    }
}

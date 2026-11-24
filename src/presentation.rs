use crate::domain::{
    DictionaryKind, DictionaryPurpose, DictionaryStage, DictionaryTag, StressFormat, ThemeMode,
};
use crate::services::dictionary_audit::{AuditKind, AuditReport};
use crate::services::dictionary_check::{CheckDiagnostic, ManifestDrift, OrderDiagnostic};

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

pub fn dictionary_stage_label(stage: DictionaryStage) -> &'static str {
    match stage {
        DictionaryStage::Rex => "REX",
        DictionaryStage::YoBase => "Ё: БАЗА",
        DictionaryStage::YoAdditions => "Ё: ДОПОЛНЕНИЯ",
        DictionaryStage::User => "ПОЛЬЗОВАТЕЛЬСКИЙ",
        DictionaryStage::Homographs => "ОМОГРАФЫ",
        DictionaryStage::Cleanup => "ЧИСТКА",
        DictionaryStage::Author => "АВТОР",
        DictionaryStage::Converters => "КОНВЕРТЕРЫ",
        DictionaryStage::StressWords => "УДАРЕНИЯ",
        DictionaryStage::PhrasesImport => "ИМПОРТ: ФРАЗЫ",
        DictionaryStage::NamesImport => "ИМПОРТ: ИМЕНА",
        DictionaryStage::Orthoepy => "БАЗА",
        DictionaryStage::HardE => "HARD-E",
        DictionaryStage::Unknown => "НЕИЗВЕСТНО",
        DictionaryStage::Unnumbered => "БЕЗ НОМЕРА",
    }
}

pub fn dictionary_purpose_label(purpose: DictionaryPurpose) -> &'static str {
    match purpose {
        DictionaryPurpose::YoFication => "ё-фикация",
        DictionaryPurpose::AllVsYoStars => "все/всё: звёзды",
        DictionaryPurpose::AllVsYo => "все/всё",
        DictionaryPurpose::HomographsStars => "омографы: звёзды",
        DictionaryPurpose::HomographsPhrases => "омографы: фразы",
        DictionaryPurpose::PostProcessing => "пост-обработка",
        DictionaryPurpose::Transliteration => "транслитерация",
        DictionaryPurpose::Author => "автор",
        DictionaryPurpose::Converters => "конвертеры",
        DictionaryPurpose::StressWords => "ударения",
        DictionaryPurpose::Abbreviations => "аббревиатуры",
        DictionaryPurpose::Profanity => "мат",
        DictionaryPurpose::Names => "имена",
        DictionaryPurpose::PhrasesImport => "импорт: фразы",
        DictionaryPurpose::Base => "база",
        DictionaryPurpose::HardE => "твёрдое э",
        DictionaryPurpose::User => "пользовательский",
        DictionaryPurpose::Other => "прочее",
    }
}

pub fn check_diagnostic_text(diagnostic: &CheckDiagnostic) -> String {
    match diagnostic {
        CheckDiagnostic::Order(diagnostic) => match diagnostic {
            OrderDiagnostic::RexWithoutNumber { name } => {
                format!("{name}: .rex без номера — проверьте порядок")
            }
            OrderDiagnostic::Unnumbered { name } => {
                format!("{name}: .dic без номера — загрузчик поставит его после HARD-E")
            }
            OrderDiagnostic::UnknownStage { name } => {
                format!("{name}: неизвестный этап — проверьте порядок вручную")
            }
            OrderDiagnostic::DuplicateHardE { name } => {
                format!("{name}: второй словарь HARD-E")
            }
            OrderDiagnostic::HardEMissingMarker { name } => {
                format!("{name}: номер 96 без маркера (hard-e) — это не HARD-E")
            }
            OrderDiagnostic::DuplicateYoBase { name } => {
                format!("{name}: второй базовый ё-словарь")
            }
            OrderDiagnostic::HardEStageName { name } => {
                format!("{name}: HARD-E должен называться 96_... (hard-e).dic")
            }
            OrderDiagnostic::StageRegression {
                name,
                stage,
                previous_name,
                previous_stage,
            } => format!(
                "{name} ({}) идёт после {previous_name} ({}), но должен быть раньше",
                dictionary_stage_label(*stage),
                dictionary_stage_label(*previous_stage)
            ),
            OrderDiagnostic::RexAfterDic { name } => {
                format!("{name}: .rex должен обрабатываться до .dic")
            }
            OrderDiagnostic::AfterHardE { name } => format!("{name}: идёт после HARD-E"),
        },
        CheckDiagnostic::Manifest(diagnostic) => match diagnostic {
            ManifestDrift::MissingFromManifest { name } => {
                format!("Словарь {name} отсутствует в манифесте")
            }
            ManifestDrift::Changed { name } => {
                format!("Словарь {name} изменился относительно манифеста")
            }
            ManifestDrift::MissingOnDisk { name } => {
                format!("Словарь {name} из манифеста отсутствует на диске")
            }
        },
        CheckDiagnostic::LoadFailed(error) => format!("Словарь не загружен: {error}"),
        CheckDiagnostic::CompileWarning {
            dictionary,
            warning,
        } => {
            format!("{dictionary}: {warning}")
        }
        CheckDiagnostic::AuditFailed(error) => format!("Аудит фонетики не выполнен: {error}"),
        CheckDiagnostic::GoldenMismatch {
            case,
            expected,
            actual,
        } => format!("{case}: ожидалось «{expected}», получено «{actual}»"),
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
            out.push_str(&format!(
                "  {name} [{}]\n",
                dictionary_purpose_label(*purpose)
            ));
        }
    }
    for dictionary in &report.dictionaries {
        if dictionary.candidates.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "\n== {} [{}] — {} из {} правил\n",
            dictionary.name,
            dictionary_purpose_label(dictionary.purpose),
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
    use crate::domain::{DictionaryPurpose, DictionaryStage};
    use crate::services::dictionary_audit::{AuditCandidate, DictionaryAudit};
    use crate::services::dictionary_check::{CheckDiagnostic, ManifestDrift, OrderDiagnostic};

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
    fn dictionary_stage_labels_match_the_check_contract() {
        let cases = [
            (DictionaryStage::Rex, "REX"),
            (DictionaryStage::YoBase, "Ё: БАЗА"),
            (DictionaryStage::YoAdditions, "Ё: ДОПОЛНЕНИЯ"),
            (DictionaryStage::User, "ПОЛЬЗОВАТЕЛЬСКИЙ"),
            (DictionaryStage::Homographs, "ОМОГРАФЫ"),
            (DictionaryStage::Cleanup, "ЧИСТКА"),
            (DictionaryStage::Author, "АВТОР"),
            (DictionaryStage::Converters, "КОНВЕРТЕРЫ"),
            (DictionaryStage::StressWords, "УДАРЕНИЯ"),
            (DictionaryStage::PhrasesImport, "ИМПОРТ: ФРАЗЫ"),
            (DictionaryStage::NamesImport, "ИМПОРТ: ИМЕНА"),
            (DictionaryStage::Orthoepy, "БАЗА"),
            (DictionaryStage::HardE, "HARD-E"),
            (DictionaryStage::Unknown, "НЕИЗВЕСТНО"),
            (DictionaryStage::Unnumbered, "БЕЗ НОМЕРА"),
        ];

        for (stage, expected) in cases {
            assert_eq!(dictionary_stage_label(stage), expected, "{stage:?}");
        }
    }

    #[test]
    fn dictionary_purpose_labels_match_the_audit_contract() {
        let cases = [
            (DictionaryPurpose::YoFication, "ё-фикация"),
            (DictionaryPurpose::AllVsYoStars, "все/всё: звёзды"),
            (DictionaryPurpose::AllVsYo, "все/всё"),
            (DictionaryPurpose::HomographsStars, "омографы: звёзды"),
            (DictionaryPurpose::HomographsPhrases, "омографы: фразы"),
            (DictionaryPurpose::PostProcessing, "пост-обработка"),
            (DictionaryPurpose::Transliteration, "транслитерация"),
            (DictionaryPurpose::Author, "автор"),
            (DictionaryPurpose::Converters, "конвертеры"),
            (DictionaryPurpose::StressWords, "ударения"),
            (DictionaryPurpose::Abbreviations, "аббревиатуры"),
            (DictionaryPurpose::Profanity, "мат"),
            (DictionaryPurpose::Names, "имена"),
            (DictionaryPurpose::PhrasesImport, "импорт: фразы"),
            (DictionaryPurpose::Base, "база"),
            (DictionaryPurpose::HardE, "твёрдое э"),
            (DictionaryPurpose::User, "пользовательский"),
            (DictionaryPurpose::Other, "прочее"),
        ];

        for (purpose, expected) in cases {
            assert_eq!(dictionary_purpose_label(purpose), expected, "{purpose:?}");
        }
    }

    #[test]
    fn order_diagnostic_text_preserves_every_russian_message() {
        let cases = [
            (
                OrderDiagnostic::RexWithoutNumber {
                    name: "numbers.rex".to_string(),
                },
                "numbers.rex: .rex без номера — проверьте порядок",
            ),
            (
                OrderDiagnostic::Unnumbered {
                    name: "custom.dic".to_string(),
                },
                "custom.dic: .dic без номера — загрузчик поставит его после HARD-E",
            ),
            (
                OrderDiagnostic::UnknownStage {
                    name: "97_custom.dic".to_string(),
                },
                "97_custom.dic: неизвестный этап — проверьте порядок вручную",
            ),
            (
                OrderDiagnostic::DuplicateHardE {
                    name: "96_second(hard-e).dic".to_string(),
                },
                "96_second(hard-e).dic: второй словарь HARD-E",
            ),
            (
                OrderDiagnostic::HardEMissingMarker {
                    name: "96_first.dic".to_string(),
                },
                "96_first.dic: номер 96 без маркера (hard-e) — это не HARD-E",
            ),
            (
                OrderDiagnostic::DuplicateYoBase {
                    name: "other_yo.dic".to_string(),
                },
                "other_yo.dic: второй базовый ё-словарь",
            ),
            (
                OrderDiagnostic::HardEStageName {
                    name: "33_bad(hard-e).dic".to_string(),
                },
                "33_bad(hard-e).dic: HARD-E должен называться 96_... (hard-e).dic",
            ),
            (
                OrderDiagnostic::StageRegression {
                    name: "76_phrases.dic".to_string(),
                    stage: DictionaryStage::PhrasesImport,
                    previous_name: "77_names.dic".to_string(),
                    previous_stage: DictionaryStage::NamesImport,
                },
                "76_phrases.dic (ИМПОРТ: ФРАЗЫ) идёт после 77_names.dic (ИМПОРТ: ИМЕНА), но должен быть раньше",
            ),
            (
                OrderDiagnostic::RexAfterDic {
                    name: "20_late.rex".to_string(),
                },
                "20_late.rex: .rex должен обрабатываться до .dic",
            ),
            (
                OrderDiagnostic::AfterHardE {
                    name: "97_after.dic".to_string(),
                },
                "97_after.dic: идёт после HARD-E",
            ),
        ];

        for (diagnostic, expected) in cases {
            assert_eq!(
                check_diagnostic_text(&CheckDiagnostic::Order(diagnostic)),
                expected
            );
        }
    }

    #[test]
    fn manifest_drift_text_preserves_every_russian_message() {
        let cases = [
            (
                ManifestDrift::MissingFromManifest {
                    name: "extra.dic".to_string(),
                },
                "Словарь extra.dic отсутствует в манифесте",
            ),
            (
                ManifestDrift::Changed {
                    name: "changed.dic".to_string(),
                },
                "Словарь changed.dic изменился относительно манифеста",
            ),
            (
                ManifestDrift::MissingOnDisk {
                    name: "missing.dic".to_string(),
                },
                "Словарь missing.dic из манифеста отсутствует на диске",
            ),
        ];

        for (diagnostic, expected) in cases {
            assert_eq!(
                check_diagnostic_text(&CheckDiagnostic::Manifest(diagnostic)),
                expected
            );
        }
    }

    #[test]
    fn check_diagnostic_text_preserves_every_russian_message() {
        let cases = [
            (
                CheckDiagnostic::Order(OrderDiagnostic::Unnumbered {
                    name: "custom.dic".to_string(),
                }),
                "custom.dic: .dic без номера — загрузчик поставит его после HARD-E",
            ),
            (
                CheckDiagnostic::Manifest(ManifestDrift::Changed {
                    name: "changed.dic".to_string(),
                }),
                "Словарь changed.dic изменился относительно манифеста",
            ),
            (
                CheckDiagnostic::LoadFailed("10_bad.dic: ошибка чтения".to_string()),
                "Словарь не загружен: 10_bad.dic: ошибка чтения",
            ),
            (
                CheckDiagnostic::CompileWarning {
                    dictionary: "96_hard(hard-e).dic".to_string(),
                    warning: "многословные правила не поддерживаются".to_string(),
                },
                "96_hard(hard-e).dic: многословные правила не поддерживаются",
            ),
            (
                CheckDiagnostic::AuditFailed("ошибка чтения".to_string()),
                "Аудит фонетики не выполнен: ошибка чтения",
            ),
            (
                CheckDiagnostic::GoldenMismatch {
                    case: "числа",
                    expected: "п+ять кил+о",
                    actual: "5 кг".to_string(),
                },
                "числа: ожидалось «п+ять кил+о», получено «5 кг»",
            ),
        ];

        for (diagnostic, expected) in cases {
            assert_eq!(check_diagnostic_text(&diagnostic), expected);
        }
    }

    #[test]
    fn audit_report_text_preserves_the_complete_russian_output() {
        let report = AuditReport {
            dictionaries: vec![
                DictionaryAudit {
                    name: "60_пост обработка@.dic".to_string(),
                    purpose: DictionaryPurpose::PostProcessing,
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
                    purpose: DictionaryPurpose::Other,
                    total_rules: 1,
                    candidates: Vec::new(),
                },
            ],
            skipped: vec![
                (
                    "65_ЛАТИНИЦА@.dic".to_string(),
                    DictionaryPurpose::Transliteration,
                ),
                (
                    "95.NEW_Орфоэпия(phoneme).dic".to_string(),
                    DictionaryPurpose::Base,
                ),
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

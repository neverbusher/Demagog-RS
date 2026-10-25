#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedDictionary {
    pub name: String,
    pub stage: &'static str,
}

#[derive(Debug, Default)]
pub struct OrderReport {
    pub ordered: Vec<OrderedDictionary>,
    pub warnings: Vec<String>,
    pub failures: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Rex,
    YoBase,
    YoAdditions,
    User,
    Homographs,
    Cleanup,
    Author,
    Converters,
    StressWords,
    PhrasesImport,
    NamesImport,
    Orthoepy,
    HardE,
    Unknown,
    Unnumbered,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Self::Rex => "REX",
            Self::YoBase => "Ё: БАЗА",
            Self::YoAdditions => "Ё: ДОПОЛНЕНИЯ",
            Self::User => "ПОЛЬЗОВАТЕЛЬСКИЙ",
            Self::Homographs => "ОМОГРАФЫ",
            Self::Cleanup => "ЧИСТКА",
            Self::Author => "АВТОР",
            Self::Converters => "КОНВЕРТЕРЫ",
            Self::StressWords => "УДАРЕНИЯ",
            Self::PhrasesImport => "ИМПОРТ: ФРАЗЫ",
            Self::NamesImport => "ИМПОРТ: ИМЕНА",
            Self::Orthoepy => "БАЗА",
            Self::HardE => "HARD-E",
            Self::Unknown => "НЕИЗВЕСТНО",
            Self::Unnumbered => "БЕЗ НОМЕРА",
        }
    }
}

fn leading_number(name: &str) -> Option<u32> {
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn classify(name: &str) -> Stage {
    let lower = name.to_lowercase();
    if lower.ends_with(".rex") {
        return Stage::Rex;
    }
    let Some(number) = leading_number(name) else {
        return if lower.starts_with("edge_yo") {
            Stage::YoBase
        } else {
            Stage::Unnumbered
        };
    };
    match number {
        33 => Stage::YoAdditions,
        35 => Stage::User,
        40..=42 | 50..=53 => Stage::Homographs,
        60 | 65 => Stage::Cleanup,
        67 => Stage::Author,
        69 | 70 => Stage::Converters,
        71..=75 => Stage::StressWords,
        76 => Stage::PhrasesImport,
        77 => Stage::NamesImport,
        95 => Stage::Orthoepy,
        96 => Stage::HardE,
        _ => Stage::Unknown,
    }
}

pub fn validate_order(names: &[String]) -> OrderReport {
    let mut report = OrderReport::default();
    let mut previous: Option<(Stage, &str)> = None;
    let mut hard_e_index: Option<usize> = None;
    let mut yo_base_index: Option<usize> = None;
    let mut last_rex: Option<usize> = None;
    let mut first_dic: Option<usize> = None;

    for (index, name) in names.iter().enumerate() {
        let stage = classify(name);
        let lower = name.to_lowercase();
        let has_hard_marker = lower.contains("(hard-e)");
        report.ordered.push(OrderedDictionary {
            name: name.clone(),
            stage: stage.label(),
        });

        if stage == Stage::Rex {
            last_rex = Some(index);
            if leading_number(name).is_none() {
                report
                    .warnings
                    .push(format!("{name}: .rex без номера — проверьте порядок"));
            }
        } else if first_dic.is_none() {
            first_dic = Some(index);
        }

        match stage {
            Stage::Unnumbered => report.failures.push(format!(
                "{name}: .dic без номера — загрузчик поставит его после HARD-E"
            )),
            Stage::Unknown => report.warnings.push(format!(
                "{name}: неизвестный этап — проверьте порядок вручную"
            )),
            Stage::HardE => {
                if hard_e_index.is_some() {
                    report
                        .warnings
                        .push(format!("{name}: второй словарь HARD-E"));
                }
                hard_e_index = Some(index);
                if !has_hard_marker {
                    report.warnings.push(format!(
                        "{name}: номер 96 без маркера (hard-e) — это не HARD-E"
                    ));
                }
            }
            Stage::YoBase => {
                if yo_base_index.is_some() {
                    report
                        .warnings
                        .push(format!("{name}: второй базовый ё-словарь"));
                }
                yo_base_index = Some(index);
            }
            _ => {}
        }

        if has_hard_marker && stage != Stage::HardE {
            report.failures.push(format!(
                "{name}: HARD-E должен называться 96_... (hard-e).dic"
            ));
        }

        if !matches!(stage, Stage::Unknown | Stage::Unnumbered) {
            if let Some((previous_stage, previous_name)) = previous
                && stage < previous_stage
            {
                report.failures.push(format!(
                    "{name} ({}) идёт после {previous_name} ({}), но должен быть раньше",
                    stage.label(),
                    previous_stage.label()
                ));
            }
            previous = Some((stage, name));
        }
    }

    if let (Some(last_rex), Some(first_dic)) = (last_rex, first_dic)
        && last_rex > first_dic
    {
        report.failures.push(format!(
            "{}: .rex должен обрабатываться до .dic",
            names[last_rex]
        ));
    }

    if let Some(hard_e_index) = hard_e_index {
        for name in &names[hard_e_index + 1..] {
            report.failures.push(format!("{name}: идёт после HARD-E"));
        }
    }

    report
}

use std::path::Path;
use std::sync::Arc;

use crate::domain::{ChunkConfig, StressFormat};
use crate::engine::loader::{load_dictionaries_parallel, sorted_dictionary_paths};
use crate::ports::TextProcessor;
use crate::services::chunker::SentenceAwareChunker;
use crate::services::dictionary_audit::{AuditReport, audit_dictionaries};
use crate::services::pipeline::{PipelineOptions, ProcessingPipeline};
use crate::services::stress::RussianStressFormatter;

pub struct GoldenCase {
    pub name: &'static str,
    pub input: &'static str,
    pub expected: &'static str,
}

pub const GOLDEN_CASES: &[GoldenCase] = &[
    GoldenCase {
        name: "числа",
        input: "5 кг",
        expected: "п+ять кил+о",
    },
    GoldenCase {
        name: "все/всё",
        input: "все равно",
        expected: "всё равн+о",
    },
    GoldenCase {
        name: "омограф замок",
        input: "замок на двери",
        expected: "зам+ок на двер+и",
    },
    GoldenCase {
        name: "ё-фикация",
        input: "елка",
        expected: "ёлка",
    },
    GoldenCase {
        name: "дополнения ё",
        input: "осужденная",
        expected: "осуждённая",
    },
    GoldenCase {
        name: "заглавные -> +",
        input: "завоевывать земли",
        expected: "завоёвывать з+емли",
    },
    GoldenCase {
        name: "пост-обработка",
        input: "дренажом",
        expected: "дрен+ажем",
    },
    GoldenCase {
        name: "транслитерация",
        input: "iPhone",
        expected: "ифон",
    },
    GoldenCase {
        name: "словарь автора",
        input: "яичница",
        expected: "я+ишница",
    },
    GoldenCase {
        name: "аббревиатуры",
        input: "МВД",
        expected: "эмвэд+э",
    },
    GoldenCase {
        name: "имена 75",
        input: "Абаян",
        expected: "Аба+ян",
    },
    GoldenCase {
        name: "фразы ruvoice",
        input: "моя команда",
        expected: "мо+я ком+анда",
    },
    GoldenCase {
        name: "маска ruvoice",
        input: "Ильинична",
        expected: "Иль+инишна",
    },
    GoldenCase {
        name: "имена ruvoice",
        input: "Апплеты",
        expected: "Аппл+еты",
    },
    GoldenCase {
        name: "числительные ruvoice",
        input: "трёхсот",
        expected: "трёхс+от",
    },
    GoldenCase {
        name: "основная орфоэпия",
        input: "человек",
        expected: "челов+ек",
    },
    GoldenCase {
        name: "твёрдое э",
        input: "тест",
        expected: "т+эст",
    },
    GoldenCase {
        name: "твёрдое э после ударения",
        input: "энергия",
        expected: "эн+эргия",
    },
    GoldenCase {
        name: "твёрдое э в маске",
        input: "бизнес-план",
        expected: "б+изнэс-пл+ан",
    },
    GoldenCase {
        name: "регрессия свои",
        input: "свои",
        expected: "сво+и",
    },
    GoldenCase {
        name: "регрессия знамения",
        input: "знамения века",
        expected: "знамения в+ека",
    },
    GoldenCase {
        name: "маска темно-",
        input: "Тёмно-синий шарф",
        expected: "Тёмно-с+иний ш+арф",
    },
];

#[derive(Debug)]
pub struct CheckReport {
    pub order: OrderReport,
    pub audit: AuditReport,
    pub loaded: usize,
    pub total_rules: usize,
    pub passed: usize,
}

impl CheckReport {
    pub fn success(&self) -> bool {
        self.order.failures.is_empty()
    }
}

pub fn run_check(dir: &Path) -> Result<CheckReport, String> {
    let paths = sorted_dictionary_paths(dir)?;
    let names: Vec<String> = paths
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("<unknown>")
                .to_string()
        })
        .collect();
    let mut order = validate_order(&names);
    let report = load_dictionaries_parallel(dir)?;
    let loaded = report.compiled.len();
    let total_rules: usize = report
        .compiled
        .iter()
        .map(|item| item.meta.rule_count)
        .sum();

    for error in report.errors {
        order.failures.push(format!("Словарь не загружен: {error}"));
    }
    for dictionary in &report.compiled {
        for warning in &dictionary.meta.warnings {
            order
                .warnings
                .push(format!("{}: {warning}", dictionary.meta.name));
        }
    }
    let audit = match audit_dictionaries(dir) {
        Ok(audit) => audit,
        Err(error) => {
            order
                .warnings
                .push(format!("Аудит фонетики не выполнен: {error}"));
            AuditReport::default()
        }
    };
    if !order.failures.is_empty() {
        return Ok(CheckReport {
            order,
            audit,
            loaded,
            total_rules,
            passed: 0,
        });
    }

    let processors: Vec<Arc<dyn TextProcessor>> = report
        .compiled
        .into_iter()
        .map(|dictionary| dictionary.processor)
        .collect();
    let pipeline = ProcessingPipeline::new(
        Arc::new(RussianStressFormatter),
        Arc::new(SentenceAwareChunker),
    );

    let mut passed = 0usize;
    for case in GOLDEN_CASES {
        let result = pipeline.execute(
            case.input,
            &processors,
            PipelineOptions {
                stress_format: StressFormat::PlusBeforeVowel,
                chunk_config: ChunkConfig::default(),
                capture_diff: false,
            },
            None,
            None,
        );
        match result {
            Some(result) if result.processed_text == case.expected => passed += 1,
            Some(result) => order.failures.push(format!(
                "{}: ожидалось «{}», получено «{}»",
                case.name, case.expected, result.processed_text
            )),
            None => order
                .failures
                .push(format!("{}: обработка отменена", case.name)),
        }
    }

    Ok(CheckReport {
        order,
        audit,
        loaded,
        total_rules,
        passed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn accepts_stage_order() {
        let report = validate_order(&names(&[
            "10_REX_числа.rex",
            "edge_Yo_.dic",
            "33_eyo_safe_дополнения.dic",
            "35_мой словарь.dic",
            "50_ОМОГРАФЫ 1 Омо.dic",
            "60_пост обработка@.dic",
            "67_TTS_Offline АВТОР(phoneme).dic",
            "69_TTS_Offline+УДАРЕНИЯ@(phoneme).dic",
            "76_ruvoice_фразы.dic",
            "77_ruvoice_имена.dic",
            "95.NEW_Орфоэпия(phoneme).dic",
            "96_Твёрдое Э(hard-e).dic",
        ]));

        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.ordered.first().unwrap().stage, "REX");
        assert_eq!(report.ordered.last().unwrap().stage, "HARD-E");
    }

    #[test]
    fn rejects_names_import_before_phrases_import() {
        let report = validate_order(&names(&[
            "10_REX_числа.rex",
            "edge_Yo_.dic",
            "33_eyo_safe_дополнения.dic",
            "67_TTS_Offline АВТОР(phoneme).dic",
            "77_ruvoice_имена.dic",
            "76_ruvoice_фразы.dic",
            "95.NEW_Орфоэпия(phoneme).dic",
            "96_Твёрдое Э(hard-e).dic",
        ]));

        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.contains("76_ruvoice_фразы.dic")),
            "{:?}",
            report.failures
        );
    }

    #[test]
    fn rejects_dictionary_without_number() {
        let report = validate_order(&names(&[
            "10_REX_числа.rex",
            "edge_Yo_.dic",
            "custom.dic",
            "95.NEW_Орфоэпия(phoneme).dic",
            "96_Твёрдое Э(hard-e).dic",
        ]));

        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.contains("custom.dic")),
            "{:?}",
            report.failures
        );
    }

    #[test]
    fn reports_unknown_stage_and_file_after_hard_e() {
        let report = validate_order(&names(&[
            "10_REX_числа.rex",
            "edge_Yo_.dic",
            "95.NEW_Орфоэпия(phoneme).dic",
            "96_Твёрдое Э(hard-e).dic",
            "97_custom.dic",
        ]));

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("97_custom.dic")),
            "{:?}",
            report.warnings
        );
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.contains("97_custom.dic")),
            "{:?}",
            report.failures
        );
    }

    #[test]
    fn allows_unknown_stage_before_hard_e_with_warning() {
        let report = validate_order(&names(&[
            "10_REX_числа.rex",
            "edge_Yo_.dic",
            "78_custom.dic",
            "95.NEW_Орфоэпия(phoneme).dic",
            "96_Твёрдое Э(hard-e).dic",
        ]));

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("78_custom.dic")),
            "{:?}",
            report.warnings
        );
        assert!(report.failures.is_empty(), "{:?}", report.failures);
    }

    #[test]
    #[ignore]
    fn checks_bundled_dictionaries() {
        let report = run_check(std::path::Path::new("dic")).expect("check bundled dictionaries");
        for warning in &report.order.warnings {
            eprintln!("warning: {warning}");
        }
        assert!(
            report.order.failures.is_empty(),
            "{:#?}",
            report.order.failures
        );
        assert_eq!(report.passed, GOLDEN_CASES.len());
    }
}

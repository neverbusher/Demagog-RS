use super::DictionaryTag;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DictionaryStage {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictionaryPurpose {
    YoFication,
    AllVsYoStars,
    AllVsYo,
    HomographsStars,
    HomographsPhrases,
    PostProcessing,
    Transliteration,
    Author,
    Converters,
    StressWords,
    Abbreviations,
    Profanity,
    Names,
    PhrasesImport,
    Base,
    HardE,
    User,
    Other,
}

impl DictionaryPurpose {
    pub fn expected_by_design(self) -> bool {
        matches!(
            self,
            Self::YoFication
                | Self::Base
                | Self::HomographsStars
                | Self::AllVsYoStars
                | Self::Transliteration
                | Self::Converters
                | Self::Abbreviations
                | Self::Profanity
                | Self::Names
                | Self::HardE
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorFlavor {
    HardE,
    Rex,
    Dic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DictionaryProfile {
    pub stage: DictionaryStage,
    pub purpose: DictionaryPurpose,
    pub tag: DictionaryTag,
    pub description: Option<&'static str>,
    pub outputs_plus_stress: Option<bool>,
    pub processor: ProcessorFlavor,
    pub number: Option<u32>,
    pub has_hard_marker: bool,
}

impl DictionaryProfile {
    pub fn sort_key(&self, name: &str) -> (u8, u32, String) {
        let lower = name.to_lowercase();
        if self.stage == DictionaryStage::Rex {
            (0, self.number.unwrap_or(500), lower)
        } else if self.number.is_none() && lower.contains("yo") {
            (1, 32, lower)
        } else if let Some(number) = self.number {
            (2, number, lower)
        } else {
            (3, 999, lower)
        }
    }
}

fn leading_number(name: &str) -> Option<u32> {
    name.chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

fn has_rex_extension(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("rex"))
}

fn classify_stage(lower: &str, number: Option<u32>) -> DictionaryStage {
    if lower.ends_with(".rex") {
        DictionaryStage::Rex
    } else if let Some(number) = number {
        match number {
            33 => DictionaryStage::YoAdditions,
            35 => DictionaryStage::User,
            40..=42 | 50..=53 => DictionaryStage::Homographs,
            60 | 65 => DictionaryStage::Cleanup,
            67 => DictionaryStage::Author,
            69 | 70 => DictionaryStage::Converters,
            71..=75 => DictionaryStage::StressWords,
            76 => DictionaryStage::PhrasesImport,
            77 => DictionaryStage::NamesImport,
            95 => DictionaryStage::Orthoepy,
            96 => DictionaryStage::HardE,
            _ => DictionaryStage::Unknown,
        }
    } else if lower.contains("yo") {
        DictionaryStage::YoBase
    } else {
        DictionaryStage::Unnumbered
    }
}

fn classify_purpose(lower: &str) -> DictionaryPurpose {
    if lower.contains("edge_yo") || lower.starts_with("33_") {
        DictionaryPurpose::YoFication
    } else if lower.starts_with("41_1")
        || lower.starts_with("41_2")
        || lower.starts_with("42_1")
        || lower.starts_with("42_2")
    {
        DictionaryPurpose::AllVsYoStars
    } else if lower.starts_with("41_") || lower.starts_with("42_") {
        DictionaryPurpose::AllVsYo
    } else if lower.starts_with("40.") {
        DictionaryPurpose::HomographsStars
    } else if lower.starts_with("50_")
        || lower.starts_with("51_")
        || lower.starts_with("52_")
        || lower.starts_with("53_")
    {
        DictionaryPurpose::HomographsPhrases
    } else if lower.starts_with("60_") {
        DictionaryPurpose::PostProcessing
    } else if lower.starts_with("65_") {
        DictionaryPurpose::Transliteration
    } else if lower.starts_with("67_") {
        DictionaryPurpose::Author
    } else if lower.starts_with("69_") || lower.starts_with("70_") {
        DictionaryPurpose::Converters
    } else if lower.starts_with("71_") || lower.starts_with("72_") {
        DictionaryPurpose::StressWords
    } else if lower.starts_with("73_") {
        DictionaryPurpose::Abbreviations
    } else if lower.starts_with("74_") {
        DictionaryPurpose::Profanity
    } else if lower.starts_with("75_") || lower.starts_with("77_") {
        DictionaryPurpose::Names
    } else if lower.starts_with("76_") {
        DictionaryPurpose::PhrasesImport
    } else if lower.starts_with("95.") {
        DictionaryPurpose::Base
    } else if lower.contains("(hard-e)") || lower.starts_with("96_") {
        DictionaryPurpose::HardE
    } else if lower.starts_with("35_") {
        DictionaryPurpose::User
    } else {
        DictionaryPurpose::Other
    }
}

fn classify_metadata(lower: &str) -> (DictionaryTag, Option<&'static str>, Option<bool>) {
    if lower.contains("(hard-e)") {
        (
            DictionaryTag::HardPronunciation,
            Some("Твёрдое «э» в заимствованиях: тест → тэст (1.5 тыс. правил)"),
            Some(false),
        )
    } else if lower.starts_with("10_") || lower.contains("числа") || lower.contains("chisla") {
        (
            DictionaryTag::NumbersAndDates,
            Some("Переводит числа, даты, римские цифры и валюты в текст словами"),
            Some(false),
        )
    } else if lower.starts_with("15_") {
        (
            DictionaryTag::YoFication,
            Some("Контекстный выбор «все» или «всё» по регулярным выражениям"),
            Some(false),
        )
    } else if lower.starts_with("20_") {
        (
            DictionaryTag::HomographsCap,
            Some("Индексный словарь омографов от waska: ставит ЗАГЛАВНУЮ ударную (зАмок/замОк)"),
            Some(false),
        )
    } else if lower.starts_with("25_") {
        (
            DictionaryTag::HomographsCap,
            Some("Контекстные ударения в именах и фамилиях (через заглавную гласную)"),
            Some(false),
        )
    } else if lower.starts_with("30_") {
        (
            DictionaryTag::CleanupAndTranslit,
            Some("Чистка лишних пробелов перед знаками препинания"),
            Some(false),
        )
    } else if lower.contains("edge_yo") || lower == "yo.dic" {
        (
            DictionaryTag::YoFication,
            Some("Словарь Ё-фикации (337 тыс. слов): заменяет «е» на «ё» без знаков «+»"),
            Some(false),
        )
    } else if lower.starts_with("33_") || lower.contains("eyo_safe") {
        (
            DictionaryTag::YoFication,
            Some(
                "Дополнения ё-фикации из eyo-kernel: 2.4 тыс. форм, которых нет в словарях Demagog-RS",
            ),
            Some(false),
        )
    } else if lower.starts_with("35_") {
        (
            DictionaryTag::PlusStressWords,
            Some("Пользовательский словарь собственных слов со знаком ударения «+»"),
            Some(true),
        )
    } else if lower.starts_with("40.1") {
        (
            DictionaryTag::HomographsCap,
            Some("Омографы по окончаниям со звёздочкой (*): ставит ЗАГЛАВНУЮ ударную"),
            Some(false),
        )
    } else if lower.starts_with("40.2") {
        (
            DictionaryTag::HomographsCap,
            Some("Омографы во фразах с пропущенным словом (*): ставит ЗАГЛАВНУЮ ударную"),
            Some(false),
        )
    } else if lower.starts_with("41_") {
        (
            DictionaryTag::YoFication,
            Some("Устойчивые фразы и шаблоны: уточнение «все» -> «всё»"),
            Some(false),
        )
    } else if lower.starts_with("42_") {
        (
            DictionaryTag::YoFication,
            Some("Обратная коррекция: исправляет ошибочное «всё» на «все» во фразах"),
            Some(false),
        )
    } else if lower.starts_with("50_")
        || lower.starts_with("51_")
        || lower.starts_with("52_")
        || lower.starts_with("53_")
    {
        (
            DictionaryTag::HomographsCap,
            Some("Словарь словосочетаний для омографов: ставит ЗАГЛАВНУЮ ударную (зЕмли)"),
            Some(false),
        )
    } else if lower.starts_with("60_") {
        (
            DictionaryTag::CleanupAndTranslit,
            Some("Пост-обработка: расстановка запятых и фонетическая правка сложных слов"),
            Some(false),
        )
    } else if lower.starts_with("65_") {
        (
            DictionaryTag::CleanupAndTranslit,
            Some("Транслитерация английских/латинских слов и корней русскими буквами"),
            Some(false),
        )
    } else if lower.starts_with("67_") {
        (
            DictionaryTag::PlusStressWords,
            Some("Большой словарь фраз и слов TTS_Offline: ставит знак ударения «+»"),
            Some(true),
        )
    } else if lower.starts_with("69_") {
        (
            DictionaryTag::CapToPlusConverter,
            Some("Конвертер: переводит заглавные ударные из омографов (зЕмли) в знак «+» (з+емли)"),
            Some(true),
        )
    } else if lower.starts_with("70_") {
        (
            DictionaryTag::CapToPlusConverter,
            Some(
                "Конвертер: ставит «+» перед первой заглавной буквой омографов ($Адреса -> +Адреса)",
            ),
            Some(true),
        )
    } else if lower.starts_with("71_") || lower.starts_with("72_") {
        (
            DictionaryTag::PlusStressWords,
            Some("Точечные правки ударений со знаком «+» в отдельных словах и корнях"),
            Some(true),
        )
    } else if lower.starts_with("73_") {
        (
            DictionaryTag::PlusStressWords,
            Some("Расшифровка аббревиатур (МВД, КГБ, США и др.) с ударениями «+»"),
            Some(true),
        )
    } else if lower.starts_with("74_") {
        (
            DictionaryTag::PlusStressWords,
            Some("Раскрытие запиканного точками мата (*...здят) с ударениями «+»"),
            Some(true),
        )
    } else if lower.starts_with("75_") {
        (
            DictionaryTag::PlusStressWords,
            Some("Словарь имён собственных, городов и фамилий со знаком ударения «+»"),
            Some(true),
        )
    } else if lower.starts_with("76_") {
        (
            DictionaryTag::PlusStressWords,
            Some(
                "Контекстные ударения из ruvoice-tts: фразы, маски и междометия (11.8 тыс. правил)",
            ),
            Some(true),
        )
    } else if lower.starts_with("77_") {
        (
            DictionaryTag::PlusStressWords,
            Some("Имена, топонимы и поправки ударений из ruvoice-tts (96.7 тыс. слов)"),
            Some(true),
        )
    } else if lower.starts_with("95.") {
        (
            DictionaryTag::PlusStressOrthoepy,
            Some("Главный орфоэпический словарь (2.9 млн форм): ставит «+» в КАЖДОМ обычном слове"),
            Some(true),
        )
    } else if lower.contains("(phoneme)") {
        (
            DictionaryTag::PlusStressWords,
            Some("Словарь со знаком ударения «+» перед гласной"),
            Some(true),
        )
    } else {
        (DictionaryTag::CleanupAndTranslit, None, None)
    }
}

pub fn classify_dictionary(name: &str) -> DictionaryProfile {
    let lower = name.to_lowercase();
    let number = leading_number(name);
    let stage = classify_stage(&lower, number);
    let purpose = classify_purpose(&lower);
    let (tag, description, outputs_plus_stress) = classify_metadata(&lower);
    let has_hard_marker = lower.contains("(hard-e)");
    let processor = if has_hard_marker {
        ProcessorFlavor::HardE
    } else if has_rex_extension(name) {
        ProcessorFlavor::Rex
    } else {
        ProcessorFlavor::Dic
    };

    DictionaryProfile {
        stage,
        purpose,
        tag,
        description,
        outputs_plus_stress,
        processor,
        number,
        has_hard_marker,
    }
}

pub fn describe_dictionary(name: &str, content: &str) -> (DictionaryTag, String, bool) {
    let profile = classify_dictionary(name);
    if let (Some(description), Some(outputs_plus_stress)) =
        (profile.description, profile.outputs_plus_stress)
    {
        return (profile.tag, description.to_string(), outputs_plus_stress);
    }

    let has_plus_in_rules = content
        .lines()
        .take(200)
        .filter_map(|line| line.split_once('='))
        .any(|(_, rhs)| rhs.contains('+'));
    if has_plus_in_rules {
        (
            DictionaryTag::PlusStressWords,
            "Словарь со знаком ударения «+» перед гласной".to_string(),
            true,
        )
    } else {
        (
            DictionaryTag::CleanupAndTranslit,
            "Текстовый словарь замен".to_string(),
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        DictionaryProfile, DictionaryPurpose, DictionaryStage, DictionaryTag, ProcessorFlavor,
        classify_dictionary, describe_dictionary,
    };

    #[test]
    fn purpose_design_expectations_match_the_audit() {
        let cases = [
            (DictionaryPurpose::YoFication, true),
            (DictionaryPurpose::AllVsYoStars, true),
            (DictionaryPurpose::AllVsYo, false),
            (DictionaryPurpose::HomographsStars, true),
            (DictionaryPurpose::HomographsPhrases, false),
            (DictionaryPurpose::PostProcessing, false),
            (DictionaryPurpose::Transliteration, true),
            (DictionaryPurpose::Author, false),
            (DictionaryPurpose::Converters, true),
            (DictionaryPurpose::StressWords, false),
            (DictionaryPurpose::Abbreviations, true),
            (DictionaryPurpose::Profanity, true),
            (DictionaryPurpose::Names, true),
            (DictionaryPurpose::PhrasesImport, false),
            (DictionaryPurpose::Base, true),
            (DictionaryPurpose::HardE, true),
            (DictionaryPurpose::User, false),
            (DictionaryPurpose::Other, false),
        ];

        for (purpose, expected_by_design) in cases {
            assert_eq!(
                purpose.expected_by_design(),
                expected_by_design,
                "{purpose:?}"
            );
        }
    }

    #[test]
    fn classifies_every_dictionary_branch() {
        let cases = [
            (
                "10_REX_числа.rex",
                DictionaryProfile {
                    stage: DictionaryStage::Rex,
                    purpose: DictionaryPurpose::Other,
                    tag: DictionaryTag::NumbersAndDates,
                    description: Some(
                        "Переводит числа, даты, римские цифры и валюты в текст словами",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Rex,
                    number: Some(10),
                    has_hard_marker: false,
                },
            ),
            (
                "edge_Yo_.dic",
                DictionaryProfile {
                    stage: DictionaryStage::YoBase,
                    purpose: DictionaryPurpose::YoFication,
                    tag: DictionaryTag::YoFication,
                    description: Some(
                        "Словарь Ё-фикации (337 тыс. слов): заменяет «е» на «ё» без знаков «+»",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: None,
                    has_hard_marker: false,
                },
            ),
            (
                "yo.dic",
                DictionaryProfile {
                    stage: DictionaryStage::YoBase,
                    purpose: DictionaryPurpose::Other,
                    tag: DictionaryTag::YoFication,
                    description: Some(
                        "Словарь Ё-фикации (337 тыс. слов): заменяет «е» на «ё» без знаков «+»",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: None,
                    has_hard_marker: false,
                },
            ),
            (
                "33_eyo_safe_дополнения.dic",
                DictionaryProfile {
                    stage: DictionaryStage::YoAdditions,
                    purpose: DictionaryPurpose::YoFication,
                    tag: DictionaryTag::YoFication,
                    description: Some(
                        "Дополнения ё-фикации из eyo-kernel: 2.4 тыс. форм, которых нет в словарях Demagog-RS",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: Some(33),
                    has_hard_marker: false,
                },
            ),
            (
                "35_мой словарь.dic",
                DictionaryProfile {
                    stage: DictionaryStage::User,
                    purpose: DictionaryPurpose::User,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some(
                        "Пользовательский словарь собственных слов со знаком ударения «+»",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(35),
                    has_hard_marker: false,
                },
            ),
            (
                "40.1_окончания.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Homographs,
                    purpose: DictionaryPurpose::HomographsStars,
                    tag: DictionaryTag::HomographsCap,
                    description: Some(
                        "Омографы по окончаниям со звёздочкой (*): ставит ЗАГЛАВНУЮ ударную",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: Some(40),
                    has_hard_marker: false,
                },
            ),
            (
                "40.2_фразы.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Homographs,
                    purpose: DictionaryPurpose::HomographsStars,
                    tag: DictionaryTag::HomographsCap,
                    description: Some(
                        "Омографы во фразах с пропущенным словом (*): ставит ЗАГЛАВНУЮ ударную",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: Some(40),
                    has_hard_marker: false,
                },
            ),
            (
                "41_1_звёзды.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Homographs,
                    purpose: DictionaryPurpose::AllVsYoStars,
                    tag: DictionaryTag::YoFication,
                    description: Some("Устойчивые фразы и шаблоны: уточнение «все» -> «всё»"),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: Some(41),
                    has_hard_marker: false,
                },
            ),
            (
                "41_3_фразы.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Homographs,
                    purpose: DictionaryPurpose::AllVsYo,
                    tag: DictionaryTag::YoFication,
                    description: Some("Устойчивые фразы и шаблоны: уточнение «все» -> «всё»"),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: Some(41),
                    has_hard_marker: false,
                },
            ),
            (
                "42_2_звёзды.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Homographs,
                    purpose: DictionaryPurpose::AllVsYoStars,
                    tag: DictionaryTag::YoFication,
                    description: Some(
                        "Обратная коррекция: исправляет ошибочное «всё» на «все» во фразах",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: Some(42),
                    has_hard_marker: false,
                },
            ),
            (
                "50_омографы.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Homographs,
                    purpose: DictionaryPurpose::HomographsPhrases,
                    tag: DictionaryTag::HomographsCap,
                    description: Some(
                        "Словарь словосочетаний для омографов: ставит ЗАГЛАВНУЮ ударную (зЕмли)",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: Some(50),
                    has_hard_marker: false,
                },
            ),
            (
                "60_пост обработка@.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Cleanup,
                    purpose: DictionaryPurpose::PostProcessing,
                    tag: DictionaryTag::CleanupAndTranslit,
                    description: Some(
                        "Пост-обработка: расстановка запятых и фонетическая правка сложных слов",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: Some(60),
                    has_hard_marker: false,
                },
            ),
            (
                "65_ЛАТИНИЦА@.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Cleanup,
                    purpose: DictionaryPurpose::Transliteration,
                    tag: DictionaryTag::CleanupAndTranslit,
                    description: Some(
                        "Транслитерация английских/латинских слов и корней русскими буквами",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::Dic,
                    number: Some(65),
                    has_hard_marker: false,
                },
            ),
            (
                "67_TTS_Offline АВТОР(phoneme).dic",
                DictionaryProfile {
                    stage: DictionaryStage::Author,
                    purpose: DictionaryPurpose::Author,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some(
                        "Большой словарь фраз и слов TTS_Offline: ставит знак ударения «+»",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(67),
                    has_hard_marker: false,
                },
            ),
            (
                "69_TTS_Offline+УДАРЕНИЯ@(phoneme).dic",
                DictionaryProfile {
                    stage: DictionaryStage::Converters,
                    purpose: DictionaryPurpose::Converters,
                    tag: DictionaryTag::CapToPlusConverter,
                    description: Some(
                        "Конвертер: переводит заглавные ударные из омографов (зЕмли) в знак «+» (з+емли)",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(69),
                    has_hard_marker: false,
                },
            ),
            (
                "70_конвертер.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Converters,
                    purpose: DictionaryPurpose::Converters,
                    tag: DictionaryTag::CapToPlusConverter,
                    description: Some(
                        "Конвертер: ставит «+» перед первой заглавной буквой омографов ($Адреса -> +Адреса)",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(70),
                    has_hard_marker: false,
                },
            ),
            (
                "71_ударения.dic",
                DictionaryProfile {
                    stage: DictionaryStage::StressWords,
                    purpose: DictionaryPurpose::StressWords,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some(
                        "Точечные правки ударений со знаком «+» в отдельных словах и корнях",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(71),
                    has_hard_marker: false,
                },
            ),
            (
                "72_ударения.dic",
                DictionaryProfile {
                    stage: DictionaryStage::StressWords,
                    purpose: DictionaryPurpose::StressWords,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some(
                        "Точечные правки ударений со знаком «+» в отдельных словах и корнях",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(72),
                    has_hard_marker: false,
                },
            ),
            (
                "73_аббревиатуры.dic",
                DictionaryProfile {
                    stage: DictionaryStage::StressWords,
                    purpose: DictionaryPurpose::Abbreviations,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some(
                        "Расшифровка аббревиатур (МВД, КГБ, США и др.) с ударениями «+»",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(73),
                    has_hard_marker: false,
                },
            ),
            (
                "74_мат.dic",
                DictionaryProfile {
                    stage: DictionaryStage::StressWords,
                    purpose: DictionaryPurpose::Profanity,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some(
                        "Раскрытие запиканного точками мата (*...здят) с ударениями «+»",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(74),
                    has_hard_marker: false,
                },
            ),
            (
                "75_имена.dic",
                DictionaryProfile {
                    stage: DictionaryStage::StressWords,
                    purpose: DictionaryPurpose::Names,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some(
                        "Словарь имён собственных, городов и фамилий со знаком ударения «+»",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(75),
                    has_hard_marker: false,
                },
            ),
            (
                "76_ruvoice_фразы.dic",
                DictionaryProfile {
                    stage: DictionaryStage::PhrasesImport,
                    purpose: DictionaryPurpose::PhrasesImport,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some(
                        "Контекстные ударения из ruvoice-tts: фразы, маски и междометия (11.8 тыс. правил)",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(76),
                    has_hard_marker: false,
                },
            ),
            (
                "77_ruvoice_имена.dic",
                DictionaryProfile {
                    stage: DictionaryStage::NamesImport,
                    purpose: DictionaryPurpose::Names,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some(
                        "Имена, топонимы и поправки ударений из ruvoice-tts (96.7 тыс. слов)",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(77),
                    has_hard_marker: false,
                },
            ),
            (
                "95.NEW_Орфоэпия(phoneme).dic",
                DictionaryProfile {
                    stage: DictionaryStage::Orthoepy,
                    purpose: DictionaryPurpose::Base,
                    tag: DictionaryTag::PlusStressOrthoepy,
                    description: Some(
                        "Главный орфоэпический словарь (2.9 млн форм): ставит «+» в КАЖДОМ обычном слове",
                    ),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: Some(95),
                    has_hard_marker: false,
                },
            ),
            (
                "96_Твёрдое Э(hard-e).dic",
                DictionaryProfile {
                    stage: DictionaryStage::HardE,
                    purpose: DictionaryPurpose::HardE,
                    tag: DictionaryTag::HardPronunciation,
                    description: Some(
                        "Твёрдое «э» в заимствованиях: тест → тэст (1.5 тыс. правил)",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::HardE,
                    number: Some(96),
                    has_hard_marker: true,
                },
            ),
            (
                "96_без маркера.dic",
                DictionaryProfile {
                    stage: DictionaryStage::HardE,
                    purpose: DictionaryPurpose::HardE,
                    tag: DictionaryTag::CleanupAndTranslit,
                    description: None,
                    outputs_plus_stress: None,
                    processor: ProcessorFlavor::Dic,
                    number: Some(96),
                    has_hard_marker: false,
                },
            ),
            (
                ".rex",
                DictionaryProfile {
                    stage: DictionaryStage::Rex,
                    purpose: DictionaryPurpose::Other,
                    tag: DictionaryTag::CleanupAndTranslit,
                    description: None,
                    outputs_plus_stress: None,
                    processor: ProcessorFlavor::Dic,
                    number: None,
                    has_hard_marker: false,
                },
            ),
            (
                "custom(phoneme).dic",
                DictionaryProfile {
                    stage: DictionaryStage::Unnumbered,
                    purpose: DictionaryPurpose::Other,
                    tag: DictionaryTag::PlusStressWords,
                    description: Some("Словарь со знаком ударения «+» перед гласной"),
                    outputs_plus_stress: Some(true),
                    processor: ProcessorFlavor::Dic,
                    number: None,
                    has_hard_marker: false,
                },
            ),
            (
                "97_custom.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Unknown,
                    purpose: DictionaryPurpose::Other,
                    tag: DictionaryTag::CleanupAndTranslit,
                    description: None,
                    outputs_plus_stress: None,
                    processor: ProcessorFlavor::Dic,
                    number: Some(97),
                    has_hard_marker: false,
                },
            ),
            (
                "custom.dic",
                DictionaryProfile {
                    stage: DictionaryStage::Unnumbered,
                    purpose: DictionaryPurpose::Other,
                    tag: DictionaryTag::CleanupAndTranslit,
                    description: None,
                    outputs_plus_stress: None,
                    processor: ProcessorFlavor::Dic,
                    number: None,
                    has_hard_marker: false,
                },
            ),
            (
                "custom_yo.dic",
                DictionaryProfile {
                    stage: DictionaryStage::YoBase,
                    purpose: DictionaryPurpose::Other,
                    tag: DictionaryTag::CleanupAndTranslit,
                    description: None,
                    outputs_plus_stress: None,
                    processor: ProcessorFlavor::Dic,
                    number: None,
                    has_hard_marker: false,
                },
            ),
            (
                "97_Твёрдое Э(hard-e).rex",
                DictionaryProfile {
                    stage: DictionaryStage::Rex,
                    purpose: DictionaryPurpose::HardE,
                    tag: DictionaryTag::HardPronunciation,
                    description: Some(
                        "Твёрдое «э» в заимствованиях: тест → тэст (1.5 тыс. правил)",
                    ),
                    outputs_plus_stress: Some(false),
                    processor: ProcessorFlavor::HardE,
                    number: Some(97),
                    has_hard_marker: true,
                },
            ),
        ];

        for (name, expected) in cases {
            assert_eq!(classify_dictionary(name), expected, "{name}");
            if let (Some(description), Some(plus)) =
                (expected.description, expected.outputs_plus_stress)
            {
                for content in ["", "а=б+в"] {
                    assert_eq!(
                        describe_dictionary(name, content),
                        (expected.tag, description.to_string(), plus),
                        "{name} with content {content:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn hard_marker_overrides_processor_but_not_stage_or_purpose() {
        let cases = [
            (
                "33_Твёрдое Э(hard-e).dic",
                DictionaryStage::YoAdditions,
                DictionaryPurpose::YoFication,
                Some(33),
            ),
            (
                "35_Твёрдое Э(hard-e).dic",
                DictionaryStage::User,
                DictionaryPurpose::HardE,
                Some(35),
            ),
        ];

        for (name, stage, purpose, number) in cases {
            let profile = classify_dictionary(name);
            assert_eq!(profile.stage, stage, "{name}");
            assert_eq!(profile.purpose, purpose, "{name}");
            assert_eq!(profile.processor, ProcessorFlavor::HardE, "{name}");
            assert_eq!(profile.number, number, "{name}");
            assert!(profile.has_hard_marker, "{name}");
            assert_eq!(profile.tag, DictionaryTag::HardPronunciation, "{name}");
            assert_eq!(
                describe_dictionary(name, "а=б+в"),
                (
                    DictionaryTag::HardPronunciation,
                    "Твёрдое «э» в заимствованиях: тест → тэст (1.5 тыс. правил)".to_string(),
                    false,
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn classifies_remaining_stage_and_metadata_branches() {
        let cases = [
            (
                "15_все или всё.dic",
                DictionaryStage::Unknown,
                DictionaryPurpose::Other,
                DictionaryTag::YoFication,
                Some("Контекстный выбор «все» или «всё» по регулярным выражениям"),
                Some(false),
                Some(15),
            ),
            (
                "20_омографы.dic",
                DictionaryStage::Unknown,
                DictionaryPurpose::Other,
                DictionaryTag::HomographsCap,
                Some(
                    "Индексный словарь омографов от waska: ставит ЗАГЛАВНУЮ ударную (зАмок/замОк)",
                ),
                Some(false),
                Some(20),
            ),
            (
                "25_имена.dic",
                DictionaryStage::Unknown,
                DictionaryPurpose::Other,
                DictionaryTag::HomographsCap,
                Some("Контекстные ударения в именах и фамилиях (через заглавную гласную)"),
                Some(false),
                Some(25),
            ),
            (
                "30_чистка.dic",
                DictionaryStage::Unknown,
                DictionaryPurpose::Other,
                DictionaryTag::CleanupAndTranslit,
                Some("Чистка лишних пробелов перед знаками препинания"),
                Some(false),
                Some(30),
            ),
            (
                "51_омографы.dic",
                DictionaryStage::Homographs,
                DictionaryPurpose::HomographsPhrases,
                DictionaryTag::HomographsCap,
                Some("Словарь словосочетаний для омографов: ставит ЗАГЛАВНУЮ ударную (зЕмли)"),
                Some(false),
                Some(51),
            ),
            (
                "52_омографы.dic",
                DictionaryStage::Homographs,
                DictionaryPurpose::HomographsPhrases,
                DictionaryTag::HomographsCap,
                Some("Словарь словосочетаний для омографов: ставит ЗАГЛАВНУЮ ударную (зЕмли)"),
                Some(false),
                Some(52),
            ),
            (
                "53_омографы.dic",
                DictionaryStage::Homographs,
                DictionaryPurpose::HomographsPhrases,
                DictionaryTag::HomographsCap,
                Some("Словарь словосочетаний для омографов: ставит ЗАГЛАВНУЮ ударную (зЕмли)"),
                Some(false),
                Some(53),
            ),
            (
                "41_2_звёзды.dic",
                DictionaryStage::Homographs,
                DictionaryPurpose::AllVsYoStars,
                DictionaryTag::YoFication,
                Some("Устойчивые фразы и шаблоны: уточнение «все» -> «всё»"),
                Some(false),
                Some(41),
            ),
            (
                "42_1_звёзды.dic",
                DictionaryStage::Homographs,
                DictionaryPurpose::AllVsYoStars,
                DictionaryTag::YoFication,
                Some("Обратная коррекция: исправляет ошибочное «всё» на «все» во фразах"),
                Some(false),
                Some(42),
            ),
            (
                "42_3_фразы.dic",
                DictionaryStage::Homographs,
                DictionaryPurpose::AllVsYo,
                DictionaryTag::YoFication,
                Some("Обратная коррекция: исправляет ошибочное «всё» на «все» во фразах"),
                Some(false),
                Some(42),
            ),
            (
                "40.3_омографы.dic",
                DictionaryStage::Homographs,
                DictionaryPurpose::HomographsStars,
                DictionaryTag::CleanupAndTranslit,
                None,
                None,
                Some(40),
            ),
            (
                "95_орфоэпия.dic",
                DictionaryStage::Orthoepy,
                DictionaryPurpose::Other,
                DictionaryTag::CleanupAndTranslit,
                None,
                None,
                Some(95),
            ),
            (
                "96без подчёркивания.dic",
                DictionaryStage::HardE,
                DictionaryPurpose::Other,
                DictionaryTag::CleanupAndTranslit,
                None,
                None,
                Some(96),
            ),
            (
                "custom_eyo_safe.dic",
                DictionaryStage::YoBase,
                DictionaryPurpose::Other,
                DictionaryTag::YoFication,
                Some(
                    "Дополнения ё-фикации из eyo-kernel: 2.4 тыс. форм, которых нет в словарях Demagog-RS",
                ),
                Some(false),
                None,
            ),
            (
                "заметки_числа.dic",
                DictionaryStage::Unnumbered,
                DictionaryPurpose::Other,
                DictionaryTag::NumbersAndDates,
                Some("Переводит числа, даты, римские цифры и валюты в текст словами"),
                Some(false),
                None,
            ),
            (
                "chisla.dic",
                DictionaryStage::Unnumbered,
                DictionaryPurpose::Other,
                DictionaryTag::NumbersAndDates,
                Some("Переводит числа, даты, римские цифры и валюты в текст словами"),
                Some(false),
                None,
            ),
        ];

        for (name, stage, purpose, tag, description, outputs_plus_stress, number) in cases {
            let profile = classify_dictionary(name);
            assert_eq!(profile.stage, stage, "{name}");
            assert_eq!(profile.purpose, purpose, "{name}");
            assert_eq!(profile.tag, tag, "{name}");
            assert_eq!(profile.description, description, "{name}");
            assert_eq!(profile.outputs_plus_stress, outputs_plus_stress, "{name}");
            assert_eq!(profile.number, number, "{name}");
            assert_eq!(profile.processor, ProcessorFlavor::Dic, "{name}");

            if let (Some(expected_description), Some(expected_plus)) =
                (description, outputs_plus_stress)
            {
                assert_eq!(
                    describe_dictionary(name, ""),
                    (tag, expected_description.to_string(), expected_plus),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn preserves_loader_sort_keys() {
        let cases = [
            ("10_REX_числа.rex", (0, 10, "10_rex_числа.rex")),
            ("numbers.rex", (0, 500, "numbers.rex")),
            (".rex", (0, 500, ".rex")),
            ("edge_Yo_.dic", (1, 32, "edge_yo_.dic")),
            ("custom_yo.dic", (1, 32, "custom_yo.dic")),
            (
                "33_eyo_safe_дополнения.dic",
                (2, 33, "33_eyo_safe_дополнения.dic"),
            ),
            ("95.NEW_Орфоэпия.dic", (2, 95, "95.new_орфоэпия.dic")),
            ("97_custom.dic", (2, 97, "97_custom.dic")),
            (
                "97_Твёрдое Э(hard-e).rex",
                (0, 97, "97_твёрдое э(hard-e).rex"),
            ),
            ("numbers.REX", (0, 500, "numbers.rex")),
            ("custom.dic", (3, 999, "custom.dic")),
            (
                "999999999999999999999_yo.dic",
                (1, 32, "999999999999999999999_yo.dic"),
            ),
        ];

        for (name, (group, number, lower_name)) in cases {
            assert_eq!(
                classify_dictionary(name).sort_key(name),
                (group, number, lower_name.to_string()),
                "{name}"
            );
        }
    }

    #[test]
    fn describes_unknown_dictionaries_using_only_the_fallback_content_rule() {
        let cases = [
            (
                "custom.dic",
                "а=б",
                DictionaryTag::CleanupAndTranslit,
                "Текстовый словарь замен",
                false,
            ),
            (
                "custom.dic",
                "а=б+в",
                DictionaryTag::PlusStressWords,
                "Словарь со знаком ударения «+» перед гласной",
                true,
            ),
            (
                "97_custom.dic",
                "а=б",
                DictionaryTag::CleanupAndTranslit,
                "Текстовый словарь замен",
                false,
            ),
            (
                "97_custom.dic",
                "а=б+в",
                DictionaryTag::PlusStressWords,
                "Словарь со знаком ударения «+» перед гласной",
                true,
            ),
            (
                "96_без маркера.dic",
                "а=б",
                DictionaryTag::CleanupAndTranslit,
                "Текстовый словарь замен",
                false,
            ),
            (
                "96_без маркера.dic",
                "а=б+в",
                DictionaryTag::PlusStressWords,
                "Словарь со знаком ударения «+» перед гласной",
                true,
            ),
            (
                "custom_yo.dic",
                "а=б",
                DictionaryTag::CleanupAndTranslit,
                "Текстовый словарь замен",
                false,
            ),
            (
                "custom_yo.dic",
                "а=б+в",
                DictionaryTag::PlusStressWords,
                "Словарь со знаком ударения «+» перед гласной",
                true,
            ),
            (
                "10_REX_числа.rex",
                "а=б+в",
                DictionaryTag::NumbersAndDates,
                "Переводит числа, даты, римские цифры и валюты в текст словами",
                false,
            ),
            (
                "96_Твёрдое Э(hard-e).dic",
                "а=б+в",
                DictionaryTag::HardPronunciation,
                "Твёрдое «э» в заимствованиях: тест → тэст (1.5 тыс. правил)",
                false,
            ),
        ];

        for (name, content, expected_tag, expected_description, expected_plus) in cases {
            assert_eq!(
                describe_dictionary(name, content),
                (
                    expected_tag,
                    expected_description.to_string(),
                    expected_plus
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn plus_fallback_scans_only_the_first_200_rule_lines() {
        let mut within_limit = "а=б\n".repeat(199);
        within_limit.push_str("последнее=сл+ово\n");
        assert_eq!(
            describe_dictionary("custom.dic", &within_limit),
            (
                DictionaryTag::PlusStressWords,
                "Словарь со знаком ударения «+» перед гласной".to_string(),
                true,
            )
        );

        let mut beyond_limit = "а=б\n".repeat(200);
        beyond_limit.push_str("последнее=сл+ово\n");
        assert_eq!(
            describe_dictionary("custom.dic", &beyond_limit),
            (
                DictionaryTag::CleanupAndTranslit,
                "Текстовый словарь замен".to_string(),
                false,
            )
        );
    }

    #[test]
    fn phoneme_marker_keeps_the_existing_plus_fallback() {
        assert_eq!(
            describe_dictionary("custom(phoneme).dic", "а=б"),
            (
                DictionaryTag::PlusStressWords,
                "Словарь со знаком ударения «+» перед гласной".to_string(),
                true,
            )
        );
    }

    #[test]
    fn preserves_imported_dictionary_descriptions() {
        let cases = [
            (
                "33_eyo_safe_дополнения.dic",
                DictionaryTag::YoFication,
                "Дополнения ё-фикации из eyo-kernel: 2.4 тыс. форм, которых нет в словарях Demagog-RS",
                false,
            ),
            (
                "76_ruvoice_фразы.dic",
                DictionaryTag::PlusStressWords,
                "Контекстные ударения из ruvoice-tts: фразы, маски и междометия (11.8 тыс. правил)",
                true,
            ),
            (
                "77_ruvoice_имена.dic",
                DictionaryTag::PlusStressWords,
                "Имена, топонимы и поправки ударений из ruvoice-tts (96.7 тыс. слов)",
                true,
            ),
            (
                "96_Твёрдое Э(hard-e).dic",
                DictionaryTag::HardPronunciation,
                "Твёрдое «э» в заимствованиях: тест → тэст (1.5 тыс. правил)",
                false,
            ),
        ];

        for (name, expected_tag, expected_description, expected_plus) in cases {
            assert_eq!(
                describe_dictionary(name, "а=б"),
                (
                    expected_tag,
                    expected_description.to_string(),
                    expected_plus
                ),
                "{name}"
            );
        }
    }
}

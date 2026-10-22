use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    Warm,
}

impl ThemeMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Тёмная",
            Self::Light => "Светлая",
            Self::Warm => "Тёплая",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StressFormat {
    /// Знак '+' перед ударной гласной: з+амок, +Август (для TTS)
    PlusBeforeVowel,
    /// Комбинируемый знак ударения Юникода U+0301 после гласной: за́мок
    AcuteAccent,
    /// Чистый текст без '+' и внутренних заглавных, но с сохранением 'ё'
    CleanYoOnly,
}

impl StressFormat {
    pub fn label(&self) -> &'static str {
        match self {
            Self::PlusBeforeVowel => "+ перед гласной (з+амок)",
            Self::AcuteAccent => "Ударение Юникода (за́мок) [U+0301]",
            Self::CleanYoOnly => "Без ударений — только Ё (замок)",
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            Self::PlusBeforeVowel => "+ перед гласной",
            Self::AcuteAccent => "Ударение Юникода",
            Self::CleanYoOnly => "Только Ё",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkConfig {
    pub max_chars: usize,
    pub preserve_paragraphs: bool,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_chars: 600,
            preserve_paragraphs: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DictionaryKind {
    RegexStandard,
    RegexIndexed,
    DicStandard,
    DicStressConverter,
}

impl DictionaryKind {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::RegexStandard => "REX",
            Self::RegexIndexed => "REX-IDX",
            Self::DicStandard => "DIC",
            Self::DicStressConverter => "DIC-STRESS",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DictionaryTag {
    NumbersAndDates,
    YoFication,
    HomographsCap,
    CapToPlusConverter,
    PlusStressWords,
    PlusStressOrthoepy,
    CleanupAndTranslit,
}

impl DictionaryTag {
    /// Display order of tag groups in the sidebar.
    pub const ALL: [Self; 7] = [
        Self::NumbersAndDates,
        Self::YoFication,
        Self::HomographsCap,
        Self::CapToPlusConverter,
        Self::PlusStressWords,
        Self::PlusStressOrthoepy,
        Self::CleanupAndTranslit,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::NumbersAndDates => "ЧИСЛА И ДАТЫ",
            Self::YoFication => "Ё-ФИКАЦИЯ",
            Self::HomographsCap => "ОМОГРАФЫ (загл.)",
            Self::CapToPlusConverter => "ЗАГЛАВНЫЕ -> '+'",
            Self::PlusStressWords => "УДАРЕНИЯ '+'",
            Self::PlusStressOrthoepy => "ОРФОЭПИЯ '+' (2.9М)",
            Self::CleanupAndTranslit => "ЧИСТКА / ТРАНСЛИТ",
        }
    }
}

fn default_true() -> bool {
    true
}

fn legacy_theme(dark_mode: Option<bool>, warm_mode: Option<bool>) -> Option<ThemeMode> {
    match (dark_mode, warm_mode) {
        (None, None) => None,
        (Some(true), _) => Some(ThemeMode::Dark),
        (_, Some(true)) => Some(ThemeMode::Warm),
        _ => Some(ThemeMode::Light),
    }
}

#[derive(Debug, Clone)]
pub struct DictionaryMeta {
    pub name: String,
    pub kind: DictionaryKind,
    pub tag: DictionaryTag,
    pub description: String,
    pub outputs_plus_stress: bool,
    pub enabled: bool,
    pub rule_count: usize,
    pub encoding: &'static str,
    pub warnings: Vec<String>,
    pub last_ms: Option<u128>,
    pub last_replacements: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ReplacementEvent {
    pub dictionary_name: String,
    pub rule_summary: String,
    pub before_snippet: String,
    pub after_snippet: String,
}

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub text: String,
    pub replacements_count: usize,
    pub events: Vec<ReplacementEvent>,
}

#[derive(Debug, Clone)]
pub struct StepReport {
    pub dictionary_name: String,
    pub kind: DictionaryKind,
    pub rule_count: usize,
    pub elapsed_ms: u128,
    pub replacements_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct PipelineReport {
    pub steps: Vec<StepReport>,
    pub total_ms: u128,
    pub total_replacements: usize,
    pub events: Vec<ReplacementEvent>,
}

#[derive(Debug, Clone, Default)]
pub struct BookDocument {
    pub title: String,
    pub source_path: Option<PathBuf>,
    pub raw_text: String,
    pub processed_text: String,
    pub chunks: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PipelineResult {
    pub raw_text: String,
    pub processed_text: String,
    pub chunks: Vec<String>,
    pub report: PipelineReport,
}

#[derive(Debug, Clone)]
pub enum PipelineProgress {
    StepStarted {
        index: usize,
        total: usize,
        name: String,
    },
    StepFinished(StepReport),
    Completed(PipelineResult),
    Cancelled,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub ui_font_size: f32,
    pub editor_font_size: f32,
    #[serde(default)]
    pub theme: ThemeMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) dark_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) warm_mode: Option<bool>,
    pub stress_format: StressFormat,
    pub chunk_config: ChunkConfig,
    pub dic_folder: PathBuf,
    pub disabled_dictionaries: Vec<String>,
    #[serde(default = "default_true")]
    pub group_dictionaries_by_type: bool,
    #[serde(default)]
    pub collapsed_dictionary_groups: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            ui_font_size: 16.0,
            editor_font_size: 19.0,
            theme: ThemeMode::Dark,
            dark_mode: None,
            warm_mode: None,
            stress_format: StressFormat::PlusBeforeVowel,
            chunk_config: ChunkConfig::default(),
            dic_folder: PathBuf::from("dic"),
            disabled_dictionaries: Vec::new(),
            group_dictionaries_by_type: true,
            collapsed_dictionary_groups: Vec::new(),
        }
    }
}

impl AppSettings {
    pub fn theme(&self) -> ThemeMode {
        self.theme
    }

    pub fn set_theme(&mut self, theme: ThemeMode) {
        self.theme = theme;
    }

    pub fn settings_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            return dir.join("settings.json");
        }
        PathBuf::from("settings.json")
    }

    pub fn load(default_dic_folder: PathBuf) -> Self {
        let path = Self::settings_path();
        if let Ok(bytes) = std::fs::read(&path)
            && let Some(settings) = Self::from_json(&bytes, default_dic_folder.clone())
        {
            return settings;
        }
        Self {
            dic_folder: default_dic_folder,
            ..Self::default()
        }
    }

    fn from_json(bytes: &[u8], default_dic_folder: PathBuf) -> Option<Self> {
        let mut settings = serde_json::from_slice::<Self>(bytes).ok()?;
        if let Some(theme) = legacy_theme(settings.dark_mode, settings.warm_mode) {
            settings.theme = theme;
            settings.dark_mode = None;
            settings.warm_mode = None;
        }
        let is_old_demagog = settings
            .dic_folder
            .to_string_lossy()
            .contains("Demagog-x64-Piper");
        if (!settings.dic_folder.is_dir() || is_old_demagog) && default_dic_folder.is_dir() {
            settings.dic_folder = default_dic_folder;
        }
        settings.ui_font_size = settings.ui_font_size.clamp(12.0, 28.0);
        settings.editor_font_size = settings.editor_font_size.clamp(12.0, 36.0);
        Some(settings)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::settings_path();
        let json = serde_json::to_string_pretty(self).map_err(|err| err.to_string())?;
        std::fs::write(&path, json).map_err(|err| format!("{}: {err}", path.display()))
    }

    pub fn is_dic_enabled(&self, name: &str) -> bool {
        !self.disabled_dictionaries.iter().any(|d| d == name)
    }
}

pub fn describe_dictionary(name: &str, content: &str) -> (DictionaryTag, String, bool) {
    let lower = name.to_lowercase();

    // Detect if rules in this file insert '+' stress marks
    let has_plus_in_rules = lower.contains("(phoneme)")
        || content
            .lines()
            .take(200)
            .filter_map(|l| l.split_once('='))
            .any(|(_, rhs)| rhs.contains('+'));

    if lower.starts_with("10_") || lower.contains("числа") || lower.contains("chisla") {
        (
            DictionaryTag::NumbersAndDates,
            "Переводит числа, даты, римские цифры и валюты в текст словами".into(),
            false,
        )
    } else if lower.starts_with("15_") {
        (
            DictionaryTag::YoFication,
            "Контекстный выбор «все» или «всё» по регулярным выражениям".into(),
            false,
        )
    } else if lower.starts_with("20_") {
        (
            DictionaryTag::HomographsCap,
            "Индексный словарь омографов от waska: ставит ЗАГЛАВНУЮ ударную (зАмок/замОк)".into(),
            false,
        )
    } else if lower.starts_with("25_") {
        (
            DictionaryTag::HomographsCap,
            "Контекстные ударения в именах и фамилиях (через заглавную гласную)".into(),
            false,
        )
    } else if lower.starts_with("30_") {
        (
            DictionaryTag::CleanupAndTranslit,
            "Чистка лишних пробелов перед знаками препинания".into(),
            false,
        )
    } else if lower.contains("edge_yo") || lower == "yo.dic" {
        (
            DictionaryTag::YoFication,
            "Словарь Ё-фикации (337 тыс. слов): заменяет «е» на «ё» без знаков «+»".into(),
            false,
        )
    } else if lower.starts_with("35_") {
        (
            DictionaryTag::PlusStressWords,
            "Пользовательский словарь собственных слов со знаком ударения «+»".into(),
            true,
        )
    } else if lower.starts_with("40.1") {
        (
            DictionaryTag::HomographsCap,
            "Омографы по окончаниям со звёздочкой (*): ставит ЗАГЛАВНУЮ ударную".into(),
            false,
        )
    } else if lower.starts_with("40.2") {
        (
            DictionaryTag::HomographsCap,
            "Омографы во фразах с пропущенным словом (*): ставит ЗАГЛАВНУЮ ударную".into(),
            false,
        )
    } else if lower.starts_with("41_") {
        (
            DictionaryTag::YoFication,
            "Устойчивые фразы и шаблоны: уточнение «все» -> «всё»".into(),
            false,
        )
    } else if lower.starts_with("42_") {
        (
            DictionaryTag::YoFication,
            "Обратная коррекция: исправляет ошибочное «всё» на «все» во фразах".into(),
            false,
        )
    } else if lower.starts_with("50_")
        || lower.starts_with("51_")
        || lower.starts_with("52_")
        || lower.starts_with("53_")
    {
        (
            DictionaryTag::HomographsCap,
            "Словарь словосочетаний для омографов: ставит ЗАГЛАВНУЮ ударную (зЕмли)".into(),
            false,
        )
    } else if lower.starts_with("60_") {
        (
            DictionaryTag::CleanupAndTranslit,
            "Пост-обработка: расстановка запятых и фонетическая правка сложных слов".into(),
            false,
        )
    } else if lower.starts_with("65_") {
        (
            DictionaryTag::CleanupAndTranslit,
            "Транслитерация английских/латинских слов и корней русскими буквами".into(),
            false,
        )
    } else if lower.starts_with("67_") {
        (
            DictionaryTag::PlusStressWords,
            "Большой словарь фраз и слов TTS_Offline: ставит знак ударения «+»".into(),
            true,
        )
    } else if lower.starts_with("69_") {
        (
            DictionaryTag::CapToPlusConverter,
            "Конвертер: переводит заглавные ударные из омографов (зЕмли) в знак «+» (з+емли)"
                .into(),
            true,
        )
    } else if lower.starts_with("70_") {
        (
            DictionaryTag::CapToPlusConverter,
            "Конвертер: ставит «+» перед первой заглавной буквой омографов ($Адреса -> +Адреса)"
                .into(),
            true,
        )
    } else if lower.starts_with("71_") || lower.starts_with("72_") {
        (
            DictionaryTag::PlusStressWords,
            "Точечные правки ударений со знаком «+» в отдельных словах и корнях".into(),
            true,
        )
    } else if lower.starts_with("73_") {
        (
            DictionaryTag::PlusStressWords,
            "Расшифровка аббревиатур (МВД, КГБ, США и др.) с ударениями «+»".into(),
            true,
        )
    } else if lower.starts_with("74_") {
        (
            DictionaryTag::PlusStressWords,
            "Раскрытие запиканного точками мата (*...здят) с ударениями «+»".into(),
            true,
        )
    } else if lower.starts_with("75_") {
        (
            DictionaryTag::PlusStressWords,
            "Словарь имён собственных, городов и фамилий со знаком ударения «+»".into(),
            true,
        )
    } else if lower.starts_with("95.") {
        (
            DictionaryTag::PlusStressOrthoepy,
            "Главный орфоэпический словарь (2.9 млн форм): ставит «+» в КАЖДОМ обычном слове"
                .into(),
            true,
        )
    } else if has_plus_in_rules {
        (
            DictionaryTag::PlusStressWords,
            "Словарь со знаком ударения «+» перед гласной".into(),
            true,
        )
    } else {
        (
            DictionaryTag::CleanupAndTranslit,
            "Текстовый словарь замен".into(),
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_roundtrip() {
        let mut settings = AppSettings::default();
        assert_eq!(settings.theme(), ThemeMode::Dark);
        settings.set_theme(ThemeMode::Light);
        assert_eq!(settings.theme(), ThemeMode::Light);
        settings.set_theme(ThemeMode::Warm);
        assert_eq!(settings.theme(), ThemeMode::Warm);
    }

    #[test]
    fn legacy_theme_flags_are_migrated() {
        assert_eq!(legacy_theme(Some(true), Some(true)), Some(ThemeMode::Dark));
        assert_eq!(legacy_theme(Some(false), Some(true)), Some(ThemeMode::Warm));
        assert_eq!(
            legacy_theme(Some(false), Some(false)),
            Some(ThemeMode::Light)
        );
        assert_eq!(legacy_theme(None, None), None);
    }

    #[test]
    fn theme_is_serialized_as_a_single_field() {
        let mut settings = AppSettings::default();
        settings.set_theme(ThemeMode::Warm);

        let value = serde_json::to_value(&settings).expect("serialize settings");
        assert_eq!(value["theme"], "Warm");
        assert!(value.get("dark_mode").is_none());
        assert!(value.get("warm_mode").is_none());

        let parsed: AppSettings = serde_json::from_value(value).expect("deserialize settings");
        assert_eq!(parsed.theme(), ThemeMode::Warm);
    }

    #[test]
    fn loads_legacy_theme_flags_from_json() {
        let mut value =
            serde_json::to_value(AppSettings::default()).expect("serialize default settings");
        value["dark_mode"] = serde_json::json!(false);
        value["warm_mode"] = serde_json::json!(true);
        value
            .as_object_mut()
            .expect("settings object")
            .remove("theme");

        let loaded = AppSettings::from_json(
            &serde_json::to_vec(&value).expect("legacy JSON"),
            PathBuf::from("dic"),
        )
        .expect("load legacy settings");

        assert_eq!(loaded.theme(), ThemeMode::Warm);
        assert!(loaded.dark_mode.is_none());
        assert!(loaded.warm_mode.is_none());
    }
}

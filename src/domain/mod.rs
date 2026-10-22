use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// Знак '+' перед ударной гласной: з+амок, +Август (для FishAudio Drama3 / Silero / F5-TTS)
    PlusBeforeVowel,
    /// Комбинируемый знак ударения Юникода U+0301 после гласной: за́мок
    AcuteAccent,
    /// Чистый текст без '+' и внутренних заглавных, но с сохранением 'ё'
    CleanYoOnly,
}

impl StressFormat {
    pub fn label(&self) -> &'static str {
        match self {
            Self::PlusBeforeVowel => "+ перед гласной (з+амок) [Drama3 / Silero]",
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

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DictionaryMeta {
    pub name: String,
    pub path: PathBuf,
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
#[allow(dead_code)]
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
    StepStarted { index: usize, total: usize, name: String },
    StepFinished(StepReport),
    Completed(PipelineResult),
    Cancelled,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub ui_font_size: f32,
    pub editor_font_size: f32,
    pub dark_mode: bool,
    #[serde(default)]
    pub warm_mode: bool,
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
            dark_mode: true,
            warm_mode: false,
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
        if self.dark_mode {
            ThemeMode::Dark
        } else if self.warm_mode {
            ThemeMode::Warm
        } else {
            ThemeMode::Light
        }
    }

    pub fn set_theme(&mut self, theme: ThemeMode) {
        self.dark_mode = theme == ThemeMode::Dark;
        self.warm_mode = theme == ThemeMode::Warm;
    }

    pub fn settings_path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return dir.join("settings.json");
            }
        }
        PathBuf::from("settings.json")
    }

    pub fn load(default_dic_folder: PathBuf) -> Self {
        let path = Self::settings_path();
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(mut s) = serde_json::from_slice::<Self>(&bytes) {
                let is_old_demagog = s
                    .dic_folder
                    .to_string_lossy()
                    .contains("Demagog-x64-Piper");
                if (!s.dic_folder.is_dir() || is_old_demagog) && default_dic_folder.is_dir() {
                    s.dic_folder = default_dic_folder;
                }
                s.ui_font_size = s.ui_font_size.clamp(12.0, 28.0);
                s.editor_font_size = s.editor_font_size.clamp(12.0, 36.0);
                return s;
            }
        }
        Self {
            dic_folder: default_dic_folder,
            ..Self::default()
        }
    }

    pub fn save(&self) {
        let path = Self::settings_path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn is_dic_enabled(&self, name: &str) -> bool {
        !self.disabled_dictionaries.iter().any(|d| d == name)
    }

    pub fn sync_disabled_from_metas(&mut self, metas: &[DictionaryMeta]) {
        self.disabled_dictionaries = metas
            .iter()
            .filter(|m| !m.enabled)
            .map(|m| m.name.clone())
            .collect();
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
            "Конвертер: переводит заглавные ударные из омографов (зЕмли) в знак «+» (з+емли)".into(),
            true,
        )
    } else if lower.starts_with("70_") {
        (
            DictionaryTag::CapToPlusConverter,
            "Конвертер: ставит «+» перед первой заглавной буквой омографов ($Адреса -> +Адреса)".into(),
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
            "Главный орфоэпический словарь (2.9 млн форм): ставит «+» в КАЖДОМ обычном слове".into(),
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
        assert!(!settings.dark_mode && !settings.warm_mode);
        settings.set_theme(ThemeMode::Warm);
        assert_eq!(settings.theme(), ThemeMode::Warm);
        assert!(!settings.dark_mode && settings.warm_mode);
    }
}

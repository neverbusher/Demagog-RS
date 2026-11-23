use crate::domain::{DictionaryStage, classify_dictionary};

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

pub fn validate_order(names: &[String]) -> OrderReport {
    let mut report = OrderReport::default();
    let mut previous: Option<(DictionaryStage, &str)> = None;
    let mut hard_e_index: Option<usize> = None;
    let mut yo_base_index: Option<usize> = None;
    let mut last_rex: Option<usize> = None;
    let mut first_dic: Option<usize> = None;

    for (index, name) in names.iter().enumerate() {
        let profile = classify_dictionary(name);
        let stage = profile.stage;
        report.ordered.push(OrderedDictionary {
            name: name.clone(),
            stage: stage.label(),
        });

        if stage == DictionaryStage::Rex {
            last_rex = Some(index);
            if profile.number.is_none() {
                report
                    .warnings
                    .push(format!("{name}: .rex без номера — проверьте порядок"));
            }
        } else if first_dic.is_none() {
            first_dic = Some(index);
        }

        match stage {
            DictionaryStage::Unnumbered => report.failures.push(format!(
                "{name}: .dic без номера — загрузчик поставит его после HARD-E"
            )),
            DictionaryStage::Unknown => report.warnings.push(format!(
                "{name}: неизвестный этап — проверьте порядок вручную"
            )),
            DictionaryStage::HardE => {
                if hard_e_index.is_some() {
                    report
                        .warnings
                        .push(format!("{name}: второй словарь HARD-E"));
                }
                hard_e_index = Some(index);
                if !profile.has_hard_marker {
                    report.warnings.push(format!(
                        "{name}: номер 96 без маркера (hard-e) — это не HARD-E"
                    ));
                }
            }
            DictionaryStage::YoBase => {
                if yo_base_index.is_some() {
                    report
                        .warnings
                        .push(format!("{name}: второй базовый ё-словарь"));
                }
                yo_base_index = Some(index);
            }
            _ => {}
        }

        if profile.has_hard_marker && stage != DictionaryStage::HardE {
            report.failures.push(format!(
                "{name}: HARD-E должен называться 96_... (hard-e).dic"
            ));
        }

        if !matches!(
            stage,
            DictionaryStage::Unknown | DictionaryStage::Unnumbered
        ) {
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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicBool;

use crate::domain::{AppError, AppResult, ChunkConfig, StressFormat, TextCleanupConfig};
use crate::engine::loader::{load_dictionaries_parallel_with_cancel, sorted_dictionary_paths};
use crate::ports::{TextProcessor, is_cancelled};
use crate::services::chunker::SentenceAwareChunker;
use crate::services::dictionary_audit::{AuditReport, audit_dictionaries_with_cancel};
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

/// Committed manifest pinning the dictionary set the golden corpus was
/// validated against. Lives in the repo (dic/ itself is gitignored).
pub const MANIFEST_PATH: &str = "assets/dictionaries.sha256";

/// Prefers the manifest shipped next to the executable (packaged app),
/// falling back to the working directory (repo checkout during development).
pub fn manifest_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()));
    manifest_path_for(exe_dir.as_deref())
}

fn manifest_path_for(exe_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = exe_dir {
        let candidate = dir.join(MANIFEST_PATH);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(MANIFEST_PATH)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Hashes every top-level .dic/.rex file, sorted by name.
pub fn dictionary_hashes(dir: &Path) -> AppResult<Vec<(String, String)>> {
    let paths = sorted_dictionary_paths(dir)?;
    let mut out = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unknown>")
            .to_string();
        let bytes = std::fs::read(&path).map_err(|error| {
            AppError::io(format!("Не удалось прочитать {}: {error}", path.display()))
        })?;
        out.push((name, sha256_hex(&bytes)));
    }
    out.sort();
    Ok(out)
}

pub fn write_manifest(dir: &Path, manifest: &Path) -> AppResult<usize> {
    let hashes = dictionary_hashes(dir)?;
    if let Some(parent) = manifest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| AppError::io(format!("{}: {error}", parent.display())))?;
    }
    let body: String = hashes
        .iter()
        .map(|(name, hash)| format!("{hash}  {name}\n"))
        .collect();
    std::fs::write(manifest, body)
        .map_err(|error| AppError::io(format!("{}: {error}", manifest.display())))?;
    Ok(hashes.len())
}

/// Warns when the on-disk dictionary set drifts from the committed manifest.
/// Returns no warnings when the manifest is absent (e.g. a fresh checkout
/// without dictionaries).
pub fn manifest_drift(dir: &Path, manifest: &Path) -> Vec<String> {
    let Ok(body) = std::fs::read_to_string(manifest) else {
        return Vec::new();
    };
    let expected: std::collections::HashMap<String, String> = body
        .lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once("  ")?;
            Some((name.trim().to_string(), hash.trim().to_string()))
        })
        .collect();
    let Ok(actual) = dictionary_hashes(dir) else {
        return Vec::new();
    };
    let actual: std::collections::HashMap<String, String> = actual.into_iter().collect();
    let mut warnings = Vec::new();
    for (name, hash) in &actual {
        match expected.get(name) {
            None => warnings.push(format!("Словарь {name} отсутствует в манифесте")),
            Some(want) if want != hash => {
                warnings.push(format!("Словарь {name} изменился относительно манифеста"));
            }
            Some(_) => {}
        }
    }
    for name in expected.keys() {
        if !actual.contains_key(name) {
            warnings.push(format!("Словарь {name} из манифеста отсутствует на диске"));
        }
    }
    warnings.sort();
    warnings
}

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

pub fn run_check(dir: &Path) -> AppResult<CheckReport> {
    run_check_with_cancel(dir, None)
}

const CHECK_CANCELLED: &str = "Проверка словарей отменена";

fn check_cancelled() -> AppError {
    AppError::cancelled(CHECK_CANCELLED)
}

pub fn run_check_with_cancel(dir: &Path, cancel: Option<&AtomicBool>) -> AppResult<CheckReport> {
    let is_cancelled = || is_cancelled(cancel);
    if is_cancelled() {
        return Err(check_cancelled());
    }
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
    for warning in manifest_drift(dir, &manifest_path()) {
        order.warnings.push(warning);
    }
    let report = load_dictionaries_parallel_with_cancel(dir, cancel)?;
    if is_cancelled() {
        return Err(check_cancelled());
    }
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
    let audit = match audit_dictionaries_with_cancel(dir, cancel) {
        Ok(audit) => audit,
        Err(error) => {
            if is_cancelled() || error.is_cancelled() {
                return Err(check_cancelled());
            }
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
                cleanup: TextCleanupConfig::default(),
            },
            cancel,
            None,
        );
        match result {
            Ok(result) if result.processed_text == case.expected => passed += 1,
            Ok(result) => order.failures.push(format!(
                "{}: ожидалось «{}», получено «{}»",
                case.name, case.expected, result.processed_text
            )),
            Err(error) if error.is_cancelled() => return Err(check_cancelled()),
            Err(error) => return Err(error),
        }
    }

    if is_cancelled() {
        return Err(check_cancelled());
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
    use crate::domain::{AppFailure, FailureKind};
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let sequence = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "demagog-dictionary-check-{tag}-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn expect_failure(error: &AppError, kind: FailureKind) -> &AppFailure {
        let AppError::Failed(failure) = error else {
            panic!("expected {kind:?} failure, got {error:?}");
        };
        assert_eq!(failure.kind, kind);
        failure
    }

    #[cfg(windows)]
    fn lock_for_read(path: &std::path::Path) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;

        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .expect("lock dictionary fixture")
    }

    #[test]
    fn unnumbered_yo_dictionary_passes_order_check() {
        let names = vec![
            "10_nums.rex".to_string(),
            "yo.dic".to_string(),
            "96_hard(hard-e).dic".to_string(),
        ];
        let report = validate_order(&names);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.ordered[1].stage, "Ё: БАЗА");
    }

    #[test]
    fn unnumbered_non_yo_dictionary_still_fails_order_check() {
        let names = vec!["96_hard(hard-e).dic".to_string(), "custom.dic".to_string()];
        let report = validate_order(&names);
        assert!(!report.failures.is_empty());
    }

    #[test]
    fn manifest_resolves_next_to_the_exe_when_present() {
        let dir = std::env::temp_dir().join(format!("demagog-manifest-exe-{}", std::process::id()));
        let assets = dir.join("assets");
        std::fs::create_dir_all(&assets).expect("assets dir");
        let manifest = assets.join("dictionaries.sha256");
        std::fs::write(&manifest, "hash  name.dic\n").expect("write manifest");

        assert_eq!(manifest_path_for(Some(&dir)), manifest);
        assert_eq!(
            manifest_path_for(None),
            std::path::PathBuf::from(MANIFEST_PATH)
        );

        let empty =
            std::env::temp_dir().join(format!("demagog-manifest-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).expect("empty dir");
        assert_eq!(
            manifest_path_for(Some(&empty)),
            std::path::PathBuf::from(MANIFEST_PATH)
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn manifest_roundtrip_and_drift_detection() {
        let dir = std::env::temp_dir().join(format!(
            "demagog-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::write(dir.join("10_a.dic"), "а=б\n").expect("write a");
        std::fs::write(dir.join("20_b.dic"), "в=г\n").expect("write b");
        let manifest = dir.join("manifest.sha256");

        let count = write_manifest(&dir, &manifest).expect("write manifest");
        assert_eq!(count, 2);
        assert!(manifest_drift(&dir, &manifest).is_empty());

        std::fs::write(dir.join("20_b.dic"), "в=г изменено\n").expect("modify b");
        let drift = manifest_drift(&dir, &manifest);
        assert!(drift.iter().any(|w| w.contains("20_b.dic")));

        std::fs::remove_file(dir.join("10_a.dic")).expect("remove a");
        let drift = manifest_drift(&dir, &manifest);
        assert!(drift.iter().any(|w| w.contains("10_a.dic")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dictionary_hashes_propagates_directory_io() {
        let root = temp_dir("hash-directory-io");
        let blocker = root.join("not-a-directory");
        std::fs::write(&blocker, "file").unwrap();
        let io_error = std::fs::read_dir(&blocker).expect_err("read_dir must fail");
        let expected = format!(
            "Не удалось открыть папку словарей {}: {io_error}",
            blocker.display()
        );

        let error = dictionary_hashes(&blocker).expect_err("dictionary hashes must fail");

        let failure = expect_failure(&error, FailureKind::Io);
        assert_eq!(failure.message, expected);
        assert_eq!(error.to_string(), failure.message);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn dictionary_hashes_reports_dictionary_read_failure_as_io() {
        let root = temp_dir("hash-read-io");
        let dictionary = root.join("10_a.dic");
        std::fs::write(&dictionary, "а=б\n").unwrap();
        let lock = lock_for_read(&dictionary);
        let io_error = std::fs::read(&dictionary).expect_err("locked dictionary is unreadable");
        let expected = format!("Не удалось прочитать {}: {io_error}", dictionary.display());

        let error = dictionary_hashes(&root).expect_err("dictionary hashes must fail");

        let failure = expect_failure(&error, FailureKind::Io);
        assert_eq!(failure.message, expected);
        assert_eq!(error.to_string(), failure.message);
        drop(lock);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_manifest_propagates_dictionary_directory_io() {
        let root = temp_dir("manifest-hash-io");
        let blocker = root.join("not-a-directory");
        std::fs::write(&blocker, "file").unwrap();
        let expected = dictionary_hashes(&blocker).expect_err("dictionary hashes must fail");

        let error = write_manifest(&blocker, &root.join("manifest.sha256"))
            .expect_err("manifest write must fail");

        assert_eq!(error, expected);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_manifest_reports_directory_creation_failure_as_io() {
        let root = temp_dir("manifest-directory-io");
        let dictionaries = root.join("dic");
        std::fs::create_dir_all(&dictionaries).unwrap();
        std::fs::write(dictionaries.join("10_a.dic"), "а=б\n").unwrap();
        let blocker = root.join("not-a-directory");
        std::fs::write(&blocker, "file").unwrap();
        let manifest = blocker.join("manifest.sha256");
        let io_error =
            std::fs::create_dir_all(&blocker).expect_err("manifest parent creation must fail");
        let expected = format!("{}: {io_error}", blocker.display());

        let error = write_manifest(&dictionaries, &manifest).expect_err("manifest write must fail");

        let failure = expect_failure(&error, FailureKind::Io);
        assert_eq!(failure.message, expected);
        assert_eq!(error.to_string(), failure.message);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_manifest_reports_write_failure_as_io() {
        let root = temp_dir("manifest-write-io");
        let dictionaries = root.join("dic");
        std::fs::create_dir_all(&dictionaries).unwrap();
        std::fs::write(dictionaries.join("10_a.dic"), "а=б\n").unwrap();
        let manifest = root.join("manifest.sha256");
        std::fs::create_dir_all(&manifest).unwrap();
        let io_error = std::fs::write(&manifest, "blocked").expect_err("directory write must fail");
        let expected = format!("{}: {io_error}", manifest.display());

        let error = write_manifest(&dictionaries, &manifest).expect_err("manifest write must fail");

        let failure = expect_failure(&error, FailureKind::Io);
        assert_eq!(failure.message, expected);
        assert_eq!(error.to_string(), failure.message);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dictionary_check_honors_a_cancel_flag() {
        let cancel = AtomicBool::new(true);
        let result = run_check_with_cancel(std::path::Path::new("dic"), Some(&cancel));
        assert!(
            matches!(result, Err(AppError::Cancelled(_))),
            "cancelled dictionary check must report cancellation"
        );
    }

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

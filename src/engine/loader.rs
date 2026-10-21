use std::path::{Path, PathBuf};
use std::sync::Arc;
use rayon::prelude::*;

use crate::domain::{describe_dictionary, DictionaryMeta};
use crate::engine::dic_processor::DicProcessor;
use crate::engine::rex_processor::RexProcessor;
use crate::io::encoding::decode_text_bytes;
use crate::ports::TextProcessor;

pub struct CompiledDictionary {
    pub meta: DictionaryMeta,
    pub processor: Arc<dyn TextProcessor>,
}

pub fn load_dictionaries_parallel(dir: &Path) -> Result<Vec<CompiledDictionary>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Не удалось открыть папку словарей {}: {e}", dir.display()))?;

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && matches!(
                    p.extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_ascii_lowercase())
                        .as_deref(),
                    Some("rex" | "dic")
                )
        })
        .collect();

    paths.sort_by_key(|p| pipeline_sort_key(p));

    let compiled: Vec<CompiledDictionary> = paths
        .par_iter()
        .filter_map(|path| compile_single_dictionary(path).ok())
        .collect();

    Ok(compiled)
}

pub fn compile_single_dictionary(path: &Path) -> Result<CompiledDictionary, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (content, encoding) = decode_text_bytes(&bytes);
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let (tag, description, outputs_plus_stress) = describe_dictionary(&name, &content);

    let is_rex = path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("rex"));

    let processor: Arc<dyn TextProcessor> = if is_rex {
        Arc::new(RexProcessor::compile(name.clone(), &content, encoding))
    } else {
        Arc::new(DicProcessor::compile(name.clone(), &content, encoding))
    };

    let meta = DictionaryMeta {
        name,
        path: path.to_path_buf(),
        kind: processor.kind(),
        tag,
        description,
        outputs_plus_stress,
        enabled: true,
        rule_count: processor.rule_count(),
        encoding,
        last_ms: None,
        last_replacements: None,
    };

    Ok(CompiledDictionary { meta, processor })
}

fn pipeline_sort_key(path: &Path) -> (u8, u32, String) {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let lower = name.to_lowercase();
    let is_rex = lower.ends_with(".rex");

    let leading_num: Option<u32> = name
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok();

    if is_rex {
        (0, leading_num.unwrap_or(500), lower)
    } else if lower.contains("yo") && leading_num.is_none() {
        (1, 32, lower)
    } else if let Some(n) = leading_num {
        (2, n, lower)
    } else {
        (3, 999, lower)
    }
}

use std::sync::Arc;

use crate::domain::ProcessorFlavor;
use crate::engine::dic_processor::DicProcessor;
use crate::engine::hard_e_processor::HardEProcessor;
use crate::engine::rex_processor::RexProcessor;
use crate::ports::TextProcessor;

pub(super) struct CompiledProcessor {
    pub processor: Arc<dyn TextProcessor>,
    pub warnings: Vec<String>,
}

type ProcessorCompiler = fn(String, &str, &'static str) -> CompiledProcessor;

struct Registration {
    flavor: ProcessorFlavor,
    compile: ProcessorCompiler,
}

const REGISTRY: [Registration; 3] = [
    Registration {
        flavor: ProcessorFlavor::HardE,
        compile: compile_hard_e,
    },
    Registration {
        flavor: ProcessorFlavor::Rex,
        compile: compile_rex,
    },
    Registration {
        flavor: ProcessorFlavor::Dic,
        compile: compile_dic,
    },
];

pub(super) fn compile_processor(
    flavor: ProcessorFlavor,
    name: String,
    content: &str,
    encoding: &'static str,
) -> CompiledProcessor {
    let registration = REGISTRY
        .iter()
        .find(|registration| registration.flavor == flavor)
        .expect("every processor flavor must be registered");
    (registration.compile)(name, content, encoding)
}

fn compile_hard_e(name: String, content: &str, _encoding: &'static str) -> CompiledProcessor {
    let processor = HardEProcessor::compile(name, content);
    let warnings = processor.compile_warnings().to_vec();
    CompiledProcessor {
        processor: Arc::new(processor),
        warnings,
    }
}

fn compile_rex(name: String, content: &str, encoding: &'static str) -> CompiledProcessor {
    let processor = RexProcessor::compile(name, content, encoding);
    let warnings = processor.compile_warnings().to_vec();
    CompiledProcessor {
        processor: Arc::new(processor),
        warnings,
    }
}

fn compile_dic(name: String, content: &str, encoding: &'static str) -> CompiledProcessor {
    let processor = DicProcessor::compile(name, content, encoding);
    let warnings = processor.compile_warnings().to_vec();
    CompiledProcessor {
        processor: Arc::new(processor),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DictionaryKind, ProcessorFlavor};

    const ALL_FLAVORS: [ProcessorFlavor; 3] = [
        ProcessorFlavor::HardE,
        ProcessorFlavor::Rex,
        ProcessorFlavor::Dic,
    ];

    #[test]
    fn registry_contains_every_processor_flavor_exactly_once() {
        assert_eq!(REGISTRY.len(), ALL_FLAVORS.len());
        for flavor in ALL_FLAVORS {
            assert_eq!(
                REGISTRY
                    .iter()
                    .filter(|registration| registration.flavor == flavor)
                    .count(),
                1,
                "{flavor:?} must be registered exactly once"
            );
        }
    }

    #[test]
    fn every_flavor_compiles_a_real_rule_with_the_expected_kind() {
        for (flavor, name, content) in [
            (ProcessorFlavor::HardE, "96_(hard-e).dic", "тест=тэст\n"),
            (ProcessorFlavor::Rex, "10_nums.rex", "кот=к+от\n"),
            (ProcessorFlavor::Dic, "35_custom.dic", "кот=к+от\n"),
        ] {
            let compiled = compile_processor(flavor, name.to_string(), content, "UTF-8");

            assert_eq!(compiled.processor.name(), name);
            assert_eq!(compiled.processor.rule_count(), 1);
            match flavor {
                ProcessorFlavor::HardE => {
                    assert_eq!(compiled.processor.kind(), DictionaryKind::HardE);
                }
                ProcessorFlavor::Rex => assert!(matches!(
                    compiled.processor.kind(),
                    DictionaryKind::RegexStandard | DictionaryKind::RegexIndexed
                )),
                ProcessorFlavor::Dic => {
                    assert_eq!(compiled.processor.kind(), DictionaryKind::DicStandard);
                }
            }
        }
    }

    #[test]
    fn compile_warnings_are_propagated_unchanged() {
        let compiled = compile_processor(
            ProcessorFlavor::HardE,
            "96_(hard-e).dic".to_string(),
            "а темпо=а тэмпо\n",
            "UTF-8",
        );

        assert_eq!(
            compiled.warnings,
            vec!["а темпо=а тэмпо: многословные правила не поддерживаются"]
        );
    }
}

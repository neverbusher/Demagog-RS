use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use crate::domain::{
    ChunkConfig, PipelineProgress, PipelineReport, PipelineResult, StepReport, StressFormat,
};
use crate::ports::{StressFormatter, TextChunker, TextProcessor};

pub struct ProcessingPipeline {
    stress_formatter: Arc<dyn StressFormatter>,
    chunker: Arc<dyn TextChunker>,
}

impl ProcessingPipeline {
    pub fn new(
        stress_formatter: Arc<dyn StressFormatter>,
        chunker: Arc<dyn TextChunker>,
    ) -> Self {
        Self {
            stress_formatter,
            chunker,
        }
    }

    pub fn execute(
        &self,
        input_text: &str,
        processors: &[Arc<dyn TextProcessor>],
        stress_format: StressFormat,
        chunk_config: &ChunkConfig,
        capture_diff: bool,
        progress_tx: Option<&Sender<PipelineProgress>>,
    ) -> PipelineResult {
        let total_start = Instant::now();
        let mut current = input_text.to_string();
        let mut report = PipelineReport::default();
        let total = processors.len();

        for (idx, proc) in processors.iter().enumerate() {
            if let Some(tx) = progress_tx {
                let _ = tx.send(PipelineProgress::StepStarted {
                    index: idx + 1,
                    total,
                    name: proc.name().to_string(),
                });
            }

            let step_start = Instant::now();
            let out = proc.process(&current, capture_diff);
            let elapsed_ms = step_start.elapsed().as_millis();

            let step_report = StepReport {
                dictionary_name: proc.name().to_string(),
                kind: proc.kind(),
                rule_count: proc.rule_count(),
                elapsed_ms,
                replacements_count: out.replacements_count,
            };

            if let Some(tx) = progress_tx {
                let _ = tx.send(PipelineProgress::StepFinished(step_report.clone()));
            }

            report.total_replacements += out.replacements_count;
            report.steps.push(step_report);
            if capture_diff && report.events.len() < 500 {
                report.events.extend(out.events);
            }
            current = out.text;
        }

        let raw_text = current;
        let processed_text = self.stress_formatter.format_stress(&raw_text, stress_format);
        let chunks = self.chunker.split_into_chunks(&processed_text, chunk_config);
        report.total_ms = total_start.elapsed().as_millis();

        let result = PipelineResult {
            raw_text,
            processed_text,
            chunks,
            report,
        };

        if let Some(tx) = progress_tx {
            let _ = tx.send(PipelineProgress::Completed(result.clone()));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DictionaryKind, ProcessOutput};
    use crate::ports::TextProcessor;
    use crate::services::chunker::SentenceAwareChunker;
    use crate::services::stress::RussianStressFormatter;

    struct StressAddingProcessor;

    impl TextProcessor for StressAddingProcessor {
        fn name(&self) -> &str {
            "dummy"
        }

        fn kind(&self) -> DictionaryKind {
            DictionaryKind::RegexStandard
        }

        fn rule_count(&self) -> usize {
            1
        }

        fn process(&self, _text: &str, _capture_diff: bool) -> ProcessOutput {
            ProcessOutput {
                text: "з+амок".to_string(),
                replacements_count: 1,
                events: Vec::new(),
            }
        }
    }

    #[test]
    fn keeps_raw_text_for_later_reformatting() {
        let pipeline = ProcessingPipeline::new(
            Arc::new(RussianStressFormatter),
            Arc::new(SentenceAwareChunker),
        );
        let processors: Vec<Arc<dyn TextProcessor>> = vec![Arc::new(StressAddingProcessor)];

        let result = pipeline.execute(
            "замок",
            &processors,
            StressFormat::CleanYoOnly,
            &ChunkConfig::default(),
            false,
            None,
        );

        assert_eq!(result.raw_text, "з+амок");
        assert_eq!(result.processed_text, "замок");
        assert_eq!(
            RussianStressFormatter.format_stress(&result.raw_text, StressFormat::AcuteAccent),
            "за\u{0301}мок"
        );
    }
}

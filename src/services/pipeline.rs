use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use crate::domain::{
    ChunkConfig, PipelineProgress, PipelineReport, StepReport, StressFormat,
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
    ) -> (String, Vec<String>, PipelineReport) {
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

        let formatted = self.stress_formatter.format_stress(&current, stress_format);
        let chunks = self.chunker.split_into_chunks(&formatted, chunk_config);
        report.total_ms = total_start.elapsed().as_millis();

        if let Some(tx) = progress_tx {
            let _ = tx.send(PipelineProgress::Completed {
                processed_text: formatted.clone(),
                chunks: chunks.clone(),
                report: report.clone(),
            });
        }

        (formatted, chunks, report)
    }
}

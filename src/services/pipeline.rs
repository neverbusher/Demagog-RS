use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Instant;

use crate::domain::{
    ChunkConfig, PipelineProgress, PipelineReport, PipelineResult, StepReport, StressFormat,
};
use crate::ports::{StressFormatter, TextChunker, TextProcessor};

pub struct ProcessingPipeline {
    stress_formatter: Arc<dyn StressFormatter>,
    chunker: Arc<dyn TextChunker>,
}

pub struct PipelineOptions {
    pub stress_format: StressFormat,
    pub chunk_config: ChunkConfig,
    pub capture_diff: bool,
}

impl ProcessingPipeline {
    pub fn new(stress_formatter: Arc<dyn StressFormatter>, chunker: Arc<dyn TextChunker>) -> Self {
        Self {
            stress_formatter,
            chunker,
        }
    }

    pub fn execute(
        &self,
        input_text: &str,
        processors: &[Arc<dyn TextProcessor>],
        options: PipelineOptions,
        cancel: Option<&AtomicBool>,
        progress_tx: Option<&Sender<PipelineProgress>>,
    ) -> Option<PipelineResult> {
        let PipelineOptions {
            stress_format,
            chunk_config,
            capture_diff,
        } = options;
        let total_start = Instant::now();
        let mut current = input_text.to_string();
        let mut report = PipelineReport::default();
        let total = processors.len();

        for (idx, proc) in processors.iter().enumerate() {
            if is_cancelled(cancel) {
                notify_cancelled(progress_tx);
                return None;
            }
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

        if is_cancelled(cancel) {
            notify_cancelled(progress_tx);
            return None;
        }

        let raw_text = current;
        let processed_text = self
            .stress_formatter
            .format_stress(&raw_text, stress_format);
        if is_cancelled(cancel) {
            notify_cancelled(progress_tx);
            return None;
        }
        let chunks = self
            .chunker
            .split_into_chunks(&processed_text, &chunk_config);
        report.total_ms = total_start.elapsed().as_millis();

        let result = PipelineResult {
            raw_text,
            processed_text,
            chunks,
            report,
        };

        if is_cancelled(cancel) {
            notify_cancelled(progress_tx);
            return None;
        }

        if let Some(tx) = progress_tx {
            let _ = tx.send(PipelineProgress::Completed(result.clone()));
        }

        Some(result)
    }
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

fn notify_cancelled(progress_tx: Option<&Sender<PipelineProgress>>) {
    if let Some(tx) = progress_tx {
        let _ = tx.send(PipelineProgress::Cancelled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DictionaryKind, ProcessOutput};
    use crate::ports::TextProcessor;
    use crate::services::chunker::SentenceAwareChunker;
    use crate::services::stress::RussianStressFormatter;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    struct CancelAfterFirst {
        cancel: Arc<AtomicBool>,
    }

    impl TextProcessor for CancelAfterFirst {
        fn name(&self) -> &str {
            "cancel"
        }

        fn kind(&self) -> DictionaryKind {
            DictionaryKind::RegexStandard
        }

        fn rule_count(&self) -> usize {
            1
        }

        fn process(&self, _text: &str, _capture_diff: bool) -> ProcessOutput {
            self.cancel.store(true, Ordering::Relaxed);
            ProcessOutput {
                text: "after-cancel".to_string(),
                replacements_count: 0,
                events: Vec::new(),
            }
        }
    }

    struct CountingProcessor {
        calls: Arc<AtomicUsize>,
    }

    impl TextProcessor for CountingProcessor {
        fn name(&self) -> &str {
            "counting"
        }

        fn kind(&self) -> DictionaryKind {
            DictionaryKind::RegexStandard
        }

        fn rule_count(&self) -> usize {
            1
        }

        fn process(&self, text: &str, _capture_diff: bool) -> ProcessOutput {
            self.calls.fetch_add(1, Ordering::Relaxed);
            ProcessOutput {
                text: text.to_string(),
                replacements_count: 0,
                events: Vec::new(),
            }
        }
    }

    struct SlowCountingProcessor {
        calls: Arc<AtomicUsize>,
        delay: std::time::Duration,
    }

    impl TextProcessor for SlowCountingProcessor {
        fn name(&self) -> &str {
            "slow"
        }

        fn kind(&self) -> DictionaryKind {
            DictionaryKind::RegexStandard
        }

        fn rule_count(&self) -> usize {
            1
        }

        fn process(&self, text: &str, _capture_diff: bool) -> ProcessOutput {
            self.calls.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(self.delay);
            ProcessOutput {
                text: text.to_string(),
                replacements_count: 0,
                events: Vec::new(),
            }
        }
    }

    #[test]
    fn sees_cancellation_from_another_thread() {
        let pipeline = ProcessingPipeline::new(
            Arc::new(RussianStressFormatter),
            Arc::new(SentenceAwareChunker),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let processors: Vec<Arc<dyn TextProcessor>> = (0..50)
            .map(|_| {
                Arc::new(SlowCountingProcessor {
                    calls: calls.clone(),
                    delay: std::time::Duration::from_millis(20),
                }) as Arc<dyn TextProcessor>
            })
            .collect();

        let canceller = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(30));
                cancel.store(true, Ordering::Relaxed);
            })
        };

        let result = pipeline.execute(
            "start",
            &processors,
            PipelineOptions {
                stress_format: StressFormat::CleanYoOnly,
                chunk_config: ChunkConfig::default(),
                capture_diff: false,
            },
            Some(cancel.as_ref()),
            None,
        );
        canceller.join().unwrap();

        assert!(
            result.is_none(),
            "cancelled pipeline must not return a result"
        );
        let processed = calls.load(Ordering::Relaxed);
        assert!(
            processed < 10,
            "pipeline ignored cancellation after {processed} steps"
        );
    }

    #[test]
    fn stops_before_next_dictionary_when_cancelled() {
        let pipeline = ProcessingPipeline::new(
            Arc::new(RussianStressFormatter),
            Arc::new(SentenceAwareChunker),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let processors: Vec<Arc<dyn TextProcessor>> = vec![
            Arc::new(CancelAfterFirst {
                cancel: cancel.clone(),
            }),
            Arc::new(CountingProcessor {
                calls: calls.clone(),
            }),
        ];

        let result = pipeline.execute(
            "start",
            &processors,
            PipelineOptions {
                stress_format: StressFormat::CleanYoOnly,
                chunk_config: ChunkConfig::default(),
                capture_diff: false,
            },
            Some(cancel.as_ref()),
            None,
        );

        assert!(
            result.is_none(),
            "cancelled pipeline must not return a result"
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn keeps_raw_text_for_later_reformatting() {
        let pipeline = ProcessingPipeline::new(
            Arc::new(RussianStressFormatter),
            Arc::new(SentenceAwareChunker),
        );
        let processors: Vec<Arc<dyn TextProcessor>> = vec![Arc::new(StressAddingProcessor)];

        let result = pipeline
            .execute(
                "замок",
                &processors,
                PipelineOptions {
                    stress_format: StressFormat::CleanYoOnly,
                    chunk_config: ChunkConfig::default(),
                    capture_diff: false,
                },
                None,
                None,
            )
            .expect("not cancelled");

        assert_eq!(result.raw_text, "з+амок");
        assert_eq!(result.processed_text, "замок");
        assert_eq!(
            RussianStressFormatter.format_stress(&result.raw_text, StressFormat::AcuteAccent),
            "за\u{0301}мок"
        );
    }
}

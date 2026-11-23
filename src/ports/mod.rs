use crate::domain::{
    AppResult, BookDocument, ChunkConfig, DictionaryKind, ProcessOutput, StressFormat,
};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Единая проверка флага отмены для всех слоёв (io/engine/services/ui).
pub fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

/// Single Responsibility: A compiled dictionary processor that transforms text.
/// Open/Closed: New dictionary formats can be added by implementing `TextProcessor`.
pub trait TextProcessor: Send + Sync {
    fn name(&self) -> &str;
    fn kind(&self) -> DictionaryKind;
    fn rule_count(&self) -> usize;
    fn process(&self, text: &str, capture_diff: bool) -> ProcessOutput;
    /// Interruptible variant; the default cannot be cancelled mid-step.
    fn process_with_cancel(
        &self,
        text: &str,
        capture_diff: bool,
        _cancel: Option<&AtomicBool>,
    ) -> ProcessOutput {
        self.process(text, capture_diff)
    }
}

/// Single Responsibility: Reads a book file (.txt, .fb2, .epub, .docx) into a `BookDocument`.
/// The cancel-aware variant is the required method; the plain one is a default
/// convenience so implementers only write the interruptible path once.
pub trait DocumentReader: Send + Sync {
    fn can_read(&self, path: &Path) -> bool;
    /// Reads a document and aborts between large file/archive/XML work when
    /// the flag is set.
    fn read_document_with_cancel(
        &self,
        path: &Path,
        cancel: Option<&AtomicBool>,
    ) -> AppResult<BookDocument>;
    fn read_document(&self, path: &Path) -> AppResult<BookDocument> {
        self.read_document_with_cancel(path, None)
    }
}

/// Single Responsibility: Formats stress marks in the processed text for the target TTS engine.
pub trait StressFormatter: Send + Sync {
    fn format_stress(&self, text: &str, format: StressFormat) -> String;
}

/// Single Responsibility: Splits processed text into sentence-safe chunks for TTS.
pub trait TextChunker: Send + Sync {
    fn split_into_chunks(&self, text: &str, config: &ChunkConfig) -> Vec<String>;
}

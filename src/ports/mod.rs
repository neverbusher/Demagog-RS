use crate::domain::{BookDocument, ChunkConfig, DictionaryKind, ProcessOutput, StressFormat};
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Single Responsibility: A compiled dictionary processor that transforms text.
/// Open/Closed: New dictionary formats can be added by implementing `TextProcessor`.
pub trait TextProcessor: Send + Sync {
    fn name(&self) -> &str;
    fn kind(&self) -> DictionaryKind;
    fn rule_count(&self) -> usize;
    fn process(&self, text: &str, capture_diff: bool) -> ProcessOutput;
}

/// Single Responsibility: Reads a book file (.txt, .fb2, .epub, .docx) into a `BookDocument`.
pub trait DocumentReader: Send + Sync {
    fn can_read(&self, path: &Path) -> bool;
    fn read_document(&self, path: &Path) -> Result<BookDocument, String>;
    /// Reads a document and aborts between large file/archive/XML work when
    /// the flag is set. The default is for readers that cannot cancel while
    /// a single small file is being decoded.
    fn read_document_with_cancel(
        &self,
        path: &Path,
        _cancel: Option<&AtomicBool>,
    ) -> Result<BookDocument, String> {
        self.read_document(path)
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

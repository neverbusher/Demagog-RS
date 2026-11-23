//! Reading supported book formats into a [`BookDocument`].
//!
//! [`CompositeBookReader`] is the facade; XML parsing, archive limits and the
//! per-format readers live in submodules.

mod docx;
mod epub;
mod fb2;
mod limits;
mod txt;
mod xml;

use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::domain::BookDocument;
use crate::io::paths::book_stem;
use crate::ports::{DocumentReader, is_cancelled as book_cancelled};

pub use docx::DocxDocumentReader;
pub use epub::EpubDocumentReader;
pub use fb2::Fb2DocumentReader;
pub use txt::TxtDocumentReader;

pub const SUPPORTED_BOOK_FORMATS: &str =
    "Поддерживаются TXT, MD, LOG, FB2, EPUB, DOCX и ZIP с FB2 внутри.";

pub fn can_read_book(path: &Path) -> bool {
    CompositeBookReader::new().can_read(path)
}

/// Default title for books whose metadata has no title: the file stem with a
/// compound `.fb2.zip` suffix removed.
fn document_title_from_path(path: &Path) -> String {
    book_stem(path)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "Без названия".to_string())
}

pub struct CompositeBookReader {
    readers: Vec<Box<dyn DocumentReader>>,
}

impl CompositeBookReader {
    pub fn new() -> Self {
        Self {
            readers: vec![
                Box::new(TxtDocumentReader),
                Box::new(Fb2DocumentReader),
                Box::new(EpubDocumentReader),
                Box::new(DocxDocumentReader),
            ],
        }
    }
}

impl DocumentReader for CompositeBookReader {
    fn can_read(&self, path: &Path) -> bool {
        self.readers.iter().any(|r| r.can_read(path))
    }

    fn read_document_with_cancel(
        &self,
        path: &Path,
        cancel: Option<&AtomicBool>,
    ) -> Result<BookDocument, String> {
        for r in &self.readers {
            if book_cancelled(cancel) {
                return limits::cancelled_book();
            }
            if r.can_read(path) {
                return r.read_document_with_cancel(path, cancel);
            }
        }
        Err(format!(
            "Формат файла не поддерживается. {SUPPORTED_BOOK_FORMATS}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_book_paths_case_insensitively() {
        for name in [
            "book.txt",
            "book.MD",
            "book.log",
            "book.FB2",
            "book.fb2.zip",
            "book.zip",
            "book.epub",
            "book.docx",
        ] {
            assert!(can_read_book(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn rejects_unsupported_book_paths() {
        for name in ["cover.png", "book", "book.rtf"] {
            assert!(!can_read_book(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn title_strips_compound_fb2_zip_extension() {
        assert_eq!(
            document_title_from_path(Path::new("Война и мир.fb2.zip")),
            "Война и мир"
        );
        assert_eq!(document_title_from_path(Path::new("BOOK.FB2.ZIP")), "BOOK");
        assert_eq!(document_title_from_path(Path::new("book.zip")), "book");
        assert_eq!(document_title_from_path(Path::new("book.fb2")), "book");
    }
}

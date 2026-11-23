use std::fs::File;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::domain::{AppError, AppResult, BookDocument};
use crate::io::encoding::{decode_text_bytes, looks_like_text_bytes};
use crate::ports::{DocumentReader, is_cancelled as book_cancelled};

use super::SUPPORTED_BOOK_FORMATS;
use super::document_title_from_path;
use super::limits::{MAX_ARCHIVE_ENTRY_BYTES, cancelled_book, read_stream_with_cancel};

pub struct TxtDocumentReader;

impl DocumentReader for TxtDocumentReader {
    fn can_read(&self, path: &Path) -> bool {
        matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .as_deref(),
            Some("txt" | "md" | "log")
        )
    }

    fn read_document_with_cancel(
        &self,
        path: &Path,
        cancel: Option<&AtomicBool>,
    ) -> AppResult<BookDocument> {
        if book_cancelled(cancel) {
            return cancelled_book();
        }
        let file =
            File::open(path).map_err(|e| AppError::io(format!("Ошибка чтения файла: {e}")))?;
        let bytes = read_stream_with_cancel(
            file,
            path.display().to_string().as_str(),
            MAX_ARCHIVE_ENTRY_BYTES,
            cancel,
        )?;
        if !looks_like_text_bytes(&bytes) {
            return Err(AppError::invalid_data(format!(
                "Файл не похож на текст. {SUPPORTED_BOOK_FORMATS}"
            )));
        }
        let (text, _) = decode_text_bytes(&bytes);
        if text.trim().is_empty() {
            return Err(AppError::invalid_data(
                "Файл пуст или не содержит читаемого текста.",
            ));
        }
        Ok(BookDocument {
            title: document_title_from_path(path),
            source_path: Some(path.to_path_buf()),
            raw_text: text,
            processed_text: String::new(),
            chunks: Vec::new(),
            headings: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::FailureKind;
    use crate::io::book_reader::test_temp_path;

    #[test]
    fn invalid_txt_content_has_invalid_data_kind() {
        for (label, bytes, expected_message) in [
            (
                "binary",
                b"\0\x01\x02\x03".as_slice(),
                format!("Файл не похож на текст. {SUPPORTED_BOOK_FORMATS}"),
            ),
            (
                "empty",
                b" \n\t".as_slice(),
                "Файл пуст или не содержит читаемого текста.".to_string(),
            ),
        ] {
            let path = test_temp_path(&format!("txt-{label}"), "txt");
            std::fs::write(&path, bytes).expect("write invalid TXT fixture");

            let error = TxtDocumentReader
                .read_document(&path)
                .expect_err("invalid TXT content must fail");

            assert_eq!(error.failure_kind(), Some(FailureKind::InvalidData));
            assert_eq!(error.to_string(), expected_message);
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn txt_documents_have_no_headings() {
        let dir = std::env::temp_dir().join(format!("demagog-txt-head-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("book.txt");
        std::fs::write(&path, "Глава 1\nТекст.").expect("write txt");
        let doc = TxtDocumentReader.read_document(&path).expect("read txt");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(doc.headings.is_empty());
    }
}

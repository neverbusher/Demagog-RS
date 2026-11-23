use std::fs::File;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use zip::ZipArchive;

use crate::domain::{AppResult, BookDocument};
use crate::ports::{DocumentReader, is_cancelled as book_cancelled};

use super::document_title_from_path;
use super::limits::{
    MAX_ARCHIVE_ENTRY_BYTES, cancelled_book, read_limited_with_cancel, read_stream_with_cancel,
};
use super::xml::{decode_xml_with_declared_encoding, extract_xml_paragraphs, xml_element_text};

pub struct Fb2DocumentReader;

impl DocumentReader for Fb2DocumentReader {
    fn can_read(&self, path: &Path) -> bool {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        name.ends_with(".fb2") || name.ends_with(".zip")
    }

    fn read_document_with_cancel(
        &self,
        path: &Path,
        cancel: Option<&AtomicBool>,
    ) -> AppResult<BookDocument> {
        if book_cancelled(cancel) {
            return cancelled_book();
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bytes = if name.ends_with(".zip") {
            let file = File::open(path).map_err(|e| format!("Ошибка открытия архива: {e}"))?;
            let mut archive = ZipArchive::new(file).map_err(|e| format!("Ошибка ZIP: {e}"))?;
            let mut fb2_bytes = Vec::new();
            for i in 0..archive.len() {
                if book_cancelled(cancel) {
                    return cancelled_book();
                }
                let entry = archive.by_index(i).map_err(|e| e.to_string())?;
                let entry_name = entry.name().to_string();
                if entry_name.to_ascii_lowercase().ends_with(".fb2") {
                    if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
                        return Err(format!(
                            "FB2 в архиве больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт"
                        )
                        .into());
                    }
                    fb2_bytes = read_limited_with_cancel(
                        entry,
                        &entry_name,
                        MAX_ARCHIVE_ENTRY_BYTES,
                        cancel,
                    )?;
                    break;
                }
            }
            if fb2_bytes.is_empty() {
                return Err("Внутри архива не найден .fb2 файл".into());
            }
            fb2_bytes
        } else {
            if book_cancelled(cancel) {
                return cancelled_book();
            }
            let file = std::fs::File::open(path).map_err(|e| format!("Ошибка чтения FB2: {e}"))?;
            read_stream_with_cancel(
                file,
                &path.display().to_string(),
                MAX_ARCHIVE_ENTRY_BYTES,
                cancel,
            )?
        };

        if book_cancelled(cancel) {
            return cancelled_book();
        }
        let xml = decode_xml_with_declared_encoding(&bytes);
        let mut headings = Vec::new();
        let text = extract_xml_paragraphs(
            &xml,
            &["p", "v", "subtitle", "title", "text-author"],
            &["binary", "description"],
            cancel,
            Some(&mut headings),
        )?;
        if text.trim().is_empty() {
            return Err("В FB2 не найден текст книги.".into());
        }
        let title =
            xml_element_text(&xml, b"book-title").unwrap_or_else(|| document_title_from_path(path));
        Ok(BookDocument {
            title,
            source_path: Some(path.to_path_buf()),
            raw_text: text,
            processed_text: String::new(),
            chunks: Vec::new(),
            headings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fb2_xml_parsing_honors_cancel_flag() {
        let cancel = AtomicBool::new(true);
        let xml = "<body><p>Глава</p></body>";
        let error = extract_xml_paragraphs(xml, &["p"], &[], Some(&cancel), None)
            .expect_err("cancelled parsing must report cancellation");
        assert!(error.is_cancelled(), "{error}");
    }

    #[test]
    fn extracts_fb2_paragraphs_and_skips_service_sections() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<FictionBook><description><title-info><book-title>Служебное</book-title></title-info></description>
<body><section><title><p>Глава</p></title><p>Первый абзац.</p><p>Второй абзац.</p></section></body>
<binary>AAEC</binary></FictionBook>"#;

        let text = extract_xml_paragraphs(
            xml,
            &["p", "v", "subtitle", "title", "text-author"],
            &["binary", "description"],
            None,
            None,
        )
        .expect("well-formed FB2");

        assert!(text.contains("Глава"));
        assert!(text.contains("Первый абзац."));
        assert!(text.contains("Второй абзац."));
        assert!(!text.contains("Служебное"));
        assert!(!text.contains("AAEC"));
    }

    #[test]
    fn malformed_fb2_reports_an_error_instead_of_empty_text() {
        let xml = "<FictionBook><body><p>Текст</body></FictionBook>";
        assert!(
            extract_xml_paragraphs(xml, &["p"], &["description"], None, None).is_err(),
            "broken XML must not silently become an empty book"
        );
    }

    #[test]
    fn fb2_captures_section_titles_as_headings() {
        let dir = std::env::temp_dir().join(format!("demagog-fb2-head-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("book.fb2");
        std::fs::write(
            &path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<FictionBook><description><title-info><book-title>Книга</book-title></title-info></description>
<body><section><title><p>Глава первая</p></title><p>Текст один.</p></section>
<section><title><p>Глава вторая</p></title><p>Текст два.</p></section></body></FictionBook>"#,
        )
        .expect("write fb2");
        let doc = Fb2DocumentReader.read_document(&path).expect("read fb2");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(doc.headings.len(), 2);
        assert_eq!(doc.headings[0].title, "Глава первая");
        assert!(doc.raw_text[doc.headings[0].offset..].starts_with("Глава первая"));
        assert_eq!(doc.headings[1].title, "Глава вторая");
        assert!(doc.raw_text[doc.headings[1].offset..].starts_with("Глава вторая"));
    }

    #[test]
    fn fb2_heading_offsets_survive_leading_trim() {
        let dir = std::env::temp_dir().join(format!("demagog-fb2-trim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("book.fb2");
        std::fs::write(
            &path,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<FictionBook><body>   \n  <section><title><p>Глава</p></title><p>Текст.</p></section></body></FictionBook>",
        )
        .expect("write fb2");
        let doc = Fb2DocumentReader.read_document(&path).expect("read fb2");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(doc.headings.len(), 1);
        assert!(doc.raw_text[doc.headings[0].offset..].starts_with("Глава"));
    }
}

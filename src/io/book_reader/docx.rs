use std::fs::File;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::ZipArchive;

use crate::domain::{AppError, AppResult, BookDocument};
use crate::io::encoding::decode_text_bytes;
use crate::ports::{DocumentReader, is_cancelled as book_cancelled};

use super::document_title_from_path;
use super::limits::{
    MAX_ARCHIVE_ENTRY_BYTES, cancelled_book, read_limited_with_cancel, read_zip_entry_optional,
};
use super::xml::{decode_xml_with_declared_encoding, resolve_html_entity, xml_element_text};

pub struct DocxDocumentReader;

impl DocumentReader for DocxDocumentReader {
    fn can_read(&self, path: &Path) -> bool {
        matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .as_deref(),
            Some("docx")
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
            File::open(path).map_err(|e| AppError::io(format!("Ошибка открытия DOCX: {e}")))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|e| AppError::archive(format!("Ошибка чтения ZIP/DOCX: {e}")))?;
        if book_cancelled(cancel) {
            return cancelled_book();
        }

        let title = read_zip_entry_optional(&mut archive, "docProps/core.xml", cancel)?
            .map(|bytes| decode_xml_with_declared_encoding(&bytes))
            .and_then(|xml| xml_element_text(&xml, b"title"))
            .unwrap_or_else(|| document_title_from_path(path));
        if book_cancelled(cancel) {
            return cancelled_book();
        }

        let doc_xml = {
            let entry = match archive.by_name("word/document.xml") {
                Ok(entry) => entry,
                Err(zip::result::ZipError::FileNotFound) => {
                    return Err(AppError::invalid_data("Не найден word/document.xml в DOCX"));
                }
                Err(_) => {
                    return Err(AppError::archive("Не найден word/document.xml в DOCX"));
                }
            };
            if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
                return Err(AppError::limit_exceeded(format!(
                    "word/document.xml больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт"
                )));
            }
            read_limited_with_cancel(entry, "word/document.xml", MAX_ARCHIVE_ENTRY_BYTES, cancel)?
        };

        if book_cancelled(cancel) {
            return cancelled_book();
        }

        let (xml_str, _) = decode_text_bytes(&doc_xml);
        let mut reader = Reader::from_str(&xml_str);
        reader.config_mut().trim_text(false);

        let mut out = String::new();
        let mut in_text = false;
        // mc:Fallback repeats the text of mc:Choice; keep only the Choice branch.
        let mut fallback_depth = 0usize;
        loop {
            if book_cancelled(cancel) {
                return cancelled_book();
            }
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let local = e.local_name();
                    if local.as_ref() == b"Fallback" {
                        fallback_depth += 1;
                    } else if local.as_ref() == b"t" && fallback_depth == 0 {
                        in_text = true;
                    }
                }
                Ok(Event::Empty(e)) => {
                    if fallback_depth == 0 {
                        match e.local_name().as_ref() {
                            b"br" | b"cr" => out.push('\n'),
                            b"tab" => out.push('\t'),
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(e)) => {
                    let local = e.local_name();
                    if local.as_ref() == b"Fallback" {
                        fallback_depth = fallback_depth.saturating_sub(1);
                    } else if local.as_ref() == b"t" {
                        in_text = false;
                    } else if local.as_ref() == b"p" && fallback_depth == 0 && !out.ends_with('\n')
                    {
                        out.push('\n');
                    }
                }
                Ok(Event::Text(t)) if in_text => match t.unescape_with(resolve_html_entity) {
                    Ok(unescaped) => out.push_str(&unescaped),
                    Err(_) => out.push_str(&String::from_utf8_lossy(&t)),
                },
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(AppError::xml(format!("Ошибка разбора document.xml: {e}")));
                }
                _ => {}
            }
        }

        let text = out.trim().to_string();
        if text.is_empty() {
            return Err(AppError::invalid_data("В DOCX не найден текст."));
        }
        Ok(BookDocument {
            title,
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
    use crate::io::book_reader::limits::{write_zip_entry, write_zip_entry_with_declared_size};
    use crate::io::book_reader::test_temp_path;

    fn write_docx_fixture(path: &Path, document_xml: &str) {
        let file = File::create(path).expect("create docx");
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("start document.xml");
        use std::io::Write;
        archive
            .write_all(document_xml.as_bytes())
            .expect("write document.xml");
        archive.finish().expect("finish docx");
    }

    #[test]
    fn broken_docx_zip_is_an_archive_failure() {
        let path = test_temp_path("docx-broken-zip", "docx");
        std::fs::write(&path, b"not a ZIP archive").expect("write broken DOCX fixture");

        let error = DocxDocumentReader
            .read_document(&path)
            .expect_err("broken DOCX ZIP must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::Archive));
        assert!(
            error.to_string().starts_with("Ошибка чтения ZIP/DOCX: "),
            "{error}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_docx_document_is_invalid_data() {
        let path = test_temp_path("docx-missing-document", "docx");
        write_zip_entry(&path, "docProps/core.xml", b"<core/>");

        let error = DocxDocumentReader
            .read_document(&path)
            .expect_err("missing document.xml must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::InvalidData));
        assert_eq!(error.to_string(), "Не найден word/document.xml в DOCX");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn oversized_docx_document_is_a_limit_failure() {
        let path = test_temp_path("docx-oversized", "docx");
        write_zip_entry_with_declared_size(
            &path,
            "word/document.xml",
            b"<document/>",
            MAX_ARCHIVE_ENTRY_BYTES + 1,
        );

        let error = DocxDocumentReader
            .read_document(&path)
            .expect_err("oversized document.xml must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::LimitExceeded));
        assert_eq!(
            error.to_string(),
            format!("word/document.xml больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn malformed_docx_document_has_xml_failure_kind() {
        let path = test_temp_path("docx-malformed", "docx");
        write_docx_fixture(
            &path,
            "<w:document><w:body><w:p><w:t>Текст</w:body></w:document>",
        );

        let error = DocxDocumentReader
            .read_document(&path)
            .expect_err("malformed document.xml must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::Xml));
        assert!(
            error
                .to_string()
                .starts_with("Ошибка разбора document.xml: "),
            "{error}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn empty_docx_text_is_invalid_data() {
        let path = test_temp_path("docx-empty", "docx");
        write_docx_fixture(&path, "<w:document><w:body/></w:document>");

        let error = DocxDocumentReader
            .read_document(&path)
            .expect_err("empty DOCX text must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::InvalidData));
        assert_eq!(error.to_string(), "В DOCX не найден текст.");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn docx_preserves_line_breaks_and_tabs() {
        let dir = std::env::temp_dir().join(format!("demagog-docx-br-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("book.docx");
        write_docx_fixture(
            &path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Первая</w:t></w:r><w:r><w:br/></w:r><w:r><w:t>Вторая</w:t></w:r><w:r><w:cr/></w:r><w:r><w:t>Третья</w:t></w:r></w:p><w:p><w:r><w:t>Лево</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>Право</w:t></w:r></w:p></w:body></w:document>"#,
        );
        let doc = DocxDocumentReader.read_document(&path).expect("read docx");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(doc.raw_text, "Первая\nВторая\nТретья\nЛево\tПраво");
    }

    #[test]
    fn docx_alternate_content_keeps_single_copy() {
        let dir = std::env::temp_dir().join(format!("demagog-docx-mc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("book.docx");
        write_docx_fixture(
            &path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:body><w:p><mc:AlternateContent><mc:Choice Requires="wps"><w:r><w:t>Дубль</w:t></w:r></mc:Choice><mc:Fallback><w:p><w:r><w:t>Дубль</w:t></w:r></w:p></mc:Fallback></mc:AlternateContent></w:p></w:body></w:document>"#,
        );
        let doc = DocxDocumentReader.read_document(&path).expect("read docx");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(doc.raw_text, "Дубль");
    }
}

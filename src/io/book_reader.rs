use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

use crate::domain::BookDocument;
use crate::io::encoding::{decode_text_bytes, looks_like_text_bytes};
use crate::ports::DocumentReader;

pub const SUPPORTED_BOOK_FORMATS: &str =
    "Поддерживаются TXT, MD, LOG, FB2, EPUB, DOCX и ZIP с FB2 внутри.";

pub fn can_read_book(path: &Path) -> bool {
    CompositeBookReader::new().can_read(path)
}

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

    fn read_document(&self, path: &Path) -> Result<BookDocument, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("Ошибка чтения файла: {e}"))?;
        if !looks_like_text_bytes(&bytes) {
            return Err(format!("Файл не похож на текст. {SUPPORTED_BOOK_FORMATS}"));
        }
        let (text, _) = decode_text_bytes(&bytes);
        if text.trim().is_empty() {
            return Err("Файл пуст или не содержит читаемого текста.".to_string());
        }
        Ok(BookDocument {
            title: file_stem_title(path),
            source_path: Some(path.to_path_buf()),
            raw_text: text,
            processed_text: String::new(),
            chunks: Vec::new(),
        })
    }
}

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

    fn read_document(&self, path: &Path) -> Result<BookDocument, String> {
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
                let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
                if entry.name().to_ascii_lowercase().ends_with(".fb2") {
                    entry
                        .read_to_end(&mut fb2_bytes)
                        .map_err(|e| e.to_string())?;
                    break;
                }
            }
            if fb2_bytes.is_empty() {
                return Err("Внутри архива не найден .fb2 файл".into());
            }
            fb2_bytes
        } else {
            std::fs::read(path).map_err(|e| format!("Ошибка чтения FB2: {e}"))?
        };

        let text = extract_xml_paragraphs(
            &bytes,
            &["p", "v", "subtitle", "title", "text-author"],
            &["binary", "description"],
        )?;
        if text.trim().is_empty() {
            return Err("В FB2 не найден текст книги.".to_string());
        }
        Ok(BookDocument {
            title: file_stem_title(path),
            source_path: Some(path.to_path_buf()),
            raw_text: text,
            processed_text: String::new(),
            chunks: Vec::new(),
        })
    }
}

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

    fn read_document(&self, path: &Path) -> Result<BookDocument, String> {
        let file = File::open(path).map_err(|e| format!("Ошибка открытия DOCX: {e}"))?;
        let mut archive =
            ZipArchive::new(file).map_err(|e| format!("Ошибка чтения ZIP/DOCX: {e}"))?;
        let mut doc_xml = Vec::new();
        {
            let mut entry = archive
                .by_name("word/document.xml")
                .map_err(|_| "Не найден word/document.xml в DOCX".to_string())?;
            entry.read_to_end(&mut doc_xml).map_err(|e| e.to_string())?;
        }
        let (xml_str, _) = decode_text_bytes(&doc_xml);
        let mut reader = Reader::from_str(&xml_str);
        reader.config_mut().trim_text(false);

        let mut out = String::new();
        let mut in_text = false;
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let local = e.local_name();
                    if local.as_ref() == b"t" {
                        in_text = true;
                    }
                }
                Ok(Event::End(e)) => {
                    let local = e.local_name();
                    if local.as_ref() == b"t" {
                        in_text = false;
                    } else if local.as_ref() == b"p" && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                Ok(Event::Text(t)) if in_text => {
                    if let Ok(unescaped) = t.unescape() {
                        out.push_str(&unescaped);
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("Ошибка разбора document.xml: {e}")),
                _ => {}
            }
        }

        let text = out.trim().to_string();
        if text.is_empty() {
            return Err("В DOCX не найден текст.".to_string());
        }
        Ok(BookDocument {
            title: file_stem_title(path),
            source_path: Some(path.to_path_buf()),
            raw_text: text,
            processed_text: String::new(),
            chunks: Vec::new(),
        })
    }
}

pub struct EpubDocumentReader;

impl DocumentReader for EpubDocumentReader {
    fn can_read(&self, path: &Path) -> bool {
        matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .as_deref(),
            Some("epub")
        )
    }

    fn read_document(&self, path: &Path) -> Result<BookDocument, String> {
        let file = File::open(path).map_err(|e| format!("Ошибка открытия EPUB: {e}"))?;
        let mut archive = ZipArchive::new(file).map_err(|e| format!("Ошибка чтения EPUB: {e}"))?;

        let mut html_entries: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
            .filter(|name| {
                let lower = name.to_ascii_lowercase();
                (lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm"))
                    && !lower.contains("toc")
                    && !lower.contains("nav")
            })
            .collect();
        html_entries.sort();

        let mut full_text = String::new();
        for name in html_entries {
            let mut bytes = Vec::new();
            if let Ok(mut entry) = archive.by_name(&name)
                && entry.read_to_end(&mut bytes).is_ok()
                && let Ok(part) = extract_xml_paragraphs(
                    &bytes,
                    &["p", "h1", "h2", "h3", "h4", "li", "div"],
                    &["head", "style", "script"],
                )
                && !part.trim().is_empty()
            {
                if !full_text.is_empty() {
                    full_text.push_str("\n\n");
                }
                full_text.push_str(part.trim());
            }
        }

        if full_text.trim().is_empty() {
            return Err("В EPUB не найден текст: файлы внутри повреждены или пусты.".to_string());
        }
        Ok(BookDocument {
            title: file_stem_title(path),
            source_path: Some(path.to_path_buf()),
            raw_text: full_text,
            processed_text: String::new(),
            chunks: Vec::new(),
        })
    }
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

    fn read_document(&self, path: &Path) -> Result<BookDocument, String> {
        for r in &self.readers {
            if r.can_read(path) {
                return r.read_document(path);
            }
        }
        Err(format!(
            "Формат файла не поддерживается. {SUPPORTED_BOOK_FORMATS}"
        ))
    }
}

fn file_stem_title(path: &Path) -> String {
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let base = if file_name.to_ascii_lowercase().ends_with(".fb2.zip") {
        &file_name[..file_name.len() - ".fb2.zip".len()]
    } else {
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("")
    };
    if base.is_empty() {
        "Без названия".to_string()
    } else {
        base.to_string()
    }
}

fn extract_xml_paragraphs(
    bytes: &[u8],
    block_tags: &[&str],
    skip_tags: &[&str],
) -> Result<String, String> {
    let xml_str = decode_xml_with_declared_encoding(bytes);
    let mut reader = Reader::from_str(&xml_str);
    reader.config_mut().trim_text(false);

    let mut out = String::with_capacity(xml_str.len() / 2);
    let mut skip_depth = 0usize;
    let mut in_body = !xml_str.contains("<body");

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let local_bytes = e.local_name();
                let local = std::str::from_utf8(local_bytes.as_ref())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if local == "body" {
                    in_body = true;
                }
                if skip_tags.iter().any(|&t| t == local) {
                    skip_depth += 1;
                } else if skip_depth == 0
                    && in_body
                    && block_tags.iter().any(|&t| t == local)
                    && !out.is_empty()
                    && !out.ends_with('\n')
                {
                    out.push('\n');
                }
            }
            Ok(Event::End(e)) => {
                let local_bytes = e.local_name();
                let local = std::str::from_utf8(local_bytes.as_ref())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if skip_tags.iter().any(|&t| t == local) {
                    skip_depth = skip_depth.saturating_sub(1);
                } else if skip_depth == 0
                    && in_body
                    && block_tags.iter().any(|&t| t == local)
                    && !out.ends_with('\n')
                {
                    out.push('\n');
                }
            }
            Ok(Event::Text(t)) if skip_depth == 0 && in_body => {
                if let Ok(unescaped) = t.unescape() {
                    out.push_str(&unescaped);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("Ошибка разбора XML: {e}")),
            _ => {}
        }
    }

    Ok(out.trim().to_string())
}

fn decode_xml_with_declared_encoding(bytes: &[u8]) -> String {
    let header_len = bytes.len().min(256);
    let header_ascii = String::from_utf8_lossy(&bytes[..header_len]).to_ascii_lowercase();
    if header_ascii.contains("windows-1251") || header_ascii.contains("cp1251") {
        let (cow, _, _) = encoding_rs::WINDOWS_1251.decode(bytes);
        return crate::io::encoding::normalize_line_endings(cow.into_owned());
    }
    decode_text_bytes(bytes).0
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
            file_stem_title(Path::new("Война и мир.fb2.zip")),
            "Война и мир"
        );
        assert_eq!(file_stem_title(Path::new("BOOK.FB2.ZIP")), "BOOK");
        assert_eq!(file_stem_title(Path::new("book.zip")), "book");
        assert_eq!(file_stem_title(Path::new("book.fb2")), "book");
    }

    #[test]
    fn extracts_fb2_paragraphs_and_skips_service_sections() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<FictionBook><description><title-info><book-title>Служебное</book-title></title-info></description>
<body><section><title><p>Глава</p></title><p>Первый абзац.</p><p>Второй абзац.</p></section></body>
<binary>AAEC</binary></FictionBook>"#;

        let text = extract_xml_paragraphs(
            xml.as_bytes(),
            &["p", "v", "subtitle", "title", "text-author"],
            &["binary", "description"],
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
            extract_xml_paragraphs(xml.as_bytes(), &["p"], &["description"]).is_err(),
            "broken XML must not silently become an empty book"
        );
    }
}

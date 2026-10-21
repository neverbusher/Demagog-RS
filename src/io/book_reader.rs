use std::fs::File;
use std::io::Read;
use std::path::Path;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::ZipArchive;

use crate::domain::BookDocument;
use crate::io::encoding::decode_text_bytes;
use crate::ports::DocumentReader;

pub struct TxtDocumentReader;

impl DocumentReader for TxtDocumentReader {
    fn can_read(&self, path: &Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
            Some("txt" | "md" | "log")
        )
    }

    fn read_document(&self, path: &Path) -> Result<BookDocument, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("Ошибка чтения файла: {e}"))?;
        let (text, _) = decode_text_bytes(&bytes);
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
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        name.ends_with(".fb2") || name.ends_with(".fb2.zip")
    }

    fn read_document(&self, path: &Path) -> Result<BookDocument, String> {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        let bytes = if name.ends_with(".zip") {
            let file = File::open(path).map_err(|e| format!("Ошибка открытия архива: {e}"))?;
            let mut archive = ZipArchive::new(file).map_err(|e| format!("Ошибка ZIP: {e}"))?;
            let mut fb2_bytes = Vec::new();
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
                if entry.name().to_ascii_lowercase().ends_with(".fb2") {
                    entry.read_to_end(&mut fb2_bytes).map_err(|e| e.to_string())?;
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

        let text = extract_xml_paragraphs(&bytes, &["p", "v", "subtitle", "title", "text-author"], &["binary", "description"])?;
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
            path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
            Some("docx")
        )
    }

    fn read_document(&self, path: &Path) -> Result<BookDocument, String> {
        let file = File::open(path).map_err(|e| format!("Ошибка открытия DOCX: {e}"))?;
        let mut archive = ZipArchive::new(file).map_err(|e| format!("Ошибка чтения ZIP/DOCX: {e}"))?;
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
                    } else if local.as_ref() == b"p" {
                        if !out.ends_with('\n') {
                            out.push('\n');
                        }
                    }
                }
                Ok(Event::Text(t)) if in_text => {
                    if let Ok(unescaped) = t.unescape() {
                        out.push_str(&unescaped);
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }

        Ok(BookDocument {
            title: file_stem_title(path),
            source_path: Some(path.to_path_buf()),
            raw_text: out.trim().to_string(),
            processed_text: String::new(),
            chunks: Vec::new(),
        })
    }
}

pub struct EpubDocumentReader;

impl DocumentReader for EpubDocumentReader {
    fn can_read(&self, path: &Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
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
            if let Ok(mut entry) = archive.by_name(&name) {
                if entry.read_to_end(&mut bytes).is_ok() {
                    if let Ok(part) = extract_xml_paragraphs(
                        &bytes,
                        &["p", "h1", "h2", "h3", "h4", "li", "div"],
                        &["head", "style", "script"],
                    ) {
                        if !part.trim().is_empty() {
                            if !full_text.is_empty() {
                                full_text.push_str("\n\n");
                            }
                            full_text.push_str(part.trim());
                        }
                    }
                }
            }
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
        TxtDocumentReader.read_document(path)
    }
}

fn file_stem_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Без названия")
        .to_string()
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
                let local = std::str::from_utf8(local_bytes.as_ref()).unwrap_or("").to_ascii_lowercase();
                if local == "body" {
                    in_body = true;
                }
                if skip_tags.iter().any(|&t| t == local) {
                    skip_depth += 1;
                } else if skip_depth == 0 && in_body && block_tags.iter().any(|&t| t == local) {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
            }
            Ok(Event::End(e)) => {
                let local_bytes = e.local_name();
                let local = std::str::from_utf8(local_bytes.as_ref()).unwrap_or("").to_ascii_lowercase();
                if skip_tags.iter().any(|&t| t == local) {
                    skip_depth = skip_depth.saturating_sub(1);
                } else if skip_depth == 0 && in_body && block_tags.iter().any(|&t| t == local) {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
            }
            Ok(Event::Text(t)) if skip_depth == 0 && in_body => {
                if let Ok(unescaped) = t.unescape() {
                    out.push_str(&unescaped);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
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

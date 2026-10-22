use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

use crate::domain::BookDocument;
use crate::io::encoding::{decode_text_bytes, looks_like_text_bytes};
use crate::ports::DocumentReader;

pub const SUPPORTED_BOOK_FORMATS: &str =
    "Поддерживаются TXT, MD, LOG, FB2, EPUB, DOCX и ZIP с FB2 внутри.";

const MAX_ARCHIVE_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARCHIVE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

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
            title: document_title_from_path(path),
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
                let entry = archive.by_index(i).map_err(|e| e.to_string())?;
                let entry_name = entry.name().to_string();
                if entry_name.to_ascii_lowercase().ends_with(".fb2") {
                    if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
                        return Err(format!(
                            "FB2 в архиве больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт"
                        ));
                    }
                    fb2_bytes = read_limited(entry, &entry_name, MAX_ARCHIVE_ENTRY_BYTES)?;
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
        let title = xml_element_text(&bytes, b"book-title")
            .unwrap_or_else(|| document_title_from_path(path));
        Ok(BookDocument {
            title,
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
        let title = read_zip_entry_optional(&mut archive, "docProps/core.xml")?
            .and_then(|bytes| xml_element_text(&bytes, b"title"))
            .unwrap_or_else(|| document_title_from_path(path));
        let doc_xml = {
            let entry = archive
                .by_name("word/document.xml")
                .map_err(|_| "Не найден word/document.xml в DOCX".to_string())?;
            if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
                return Err(format!(
                    "word/document.xml больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт"
                ));
            }
            read_limited(entry, "word/document.xml", MAX_ARCHIVE_ENTRY_BYTES)?
        };
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
            title,
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
        let EpubContents { entries, title } = epub_contents(&mut archive)?;
        let title = title.unwrap_or_else(|| document_title_from_path(path));

        let mut full_text = String::new();
        let mut total_bytes: u64 = 0;
        for name in entries {
            let bytes = read_epub_zip_entry(&mut archive, &name)?;
            total_bytes = total_bytes.saturating_add(bytes.len() as u64);
            if total_bytes > MAX_ARCHIVE_TOTAL_BYTES {
                return Err(format!(
                    "Суммарный размер распакованных глав больше допустимых {MAX_ARCHIVE_TOTAL_BYTES} байт"
                ));
            }
            let part = extract_xml_paragraphs(
                &bytes,
                &["p", "h1", "h2", "h3", "h4", "li", "div"],
                &["head", "style", "script"],
            )
            .map_err(|e| format!("Ошибка разбора главы «{name}»: {e}"))?;
            if !part.trim().is_empty() {
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
            title,
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

fn document_title_from_path(path: &Path) -> String {
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

fn xml_element_text(bytes: &[u8], element: &[u8]) -> Option<String> {
    let xml = decode_xml_with_declared_encoding(bytes);
    let mut reader = Reader::from_str(&xml);
    let mut inside = false;
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == element => {
                inside = true;
                text.clear();
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == element && inside => {
                let text = text.trim();
                return (!text.is_empty()).then(|| text.to_string());
            }
            Ok(Event::Text(event)) if inside => {
                if let Ok(unescaped) = event.unescape() {
                    text.push_str(&unescaped);
                }
            }
            Ok(Event::CData(event)) if inside => {
                text.push_str(&String::from_utf8_lossy(event.as_ref()));
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

fn epub_rootfile_path(bytes: &[u8]) -> Result<Option<String>, String> {
    let xml = decode_xml_with_declared_encoding(bytes);
    let mut reader = Reader::from_str(&xml);

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.local_name().as_ref() == b"rootfile" => {
                if let Some(path) = xml_attribute(&e, b"full-path")? {
                    let path = path.trim();
                    if !path.is_empty() {
                        return Ok(Some(path.to_string()));
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("Ошибка разбора META-INF/container.xml: {e}")),
            _ => {}
        }
    }

    Ok(None)
}

fn epub_spine_paths(bytes: &[u8], opf_path: &str) -> Result<Vec<String>, String> {
    let xml = decode_xml_with_declared_encoding(bytes);
    let mut reader = Reader::from_str(&xml);
    let mut manifest: HashMap<String, (String, bool)> = HashMap::new();
    let mut spine_ids = Vec::new();
    let mut depth = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                depth += 1;
                collect_opf_element(&e, &mut manifest, &mut spine_ids, opf_path)?;
            }
            Ok(Event::Empty(e)) => {
                collect_opf_element(&e, &mut manifest, &mut spine_ids, opf_path)?;
            }
            Ok(Event::End(_)) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "Лишний закрывающий тег в OPF".to_string())?;
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("Ошибка разбора OPF: {e}")),
            _ => {}
        }
    }

    if depth != 0 {
        return Err("OPF содержит незакрытые теги".to_string());
    }

    let mut entries = Vec::new();
    for idref in spine_ids {
        let (path, is_document) = manifest
            .get(&idref)
            .ok_or_else(|| format!("В spine указан отсутствующий id «{idref}»"))?;
        if *is_document && !is_epub_navigation(path) {
            entries.push(path.clone());
        }
    }
    Ok(entries)
}

fn collect_opf_element(
    event: &BytesStart<'_>,
    manifest: &mut HashMap<String, (String, bool)>,
    spine_ids: &mut Vec<String>,
    opf_path: &str,
) -> Result<(), String> {
    match event.local_name().as_ref() {
        b"item" => {
            let Some(id) = xml_attribute(event, b"id")? else {
                return Ok(());
            };
            let Some(href) = xml_attribute(event, b"href")? else {
                return Ok(());
            };
            if href.contains("://") {
                return Ok(());
            }
            let media_type = xml_attribute(event, b"media-type")?.unwrap_or_default();
            let is_document = is_epub_document(&href, &media_type);
            manifest.insert(id, (resolve_epub_path(opf_path, &href), is_document));
        }
        b"itemref" => {
            if let Some(idref) = xml_attribute(event, b"idref")? {
                spine_ids.push(idref);
            }
        }
        _ => {}
    }
    Ok(())
}

fn xml_attribute(event: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>, String> {
    for attribute in event.attributes().with_checks(false) {
        let attribute = attribute.map_err(|e| format!("Ошибка атрибута XML: {e}"))?;
        if attribute.key.local_name().as_ref() == name {
            let value = attribute
                .unescape_value()
                .map_err(|e| format!("Ошибка значения атрибута XML: {e}"))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn resolve_epub_path(opf_path: &str, href: &str) -> String {
    let href = href.split(['#', '?']).next().unwrap_or("");
    let href = percent_decode(href);
    let href = href.trim_start_matches('/');
    let directory = opf_path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let combined = if directory.is_empty() {
        href.to_string()
    } else {
        format!("{directory}/{href}")
    };

    let mut parts = Vec::new();
    for part in combined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2]))
        {
            out.push(high * 16 + low);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_epub_document(href: &str, media_type: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    media_type.eq_ignore_ascii_case("application/xhtml+xml")
        || lower.ends_with(".xhtml")
        || lower.ends_with(".html")
        || lower.ends_with(".htm")
}

fn is_epub_navigation(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("toc") || lower.contains("nav")
}

fn read_limited<R: Read>(reader: R, name: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Ошибка чтения «{name}»: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("Файл «{name}» больше допустимых {limit} байт"));
    }
    Ok(bytes)
}

fn read_epub_zip_entry(archive: &mut ZipArchive<File>, name: &str) -> Result<Vec<u8>, String> {
    let entry = archive
        .by_name(name)
        .map_err(|e| format!("В EPUB не найден файл «{name}»: {e}"))?;
    if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
        return Err(format!(
            "В EPUB файл «{name}» больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт"
        ));
    }
    read_limited(entry, name, MAX_ARCHIVE_ENTRY_BYTES)
}

fn read_zip_entry_optional(
    archive: &mut ZipArchive<File>,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(format!("Ошибка открытия «{name}»: {error}")),
    };
    if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
        return Err(format!(
            "В EPUB файл «{name}» больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт"
        ));
    }
    read_limited(entry, name, MAX_ARCHIVE_ENTRY_BYTES).map(Some)
}

fn epub_contents(archive: &mut ZipArchive<File>) -> Result<EpubContents, String> {
    let Some(container) = read_zip_entry_optional(archive, "META-INF/container.xml")? else {
        return Ok(EpubContents {
            entries: fallback_epub_entries(archive),
            title: None,
        });
    };
    let opf_path = epub_rootfile_path(&container)?
        .ok_or_else(|| "В META-INF/container.xml не указан rootfile".to_string())?;
    let opf = read_epub_zip_entry(archive, &opf_path)?;
    let entries = epub_spine_paths(&opf, &opf_path)?;
    if entries.is_empty() {
        return Err("В OPF не найден spine с главами".to_string());
    }
    Ok(EpubContents {
        entries,
        title: xml_element_text(&opf, b"title"),
    })
}

struct EpubContents {
    entries: Vec<String>,
    title: Option<String>,
}

fn fallback_epub_entries(archive: &mut ZipArchive<File>) -> Vec<String> {
    let mut entries: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            (lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm"))
                && !is_epub_navigation(name)
        })
        .collect();
    entries.sort();
    entries
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
            document_title_from_path(Path::new("Война и мир.fb2.zip")),
            "Война и мир"
        );
        assert_eq!(document_title_from_path(Path::new("BOOK.FB2.ZIP")), "BOOK");
        assert_eq!(document_title_from_path(Path::new("book.zip")), "book");
        assert_eq!(document_title_from_path(Path::new("book.fb2")), "book");
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

    #[test]
    fn finds_the_epub_package_from_the_container() {
        let container = r#"<?xml version="1.0" encoding="utf-8"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

        assert_eq!(
            epub_rootfile_path(container.as_bytes())
                .expect("valid container")
                .as_deref(),
            Some("OEBPS/content.opf")
        );
    }

    #[test]
    fn follows_the_epub_spine_instead_of_filename_order() {
        let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="chapter10" href="Text/chapter10.xhtml" media-type="application/xhtml+xml"/>
    <item id="chapter2" href="Text/chapter2.xhtml" media-type="application/xhtml+xml"/>
    <item id="style" href="Styles/style.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="chapter2"/>
    <itemref idref="chapter10"/>
  </spine>
</package>"#;

        assert_eq!(
            epub_spine_paths(opf.as_bytes(), "OEBPS/content.opf").expect("valid OPF"),
            vec![
                "OEBPS/Text/chapter2.xhtml".to_string(),
                "OEBPS/Text/chapter10.xhtml".to_string()
            ]
        );
    }

    #[test]
    fn resolves_epub_links_relative_to_the_package() {
        assert_eq!(
            resolve_epub_path("OEBPS/content.opf", "Text/ch%201.xhtml#top"),
            "OEBPS/Text/ch 1.xhtml"
        );
        assert_eq!(
            resolve_epub_path("content.opf", "../Text/ch1.xhtml"),
            "Text/ch1.xhtml"
        );
    }

    #[test]
    fn malformed_epub_opf_reports_an_error() {
        assert!(epub_spine_paths(b"<package><manifest>", "content.opf").is_err());
    }

    struct TempEpub(std::path::PathBuf);

    impl Drop for TempEpub {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn write_test_epub(entries: &[(&str, &str)]) -> TempEpub {
        use std::io::Write;
        use std::time::{SystemTime, UNIX_EPOCH};

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "demagog-rs-epub-{}-{stamp}.epub",
            std::process::id()
        ));
        let file = File::create(&path).expect("create test EPUB");
        let mut archive = zip::ZipWriter::new(file);
        for (name, content) in entries {
            archive
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .expect("start EPUB entry");
            archive
                .write_all(content.as_bytes())
                .expect("write EPUB entry");
        }
        archive.finish().expect("finish test EPUB");
        TempEpub(path)
    }

    const TEST_EPUB_CONTAINER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    #[test]
    fn reads_epub_chapters_in_spine_order() {
        let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/" version="3.0">
  <metadata><dc:title>Настоящее название</dc:title></metadata>
  <manifest>
    <item id="chapter10" href="Text/chapter10.xhtml" media-type="application/xhtml+xml"/>
    <item id="chapter2" href="Text/chapter2.xhtml" media-type="application/xhtml+xml"/>
    <item id="style" href="Styles/style.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="chapter2"/>
    <itemref idref="chapter10"/>
  </spine>
</package>"#;
        let epub = write_test_epub(&[
            ("META-INF/container.xml", TEST_EPUB_CONTAINER),
            ("OEBPS/content.opf", opf),
            (
                "OEBPS/Text/chapter2.xhtml",
                "<html><body><p>Вторая глава.</p></body></html>",
            ),
            (
                "OEBPS/Text/chapter10.xhtml",
                "<html><body><p>Десятая глава.</p></body></html>",
            ),
        ]);

        let document = EpubDocumentReader
            .read_document(&epub.0)
            .expect("read EPUB");
        assert_eq!(document.title, "Настоящее название");
        let second = document
            .raw_text
            .find("Вторая глава")
            .expect("second chapter");
        let tenth = document
            .raw_text
            .find("Десятая глава")
            .expect("tenth chapter");
        assert!(second < tenth, "chapters must follow spine order");
    }

    #[test]
    fn missing_epub_spine_file_is_an_error() {
        let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="chapter1" href="Text/missing.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="chapter1"/></spine>
</package>"#;
        let epub = write_test_epub(&[
            ("META-INF/container.xml", TEST_EPUB_CONTAINER),
            ("OEBPS/content.opf", opf),
        ]);

        let error = EpubDocumentReader
            .read_document(&epub.0)
            .expect_err("missing chapter must not be ignored");
        assert!(error.contains("не найден файл"), "{error}");
    }

    #[test]
    fn extracts_title_text_from_xml_metadata() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>Война &amp; мир</dc:title></metadata></package>"#;
        assert_eq!(
            xml_element_text(opf.as_bytes(), b"title").as_deref(),
            Some("Война & мир")
        );

        let fb2 = r#"<FictionBook><description><title-info><book-title>Мир и война</book-title></title-info></description></FictionBook>"#;
        assert_eq!(
            xml_element_text(fb2.as_bytes(), b"book-title").as_deref(),
            Some("Мир и война")
        );
    }

    #[test]
    fn missing_xml_title_is_none() {
        assert_eq!(
            xml_element_text(b"<package><metadata></metadata></package>", b"title"),
            None
        );
    }

    #[test]
    fn limited_reader_rejects_entries_above_the_limit() {
        let exact = read_limited(std::io::Cursor::new(b"1234567890"), "chapter", 10)
            .expect("entry exactly at the limit");
        assert_eq!(exact, b"1234567890");

        let error = read_limited(std::io::Cursor::new(b"12345678901"), "chapter", 10)
            .expect_err("entry above the limit");
        assert!(error.contains("больше"), "{error}");
    }
}

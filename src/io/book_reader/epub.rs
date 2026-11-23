use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use zip::ZipArchive;

use crate::domain::{AppError, AppResult, BookDocument, BookHeading};
use crate::ports::{DocumentReader, is_cancelled as book_cancelled};

use super::document_title_from_path;
use super::limits::{
    MAX_ARCHIVE_ENTRY_BYTES, MAX_ARCHIVE_TOTAL_BYTES, cancelled_book, read_limited_with_cancel,
    read_zip_entry_optional,
};
use super::xml::{decode_xml_with_declared_encoding, extract_xml_paragraphs, xml_element_text};

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

    fn read_document_with_cancel(
        &self,
        path: &Path,
        cancel: Option<&AtomicBool>,
    ) -> AppResult<BookDocument> {
        if book_cancelled(cancel) {
            return cancelled_book();
        }
        let file =
            File::open(path).map_err(|e| AppError::io(format!("Ошибка открытия EPUB: {e}")))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|e| AppError::archive(format!("Ошибка чтения EPUB: {e}")))?;
        let EpubContents { entries, title } = epub_contents(&mut archive, cancel)?;
        let title = title.unwrap_or_else(|| document_title_from_path(path));

        let mut full_text = String::new();
        let mut headings: Vec<BookHeading> = Vec::new();
        let mut total_bytes: u64 = 0;
        for name in entries {
            if book_cancelled(cancel) {
                return cancelled_book();
            }
            let bytes = read_epub_zip_entry(&mut archive, &name, cancel)?;

            total_bytes = total_bytes.saturating_add(bytes.len() as u64);
            check_epub_total_size(total_bytes)?;
            let chapter_xml = decode_xml_with_declared_encoding(&bytes);
            let part = match extract_xml_paragraphs(
                &chapter_xml,
                &["p", "h1", "h2", "h3", "h4", "li", "div"],
                &["head", "style", "script"],
                cancel,
                None,
            ) {
                Ok(part) => part,
                Err(error) if book_cancelled(cancel) || error.is_cancelled() => {
                    return cancelled_book();
                }
                Err(error) => {
                    return Err(error.map_failure_message(|message| {
                        format!("Ошибка разбора главы «{name}»: {message}")
                    }));
                }
            };
            if !part.trim().is_empty() {
                let offset = full_text.len() + if full_text.is_empty() { 0 } else { 2 };
                let title = part.trim().lines().next().unwrap_or("").trim().to_string();
                headings.push(BookHeading { offset, title });
                if !full_text.is_empty() {
                    full_text.push_str("\n\n");
                }
                full_text.push_str(part.trim());
            }
        }

        if full_text.trim().is_empty() {
            return Err(AppError::invalid_data(
                "В EPUB не найден текст: файлы внутри повреждены или пусты.",
            ));
        }
        Ok(BookDocument {
            title,
            source_path: Some(path.to_path_buf()),
            raw_text: full_text,
            processed_text: String::new(),
            chunks: Vec::new(),
            headings,
        })
    }
}

fn read_epub_zip_entry(
    archive: &mut ZipArchive<File>,
    name: &str,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<u8>> {
    let entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(error @ zip::result::ZipError::FileNotFound) => {
            return Err(AppError::invalid_data(format!(
                "В EPUB не найден файл «{name}»: {error}"
            )));
        }
        Err(error) => {
            return Err(AppError::archive(format!(
                "В EPUB не найден файл «{name}»: {error}"
            )));
        }
    };
    if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
        return Err(AppError::limit_exceeded(format!(
            "В EPUB файл «{name}» больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт"
        )));
    }
    read_limited_with_cancel(entry, name, MAX_ARCHIVE_ENTRY_BYTES, cancel)
}

fn check_epub_total_size(total_bytes: u64) -> AppResult<()> {
    if total_bytes > MAX_ARCHIVE_TOTAL_BYTES {
        return Err(AppError::limit_exceeded(format!(
            "Суммарный размер распакованных глав больше допустимых {MAX_ARCHIVE_TOTAL_BYTES} байт"
        )));
    }
    Ok(())
}

fn epub_contents(
    archive: &mut ZipArchive<File>,
    cancel: Option<&AtomicBool>,
) -> AppResult<EpubContents> {
    let Some(container) = read_zip_entry_optional(archive, "META-INF/container.xml", cancel)?
    else {
        return Ok(EpubContents {
            entries: fallback_epub_entries(archive),
            title: None,
        });
    };
    let opf_path = epub_rootfile_path(&container)?
        .ok_or_else(|| AppError::invalid_data("В META-INF/container.xml не указан rootfile"))?;
    let opf = read_epub_zip_entry(archive, &opf_path, cancel)?;
    let entries = epub_spine_paths(&opf, &opf_path)?;
    if entries.is_empty() {
        return Err(AppError::invalid_data("В OPF не найден spine с главами"));
    }
    Ok(EpubContents {
        entries,
        title: xml_element_text(&decode_xml_with_declared_encoding(&opf), b"title"),
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

fn epub_rootfile_path(bytes: &[u8]) -> AppResult<Option<String>> {
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
            Err(e) => {
                return Err(AppError::xml(format!(
                    "Ошибка разбора META-INF/container.xml: {e}"
                )));
            }
            _ => {}
        }
    }

    Ok(None)
}

fn epub_spine_paths(bytes: &[u8], opf_path: &str) -> AppResult<Vec<String>> {
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
                    .ok_or_else(|| AppError::invalid_data("Лишний закрывающий тег в OPF"))?;
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(AppError::xml(format!("Ошибка разбора OPF: {e}")));
            }
            _ => {}
        }
    }

    if depth != 0 {
        return Err(AppError::invalid_data("OPF содержит незакрытые теги"));
    }

    let mut entries = Vec::new();
    for idref in spine_ids {
        let (path, is_document) = manifest.get(&idref).ok_or_else(|| {
            AppError::invalid_data(format!("В spine указан отсутствующий id «{idref}»"))
        })?;
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
) -> AppResult<()> {
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
            let properties = xml_attribute(event, b"properties")?.unwrap_or_default();
            let is_nav = properties.split_whitespace().any(|p| p == "nav");
            let is_document = !is_nav && is_epub_document(&href, &media_type);
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

fn xml_attribute(event: &BytesStart<'_>, name: &[u8]) -> AppResult<Option<String>> {
    for attribute in event.attributes().with_checks(false) {
        let attribute =
            attribute.map_err(|e| AppError::xml(format!("Ошибка атрибута XML: {e}")))?;
        if attribute.key.local_name().as_ref() == name {
            let value = attribute
                .unescape_value()
                .map_err(|e| AppError::xml(format!("Ошибка значения атрибута XML: {e}")))?;
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
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    // Match navigation documents by name, not by substring: a chapter called
    // "naval.xhtml" or "my-navigation.xhtml" must not be skipped.
    base.starts_with("toc.")
        || base.starts_with("nav.")
        || base == "toc.ncx"
        || base.starts_with("navigation.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::FailureKind;
    use crate::io::book_reader::limits::write_zip_entry_with_declared_size;
    use crate::io::book_reader::test_temp_path;

    #[test]
    fn epub_reading_honors_a_cancel_flag() {
        let file = File::create(
            std::env::temp_dir().join(format!("demagog-rs-cancel-{}.epub", std::process::id())),
        )
        .expect("create temp EPUB");
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "META-INF/container.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("start container");
        use std::io::Write;
        archive
            .write_all(r#"<?xml version="1.0" encoding="utf-8"?><container><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#.as_bytes())
            .expect("write container");
        archive
            .start_file("content.opf", zip::write::SimpleFileOptions::default())
            .expect("start OPF");
        archive
            .write_all(r#"<?xml version="1.0" encoding="utf-8"?><package><metadata><dc:title>Отмена</dc:title></metadata><spine><itemref idref="chapter"/></spine></package>"#.as_bytes())
            .expect("write OPF");
        archive
            .start_file(
                "Text/chapter.xhtml",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("start chapter");
        archive
            .write_all("<html><body><p>Текст</p></body></html>".as_bytes())
            .expect("write chapter");
        archive.finish().expect("finish EPUB");

        let path =
            std::env::temp_dir().join(format!("demagog-rs-cancel-{}.epub", std::process::id()));
        let cancel = AtomicBool::new(true);
        let error = EpubDocumentReader
            .read_document_with_cancel(&path, Some(&cancel))
            .expect_err("cancelled book loading must report cancellation");
        assert!(error.is_cancelled(), "{error}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn epub_entry_read_honors_cancel_flag() {
        let path =
            std::env::temp_dir().join(format!("demagog-cancel-entry-{}.epub", std::process::id()));
        let file = File::create(&path).expect("create zip");
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("chapter.xhtml", zip::write::SimpleFileOptions::default())
            .expect("start");
        use std::io::Write;
        archive
            .write_all(b"<html><body><p>text</p></body></html>")
            .expect("write");
        archive.finish().expect("finish");
        let mut archive = ZipArchive::new(File::open(&path).expect("open")).expect("read zip");
        let cancel = AtomicBool::new(true);
        let error = read_epub_zip_entry(&mut archive, "chapter.xhtml", Some(&cancel))
            .expect_err("cancelled entry read must report cancellation");
        assert!(error.is_cancelled(), "{error}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn navigation_detection_keeps_chapters_named_with_navigation() {
        assert!(!is_epub_navigation("OEBPS/my-navigation.xhtml"));
        assert!(is_epub_navigation("OEBPS/navigation.xhtml"));
    }

    #[test]
    fn epub_spine_skips_item_with_nav_properties() {
        let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
<manifest>
<item id="nav" href="content.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
</manifest>
<spine><itemref idref="nav"/><itemref idref="c1"/></spine>
</package>"#;
        let entries = epub_spine_paths(opf.as_bytes(), "OEBPS/content.opf").expect("spine");
        assert_eq!(entries, vec!["OEBPS/chapter1.xhtml".to_string()]);
    }

    #[test]
    fn navigation_detection_matches_names_not_substrings() {
        assert!(is_epub_navigation("OEBPS/nav.xhtml"));
        assert!(is_epub_navigation("OEBPS/toc.ncx"));
        assert!(is_epub_navigation("navigation.xhtml"));
        assert!(!is_epub_navigation("naval.xhtml"));
        assert!(!is_epub_navigation("chapter1.xhtml"));
        assert!(!is_epub_navigation("OEBPS/contemporary.xhtml"));
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
        let error = epub_spine_paths(b"<package><manifest>", "content.opf")
            .expect_err("unclosed OPF must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::InvalidData));
        assert_eq!(error.to_string(), "OPF содержит незакрытые теги");
    }

    #[test]
    fn malformed_epub_metadata_has_xml_failure_kind() {
        let container_error =
            epub_rootfile_path(b"<container><").expect_err("malformed container must fail");
        assert_eq!(container_error.failure_kind(), Some(FailureKind::Xml));
        assert!(
            container_error
                .to_string()
                .starts_with("Ошибка разбора META-INF/container.xml: "),
            "{container_error}"
        );

        let opf_error =
            epub_spine_paths(b"<package><", "content.opf").expect_err("malformed OPF must fail");
        assert_eq!(opf_error.failure_kind(), Some(FailureKind::Xml));
        assert!(
            opf_error.to_string().starts_with("Ошибка разбора OPF: "),
            "{opf_error}"
        );
    }

    #[test]
    fn missing_epub_spine_id_is_invalid_data() {
        let error = epub_spine_paths(
            b"<package><spine><itemref idref=\"missing\"/></spine></package>",
            "content.opf",
        )
        .expect_err("missing manifest id must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::InvalidData));
        assert_eq!(
            error.to_string(),
            "В spine указан отсутствующий id «missing»"
        );
    }

    #[test]
    fn malformed_epub_attribute_has_xml_failure_kind() {
        let mut reader = Reader::from_str("<item href=\"broken&unknown;\"/>");
        let Event::Empty(event) = reader.read_event().expect("read item event") else {
            panic!("fixture must contain an empty item element");
        };

        let error = xml_attribute(&event, b"href").expect_err("malformed attribute must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::Xml));
        assert!(
            error
                .to_string()
                .starts_with("Ошибка значения атрибута XML: "),
            "{error}"
        );
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
    fn broken_epub_zip_is_an_archive_failure() {
        let path = test_temp_path("epub-broken-zip", "epub");
        std::fs::write(&path, b"not a ZIP archive").expect("write broken EPUB fixture");

        let error = EpubDocumentReader
            .read_document(&path)
            .expect_err("broken EPUB ZIP must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::Archive));
        assert!(
            error.to_string().starts_with("Ошибка чтения EPUB: "),
            "{error}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_epub_rootfile_is_invalid_data() {
        let epub = write_test_epub(&[(
            "META-INF/container.xml",
            "<container><rootfiles/></container>",
        )]);

        let error = EpubDocumentReader
            .read_document(&epub.0)
            .expect_err("missing rootfile must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::InvalidData));
        assert_eq!(
            error.to_string(),
            "В META-INF/container.xml не указан rootfile"
        );
    }

    #[test]
    fn missing_epub_spine_is_invalid_data() {
        let epub = write_test_epub(&[
            ("META-INF/container.xml", TEST_EPUB_CONTAINER),
            ("OEBPS/content.opf", "<package><manifest/></package>"),
        ]);

        let error = EpubDocumentReader
            .read_document(&epub.0)
            .expect_err("missing spine must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::InvalidData));
        assert_eq!(error.to_string(), "В OPF не найден spine с главами");
    }

    #[test]
    fn empty_epub_text_is_invalid_data() {
        let opf = r#"<package><manifest><item id="chapter" href="Text/chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#;
        let epub = write_test_epub(&[
            ("META-INF/container.xml", TEST_EPUB_CONTAINER),
            ("OEBPS/content.opf", opf),
            (
                "OEBPS/Text/chapter.xhtml",
                "<html><body><p> \n </p></body></html>",
            ),
        ]);

        let error = EpubDocumentReader
            .read_document(&epub.0)
            .expect_err("empty EPUB text must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::InvalidData));
        assert_eq!(
            error.to_string(),
            "В EPUB не найден текст: файлы внутри повреждены или пусты."
        );
    }

    #[test]
    fn malformed_epub_chapter_preserves_xml_kind_with_context() {
        let opf = r#"<package><manifest><item id="chapter" href="Text/chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#;
        let epub = write_test_epub(&[
            ("META-INF/container.xml", TEST_EPUB_CONTAINER),
            ("OEBPS/content.opf", opf),
            (
                "OEBPS/Text/chapter.xhtml",
                "<html><body><p>Текст</body></html>",
            ),
        ]);

        let error = EpubDocumentReader
            .read_document(&epub.0)
            .expect_err("malformed chapter must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::Xml));
        assert!(
            error.to_string().starts_with(
                "Ошибка разбора главы «OEBPS/Text/chapter.xhtml»: Ошибка разбора XML: "
            ),
            "{error}"
        );
    }

    #[test]
    fn oversized_epub_chapter_is_a_limit_failure() {
        let path = test_temp_path("epub-oversized", "epub");
        write_zip_entry_with_declared_size(
            &path,
            "chapter.xhtml",
            b"<html/>",
            MAX_ARCHIVE_ENTRY_BYTES + 1,
        );

        let error = EpubDocumentReader
            .read_document(&path)
            .expect_err("oversized EPUB chapter must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::LimitExceeded));
        assert_eq!(
            error.to_string(),
            format!("В EPUB файл «chapter.xhtml» больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn epub_total_limit_preserves_message_and_kind() {
        check_epub_total_size(MAX_ARCHIVE_TOTAL_BYTES).expect("total exactly at the limit");

        let error = check_epub_total_size(MAX_ARCHIVE_TOTAL_BYTES + 1)
            .expect_err("total above the limit must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::LimitExceeded));
        assert_eq!(
            error.to_string(),
            format!(
                "Суммарный размер распакованных глав больше допустимых {MAX_ARCHIVE_TOTAL_BYTES} байт"
            )
        );
    }

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
        assert!(error.to_string().contains("не найден файл"), "{error}");
        assert_eq!(error.failure_kind(), Some(FailureKind::InvalidData));
        assert!(
            error
                .to_string()
                .starts_with("В EPUB не найден файл «OEBPS/Text/missing.xhtml»: "),
            "{error}"
        );
    }

    #[test]
    fn epub_captures_spine_documents_as_headings() {
        let dir = std::env::temp_dir().join(format!("demagog-epub-head-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("book.epub");
        let file = File::create(&path).expect("create epub");
        let mut archive = zip::ZipWriter::new(file);
        use std::io::Write;
        let opts = zip::write::SimpleFileOptions::default();
        archive.start_file("META-INF/container.xml", opts).unwrap();
        archive.write_all(r#"<?xml version="1.0" encoding="utf-8"?><container><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#.as_bytes()).unwrap();
        archive.start_file("content.opf", opts).unwrap();
        archive.write_all(r#"<?xml version="1.0" encoding="utf-8"?><package><metadata><dc:title>Книга</dc:title></metadata><manifest><item id="c1" href="Text/c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="Text/c2.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#.as_bytes()).unwrap();
        archive.start_file("Text/c1.xhtml", opts).unwrap();
        archive
            .write_all("<html><body><h1>Глава 1</h1><p>Текст один.</p></body></html>".as_bytes())
            .unwrap();
        archive.start_file("Text/c2.xhtml", opts).unwrap();
        archive
            .write_all("<html><body><h1>Глава 2</h1><p>Текст два.</p></body></html>".as_bytes())
            .unwrap();
        archive.finish().unwrap();

        let doc = EpubDocumentReader.read_document(&path).expect("read epub");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(doc.headings.len(), 2);
        assert_eq!(doc.headings[0].offset, 0);
        assert_eq!(doc.headings[0].title, "Глава 1");
        assert_eq!(doc.headings[1].title, "Глава 2");
        assert!(doc.raw_text[doc.headings[1].offset..].starts_with("Глава 2"));
    }
}

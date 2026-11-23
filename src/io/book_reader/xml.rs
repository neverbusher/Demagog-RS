use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::domain::BookHeading;
use crate::io::encoding::decode_text_bytes;
use crate::ports::is_cancelled as book_cancelled;

pub(super) fn xml_element_text(xml: &str, element: &[u8]) -> Option<String> {
    let mut reader = Reader::from_str(xml);
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

pub(super) fn extract_xml_paragraphs(
    xml_str: &str,
    block_tags: &[&str],
    skip_tags: &[&str],
    cancel: Option<&std::sync::atomic::AtomicBool>,
    mut headings: Option<&mut Vec<BookHeading>>,
) -> Result<String, String> {
    let mut reader = Reader::from_str(xml_str);
    reader.config_mut().trim_text(false);

    let mut out = String::with_capacity(xml_str.len() / 2);
    let mut skip_depth = 0usize;
    let mut in_body = !xml_str.contains("<body");
    let mut section_depth = 0usize;
    let mut title_start: Option<usize> = None;

    loop {
        if book_cancelled(cancel) {
            return Err("Загрузка книги отменена".to_string());
        }
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
                if skip_depth == 0 && in_body {
                    if local == "section" {
                        section_depth += 1;
                    }
                    if local == "title" && section_depth > 0 && headings.is_some() {
                        title_start = Some(out.len());
                    }
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
                if skip_depth == 0 && in_body {
                    if local == "section" {
                        section_depth = section_depth.saturating_sub(1);
                    }
                    if local == "title" {
                        if let (Some(start), Some(headings)) =
                            (title_start.take(), headings.as_mut())
                        {
                            if let Some(title) =
                                out[start..].lines().map(str::trim).find(|l| !l.is_empty())
                            {
                                headings.push(BookHeading {
                                    offset: start,
                                    title: title.to_string(),
                                });
                            }
                        }
                    }
                }
            }
            Ok(Event::Text(t)) if skip_depth == 0 && in_body => {
                match t.unescape_with(resolve_html_entity) {
                    Ok(unescaped) => out.push_str(&unescaped),
                    Err(_) => out.push_str(&String::from_utf8_lossy(&t)),
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("Ошибка разбора XML: {e}")),
            _ => {}
        }
    }

    let trim_start = out.len() - out.trim_start().len();
    if let Some(headings) = headings.as_mut() {
        for heading in headings.iter_mut() {
            heading.offset = heading.offset.saturating_sub(trim_start);
        }
    }
    Ok(out.trim().to_string())
}

/// Resolves XML predefined entities plus the HTML named entities common in
/// EPUB/DOCX XHTML. Unknown entities return None so the caller can fall back
/// to the raw text instead of silently dropping the whole text run.
pub(super) fn resolve_html_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{00A0}",
        "mdash" => "\u{2014}",
        "ndash" => "\u{2013}",
        "hellip" => "\u{2026}",
        "laquo" => "\u{00AB}",
        "raquo" => "\u{00BB}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "bull" => "\u{2022}",
        "copy" => "\u{00A9}",
        "reg" => "\u{00AE}",
        "trade" => "\u{2122}",
        "euro" => "\u{20AC}",
        "pound" => "\u{00A3}",
        "yen" => "\u{00A5}",
        "cent" => "\u{00A2}",
        "sect" => "\u{00A7}",
        "para" => "\u{00B6}",
        "middot" => "\u{00B7}",
        "times" => "\u{00D7}",
        "divide" => "\u{00F7}",
        "plusmn" => "\u{00B1}",
        "deg" => "\u{00B0}",
        "micro" => "\u{00B5}",
        "sup2" => "\u{00B2}",
        "sup3" => "\u{00B3}",
        "frac12" => "\u{00BD}",
        _ => return None,
    })
}

pub(super) fn decode_xml_with_declared_encoding(bytes: &[u8]) -> String {
    if let Some(encoding) = declared_xml_encoding(bytes) {
        let (cow, _, _) = encoding.decode(bytes);
        return crate::io::encoding::normalize_line_endings(cow.into_owned());
    }
    decode_text_bytes(bytes).0
}

/// Reads the encoding only from the XML declaration, so comments or text
/// mentioning an encoding name cannot override it.
fn declared_xml_encoding(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let header = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
    let header = header
        .trim_start_matches('\u{feff}')
        .trim_start()
        .to_ascii_lowercase();
    let decl = header.strip_prefix("<?xml")?;
    let decl = &decl[..decl.find("?>")?];
    let pos = decl.find("encoding=")?;
    let rest = &decl[pos + "encoding=".len()..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = rest[1..].find(quote)?;
    encoding_rs::Encoding::for_label(&rest.as_bytes()[1..1 + end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_koi8r_encoding_is_honored() {
        let (body, _, _) = encoding_rs::KOI8_R.encode("Привет");
        let mut bytes = br#"<?xml version="1.0" encoding="koi8-r"?><body><p>"#.to_vec();
        bytes.extend_from_slice(&body);
        bytes.extend_from_slice(b"</p></body>");
        let decoded = decode_xml_with_declared_encoding(&bytes);
        assert!(decoded.contains("Привет"), "{decoded}");
    }

    #[test]
    fn encoding_mention_in_comment_does_not_override_declaration() {
        let xml =
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><!-- cp1251 --><body><p>Привет</p></body>";
        let decoded = decode_xml_with_declared_encoding(xml.as_bytes());
        assert!(decoded.contains("Привет"), "{decoded}");
    }

    #[test]
    fn html_entities_do_not_drop_text() {
        let xml = "<html><body><p>Первый&nbsp;абзац &mdash; продолжение.</p></body></html>";
        let text = extract_xml_paragraphs(xml, &["p"], &[], None, None)
            .expect("well-formed XHTML with HTML entities");

        assert!(text.contains("Первый"), "text before entity lost: {text}");
        assert!(
            text.contains("продолжение"),
            "text after entity lost: {text}"
        );
    }

    #[test]
    fn extracts_title_text_from_xml_metadata() {
        let opf = r#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>Война &amp; мир</dc:title></metadata></package>"#;
        assert_eq!(
            xml_element_text(opf, b"title").as_deref(),
            Some("Война & мир")
        );

        let fb2 = r#"<FictionBook><description><title-info><book-title>Мир и война</book-title></title-info></description></FictionBook>"#;
        assert_eq!(
            xml_element_text(fb2, b"book-title").as_deref(),
            Some("Мир и война")
        );
    }

    #[test]
    fn missing_xml_title_is_none() {
        assert_eq!(
            xml_element_text("<package><metadata></metadata></package>", b"title"),
            None
        );
    }
}

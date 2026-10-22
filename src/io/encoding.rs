use encoding_rs::{UTF_16BE, UTF_16LE, WINDOWS_1251};

/// Automatically detects UTF-8 (with/without BOM), UTF-16LE/BE, or Windows-1251,
/// and normalizes all line endings (\r\n and standalone Mac \r) to \n.
pub fn decode_text_bytes(bytes: &[u8]) -> (String, &'static str) {
    let (raw, enc_name) = if let Some(stripped) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        match std::str::from_utf8(stripped) {
            Ok(s) => (s.to_owned(), "UTF-8 BOM"),
            Err(_) => {
                let (cow, _, _) = WINDOWS_1251.decode(stripped);
                (cow.into_owned(), "CP1251")
            }
        }
    } else if let Some(stripped) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let (cow, _, _) = UTF_16LE.decode(stripped);
        (cow.into_owned(), "UTF-16LE")
    } else if let Some(stripped) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let (cow, _, _) = UTF_16BE.decode(stripped);
        (cow.into_owned(), "UTF-16BE")
    } else if looks_like_utf16le_no_bom(bytes) {
        let (cow, _, _) = UTF_16LE.decode(bytes);
        (cow.into_owned(), "UTF-16LE")
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        (s.to_owned(), "UTF-8")
    } else {
        let (cow, _, _) = WINDOWS_1251.decode(bytes);
        (cow.into_owned(), "CP1251")
    };

    (normalize_line_endings(raw), enc_name)
}

fn looks_like_utf16le_no_bom(bytes: &[u8]) -> bool {
    if bytes.len() < 8 || bytes.len() % 2 != 0 {
        return false;
    }
    let sample_pairs = (bytes.len() / 2).min(64);
    let mut null_or_cyr_high = 0usize;
    for i in 0..sample_pairs {
        let hi = bytes[i * 2 + 1];
        if hi == 0x00 || hi == 0x04 {
            null_or_cyr_high += 1;
        }
    }
    null_or_cyr_high * 10 >= sample_pairs * 8
}

pub fn looks_like_text_bytes(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF])
        || bytes.starts_with(&[0xFF, 0xFE])
        || bytes.starts_with(&[0xFE, 0xFF])
        || looks_like_utf16le_no_bom(bytes)
    {
        return true;
    }

    const BINARY_SIGNATURES: &[&[u8]] = &[
        b"\x89PNG\r\n\x1a\n",
        b"\xFF\xD8\xFF",
        b"%PDF-",
        b"PK\x03\x04",
        b"MZ",
        b"GIF87a",
        b"GIF89a",
        b"\x7FELF",
        b"Rar!\x1a\x07",
    ];
    if BINARY_SIGNATURES
        .iter()
        .any(|signature| bytes.starts_with(signature))
    {
        return false;
    }

    let sample = &bytes[..bytes.len().min(8192)];
    let mut control = 0usize;
    for &byte in sample {
        if byte == 0 {
            return false;
        }
        if byte < 0x09 || (byte > 0x0d && byte < 0x20) {
            control += 1;
        }
    }
    control * 100 <= sample.len() * 2
}

pub fn normalize_line_endings(mut s: String) -> String {
    if !s.contains('\r') {
        return s;
    }
    if s.contains("\r\n") {
        s = s.replace("\r\n", "\n");
    }
    if s.contains('\r') {
        // Converts standalone Mac \r (e.g. in 95.NEW_Орфоэпия(phoneme).dic) into \n
        s = s.replace('\r', "\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_cp1251() {
        let bytes = [0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2];
        let (text, encoding) = decode_text_bytes(&bytes);
        assert_eq!(text, "Привет");
        assert_eq!(encoding, "CP1251");
    }

    #[test]
    fn strips_utf8_bom_and_normalizes_line_endings() {
        let bytes = b"\xEF\xBB\xBFone\r\ntwo\rthree";
        let (text, encoding) = decode_text_bytes(bytes);
        assert_eq!(text, "one\ntwo\nthree");
        assert_eq!(encoding, "UTF-8 BOM");
    }

    #[test]
    fn detects_text_but_rejects_binary_data() {
        assert!(looks_like_text_bytes("Привет, мир!\n".as_bytes()));
        assert!(looks_like_text_bytes(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]));
        assert!(looks_like_text_bytes(&[0xFF, 0xFE, 0x1F, 0x04]));
        assert!(!looks_like_text_bytes(b"\x89PNG\r\n\x1a\n"));
        assert!(!looks_like_text_bytes(b"MZ\x90\x00"));
        assert!(!looks_like_text_bytes(&[0x00, 0x01, 0x02, 0x03]));
    }
}

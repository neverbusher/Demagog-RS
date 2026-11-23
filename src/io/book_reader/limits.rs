use std::fs::File;
use std::io::Read;
use std::sync::atomic::AtomicBool;

use zip::ZipArchive;

use crate::domain::{AppError, AppResult, BookDocument};
use crate::ports::is_cancelled as book_cancelled;

pub(super) const MAX_ARCHIVE_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const MAX_ARCHIVE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

const CHUNK_BYTES: usize = 64 * 1024;

pub(super) fn cancelled_book() -> AppResult<BookDocument> {
    Err(AppError::cancelled("Загрузка книги отменена"))
}

pub(super) fn read_stream_with_cancel<R: Read>(
    mut reader: R,
    name: &str,
    limit: u64,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; CHUNK_BYTES];
    loop {
        if book_cancelled(cancel) {
            return Err(AppError::cancelled("Загрузка книги отменена"));
        }
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.len() as u64 > limit {
                    return Err(format!("Файл «{name}» больше допустимых {limit} байт").into());
                }
            }
            Err(err) => return Err(format!("Ошибка чтения «{name}»: {err}").into()),
        }
    }
    Ok(bytes)
}

pub(super) fn read_limited_with_cancel<R: Read>(
    reader: R,
    name: &str,
    limit: u64,
    cancel: Option<&AtomicBool>,
) -> AppResult<Vec<u8>> {
    read_stream_with_cancel(reader, name, limit, cancel)
}

pub(super) fn read_zip_entry_optional(
    archive: &mut ZipArchive<File>,
    name: &str,
    cancel: Option<&AtomicBool>,
) -> AppResult<Option<Vec<u8>>> {
    let entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(format!("Ошибка открытия «{name}»: {error}").into()),
    };
    if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
        return Err(format!(
            "В EPUB файл «{name}» больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт"
        )
        .into());
    }
    read_limited_with_cancel(entry, name, MAX_ARCHIVE_ENTRY_BYTES, cancel).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_reader_rejects_entries_above_the_limit() {
        let exact =
            read_limited_with_cancel(std::io::Cursor::new(b"1234567890"), "chapter", 10, None)
                .expect("entry exactly at the limit");
        assert_eq!(exact, b"1234567890");

        let error =
            read_limited_with_cancel(std::io::Cursor::new(b"12345678901"), "chapter", 10, None)
                .expect_err("entry above the limit");
        assert!(error.to_string().contains("больше"), "{error}");
    }
}

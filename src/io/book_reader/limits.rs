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
                    return Err(AppError::limit_exceeded(format!(
                        "Файл «{name}» больше допустимых {limit} байт"
                    )));
                }
            }
            Err(err) => {
                return Err(AppError::io(format!("Ошибка чтения «{name}»: {err}")));
            }
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
        Err(error) => {
            return Err(AppError::archive(format!(
                "Ошибка открытия «{name}»: {error}"
            )));
        }
    };
    if entry.size() > MAX_ARCHIVE_ENTRY_BYTES {
        return Err(AppError::limit_exceeded(format!(
            "В EPUB файл «{name}» больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт"
        )));
    }
    read_limited_with_cancel(entry, name, MAX_ARCHIVE_ENTRY_BYTES, cancel).map(Some)
}

#[cfg(test)]
pub(super) fn write_zip_entry(path: &std::path::Path, name: &str, content: &[u8]) {
    use std::io::Write;

    let file = File::create(path).expect("create ZIP fixture");
    let mut archive = zip::ZipWriter::new(file);
    archive
        .start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .expect("start ZIP fixture entry");
    archive.write_all(content).expect("write ZIP fixture entry");
    archive.finish().expect("finish ZIP fixture");
}

#[cfg(test)]
pub(super) fn write_zip_entry_with_declared_size(
    path: &std::path::Path,
    name: &str,
    content: &[u8],
    declared_size: u64,
) {
    write_zip_entry(path, name, content);
    let mut bytes = std::fs::read(path).expect("read ZIP fixture");
    let size = u32::try_from(declared_size)
        .expect("test entry size must fit a standard ZIP header")
        .to_le_bytes();
    let mut found_local_header = false;
    let mut found_central_header = false;
    for index in 0..bytes.len().saturating_sub(3) {
        if &bytes[index..index + 4] == b"PK\x03\x04" && index + 30 <= bytes.len() {
            bytes[index + 22..index + 26].copy_from_slice(&size);
            found_local_header = true;
        }
        if &bytes[index..index + 4] == b"PK\x01\x02" && index + 32 <= bytes.len() {
            bytes[index + 24..index + 28].copy_from_slice(&size);
            found_central_header = true;
        }
    }
    assert!(found_local_header && found_central_header);
    std::fs::write(path, bytes).expect("update ZIP fixture entry size");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::FailureKind;

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
        assert_eq!(error.failure_kind(), Some(FailureKind::LimitExceeded));
        assert_eq!(
            error.to_string(),
            "Файл «chapter» больше допустимых 10 байт"
        );
    }

    #[test]
    fn stream_read_failure_is_io() {
        let path = super::super::test_temp_path("stream-read-error", "");
        std::fs::create_dir_all(&path).expect("create directory fixture");
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(&path)
                .expect("open directory for a failing read")
        };
        #[cfg(unix)]
        let file = File::open(&path).expect("open directory for a failing read");

        let error = read_stream_with_cancel(file, "directory", 1024, None)
            .expect_err("reading a directory must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::Io));
        assert!(
            error.to_string().starts_with("Ошибка чтения «directory»: "),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn missing_optional_zip_entry_remains_none() {
        let path = super::super::test_temp_path("zip-missing", "zip");
        write_zip_entry(&path, "other.txt", b"text");
        let file = File::open(&path).expect("open ZIP fixture");
        let mut archive = ZipArchive::new(file).expect("read ZIP fixture");

        let entry = read_zip_entry_optional(&mut archive, "missing.txt", None)
            .expect("missing optional entry is not an error");

        assert_eq!(entry, None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn oversized_zip_entry_is_a_limit_failure() {
        let path = super::super::test_temp_path("zip-oversized", "zip");
        write_zip_entry_with_declared_size(
            &path,
            "chapter.xhtml",
            b"<html/>",
            MAX_ARCHIVE_ENTRY_BYTES + 1,
        );
        let file = File::open(&path).expect("open ZIP fixture");
        let mut archive = ZipArchive::new(file).expect("read ZIP fixture");

        let error = read_zip_entry_optional(&mut archive, "chapter.xhtml", None)
            .expect_err("oversized ZIP entry must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::LimitExceeded));
        assert_eq!(
            error.to_string(),
            format!("В EPUB файл «chapter.xhtml» больше допустимых {MAX_ARCHIVE_ENTRY_BYTES} байт")
        );
        let _ = std::fs::remove_file(path);
    }
}

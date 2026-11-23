//! Persistence for [`AppSettings`]: locating, reading, backing up and atomically
//! writing `settings.json`. The domain type stays free of filesystem access.

use std::path::{Path, PathBuf};

use crate::domain::{AppError, AppResult, AppSettings};

pub fn settings_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        return dir.join("settings.json");
    }
    PathBuf::from("settings.json")
}

pub fn load(default_dic_folder: PathBuf) -> (AppSettings, Option<AppError>) {
    load_from_path(&settings_path(), default_dic_folder)
}

pub fn load_from_path(path: &Path, default_dic_folder: PathBuf) -> (AppSettings, Option<AppError>) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (fallback(default_dic_folder), None);
        }
        Err(error) => {
            return (
                fallback(default_dic_folder),
                Some(AppError::io(format!(
                    "Не удалось прочитать настройки {}: {error}",
                    path.display()
                ))),
            );
        }
    };

    match AppSettings::from_json(&bytes) {
        Ok(mut settings) => {
            apply_dic_folder(&mut settings, default_dic_folder);
            (settings, None)
        }
        Err(error) => {
            let backup = path.with_extension("json.corrupt");
            let backup_note = match std::fs::copy(path, &backup) {
                Ok(_) => format!("Сохранена копия: {}", backup.display()),
                Err(copy_error) => format!(
                    "Не удалось сохранить копию {}: {copy_error}",
                    backup.display()
                ),
            };
            (
                fallback(default_dic_folder),
                Some(AppError::invalid_data(format!(
                    "Не удалось прочитать настройки {}: {error}. {backup_note}",
                    path.display()
                ))),
            )
        }
    }
}

fn fallback(default_dic_folder: PathBuf) -> AppSettings {
    AppSettings {
        dic_folder: default_dic_folder,
        ..AppSettings::default()
    }
}

fn apply_dic_folder(settings: &mut AppSettings, default_dic_folder: PathBuf) {
    let is_old_demagog = settings
        .dic_folder
        .to_string_lossy()
        .contains("Demagog-x64-Piper");
    if (!settings.dic_folder.is_dir() || is_old_demagog) && default_dic_folder.is_dir() {
        settings.dic_folder = default_dic_folder;
    }
}

pub fn save(settings: &AppSettings) -> AppResult<()> {
    save_to_path(settings, &settings_path())
}

/// Writes via a temporary file + rename so a crash mid-write cannot leave
/// a truncated settings.json (which would reset the app to defaults).
pub fn save_to_path(settings: &AppSettings, path: &Path) -> AppResult<()> {
    let json = serde_json::to_string_pretty(settings)
        .map_err(|err| AppError::serialization(err.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|err| AppError::io(format!("{}: {err}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|err| AppError::io(format!("{}: {err}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AppError, FailureKind, ThemeMode};

    fn temp_dir(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("demagog-{tag}-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn assert_failure(error: &AppError, kind: FailureKind, message: &str) {
        assert_eq!(error.failure_kind(), Some(kind));
        assert_eq!(error.to_string(), message);
    }

    #[test]
    fn save_to_path_roundtrips_and_leaves_no_temp_file() {
        let dir = temp_dir("save");
        let path = dir.join("settings.json");

        let settings = AppSettings::default();
        save_to_path(&settings, &path).expect("atomic save");

        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
        let (loaded, warning) = load_from_path(&path, PathBuf::from("dic"));
        assert!(warning.is_none());
        assert_eq!(loaded.theme(), settings.theme());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_returns_defaults() {
        let dir = temp_dir("missing");
        let path = dir.join("absent.json");

        let (settings, warning) = load_from_path(&path, dir.clone());

        assert!(warning.is_none());
        assert_eq!(settings.theme(), ThemeMode::Dark);
        assert_eq!(settings.ui_font_size, AppSettings::default().ui_font_size);
        assert_eq!(settings.dic_folder, dir);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_dic_folder_is_replaced_with_the_default_folder() {
        let dir = temp_dir("folder");
        let path = dir.join("settings.json");
        let settings = AppSettings {
            dic_folder: dir.join("gone"),
            ..Default::default()
        };
        save_to_path(&settings, &path).expect("save settings");

        let (loaded, warning) = load_from_path(&path, dir.clone());

        assert!(warning.is_none());
        assert_eq!(loaded.dic_folder, dir);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_settings_are_reported_and_backed_up() {
        let dir = temp_dir("corrupt");
        let path = dir.join("settings.json");
        std::fs::write(&path, b"{not valid json").expect("write corrupt settings");

        let default_dic_folder = dir.join("missing-dic-folder");
        let (settings, warning) = load_from_path(&path, default_dic_folder.clone());
        assert_eq!(settings.theme(), ThemeMode::Dark);
        assert_eq!(settings.dic_folder, default_dic_folder);
        let warning = warning.expect("corrupt settings must be reported");
        let backup = path.with_extension("json.corrupt");
        assert_failure(
            &warning,
            FailureKind::InvalidData,
            &format!(
                "Не удалось прочитать настройки {}: key must be a string at line 1 column 2. Сохранена копия: {}",
                path.display(),
                backup.display()
            ),
        );

        assert!(backup.exists(), "corrupt settings must be backed up");
        assert_eq!(
            std::fs::read(&backup).expect("read backup"),
            b"{not valid json"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_settings_backup_failure_is_included_in_the_exact_message() {
        let dir = temp_dir("corrupt-backup-failure");
        let path = dir.join("settings.json");
        let backup = path.with_extension("json.corrupt");
        std::fs::write(&path, b"{not valid json").expect("write corrupt settings");
        std::fs::create_dir(&backup).expect("create blocking backup directory");
        let copy_error =
            std::fs::copy(&path, &backup).expect_err("copying onto a directory must fail");

        let (settings, warning) = load_from_path(&path, dir.join("missing-dic-folder"));
        assert_eq!(settings.theme(), ThemeMode::Dark);
        let warning = warning.expect("corrupt settings must be reported");
        assert_failure(
            &warning,
            FailureKind::InvalidData,
            &format!(
                "Не удалось прочитать настройки {}: key must be a string at line 1 column 2. Не удалось сохранить копию {}: {copy_error}",
                path.display(),
                backup.display()
            ),
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_read_failure_is_io_with_the_exact_message() {
        let dir = temp_dir("read-failure");
        let default_dic_folder = dir.join("default-dic-folder");
        let read_error = std::fs::read(&dir).expect_err("reading a directory must fail");

        let (settings, warning) = load_from_path(&dir, default_dic_folder.clone());

        assert_eq!(settings.theme(), ThemeMode::Dark);
        assert_eq!(settings.dic_folder, default_dic_folder);
        let warning = warning.expect("read failure must be reported");
        assert_failure(
            &warning,
            FailureKind::Io,
            &format!(
                "Не удалось прочитать настройки {}: {read_error}",
                dir.display()
            ),
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serialization_failure_preserves_the_bare_error_message() {
        let dir = temp_dir("serialization-failure");
        let path = dir.join("settings.json");
        #[cfg(windows)]
        let dic_folder = {
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_wide(&[0xD800]))
        };
        #[cfg(unix)]
        let dic_folder = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(vec![0xFF]))
        };
        let settings = AppSettings {
            dic_folder,
            ..Default::default()
        };

        let error =
            save_to_path(&settings, &path).expect_err("invalid UTF-8 path must not serialize");

        assert_failure(
            &error,
            FailureKind::Serialization,
            "path contains invalid UTF-8 characters",
        );
        assert!(!path.exists());
        assert!(!path.with_extension("json.tmp").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_parent_save_failure_is_io_with_the_exact_temp_path_prefix() {
        let dir = temp_dir("save-failure");
        let path = dir.join("missing").join("settings.json");
        let expected_prefix = format!("{}: ", path.with_extension("json.tmp").display());

        let error = save_to_path(&AppSettings::default(), &path)
            .expect_err("saving into a missing parent directory must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::Io));
        assert!(
            error.to_string().starts_with(&expected_prefix),
            "unexpected save error: {error}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_failure_is_io_with_the_exact_destination_path_prefix() {
        let dir = temp_dir("rename-failure");
        let path = dir.join("settings.json");
        std::fs::create_dir(&path).expect("create blocking destination directory");
        let expected_prefix = format!("{}: ", path.display());

        let error = save_to_path(&AppSettings::default(), &path)
            .expect_err("renaming over a directory must fail");

        assert_eq!(error.failure_kind(), Some(FailureKind::Io));
        assert!(
            error.to_string().starts_with(&expected_prefix),
            "unexpected rename error: {error}"
        );
        assert!(path.with_extension("json.tmp").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

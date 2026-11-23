//! Persistence for [`AppSettings`]: locating, reading, backing up and atomically
//! writing `settings.json`. The domain type stays free of filesystem access.

use std::path::{Path, PathBuf};

use crate::domain::AppSettings;

pub fn settings_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        return dir.join("settings.json");
    }
    PathBuf::from("settings.json")
}

pub fn load(default_dic_folder: PathBuf) -> (AppSettings, Option<String>) {
    load_from_path(&settings_path(), default_dic_folder)
}

pub fn load_from_path(path: &Path, default_dic_folder: PathBuf) -> (AppSettings, Option<String>) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (fallback(default_dic_folder), None);
        }
        Err(error) => {
            return (
                fallback(default_dic_folder),
                Some(format!(
                    "Не удалось прочитать настройки {}: {error}",
                    path.display()
                )),
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
                Some(format!(
                    "Не удалось прочитать настройки {}: {error}. {backup_note}",
                    path.display()
                )),
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

pub fn save(settings: &AppSettings) -> Result<(), String> {
    save_to_path(settings, &settings_path())
}

/// Writes via a temporary file + rename so a crash mid-write cannot leave
/// a truncated settings.json (which would reset the app to defaults).
pub fn save_to_path(settings: &AppSettings, path: &Path) -> Result<(), String> {
    let json = serde_json::to_string_pretty(settings).map_err(|err| err.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|err| format!("{}: {err}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|err| format!("{}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ThemeMode;

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

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_dic_folder_is_replaced_with_the_default_folder() {
        let dir = temp_dir("folder");
        let path = dir.join("settings.json");
        let mut settings = AppSettings::default();
        settings.dic_folder = dir.join("gone");
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

        let (settings, warning) = load_from_path(&path, dir.join("missing-dic-folder"));
        assert_eq!(settings.theme(), ThemeMode::Dark);
        let warning = warning.expect("corrupt settings must be reported");
        assert!(warning.contains("настройки"), "{warning}");

        let backup = path.with_extension("json.corrupt");
        assert!(backup.exists(), "corrupt settings must be backed up");
        assert_eq!(
            std::fs::read(&backup).expect("read backup"),
            b"{not valid json"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

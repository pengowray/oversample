use tauri::Manager;

#[tauri::command]
pub fn read_sidecar(path: String) -> Result<Option<String>, String> {
    let sidecar = format!("{}.batm", path);
    if std::path::Path::new(&sidecar).exists() {
        std::fs::read_to_string(&sidecar)
            .map(Some)
            .map_err(|e| format!("Failed to read sidecar: {e}"))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub fn write_sidecar(path: String, yaml: String) -> Result<(), String> {
    let sidecar = format!("{}.batm", path);
    // Atomic write: write to temp, then rename
    let tmp = format!("{}.batm.tmp", path);
    std::fs::write(&tmp, &yaml).map_err(|e| format!("Failed to write sidecar: {e}"))?;
    std::fs::rename(&tmp, &sidecar).map_err(|e| format!("Failed to rename sidecar: {e}"))?;
    Ok(())
}

/// Whether `file_key` is safe as a file name in the central folder: only
/// letters, digits, `.`, `-` and `_` (the app's own keys), so no path
/// separator, drive letter or `..` can reach the path. Keys can come from
/// Google Drive copies.
fn check_key(file_key: &str) -> Result<(), String> {
    let ok = !file_key.is_empty()
        && file_key.len() <= 255
        && !file_key.starts_with('.')
        && !file_key.contains("..")
        && file_key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if ok {
        Ok(())
    } else {
        Err("invalid annotation key".into())
    }
}

#[tauri::command]
pub fn read_central_annotations(
    app: tauri::AppHandle,
    file_key: String,
) -> Result<Option<String>, String> {
    check_key(&file_key)?;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("annotations");
    let path = dir.join(format!("{}.batm", file_key));
    if path.exists() {
        std::fs::read_to_string(&path)
            .map(Some)
            .map_err(|e| format!("Failed to read annotations: {e}"))
    } else {
        Ok(None)
    }
}

/// Keys of every annotation set in the central annotations folder (the
/// `file_key` that `read_central_annotations` takes). Files are stored as
/// `<file_key>.batm`; unfinished `.tmp` files are left out.
#[tauri::command]
pub fn list_central_annotations(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("annotations");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    Ok(central_keys(
        entries
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok()),
    ))
}

/// The keys among central annotation file names.
fn central_keys(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut keys: Vec<String> = names
        .filter_map(|n| n.strip_suffix(".batm").map(str::to_string))
        .filter(|k| !k.is_empty())
        .collect();
    keys.sort();
    keys
}

#[tauri::command]
pub fn write_central_annotations(
    app: tauri::AppHandle,
    file_key: String,
    yaml: String,
) -> Result<(), String> {
    check_key(&file_key)?;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("annotations");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.batm", file_key));
    let tmp = dir.join(format!("{}.batm.tmp", file_key));
    std::fs::write(&tmp, &yaml).map_err(|e| format!("Failed to write annotations: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("Failed to rename annotations: {e}"))?;
    Ok(())
}

/// Delete a set from the central folder. Not there is not an error.
#[tauri::command]
pub fn delete_central_annotations(app: tauri::AppHandle, file_key: String) -> Result<(), String> {
    check_key(&file_key)?;
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("annotations")
        .join(format!("{}.batm", file_key));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Failed to delete annotations: {e}")),
    }
}

/// The per-folder settings file, beside the recordings (TOML). Today it
/// holds one setting, `annotations = "beside"`: annotations of the
/// recordings in this folder are kept in a .batm beside each recording,
/// and only there.
const FOLDER_SETTINGS: &str = ".oversample";

/// What the folder's `.oversample` says (see `FOLDER_SETTINGS`).
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSettings {
    pub annotations_beside: bool,
}

fn folder_settings_path(audio_path: &str) -> Result<std::path::PathBuf, String> {
    std::path::Path::new(audio_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.join(FOLDER_SETTINGS))
        .ok_or_else(|| "no folder".to_string())
}

fn read_folder_table(path: &std::path::Path) -> Result<toml::Table, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<toml::Table>()
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// The settings of the folder that holds `audio_path`. No file, or a file
/// that cannot be read, is the default (nothing set).
#[tauri::command]
pub fn read_folder_settings(audio_path: String) -> FolderSettings {
    let Ok(path) = folder_settings_path(&audio_path) else {
        return FolderSettings::default();
    };
    let table = read_folder_table(&path).unwrap_or_default();
    FolderSettings {
        annotations_beside: table.get("annotations").and_then(|v| v.as_str()) == Some("beside"),
    }
}

/// Turn "annotations beside the recordings" on or off for the folder that
/// holds `audio_path`, keeping any other settings in its `.oversample`. A
/// file left with no settings is deleted.
#[tauri::command]
pub fn set_folder_annotations_beside(audio_path: String, beside: bool) -> Result<(), String> {
    let path = folder_settings_path(&audio_path)?;
    let mut table = read_folder_table(&path)?;
    if beside {
        table.insert("annotations".into(), toml::Value::String("beside".into()));
    } else {
        table.remove("annotations");
    }
    if table.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Show a native save dialog and export annotations to the chosen path.
/// Returns the saved path, or empty string if cancelled.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn export_annotations_file(filename: String, yaml: String) -> Result<String, String> {
    let handle = rfd::AsyncFileDialog::new()
        .set_file_name(&filename)
        .add_filter("Oversample annotations", &["batm"])
        .add_filter("YAML files", &["yaml", "yml"])
        .set_title("Export annotations")
        .save_file()
        .await;
    match handle {
        Some(file) => {
            let path = file.path().to_string_lossy().to_string();
            std::fs::write(file.path(), &yaml)
                .map_err(|e| format!("Failed to write export: {e}"))?;
            Ok(path)
        }
        None => Ok(String::new()), // cancelled
    }
}

#[cfg(target_os = "android")]
#[tauri::command]
pub async fn export_annotations_file(_filename: String, _yaml: String) -> Result<String, String> {
    Err("File export dialog not supported on Android".into())
}

/// Show a native file-open dialog and return the selected paths.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn open_file_dialog() -> Result<Vec<String>, String> {
    let handle = rfd::AsyncFileDialog::new()
        .add_filter(
            "Audio files",
            &["wav", "w4v", "flac", "ogg", "mp3", "m4a", "m4b"],
        )
        .add_filter("All files", &["*"])
        .set_title("Open audio files")
        .pick_files()
        .await;
    match handle {
        Some(files) => Ok(files
            .iter()
            .map(|f| f.path().to_string_lossy().to_string())
            .collect()),
        None => Ok(Vec::new()), // cancelled
    }
}

#[cfg(target_os = "android")]
#[tauri::command]
pub async fn open_file_dialog() -> Result<Vec<String>, String> {
    Err("File open dialog not supported on Android".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn folder_settings_round_trip() {
        let dir = std::env::temp_dir().join(format!("oversample-folder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("a.wav").to_string_lossy().to_string();
        let file = dir.join(super::FOLDER_SETTINGS);
        assert!(!super::read_folder_settings(audio.clone()).annotations_beside);
        super::set_folder_annotations_beside(audio.clone(), true).unwrap();
        assert!(super::read_folder_settings(audio.clone()).annotations_beside);
        // Other settings are kept; with none left the file goes.
        std::fs::write(&file, "annotations = \"beside\"\nsite = \"Pond\"\n").unwrap();
        super::set_folder_annotations_beside(audio.clone(), false).unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("site"));
        std::fs::write(&file, "annotations = \"beside\"\n").unwrap();
        super::set_folder_annotations_beside(audio.clone(), false).unwrap();
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_that_could_leave_the_folder_are_refused() {
        for bad in [
            "C:x.batm",
            "../x.batm",
            "a/b.batm",
            "a\\b.batm",
            ".hidden.batm",
            "",
            "a..b.batm",
        ] {
            assert!(super::check_key(bad).is_err(), "{bad}");
        }
        for good in [
            "b3_0123abcd.batm",
            "Fledermäuse_teich.wav_100.batm",
            "x-y.wav_1.batm",
        ] {
            assert!(super::check_key(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn central_keys_drop_one_batm_and_temp_files() {
        let names = [
            "b3_abc.batm.batm",
            "b3_abc.batm.tmp",
            "x.wav_10.batm.batm",
            "notes.txt",
        ];
        assert_eq!(
            super::central_keys(names.iter().map(|s| s.to_string())),
            vec!["b3_abc.batm".to_string(), "x.wav_10.batm".to_string()]
        );
    }
}

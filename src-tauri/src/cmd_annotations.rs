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

//! The "Folder for new recordings" setting (`crate::save_folder`) and opening
//! a file's folder in the system file manager.
//!
//! No command takes a folder to save in from the webview: a folder is only
//! ever set from the system's folder picker, here.

use oversample_ipc::mic::RecordingFolder;

/// Where the next recording will be saved.
#[tauri::command]
pub fn recording_folder_get(app: tauri::AppHandle) -> Result<RecordingFolder, String> {
    crate::save_folder::describe(&app)
}

/// Ask for a folder with the system's folder picker and keep it as the
/// folder for new recordings. `None` when the user cancels.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn recording_folder_choose(
    app: tauri::AppHandle,
) -> Result<Option<RecordingFolder>, String> {
    use tauri::Manager;
    let current = crate::save_folder::describe(&app)?;
    let start = std::path::PathBuf::from(&current.path);
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Choose a folder for new recordings");
    // The dialog opens in the current folder, or the nearest folder above it
    // that exists (the default one is made only when a recording is saved).
    if let Some(dir) = start.ancestors().find(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    let Some(picked) = dialog.pick_folder().await else {
        return Ok(None);
    };
    let config = app.path().app_config_dir().map_err(|e| e.to_string())?;
    crate::save_folder::save_custom(&config, Some(picked.path())).map_err(|e| e.to_string())?;
    crate::save_folder::describe(&app).map(Some)
}

#[cfg(target_os = "android")]
#[tauri::command]
pub async fn recording_folder_choose(
    _app: tauri::AppHandle,
) -> Result<Option<RecordingFolder>, String> {
    Err("There is no folder picker on Android".into())
}

/// Go back to the default folder for new recordings.
#[tauri::command]
pub fn recording_folder_reset(app: tauri::AppHandle) -> Result<RecordingFolder, String> {
    use tauri::Manager;
    let config = app.path().app_config_dir().map_err(|e| e.to_string())?;
    crate::save_folder::save_custom(&config, None).map_err(|e| e.to_string())?;
    crate::save_folder::describe(&app)
}

/// The folder to open for `path`: the folder itself, or the folder a file is
/// in. Only an absolute path to something that exists, and only ever a
/// folder, so no file is opened or run.
fn folder_to_open(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("not an absolute path: {}", path.display()));
    }
    let dir = if path.is_dir() {
        path
    } else {
        path.parent()
            .ok_or_else(|| format!("no folder for {}", path.display()))?
    };
    if dir.is_dir() {
        Ok(dir.to_path_buf())
    } else {
        Err(format!("folder not found: {}", dir.display()))
    }
}

/// Open the folder that `path` is in with the system file manager.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[tauri::command]
pub fn open_containing_folder(path: String) -> Result<(), String> {
    let dir = folder_to_open(std::path::Path::new(&path))?;
    open::that_detached(&dir).map_err(|e| e.to_string())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
#[tauri::command]
pub fn open_containing_folder(path: String) -> Result<(), String> {
    let _ = folder_to_open(std::path::Path::new(&path))?;
    Err("Opening a folder is not supported on this device".into())
}

#[cfg(test)]
mod tests {
    use super::folder_to_open;

    #[test]
    fn only_a_folder_is_opened() {
        let dir = std::env::temp_dir().join(format!("oversample-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rec.wav");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(folder_to_open(&file).unwrap(), dir);
        assert_eq!(folder_to_open(&dir).unwrap(), dir);
        assert!(folder_to_open(std::path::Path::new("rec.wav")).is_err());
        assert!(folder_to_open(&dir.join("gone/rec.wav")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

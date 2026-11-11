//! Model files for BirdNET and BattyBirdNET, kept in the app data folder's
//! `models/`. The webview downloads them (with a SHA-256 check) and sends the
//! bytes here in parts, as raw IPC bodies; WebKitGTK has no OPFS to keep them in.
//!
//! Only file names from the catalog (`oversample_ipc::classifier::catalog`) are
//! accepted, so no command can touch a path outside `models/`.

use oversample_ipc::classifier::catalog;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use tauri::Manager;

fn models_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn checked(file: &str) -> Result<&'static catalog::ModelFile, String> {
    catalog::file_by_id(file).ok_or_else(|| format!("unknown model file '{file}'"))
}

fn part_name(id: &str) -> String {
    format!("{id}.part")
}

/// Stored catalog files and their sizes in bytes.
#[tauri::command]
pub fn models_sizes(app: tauri::AppHandle) -> Result<Vec<(String, u64)>, String> {
    let dir = models_dir(&app)?;
    Ok(catalog::all_files()
        .filter_map(|f| {
            let meta = std::fs::metadata(dir.join(f.id)).ok()?;
            Some((f.id.to_string(), meta.len()))
        })
        .collect())
}

/// Write one part of a file being stored. Raw body; headers `x-model-file`
/// (catalog id) and `x-model-offset` (byte offset). Offset 0 starts the file
/// again. Parts go to `<id>.part` until [`models_commit`].
#[tauri::command]
pub fn models_write_part(
    app: tauri::AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw bytes".into());
    };
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| format!("missing header {name}"))
    };
    let file = checked(&header("x-model-file")?)?;
    let offset: u64 = header("x-model-offset")?
        .parse()
        .map_err(|_| "bad x-model-offset".to_string())?;
    if offset + bytes.len() as u64 > file.size {
        return Err(format!("{} is larger than expected", file.id));
    }
    let path = models_dir(&app)?.join(part_name(file.id));
    let mut f = if offset == 0 {
        std::fs::File::create(&path).map_err(|e| e.to_string())?
    } else {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let len = f.metadata().map_err(|e| e.to_string())?.len();
        if len != offset {
            return Err(format!("expected offset {len}, got {offset}"));
        }
        f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        f
    };
    f.write_all(bytes).map_err(|e| e.to_string())?;
    Ok(())
}

/// Finish storing a file: check the part file's size and move it into place.
#[tauri::command]
pub fn models_commit(app: tauri::AppHandle, file: String, size: u64) -> Result<(), String> {
    let file = checked(&file)?;
    let dir = models_dir(&app)?;
    let part = dir.join(part_name(file.id));
    let len = std::fs::metadata(&part).map_err(|e| e.to_string())?.len();
    if len != size || size != file.size {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "{} has {len} bytes; expected {}",
            file.id, file.size
        ));
    }
    std::fs::rename(&part, dir.join(file.id)).map_err(|e| e.to_string())
}

/// Read a stored file, as raw bytes.
#[tauri::command]
pub fn models_read(app: tauri::AppHandle, file: String) -> Result<tauri::ipc::Response, String> {
    let file = checked(&file)?;
    let bytes = std::fs::read(models_dir(&app)?.join(file.id)).map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Delete a stored file (and any unfinished part). Not stored is not an error.
#[tauri::command]
pub fn models_delete(app: tauri::AppHandle, file: String) -> Result<(), String> {
    let file = checked(&file)?;
    let dir = models_dir(&app)?;
    let _ = std::fs::remove_file(dir.join(part_name(file.id)));
    match std::fs::remove_file(dir.join(file.id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

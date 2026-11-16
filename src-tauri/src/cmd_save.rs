//! Files the webview saves through native code, written in parts: exports and
//! recordings the app encoded in memory.
//!
//! A file is opened one of three ways, each giving an id:
//! - [`save_open_dialog`] (desktop): a "Save As" dialog.
//! - [`save_open_recording`]: a new file in the app data folder's `recordings/`.
//! - [`save_adopt_fd`] (Android): a file descriptor from the media-store
//!   plugin's `createExportEntry`.
//!
//! The webview then sends the bytes with `tauri_invoke_raw` ([`save_write`])
//! and ends with [`save_close`], or [`save_discard`] if something failed.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Seek, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tauri::Manager;

struct OpenSave {
    file: File,
    /// The file's path when this module created it, so [`save_discard`] can
    /// delete it. `None` for an adopted fd (the plugin deletes the entry).
    path: Option<PathBuf>,
}

/// Files open for writing, by id.
#[derive(Default)]
pub struct SaveFiles {
    files: Mutex<HashMap<u32, OpenSave>>,
    next_id: AtomicU32,
}

impl SaveFiles {
    fn add(&self, file: File, path: Option<PathBuf>) -> Result<u32, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.files
            .lock()
            .map_err(|e| e.to_string())?
            .insert(id, OpenSave { file, path });
        Ok(id)
    }
}

/// A file opened for saving: its id for [`save_write`] and where it is.
#[derive(serde::Serialize)]
pub struct OpenedSave {
    id: u32,
    path: String,
}

fn create(state: &SaveFiles, path: PathBuf) -> Result<OpenedSave, String> {
    let file =
        File::create(&path).map_err(|e| format!("Failed to create {}: {e}", path.display()))?;
    let shown = path.to_string_lossy().to_string();
    let id = state.add(file, Some(path))?;
    Ok(OpenedSave { id, path: shown })
}

/// Ask where to save `filename` with a "Save As" dialog and open that file.
/// `None` when the user cancels.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn save_open_dialog(
    state: tauri::State<'_, SaveFiles>,
    filename: String,
) -> Result<Option<OpenedSave>, String> {
    let Some(handle) = rfd::AsyncFileDialog::new()
        .set_file_name(&filename)
        .set_title("Export")
        .save_file()
        .await
    else {
        return Ok(None);
    };
    create(&state, handle.path().to_path_buf()).map(Some)
}

#[cfg(target_os = "android")]
#[tauri::command]
pub async fn save_open_dialog(
    _state: tauri::State<'_, SaveFiles>,
    _filename: String,
) -> Result<Option<OpenedSave>, String> {
    Err("There is no Save As dialog on Android".into())
}

/// Open a new file `filename` in the app data folder's `recordings/`.
#[tauri::command]
pub fn save_open_recording(
    app: tauri::AppHandle,
    state: tauri::State<SaveFiles>,
    filename: String,
) -> Result<OpenedSave, String> {
    if filename.is_empty() || filename == "." || filename == ".." || filename.contains(['/', '\\'])
    {
        return Err(format!("bad recording file name '{filename}'"));
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    create(&state, dir.join(filename))
}

/// Take over a file descriptor that the media-store plugin opened for writing
/// (`createExportEntry`) and return an id for [`save_write`].
#[tauri::command]
pub fn save_adopt_fd(state: tauri::State<SaveFiles>, fd: i32) -> Result<u32, String> {
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        // 0 to 2 are stdin, stdout and stderr, never a file the plugin opened.
        if fd <= 2 {
            return Err(format!("bad fd {fd}"));
        }
        state.add(unsafe { File::from_raw_fd(fd) }, None)
    }
    #[cfg(not(unix))]
    {
        let _ = (state, fd);
        Err("save_adopt_fd needs a Unix file descriptor".into())
    }
}

/// Append one part to an open file. Bytes from [`crate::ipc_bytes::body`];
/// headers `x-save-id` and `x-save-offset` (the byte offset the part starts
/// at, which must be the end of the file so far).
#[tauri::command]
pub fn save_write(
    state: tauri::State<SaveFiles>,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let bytes = crate::ipc_bytes::body(&request)?;
    let id: u32 = crate::ipc_bytes::header(&request, "x-save-id")?
        .parse()
        .map_err(|_| "bad x-save-id".to_string())?;
    let offset: u64 = crate::ipc_bytes::header(&request, "x-save-offset")?
        .parse()
        .map_err(|_| "bad x-save-offset".to_string())?;
    let mut files = state.files.lock().map_err(|e| e.to_string())?;
    let file = &mut files.get_mut(&id).ok_or(format!("no open file {id}"))?.file;
    let at = file.stream_position().map_err(|e| e.to_string())?;
    if at != offset {
        return Err(format!("expected offset {at}, got {offset}"));
    }
    file.write_all(&bytes).map_err(|e| e.to_string())
}

/// Close an open file once all `size` bytes are written.
#[tauri::command]
pub fn save_close(state: tauri::State<SaveFiles>, id: u32, size: u64) -> Result<(), String> {
    let mut open = state
        .files
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&id)
        .ok_or(format!("no open file {id}"))?;
    let at = open.file.stream_position().map_err(|e| e.to_string())?;
    if at != size {
        return Err(format!("wrote {at} bytes; expected {size}"));
    }
    open.file.sync_all().map_err(|e| e.to_string())
}

/// Close an open file without finishing it, and delete it if this module
/// created it. (For an adopted fd, the caller deletes the entry with the
/// plugin's `cancelExportEntry`.)
#[tauri::command]
pub fn save_discard(state: tauri::State<SaveFiles>, id: u32) -> Result<(), String> {
    let open = state.files.lock().map_err(|e| e.to_string())?.remove(&id);
    if let Some(OpenSave {
        file,
        path: Some(path),
    }) = open
    {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

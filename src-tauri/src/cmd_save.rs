//! Files the webview writes through the native side, in parts: exports saved
//! to Android's shared storage. The media-store plugin creates the entry and
//! returns its file descriptor; [`save_adopt_fd`] takes the fd over, the
//! webview sends the bytes with `tauri_invoke_raw` ([`save_write`]), and
//! [`save_close`] closes the file before the plugin makes the entry visible.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Seek, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

/// Files open for writing, by the id [`save_adopt_fd`] returned.
#[derive(Default)]
pub struct SaveFiles {
    files: Mutex<HashMap<u32, File>>,
    next_id: AtomicU32,
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
        let file = unsafe { File::from_raw_fd(fd) };
        let id = state.next_id.fetch_add(1, Ordering::Relaxed);
        state
            .files
            .lock()
            .map_err(|e| e.to_string())?
            .insert(id, file);
        Ok(id)
    }
    #[cfg(not(unix))]
    {
        let _ = (state, fd);
        Err("save_adopt_fd needs a Unix file descriptor".into())
    }
}

/// Append one part to a file from [`save_adopt_fd`]. Bytes from
/// [`crate::ipc_bytes::body`]; headers `x-save-id` and `x-save-offset` (the
/// byte offset the part starts at, which must be the end of the file so far).
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
    let file = files.get_mut(&id).ok_or(format!("no open file {id}"))?;
    let at = file.stream_position().map_err(|e| e.to_string())?;
    if at != offset {
        return Err(format!("expected offset {at}, got {offset}"));
    }
    file.write_all(&bytes).map_err(|e| e.to_string())
}

/// Close a file from [`save_adopt_fd`] once all `size` bytes are written.
#[tauri::command]
pub fn save_close(state: tauri::State<SaveFiles>, id: u32, size: u64) -> Result<(), String> {
    let mut file = state
        .files
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&id)
        .ok_or(format!("no open file {id}"))?;
    let at = file.stream_position().map_err(|e| e.to_string())?;
    if at != size {
        return Err(format!("wrote {at} bytes; expected {size}"));
    }
    file.sync_all().map_err(|e| e.to_string())
}

/// Close a file from [`save_adopt_fd`] without finishing it. The caller then
/// deletes the entry (`cancelExportEntry`).
#[tauri::command]
pub fn save_discard(state: tauri::State<SaveFiles>, id: u32) -> Result<(), String> {
    state.files.lock().map_err(|e| e.to_string())?.remove(&id);
    Ok(())
}

//! Saving bytes the app made (exports, recordings encoded in memory) through
//! native code. Rust opens the file and the bytes go to it in parts
//! ([`crate::tauri_bridge::tauri_invoke_raw`], base64 text on Android), not
//! as one JSON list of numbers. The commands are in `src-tauri/src/cmd_save.rs`.
//!
//! - [`save_with_dialog`] (desktop): a "Save As" dialog.
//! - [`save_recording`]: the folder recordings are saved in (`crate::recording_folder`).
//! - [`save_to_shared_storage`] (Android): shared storage, where the gallery and
//!   Files app see it. The media-store plugin creates the entry and returns its
//!   file descriptor, then makes the entry visible, or deletes it if a write
//!   failed.

use crate::tauri_bridge::{
    raw_part_limit, tauri_invoke_args, tauri_invoke_raw, tauri_invoke_typed_args,
};
use oversample_ipc::plugins::{CreateExportEntryArgs, CreateExportEntryResult, ExportEntryArgs};
use serde::{Deserialize, Serialize};

/// A file `save_open_dialog` or `save_open_recording` opened.
#[derive(Deserialize)]
struct OpenedSave {
    id: u32,
    path: String,
}

#[derive(Serialize)]
struct Filename<'a> {
    filename: &'a str,
}

/// Desktop: ask where to save with a "Save As" dialog, then write `data`
/// there. `Ok(None)` when the user cancels.
pub async fn save_with_dialog(data: &[u8], filename: &str) -> Result<Option<String>, String> {
    let opened: Option<OpenedSave> =
        tauri_invoke_typed_args("save_open_dialog", &Filename { filename }).await?;
    let Some(opened) = opened else {
        return Ok(None);
    };
    write_and_close(opened.id, data).await?;
    Ok(Some(opened.path))
}

/// Save `data` as `filename` in the folder recordings are saved in, and
/// return its path (numbered if the name was taken).
pub async fn save_recording(data: &[u8], filename: &str) -> Result<String, String> {
    let opened: OpenedSave =
        tauri_invoke_typed_args("save_open_recording", &Filename { filename }).await?;
    write_and_close(opened.id, data).await?;
    Ok(opened.path)
}

pub enum SharedSave {
    /// Saved; the entry's content:// URI (or file path before Android 10).
    Saved(String),
    /// Nothing saved: Android just asked for the storage permission, so the
    /// user has to try again.
    NeedsRetry,
}

/// Android: save `data` as `filename` in the shared folder `relative_path`
/// (e.g. `Movies/Oversample/exports`). The MIME type picks the MediaStore
/// collection.
pub async fn save_to_shared_storage(
    data: &[u8],
    filename: &str,
    relative_path: &str,
    mime_type: &str,
) -> Result<SharedSave, String> {
    let entry: CreateExportEntryResult = tauri_invoke_typed_args(
        "plugin:media-store|createExportEntry",
        &CreateExportEntryArgs {
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
            relative_path: relative_path.to_string(),
        },
    )
    .await?;
    let Some(fd) = entry.fd else {
        return Ok(SharedSave::NeedsRetry);
    };
    let entry_args = ExportEntryArgs {
        uri: entry.uri.clone(),
        mime_type: mime_type.to_string(),
    };
    #[derive(Serialize)]
    struct Fd {
        fd: i32,
    }
    let written = match tauri_invoke_typed_args::<_, u32>("save_adopt_fd", &Fd { fd }).await {
        Ok(id) => write_and_close(id, data).await,
        Err(e) => Err(e),
    };
    match written {
        Ok(()) => {
            tauri_invoke_args("plugin:media-store|finishExportEntry", &entry_args).await?;
            Ok(SharedSave::Saved(entry.path))
        }
        Err(e) => {
            if let Err(e2) =
                tauri_invoke_args("plugin:media-store|cancelExportEntry", &entry_args).await
            {
                log::warn!("cancelExportEntry failed: {e2}");
            }
            Err(e)
        }
    }
}

/// Write all of `data` to an open file, then close it. A failed write
/// discards the file.
async fn write_and_close(id: u32, data: &[u8]) -> Result<(), String> {
    #[derive(Serialize)]
    struct Close {
        id: u32,
        size: u64,
    }
    #[derive(Serialize)]
    struct Id {
        id: u32,
    }
    let step = raw_part_limit() as usize;
    for (i, part) in data.chunks(step).enumerate() {
        let res = tauri_invoke_raw(
            "save_write",
            &js_sys::Uint8Array::from(part),
            &[
                ("x-save-id", id.to_string()),
                ("x-save-offset", (i * step).to_string()),
            ],
        )
        .await;
        if let Err(e) = res {
            let _ = tauri_invoke_args("save_discard", &Id { id }).await;
            return Err(e);
        }
    }
    let closed = tauri_invoke_args(
        "save_close",
        &Close {
            id,
            size: data.len() as u64,
        },
    )
    .await;
    if closed.is_err() {
        let _ = tauri_invoke_args("save_discard", &Id { id }).await;
    }
    closed
}

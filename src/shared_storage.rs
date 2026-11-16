//! Saving bytes the app made (exports, recordings encoded in memory) into
//! Android's shared storage, where the gallery and Files app can see them.
//!
//! The media-store plugin creates the entry and returns its file descriptor.
//! Rust takes the fd over and the bytes go to it in parts
//! ([`crate::tauri_bridge::tauri_invoke_raw`], base64 text on Android), instead
//! of the whole file as one JSON list of numbers. The plugin then makes the
//! entry visible, or deletes it if a write failed.

use crate::tauri_bridge::{
    raw_part_limit, tauri_invoke_args, tauri_invoke_raw, tauri_invoke_typed_args,
};
use oversample_ipc::plugins::{CreateExportEntryArgs, CreateExportEntryResult, ExportEntryArgs};

pub enum SharedSave {
    /// Saved; the entry's content:// URI (or file path before Android 10).
    Saved(String),
    /// Nothing saved: Android just asked for the storage permission, so the
    /// user has to try again.
    NeedsRetry,
}

/// Save `data` as `filename` in the shared folder `relative_path` (e.g.
/// `Movies/Oversample/exports`). The MIME type picks the MediaStore collection.
pub async fn save(
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
    match write_fd(fd, data).await {
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

/// Write all of `data` to a file descriptor the plugin opened, then close it.
async fn write_fd(fd: i32, data: &[u8]) -> Result<(), String> {
    #[derive(serde::Serialize)]
    struct Fd {
        fd: i32,
    }
    #[derive(serde::Serialize)]
    struct Close {
        id: u32,
        size: u64,
    }
    #[derive(serde::Serialize)]
    struct Id {
        id: u32,
    }
    let id: u32 = tauri_invoke_typed_args("save_adopt_fd", &Fd { fd }).await?;
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
    tauri_invoke_args(
        "save_close",
        &Close {
            id,
            size: data.len() as u64,
        },
    )
    .await
}

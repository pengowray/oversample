//! Model files on this device: download (with progress and a SHA-256 check),
//! check, read and delete.
//!
//! - Browser: the webview fetches the file, checks it, and keeps it in the OPFS
//!   directory `oversample-models`.
//! - Tauri: native code downloads and checks the file and keeps it in the app
//!   data folder's `models/` (WebKitGTK, the Linux webview, has no OPFS). The
//!   `models_*` commands in `src-tauri/src/cmd_models.rs` do the work; the
//!   `models-download-progress` event reports progress.
//!
//! Bytes stay in JS `Uint8Array`s from the download (or `models_read`) to the
//! worker, so a 67 MB model never passes through (and never grows) the app's
//! WASM memory.

use super::catalog::ModelFile;
use super::ClassifierError;
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemWritableFileStream};

const OPFS_DIR: &str = "oversample-models";
/// The event `models_download` sends `{file, fraction}` with.
const PROGRESS_EVENT: &str = "models-download-progress";

thread_local! {
    /// The progress callback of each native download in progress, by file id.
    static NATIVE_PROGRESS: RefCell<HashMap<String, Box<dyn FnMut(f64)>>> =
        RefCell::new(HashMap::new());
    /// Whether the app-wide [`PROGRESS_EVENT`] listener is registered.
    static PROGRESS_LISTENING: Cell<bool> = const { Cell::new(false) };
}

/// Arguments of the `models_*` commands that take one file.
#[derive(serde::Serialize)]
struct FileArgs {
    file: &'static str,
}

impl FileArgs {
    fn of(file: &ModelFile) -> Self {
        Self { file: file.id }
    }
}

fn is_tauri() -> bool {
    crate::tauri_bridge::get_tauri_internals().is_some()
}

fn storage_err(file: &ModelFile, message: impl Into<String>) -> ClassifierError {
    ClassifierError::Storage {
        file: file.id.to_string(),
        message: message.into(),
    }
}

fn js_msg(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            js_sys::Reflect::get(e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

async fn opfs_dir() -> Result<FileSystemDirectoryHandle, String> {
    crate::opfs::get_opfs_subdir(OPFS_DIR).await
}

async fn opfs_handle(id: &str) -> Option<FileSystemFileHandle> {
    let dir = opfs_dir().await.ok()?;
    JsFuture::from(dir.get_file_handle(id))
        .await
        .ok()
        .map(|h| h.unchecked_into())
}

/// Stored size of `file`, or `None` when it is not stored.
async fn stored_size(file: &ModelFile) -> Option<u64> {
    if is_tauri() {
        let sizes: Vec<(String, u64)> =
            crate::tauri_bridge::tauri_invoke_typed_no_args("models_sizes")
                .await
                .ok()?;
        return sizes
            .into_iter()
            .find(|(n, _)| n == file.id)
            .map(|(_, s)| s);
    }
    let handle = opfs_handle(file.id).await?;
    let f: web_sys::File = JsFuture::from(handle.get_file())
        .await
        .ok()?
        .unchecked_into();
    Some(f.size() as u64)
}

/// True when `file` is stored with the catalog's size. (The SHA-256 is checked
/// when it is downloaded, not on every check.)
pub async fn is_downloaded(file: &ModelFile) -> bool {
    stored_size(file).await == Some(file.size)
}

/// Sizes of every stored file, for checking the whole catalog at once.
pub async fn stored_sizes() -> Vec<(String, u64)> {
    if is_tauri() {
        return crate::tauri_bridge::tauri_invoke_typed_no_args("models_sizes")
            .await
            .unwrap_or_default();
    }
    let mut out = Vec::new();
    for f in super::catalog::all_files() {
        if let Some(s) = stored_size(f).await {
            out.push((f.id.to_string(), s));
        }
    }
    out
}

/// Read a stored file.
pub async fn read_js(file: &ModelFile) -> Result<js_sys::Uint8Array, ClassifierError> {
    let bytes = if is_tauri() {
        let buf = crate::tauri_bridge::tauri_invoke_bytes("models_read", &FileArgs::of(file))
            .await
            .map_err(|e| storage_err(file, e))?;
        js_sys::Uint8Array::new(&buf)
    } else {
        let handle = opfs_handle(file.id)
            .await
            .ok_or(ClassifierError::NotDownloaded {
                file: file.id.to_string(),
            })?;
        let f: web_sys::File = JsFuture::from(handle.get_file())
            .await
            .map_err(|e| storage_err(file, js_msg(&e)))?
            .unchecked_into();
        let buf = JsFuture::from(f.array_buffer())
            .await
            .map_err(|e| storage_err(file, js_msg(&e)))?;
        js_sys::Uint8Array::new(&buf)
    };
    if bytes.length() as u64 != file.size {
        return Err(ClassifierError::NotDownloaded {
            file: file.id.to_string(),
        });
    }
    Ok(bytes)
}

/// Read a stored file into WASM memory (for the small label files).
pub async fn read(file: &ModelFile) -> Result<Vec<u8>, ClassifierError> {
    Ok(read_js(file).await?.to_vec())
}

/// Delete a stored file. Deleting a file that is not stored is not an error.
pub async fn delete(file: &ModelFile) -> Result<(), ClassifierError> {
    if is_tauri() {
        crate::tauri_bridge::tauri_invoke_args("models_delete", &FileArgs::of(file))
            .await
            .map_err(|e| storage_err(file, e))?;
        return Ok(());
    }
    let dir = opfs_dir().await.map_err(|e| storage_err(file, e))?;
    if opfs_handle(file.id).await.is_some() {
        JsFuture::from(dir.remove_entry(file.id))
            .await
            .map_err(|e| storage_err(file, js_msg(&e)))?;
    }
    Ok(())
}

/// Delete the files that earlier versions of the app stored and nothing uses
/// now ([`super::catalog::RETIRED_FILES`]). Errors are ignored: a file that
/// cannot be deleted is tried again the next time.
pub async fn delete_retired() {
    for file in super::catalog::RETIRED_FILES {
        let _ = delete(file).await;
    }
}

async fn write_opfs(file: &ModelFile, bytes: &js_sys::Uint8Array) -> Result<(), ClassifierError> {
    let dir = opfs_dir().await.map_err(|e| storage_err(file, e))?;
    let opts = web_sys::FileSystemGetFileOptions::new();
    opts.set_create(true);
    let handle: FileSystemFileHandle =
        JsFuture::from(dir.get_file_handle_with_options(file.id, &opts))
            .await
            .map_err(|e| storage_err(file, js_msg(&e)))?
            .unchecked_into();
    // The writable stream writes to a temporary copy and replaces the file on
    // close, so an interrupted write never leaves half a file.
    let w: FileSystemWritableFileStream = JsFuture::from(handle.create_writable())
        .await
        .map_err(|e| storage_err(file, js_msg(&e)))?
        .unchecked_into();
    JsFuture::from(
        w.write_with_js_u8_array(bytes)
            .map_err(|e| storage_err(file, js_msg(&e)))?,
    )
    .await
    .map_err(|e| storage_err(file, js_msg(&e)))?;
    JsFuture::from(w.close())
        .await
        .map_err(|e| storage_err(file, js_msg(&e)))?;
    Ok(())
}

/// Register the app-wide listener that passes [`PROGRESS_EVENT`]s to the
/// callbacks in [`NATIVE_PROGRESS`]. Events for files with no callback (a
/// download that has finished) are ignored.
fn listen_native_progress() {
    if PROGRESS_LISTENING.with(|l| l.replace(true)) {
        return;
    }
    let callback = Closure::<dyn FnMut(JsValue)>::new(|event: JsValue| {
        let Ok(payload) = js_sys::Reflect::get(&event, &"payload".into()) else {
            return;
        };
        let get = |key: &str| js_sys::Reflect::get(&payload, &key.into()).ok();
        let (Some(file), Some(fraction)) = (
            get("file").and_then(|v| v.as_string()),
            get("fraction").and_then(|v| v.as_f64()),
        ) else {
            return;
        };
        // Take the callback out while it runs, so it can never meet a
        // borrowed map.
        let Some(mut cb) = NATIVE_PROGRESS.with(|m| m.borrow_mut().remove(&file)) else {
            return;
        };
        cb(fraction);
        NATIVE_PROGRESS.with(|m| {
            m.borrow_mut().entry(file).or_insert(cb);
        });
    });
    if !crate::tauri_bridge::tauri_listen(PROGRESS_EVENT, callback) {
        PROGRESS_LISTENING.with(|l| l.set(false));
    }
}

/// Removes a file's progress callback from [`NATIVE_PROGRESS`] when its
/// download ends, however it ends.
struct ProgressRoute(&'static str);

impl Drop for ProgressRoute {
    fn drop(&mut self) {
        NATIVE_PROGRESS.with(|m| {
            m.borrow_mut().remove(self.0);
        });
    }
}

/// Tauri: download `file` in native code (`models_download`).
async fn download_native(
    file: &'static ModelFile,
    on_progress: impl FnMut(f64) + 'static,
) -> Result<(), ClassifierError> {
    listen_native_progress();
    NATIVE_PROGRESS.with(|m| {
        m.borrow_mut()
            .insert(file.id.to_string(), Box::new(on_progress));
    });
    let _route = ProgressRoute(file.id);
    match crate::tauri_bridge::tauri_invoke_js_args("models_download", &FileArgs::of(file)).await {
        Ok(_) => Ok(()),
        // The command fails with a `ClassifierError`; anything else (the
        // command missing, say) is shown as a download error.
        Err(e) => Err(
            serde_wasm_bindgen::from_value(e.clone()).unwrap_or_else(|_| {
                ClassifierError::Download {
                    file: file.id.to_string(),
                    message: js_msg(&e),
                }
            }),
        ),
    }
}

/// Download `file`, check its size and SHA-256, and store it. `on_progress`
/// gets the fraction downloaded (0 to 1) as chunks arrive.
pub async fn download(
    file: &'static ModelFile,
    mut on_progress: impl FnMut(f64) + 'static,
) -> Result<(), ClassifierError> {
    if is_tauri() {
        return download_native(file, on_progress).await;
    }
    let dl_err = |message: String| ClassifierError::Download {
        file: file.id.to_string(),
        message,
    };
    let window = web_sys::window().ok_or_else(|| dl_err("no window".into()))?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str(file.url))
        .await
        .map_err(|e| dl_err(js_msg(&e)))?
        .unchecked_into();
    if !resp.ok() {
        return Err(dl_err(format!("HTTP {}", resp.status())));
    }
    let body = resp
        .body()
        .ok_or_else(|| dl_err("the response has no body".into()))?;
    let reader: web_sys::ReadableStreamDefaultReader = body.get_reader().unchecked_into();

    let total = file.size as u32;
    let all = js_sys::Uint8Array::new_with_length(total);
    let mut at: u64 = 0;
    let mut hasher = Sha256::new();
    on_progress(0.0);
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|e| dl_err(js_msg(&e)))?;
        let done = js_sys::Reflect::get(&chunk, &"done".into())
            .ok()
            .and_then(|d| d.as_bool())
            .unwrap_or(true);
        if done {
            break;
        }
        let value: js_sys::Uint8Array = js_sys::Reflect::get(&chunk, &"value".into())
            .map_err(|e| dl_err(js_msg(&e)))?
            .unchecked_into();
        let n = value.length() as u64;
        if at + n > file.size {
            let _ = reader.cancel();
            return Err(ClassifierError::Corrupt {
                file: file.id.to_string(),
            });
        }
        all.set(&value, at as u32);
        // Hash as the chunks arrive, so there is no single long pause at the end.
        hasher.update(value.to_vec());
        at += n;
        on_progress(at as f64 / file.size as f64);
    }
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if at != file.size || hex != file.sha256 {
        return Err(ClassifierError::Corrupt {
            file: file.id.to_string(),
        });
    }
    write_opfs(file, &all).await
}

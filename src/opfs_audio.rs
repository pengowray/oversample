//! Audio files the app makes itself and is too large to hold in memory,
//! written to the browser's Origin Private File System a piece at a time.
//! The denoised copy of a long recording is one (`crate::denoise`).
//!
//! The files are working copies, not saved documents: the file list is not
//! kept between visits, so a file is deleted when its entry in the file list
//! is closed, and what a closed tab left behind is deleted by the next tab
//! that writes a file.
//!
//! Layout: `oversample-audio/<tab>/<n>/<file name>`. Each file has a folder
//! of its own so that it keeps its real name, which the file list shows. Each
//! tab holds a Web Lock named after its folder for as long as it is open, and
//! a tab deletes the folders whose lock nobody holds.

use std::cell::{Cell, RefCell};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemWritableFileStream, WritableStream,
};

const AUDIO_DIR: &str = "oversample-audio";
/// Start of the name of each tab's Web Lock; the tab's folder name follows.
const LOCK_PREFIX: &str = "oversample-audio-";

#[wasm_bindgen(inline_js = r#"
export function opfs_audio_supported() {
  return typeof FileSystemFileHandle !== "undefined"
    && "createWritable" in FileSystemFileHandle.prototype
    && !!(navigator.storage && navigator.storage.getDirectory);
}
// Takes the lock `name` and keeps it until the tab closes. Resolves once the
// lock is held.
export function opfs_audio_hold_lock(name) {
  if (!navigator.locks) return Promise.resolve();
  return new Promise((held) => {
    navigator.locks.request(name, () => { held(); return new Promise(() => {}); });
  });
}
// Deletes every folder in `dir` whose lock (`prefix` and the folder's name)
// is free, which means the tab that made it has closed.
export async function opfs_audio_sweep(dir, prefix, own) {
  if (!navigator.locks) return;
  const names = [];
  for await (const name of dir.keys()) names.push(name);
  for (const name of names) {
    if (name === own) continue;
    await navigator.locks.request(prefix + name, { ifAvailable: true }, async (lock) => {
      if (lock) await dir.removeEntry(name, { recursive: true }).catch(() => {});
    });
  }
}
export async function opfs_audio_remove(dir, name) {
  await dir.removeEntry(name, { recursive: true }).catch(() => {});
}
// Bytes the site may still store, or -1 when the browser does not say.
export async function opfs_audio_free_bytes() {
  if (!navigator.storage || !navigator.storage.estimate) return -1;
  const e = await navigator.storage.estimate();
  if (typeof e.quota !== "number" || typeof e.usage !== "number") return -1;
  return Math.max(0, e.quota - e.usage);
}
"#)]
extern "C" {
    fn opfs_audio_supported() -> bool;
    fn opfs_audio_hold_lock(name: &str) -> js_sys::Promise;
    fn opfs_audio_sweep(
        dir: &FileSystemDirectoryHandle,
        prefix: &str,
        own: &str,
    ) -> js_sys::Promise;
    fn opfs_audio_remove(dir: &FileSystemDirectoryHandle, name: &str) -> js_sys::Promise;
    fn opfs_audio_free_bytes() -> js_sys::Promise;
}

thread_local! {
    /// This tab's folder name, once its lock is held and old folders are gone.
    static TAB: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Numbers the files this tab has written.
    static NEXT_FILE: Cell<u32> = const { Cell::new(0) };
}

/// True when this browser can write a file a piece at a time.
pub fn supported() -> bool {
    opfs_audio_supported()
}

/// Bytes the site may still store. `None` when the browser does not say.
/// What closed tabs left behind is deleted first, so it does not count.
pub async fn free_bytes() -> Option<u64> {
    let _ = tab_dir().await;
    let bytes = JsFuture::from(opfs_audio_free_bytes())
        .await
        .ok()?
        .as_f64()?;
    (bytes >= 0.0).then_some(bytes as u64)
}

fn js_err(what: &str, e: JsValue) -> String {
    let message = e
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .unwrap_or_else(|| format!("{e:?}"));
    format!("{what}: {message}")
}

/// This tab's folder. The first call takes the tab's lock and deletes the
/// folders of tabs that have closed.
async fn tab_dir() -> Result<FileSystemDirectoryHandle, String> {
    let root = crate::opfs::get_opfs_subdir(AUDIO_DIR).await?;
    let tab = match TAB.with(|t| t.borrow().clone()) {
        Some(tab) => tab,
        None => {
            let tab = format!(
                "{:x}-{:x}",
                js_sys::Date::now() as u64,
                (js_sys::Math::random() * 4e9) as u64
            );
            let _ = JsFuture::from(opfs_audio_hold_lock(&format!("{LOCK_PREFIX}{tab}"))).await;
            let _ = JsFuture::from(opfs_audio_sweep(&root, LOCK_PREFIX, &tab)).await;
            TAB.with(|t| *t.borrow_mut() = Some(tab.clone()));
            tab
        }
    };
    let opts = web_sys::FileSystemGetDirectoryOptions::new();
    opts.set_create(true);
    Ok(
        JsFuture::from(root.get_directory_handle_with_options(&tab, &opts))
            .await
            .map_err(|e| js_err("storage folder", e))?
            .unchecked_into(),
    )
}

/// Where a written file is, for [`delete`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFile {
    /// The file's own folder inside this tab's folder.
    folder: String,
}

/// A file being written. [`Writer::finish`] keeps it and [`Writer::discard`]
/// deletes it; a writer that is dropped leaves its folder for the next sweep.
pub struct Writer {
    tab: FileSystemDirectoryHandle,
    folder: String,
    handle: FileSystemFileHandle,
    stream: FileSystemWritableFileStream,
}

impl Writer {
    /// Starts the file `name`.
    pub async fn create(name: &str) -> Result<Writer, String> {
        let tab = tab_dir().await?;
        let folder = NEXT_FILE.with(|n| {
            n.set(n.get() + 1);
            n.get().to_string()
        });
        let dir_opts = web_sys::FileSystemGetDirectoryOptions::new();
        dir_opts.set_create(true);
        let dir: FileSystemDirectoryHandle =
            JsFuture::from(tab.get_directory_handle_with_options(&folder, &dir_opts))
                .await
                .map_err(|e| js_err("storage folder", e))?
                .unchecked_into();
        let file_opts = web_sys::FileSystemGetFileOptions::new();
        file_opts.set_create(true);
        let handle: FileSystemFileHandle =
            JsFuture::from(dir.get_file_handle_with_options(name, &file_opts))
                .await
                .map_err(|e| js_err("new file", e))?
                .unchecked_into();
        let stream: FileSystemWritableFileStream = JsFuture::from(handle.create_writable())
            .await
            .map_err(|e| js_err("new file", e))?
            .unchecked_into();
        Ok(Writer {
            tab,
            folder,
            handle,
            stream,
        })
    }

    /// Adds `data` (an `ArrayBuffer` or a typed array) to the end of the file.
    pub async fn write(&self, data: &js_sys::Object) -> Result<(), String> {
        let written = self
            .stream
            .write_with_buffer_source(data)
            .map_err(|e| js_err("write", e))?;
        JsFuture::from(written)
            .await
            .map(|_| ())
            .map_err(|e| js_err("write", e))
    }

    /// Closes the file and returns it for reading, with where it is stored.
    pub async fn finish(self) -> Result<(web_sys::File, StoredFile), String> {
        let stream: &WritableStream = self.stream.unchecked_ref();
        if let Err(e) = JsFuture::from(stream.close()).await {
            let _ = JsFuture::from(opfs_audio_remove(&self.tab, &self.folder)).await;
            return Err(js_err("write", e));
        }
        let file = JsFuture::from(self.handle.get_file())
            .await
            .map_err(|e| js_err("read back", e))?
            .unchecked_into();
        Ok((
            file,
            StoredFile {
                folder: self.folder,
            },
        ))
    }

    /// Stops writing and deletes what was written.
    pub async fn discard(self) {
        let stream: &WritableStream = self.stream.unchecked_ref();
        let _ = JsFuture::from(stream.abort()).await;
        let _ = JsFuture::from(opfs_audio_remove(&self.tab, &self.folder)).await;
    }
}

/// Deletes a file that [`Writer::finish`] returned.
pub async fn delete(file: &StoredFile) {
    if let Ok(tab) = tab_dir().await {
        let _ = JsFuture::from(opfs_audio_remove(&tab, &file.folder)).await;
    }
}

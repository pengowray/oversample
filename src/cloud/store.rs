//! Where annotation sets and projects live on this device (browser), and
//! which of those places is in use.
//!
//! - The device store: the folders the app has always used
//!   (`oversample-annotations`, `oversample-projects`). It holds what was
//!   made while signed out.
//! - One cache per Google account, under `oversample-accounts/<id>/`, keyed
//!   by the account's stable Drive `permissionId` (not the email). While
//!   signed in, the account is the home of its files and the cache holds
//!   them on this device, with any edits not yet uploaded. `sync.json` there
//!   holds what was last seen of each Drive file.
//!
//! Each open file has one home (`home_of`), set when it is loaded; its saves
//! go there even if the active store changes later. A store switch reloads
//! the open files that belong to the new store (`cloud::sync`).
//!
//! Writes and deletions of one file go under one lock (`with_lock`, the Web
//! Locks API, shared by all tabs of the app), and a per-file counter
//! (`epoch`) lets a save that was queued before a deletion or a reload skip
//! its write.
//!
//! The Android app uses this too, with one difference: its device store's
//! annotation sets stay in the app's own annotations folder (Tauri
//! commands), where they were before Google sign-in existed. Account caches
//! are in the webview's OPFS, as in the browser.
//!
//! The desktop app does not use this yet: there the active store is always
//! the device store (its own annotations folder).

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

/// A place on this device that holds annotation sets and projects.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Store {
    /// Made while signed out (or before Google sign-in existed).
    Device,
    /// The cache of one Google account, by its Drive `permissionId`.
    Account(String),
}

/// Folder of the device store's annotation sets (as before).
pub const DEVICE_ANNOTATIONS: &str = "oversample-annotations";
/// Folder of the device store's projects (as before).
pub const DEVICE_PROJECTS: &str = "oversample-projects";
const ACCOUNTS: &str = "oversample-accounts";

impl Store {
    /// Folder (path from the OPFS root) of this store's annotation sets.
    pub fn annotations_dir(&self) -> String {
        match self {
            Store::Device => DEVICE_ANNOTATIONS.to_string(),
            Store::Account(id) => format!("{ACCOUNTS}/{id}/annotations"),
        }
    }

    /// Folder (path from the OPFS root) of this store's projects.
    pub fn projects_dir(&self) -> String {
        match self {
            Store::Device => DEVICE_PROJECTS.to_string(),
            Store::Account(id) => format!("{ACCOUNTS}/{id}/projects"),
        }
    }

    /// Folder of the account's own files (`sync.json`).
    pub fn account_dir(&self) -> Option<String> {
        match self {
            Store::Device => None,
            Store::Account(id) => Some(format!("{ACCOUNTS}/{id}")),
        }
    }
}

/// Whether `id` may be used as a folder name: Drive permission ids are
/// digits (and sometimes letters). Anything else is refused.
pub fn account_id_is_safe(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

thread_local! {
    static ACTIVE: RefCell<Store> = const { RefCell::new(Store::Device) };
    /// Open files (by `LoadedFile.id`) and the store their set belongs to.
    static HOMES: RefCell<HashMap<u64, Store>> = RefCell::new(HashMap::new());
    /// Open projects (by id) and their store.
    static PROJECT_HOMES: RefCell<HashMap<String, Store>> = RefCell::new(HashMap::new());
    /// Per file in a store: goes up at each deletion or reload, so that a
    /// write queued before it is skipped.
    static EPOCHS: RefCell<HashMap<(String, String), u64>> = RefCell::new(HashMap::new());
}

/// The store new files go to: the signed-in account's cache, or the device
/// store.
pub fn active() -> Store {
    ACTIVE.with(|a| a.borrow().clone())
}

pub fn set_active(store: Store) {
    ACTIVE.with(|a| *a.borrow_mut() = store);
}

/// The home of open file `file_id`'s annotations (the active store if it
/// has none yet).
pub fn home_of(file_id: u64) -> Store {
    HOMES
        .with(|h| h.borrow().get(&file_id).cloned())
        .unwrap_or_else(active)
}

pub fn set_home(file_id: u64, store: Store) {
    HOMES.with(|h| h.borrow_mut().insert(file_id, store));
}

/// Forget open file `file_id`'s home (it is worked out again at its next
/// load).
pub fn forget_home(file_id: u64) {
    HOMES.with(|h| h.borrow_mut().remove(&file_id));
}

/// Open files whose home is `store`.
pub fn files_with_home(store: &Store) -> Vec<u64> {
    HOMES.with(|h| {
        h.borrow()
            .iter()
            .filter(|(_, s)| *s == store)
            .map(|(id, _)| *id)
            .collect()
    })
}

pub fn project_home(id: &str) -> Store {
    PROJECT_HOMES
        .with(|h| h.borrow().get(id).cloned())
        .unwrap_or_else(active)
}

pub fn set_project_home(id: &str, store: Store) {
    PROJECT_HOMES.with(|h| h.borrow_mut().insert(id.to_string(), store));
}

/// The current write counter of `key` in folder `dir`.
pub fn epoch(dir: &str, key: &str) -> u64 {
    EPOCHS.with(|e| {
        e.borrow()
            .get(&(dir.to_string(), key.to_string()))
            .copied()
            .unwrap_or(0)
    })
}

/// Make writes of `key` in `dir` that were queued before now skip theirs.
pub fn bump_epoch(dir: &str, key: &str) {
    EPOCHS.with(|e| {
        *e.borrow_mut()
            .entry((dir.to_string(), key.to_string()))
            .or_default() += 1
    });
}

/// The lock name for one file in one folder.
pub fn file_lock(dir: &str, key: &str) -> String {
    format!("oversample:{dir}/{key}")
}

/// The lock name for an account's `sync.json`.
pub fn sync_lock(account_dir: &str) -> String {
    format!("oversample:{account_dir}/sync.json")
}

/// A future returned by a lock holder.
pub type LockedFuture<T> = Pin<Box<dyn Future<Output = T>>>;

/// Run `f` while holding the Web Lock `name`, shared by every tab of this
/// app, and return its result; `None` if the lock request failed and `f`
/// did not run. Without the Web Locks API, `f` runs unlocked. Locks are
/// always taken in the same order (a file's lock, then `sync.json`'s) so
/// that two tabs cannot wait on each other.
pub async fn with_lock<T: 'static>(
    name: &str,
    f: impl FnOnce() -> LockedFuture<T> + 'static,
) -> Option<T> {
    let locks = web_sys::window()
        .map(|w| w.navigator())
        .and_then(|n| js_sys::Reflect::get(&n, &"locks".into()).ok())
        .filter(|l| !l.is_undefined() && !l.is_null());
    let request = locks.as_ref().and_then(|l| {
        js_sys::Reflect::get(l, &"request".into())
            .ok()
            .and_then(|r| r.dyn_into::<js_sys::Function>().ok())
    });
    let (Some(locks), Some(request)) = (locks, request) else {
        return Some(f().await);
    };
    let out: Rc<RefCell<Option<T>>> = Rc::new(RefCell::new(None));
    let out2 = out.clone();
    let mut f = Some(f);
    let callback = Closure::<dyn FnMut(JsValue) -> js_sys::Promise>::new(move |_lock: JsValue| {
        let out = out2.clone();
        let f = f.take();
        wasm_bindgen_futures::future_to_promise(async move {
            if let Some(f) = f {
                let v = f().await;
                *out.borrow_mut() = Some(v);
            }
            Ok(JsValue::UNDEFINED)
        })
    });
    let promise = request.call2(
        &locks,
        &JsValue::from_str(name),
        callback.as_ref().unchecked_ref(),
    );
    if let Ok(p) = promise {
        if let Ok(p) = p.dyn_into::<js_sys::Promise>() {
            let _ = wasm_bindgen_futures::JsFuture::from(p).await;
        }
    }
    drop(callback);
    let v = out.borrow_mut().take();
    if v.is_none() {
        log::warn!("Web Lock {name}: the request failed");
    }
    v
}

// ── Files in a store ───────────────────────────────────────────────────

thread_local! {
    static ANDROID_APP: bool = crate::tauri_bridge::get_tauri_internals().is_some()
        && crate::tauri_bridge::is_android();
}

/// Running in the Android app, which keeps annotation sets in these stores
/// (the desktop app does not yet).
pub fn android_app() -> bool {
    ANDROID_APP.with(|a| *a)
}

/// Where these stores are, in user text: "in this browser", or "on this
/// device" in the Android app (matching its "Saved on this device").
pub fn here() -> &'static str {
    if android_app() {
        "on this device"
    } else {
        "in this browser"
    }
}

/// "this browser's", or "this device's" in the Android app.
pub fn heres() -> &'static str {
    if android_app() {
        "this device's"
    } else {
        "this browser's"
    }
}

/// `dir` is the Android app's device store of annotation sets, which is
/// the app's own annotations folder rather than OPFS.
fn in_app_folder(dir: &str) -> bool {
    dir == DEVICE_ANNOTATIONS && android_app()
}

pub async fn read(dir: &str, key: &str) -> Option<String> {
    read_result(dir, key).await.ok().flatten()
}

/// Like `read`, but a failed read is an error, not a missing file.
pub async fn read_result(dir: &str, key: &str) -> Result<Option<String>, String> {
    if in_app_folder(dir) {
        return crate::opfs::tauri_load_central(key).await;
    }
    crate::opfs::opfs_load_in(dir, key).await
}

pub async fn write(dir: &str, key: &str, text: &str) -> Result<(), String> {
    if in_app_folder(dir) {
        return crate::opfs::tauri_save_central(key, text).await;
    }
    crate::opfs::opfs_save_in(dir, key, text).await
}

pub async fn remove(dir: &str, key: &str) -> Result<(), String> {
    if in_app_folder(dir) {
        return crate::opfs::tauri_delete_central(key).await;
    }
    crate::opfs::opfs_remove_in(dir, key).await
}

pub async fn keys(dir: &str, extension: &str) -> Vec<String> {
    let mut names = if in_app_folder(dir) {
        crate::tauri_bridge::tauri_invoke_typed_no_args::<Vec<String>>("list_central_annotations")
            .await
            .unwrap_or_default()
    } else {
        crate::opfs::list_names_in(dir).await.unwrap_or_default()
    };
    names.retain(|n| n.ends_with(extension));
    names
}

//! When to compare, upload and download Google Drive copies, and the state
//! the settings block shows (`AppState::drive`).
//!
//! Rules (see also `rules`):
//! - Local storage stays the source of truth. Nothing here blocks, delays or
//!   fails a local save, and failures are shown only in the settings block.
//! - A key is not uploaded until its Drive copy has been looked up and
//!   compared since sign-in (`checked`).
//! - Uploads wait until 5 s after the last save of a key, one at a time per
//!   key, and start at once when the page is hidden.
//! - Before replacing a Drive file, its revision is compared with the one
//!   this device last saw. If it changed, the copies are compared again.
//! - At sign-in (and at start when signed in) every local and Drive file is
//!   compared in the background. A file opened later is compared again,
//!   because another device may have changed it since.
//! - A Drive copy for an open file goes through the normal load path
//!   (`opfs::adopt_drive_copy`). Its undo and redo steps are dropped: they
//!   are snapshots of a set that no longer exists, and undoing one would
//!   upload it over the Drive change.
//! - Sign-out and a session that ends clear the token, the cached Drive ids
//!   and revisions and the `checked` keys. Local files, the list of keys
//!   waiting to upload, and the fingerprint of each key's last-seen Drive
//!   content stay: with them, a change made while signed out is still a
//!   local change at the next sign-in, and uploads instead of being merged
//!   away. The fingerprints are dropped when a different account signs in.
//!
//! Async tasks note the `generation` when they start and stop after an
//! await if it changed (sign-out or session end in the meantime).

use super::drive::{self, DriveError, DriveFile};
use super::google_auth;
use super::rules::{self, Action, FileCopy, Kind};
use crate::annotations::AnnotationSet;
use crate::project::BatProject;
use crate::state::store_fields::*;
use crate::state::{AppState, DriveProblem};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

/// Wait this long after the last save of a file before uploading it.
const UPLOAD_DELAY_MS: i32 = 5_000;
/// Check the sign-in window this often.
const SIGN_IN_POLL_MS: i32 = 1_000;
/// Stop waiting for the sign-in window after this long.
const SIGN_IN_TIMEOUT_MS: f64 = 5.0 * 60_000.0;
/// After the sign-in window closes, ask once or twice more before giving up.
const AFTER_CLOSE_MS: f64 = 3_000.0;

const BATM: &str = "batm";
const BATPROJ: &str = "batproj";

/// localStorage: what this device last saw of each Drive file (`Memory`).
const MEMORY_KEY: &str = "oversample_drive_seen";
/// localStorage: projects deleted on this device whose Drive copies still
/// have to go to the Drive trash.
const DELETED_PROJECTS_KEY: &str = "oversample_drive_deleted_projects";

/// What this device last saw of the Drive files of one account.
#[derive(Default, Serialize, Deserialize)]
struct Memory {
    account: String,
    /// By sync key (`batm:<key>` or `batproj:<id>`).
    files: BTreeMap<String, Seen>,
}

/// One Drive file as last seen: its id, revision and content fingerprint.
#[derive(Clone, Serialize, Deserialize)]
struct Seen {
    id: String,
    rev: Option<String>,
    fp: String,
}

/// A change waiting to upload: the file name in Drive and the text.
#[derive(Clone)]
struct Pending {
    name: String,
    text: String,
    /// Order of the save that produced it; an older save never replaces a
    /// newer one.
    seq: u64,
}

#[derive(Default)]
struct Sync {
    state: Option<AppState>,
    generation: u64,
    signed_in: bool,
    /// Folder ids by `osFolder` marker.
    folders: HashMap<&'static str, String>,
    checked: HashSet<String>,
    checking: HashSet<String>,
    pending: HashMap<String, Pending>,
    timers: HashMap<String, i32>,
    uploading: HashSet<String>,
    /// Keys whose Drive copy belongs to a different recording (same name and
    /// size). They are neither uploaded nor downloaded.
    blocked: HashSet<String>,
    memory: Memory,
    reconciling: bool,
    save_seq: u64,
    /// A folder lookup or creation is running (one at a time, so that
    /// parallel uploads do not each create the folders).
    folder_busy: bool,
    /// Uploads running now, across all keys.
    uploads_running: usize,
    /// Counts sign-in attempts; an older attempt's polling stops.
    sign_in_attempt: u64,
    /// Goes up whenever the per-file state may have changed, so that
    /// `file_drive_state` callers update (browser only; made in `init`).
    version: Option<ArcRwSignal<u64>>,
}

/// Most uploads that run at the same time (a first sign-in can have many
/// files to upload).
const MAX_PARALLEL_UPLOADS: usize = 2;

thread_local! {
    static SYNC: RefCell<Sync> = RefCell::new(Sync::default());
}

fn with<R>(f: impl FnOnce(&mut Sync) -> R) -> R {
    SYNC.with(|s| f(&mut s.borrow_mut()))
}

fn app_state() -> Option<AppState> {
    with(|s| s.state)
}

fn generation() -> u64 {
    with(|s| s.generation)
}

/// Whether the session that started at `gen` is still the current one.
fn alive(gen: u64) -> bool {
    with(|s| s.signed_in && s.generation == gen)
}

fn batm_key(key: &str) -> String {
    format!("{BATM}:{key}")
}

fn project_key(id: &str) -> String {
    format!("{BATPROJ}:{id}")
}

/// The kind and local key of a sync key.
fn split(sync_key: &str) -> (&str, &str) {
    sync_key.split_once(':').unwrap_or(("", sync_key))
}

fn spawn(fut: impl std::future::Future<Output = Result<(), DriveError>> + 'static) {
    let gen = generation();
    wasm_bindgen_futures::spawn_local(async move {
        match fut.await {
            Ok(()) => {}
            Err(e) => on_error(gen, e),
        }
    });
}

// ── Memory (localStorage) ───────────────────────────────────────────────

fn load_memory() -> Memory {
    crate::settings::get_raw(MEMORY_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_memory(memory: &Memory) {
    if let Ok(json) = serde_json::to_string(memory) {
        crate::settings::set_raw(MEMORY_KEY, &json);
    }
}

fn seen(sync_key: &str) -> Option<Seen> {
    with(|s| s.memory.files.get(sync_key).cloned())
}

fn set_seen(sync_key: &str, value: Option<Seen>) {
    with(|s| {
        match value {
            Some(v) => s.memory.files.insert(sync_key.to_string(), v),
            None => s.memory.files.remove(sync_key),
        };
        save_memory(&s.memory);
    });
}

fn deleted_projects() -> Vec<String> {
    crate::settings::get_raw(DELETED_PROJECTS_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn set_deleted_projects(ids: &[String]) {
    if ids.is_empty() {
        crate::settings::remove_raw(DELETED_PROJECTS_KEY);
    } else if let Ok(json) = serde_json::to_string(ids) {
        crate::settings::set_raw(DELETED_PROJECTS_KEY, &json);
    }
}

// ── Settings block state ────────────────────────────────────────────────

/// Copy the counts the settings block shows into `AppState::drive`.
fn publish() {
    let Some(state) = app_state() else {
        return;
    };
    if let Some(v) = with(|s| s.version.clone()) {
        v.update(|n| *n = n.wrapping_add(1));
    }
    // A comparison of all files counts as saving: until it ends, "Saved to
    // Google Drive" would not yet be known to be true.
    let (waiting, uploading) = with(|s| {
        (
            s.pending.len(),
            !s.uploading.is_empty() || (s.signed_in && s.reconciling),
        )
    });
    if state.drive.waiting().get_untracked() != waiting {
        state.drive.waiting().set(waiting);
    }
    if state.drive.uploading().get_untracked() != uploading {
        state.drive.uploading().set(uploading);
    }
}

fn set_problem(problem: Option<DriveProblem>) {
    if let Some(state) = app_state() {
        if state.drive.problem().get_untracked() != problem {
            state.drive.problem().set(problem);
        }
    }
}

fn on_error(gen: u64, e: DriveError) {
    if with(|s| s.generation != gen) {
        return;
    }
    log::warn!("Google Drive: {e:?}");
    match e {
        DriveError::SignedOut => end_session(),
        DriveError::Unreachable => set_problem(Some(DriveProblem::Unreachable)),
        DriveError::Full => set_problem(Some(DriveProblem::Full)),
        DriveError::Refused(_) => set_problem(Some(DriveProblem::Refused)),
    }
}

// ── Start, sign-in, sign-out ────────────────────────────────────────────

/// Start the feature (browser only): ask the server whether it offers
/// Google sign-in and whether this browser is signed in.
pub fn init(state: AppState) {
    if state.is_tauri {
        return;
    }
    with(|s| {
        s.state = Some(state);
        s.memory = load_memory();
        s.version = Some(ArcRwSignal::new(0));
    });
    add_window_listeners();
    wasm_bindgen_futures::spawn_local(async move {
        match google_auth::status().await {
            None => state.drive.available().set(false),
            Some(signed_in) => {
                state.drive.available().set(true);
                if signed_in {
                    start_session().await;
                }
            }
        }
    });
}

/// Open the Google sign-in window. Call from a click handler (browsers
/// block pop-up windows opened later).
pub fn sign_in(state: AppState) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let url = google_auth::login_url();
    let popup = window
        .open_with_url_and_target_and_features(
            &url,
            "oversample-google-sign-in",
            "popup,width=500,height=650",
        )
        .ok()
        .flatten();
    let Some(popup) = popup else {
        state.drive.popup_blocked().set(true);
        return;
    };
    state.drive.popup_blocked().set(false);
    state.drive.signing_in().set(true);
    let attempt = with(|s| {
        s.sign_in_attempt += 1;
        s.sign_in_attempt
    });
    wasm_bindgen_futures::spawn_local(async move {
        let started = js_sys::Date::now();
        let mut closed_at: Option<f64> = None;
        loop {
            crate::web_util::sleep_ms(SIGN_IN_POLL_MS).await;
            if with(|s| s.sign_in_attempt != attempt || s.signed_in) {
                return;
            }
            if google_auth::status().await == Some(true) {
                state.drive.signing_in().set(false);
                if !with(|s| s.signed_in) {
                    start_session().await;
                }
                return;
            }
            let now = js_sys::Date::now();
            // Some sign-in pages cut the link to this window, which then
            // reads as closed while it is still open. So after it reads
            // closed the button is offered again, but the status is still
            // checked until the time limit.
            if popup.closed().unwrap_or(true) {
                let at = *closed_at.get_or_insert(now);
                if now - at > AFTER_CLOSE_MS {
                    state.drive.signing_in().set(false);
                }
            }
            if now - started > SIGN_IN_TIMEOUT_MS {
                break;
            }
        }
        if with(|s| s.sign_in_attempt == attempt) {
            state.drive.signing_in().set(false);
        }
    });
}

/// Signed in: find the account, compare all files, upload what waits.
async fn start_session() {
    let Some(state) = app_state() else {
        return;
    };
    let Some(gen) = with(|s| {
        if s.signed_in && s.reconciling {
            return None;
        }
        s.signed_in = true;
        Some(s.generation)
    }) else {
        return;
    };
    state.drive.signed_in().set(true);
    state.drive.session_ended().set(false);
    let email = match drive::account_email().await {
        Ok(email) => email,
        Err(e) => {
            on_error(gen, e);
            return;
        }
    };
    if !alive(gen) {
        return;
    }
    with(|s| {
        // Another account: what was seen of the last one's Drive is no base.
        if s.memory.account != email {
            s.memory = Memory {
                account: email.clone(),
                files: BTreeMap::new(),
            };
            save_memory(&s.memory);
        }
    });
    state.drive.email().set(Some(email));
    set_problem(None);
    if let Err(e) = reconcile(gen).await {
        on_error(gen, e);
    }
}

/// Forget everything about the Drive session. Local files and the keys
/// waiting to upload stay.
fn clear_session() {
    google_auth::forget_token();
    with(|s| {
        s.generation += 1;
        s.signed_in = false;
        s.folders.clear();
        s.checked.clear();
        s.checking.clear();
        s.uploading.clear();
        s.blocked.clear();
        s.reconciling = false;
        s.folder_busy = false;
        s.uploads_running = 0;
        // Keep each key's fingerprint (see the module notes); forget the
        // Drive ids and revisions.
        for seen in s.memory.files.values_mut() {
            seen.id.clear();
            seen.rev = None;
        }
        save_memory(&s.memory);
        let window = web_sys::window();
        for (_, handle) in s.timers.drain() {
            if let Some(w) = &window {
                w.clear_timeout_with_handle(handle);
            }
        }
    });
    if let Some(state) = app_state() {
        state.drive.signed_in().set(false);
        state.drive.email().set(None);
        state.drive.problem().set(None);
        state.drive.signing_in().set(false);
    }
    publish();
}

/// The server says the sign-in is no longer valid.
fn end_session() {
    clear_session();
    if let Some(state) = app_state() {
        state.drive.session_ended().set(true);
    }
}

/// Sign out at the user's request.
pub fn sign_out(state: AppState) {
    clear_session();
    state.drive.session_ended().set(false);
    wasm_bindgen_futures::spawn_local(async move {
        google_auth::logout().await;
    });
}

/// Try again after a failure: compare all files and upload what waits.
pub fn retry() {
    if !with(|s| s.signed_in) {
        return;
    }
    set_problem(None);
    let needs_account = app_state().is_some_and(|s| s.drive.email().get_untracked().is_none());
    if needs_account {
        wasm_bindgen_futures::spawn_local(start_session());
        return;
    }
    restart_waiting_uploads();
    let gen = generation();
    spawn(async move { reconcile(gen).await });
}

/// Start the upload of every compared key that waits with no upload timer
/// and no upload running (its last upload failed).
fn restart_waiting_uploads() {
    let keys: Vec<String> = with(|s| {
        s.pending
            .keys()
            .filter(|k| {
                s.checked.contains(*k) && !s.timers.contains_key(*k) && !s.uploading.contains(*k)
            })
            .cloned()
            .collect()
    });
    for sk in keys {
        schedule_upload(&sk, 0);
    }
}

fn add_window_listeners() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let on_visibility = Closure::<dyn Fn()>::new(|| {
        let hidden = web_sys::window()
            .and_then(|w| w.document())
            .is_some_and(|d| d.visibility_state() == web_sys::VisibilityState::Hidden);
        if hidden {
            flush_now();
        }
    });
    if let Some(doc) = window.document() {
        let _ = doc.add_event_listener_with_callback(
            "visibilitychange",
            on_visibility.as_ref().unchecked_ref(),
        );
    }
    on_visibility.forget();
    // Back from the sign-in window: ask at once whether sign-in finished.
    let on_focus = Closure::<dyn Fn()>::new(|| {
        let Some(state) = app_state() else {
            return;
        };
        if with(|s| s.signed_in) || !state.drive.available().get_untracked() {
            return;
        }
        if with(|s| s.sign_in_attempt) == 0 {
            return;
        }
        wasm_bindgen_futures::spawn_local(async move {
            if google_auth::status().await == Some(true) && !with(|s| s.signed_in) {
                state.drive.signing_in().set(false);
                start_session().await;
            }
        });
    });
    let _ = window.add_event_listener_with_callback("focus", on_focus.as_ref().unchecked_ref());
    on_focus.forget();
    let on_online = Closure::<dyn Fn()>::new(retry);
    let _ = window.add_event_listener_with_callback("online", on_online.as_ref().unchecked_ref());
    on_online.forget();
}

// ── Per-file state ──────────────────────────────────────────────────────

/// Where the Drive copy of one annotation file stands, for the line that
/// says where a file's annotations are saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDriveState {
    /// Not compared with Drive yet in this sign-in.
    Checking,
    /// The Drive copy matches the local copy.
    Saved,
    /// A change waits for the upload delay.
    Waiting,
    Uploading,
    /// A change waits, and the last Drive request failed.
    Problem(DriveProblem),
    /// Drive holds a set for another recording under this key; this file's
    /// annotations are not copied.
    NotCopied,
}

/// The Drive state of the annotation file stored under `key`, or `None`
/// when not signed in (or in Tauri). Reactive: re-runs when it may change.
pub fn file_drive_state(state: AppState, key: &str) -> Option<FileDriveState> {
    if state.is_tauri || !state.drive.signed_in().get() {
        return None;
    }
    let problem = state.drive.problem().get();
    if let Some(v) = with(|s| s.version.clone()) {
        v.track();
    }
    let sk = batm_key(key);
    Some(with(|s| {
        if s.blocked.contains(&sk) {
            FileDriveState::NotCopied
        } else if s.uploading.contains(&sk) {
            FileDriveState::Uploading
        } else if s.pending.contains_key(&sk) {
            match problem {
                Some(p) => FileDriveState::Problem(p),
                None if s.checked.contains(&sk) => FileDriveState::Waiting,
                None => FileDriveState::Checking,
            }
        } else if s.checked.contains(&sk) {
            FileDriveState::Saved
        } else {
            FileDriveState::Checking
        }
    }))
}

// ── Hooks called by local storage ───────────────────────────────────────

/// A save order number for `annotation_saved`. Take it when the save
/// starts, before the local write is awaited.
pub fn next_save_seq() -> u64 {
    with(|s| {
        s.save_seq += 1;
        s.save_seq
    })
}

/// An annotation set was written to local storage under `key`. `text` is
/// what goes to Drive (without the Tauri-only file path).
pub fn annotation_saved(key: &str, audio_name: &str, text: String, seq: u64) {
    if with(|s| s.state.is_none()) {
        return;
    }
    queue(batm_key(key), format!("{audio_name}.batm"), text, seq);
}

/// A project was written to local storage.
pub fn project_saved(project: &BatProject, text: String) {
    if with(|s| s.state.is_none()) {
        return;
    }
    let seq = next_save_seq();
    queue(
        project_key(&project.id),
        project_file_name(project),
        text,
        seq,
    );
}

fn project_file_name(project: &BatProject) -> String {
    let name = project
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("Untitled project");
    format!("{name}.batproj")
}

/// A project was deleted from local storage: its Drive copy goes to the
/// Drive trash (now, or at the next sign-in).
pub fn project_deleted(id: &str) {
    if with(|s| s.state.is_none()) {
        return;
    }
    let mut ids = deleted_projects();
    if !ids.iter().any(|i| i == id) {
        ids.push(id.to_string());
        set_deleted_projects(&ids);
    }
    let sk = project_key(id);
    with(|s| {
        s.pending.remove(&sk);
        s.checked.remove(&sk);
    });
    publish();
    if with(|s| s.signed_in) {
        let gen = generation();
        spawn(async move { trash_deleted_projects(gen).await });
    }
}

/// A file's saved annotations have been read from local storage (or there
/// were none): compare with the Drive copy.
pub fn annotations_loaded(state: AppState, file_id: u64) {
    if state.is_tauri || !with(|s| s.signed_in) {
        return;
    }
    let Some(key) = open_file_key(state, file_id) else {
        return;
    };
    // Compared again even if already compared since sign-in: another
    // device may have changed it since.
    let sk = batm_key(&key);
    if with(|s| s.checking.contains(&sk)) {
        return;
    }
    let gen = generation();
    spawn(async move { check_annotations(gen, key, None).await });
}

/// Keep the newest text for `sync_key` and upload it after the delay.
fn queue(sync_key: String, name: String, text: String, seq: u64) {
    let newer = with(|s| {
        if s.pending.get(&sync_key).is_some_and(|p| p.seq > seq) {
            return false;
        }
        s.pending
            .insert(sync_key.clone(), Pending { name, text, seq });
        true
    });
    if !newer {
        return;
    }
    publish();
    if !with(|s| s.signed_in) {
        return;
    }
    if with(|s| s.checked.contains(&sync_key)) {
        schedule_upload(&sync_key, UPLOAD_DELAY_MS);
    } else {
        start_check(sync_key);
    }
}

/// Compare `sync_key` with its Drive copy, unless that is running already.
fn start_check(sync_key: String) {
    if with(|s| s.checking.contains(&sync_key) || s.blocked.contains(&sync_key)) {
        return;
    }
    let gen = generation();
    let (kind, key) = split(&sync_key);
    let key = key.to_string();
    if kind == BATPROJ {
        spawn(async move { check_project(gen, key, None).await });
    } else {
        spawn(async move { check_annotations(gen, key, None).await });
    }
}

fn schedule_upload(sync_key: &str, delay_ms: i32) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Some(old) = with(|s| s.timers.remove(sync_key)) {
        window.clear_timeout_with_handle(old);
    }
    let sk = sync_key.to_string();
    let cb = Closure::once(move || {
        with(|s| s.timers.remove(&sk));
        let gen = generation();
        spawn(async move { upload(gen, sk).await });
    });
    if let Ok(handle) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        cb.as_ref().unchecked_ref(),
        delay_ms,
    ) {
        with(|s| s.timers.insert(sync_key.to_string(), handle));
    }
    cb.forget();
}

/// Start every waiting upload now (the page is being hidden).
fn flush_now() {
    if !with(|s| s.signed_in) {
        return;
    }
    let keys: Vec<String> = with(|s| s.pending.keys().cloned().collect());
    let window = web_sys::window();
    for sk in keys {
        if !with(|s| s.checked.contains(&sk)) {
            continue;
        }
        if let (Some(handle), Some(w)) = (with(|s| s.timers.remove(&sk)), &window) {
            w.clear_timeout_with_handle(handle);
        }
        let gen = generation();
        spawn(async move { upload(gen, sk).await });
    }
}

// ── Folders ─────────────────────────────────────────────────────────────

/// The Drive folder for files of `kind`, found or created once per session.
async fn folder_for(kind: &str) -> Result<String, DriveError> {
    let (marker, name): (&'static str, &str) = if kind == BATPROJ {
        ("projects", "projects")
    } else {
        ("annotations", "annotations")
    };
    // One lookup at a time: parallel uploads would each create the folders.
    while with(|s| s.folder_busy) {
        crate::web_util::sleep_ms(50).await;
    }
    if let Some(id) = with(|s| s.folders.get(marker).cloned()) {
        return Ok(id);
    }
    with(|s| s.folder_busy = true);
    let result = find_folders(marker, name).await;
    with(|s| s.folder_busy = false);
    result
}

async fn find_folders(marker: &'static str, name: &str) -> Result<String, DriveError> {
    let root = match with(|s| s.folders.get("root").cloned()) {
        Some(id) => id,
        None => {
            let id = drive::folder("root", "Oversample", None).await?;
            with(|s| s.folders.insert("root", id.clone()));
            id
        }
    };
    let id = drive::folder(marker, name, Some(&root)).await?;
    with(|s| s.folders.insert(marker, id.clone()));
    Ok(id)
}

// ── Uploads ─────────────────────────────────────────────────────────────

fn fingerprint_of(kind: &str, text: &str) -> Option<String> {
    if kind == BATPROJ {
        yaml_serde::from_str::<BatProject>(text)
            .ok()
            .map(|p| rules::project_fingerprint(&p))
    } else {
        yaml_serde::from_str::<AnnotationSet>(text)
            .ok()
            .map(|s| rules::annotation_fingerprint(&s))
    }
}

fn properties(kind: &str, key: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (drive::KIND_PROPERTY.to_string(), kind.to_string()),
        (rules::KEY_PROPERTY.to_string(), rules::drive_key(key)),
    ])
}

/// Upload the waiting text of `sync_key`, if it differs from the Drive copy.
async fn upload(gen: u64, sync_key: String) -> Result<(), DriveError> {
    if !alive(gen) {
        return Ok(());
    }
    if !with(|s| s.checked.contains(&sync_key)) {
        start_check(sync_key);
        return Ok(());
    }
    if with(|s| s.uploading.contains(&sync_key)) {
        // Uploaded again when the running upload ends.
        return Ok(());
    }
    let Some(pending) = with(|s| s.pending.get(&sync_key).cloned()) else {
        return Ok(());
    };
    let (kind, key) = split(&sync_key);
    let (kind, key) = (kind.to_string(), key.to_string());
    let Some(fp) = fingerprint_of(&kind, &pending.text) else {
        log::warn!("Google Drive: not uploading {sync_key}: the text does not parse");
        drop_pending(&sync_key, pending.seq);
        return Ok(());
    };
    let last = seen(&sync_key);
    if last.as_ref().is_some_and(|l| l.fp == fp) {
        drop_pending(&sync_key, pending.seq);
        return Ok(());
    }

    with(|s| s.uploading.insert(sync_key.clone()));
    publish();
    while with(|s| s.uploads_running >= MAX_PARALLEL_UPLOADS) {
        crate::web_util::sleep_ms(100).await;
    }
    if !alive(gen) {
        return Ok(());
    }
    with(|s| s.uploads_running += 1);
    let result = upload_inner(gen, &sync_key, &kind, &key, &pending, last).await;
    with(|s| {
        s.uploads_running = s.uploads_running.saturating_sub(1);
        s.uploading.remove(&sync_key);
    });
    if !alive(gen) {
        publish();
        return Ok(());
    }
    match result {
        Ok(Some(file)) => {
            set_seen(
                &sync_key,
                Some(Seen {
                    id: file.id,
                    rev: file.head_revision_id,
                    fp,
                }),
            );
            drop_pending(&sync_key, pending.seq);
            set_problem(None);
        }
        Ok(None) => {
            // The Drive copy changed since it was last seen: compare again.
            with(|s| s.checked.remove(&sync_key));
            start_check(sync_key.clone());
        }
        Err(e) => {
            publish();
            return Err(e);
        }
    }
    publish();
    // A newer save arrived during the upload.
    if with(|s| s.pending.contains_key(&sync_key) && s.checked.contains(&sync_key)) {
        schedule_upload(&sync_key, UPLOAD_DELAY_MS);
    }
    Ok(())
}

/// Write `pending` to Drive. `Ok(None)` when the Drive copy changed since
/// it was last seen, so it must not be overwritten.
async fn upload_inner(
    gen: u64,
    sync_key: &str,
    kind: &str,
    key: &str,
    pending: &Pending,
    last: Option<Seen>,
) -> Result<Option<DriveFile>, DriveError> {
    let props = properties(kind, key);
    // A Drive id is forgotten at sign-out; the fingerprint alone is no file.
    if let Some(last) = last.filter(|l| !l.id.is_empty()) {
        match drive::metadata(&last.id).await? {
            Some(now) => {
                if !rules::may_overwrite(now.head_revision_id.as_deref(), last.rev.as_deref()) {
                    return Ok(None);
                }
                let file = drive::update(&last.id, &pending.name, &props, &pending.text).await?;
                return Ok(Some(file));
            }
            None => {
                // Gone from Drive (deleted or trashed there): make a new one.
                set_seen(sync_key, None);
            }
        }
    }
    // No Drive file known. If one appeared since the comparison (another
    // device), compare again instead of making a second one.
    let existing = drive::files_with_key(kind, &rules::drive_key(key)).await?;
    if !alive(gen) {
        return Ok(None);
    }
    if !existing.is_empty() {
        return Ok(None);
    }
    let folder = folder_for(kind).await?;
    let file = drive::create(&folder, &pending.name, &props, &pending.text).await?;
    Ok(Some(file))
}

/// Forget the waiting text of `sync_key` if no newer save replaced it.
fn drop_pending(sync_key: &str, seq: u64) {
    with(|s| {
        if s.pending.get(sync_key).is_some_and(|p| p.seq == seq) {
            s.pending.remove(sync_key);
        }
    });
    publish();
}

// ── Comparing one file ──────────────────────────────────────────────────

/// Marks a key as being compared, and unmarks it when dropped.
struct Checking(String);

impl Checking {
    fn start(sync_key: &str) -> Option<Self> {
        with(|s| s.checking.insert(sync_key.to_string())).then(|| Checking(sync_key.to_string()))
    }
}

impl Drop for Checking {
    fn drop(&mut self) {
        with(|s| s.checking.remove(&self.0));
    }
}

/// The comparison of `sync_key` is done: uploads may go ahead.
fn mark_checked(sync_key: &str) {
    with(|s| s.checked.insert(sync_key.to_string()));
    publish();
    if with(|s| s.pending.contains_key(sync_key)) {
        schedule_upload(sync_key, UPLOAD_DELAY_MS);
    }
}

/// The Drive copy to compare: the newest file with the key, with its
/// content when it has to be read.
struct DriveCopy {
    file: DriveFile,
    fp: String,
    text: Option<String>,
}

/// Find the fingerprint of the newest of `files`: from memory if it is the
/// revision last seen, else by downloading it.
async fn drive_copy(
    sync_key: &str,
    files: &[DriveFile],
    fingerprint: impl Fn(&str) -> Option<String>,
) -> Result<Option<DriveCopy>, DriveError> {
    let Some(newest) = rules::newest(files, |f| f.modified_time.as_str()).cloned() else {
        return Ok(None);
    };
    let last = seen(sync_key);
    if let Some(last) = last.filter(|l| {
        l.id == newest.id && newest.head_revision_id.is_some() && l.rev == newest.head_revision_id
    }) {
        return Ok(Some(DriveCopy {
            file: newest,
            fp: last.fp,
            text: None,
        }));
    }
    let text = drive::download(&newest.id).await?;
    let Some(fp) = fingerprint(&text) else {
        log::warn!("Google Drive: {sync_key} on Drive does not parse; left alone");
        with(|s| s.blocked.insert(sync_key.to_string()));
        return Ok(None);
    };
    Ok(Some(DriveCopy {
        file: newest,
        fp,
        text: Some(text),
    }))
}

/// Drive text of a copy, downloading it if only its fingerprint was known.
async fn drive_text(copy: &DriveCopy) -> Result<String, DriveError> {
    match &copy.text {
        Some(t) => Ok(t.clone()),
        None => drive::download(&copy.file.id).await,
    }
}

fn seen_from(copy: &DriveCopy) -> Seen {
    Seen {
        id: copy.file.id.clone(),
        rev: copy.file.head_revision_id.clone(),
        fp: copy.fp.clone(),
    }
}

/// The key of the open file `file_id`, once its hash is known.
fn open_file_key(state: AppState, file_id: u64) -> Option<String> {
    if crate::file_identity::spot_hash_pending(file_id) {
        return None;
    }
    let idx = state.file_idx_for_id(file_id)?;
    state.library.files().with_untracked(|files| {
        let f = files.get(idx)?;
        if f.read_only {
            return None;
        }
        f.identity.as_ref().map(crate::opfs::opfs_key)
    })
}

/// The open file whose key is `key`, if any.
fn open_file_with_key(state: AppState, key: &str) -> Option<u64> {
    state.library.files().with_untracked(|files| {
        files.iter().find_map(|f| {
            let id = f.identity.as_ref()?;
            (crate::opfs::opfs_key(id) == key && !crate::file_identity::spot_hash_pending(f.id))
                .then_some(f.id)
        })
    })
}

/// The open file's annotation set as it would be saved now (file units,
/// no file path), or `None` if it has none.
fn open_file_set(state: AppState, file_id: u64) -> Option<AnnotationSet> {
    let idx = state.file_idx_for_id(file_id)?;
    let set = state
        .annotations
        .store()
        .with_untracked(|store| store.get(file_id).cloned())?;
    let mut set = crate::te_correction::to_file_units_at(state, idx, set);
    set.file_identity.file_path = None;
    Some(set)
}

fn yaml_of<T: Serialize>(value: &T) -> Option<String> {
    yaml_serde::to_string(value).ok()
}

/// Compare the annotation set under `key` with its Drive copy and act.
/// `files`: the Drive files with this key, if already listed.
async fn check_annotations(
    gen: u64,
    key: String,
    files: Option<Vec<DriveFile>>,
) -> Result<(), DriveError> {
    let Some(state) = app_state() else {
        return Ok(());
    };
    let sk = batm_key(&key);
    if !alive(gen) || with(|s| s.blocked.contains(&sk)) {
        return Ok(());
    }
    let Some(_checking) = Checking::start(&sk) else {
        return Ok(());
    };
    let files = match files {
        Some(f) => f,
        None => drive::files_with_key(BATM, &rules::drive_key(&key)).await?,
    };
    if !alive(gen) {
        return Ok(());
    }
    let parse_fp = |text: &str| {
        yaml_serde::from_str::<AnnotationSet>(text)
            .ok()
            .map(|s| rules::annotation_fingerprint(&s))
    };
    let drive = drive_copy(&sk, &files, parse_fp).await?;
    if !alive(gen) || with(|s| s.blocked.contains(&sk)) {
        return Ok(());
    }

    // The local copy: the open file's set, or the one in local storage.
    let open = open_file_with_key(state, &key);
    if let Some(file_id) = open {
        let read_only = state.file_idx_for_id(file_id).is_some_and(|idx| {
            state
                .library
                .files()
                .with_untracked(|f| f.get(idx).is_some_and(|f| f.read_only))
        });
        if read_only {
            mark_checked(&sk);
            return Ok(());
        }
    }
    let local: Option<AnnotationSet> = match open {
        Some(file_id) => open_file_set(state, file_id),
        None => match crate::opfs::opfs_load(&key).await {
            Ok(Some(text)) => yaml_serde::from_str(&text).ok(),
            _ => None,
        },
    };
    let local_fp = local.as_ref().map(rules::annotation_fingerprint);
    let base = seen(&sk).map(|s| s.fp);
    let action = rules::decide(
        Kind::Annotations,
        local
            .as_ref()
            .zip(local_fp.as_deref())
            .map(|(set, fp)| FileCopy {
                fingerprint: fp,
                modified_at: set.modified_at.as_deref(),
            }),
        drive.as_ref().map(|d| FileCopy {
            fingerprint: &d.fp,
            modified_at: None,
        }),
        base.as_deref(),
    );
    // `decide` needs the Drive copy's time only when both copies changed;
    // read it then.
    let action = match (&action, &drive, &local, &local_fp) {
        (Action::Upload | Action::Download, Some(d), Some(l), Some(lfp))
            if base.as_deref().is_some_and(|b| b != d.fp && b != lfp) =>
        {
            let text = drive_text(d).await?;
            let at = yaml_serde::from_str::<AnnotationSet>(&text)
                .ok()
                .and_then(|s| s.modified_at);
            rules::decide(
                Kind::Annotations,
                Some(FileCopy {
                    fingerprint: lfp,
                    modified_at: l.modified_at.as_deref(),
                }),
                Some(FileCopy {
                    fingerprint: &d.fp,
                    modified_at: at.as_deref(),
                }),
                base.as_deref(),
            )
        }
        _ => action,
    };
    if !alive(gen) {
        return Ok(());
    }

    match (action, drive) {
        (Action::Nothing, Some(d)) => set_seen(&sk, Some(seen_from(&d))),
        (Action::Nothing, None) => {}
        (Action::Upload, d) => {
            match &d {
                Some(d) => set_seen(&sk, Some(seen_from(d))),
                None => set_seen(&sk, None),
            }
            // Only a set saved under its own key goes up: a copy left under
            // an old name-and-size key stays local.
            let own_key = local
                .as_ref()
                .is_some_and(|l| crate::opfs::opfs_key(&l.file_identity) == key);
            if let (Some(l), true) = (&local, open.is_some() || own_key || d.is_some()) {
                if let Some(text) = yaml_of(l) {
                    let seq = next_save_seq();
                    let name = format!("{}.batm", l.file_identity.filename);
                    with(|s| {
                        s.pending
                            .entry(sk.clone())
                            .or_insert(Pending { name, text, seq });
                    });
                }
            }
        }
        (Action::Download | Action::Merge, Some(d)) => {
            let text = drive_text(&d).await?;
            if !alive(gen) {
                return Ok(());
            }
            let Ok(drive_set) = yaml_serde::from_str::<AnnotationSet>(&text) else {
                with(|s| s.blocked.insert(sk.clone()));
                return Ok(());
            };
            // A set saved for a different recording with the same name and
            // size: leave both alone.
            let identity = match open {
                Some(file_id) => state.file_idx_for_id(file_id).and_then(|idx| {
                    state
                        .library
                        .files()
                        .with_untracked(|f| f.get(idx).and_then(|f| f.identity.clone()))
                }),
                None => local.as_ref().map(|l| l.file_identity.clone()),
            };
            if identity.is_some_and(|id| {
                !crate::file_identity::saved_identity_agrees(&drive_set.file_identity, &id)
            }) {
                log::info!("Google Drive: {sk} on Drive belongs to another recording; left alone");
                with(|s| s.blocked.insert(sk.clone()));
                return Ok(());
            }
            // What was seen of Drive is recorded once the local copy holds
            // it; if the local write fails, the next comparison downloads
            // again instead of uploading the old local copy.
            match open {
                Some(file_id) => {
                    // The user may have changed the set while Drive was read.
                    let now_fp = open_file_set(state, file_id)
                        .as_ref()
                        .map(rules::annotation_fingerprint);
                    if now_fp != local_fp {
                        drop(_checking);
                        start_check(sk);
                        return Ok(());
                    }
                    if !crate::opfs::adopt_drive_copy(
                        state,
                        file_id,
                        drive_set,
                        action == Action::Merge,
                    ) {
                        with(|s| s.blocked.insert(sk.clone()));
                        return Ok(());
                    }
                    // Before the adopted set's own save reaches the upload
                    // queue (it waits for the local write).
                    set_seen(&sk, Some(seen_from(&d)));
                }
                None => {
                    let (out_text, out_fp) = if action == Action::Merge {
                        let merged = match &local {
                            Some(l) => rules::merge_annotation_sets(drive_set, l),
                            None => drive_set,
                        };
                        let fp = rules::annotation_fingerprint(&merged);
                        (yaml_of(&merged).unwrap_or(text), fp)
                    } else {
                        (text, d.fp.clone())
                    };
                    if let Err(e) = crate::opfs::opfs_save(&key, &out_text).await {
                        log::warn!("Google Drive: could not store {key} locally: {e}");
                        return Ok(());
                    }
                    set_seen(&sk, Some(seen_from(&d)));
                    if out_fp != d.fp {
                        let name = yaml_serde::from_str::<AnnotationSet>(&out_text)
                            .map(|s| format!("{}.batm", s.file_identity.filename))
                            .unwrap_or_else(|_| d.file.name.clone());
                        let seq = next_save_seq();
                        with(|s| {
                            s.pending.insert(
                                sk.clone(),
                                Pending {
                                    name,
                                    text: out_text,
                                    seq,
                                },
                            )
                        });
                    }
                }
            }
        }
        (Action::Download | Action::Merge, None) => {}
    }
    mark_checked(&sk);
    set_problem(None);
    publish();
    Ok(())
}

/// Compare the project `id` with its Drive copy and act.
async fn check_project(
    gen: u64,
    id: String,
    files: Option<Vec<DriveFile>>,
) -> Result<(), DriveError> {
    let Some(state) = app_state() else {
        return Ok(());
    };
    let sk = project_key(&id);
    if !alive(gen) || with(|s| s.blocked.contains(&sk)) {
        return Ok(());
    }
    if deleted_projects().contains(&id) {
        return Ok(());
    }
    let Some(_checking) = Checking::start(&sk) else {
        return Ok(());
    };
    let files = match files {
        Some(f) => f,
        None => drive::files_with_key(BATPROJ, &rules::drive_key(&id)).await?,
    };
    let parse_fp = |text: &str| {
        yaml_serde::from_str::<BatProject>(text)
            .ok()
            .map(|p| rules::project_fingerprint(&p))
    };
    let drive = drive_copy(&sk, &files, parse_fp).await?;
    if !alive(gen) {
        return Ok(());
    }
    let is_open = |state: AppState| {
        state
            .project
            .current()
            .with_untracked(|p| p.as_ref().is_some_and(|p| p.id == id))
    };
    let local: Option<BatProject> = if is_open(state) {
        state.project.current().get_untracked()
    } else {
        crate::project_store::load_project(&id).await.ok().flatten()
    };
    let local_fp = local.as_ref().map(rules::project_fingerprint);
    // Projects need the Drive copy's time for a first comparison too.
    let drive_text_now = match &drive {
        Some(d) if local_fp.as_deref().is_some_and(|l| l != d.fp) => Some(drive_text(d).await?),
        _ => None,
    };
    let drive_project: Option<BatProject> = drive_text_now
        .as_deref()
        .and_then(|t| yaml_serde::from_str(t).ok());
    let base = seen(&sk).map(|s| s.fp);
    let action = rules::decide(
        Kind::Project,
        local
            .as_ref()
            .zip(local_fp.as_deref())
            .map(|(p, fp)| FileCopy {
                fingerprint: fp,
                modified_at: p.modified_at.as_deref(),
            }),
        drive.as_ref().map(|d| FileCopy {
            fingerprint: &d.fp,
            modified_at: drive_project
                .as_ref()
                .and_then(|p| p.modified_at.as_deref()),
        }),
        base.as_deref(),
    );
    if !alive(gen) {
        return Ok(());
    }
    match (action, drive) {
        (Action::Nothing, Some(d)) => set_seen(&sk, Some(seen_from(&d))),
        (Action::Nothing, None) => {}
        (Action::Upload, d) => {
            match &d {
                Some(d) => set_seen(&sk, Some(seen_from(d))),
                None => set_seen(&sk, None),
            }
            if let Some(p) = &local {
                if let Some(text) = yaml_of(p) {
                    let seq = next_save_seq();
                    let name = project_file_name(p);
                    with(|s| {
                        s.pending
                            .entry(sk.clone())
                            .or_insert(Pending { name, text, seq });
                    });
                }
            }
        }
        (Action::Download | Action::Merge, Some(d)) => {
            let text = match drive_text_now {
                Some(t) => t,
                None => drive_text(&d).await?,
            };
            let Ok(project) = yaml_serde::from_str::<BatProject>(&text) else {
                with(|s| s.blocked.insert(sk.clone()));
                return Ok(());
            };
            if project.id != id {
                with(|s| s.blocked.insert(sk.clone()));
                return Ok(());
            }
            if let Err(e) = crate::project_store::write_project_local(&project).await {
                log::warn!("Google Drive: could not store project {id} locally: {e}");
                return Ok(());
            }
            set_seen(&sk, Some(seen_from(&d)));
            // The open project changes too. (If it had unsaved changes, the
            // Drive copy was the later one.)
            if is_open(state) {
                state.project.current().set(Some(project));
                state.project.dirty().set(false);
            }
            state
                .drive
                .projects_changed()
                .update(|n| *n = n.wrapping_add(1));
        }
        (Action::Download | Action::Merge, None) => {}
    }
    mark_checked(&sk);
    set_problem(None);
    publish();
    Ok(())
}

// ── Comparing everything ────────────────────────────────────────────────

/// Group Drive files by their key property.
fn by_key(files: Vec<DriveFile>) -> HashMap<String, Vec<DriveFile>> {
    let mut map: HashMap<String, Vec<DriveFile>> = HashMap::new();
    for f in files {
        if let Some(k) = f.property(rules::KEY_PROPERTY) {
            map.entry(k.to_string()).or_default().push(f);
        }
    }
    map
}

/// Compare every local and Drive file, in the background.
async fn reconcile(gen: u64) -> Result<(), DriveError> {
    if with(|s| s.reconciling) {
        return Ok(());
    }
    with(|s| s.reconciling = true);
    publish();
    let result = reconcile_inner(gen).await;
    if alive(gen) {
        with(|s| s.reconciling = false);
    }
    publish();
    result
}

async fn reconcile_inner(gen: u64) -> Result<(), DriveError> {
    let Some(state) = app_state() else {
        return Ok(());
    };
    trash_deleted_projects(gen).await?;

    // Annotation sets.
    let mut drive_sets = by_key(drive::files_of_kind(BATM).await?);
    if !alive(gen) {
        return Ok(());
    }
    let local_keys = crate::opfs::annotation_keys().await.unwrap_or_default();
    for key in local_keys {
        let files = drive_sets
            .remove(&rules::drive_key(&key))
            .unwrap_or_default();
        if with(|s| s.checked.contains(&batm_key(&key))) {
            continue;
        }
        check_annotations(gen, key, Some(files)).await?;
        if !alive(gen) {
            return Ok(());
        }
    }
    for (property, files) in drive_sets {
        let key = if property.starts_with("h_") {
            // A long key: work it out from the recording named in the set.
            let Some(newest) = rules::newest(&files, |f| f.modified_time.as_str()) else {
                continue;
            };
            let text = drive::download(&newest.id).await?;
            let Ok(set) = yaml_serde::from_str::<AnnotationSet>(&text) else {
                continue;
            };
            let candidates = [
                crate::opfs::opfs_key(&set.file_identity),
                crate::opfs::opfs_fallback_key(
                    &set.file_identity.filename,
                    set.file_identity.file_size,
                ),
            ];
            rules::local_annotation_key(&property, &candidates)
        } else {
            rules::local_annotation_key(&property, &[])
        };
        let Some(key) = key else {
            continue;
        };
        if with(|s| s.checked.contains(&batm_key(&key))) {
            continue;
        }
        check_annotations(gen, key, Some(files)).await?;
        if !alive(gen) {
            return Ok(());
        }
    }

    // Projects.
    let mut drive_projects = by_key(drive::files_of_kind(BATPROJ).await?);
    if !alive(gen) {
        return Ok(());
    }
    let mut local_ids = crate::project_store::project_ids()
        .await
        .unwrap_or_default();
    if let Some(open) = state
        .project
        .current()
        .with_untracked(|p| p.as_ref().map(|p| p.id.clone()))
    {
        if !local_ids.contains(&open) {
            local_ids.push(open);
        }
    }
    for id in local_ids {
        let files = drive_projects
            .remove(&rules::drive_key(&id))
            .unwrap_or_default();
        check_project(gen, id, Some(files)).await?;
        if !alive(gen) {
            return Ok(());
        }
    }
    for (property, files) in drive_projects {
        if !rules::local_key_is_safe(&format!("{property}.batproj"), ".batproj") {
            continue;
        }
        check_project(gen, property, Some(files)).await?;
        if !alive(gen) {
            return Ok(());
        }
    }

    // Changes saved while signed out, for keys not in local storage (rare:
    // the write failed) still wait; compare them too.
    let waiting: Vec<String> = with(|s| {
        s.pending
            .keys()
            .filter(|k| !s.checked.contains(*k))
            .cloned()
            .collect()
    });
    for sk in waiting {
        start_check(sk);
    }
    restart_waiting_uploads();
    set_problem(None);
    publish();
    Ok(())
}

/// Move the Drive copies of projects deleted on this device to the Drive
/// trash.
async fn trash_deleted_projects(gen: u64) -> Result<(), DriveError> {
    for id in deleted_projects() {
        let files = drive::files_with_key(BATPROJ, &rules::drive_key(&id)).await?;
        for f in files {
            drive::trash(&f.id).await?;
        }
        if !alive(gen) {
            return Ok(());
        }
        set_seen(&project_key(&id), None);
        let rest: Vec<String> = deleted_projects()
            .into_iter()
            .filter(|i| *i != id)
            .collect();
        set_deleted_projects(&rest);
    }
    Ok(())
}

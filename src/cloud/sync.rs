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
use super::rules::merge::{self, Conflict, MergeBase, Side};
use super::rules::{self, Action, DriveSide, FileCopy, Kind};
use super::store::{self, Store};
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

/// localStorage, before per-account caches: what this browser last saw of
/// each Drive file. Read once, by the migration (`migrate_legacy`).
const LEGACY_MEMORY_KEY: &str = "oversample_drive_seen";
/// localStorage, before per-account caches: projects deleted here whose
/// Drive copies still had to go to the Drive trash.
const LEGACY_DELETED_PROJECTS_KEY: &str = "oversample_drive_deleted_projects";
/// File in an account's folder that holds its `Memory`.
const MEMORY_FILE: &str = "sync.json";

/// What this device last saw of the Drive files of one account, kept in the
/// account's folder (`sync.json`).
#[derive(Clone, Default, Serialize, Deserialize)]
struct Memory {
    /// The account's email when last seen (shown only).
    #[serde(default)]
    email: String,
    /// By sync key (`batm:<key>` or `batproj:<id>`).
    #[serde(default)]
    files: BTreeMap<String, Seen>,
    /// Projects deleted here whose Drive copies still have to go to the
    /// Drive trash.
    #[serde(default)]
    deleted_projects: Vec<String>,
    /// The old per-browser record has been moved in (once per account).
    #[serde(default)]
    migrated: bool,
    /// Merges of copies that both changed, not yet seen by the user, by
    /// sync key (shown when the file is open).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    merges: BTreeMap<String, MergeNotice>,
}

/// What a merge of two changed copies did, for the notice shown with the
/// file (`components::annotation_storage`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MergeNotice {
    /// What the Drive copy's changes brought in.
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    /// Changes both copies made, and whose was kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<NoticeConflict>,
    /// No base was known: annotations deleted in one copy may be back.
    #[serde(default)]
    pub without_base: bool,
    /// This browser's copy from before the merge went to Drive first, so it
    /// is in the Drive file's version history.
    #[serde(default)]
    pub before_in_history: bool,
    /// When that copy reached Drive (ISO 8601), to find it in the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_at: Option<String>,
    /// The file's page in Google Drive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_link: Option<String>,
}

/// One change both copies made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoticeConflict {
    pub what: ConflictWhat,
    /// The annotation's name, or the program's tag.
    #[serde(default)]
    pub name: String,
    /// This browser's version was kept (else the Drive copy's).
    pub kept_here: bool,
    /// For a program run in both copies: how many annotations of the other
    /// run were removed.
    #[serde(default)]
    pub count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictWhat {
    /// Changed in both copies.
    BothChanged,
    /// Deleted in one copy and changed in the other: kept.
    ChangedAndDeleted,
    /// A program (call detection, a classifier, a Xeno-canto import) was run
    /// in both copies: one run was kept.
    RunTwice,
    LowestBatHz,
    NoiseProfile,
}

/// The old per-browser record.
#[derive(Default, Deserialize)]
struct LegacyMemory {
    #[serde(default)]
    account: String,
    #[serde(default)]
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
    /// Per key, the `save_seq` value when its last upload ended. A
    /// comparison of all files skips keys uploaded after its Drive listing
    /// was made, since that listing is out of date for them.
    uploaded_at: HashMap<String, u64>,
    /// Per key, this browser's copy from before a merge that dropped some of
    /// it: uploaded before the merge, so that it stays in the Drive file's
    /// version history.
    before_merge: HashMap<String, String>,
    /// Per key, the fingerprint of the base summary stored for it (so an
    /// unchanged one is not written again).
    base_fps: HashMap<String, String>,
    /// Keys whose Drive copy belongs to a different recording (same name and
    /// size). They are neither uploaded nor downloaded.
    blocked: HashSet<String>,
    memory: Memory,
    /// The signed-in account's store (its cache on this device), once known.
    account: Option<Store>,
    /// Keys a comparison found deleted on Drive and unchanged here. They are
    /// removed from the cache after the comparison of all files, if
    /// `rules::removals_allowed` says so.
    gone: Vec<String>,
    reconciling: bool,
    save_seq: u64,
    /// A folder lookup or creation is running (one at a time, so that
    /// parallel uploads do not each create the folders).
    folder_busy: bool,
    /// Uploads running now, across all keys.
    uploads_running: usize,
    /// Counts sign-in attempts; an older attempt's polling stops.
    sign_in_attempt: u64,
    /// Google sign-in is offered here. Until it is known (and on Android or
    /// a desktop build with no Google client) saves are not queued: a
    /// comparison at sign-in covers them.
    available: bool,
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

/// Whether Drive copies are made here. Off in the desktop app until the
/// account-home model lands: there, sign-in works and a read-only check of
/// the account's Drive runs (`desktop_check`), but nothing is compared,
/// uploaded, downloaded or created.
fn copies_enabled() -> bool {
    google_auth::provider() == google_auth::Provider::Web
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

/// The signed-in account's folder (where `sync.json` is).
fn account_dir() -> Option<String> {
    with(|s| s.account.as_ref().and_then(Store::account_dir))
}

/// Read an account's `Memory` from its folder.
async fn read_memory(account_dir: &str) -> Memory {
    store::read(account_dir, MEMORY_FILE)
        .await
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Change the account's `sync.json` under its lock: read it, apply `change`,
/// write it. Every tab writes this way, so two tabs never overwrite each
/// other's entries. The copy in memory gets the same change.
async fn update_memory(change: impl Fn(&mut Memory) + Clone + 'static) {
    with(|s| change(&mut s.memory));
    let Some(dir) = account_dir() else {
        return;
    };
    let lock = store::sync_lock(&dir);
    store::with_lock(&lock, move || {
        Box::pin(async move {
            let mut m = read_memory(&dir).await;
            change(&mut m);
            if let Ok(json) = serde_json::to_string(&m) {
                if let Err(e) = store::write(&dir, MEMORY_FILE, &json).await {
                    log::warn!("Google Drive: could not save sync.json: {e}");
                }
            }
        })
    })
    .await;
}

/// Read the latest record of one key from `sync.json` (another tab may have
/// changed it) into memory, and return it.
async fn refresh_seen(sync_key: &str) -> Option<Seen> {
    if let Some(dir) = account_dir() {
        let on_disk = read_memory(&dir).await;
        let v = on_disk.files.get(sync_key).cloned();
        with(|s| match &v {
            Some(v) => {
                s.memory.files.insert(sync_key.to_string(), v.clone());
            }
            None => {
                s.memory.files.remove(sync_key);
            }
        });
        return v;
    }
    seen(sync_key)
}

// ── Merge bases and notices ─────────────────────────────────────────────

/// Folder of the base summaries (`merge::MergeBase`), one file per
/// annotation key: what each key's Drive copy held when last synced.
fn bases_dir() -> Option<String> {
    account_dir().map(|d| format!("{d}/bases"))
}

/// Store `set` as the base of `sync_key` (an annotation key), if it is not
/// the one stored already.
fn note_base(sync_key: &str, set: &AnnotationSet) {
    let (kind, key) = split(sync_key);
    if kind != BATM {
        return;
    }
    let base = MergeBase::of(set);
    if with(|s| s.base_fps.get(sync_key) == Some(&base.fp)) {
        return;
    }
    let Some(dir) = bases_dir() else {
        return;
    };
    let Ok(json) = serde_json::to_string(&base) else {
        return;
    };
    with(|s| s.base_fps.insert(sync_key.to_string(), base.fp.clone()));
    let name = format!("{key}.json");
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = store::write(&dir, &name, &json).await {
            log::warn!("Google Drive: could not store the merge base of {name}: {e}");
        }
    });
}

/// The stored base of `sync_key`, if it is the one with fingerprint `fp`.
async fn read_base(sync_key: &str, fp: &str) -> Option<MergeBase> {
    let (_, key) = split(sync_key);
    let text = store::read(&bases_dir()?, &format!("{key}.json")).await?;
    serde_json::from_str::<MergeBase>(&text)
        .ok()
        .filter(|b| b.fp == fp)
}

/// The base of `sync_key` with fingerprint `fp`: stored here, or else the
/// revision of the Drive file last seen, read from Drive's version history
/// (Drive keeps old revisions of uploaded files only for a while).
async fn merge_base(
    sync_key: &str,
    fp: &str,
    drive: &DriveCopy,
) -> Result<Option<MergeBase>, DriveError> {
    if let Some(b) = read_base(sync_key, fp).await {
        return Ok(Some(b));
    }
    let Some(last) = seen(sync_key) else {
        return Ok(None);
    };
    let Some(rev) = last.rev.filter(|_| last.id == drive.file.id) else {
        return Ok(None);
    };
    let Some(text) = drive::revision(&last.id, &rev).await? else {
        return Ok(None);
    };
    Ok(yaml_serde::from_str::<AnnotationSet>(&text)
        .ok()
        .map(|set| MergeBase::of(&set))
        .filter(|b| b.fp == fp))
}

/// Keep `notice` for `sync_key` until the user dismisses it.
fn add_merge_notice(sync_key: &str, notice: MergeNotice) {
    let sk = sync_key.to_string();
    with(|s| s.memory.merges.insert(sk.clone(), notice.clone()));
    publish();
    wasm_bindgen_futures::spawn_local(async move {
        update_memory(move |m| {
            m.merges.insert(sk.clone(), notice.clone());
        })
        .await;
    });
}

/// Note in the notice of `sync_key` that this browser's copy from before
/// the merge reached Drive.
fn note_before_in_history(sync_key: &str) {
    let sk = sync_key.to_string();
    if !with(|s| s.memory.merges.contains_key(&sk)) {
        return;
    }
    let at = crate::annotations::now_iso8601();
    let at2 = at.clone();
    with(|s| {
        if let Some(n) = s.memory.merges.get_mut(&sk) {
            n.before_in_history = true;
            n.before_at = Some(at2);
        }
    });
    publish();
    wasm_bindgen_futures::spawn_local(async move {
        update_memory(move |m| {
            if let Some(n) = m.merges.get_mut(&sk) {
                n.before_in_history = true;
                n.before_at = Some(at.clone());
            }
        })
        .await;
    });
}

/// The merge notice of the annotation set under `key`, if any (reactive).
pub fn merge_notice(key: &str) -> Option<MergeNotice> {
    if let Some(v) = with(|s| s.version.clone()) {
        v.track();
    }
    with(|s| s.memory.merges.get(&batm_key(key)).cloned())
}

/// The user has read the merge notice of `key`.
pub fn dismiss_merge_notice(key: &str) {
    let sk = batm_key(key);
    with(|s| s.memory.merges.remove(&sk));
    publish();
    wasm_bindgen_futures::spawn_local(async move {
        update_memory(move |m| {
            m.merges.remove(&sk);
        })
        .await;
    });
}

/// The notice for a three-way merge. `a` is this browser's copy, `b` the
/// Drive copy.
fn notice_of(report: &merge::MergeReport, drive_link: Option<String>) -> MergeNotice {
    let conflicts = report
        .conflicts
        .iter()
        .map(|c| match c {
            Conflict::BothChanged { label, kept, .. } => NoticeConflict {
                what: ConflictWhat::BothChanged,
                name: label.clone(),
                kept_here: *kept == Side::A,
                count: 0,
            },
            Conflict::ChangedAndDeleted {
                label, deleted_on, ..
            } => NoticeConflict {
                what: ConflictWhat::ChangedAndDeleted,
                name: label.clone(),
                kept_here: *deleted_on == Side::B,
                count: 0,
            },
            Conflict::GeneratedTwice { tag, kept, dropped } => NoticeConflict {
                what: ConflictWhat::RunTwice,
                name: tag.clone(),
                kept_here: *kept == Side::A,
                count: *dropped,
            },
            Conflict::LowestBatHz { kept } => NoticeConflict {
                what: ConflictWhat::LowestBatHz,
                name: String::new(),
                kept_here: *kept == Side::A,
                count: 0,
            },
            Conflict::NoiseProfile { kept } => NoticeConflict {
                what: ConflictWhat::NoiseProfile,
                name: String::new(),
                kept_here: *kept == Side::A,
                count: 0,
            },
        })
        .collect();
    // Changes the Drive copy won in a conflict are listed as conflicts, not
    // counted again.
    let won_by_drive = report
        .conflicts
        .iter()
        .filter(|c| match c {
            Conflict::BothChanged { kept, .. } => *kept == Side::B,
            Conflict::ChangedAndDeleted { deleted_on, .. } => *deleted_on == Side::A,
            _ => false,
        })
        .count();
    MergeNotice {
        added: report.from_b.added,
        removed: report.from_b.removed,
        changed: report.from_b.changed.saturating_sub(won_by_drive),
        conflicts,
        without_base: false,
        before_in_history: false,
        before_at: None,
        drive_link,
    }
}

/// Both copies of an annotation set, merged.
struct Merged {
    set: AnnotationSet,
    notice: Option<MergeNotice>,
    /// This browser's copy before the merge, as uploaded to Drive, when the
    /// merge dropped some of it.
    before: Option<String>,
}

/// Merge this browser's copy (`local`) with the Drive copy. With the base
/// (`base_fp`) known, annotation by annotation; without it, the Drive copy
/// plus the local annotations it lacks (deleted annotations can come back,
/// and the user is told when both copies brought something).
async fn merge_copies(
    sync_key: &str,
    base_fp: Option<&str>,
    drive: &DriveCopy,
    local: &AnnotationSet,
    drive_set: AnnotationSet,
) -> Result<Merged, DriveError> {
    let link = drive.file.web_view_link.clone();
    let base = match base_fp {
        Some(fp) => merge_base(sync_key, fp, drive).await?,
        None => None,
    };
    if let Some(base) = base {
        let (set, report) = merge::merge3(&base, local, &drive_set);
        let before = if report.dropped_from(Side::A) {
            rules::drive_text(local)
        } else {
            None
        };
        return Ok(Merged {
            set,
            notice: Some(notice_of(&report, link)),
            before,
        });
    }
    let ids = |s: &AnnotationSet| -> HashSet<String> {
        s.annotations
            .iter()
            .map(|a| a.id.as_str().to_string())
            .collect()
    };
    let (in_local, in_drive) = (ids(local), ids(&drive_set));
    let from_drive = in_drive.difference(&in_local).count();
    let from_here = in_local.difference(&in_drive).count();
    let set = rules::merge_annotation_sets(drive_set, local);
    let notice = (base_fp.is_some() || (from_drive > 0 && from_here > 0)).then(|| MergeNotice {
        added: from_drive,
        without_base: true,
        drive_link: link,
        ..MergeNotice::default()
    });
    Ok(Merged {
        set,
        notice,
        before: None,
    })
}

fn seen(sync_key: &str) -> Option<Seen> {
    with(|s| s.memory.files.get(sync_key).cloned())
}

/// Record what was seen of a key's Drive file (or forget it).
fn set_seen(sync_key: &str, value: Option<Seen>) {
    let sk = sync_key.to_string();
    let gen = generation();
    with(|s| match &value {
        Some(v) => {
            s.memory.files.insert(sk.clone(), v.clone());
        }
        None => {
            s.memory.files.remove(&sk);
        }
    });
    wasm_bindgen_futures::spawn_local(async move {
        if with(|s| s.generation) != gen {
            return;
        }
        update_memory(move |m| match &value {
            Some(v) => {
                m.files.insert(sk.clone(), v.clone());
            }
            None => {
                m.files.remove(&sk);
            }
        })
        .await;
    });
}

fn deleted_projects() -> Vec<String> {
    with(|s| s.memory.deleted_projects.clone())
}

async fn add_deleted_project(id: &str) {
    let id = id.to_string();
    update_memory(move |m| {
        if !m.deleted_projects.contains(&id) {
            m.deleted_projects.push(id.clone());
        }
    })
    .await;
}

async fn remove_deleted_project(id: &str) {
    let id = id.to_string();
    update_memory(move |m| m.deleted_projects.retain(|i| *i != id)).await;
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

/// Whether uploads wait: Drive no longer shows the "Oversample" folder
/// (`reconcile_inner`), and an upload would make a new one. Nothing is
/// created on Drive until a comparison of all files passes again.
fn uploads_held() -> bool {
    app_state().is_some_and(|state| {
        matches!(
            state.drive.problem().get_untracked(),
            Some(DriveProblem::FolderMissing)
        )
    })
}

/// Show `problem` in the Drive group, or none. Clearing leaves the problems
/// found by a comparison of all files (`FolderMissing`, `FilesMissing`) in
/// place: only a comparison that passes, or a new session, ends them
/// (`clear_problem`).
fn set_problem(problem: Option<DriveProblem>) {
    let from_comparison = app_state().is_some_and(|state| {
        matches!(
            state.drive.problem().get_untracked(),
            Some(DriveProblem::FolderMissing | DriveProblem::FilesMissing(_))
        )
    });
    if problem.is_none() && from_comparison {
        return;
    }
    clear_or_set_problem(problem);
}

/// End any problem, a held state too.
fn clear_problem() {
    clear_or_set_problem(None);
}

fn clear_or_set_problem(problem: Option<DriveProblem>) {
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

/// Start the feature: ask whether Google sign-in is offered (the server on
/// the web; the app's own Rust side on the desktop, which says no on
/// Android and in builds with no client id) and whether this device is
/// signed in.
pub fn init(state: AppState) {
    let provider = if state.is_tauri {
        google_auth::Provider::Desktop
    } else {
        google_auth::Provider::Web
    };
    google_auth::set_provider(provider);
    state
        .drive
        .desktop()
        .set(provider == google_auth::Provider::Desktop);
    with(|s| {
        s.state = Some(state);
        s.version = Some(ArcRwSignal::new(0));
    });
    add_window_listeners();
    wasm_bindgen_futures::spawn_local(async move {
        match google_auth::status().await {
            None => state.drive.available().set(false),
            Some(status) => {
                with(|s| s.available = copies_enabled());
                state.drive.available().set(true);
                state.drive.persistent().set(status.persistent);
                if status.signed_in {
                    start_session().await;
                }
            }
        }
    });
}

/// Open the Google sign-in window (web: a pop-up; desktop: the system
/// browser). Call from a click handler (browsers block pop-up windows opened
/// later).
pub fn sign_in(state: AppState) {
    if google_auth::provider() == google_auth::Provider::Desktop {
        desktop_sign_in(state);
        return;
    }
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
            if google_auth::status().await.is_some_and(|s| s.signed_in) {
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

/// Desktop: the Rust side opens the system browser and waits for Google's
/// redirect (up to 5 minutes, or until Cancel).
fn desktop_sign_in(state: AppState) {
    use crate::state::DriveSignInProblem;
    if state.drive.signing_in().get_untracked() {
        return;
    }
    state.drive.sign_in_problem().set(None);
    state.drive.signing_in().set(true);
    wasm_bindgen_futures::spawn_local(async move {
        let result = google_auth::desktop_sign_in().await;
        state.drive.signing_in().set(false);
        match result {
            Ok(status) => {
                state.drive.persistent().set(status.persistent);
                start_session().await;
            }
            Err(end) => {
                let problem = match end {
                    google_auth::SignInEnd::Cancelled => None,
                    google_auth::SignInEnd::Denied => Some(DriveSignInProblem::Denied),
                    google_auth::SignInEnd::TimedOut => Some(DriveSignInProblem::TimedOut),
                    google_auth::SignInEnd::Failed => Some(DriveSignInProblem::Failed),
                };
                state.drive.sign_in_problem().set(problem);
            }
        }
    });
}

/// Desktop: stop a sign-in that waits for the browser.
pub fn cancel_sign_in() {
    google_auth::desktop_cancel_sign_in();
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
    let (email, account_id) = match drive::account().await {
        Ok(a) => a,
        Err(e) => {
            on_error(gen, e);
            return;
        }
    };
    if !alive(gen) {
        return;
    }
    if !copies_enabled() {
        state.drive.email().set(Some(email));
        desktop_check(gen).await;
        return;
    }
    if !store::account_id_is_safe(&account_id) {
        log::warn!("Google Drive: the account id is not usable as a folder name");
        set_problem(Some(DriveProblem::Refused));
        return;
    }
    // This account's cache becomes the home of its files on this device.
    let account = Store::Account(account_id);
    let Some(dir) = account.account_dir() else {
        return;
    };
    let memory = read_memory(&dir).await;
    with(|s| {
        s.account = Some(account.clone());
        s.memory = memory;
    });
    if !with(|s| s.memory.migrated) {
        migrate_legacy(&account, &email).await;
    }
    if !alive(gen) {
        return;
    }
    store::set_active(account.clone());
    // Open files whose set is not in the device store now belong to the
    // account: reload them from its cache.
    switch_open_files(state);
    state.drive.email().set(Some(email));
    set_problem(None);
    if let Err(e) = reconcile(gen).await {
        on_error(gen, e);
    }
}

/// Move what the old per-browser record says was synced with this account
/// into the account's cache (once per account, `Memory::migrated`).
///
/// Only when the record was made by the same account (by email; the old
/// record kept no id). For each such file in the device store: copy it into
/// the cache, read the copy back, and only if it is the same, delete it from
/// the device store. An interrupted run finds both copies the same next time
/// and finishes. The old record's fingerprints left out the file identity,
/// so each key keeps its Drive id and revision but an unknown fingerprint
/// (""): the first comparison reads Drive's copy, which is the synced one
/// while its revision is unchanged, and takes its fingerprint as the base.
async fn migrate_legacy(account: &Store, email: &str) {
    let legacy: LegacyMemory = crate::settings::get_raw(LEGACY_MEMORY_KEY)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let keys = rules::keys_to_migrate(
        &legacy.account,
        email,
        legacy.files.keys().map(String::as_str),
    );
    for sk in &keys {
        let (kind, key) = split(sk);
        let (src_dir, dst_dir, name) = if kind == BATPROJ {
            (
                Store::Device.projects_dir(),
                account.projects_dir(),
                format!("{key}.batproj"),
            )
        } else {
            (
                Store::Device.annotations_dir(),
                account.annotations_dir(),
                key.to_string(),
            )
        };
        let moved = move_file(&src_dir, &dst_dir, &name).await;
        if moved {
            // An open file whose set moved now saves to the account's cache
            // (the content is the same, so it is not reloaded).
            if kind == BATM {
                if let Some(state) = app_state() {
                    if let Some(file_id) = open_file_with_key(state, key) {
                        store::set_home(file_id, account.clone());
                    }
                }
            } else {
                store::set_project_home(key, account.clone());
            }
        }
        if let Some(old) = legacy.files.get(sk) {
            let entry = Seen {
                id: old.id.clone(),
                rev: old.rev.clone(),
                fp: String::new(),
            };
            let sk2 = sk.clone();
            update_memory(move |m| {
                m.files.entry(sk2.clone()).or_insert(entry.clone());
            })
            .await;
        }
    }
    let legacy_deleted: Vec<String> = crate::settings::get_raw(LEGACY_DELETED_PROJECTS_KEY)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let same_account = !keys.is_empty() || legacy.account.eq_ignore_ascii_case(email);
    let email = email.to_string();
    update_memory(move |m| {
        if same_account {
            for id in &legacy_deleted {
                if !m.deleted_projects.contains(id) {
                    m.deleted_projects.push(id.clone());
                }
            }
        }
        m.email = email.clone();
        m.migrated = true;
    })
    .await;
    if same_account {
        crate::settings::remove_raw(LEGACY_MEMORY_KEY);
        crate::settings::remove_raw(LEGACY_DELETED_PROJECTS_KEY);
    }
}

/// Copy `name` from `src_dir` to `dst_dir`, check the copy, then delete it
/// from `src_dir`. Returns whether it moved. Under the file's lock in the
/// source folder (and then the destination's), so no save lands in between.
async fn move_file(src_dir: &str, dst_dir: &str, name: &str) -> bool {
    let (src, dst, name) = (src_dir.to_string(), dst_dir.to_string(), name.to_string());
    let lock = store::file_lock(&src, &name);
    store::with_lock(&lock, move || {
        Box::pin(async move {
            let Some(text) = store::read(&src, &name).await else {
                return false;
            };
            let dst_lock = store::file_lock(&dst, &name);
            let (dst2, name2, text2) = (dst.clone(), name.clone(), text.clone());
            let copied = store::with_lock(&dst_lock, move || {
                Box::pin(async move {
                    match store::read(&dst2, &name2).await {
                        // A newer copy is there already (another tab moved it).
                        Some(existing) if existing != text2 => true,
                        _ => {
                            store::write(&dst2, &name2, &text2).await.is_ok()
                                && store::read(&dst2, &name2).await.as_deref()
                                    == Some(text2.as_str())
                        }
                    }
                })
            })
            .await
            .unwrap_or(false);
            if !copied {
                return false;
            }
            store::bump_epoch(&src, &name);
            store::remove(&src, &name).await.is_ok()
        })
    })
    .await
    .unwrap_or(false)
}

/// After the active store changed: each open file's home is worked out
/// again (a set in the device store stays there; any other file follows the
/// active store), and a file whose home changes is reloaded from it (saves
/// held, undo cleared).
fn switch_open_files(state: AppState) {
    let open: Vec<u64> = state
        .library
        .files()
        .with_untracked(|files| files.iter().map(|f| f.id).collect());
    wasm_bindgen_futures::spawn_local(async move {
        for file_id in open {
            let Some(key) = open_file_key(state, file_id) else {
                continue;
            };
            let in_device = store::read(&Store::Device.annotations_dir(), &key)
                .await
                .is_some();
            let wanted = if in_device {
                Store::Device
            } else {
                store::active()
            };
            if store::home_of(file_id) != wanted {
                crate::opfs::reload_annotations(state, file_id);
            }
        }
    });
}

/// Desktop: look, read-only, for the "Oversample" folder and the app's
/// files that this desktop client can see, and show the result. Creates and
/// changes nothing.
async fn desktop_check(gen: u64) {
    use crate::state::DesktopDriveCheck;
    let Some(state) = app_state() else {
        return;
    };
    state
        .drive
        .desktop_check()
        .set(Some(DesktopDriveCheck::Checking));
    let result = async {
        let folders = drive::find_folders("root").await?;
        if folders.is_empty() {
            return Ok::<_, DriveError>(DesktopDriveCheck::NotFound);
        }
        let annotations = drive::files_of_kind(BATM).await?.len();
        let projects = drive::files_of_kind(BATPROJ).await?.len();
        Ok(DesktopDriveCheck::Found {
            annotations,
            projects,
        })
    }
    .await;
    if !alive(gen) {
        return;
    }
    match result {
        Ok(check) => state.drive.desktop_check().set(Some(check)),
        Err(DriveError::SignedOut) => end_session(),
        Err(e) => {
            log::warn!("Google Drive check: {e:?}");
            state
                .drive
                .desktop_check()
                .set(Some(DesktopDriveCheck::Failed));
        }
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
        s.uploaded_at.clear();
        s.base_fps.clear();
        s.blocked.clear();
        s.reconciling = false;
        s.folder_busy = false;
        s.uploads_running = 0;
        s.gone.clear();
        // What was seen of the account's Drive stays in its folder
        // (`sync.json`) for its next sign-in.
        s.memory = Memory::default();
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

/// Sign out at the user's request. The account's files stop being the
/// active store: open files that belong to it are reloaded from the device
/// store (where they usually have no set, so they show none). Nothing is
/// deleted. A sign-out the user did not ask for (`end_session`) keeps the
/// account's cache as the active store, so open files keep their sets and
/// edits are kept for the next sign-in.
pub fn sign_out(state: AppState) {
    clear_session();
    let account = with(|s| s.account.take());
    store::set_active(Store::Device);
    if let Some(account) = account {
        for file_id in store::files_with_home(&account) {
            crate::opfs::reload_annotations(state, file_id);
        }
    }
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
    if !copies_enabled() {
        let gen = generation();
        wasm_bindgen_futures::spawn_local(desktop_check(gen));
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
            if google_auth::status().await.is_some_and(|s| s.signed_in) && !with(|s| s.signed_in) {
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
    if !copies_enabled() || !state.drive.signed_in().get() {
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
                // Files missing on Drive do not stop this one's upload.
                Some(p) if !matches!(p, DriveProblem::FilesMissing(_)) => {
                    FileDriveState::Problem(p)
                }
                _ if s.checked.contains(&sk) => FileDriveState::Waiting,
                _ => FileDriveState::Checking,
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
    if !with(|s| s.available) {
        return;
    }
    queue(batm_key(key), format!("{audio_name}.batm"), text, seq);
}

/// A project was written to local storage.
pub fn project_saved(project: &BatProject) {
    if !with(|s| s.available) {
        return;
    }
    let Some(text) = rules::project_drive_text(project) else {
        return;
    };
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
    if !with(|s| s.available) {
        return;
    }
    let sk = project_key(id);
    with(|s| {
        s.pending.remove(&sk);
        s.checked.remove(&sk);
    });
    publish();
    let id = id.to_string();
    let gen = generation();
    spawn(async move {
        add_deleted_project(&id).await;
        if alive(gen) {
            trash_deleted_projects(gen).await?;
        }
        Ok(())
    });
}

/// A file's saved annotations have been read from local storage (or there
/// were none): compare with the Drive copy.
pub fn annotations_loaded(state: AppState, file_id: u64) {
    if !copies_enabled() || !with(|s| s.signed_in) {
        return;
    }
    // A file shown from the device store is not the account's.
    if with(|s| s.account.clone()).is_none_or(|a| store::home_of(file_id) != a) {
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
            .map(|s| rules::content_fingerprint(&s))
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
    if !alive(gen) || uploads_held() {
        // Held: the change waits, and goes up after the next comparison of
        // all files that passes.
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
    let mut last = seen(&sync_key);
    if last.as_ref().is_some_and(|l| l.fp == fp) {
        drop_pending(&sync_key, pending.seq);
        with(|s| s.before_merge.remove(&sync_key));
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
    // A merge dropped some of this browser's copy: that copy goes up first,
    // so it stays in the Drive file's version history. If the upload after
    // it fails, the base is that copy, so the next comparison uploads the
    // merge (only this browser changed since).
    let mut result = Ok(None);
    let mut before_failed = false;
    if let Some(before) = with(|s| s.before_merge.get(&sync_key).cloned()) {
        let p = Pending {
            name: pending.name.clone(),
            text: before.clone(),
            seq: 0,
        };
        match upload_inner(gen, &sync_key, &kind, &key, &p, last.clone()).await {
            Ok(Some(file)) => {
                let before_set = yaml_serde::from_str::<AnnotationSet>(&before).ok();
                let entry = Seen {
                    id: file.id,
                    rev: file.head_revision_id,
                    fp: fingerprint_of(&kind, &before).unwrap_or_default(),
                };
                set_seen(&sync_key, Some(entry.clone()));
                if let Some(set) = &before_set {
                    note_base(&sync_key, set);
                }
                with(|s| s.before_merge.remove(&sync_key));
                note_before_in_history(&sync_key);
                last = Some(entry);
            }
            other => {
                result = other;
                before_failed = true;
            }
        }
    }
    if !before_failed {
        result = upload_inner(gen, &sync_key, &kind, &key, &pending, last).await;
    }
    with(|s| {
        s.uploads_running = s.uploads_running.saturating_sub(1);
        s.uploading.remove(&sync_key);
    });
    let done = next_save_seq();
    with(|s| s.uploaded_at.insert(sync_key.clone(), done));
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
            if kind == BATM {
                if let Ok(set) = yaml_serde::from_str::<AnnotationSet>(&pending.text) {
                    note_base(&sync_key, &set);
                }
            }
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

// ── Annotation sets on this device ──────────────────────────────────────

/// Where annotation sets are kept on this device: the browser's private
/// folder (OPFS), or on the desktop the app's own annotations folder
/// (Tauri commands). Projects are in OPFS on both.
pub(crate) mod local {
    use super::google_auth::{self, Provider};

    fn desktop() -> bool {
        google_auth::provider() == Provider::Desktop
    }

    /// The signed-in account's folder of annotation sets (browser).
    fn account_annotations() -> Option<String> {
        super::with(|s| s.account.as_ref().map(|a| a.annotations_dir()))
    }

    /// Keys of every annotation set of the signed-in account on this device
    /// (its cache; on the desktop, the app's annotations folder).
    pub async fn keys() -> Vec<String> {
        if !desktop() {
            return match account_annotations() {
                Some(dir) => super::store::keys(&dir, ".batm").await,
                None => Vec::new(),
            };
        }
        crate::tauri_bridge::tauri_invoke_typed_no_args::<Vec<String>>("list_central_annotations")
            .await
            .unwrap_or_default()
    }

    pub async fn read(key: &str) -> Option<String> {
        if desktop() {
            return crate::opfs::tauri_load_central(key).await.ok().flatten();
        }
        super::store::read(&account_annotations()?, key).await
    }

    /// Write a set into the account's cache only if the copy there still
    /// has the content fingerprint `expected` (`None`: no copy), under the
    /// file's lock. `Ok(false)`: it changed since, nothing was written.
    pub async fn write_if_unchanged(
        key: &str,
        expected: Option<String>,
        text: &str,
    ) -> Result<bool, String> {
        if desktop() {
            return crate::opfs::tauri_save_central(key, text)
                .await
                .map(|_| true);
        }
        let dir = account_annotations().ok_or("not signed in")?;
        let lock = super::store::file_lock(&dir, key);
        let (key, text) = (key.to_string(), text.to_string());
        super::store::with_lock(&lock, move || {
            Box::pin(async move {
                let now = super::store::read(&dir, &key)
                    .await
                    .and_then(|t| {
                        yaml_serde::from_str::<crate::annotations::AnnotationSet>(&t).ok()
                    })
                    .map(|s| super::rules::content_fingerprint(&s));
                if now != expected {
                    return Ok(false);
                }
                super::store::write(&dir, &key, &text).await.map(|_| true)
            })
        })
        .await
        .unwrap_or(Err("lock".into()))
    }

    /// localStorage: keys of sets downloaded into the central folder while
    /// the recording's place on this computer was unknown.
    const UNPLACED_KEY: &str = "oversample_drive_unplaced";

    fn unplaced() -> Vec<String> {
        crate::settings::get_raw(UNPLACED_KEY)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn set_unplaced(keys: &[String]) {
        if keys.is_empty() {
            crate::settings::remove_raw(UNPLACED_KEY);
        } else if let Ok(json) = serde_json::to_string(keys) {
            crate::settings::set_raw(UNPLACED_KEY, &json);
        }
    }

    /// Desktop: note that `key` was downloaded with no known recording
    /// path, so a .batm beside the recording (found at its next open) was
    /// not rewritten and may hold an older set with a later time.
    pub fn mark_unplaced(key: &str) {
        if !desktop() {
            return;
        }
        let mut keys = unplaced();
        if !keys.iter().any(|k| k == key) {
            keys.push(key.to_string());
            set_unplaced(&keys);
        }
    }

    /// Whether `key` was marked by `mark_unplaced`; the mark is removed.
    pub fn take_unplaced(key: &str) -> bool {
        let keys = unplaced();
        if !keys.iter().any(|k| k == key) {
            return false;
        }
        let rest: Vec<String> = keys.into_iter().filter(|k| k != key).collect();
        set_unplaced(&rest);
        true
    }

    /// Desktop: rewrite the .batm beside the recording at `audio_path`, if
    /// there is one. (A new one is not made: that is the user's choice,
    /// with "Save .batm".)
    pub async fn rewrite_beside_recording(audio_path: &str, text: &str) {
        if !desktop() {
            return;
        }
        if let Ok(Some(_)) = crate::opfs::tauri_load_sidecar(audio_path).await {
            if let Err(e) = crate::opfs::tauri_save_sidecar(audio_path, text).await {
                log::warn!("Google Drive: could not update the .batm beside the recording: {e}");
            }
        }
    }
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
        !l.fp.is_empty()
            && l.id == newest.id
            && newest.head_revision_id.is_some()
            && l.rev == newest.head_revision_id
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

/// The open file with key `key` whose home is the signed-in account (a file
/// shown from the device store is not the account's).
fn open_account_file(state: AppState, key: &str) -> Option<u64> {
    let account = with(|s| s.account.clone())?;
    open_file_with_key(state, key).filter(|id| store::home_of(*id) == account)
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
    let (files, gone) = confirm_missing(&sk, files).await?;
    let parse_fp = |text: &str| {
        yaml_serde::from_str::<AnnotationSet>(text)
            .ok()
            .map(|s| rules::content_fingerprint(&s))
    };
    let drive = drive_copy(&sk, &files, parse_fp).await?;
    if !alive(gen) || with(|s| s.blocked.contains(&sk)) {
        return Ok(());
    }

    // The local copy: the open file's set, or the one in the account's
    // cache on this device.
    let open = open_account_file(state, &key);
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
        None => match local::read(&key).await {
            Some(text) => yaml_serde::from_str(&text).ok(),
            None => None,
        },
    };
    let local_fp = local.as_ref().map(rules::content_fingerprint);
    let base = base_for(&sk, drive.as_ref()).await;
    let side = match (&drive, gone) {
        (_, true) => DriveSide::Gone,
        (Some(d), _) => DriveSide::Present(FileCopy {
            fingerprint: &d.fp,
            modified_at: None,
        }),
        (None, false) => DriveSide::Absent,
    };
    if let (Action::RemoveLocal, true) = (
        rules::decide_with(
            Kind::Annotations,
            local
                .as_ref()
                .zip(local_fp.as_deref())
                .map(|(set, fp)| FileCopy {
                    fingerprint: fp,
                    modified_at: set.modified_at.as_deref(),
                }),
            side,
            base.as_deref(),
        ),
        gone,
    ) {
        note_gone(&sk);
        return Ok(());
    }
    let action = rules::decide_with(
        Kind::Annotations,
        local
            .as_ref()
            .zip(local_fp.as_deref())
            .map(|(set, fp)| FileCopy {
                fingerprint: fp,
                modified_at: set.modified_at.as_deref(),
            }),
        side,
        base.as_deref(),
    );
    if !alive(gen) {
        return Ok(());
    }

    match (action, drive) {
        (Action::Nothing, Some(d)) => {
            set_seen(&sk, Some(seen_from(&d)));
            if let Some(l) = &local {
                note_base(&sk, l);
            }
        }
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
                if let Some(text) = rules::drive_text(l) {
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
            // Both changed: merge them (three-way when the base is known).
            let merged = match (action, &local) {
                (Action::Merge, Some(l)) => {
                    Some(merge_copies(&sk, base.as_deref(), &d, l, drive_set.clone()).await?)
                }
                _ => None,
            };
            if !alive(gen) {
                return Ok(());
            }
            let base_set = drive_set.clone();
            let (out, notice, before) = match merged {
                Some(m) => (m.set, m.notice, m.before),
                None => (drive_set, None, None),
            };
            let out_fp = rules::content_fingerprint(&out);
            // What was seen of Drive is recorded once the local copy holds
            // it; if the local write fails, the next comparison downloads
            // again instead of uploading the old local copy.
            match open {
                Some(file_id) => {
                    // The user may have changed the set while Drive was read.
                    let now_fp = open_file_set(state, file_id)
                        .as_ref()
                        .map(rules::content_fingerprint);
                    if now_fp != local_fp {
                        drop(_checking);
                        start_check(sk);
                        return Ok(());
                    }
                    // Before the adopted set's own save queues its upload.
                    if let Some(b) = before {
                        with(|s| s.before_merge.insert(sk.clone(), b));
                    }
                    if !crate::opfs::adopt_drive_copy(state, file_id, out, false) {
                        with(|s| {
                            s.before_merge.remove(&sk);
                            s.blocked.insert(sk.clone());
                        });
                        return Ok(());
                    }
                    set_seen(&sk, Some(seen_from(&d)));
                    note_base(&sk, &base_set);
                }
                None => {
                    // The copy on this device keeps the recording's path (the
                    // desktop app's central copies hold it); Drive never
                    // gets it.
                    let mut on_device = out.clone();
                    on_device.file_identity.file_path = local
                        .as_ref()
                        .and_then(|l| l.file_identity.file_path.clone());
                    let Some(device_text) = yaml_of(&on_device) else {
                        return Ok(());
                    };
                    // Only over the copy that was compared: a save since (in
                    // another tab) is compared again instead of lost.
                    match local::write_if_unchanged(&key, local_fp.clone(), &device_text).await {
                        Ok(true) => {}
                        Ok(false) => {
                            drop(_checking);
                            start_check(sk);
                            return Ok(());
                        }
                        Err(e) => {
                            log::warn!("Google Drive: could not store {key} locally: {e}");
                            return Ok(());
                        }
                    }
                    let upload_text = rules::drive_text(&out);
                    // Desktop: a .batm beside the recording is rewritten too.
                    // Left as it was, the next open could prefer it over the
                    // central copy (the later modified_at wins there) and
                    // upload the old set.
                    match (&on_device.file_identity.file_path, &upload_text) {
                        (Some(path), Some(t)) => local::rewrite_beside_recording(path, t).await,
                        // The recording's place is not known yet: at its next
                        // open the central copy is used and written beside it.
                        (None, _) => local::mark_unplaced(&key),
                        _ => {}
                    }
                    set_seen(&sk, Some(seen_from(&d)));
                    note_base(&sk, &base_set);
                    if let (true, Some(text)) = (out_fp != d.fp, upload_text) {
                        if let Some(b) = before {
                            with(|s| s.before_merge.insert(sk.clone(), b));
                        }
                        let name = format!("{}.batm", out.file_identity.filename);
                        let seq = next_save_seq();
                        with(|s| s.pending.insert(sk.clone(), Pending { name, text, seq }));
                    }
                }
            }
            if let Some(n) = notice {
                add_merge_notice(&sk, n);
            }
        }
        (Action::Download | Action::Merge, None) | (Action::RemoveLocal, _) => {}
    }
    mark_checked(&sk);
    set_problem(None);
    publish();
    Ok(())
}

/// When no Drive file has the key: ask Drive about the file last seen for
/// it, by id. Returns the files to compare, and whether that file is gone
/// (Drive answered "not found" or "in the trash" for it; a missing entry in
/// a listing is never taken as gone).
async fn confirm_missing(
    sync_key: &str,
    files: Vec<DriveFile>,
) -> Result<(Vec<DriveFile>, bool), DriveError> {
    if !files.is_empty() {
        return Ok((files, false));
    }
    let Some(last) = refresh_seen(sync_key).await.filter(|l| !l.id.is_empty()) else {
        return Ok((files, false));
    };
    match drive::metadata(&last.id).await? {
        Some(f) => Ok((vec![f], false)),
        None => Ok((files, true)),
    }
}

/// The base of the three-way comparison: the fingerprint of the Drive copy
/// as last seen. A key moved in from the old record has none yet; while its
/// Drive revision is still the one last seen, Drive's current copy is the
/// synced one, and its fingerprint becomes the base.
async fn base_for(sync_key: &str, drive: Option<&DriveCopy>) -> Option<String> {
    let last = refresh_seen(sync_key).await?;
    if !last.fp.is_empty() {
        return Some(last.fp);
    }
    let d = drive?;
    if d.file.id == last.id
        && d.file.head_revision_id.is_some()
        && d.file.head_revision_id == last.rev
    {
        set_seen(sync_key, Some(seen_from(d)));
        if let Some(set) = d
            .text
            .as_deref()
            .and_then(|t| yaml_serde::from_str::<AnnotationSet>(t).ok())
        {
            note_base(sync_key, &set);
        }
        return Some(d.fp.clone());
    }
    None
}

/// A key found deleted on Drive and unchanged here: it is removed after the
/// comparison of all files (which runs now if none is running).
fn note_gone(sync_key: &str) {
    with(|s| {
        if !s.gone.iter().any(|k| k == sync_key) {
            s.gone.push(sync_key.to_string());
        }
    });
    if !with(|s| s.reconciling) {
        let gen = generation();
        spawn(async move { reconcile(gen).await });
    }
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
    let (files, gone) = confirm_missing(&sk, files).await?;
    let parse_fp = |text: &str| {
        yaml_serde::from_str::<BatProject>(text)
            .ok()
            .map(|p| rules::project_fingerprint(&p))
    };
    let drive = drive_copy(&sk, &files, parse_fp).await?;
    if !alive(gen) {
        return Ok(());
    }
    let Some(account) = with(|s| s.account.clone()) else {
        return Ok(());
    };
    // The open project counts only if it belongs to the account.
    let is_open = |state: AppState| {
        state
            .project
            .current()
            .with_untracked(|p| p.as_ref().is_some_and(|p| p.id == id))
            && store::project_home(&id) == account
    };
    let local: Option<BatProject> = if is_open(state) {
        state.project.current().get_untracked()
    } else {
        crate::project_store::load_project_in(&account, &id)
            .await
            .ok()
            .flatten()
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
    let base = base_for(&sk, drive.as_ref()).await;
    let side = match (&drive, gone) {
        (_, true) => DriveSide::Gone,
        (Some(d), _) => DriveSide::Present(FileCopy {
            fingerprint: &d.fp,
            modified_at: drive_project
                .as_ref()
                .and_then(|p| p.modified_at.as_deref()),
        }),
        (None, false) => DriveSide::Absent,
    };
    let action = rules::decide_with(
        Kind::Project,
        local
            .as_ref()
            .zip(local_fp.as_deref())
            .map(|(p, fp)| FileCopy {
                fingerprint: fp,
                modified_at: p.modified_at.as_deref(),
            }),
        side,
        base.as_deref(),
    );
    if action == Action::RemoveLocal {
        note_gone(&sk);
        return Ok(());
    }
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
                if let Some(text) = rules::project_drive_text(p) {
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
            // The recordings' paths on this device (desktop) come back from
            // the local copy; Drive never has them.
            let project = rules::with_local_paths(project, local.as_ref());
            store::set_project_home(&id, account.clone());
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
        (Action::Download | Action::Merge, None) | (Action::RemoveLocal, _) => {}
    }
    mark_checked(&sk);
    set_problem(None);
    publish();
    Ok(())
}

// ── Comparing everything ────────────────────────────────────────────────

/// Whether `sync_key` is being uploaded, or was uploaded after `listed` (a
/// `save_seq` value taken before a Drive listing): the listing is out of
/// date for it, and the upload left it compared.
fn changed_since(sync_key: &str, listed: u64) -> bool {
    with(|s| {
        s.uploading.contains(sync_key) || s.uploaded_at.get(sync_key).is_some_and(|&t| t > listed)
    })
}

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
    let Some(account) = with(|s| s.account.clone()) else {
        return Ok(());
    };
    // Files were synced with this account before, but Drive no longer shows
    // its "Oversample" folder: compare, upload and remove nothing (a client
    // also gets "not found" for what it can no longer see), and say so.
    let synced_before = with(|s| s.memory.files.values().filter(|v| !v.id.is_empty()).count());
    let folder_visible = !drive::find_folders("root").await?.is_empty();
    if !alive(gen) {
        return Ok(());
    }
    if !folder_visible && synced_before > 0 {
        with(|s| {
            s.gone.clear();
            s.folders.clear();
        });
        set_problem(Some(DriveProblem::FolderMissing));
        return Ok(());
    }
    trash_deleted_projects(gen).await?;

    // Annotation sets.
    let listed = next_save_seq();
    let mut drive_sets = by_key(drive::files_of_kind(BATM).await?);
    if !alive(gen) {
        return Ok(());
    }
    let local_keys = local::keys().await;
    for key in local_keys {
        let files = drive_sets
            .remove(&rules::drive_key(&key))
            .unwrap_or_default();
        if changed_since(&batm_key(&key), listed) {
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
        if changed_since(&batm_key(&key), listed) {
            continue;
        }
        check_annotations(gen, key, Some(files)).await?;
        if !alive(gen) {
            return Ok(());
        }
    }

    // Projects.
    let listed = next_save_seq();
    let mut drive_projects = by_key(drive::files_of_kind(BATPROJ).await?);
    if !alive(gen) {
        return Ok(());
    }
    let mut local_ids = crate::project_store::project_ids_in(&account).await;
    if let Some(open) = state
        .project
        .current()
        .with_untracked(|p| p.as_ref().map(|p| p.id.clone()))
        .filter(|id| store::project_home(id) == account)
    {
        if !local_ids.contains(&open) {
            local_ids.push(open);
        }
    }
    for id in local_ids {
        let files = drive_projects
            .remove(&rules::drive_key(&id))
            .unwrap_or_default();
        if changed_since(&project_key(&id), listed) {
            continue;
        }
        check_project(gen, id, Some(files)).await?;
        if !alive(gen) {
            return Ok(());
        }
    }
    for (property, files) in drive_projects {
        if !rules::local_key_is_safe(&format!("{property}.batproj"), ".batproj")
            || changed_since(&project_key(&property), listed)
        {
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

    // Files deleted on Drive and unchanged here leave the cache, unless
    // most synced files came back deleted at once.
    let gone = with(|s| std::mem::take(&mut s.gone));
    if !gone.is_empty() {
        if !rules::removals_allowed(folder_visible, synced_before, gone.len()) {
            clear_or_set_problem(Some(DriveProblem::FilesMissing(gone.len())));
            // Other files still go up.
            restart_waiting_uploads();
            publish();
            return Ok(());
        }
        for sk in gone {
            remove_local(gen, &account, &sk).await;
            if !alive(gen) {
                return Ok(());
            }
        }
    }
    clear_problem();
    restart_waiting_uploads();
    publish();
    Ok(())
}

/// Remove `sync_key` from the account's cache, because its Drive file was
/// deleted. Only if the copy here (and an open file's set) is still the one
/// last synced, checked again under the file's lock, so a save that landed
/// since keeps it (and it is uploaded again). An open file's set is dropped
/// inside the lock, so no later save writes it back. An open project is left
/// until it is closed.
async fn remove_local(gen: u64, account: &Store, sync_key: &str) {
    let Some(state) = app_state() else {
        return;
    };
    let Some(base) = refresh_seen(sync_key).await.filter(|s| !s.fp.is_empty()) else {
        return;
    };
    let (kind, key) = split(sync_key);
    let (kind, key) = (kind.to_string(), key.to_string());
    let (dir, name, open) = if kind == BATPROJ {
        let open_project = state
            .project
            .current()
            .with_untracked(|p| p.as_ref().is_some_and(|p| p.id == key));
        if open_project {
            return;
        }
        (account.projects_dir(), format!("{key}.batproj"), None)
    } else {
        (
            account.annotations_dir(),
            key.clone(),
            open_account_file(state, &key),
        )
    };
    let lock = store::file_lock(&dir, &name);
    let (dir2, name2, kind2, base_fp) = (dir.clone(), name.clone(), kind.clone(), base.fp.clone());
    let removed = store::with_lock(&lock, move || {
        Box::pin(async move {
            let open_fp = open
                .and_then(|id| open_file_set(state, id))
                .map(|s| rules::content_fingerprint(&s));
            // A file that does not parse is not the synced one.
            let disk_fp = store::read(&dir2, &name2)
                .await
                .map(|text| fingerprint_of(&kind2, &text).unwrap_or_default());
            if !rules::may_remove(&base_fp, disk_fp.as_deref(), open_fp.as_deref()) {
                return false;
            }
            if let Some(file_id) = open {
                crate::opfs::drop_annotations(state, file_id);
            }
            store::bump_epoch(&dir2, &name2);
            store::remove(&dir2, &name2).await.is_ok()
        })
    })
    .await
    .unwrap_or(false);
    if !alive(gen) {
        return;
    }
    if removed {
        set_seen(sync_key, None);
        with(|s| s.checked.insert(sync_key.to_string()));
        if kind == BATPROJ {
            state
                .drive
                .projects_changed()
                .update(|n| *n = n.wrapping_add(1));
        }
    } else {
        // Changed here since: compared again, and uploaded (an edit beats a
        // delete).
        with(|s| s.checked.remove(sync_key));
        start_check(sync_key.to_string());
    }
    publish();
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
        remove_deleted_project(&id).await;
    }
    Ok(())
}

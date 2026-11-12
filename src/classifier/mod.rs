//! BirdNET (birds), BattyBirdNET (bats) and Perch 2.0 (birds, frogs, insects,
//! some mammals and everyday sounds) on the device.
//!
//! The models run in a Web Worker built from the `birdnet-worker` crate, so the
//! page never blocks and tract stays out of the app's WASM. The worker starts
//! the first time something is run. Nothing here starts on its own: downloads
//! and runs begin only from the calls below.
//!
//! # API for the panel
//!
//! Model files (see [`catalog`] for the list, sizes and attribution):
//! - [`refresh_downloads`]: check which files are on this device. Call when the
//!   panel opens; until then every status is [`DownloadStatus::Unknown`].
//! - [`download`]: download what a [`ModelChoice`] needs (the 67 MB backbone,
//!   plus labels, plus a region's head for bats; Perch's three files, 136 MB).
//!   Progress goes to `state.classifier.downloads()`, per file id.
//! - [`is_ready`]: true when every file a choice needs is downloaded (reads
//!   `downloads`, so it is reactive).
//! - [`delete_files`]: delete stored files (e.g. `catalog::all_files()`).
//!
//! Analysis, one at a time:
//! - [`run`]: analyse a file (it must be the current file; switching files
//!   cancels the run). Fails with [`ClassifierError::NotDownloaded`] if a file
//!   is missing, and with [`ClassifierError::SampleRateTooLow`] for bats below
//!   96 kHz.
//! - [`rescore`]: score a file's stored BattyBirdNET results with another
//!   region's head, without running the backbone again. Downloads the region's
//!   head and labels first if needed.
//! - [`cancel`]: stop the run; the windows so far are kept, marked incomplete.
//! - [`unload`]: stop the worker and free its memory (about 300 MB while a
//!   BirdNET model is loaded, about 400 MB for Perch). The next run starts it
//!   again. Switching between BirdNET and Perch restarts it too.
//!
//! State (`state.classifier`, a [`crate::state::ClassifierState`]):
//! - `downloads()`: `HashMap<&'static str, DownloadStatus>`, keyed by
//!   [`ModelFile::id`].
//! - `run()`: [`RunStatus`]: idle, loading the model, running with a fraction,
//!   rescoring, done, or failed with a [`ClassifierError`] (its `Display` is a
//!   sentence to show).
//! - `results()`: `HashMap<(file_id, Method), ClassifierRun>`. Windows fill in
//!   while a run goes (about every 100 ms). Read with `.with(..)`, not `.get()`.
//! - `embeddings()`: `HashMap<(file_id, Method), Vec<f32>>`, 1,024 floats per
//!   window (1,536 for Perch) in window order (see [`types::embedding`]), for a
//!   later "find similar calls" feature. Kept apart from `results` so showing
//!   scores never copies them.
//!
//! Attribution: [`catalog::attribution`] gives the licence (CC BY-NC-SA 4.0 for
//! BirdNET, Apache 2.0 for Perch), the "Powered by" line and link, credits,
//! citation, source URL and revision, to show with results and in About.

pub mod location;
pub mod place;
pub mod species_map;
pub mod store;
pub mod summary;
pub mod test_hook;
pub mod types;
mod worker;

pub use location::Occurrence;

pub use oversample_ipc::classifier::catalog::{self, Attribution, ModelChoice, ModelFile};
pub use oversample_ipc::classifier::{
    BatRegion, ClassifierError, Method, Score, EMBEDDING_LEN, PERCH_CLASSES, TOP_K,
};
pub use types::{ClassifierRun, DownloadStatus, RunStatus, SpeciesLabel, WindowScores};

use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_core::audio::source::ChannelView;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use worker::{Reply, Stopped};

/// Windows of audio per piece sent to the worker. Two windows keep a cancel
/// quick (well under a second) while the next piece is read in parallel.
const WINDOWS_PER_PIECE: f64 = 2.0;
/// Pieces sent ahead of the worker.
const IN_FLIGHT: usize = 2;

fn set_download(state: AppState, file: &ModelFile, status: DownloadStatus) {
    state.classifier.downloads().update(|m| {
        m.insert(file.id, status);
    });
}

/// Check which catalog files are stored, and set their download status.
/// Files that are downloading keep their status.
pub fn refresh_downloads(state: AppState) {
    spawn_local(async move {
        let stored = store::stored_sizes().await;
        state.classifier.downloads().update(|m| {
            for f in catalog::all_files() {
                if matches!(m.get(f.id), Some(DownloadStatus::Downloading { .. })) {
                    continue;
                }
                let have = stored
                    .iter()
                    .any(|(id, size)| id == f.id && *size == f.size);
                m.insert(
                    f.id,
                    if have {
                        DownloadStatus::Downloaded
                    } else {
                        DownloadStatus::NotDownloaded
                    },
                );
            }
        });
    });
}

/// True when every file `choice` needs is downloaded. Reactive.
pub fn is_ready(state: AppState, choice: ModelChoice) -> bool {
    state.classifier.downloads().with(|m| {
        choice
            .files()
            .iter()
            .all(|f| m.get(f.id) == Some(&DownloadStatus::Downloaded))
    })
}

/// Download one file unless it is stored or already downloading.
async fn download_file(state: AppState, file: &'static ModelFile) -> Result<(), ClassifierError> {
    let status = state
        .classifier
        .downloads()
        .with_untracked(|m| m.get(file.id).cloned());
    match status {
        Some(DownloadStatus::Downloading { .. }) => {
            // Another call is downloading it; wait for that one.
            loop {
                crate::web_util::sleep_ms(200).await;
                let s = state
                    .classifier
                    .downloads()
                    .with_untracked(|m| m.get(file.id).cloned());
                match s {
                    Some(DownloadStatus::Downloading { .. }) => continue,
                    Some(DownloadStatus::Downloaded) => return Ok(()),
                    Some(DownloadStatus::Failed(e)) => return Err(e),
                    _ => break,
                }
            }
        }
        Some(DownloadStatus::Downloaded) => return Ok(()),
        _ => {}
    }
    if store::is_downloaded(file).await {
        set_download(state, file, DownloadStatus::Downloaded);
        return Ok(());
    }
    set_download(state, file, DownloadStatus::Downloading { fraction: 0.0 });
    let mut shown = 0.0;
    let res = store::download(file, |f| {
        // About 200 updates for the backbone, not one per network chunk.
        if f - shown >= 0.005 || f >= 1.0 {
            shown = f;
            set_download(state, file, DownloadStatus::Downloading { fraction: f });
        }
    })
    .await;
    match res {
        Ok(()) => {
            set_download(state, file, DownloadStatus::Downloaded);
            Ok(())
        }
        Err(e) => {
            set_download(state, file, DownloadStatus::Failed(e.clone()));
            Err(e)
        }
    }
}

/// Download every file `choice` needs that is not stored yet, one after the
/// other (the small ones first).
pub fn download(state: AppState, choice: ModelChoice) {
    spawn_local(async move {
        let _ = ensure_downloaded(state, choice).await;
    });
}

/// Download every file `choice` needs that is not stored yet, and finish when
/// all are stored or one fails.
pub async fn ensure_downloaded(
    state: AppState,
    choice: ModelChoice,
) -> Result<(), ClassifierError> {
    let mut files = choice.files();
    files.sort_by_key(|f| f.size);
    for f in files {
        download_file(state, f).await?;
    }
    Ok(())
}

/// The minimum confidence the tab uses for `method` (reactive). Perch has its
/// own, since its confidences are softmax shares.
pub fn min_confidence(state: AppState, method: Method) -> f32 {
    match method {
        Method::Perch => state.call_detection.perch_min_confidence().get(),
        Method::BirdNet | Method::Bats => state.call_detection.min_confidence().get(),
    }
}

/// [`min_confidence`] without tracking.
pub fn min_confidence_untracked(state: AppState, method: Method) -> f32 {
    match method {
        Method::Perch => state.call_detection.perch_min_confidence().get_untracked(),
        Method::BirdNet | Method::Bats => state.call_detection.min_confidence().get_untracked(),
    }
}

pub fn set_min_confidence(state: AppState, method: Method, value: f32) {
    match method {
        Method::Perch => state.call_detection.perch_min_confidence().set(value),
        Method::BirdNet | Method::Bats => state.call_detection.min_confidence().set(value),
    }
}

/// Delete stored files.
pub fn delete_files(state: AppState, files: Vec<&'static ModelFile>) {
    spawn_local(async move {
        for f in files {
            match store::delete(f).await {
                Ok(()) => set_download(state, f, DownloadStatus::NotDownloaded),
                Err(e) => set_download(state, f, DownloadStatus::Failed(e)),
            }
        }
    });
}

thread_local! {
    static LABELS: RefCell<HashMap<&'static str, Arc<Vec<SpeciesLabel>>>> =
        RefCell::new(HashMap::new());
    /// Set once Perch's English names have been fetched from the server (or
    /// tried), so a failing server is asked once per session.
    static PERCH_NAMES_FETCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

async fn labels(
    state: AppState,
    file: &'static ModelFile,
) -> Result<Arc<Vec<SpeciesLabel>>, ClassifierError> {
    if let Some(l) = LABELS.with(|m| m.borrow().get(file.id).cloned()) {
        return Ok(l);
    }
    let bytes = store::read(file).await?;
    let text = String::from_utf8_lossy(&bytes);
    let l = Arc::new(match file.kind {
        catalog::FileKind::PerchLabels => {
            let mut l = types::parse_perch_labels(&text);
            // A list that doesn't line up with the classifier would name every
            // result wrongly.
            if l.len() != PERCH_CLASSES {
                return Err(ClassifierError::Corrupt {
                    file: file.id.to_string(),
                });
            }
            // Perch's labels have no common names: take English names from
            // the lists already stored, and fetch any missing ones without
            // holding up the run.
            let (names, complete) = perch_english(state, &l, false).await;
            types::add_common_names(&mut l, &names);
            if !complete && !PERCH_NAMES_FETCHED.with(|f| f.replace(true)) {
                spawn_local(fetch_perch_names(state, file));
            }
            l
        }
        _ => types::parse_labels(&text),
    });
    LABELS.with(|m| m.borrow_mut().insert(file.id, l.clone()));
    Ok(l)
}

/// The Xeno-Canto groups whose English names name Perch's classes.
const XC_GROUPS: [&str; 5] = ["birds", "frogs", "grasshoppers", "land mammals", "bats"];

/// The groups taken from the server's names file as well: Xeno-Canto's lists
/// already name all of Perch's birds, but only about half of its frogs and
/// grasshoppers, and none of the species without recordings there.
const NAMES_FILE_GROUPS: [&str; 4] = ["frogs", "grasshoppers", "land mammals", "bats"];

/// English names for Perch's labels ("Genus species" to name). For each
/// label the first source that has one wins: Xeno-Canto's species lists, the
/// Oversample server's names file (IUCN, Catalogue of Life, Wikidata and
/// others), then the BirdNET Geomodel's labels, the only one with insects
/// other than grasshoppers and crickets. With `fetch`, sources not stored yet
/// are fetched (the Geomodel's labels are 672 KB). Returns the names and
/// whether every source was found.
async fn perch_english(
    state: AppState,
    labels: &[SpeciesLabel],
    fetch: bool,
) -> (HashMap<String, String>, bool) {
    let (xc, xc_all) = crate::xc_client::english_names(&XC_GROUPS, fetch).await;
    let (names_file, nf_all) =
        crate::xc_client::names_file_english(&NAMES_FILE_GROUPS, fetch).await;
    let geo = location::english_names(state, fetch).await;
    let complete = xc_all && nf_all && geo.is_some();
    let geo = geo.unwrap_or_default();
    let names = labels
        .iter()
        .filter_map(|l| {
            let key = l.scientific.to_lowercase();
            xc.get(&l.scientific)
                .or_else(|| names_file.get(&key))
                .or_else(|| geo.get(&key))
                .map(|en| (l.scientific.clone(), en.clone()))
        })
        .collect();
    (names, complete)
}

/// Fetch the English name lists missing from storage, then rename Perch's
/// labels and the results that use them.
async fn fetch_perch_names(state: AppState, file: &'static ModelFile) {
    let Some(old) = LABELS.with(|m| m.borrow().get(file.id).cloned()) else {
        return;
    };
    let (names, _) = perch_english(state, &old, true).await;
    if names.is_empty() {
        return;
    }
    let mut l = (*old).clone();
    types::add_common_names(&mut l, &names);
    let new = Arc::new(l);
    LABELS.with(|m| m.borrow_mut().insert(file.id, new.clone()));
    state.classifier.results().update(|m| {
        for r in m.values_mut() {
            if Arc::ptr_eq(&r.labels, &old) {
                r.labels = new.clone();
            }
        }
    });
}

fn next_generation(state: AppState) -> u32 {
    let g = state
        .classifier
        .generation()
        .get_untracked()
        .wrapping_add(1);
    state.classifier.generation().set(g);
    g
}

fn is_latest(state: AppState, generation: u32) -> bool {
    state.classifier.generation().get_untracked() == generation
}

fn current_file_id(state: AppState) -> Option<u64> {
    state
        .library
        .current_index()
        .get_untracked()
        .and_then(|i| state.file_id_at(i))
}

/// Stop whatever is running. The windows found so far stay in `results`, with
/// `complete` false.
pub fn cancel(state: AppState) {
    next_generation(state);
    worker::cancel_active();
    if state.classifier.run().with_untracked(|r| r.is_busy()) {
        state.classifier.run().set(RunStatus::Idle);
    }
}

/// Stop the worker and free its memory. Results are kept.
pub fn unload(state: AppState) {
    cancel(state);
    worker::terminate();
}

/// True while the worker is started (it holds the model in memory).
pub fn worker_running() -> bool {
    worker::is_running()
}

/// Analyse file `file_id` (the current file) with `choice`. Replaces that
/// file's earlier results for the same method, and cancels any other run.
pub fn run(state: AppState, file_id: u64, choice: ModelChoice) {
    cancel(state);
    let generation = next_generation(state);
    let method = choice.method();
    state
        .classifier
        .run()
        .set(RunStatus::LoadingModel { file_id, method });
    spawn_local(async move {
        let res = run_task(state, generation, file_id, choice).await;
        if !is_latest(state, generation) {
            return;
        }
        let res: Result<(), ClassifierError> = match res {
            Ok(()) => Ok(()),
            Err(s) => s.into(),
        };
        state.classifier.run().set(match res {
            Ok(()) => RunStatus::Done { file_id, method },
            Err(error) => RunStatus::Failed {
                file_id,
                method,
                error,
            },
        });
    });
}

async fn run_task(
    state: AppState,
    generation: u32,
    file_id: u64,
    choice: ModelChoice,
) -> Result<(), Stopped> {
    let method = choice.method();
    let file = state.library.files().with_untracked(|files| {
        files
            .iter()
            .find(|f| f.id == file_id)
            .map(|f| (f.audio.source.clone(), f.audio.sample_rate))
    });
    let (source, rate) = file.ok_or(ClassifierError::FileNotFound)?;
    if let Some(min) = method.min_source_rate() {
        if rate < min {
            return Err(ClassifierError::SampleRateTooLow { rate, min }.into());
        }
    }
    for f in choice.files() {
        if !store::is_downloaded(f).await {
            let downloading = state.classifier.downloads().with_untracked(|m| {
                matches!(m.get(f.id), Some(DownloadStatus::Downloading { .. }))
            });
            if !downloading {
                set_download(state, f, DownloadStatus::NotDownloaded);
            }
            return Err(ClassifierError::NotDownloaded {
                file: f.id.to_string(),
            }
            .into());
        }
    }
    let labels = labels(state, choice.labels()).await?;
    worker::ensure_family(method == Method::Perch);
    worker::ensure_started(state).await?;
    let models = choice.models();
    for m in &models {
        worker::ensure_model(m).await?;
    }
    let backbone = models[0];
    let head = models.get(1).copied();
    let region = match choice {
        ModelChoice::Bats(r) => Some(r),
        ModelChoice::BirdNet | ModelChoice::Perch => None,
    };
    if !is_latest(state, generation) {
        return Err(Stopped::Superseded);
    }

    let key = (file_id, method);
    state.classifier.results().update(|m| {
        m.insert(
            key,
            ClassifierRun {
                method,
                region,
                backbone,
                head,
                source_rate: rate,
                model_rate: method.model_rate(),
                window_secs: method.window_secs(),
                labels,
                windows: Vec::new(),
                complete: false,
            },
        );
    });
    state.classifier.embeddings().update(|m| {
        m.insert(key, Vec::new());
    });
    state.classifier.run().set(RunStatus::Running {
        file_id,
        method,
        fraction: 0.0,
    });
    let run_id = generation;
    worker::start(run_id, key, rate)?;

    let total = source.total_samples();
    let piece = ((WINDOWS_PER_PIECE * method.window_secs() * rate as f64) as u64).max(4096);
    let mut acks: std::collections::VecDeque<(worker::Waiter, u64)> = Default::default();
    let mut at = 0u64;
    let mut seq = 0u32;
    let set_progress = |done: u64| {
        state.classifier.run().set(RunStatus::Running {
            file_id,
            method,
            fraction: if total > 0 {
                done as f64 / total as f64
            } else {
                1.0
            },
        });
    };
    let still_wanted = || is_latest(state, generation) && current_file_id(state) == Some(file_id);
    while at < total {
        let len = piece.min(total - at) as usize;
        let read = crate::audio::streaming_source::read_region_ready(
            &*source,
            ChannelView::MonoMix,
            at,
            len,
        )
        .await;
        if !still_wanted() {
            if is_latest(state, generation) {
                cancel(state);
            }
            return Err(Stopped::Superseded);
        }
        // A piece that could not be read becomes silence, so later windows keep
        // their times.
        let mut samples = read.unwrap_or_else(|| vec![0.0; len]);
        if samples.len() < len {
            if source.length_is_estimated() {
                // The end of a file whose length was a guess.
                if !samples.is_empty() {
                    let done = at + samples.len() as u64;
                    acks.push_back((worker::send_chunk(run_id, seq, &samples)?, done));
                }
                break;
            }
            // A piece that could not be read: silence, so later times hold.
            samples.resize(len, 0.0);
        }
        acks.push_back((worker::send_chunk(run_id, seq, &samples)?, at + len as u64));
        seq += 1;
        at += len as u64;
        while acks.len() >= IN_FLIGHT {
            let (w, done) = acks.pop_front().expect("non-empty");
            worker::wait(w).await?;
            set_progress(done);
        }
    }
    while let Some((w, done)) = acks.pop_front() {
        worker::wait(w).await?;
        set_progress(done);
    }
    if !still_wanted() {
        if is_latest(state, generation) {
            cancel(state);
        }
        return Err(Stopped::Superseded);
    }
    worker::wait(worker::finish(run_id)?).await?;
    state.classifier.results().update(|m| {
        if let Some(r) = m.get_mut(&key) {
            r.complete = true;
        }
    });
    Ok(())
}

/// Score file `file_id`'s stored BattyBirdNET results with `region`'s head,
/// replacing their scores and labels. Downloads the head and labels if needed.
/// Fails at once with [`ClassifierError::Busy`] while a run is going and with
/// [`ClassifierError::NoResults`] when the file has no bat results; later
/// errors go to `run()` as [`RunStatus::Failed`].
pub fn rescore(state: AppState, file_id: u64, region: BatRegion) -> Result<(), ClassifierError> {
    if state.classifier.run().with_untracked(|r| r.is_busy()) {
        return Err(ClassifierError::Busy);
    }
    let key = (file_id, Method::Bats);
    let windows = state
        .classifier
        .results()
        .with_untracked(|m| m.get(&key).map(|r| r.windows.len()));
    if windows.unwrap_or(0) == 0 {
        return Err(ClassifierError::NoResults);
    }
    let generation = next_generation(state);
    state
        .classifier
        .run()
        .set(RunStatus::Rescoring { file_id, region });
    spawn_local(async move {
        let res = rescore_task(state, generation, key, region).await;
        if !is_latest(state, generation) {
            return;
        }
        let res: Result<(), ClassifierError> = match res {
            Ok(()) => Ok(()),
            Err(s) => s.into(),
        };
        state.classifier.run().set(match res {
            Ok(()) => RunStatus::Done {
                file_id,
                method: Method::Bats,
            },
            Err(error) => RunStatus::Failed {
                file_id,
                method: Method::Bats,
                error,
            },
        });
    });
    Ok(())
}

async fn rescore_task(
    state: AppState,
    generation: u32,
    key: (u64, Method),
    region: BatRegion,
) -> Result<(), Stopped> {
    let head = catalog::bat_head(region);
    let labels_file = catalog::bat_labels(region);
    download_file(state, labels_file).await?;
    download_file(state, head).await?;
    let labels = labels(state, labels_file).await?;
    worker::ensure_family(false);
    worker::ensure_started(state).await?;
    worker::ensure_model(head).await?;
    if !is_latest(state, generation) {
        return Err(Stopped::Superseded);
    }
    // Copy the embeddings into the message without cloning the Vec.
    let len = Method::Bats.embedding_len();
    let waiter = state.classifier.embeddings().with_untracked(|m| {
        let e = m.get(&key).map(Vec::as_slice).unwrap_or_default();
        let windows = (e.len() / len) as u32;
        worker::rescore(generation, windows, &e[..windows as usize * len])
    })?;
    let Reply::Rescored(top) = worker::wait(waiter).await? else {
        return Err(Stopped::Superseded);
    };
    if !is_latest(state, generation) {
        return Err(Stopped::Superseded);
    }
    state.classifier.results().update(|m| {
        if let Some(r) = m.get_mut(&key) {
            for (w, t) in r.windows.iter_mut().zip(top) {
                w.top = t;
            }
            r.region = Some(region);
            r.head = Some(head);
            r.labels = labels;
        }
    });
    Ok(())
}

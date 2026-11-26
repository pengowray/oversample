//! BirdNET V2.4 (birds), BattyBirdNET (bats), Perch 2.0 (birds, frogs,
//! insects, some mammals and everyday sounds) and BirdNET+ V3.0 (birds,
//! insects, frogs and mammals) on the device.
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
//!   plus labels, plus a region's head for bats, or all 11 heads, 1.1 MB, for
//!   "All regions"; Perch's three files, 136 MB; BirdNET+ V3.0's one file,
//!   72 MB, whose labels are served with the app).
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
//!   region's head (or every region's), without running the backbone again.
//!   Downloads the heads and labels first if needed.
//! - [`bat_pick::bat_scope`]: the region BattyBirdNET uses now: the one picked
//!   in the tab, or with "Auto" the one for the recording's place.
//! - [`cancel`]: stop the run; the windows so far are kept, marked incomplete.
//! - [`unload`]: stop the worker and free its memory (about 300 MB while a
//!   BirdNET model is loaded, about 400 MB for Perch and for BirdNET+ V3.0).
//!   The next run starts it again. Switching between BirdNET V2.4, BirdNET+
//!   V3.0 and Perch restarts it too.
//!
//! State (`state.classifier`, a [`crate::state::ClassifierState`]):
//! - `downloads()`: `HashMap<&'static str, DownloadStatus>`, keyed by
//!   [`ModelFile::id`].
//! - `run()`: [`RunStatus`]: idle, loading the model, running with a fraction,
//!   rescoring, done, or failed with a [`ClassifierError`] (its `Display` is a
//!   sentence to show).
//! - `results()`: `HashMap<(file_id, Method), ClassifierRun>`. Windows fill in
//!   while a run goes (about every 100 ms), kept in time order; with a step
//!   shorter than the window, later passes insert windows between earlier
//!   ones ([`ClassifierRun::insert_reading`]). The live view adds its
//!   readings of a file too, wherever it plays ([`played`]). Read with
//!   `.with(..)`, not `.get()`.
//! - `embeddings()`: `HashMap<(file_id, Method), Vec<f32>>`, 1,024 floats per
//!   window (1,536 for Perch, 1,280 for BirdNET+ V3.0) in the order the
//!   windows arrived, which only ever grows at the end; each window's `slot`
//!   says where its embedding is (see [`types::embedding`]). For the sound
//!   groups and a later "find similar calls" feature. Kept apart from
//!   `results` so showing scores never copies them.
//!
//! Attribution: [`catalog::attribution`] gives the licence (CC BY-NC-SA 4.0 for
//! BirdNET V2.4, Apache 2.0 for Perch and BirdNET+ V3.0), the "Powered by" line and link, credits,
//! citation, source URL and revision, to show with results and in About.

pub mod bat_pick;
pub mod boxes;
pub mod combined;
pub mod noise_scan;
pub mod sound_boxes;
pub mod kept_order;
pub mod location;
pub mod method_info;
pub mod played;
pub mod quiet;
pub mod sound_groups;
pub mod species_map;
pub mod store;
pub mod strips;
pub mod summary;
pub mod test_hook;
pub mod types;
mod worker;

pub use bat_pick::BatPick;
pub use location::Occurrence;
pub use method_info::{MethodFacts, MethodInfo};

pub use oversample_ipc::classifier::catalog::{self, Attribution, ModelChoice, ModelFile};
pub use oversample_ipc::classifier::{
    BatRegion, BatScope, ClassifierError, Method, Score, EMBEDDING_LEN, PERCH_CLASSES, TOP_K,
    V3_CLASSES,
};
pub use types::{ClassifierRun, DownloadStatus, RunStatus, SpeciesLabel, WindowScores};

use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_core::audio::source::ChannelView;
use oversample_core::erased_sort::SortErased;
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
    let res = store::download(file, move |f| {
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
    files.sort_erased_by_key(|f| f.size);
    for f in files {
        download_file(state, f).await?;
    }
    Ok(())
}

/// The minimum confidence the tab uses for `method` (reactive). Perch has its
/// own, since its confidences are softmax shares, and so has BirdNET+ V3.0
/// (default 0.25), whose scores run differently from V2.4's.
pub fn min_confidence(state: AppState, method: Method) -> f32 {
    min_confidence_field(state, method).get()
}

/// [`min_confidence`] without tracking.
pub fn min_confidence_untracked(state: AppState, method: Method) -> f32 {
    min_confidence_field(state, method).get_untracked()
}

pub fn set_min_confidence(state: AppState, method: Method, value: f32) {
    min_confidence_field(state, method).set(value)
}

fn min_confidence_field(
    state: AppState,
    method: Method,
) -> impl leptos::reactive::traits::Get<Value = f32>
       + leptos::reactive::traits::GetUntracked<Value = f32>
       + leptos::reactive::traits::Set<Value = f32> {
    let cd = state.call_detection;
    match method {
        Method::Perch => cd.perch_min_confidence(),
        Method::BirdNetV3 => cd.v3_min_confidence(),
        Method::BirdNet | Method::Bats => cd.min_confidence(),
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

/// BirdNET's labels, read from the stored label file.
pub async fn birdnet_labels(state: AppState) -> Result<Arc<Vec<SpeciesLabel>>, ClassifierError> {
    labels(state, &catalog::BIRDNET_LABELS).await
}

/// The labels of `file` if they have been read, as they are now (Perch's
/// get English names after a background fetch, which replaces the list).
pub fn cached_labels(file: &ModelFile) -> Option<Arc<Vec<SpeciesLabel>>> {
    LABELS.with(|m| m.borrow().get(file.id).cloned())
}

/// The labels of `method` if they have been read, as they are now: the same
/// list [`birdnet_labels`], [`perch_labels`] or [`birdnet_v3_labels`] gave.
/// `None` for BattyBirdNET, whose labels depend on the region.
pub fn cached_method_labels(method: Method) -> Option<Arc<Vec<SpeciesLabel>>> {
    let key = match method {
        Method::BirdNet => catalog::BIRDNET_LABELS.id,
        Method::Perch => catalog::PERCH_LABELS.id,
        Method::BirdNetV3 => catalog::BIRDNET_V3_LABELS_ASSET,
        Method::Bats => return None,
    };
    LABELS.with(|m| m.borrow().get(key).cloned())
}

/// Perch's labels, read from the stored label file, with English names where
/// they are known.
pub async fn perch_labels(state: AppState) -> Result<Arc<Vec<SpeciesLabel>>, ClassifierError> {
    labels(state, &catalog::PERCH_LABELS).await
}

/// BirdNET+ V3.0's labels, fetched from the app's own files (they are served
/// with it, not downloaded with the model) the first time.
pub async fn birdnet_v3_labels() -> Result<Arc<Vec<SpeciesLabel>>, ClassifierError> {
    let key = catalog::BIRDNET_V3_LABELS_ASSET;
    if let Some(l) = LABELS.with(|m| m.borrow().get(key).cloned()) {
        return Ok(l);
    }
    let bytes = crate::components::file_sidebar::fetch_bytes(key)
        .await
        .map_err(|message| ClassifierError::Download {
            file: key.to_string(),
            message,
        })?;
    let text = String::from_utf8_lossy(&bytes);
    let l = types::parse_birdnet_v3_labels(&text);
    let birds = types::parse_birdnet_v3_birds(&text);
    // A list that doesn't line up with the model would name every result
    // wrongly.
    if l.len() != V3_CLASSES || birds.len() != V3_CLASSES {
        return Err(ClassifierError::Corrupt {
            file: key.to_string(),
        });
    }
    let l = Arc::new(l);
    boxes::set_v3_birds(l.clone(), birds);
    LABELS.with(|m| m.borrow_mut().insert(key, l.clone()));
    Ok(l)
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

thread_local! {
    /// The label lists of every regional head as one, and the class of each
    /// head's outputs in it. Built the first time "All regions" is used.
    static ALL_REGIONS: RefCell<Option<AllRegions>> = const { RefCell::new(None) };
}

type AllRegions = (
    Arc<Vec<SpeciesLabel>>,
    std::rc::Rc<oversample_ipc::classifier::MergedBatLabels>,
);

async fn all_regions() -> Result<AllRegions, ClassifierError> {
    if let Some(all) = ALL_REGIONS.with(|a| a.borrow().clone()) {
        return Ok(all);
    }
    let mut texts = Vec::new();
    for f in catalog::bat_label_files(BatScope::All) {
        texts.push(String::from_utf8_lossy(&store::read(f).await?).into_owned());
    }
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let merged = oversample_ipc::classifier::merge_bat_labels(&texts);
    let labels = Arc::new(types::parse_labels(&merged.lines.join("\n")));
    let all = (labels, std::rc::Rc::new(merged));
    ALL_REGIONS.with(|a| *a.borrow_mut() = Some(all.clone()));
    Ok(all)
}

/// The model that takes the audio and the one that scores it, as a run of
/// `choice` records them (`ClassifierRun::backbone`, `head`).
pub fn model_files(choice: ModelChoice) -> (&'static ModelFile, Option<&'static ModelFile>) {
    match choice {
        ModelChoice::BirdNet => (&catalog::BACKBONE, None),
        ModelChoice::Bats(scope) => (&catalog::BACKBONE, single_bat_head(scope)),
        ModelChoice::Perch => (&catalog::PERCH_EMBEDDING, Some(&catalog::PERCH_CLASSIFIER)),
        ModelChoice::BirdNetV3 => (&catalog::BIRDNET_V3, None),
    }
}

/// The head of `scope` as one file; `None` for all regions.
fn single_bat_head(scope: BatScope) -> Option<&'static ModelFile> {
    match scope {
        BatScope::Region(r) => Some(catalog::bat_head(r)),
        BatScope::All => None,
    }
}

/// Put `scope`'s head (or every head, for all regions) in the worker, and
/// give the labels for the classes it scores. The files must be stored and
/// the worker started.
async fn load_bat_heads(
    state: AppState,
    scope: BatScope,
) -> Result<Arc<Vec<SpeciesLabel>>, Stopped> {
    match scope {
        BatScope::Region(r) => {
            let labels = labels(state, catalog::bat_labels(r)).await?;
            worker::ensure_model(catalog::bat_head(r)).await?;
            Ok(labels)
        }
        BatScope::All => {
            let (labels, merged) = all_regions().await?;
            worker::ensure_bat_heads(
                catalog::ALL_BAT_HEADS_KEY,
                &catalog::bat_heads(scope),
                &merged,
            )
            .await?;
            Ok(labels)
        }
    }
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
    if state
        .classifier
        .combined_stage()
        .with_untracked(|s| s.is_some())
    {
        state.classifier.combined_stage().set(None);
    }
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

/// The step between window starts a run of `method` uses when `step_secs`
/// is asked for, in seconds: `step_secs` when it is shorter than the window
/// and fits a whole number of times in it, otherwise the whole window.
/// A method without the Step setting ([`MethodInfo::step_setting`]:
/// BattyBirdNET) always uses the whole window.
pub fn step_for(method: Method, step_secs: f64) -> f64 {
    if !method.info().step_setting {
        return method.window_secs();
    }
    method
        .step_samples(step_secs)
        .map_or(method.window_secs(), |s| {
            s as f64 / method.model_rate() as f64
        })
}

/// True when an Analyse of file `file_id` with `choice` also finds bird
/// boxes: BirdNET+ V3.0 with "Find bird boxes" on, on a recording they apply
/// to ([`boxes::applies_to`]).
pub fn finds_boxes(state: AppState, file_id: u64, choice: ModelChoice) -> bool {
    choice == ModelChoice::BirdNetV3
        && state.call_detection.bird_boxes().get_untracked()
        && untrack(|| finds_boxes_for(state, file_id))
}

/// True when bird boxes apply to file `file_id` ([`boxes::applies_to`]).
/// Reactive.
pub fn finds_boxes_for(state: AppState, file_id: u64) -> bool {
    state.library.files().with(|files| {
        files
            .iter()
            .find(|f| f.id == file_id)
            .is_some_and(boxes::applies_to)
    })
}

/// Every file an Analyse of `file_id` with `choice` needs: the choice's,
/// and the bird box model when it finds boxes.
pub fn run_files(state: AppState, file_id: u64, choice: ModelChoice) -> Vec<&'static ModelFile> {
    let mut files = choice.files();
    if finds_boxes(state, file_id, choice) {
        files.push(&catalog::JUST_BIRD);
    }
    files
}

/// True when every file in `files` is downloaded. Reactive.
pub fn files_ready(state: AppState, files: &[&'static ModelFile]) -> bool {
    state.classifier.downloads().with(|m| {
        files
            .iter()
            .all(|f| m.get(f.id) == Some(&DownloadStatus::Downloaded))
    })
}

/// Download every file in `files` that is not stored yet, smallest first,
/// and finish when all are stored or one fails.
pub async fn ensure_files(
    state: AppState,
    mut files: Vec<&'static ModelFile>,
) -> Result<(), ClassifierError> {
    files.sort_erased_by_key(|f| f.size);
    for f in files {
        download_file(state, f).await?;
    }
    Ok(())
}

/// Analyse file `file_id` (the current file) with `choice`, with `step_secs`
/// between window starts ([`step_for`]). Replaces that file's earlier results
/// for the same method, except the readings the live view made of it
/// ([`played`]), and cancels any other run.
///
/// With a step shorter than the window the run goes over the recording more
/// than once (`oversample_ipc::classifier::refine_passes`): first the windows
/// end to end, so the whole recording has results soon, then passes that
/// each add windows between those already done, over the whole recording.
pub fn run(state: AppState, file_id: u64, choice: ModelChoice, step_secs: f64) {
    cancel(state);
    let generation = next_generation(state);
    let method = choice.method();
    state
        .classifier
        .run()
        .set(RunStatus::LoadingModel { file_id, method });
    let boxes = finds_boxes(state, file_id, choice);
    spawn_local(async move {
        let res = run_task(state, generation, file_id, choice, step_secs, boxes).await;
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

/// One Analyse of `file_id` with `choice`. With `boxes` (BirdNET+ V3.0
/// only), Just-Bird also finds bird boxes; the caller decides, so the
/// combined scan can find them whatever "Find bird boxes" is set to.
async fn run_task(
    state: AppState,
    generation: u32,
    file_id: u64,
    choice: ModelChoice,
    step_secs: f64,
    boxes: bool,
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
    let mut needed = choice.files();
    if boxes {
        needed.push(&catalog::JUST_BIRD);
    }
    for f in needed {
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
    worker::ensure_family(method.family());
    worker::ensure_started(state).await?;
    let (region, labels) = match choice {
        ModelChoice::BirdNet => {
            let labels = labels(state, &catalog::BIRDNET_LABELS).await?;
            worker::ensure_model(&catalog::BACKBONE).await?;
            (None, labels)
        }
        ModelChoice::Bats(scope) => {
            worker::ensure_model(&catalog::BACKBONE).await?;
            (Some(scope), load_bat_heads(state, scope).await?)
        }
        ModelChoice::Perch => {
            let labels = labels(state, &catalog::PERCH_LABELS).await?;
            worker::ensure_model(&catalog::PERCH_EMBEDDING).await?;
            worker::ensure_model(&catalog::PERCH_CLASSIFIER).await?;
            (None, labels)
        }
        ModelChoice::BirdNetV3 => {
            let labels = birdnet_v3_labels().await?;
            worker::ensure_model(&catalog::BIRDNET_V3).await?;
            if boxes {
                worker::ensure_model(&catalog::JUST_BIRD).await?;
            }
            (None, labels)
        }
    };
    let (backbone, head) = model_files(choice);
    if !is_latest(state, generation) {
        return Err(Stopped::Superseded);
    }

    let step_secs = step_for(method, step_secs);
    let model_rate = method.model_rate() as f64;
    let window = method.window_samples() as u32;
    let mut passes = oversample_ipc::classifier::refine_passes(
        window,
        method.step_samples(step_secs).unwrap_or(window),
    );
    // Bird boxes are named from windows every 1.5 s. A step that does not
    // give them (3 s or 1 s) gets a pass of its own for them, right after the
    // windows end to end, so the boxes have names soon. It leaves the step
    // the results report as it was.
    let grid = (oversample_ipc::classifier::boxes::CLIP_HOP_S * model_rate) as u32;
    let naming_pass = boxes && !passes.iter().any(|p| p.offsets.contains(&grid));
    if naming_pass {
        passes.insert(
            1,
            oversample_ipc::classifier::Pass {
                offsets: vec![grid],
                spacing: window,
            },
        );
    }
    let key = (file_id, method);
    // The new run starts with the readings the live view made of this file
    // (`played`), which are the windows Analyse gives for their starts; the
    // windows of an earlier Analyse go. Its own windows then go in around
    // them, and one at a kept reading's start is not added twice.
    played::flush(state, worker::now_ms());
    let old_embeddings = state
        .classifier
        .embeddings()
        .try_update_untracked(|m| m.remove(&key))
        .flatten()
        .unwrap_or_default();
    let (kept, kept_embeddings) = state.classifier.results().with_untracked(|m| {
        m.get(&key)
            .filter(|r| r.source_rate == rate)
            .map(|r| r.live_windows(&old_embeddings))
            .unwrap_or_default()
    });
    state.classifier.results().update(|m| {
        let mut run = ClassifierRun::empty(method, region, backbone, head, rate, labels, step_secs);
        run.windows = kept;
        m.insert(key, run);
    });
    state.classifier.embeddings().update(|m| {
        m.insert(key, kept_embeddings);
    });
    if boxes {
        state.classifier.boxes().update(|m| {
            m.insert(file_id, boxes::BoxRun::new(rate));
        });
    }
    let run_id = generation;
    // The spacing the results have after each pass: the extra pass for the
    // bird boxes does not change it.
    let mut spacing = window;
    for (p, pass) in passes.iter().enumerate() {
        let naming_boxes = naming_pass && p == 1;
        if !naming_boxes {
            spacing = pass.spacing;
        }
        let refining = (p > 0 && !naming_boxes).then(|| pass.spacing as f64 / model_rate);
        state.classifier.run().set(RunStatus::Running {
            file_id,
            method,
            fraction: 0.0,
            refining,
            naming_boxes,
        });
        worker::start(run_id, key, rate, pass.offsets.clone(), boxes && p == 0)?;
        // About two windows per piece, as for the windows end to end, so a
        // Stop never waits behind many slow windows.
        let piece_secs = WINDOWS_PER_PIECE * method.window_secs() / pass.offsets.len() as f64;
        let piece = ((piece_secs * rate as f64) as u64).max(4096);
        let progress = |fraction| {
            state.classifier.run().set(RunStatus::Running {
                file_id,
                method,
                fraction,
                refining,
                naming_boxes,
            })
        };
        stream_pass(
            state, generation, file_id, &*source, run_id, piece, progress,
        )
        .await?;
        state.classifier.results().update(|m| {
            if let Some(r) = m.get_mut(&key) {
                r.whole_file = true;
                r.spacing_secs = spacing as f64 / model_rate;
            }
        });
    }
    state.classifier.results().update(|m| {
        if let Some(r) = m.get_mut(&key) {
            r.complete = true;
        }
    });
    Ok(())
}

/// Send the whole recording to the worker for one pass that
/// [`worker::start`] began, `piece` samples at a time, and wait until the
/// worker has scored its last window. `progress` gets the share of the
/// recording done.
async fn stream_pass(
    state: AppState,
    generation: u32,
    file_id: u64,
    source: &dyn oversample_core::audio::source::AudioSource,
    run_id: u32,
    piece: u64,
    progress: impl Fn(f64),
) -> Result<(), Stopped> {
    let total = source.total_samples();
    let mut acks: std::collections::VecDeque<(worker::Waiter, u64)> = Default::default();
    let mut at = 0u64;
    let mut seq = 0u32;
    let set_progress = |done: u64| {
        progress(if total > 0 {
            done as f64 / total as f64
        } else {
            1.0
        })
    };
    let still_wanted = || is_latest(state, generation) && current_file_id(state) == Some(file_id);
    while at < total {
        let len = piece.min(total - at) as usize;
        let read = crate::audio::streaming_source::read_region_ready(
            source,
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
    Ok(())
}

/// Score file `file_id`'s stored BattyBirdNET results with `scope`'s head (or
/// every head), replacing their scores and labels. Downloads the heads and
/// labels if needed.
/// Fails at once with [`ClassifierError::Busy`] while a run is going and with
/// [`ClassifierError::NoResults`] when the file has no bat results; later
/// errors go to `run()` as [`RunStatus::Failed`].
pub fn rescore(state: AppState, file_id: u64, scope: BatScope) -> Result<(), ClassifierError> {
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
        .set(RunStatus::Rescoring { file_id, scope });
    spawn_local(async move {
        let res = rescore_task(state, generation, key, scope).await;
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
    scope: BatScope,
) -> Result<(), Stopped> {
    for f in catalog::bat_label_files(scope)
        .into_iter()
        .chain(catalog::bat_heads(scope))
    {
        download_file(state, f).await?;
    }
    worker::ensure_family(Method::Bats.family());
    worker::ensure_started(state).await?;
    let labels = load_bat_heads(state, scope).await?;
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
            // The scores come in the embeddings' order, the order the windows
            // arrived.
            for w in r.windows.iter_mut() {
                if let Some(t) = top.get(w.slot as usize) {
                    w.top = t.clone();
                }
            }
            r.region = Some(scope);
            r.head = single_bat_head(scope);
            r.labels = labels;
        }
    });
    Ok(())
}

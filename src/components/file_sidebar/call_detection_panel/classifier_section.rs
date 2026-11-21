//! The BirdNET, BattyBirdNET, Perch and BirdNET+ V3.0 part of the Call
//! detection tab: download the model, analyse the current file, list the
//! species found, and add them as annotations. The calls it makes are in
//! `crate::classifier`.
//!
//! Order on screen (the layout the owner approved on 2026-11-21):
//!
//! 1. The action (download, Analyse, progress) with the settings button
//!    beside it, a line listing the settings in force, and the settings
//!    themselves when opened (Step, minimum confidence, place and "Mark birds
//!    unlikely here", boxes on the spectrogram).
//! 2. "At the play position" (`live_view.rs`), for the methods with live
//!    readings.
//! 3. "In this recording": one list of the species found, the sound
//!    clusters and the picked components, each with a pin that shows it in a
//!    row above the spectrogram; then the search that adds a row.
//! 4. Folds: "Every window" and "Add N annotations".
//! 5. One line about the stored model, and the attribution the model's
//!    licence asks for.

use super::bird_boxes;
use super::live_view::LiveView;
use super::parts::{in_force_line, pin_button, section_head, settings_button, Fold};
use super::species_map::SpeciesMapDialog;
use super::{count, event_target_checked, event_target_value, undo_keys};
use crate::annotations::{now_iso8601, AnnotationId, AnnotationSet};
use crate::call_annotations::remove_annotations_where;
use crate::classifier::summary::{self, WindowKind};
use crate::classifier::{
    self, catalog, location, ClassifierError, DownloadStatus, Method, ModelChoice, RunStatus,
};
use crate::components::icons::Icon;
use crate::recording_place as place;
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;

const MB: f64 = 1_000_000.0;

/// `67 MB`, or `0.1 MB` for a small file.
fn mb(bytes: u64) -> String {
    let m = bytes as f64 / MB;
    if m >= 10.0 {
        format!("{m:.0}\u{a0}MB")
    } else {
        format!("{:.1}\u{a0}MB", m.max(0.1))
    }
}

/// `1,200`
fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Windows a run of a `duration`-second recording gives. The worker drops a
/// last window holding less than a third of a window of audio, unless it is
/// the only one (`birdnet-worker/src/windows.rs`).
fn expected_windows(duration: f64, window: f64) -> usize {
    let full = (duration / window).floor() as usize;
    let tail = duration - full as f64 * window;
    full + usize::from(full == 0 || tail >= window / 3.0)
}

/// `3 s`, `0.56 s`
fn window_len(secs: f64) -> String {
    if secs >= 1.0 {
        format!("{secs:.0} s")
    } else {
        format!("{secs:.2} s")
    }
}

/// What the panel needs to know about this file's results for its method.
#[derive(Clone, Copy, PartialEq)]
struct RunInfo {
    windows: usize,
    /// Windows Analyse made; the rest are readings at the play position.
    analysed: usize,
    /// Time the windows' cells cover, in seconds.
    covered_secs: f64,
    complete: bool,
    whole_file: bool,
    region: Option<classifier::BatScope>,
    window_secs: f64,
    /// The step the Analyse asked for, and the step the whole recording has
    /// so far (`ClassifierRun::step_secs`, `spacing_secs`).
    step_secs: f64,
    spacing_secs: f64,
}

/// "16 windows", or when windows overlap (so a count would count the same
/// audio more than once) the time of their cells: "14 s of audio".
fn windows_or_time(s: &summary::Summary, windows: usize, secs: f64) -> String {
    if s.overlapping {
        format!("{} of audio", summary::time_amount(secs))
    } else {
        count(windows, "window", "windows")
    }
}

/// The Step setting: the time between the starts of windows for the next
/// Analyse, as a group of buttons like the number of Sound clusters. Saved
/// for every classifier. None for BattyBirdNET.
fn step_row(method: Method, pick: RwSignal<summary::StepPick>) -> Option<impl IntoView> {
    let choices = summary::step_choices(method);
    if choices.is_empty() {
        return None;
    }
    let window = method.window_secs();
    let help = format!(
        "Time between the starts of windows. At 1 s a new {} window starts every second, so windows overlap. Applies to the next Analyse.",
        summary::step_text(window)
    );
    let buttons = choices
        .into_iter()
        .map(|(choice, secs)| {
            let on = move || pick.get().secs(method) == secs;
            let text = summary::step_text(secs);
            view! {
                <button
                    class=move || if on() { "psd-btn psd-btn-active" } else { "psd-btn" }
                    aria-pressed=move || on().to_string()
                    aria-label=format!("Step {text}")
                    on:click=move |_| {
                        pick.set(choice);
                        crate::settings::set_classifier_step(choice);
                    }
                >{text.clone()}</button>
            }
        })
        .collect::<Vec<_>>();
    Some(view! {
        <div class="setting-row detection-step-row" title=help>
            <span class="setting-label">"Step"</span>
            <div class="psd-btn-group" role="group" aria-label="Step">{buttons}</div>
        </div>
    })
}

/// A small square in the colour the spectrogram's label strip gives this
/// class, so a row in the lists can be matched to its label and call boxes.
/// `slot` is the species' place in `summary::colour_order`; None for classes
/// that are not a species ("Noise"), or that no window has as its top class.
/// Inline style: the colour comes from overlays.rs, the shape from the
/// species map legend's `.map-key`.
pub(super) fn species_swatch(slot: Option<usize>) -> impl IntoView {
    use crate::canvas::spectrogram_renderer::{species_colour, NOT_SPECIES_COLOUR};
    let colour = slot.map_or(NOT_SPECIES_COLOUR, |s| species_colour(s).0);
    view! {
        <span
            class="map-key cd-swatch"
            style=format!("background:{colour}")
        ></span>
    }
}

/// A class name as shown: BattyBirdNET's lower-case classes get a capital.
fn class_name(label: Option<&classifier::SpeciesLabel>) -> String {
    label.map_or_else(|| "Unknown".to_string(), classifier::strips::display_name)
}

/// True when this species has a row above the spectrogram. Reactive.
pub(super) fn strip_picked(state: AppState, method: Method, scientific: &str) -> bool {
    state.call_detection.strip_species().with(|v| {
        v.iter()
            .any(|s| s.method == method && s.scientific == scientific)
    })
}

/// Add or remove this species' row above the spectrogram.
pub(super) fn toggle_strip(state: AppState, method: Method, scientific: String) {
    state.call_detection.strip_species().update(|v| {
        classifier::strips::toggle(v, classifier::strips::StripSpecies { method, scientific })
    });
    save_strips(state);
}

/// True when component `k` (0 for PC1) has a row above the spectrogram.
/// Reactive.
pub(super) fn component_picked(state: AppState, k: usize) -> bool {
    state
        .call_detection
        .strip_components()
        .with(|v| v.contains(&k))
}

/// Add or remove component `k`'s row above the spectrogram.
pub(super) fn toggle_component(state: AppState, k: usize) {
    state
        .call_detection
        .strip_components()
        .update(|v| classifier::strips::toggle(v, k));
    save_strips(state);
}

/// Save the picked rows (species, components and sound clusters) for the
/// next session.
pub(super) fn save_strips(state: AppState) {
    let cd = state.call_detection;
    crate::settings::set_score_strips(&classifier::strips::StripPicks {
        species: cd.strip_species().get_untracked(),
        components: cd.strip_components().get_untracked(),
        sound_groups: cd.sound_groups().get_untracked(),
        groups: cd.sound_group_count().get_untracked(),
    });
}

/// The pin cell of a species row.
pub(super) fn species_pin(
    state: AppState,
    method: Method,
    scientific: String,
    name: String,
) -> impl IntoView {
    let sci = scientific.clone();
    let picked = move || strip_picked(state, method, &sci);
    view! {
        <td class="cd-col-pin">
            {pin_button(name, picked, move || toggle_strip(state, method, scientific.clone()))}
        </td>
    }
}

/// Scroll the spectrogram so the window from `start_s` to `end_s` has its
/// middle in the centre, and pick it.
pub(super) fn show_window(state: AppState, start_s: f64, end_s: f64) {
    state.call_detection.selected_window().set(Some(start_s));
    let mid = 0.5 * (start_s + end_s);
    let files = state.library.files().get_untracked();
    let idx = state.library.current_index().get_untracked();
    if let Some(file) = idx.and_then(|i| files.get(i)) {
        let canvas_w = state.viewmode.spectrogram_canvas_width().get_untracked();
        let zoom = state.view.zoom_level().get_untracked();
        let visible_time = (canvas_w / zoom) * file.spectrogram.time_resolution;
        state
            .view
            .scroll_offset()
            .set((mid - visible_time / 2.0).max(0.0));
    }
}

/// Download what `choice` still needs (a region's classifier is about 0.1 MB,
/// all 11 are 1.1 MB),
/// then analyse the current file with `step_secs` between window starts.
fn analyse(state: AppState, file_id: u64, choice: ModelChoice, step_secs: f64) {
    state.call_detection.selected_window().set(None);
    state.call_detection.picked_box().set(None);
    // With bird boxes on, BirdBox's model too.
    let files = classifier::run_files(state, file_id, choice);
    if untrack(|| classifier::files_ready(state, &files)) {
        classifier::run(state, file_id, choice, step_secs);
        return;
    }
    let method = choice.method();
    state
        .classifier
        .run()
        .set(RunStatus::LoadingModel { file_id, method });
    spawn_local(async move {
        match classifier::ensure_files(state, files).await {
            Ok(()) => classifier::run(state, file_id, choice, step_secs),
            Err(error) => state.classifier.run().set(RunStatus::Failed {
                file_id,
                method,
                error,
            }),
        }
    });
}

/// The large files the panel's download button and Delete stand for:
/// BirdNET's backbone (shared by BirdNET and BattyBirdNET; labels and regional
/// classifiers are small and download on Analyse), all of Perch's files, or
/// BirdNET+ V3.0's one file.
fn model_files(method: Method) -> Vec<&'static catalog::ModelFile> {
    match method {
        Method::BirdNet | Method::Bats => vec![&catalog::BACKBONE],
        Method::Perch => catalog::PERCH_FILES.to_vec(),
        Method::BirdNetV3 => vec![&catalog::BIRDNET_V3],
    }
}

/// The files that Delete removes.
fn deletable_files(method: Method) -> Vec<&'static catalog::ModelFile> {
    match method {
        Method::BirdNet | Method::Bats => catalog::birdnet_files().collect(),
        Method::Perch => catalog::PERCH_FILES.to_vec(),
        Method::BirdNetV3 => vec![&catalog::BIRDNET_V3],
    }
}

/// One status for several files: failed or unknown if any is, downloading
/// (with the share of all their bytes) while any is, downloaded when all are.
fn combined_status(
    downloads: &std::collections::HashMap<&'static str, DownloadStatus>,
    files: &[&'static catalog::ModelFile],
) -> DownloadStatus {
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut done = 0.0;
    let mut downloading = false;
    let mut missing = false;
    for f in files {
        match downloads.get(f.id).cloned().unwrap_or_default() {
            DownloadStatus::Failed(e) => return DownloadStatus::Failed(e),
            DownloadStatus::Unknown => return DownloadStatus::Unknown,
            DownloadStatus::Downloaded => done += f.size as f64,
            DownloadStatus::Downloading { fraction } => {
                downloading = true;
                done += fraction * f.size as f64;
            }
            DownloadStatus::NotDownloaded => missing = true,
        }
    }
    if downloading {
        DownloadStatus::Downloading {
            fraction: done / total.max(1) as f64,
        }
    } else if missing {
        DownloadStatus::NotDownloaded
    } else {
        DownloadStatus::Downloaded
    }
}

/// Add the windows at or above the minimum confidence as annotations,
/// replacing the ones this method (and region) added before.
fn add_annotations(state: AppState, file_id: u64, method: Method, merge: bool) {
    let min = classifier::min_confidence_untracked(state, method);
    let now = now_iso8601();
    let built = state.classifier.results().with_untracked(|m| {
        m.get(&(file_id, method)).map(|run| {
            let spans = summary::spans(run, min, merge);
            (
                summary::build_annotations(run, &spans, &now, AnnotationId::new, |l| {
                    untrack(|| location::unlikely_here(state, file_id, method, l))
                }),
                summary::auto_tag_value(run),
            )
        })
    });
    let Some((annotations, tag)) = built else {
        return;
    };
    if annotations.is_empty() {
        return;
    }
    let Some(idx) = state.library.current_index().get_untracked() else {
        return;
    };
    let new_set = state.library.files().with_untracked(|files| {
        files.get(idx).map(|f| {
            let id = f.identity.clone().unwrap_or_else(|| {
                crate::file_identity::identity_layer1(&f.name, f.audio.metadata.file_size as u64)
            });
            AnnotationSet::new_with_metadata(id, &f.audio, f.cached_peak_db, f.cached_full_peak_db)
        })
    });
    let Some(new_set) = new_set else {
        return;
    };
    let added = annotations.len();
    state.snapshot_annotations();
    let mut removed = 0;
    let mut remaining_ids: Vec<AnnotationId> = Vec::new();
    state.annotations.store().update(|store| {
        let set = store.entry_or_insert_with(file_id, || new_set);
        removed =
            remove_annotations_where(&mut set.annotations, |a| summary::has_auto_tag(a, &tag));
        set.annotations.extend(annotations);
        remaining_ids = set.annotations.iter().map(|a| a.id.clone()).collect();
    });
    if removed > 0 {
        state
            .annotations
            .selected_ids()
            .update(|ids| ids.retain(|id| remaining_ids.contains(id)));
    }
    state.annotations.dirty().set(true);
    state.annotations.visible().set(true);
    let mut toast = format!("{} added", count(added, "annotation", "annotations"));
    if removed > 0 {
        toast.push_str(&format!(
            ". {} removed.",
            count(removed, "previous annotation", "previous annotations")
        ));
    }
    state.show_info_toast(toast);
}

/// One species row of the "In this recording" list.
#[derive(Clone, PartialEq)]
struct SpeciesListRow {
    class: u16,
    name: String,
    scientific: String,
    label: Option<classifier::SpeciesLabel>,
    slot: Option<usize>,
    /// Windows, or the time when windows overlap.
    amount: String,
    best: f32,
    /// The best window, which a press on the row shows.
    start: f64,
    end: f64,
}

impl SpeciesListRow {
    /// Everything the row shows, so a row is drawn again only when it
    /// changes.
    fn key(&self) -> (u16, Option<usize>, String, u32, u64) {
        (
            self.class,
            self.slot,
            self.amount.clone(),
            self.best.to_bits(),
            self.start.to_bits(),
        )
    }
}

/// A species picked for a row above the spectrogram that is not among this
/// recording's species at or above the minimum confidence.
#[derive(Clone, PartialEq)]
struct MissingRow {
    scientific: String,
    name: String,
    label: Option<classifier::SpeciesLabel>,
    /// The model has the species among its classes.
    in_model: bool,
}

#[component]
pub(super) fn ClassifierSection(method: Method) -> impl IntoView {
    let state = expect_context::<AppState>();
    let bats = method == Method::Bats;
    // The BattyBirdNET region in use: the one picked, or Auto's for this file.
    let bat_scope = Memo::new(move |_| classifier::bat_pick::bat_scope(state));
    let choice = move || match method {
        Method::BirdNet => ModelChoice::BirdNet,
        Method::Bats => ModelChoice::Bats(bat_scope.get()),
        Method::Perch => ModelChoice::Perch,
        Method::BirdNetV3 => ModelChoice::BirdNetV3,
    };

    // Which model files are on this device; checked once per opening.
    classifier::refresh_downloads(state);

    // Merging neighbouring windows suits bats: one pass is many 0.56 s windows.
    let merge = RwSignal::new(bats);
    let show_credits = RwSignal::new(false);
    let map_open = RwSignal::new(false);
    let settings_open = RwSignal::new(false);

    let current_file = Memo::new(move |_| state.current_file_id_tracked());
    let source_rate = Memo::new(move |_| {
        let idx = state.library.current_index().get();
        state.library.files().with(|files| {
            idx.and_then(|i| files.get(i))
                .map(|f| (f.audio.sample_rate, f.audio.duration_secs))
        })
    });
    let model_size: u64 = model_files(method).iter().map(|f| f.size).sum();
    let backbone = Memo::new(move |_| {
        state
            .classifier
            .downloads()
            .with(|m| combined_status(m, &model_files(method)))
    });
    // The large files the action stands for: the model's, and with bird
    // boxes on (BirdNET+ V3.0) BirdBox's model too.
    let action_files = move || {
        let mut files = model_files(method);
        let boxes = method == Method::BirdNetV3
            && state.call_detection.bird_boxes().get()
            && current_file
                .get()
                .is_some_and(|f| classifier::finds_boxes(state, f, ModelChoice::BirdNetV3));
        if boxes {
            files.push(&catalog::JUST_BIRD);
        }
        files
    };
    let action_status = Memo::new(move |_| {
        let files = action_files();
        state
            .classifier
            .downloads()
            .with(|m| combined_status(m, &files))
    });
    // This file's results for this method.
    let results_info = Memo::new(move |_| {
        let file = current_file.get()?;
        state.classifier.results().with(|m| {
            m.get(&(file, method)).map(|r| RunInfo {
                windows: r.windows.len(),
                analysed: r.analysed_windows(),
                covered_secs: classifier::types::cells_secs(r.cells().iter()),
                complete: r.complete,
                whole_file: r.whole_file,
                region: r.region,
                window_secs: r.window_secs,
                step_secs: r.step_secs,
                spacing_secs: r.spacing_secs,
            })
        })
    });
    // An Analyse has run on this file (readings at the play position also
    // go into the results, so having results is not enough).
    let analysed = Memo::new(move |_| results_info.get().is_some_and(|r| r.analysed > 0));
    // The Step picked for the next Analyse (saved for every classifier).
    let step_pick = RwSignal::new(crate::settings::get_classifier_step());
    let min_conf = move || classifier::min_confidence(state, method);
    let summary = Memo::new(move |_| {
        let file = current_file.get()?;
        let min = min_conf();
        state
            .classifier
            .results()
            .with(|m| m.get(&(file, method)).map(|r| summary::summarise(r, min)))
    });
    let run_status = move || state.classifier.run().get();
    let busy_here = move || {
        let file = current_file.get();
        match run_status() {
            RunStatus::LoadingModel { file_id, method: m }
            | RunStatus::Running {
                file_id, method: m, ..
            } => Some(file_id) == file && m == method,
            RunStatus::Rescoring { file_id, .. } => Some(file_id) == file && bats,
            _ => false,
        }
    };

    // A new region relabels this file's bat results from the stored
    // embeddings; the backbone does not run again.
    if bats {
        Effect::new(move |_| {
            let region = bat_scope.get();
            let Some(file) = current_file.get() else {
                return;
            };
            let Some(RunInfo {
                windows: n,
                region: Some(have),
                ..
            }) = results_info.get()
            else {
                return;
            };
            if n > 0 && have != region && !state.classifier.run().with_untracked(|r| r.is_busy()) {
                state.call_detection.selected_window().set(None);
                if let Err(error) = classifier::rescore(state, file, region) {
                    state.classifier.run().set(RunStatus::Failed {
                        file_id: file,
                        method,
                        error,
                    });
                }
            }
        });
    }

    // ── 1. The action ────────────────────────────────────────────────────
    // Download, Analyse, or progress; the settings button stays beside it.
    let primary = move || {
        let Some(file_id) = current_file.get() else {
            return view! { <div class="cd-no-file">"No file selected"</div> }.into_any();
        };
        // What a download fetches: the files the run needs that are not on
        // this device (with bird boxes on, BirdBox's model too).
        state.call_detection.bird_boxes().track();
        let missing = move || {
            let files = classifier::run_files(state, file_id, choice());
            state.classifier.downloads().with_untracked(|m| {
                files
                    .into_iter()
                    .filter(|f| m.get(f.id) != Some(&DownloadStatus::Downloaded))
                    .collect::<Vec<_>>()
            })
        };
        let total_size: u64 = missing().iter().map(|f| f.size).sum();
        let download = move |_| {
            let files = missing();
            spawn_local(async move {
                let _ = classifier::ensure_files(state, files).await;
            });
        };
        let shown_size: u64 = action_files().iter().map(|f| f.size).sum();
        match action_status.get() {
            DownloadStatus::Unknown => {
                return view! { <div class="detection-status"></div> }.into_any();
            }
            DownloadStatus::NotDownloaded => {
                return view! {
                    <button
                        class="sidebar-btn detection-primary-btn"
                        on:click=download
                    >{format!("Download model ({})", mb(total_size))}</button>
                    <div class="detection-status-note">"Runs on this device. Recordings are not uploaded."</div>
                }
                .into_any();
            }
            DownloadStatus::Downloading { fraction } => {
                let done = (fraction * shown_size as f64 / MB).round();
                let pct = fraction * 100.0;
                return view! {
                    <div class="cd-progress-text">{format!(
                        "Downloading model: {done:.0} of {:.0} MB",
                        shown_size as f64 / MB
                    )}</div>
                    <div class="detection-progress">
                        <div class="detection-progress-fill" style=format!("width:{pct:.1}%")></div>
                    </div>
                }
                .into_any();
            }
            DownloadStatus::Failed(e) => {
                return view! {
                    <div class="detection-error">{format!("Download failed. {e}")}</div>
                    <button
                        class="sidebar-btn detection-primary-btn"
                        on:click=download
                    >{format!("Retry download ({})", mb(total_size))}</button>
                }
                .into_any();
            }
            DownloadStatus::Downloaded => {}
        }

        // Bats: the sample rate must reach 96 kHz.
        if let (Some(min), Some((rate, _))) = (method.min_source_rate(), source_rate.get()) {
            if rate < min {
                return view! {
                    <div class="detection-error">{format!(
                        "Sample rate too low: {} kHz. BattyBirdNET needs a sample rate of {} kHz or more.",
                        rate as f64 / 1000.0,
                        min / 1000
                    )}</div>
                }
                .into_any();
            }
        }

        if busy_here() {
            let (text, pct) = match run_status() {
                RunStatus::LoadingModel { .. } => (
                    bird_boxes::download_text(state)
                        .unwrap_or_else(|| "Loading the model\u{2026}".to_string()),
                    None,
                ),
                RunStatus::Rescoring { scope, .. } => {
                    (format!("Relabelling for {}\u{2026}", scope.label()), None)
                }
                // The pass that scores the windows that name the bird boxes.
                RunStatus::Running {
                    fraction,
                    naming_boxes: true,
                    ..
                } => (
                    format!(
                        "Naming bird sound boxes: {:.0}%",
                        (fraction * 100.0).floor()
                    ),
                    Some(fraction * 100.0),
                ),
                // A later pass of a run with a shorter step: the step it
                // brings the whole recording to.
                RunStatus::Running {
                    fraction,
                    refining: Some(step),
                    ..
                } => (
                    format!(
                        "Refining to a {} step: {:.0}%",
                        summary::step_text(step),
                        (fraction * 100.0).floor()
                    ),
                    Some(fraction * 100.0),
                ),
                RunStatus::Running { fraction, .. } => {
                    // Windows Analyse made: the readings at the play position
                    // kept in the results are not part of the count.
                    let (done, len) = results_info
                        .get()
                        .map(|r| (r.analysed, r.window_secs))
                        .unwrap_or((0, method.window_secs()));
                    let total = source_rate
                        .get()
                        .map(|(_, dur)| expected_windows(dur, len))
                        .unwrap_or(0)
                        .max(done);
                    (
                        format!(
                            "Analysing {} windows: {} of {}",
                            window_len(len),
                            thousands(done),
                            thousands(total)
                        ),
                        Some(fraction * 100.0),
                    )
                }
                _ => (String::new(), None),
            };
            return view! {
                <div class="detection-busy-row">
                    <span>{text}</span>
                    <button class="psd-btn detection-stop-btn" on:click=move |_| classifier::cancel(state)>"Stop"</button>
                </div>
                <div class="detection-progress">
                    {pct.map(|p| view! {
                        <div class="detection-progress-fill" style=format!("width:{p:.1}%")></div>
                    })}
                </div>
            }
            .into_any();
        }

        // The method is named in the drop-down above, so the button says
        // only what it does. After a run it is no longer the main action.
        let first = !analysed.get();
        view! {
            <button
                class=if first { "sidebar-btn detection-primary-btn" } else { "psd-btn cd-again-btn" }
                on:click=move |_| {
                    let step = step_pick.get_untracked().secs(method);
                    analyse(state, file_id, choice(), step)
                }
            >{if first { "Analyse" } else { "Analyse again" }}</button>
        }
        .into_any()
    };
    // Notes about the last run, under the button that makes the next one.
    let action_notes = move || {
        let file_id = current_file.get()?;
        if busy_here() {
            return None;
        }
        let error = match run_status() {
            RunStatus::Failed {
                file_id: f,
                method: m,
                error,
            } if f == file_id && m == method => Some(error),
            _ => None,
        };
        let partial = results_info.get().and_then(|r| {
            if r.complete || r.windows == 0 {
                None
            } else if r.whole_file {
                // Stopped in a later pass of a run with a shorter step.
                let asked = summary::step_text(r.step_secs);
                Some(format!(
                    "Stopped with a {} step, not {asked}. Analyse again for the {asked} step.",
                    summary::step_text(r.spacing_secs)
                ))
            } else if r.analysed == 0 {
                // Only readings at the play position (`classifier::played`):
                // no Analyse has run on this file.
                let length = source_rate.get().map_or(0.0, |(_, d)| d);
                Some(format!(
                    "Results for {} of the {} recording, from the play position. Analyse for the whole recording.",
                    summary::time_amount(r.covered_secs),
                    summary::time_amount(length)
                ))
            } else {
                Some(format!(
                    "Stopped after {}. Analyse again for the whole recording.",
                    count(r.analysed, "window", "windows")
                ))
            }
        });
        (error.is_some() || partial.is_some()).then(|| {
            view! {
                <div class="cd-notes">
                    {error.map(|e: ClassifierError| view! { <div class="detection-error">{e.to_string()}</div> })}
                    {partial.map(|p| view! { <div class="detection-status-note">{p}</div> })}
                </div>
            }
        })
    };

    // The settings in force, in one line under the action.
    let facts = move || {
        let mut f = Vec::new();
        if !summary::step_choices(method).is_empty() {
            f.push(format!(
                "Step {}",
                summary::step_text(step_pick.get().secs(method))
            ));
        }
        f.push(format!("Min confidence {:.2}", min_conf()));
        if !state.call_detection.classifier_boxes().get() {
            f.push("Boxes off".to_string());
        }
        if let Some(p) = place_fact(state, method, current_file.get()) {
            f.push(p);
        }
        if method == Method::BirdNetV3 {
            f.extend(bird_boxes::facts(state));
        }
        f
    };
    // "Use my location" in the Place row: None idle, Some(None) waiting,
    // Some(Some(msg)) failed.
    let locating: RwSignal<Option<Option<String>>> = RwSignal::new(None);
    let has_results = Memo::new(move |_| results_info.get().is_some_and(|r| r.windows > 0));
    let settings = move || {
        settings_open.get().then(|| {
            view! {
                <div class="cd-settings">
                    {step_row(method, step_pick)}
                    <div class="setting-row" title="Lists only windows whose top species has at least this confidence. Applies to the lists, the annotations and the boxes on the spectrogram.">
                        <span class="setting-label">"Minimum confidence"</span>
                        <span class="setting-value">{move || format!("{:.2}", min_conf())}</span>
                    </div>
                    <div class="setting-row">
                        <input
                            type="range"
                            class="setting-range"
                            aria-label="Minimum confidence"
                            min="0.05"
                            max="0.95"
                            step="0.05"
                            prop:value=move || min_conf().to_string()
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                    classifier::set_min_confidence(state, method, v);
                                }
                            }
                        />
                    </div>
                    {move || place_rows(state, method, current_file, locating, has_results)}
                    {(method == Method::BirdNetV3).then(|| bird_boxes::settings_rows(state, current_file))}
                    <div class="setting-row detection-annotate-row">
                        <label class="setting-label">
                            <input
                                type="checkbox"
                                prop:checked=move || state.call_detection.classifier_boxes().get()
                                on:change=move |ev| state.call_detection.classifier_boxes().set(event_target_checked(&ev))
                            />
                            " Boxes on the spectrogram"
                        </label>
                    </div>
                </div>
            }
        })
    };

    // The location list for this file, once it has results and while marking
    // is on (the first time downloads the Geomodel, 16 MB).
    Effect::new(move |_| {
        if !location::applies_to(method) || !state.call_detection.mark_unexpected().get() {
            return;
        }
        // Rerun when "Use my location" gives a place for recordings without one.
        state.classifier.device_position().track();
        if let (Some(file), true) = (current_file.get(), has_results.get()) {
            location::update(state, file);
        }
    });

    // ── 2. At the play position ──────────────────────────────────────────
    let live = matches!(method, Method::BirdNet | Method::Perch | Method::BirdNetV3).then(|| {
        view! { <LiveView method=method current_file=current_file has_results=has_results map_open=map_open /> }
    });

    // ── 3. In this recording ─────────────────────────────────────────────
    let in_recording = move || {
        current_file.get()?;
        Some(in_recording_section(
            state,
            method,
            current_file,
            summary,
            results_info,
            has_results,
            bat_scope,
            step_pick,
        ))
    };

    // ── 4. Folds ─────────────────────────────────────────────────────────
    // "Every window", when any window reaches the minimum confidence.
    let any_listed = Memo::new(move |_| {
        summary.with(|s| {
            s.as_ref()
                .is_some_and(|s| s.species_windows + s.not_species_windows > 0)
        })
    });
    let every_window = move || {
        any_listed.get().then(|| {
            view! {
                <Fold name="windows" title="Every window".to_string()>
                    {window_table(state, method, current_file)}
                </Fold>
            }
        })
    };
    // "Add N annotations": what Analyse found, as annotations.
    let to_add = Memo::new(move |_| {
        let Some(file_id) = current_file.get() else {
            return (0, String::new());
        };
        let min = min_conf();
        let m = merge.get();
        state.classifier.results().with(|r| {
            r.get(&(file_id, method))
                .map(|run| {
                    (
                        summary::spans(run, min, m).len(),
                        summary::auto_tag_value(run),
                    )
                })
                .unwrap_or_default()
        })
    });
    let can_annotate = Memo::new(move |_| {
        let any_species = summary.with(|s| s.as_ref().is_some_and(|s| s.species_windows > 0));
        has_results.get() && any_species
    });
    let annotate = move || {
        if !can_annotate.get() || busy_here() {
            return None;
        }
        let title = Signal::derive(move || match to_add.get().0 {
            0 => "Add annotations".to_string(),
            n => format!("Add {}", count(n, "annotation", "annotations")),
        });
        Some(view! {
            <Fold name="annotate" title=title>
                {annotate_body(state, method, current_file, merge, to_add)}
            </Fold>
        })
    };

    // ── 5. The stored model, and the attribution ─────────────────────────
    let model = move || model_line(state, method, model_size, backbone, show_credits);

    view! {
        <div class="cd-classifier">
            <div class="cd-action-row">
                <div class="cd-action">{primary}</div>
                {settings_button(settings_open)}
            </div>
            {action_notes}
            {move || current_file.get().is_some().then(|| in_force_line(settings_open, facts))}
            {settings}
            {live}
            {in_recording}
            <div class="cd-folds">
                {every_window}
                {annotate}
            </div>
            {model}
            // In a portal on <body>: the sidebar is hidden when closed on
            // phones, and the dialog must not go with it.
            {move || map_open.get().then(|| view! {
                <leptos::portal::Portal>
                    <SpeciesMapDialog open=map_open current_file=current_file method=method />
                </leptos::portal::Portal>
            })}
        </div>
    }
}

/// The "In this recording" section: the header with what was found, notes
/// on the results, the list (species, sound clusters, components) with a pin
/// on each row, the search that adds a row, the selected window's top 3, and
/// the counts of windows left out.
#[allow(clippy::too_many_arguments)]
fn in_recording_section(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
    summary: Memo<Option<summary::Summary>>,
    results_info: Memo<Option<RunInfo>>,
    has_results: Memo<bool>,
    bat_scope: Memo<classifier::BatScope>,
    step_pick: RwSignal<summary::StepPick>,
) -> impl IntoView {
    let min_conf = move || classifier::min_confidence(state, method);
    // "2 species, 1 sound, 16 windows", or "No species".
    let head_fact = move || {
        let s = summary.get()?;
        if s.species.is_empty() {
            return Some("No species".to_string());
        }
        // Named sounds (Thunder, Dog) are listed with the species but
        // counted apart, so "3 species" never includes Thunder.
        let sounds = state.classifier.results().with(|m| {
            current_file
                .get()
                .and_then(|f| m.get(&(f, method)))
                .map_or(0, |r| {
                    s.species
                        .iter()
                        .filter(|row| r.labels.get(row.class as usize).is_some_and(|l| l.is_sound))
                        .count()
                })
        });
        let animals = s.species.len() - sounds;
        let mut parts = Vec::new();
        if animals > 0 {
            parts.push(count(animals, "species", "species"));
        }
        if sounds > 0 {
            parts.push(count(sounds, "sound", "sounds"));
        }
        parts.push(windows_or_time(&s, s.species_windows, s.species_secs));
        Some(parts.join(", "))
    };
    let head_title = move || format!("At confidence {:.2} or above", min_conf());

    // Notes on what the results are: for another BattyBirdNET region (a
    // relabel is going or failed), or made with another step than the one
    // picked now.
    let results_notes = move || {
        let r = results_info.get()?;
        if r.analysed == 0 {
            return None;
        }
        if let Some(region) = r.region.filter(|g| *g != bat_scope.get()) {
            return Some(format!("Results for {}", region.label()));
        }
        if method == Method::BirdNetV3 {
            if let Some(note) = bird_boxes::analyse_again_note(state, current_file.get()) {
                return Some(note.to_string());
            }
        }
        let picked = step_pick.get().secs(method);
        let differs = !summary::step_choices(method).is_empty()
            && (r.spacing_secs - picked).abs() > 1e-6
            && r.complete;
        differs.then(|| {
            format!(
                "Results use a {} step. Analyse again for a {} step.",
                summary::step_text(r.spacing_secs),
                summary::step_text(picked)
            )
        })
    };

    let flag_col = location::applies_to(method);
    // Columns before the pin: name, the flag, time (or windows), best.
    let cols = 3 + usize::from(flag_col);

    // Species rows, worked out in a memo and drawn by key, so readings that
    // change no row (several a second while a file plays) redraw nothing.
    let species_rows = Memo::new(move |_| {
        let min = min_conf();
        state.classifier.results().with(|m| {
            let Some(run) = current_file.get().and_then(|f| m.get(&(f, method))) else {
                return (false, Vec::new());
            };
            let order = summary::colour_order(run);
            let s = summary::summarise(run, min);
            let rows = s
                .species
                .iter()
                .map(|row| {
                    let label = run.labels.get(row.class as usize).cloned();
                    let w = &run.windows[row.best_window];
                    SpeciesListRow {
                        class: row.class,
                        name: class_name(label.as_ref()),
                        scientific: label
                            .as_ref()
                            .map(|l| l.scientific.clone())
                            .unwrap_or_default(),
                        label,
                        slot: order.iter().position(|&c| c == row.class),
                        // Windows that overlap would count the same audio twice.
                        amount: if s.overlapping {
                            summary::time_amount(row.secs)
                        } else {
                            row.windows.to_string()
                        },
                        best: row.best,
                        start: w.start_s,
                        end: w.end_s,
                    }
                })
                .collect::<Vec<_>>();
            (s.overlapping, rows)
        })
    });
    let overlapping = move || species_rows.with(|r| r.0);
    let species_row = move |r: SpeciesListRow| {
        let (start, end) = (r.start, r.end);
        let class = move || {
            if state.call_detection.selected_window().get() == Some(start) {
                "detection-row selected"
            } else {
                "detection-row"
            }
        };
        let flag = flag_cell(
            state,
            method,
            current_file.get_untracked(),
            r.label.as_ref(),
        );
        view! {
            <tr class=class title=r.scientific.clone() on:click=move |_| show_window(state, start, end)>
                <td class="detection-col-kind">
                    <span class="cd-name-line">{species_swatch(r.slot)}<span>{r.name.clone()}</span></span>
                </td>
                {flag}
                <td class="detection-col-num">{r.amount.clone()}</td>
                <td class="detection-col-num">{format!("{:.2}", r.best)}</td>
                {species_pin(state, method, r.scientific.clone(), r.name.clone())}
            </tr>
        }
    };

    // Picked species with no row above: still listed, so they can be
    // unpinned.
    let missing_rows = Memo::new(move |_| {
        let picked: Vec<String> = state.call_detection.strip_species().with(|v| {
            v.iter()
                .filter(|s| s.method == method)
                .map(|s| s.scientific.clone())
                .collect()
        });
        if picked.is_empty() {
            return Vec::new();
        }
        let listed: Vec<String> =
            species_rows.with(|r| r.1.iter().map(|r| r.scientific.clone()).collect());
        state.classifier.results().with(|m| {
            let run = current_file.get().and_then(|f| m.get(&(f, method)));
            picked
                .into_iter()
                .filter(|s| !listed.contains(s))
                .map(|scientific| {
                    let label = run.and_then(|run| {
                        classifier::strips::class_of(run, &scientific)
                            .and_then(|c| run.labels.get(c as usize).cloned())
                    });
                    MissingRow {
                        name: label
                            .as_ref()
                            .map_or_else(|| scientific.clone(), classifier::strips::display_name),
                        // Without results the model's list is not known here.
                        in_model: label.is_some() || run.is_none(),
                        label,
                        scientific,
                    }
                })
                .collect()
        })
    });
    let missing_row = move |r: MissingRow| {
        let flag = flag_cell(
            state,
            method,
            current_file.get_untracked(),
            r.label.as_ref(),
        );
        let cells = if r.in_model {
            view! {
                <td class="detection-col-num cd-missing">"\u{2013}"</td>
                <td class="detection-col-num cd-missing" title="Highest confidence in this recording is below the minimum">
                    {move || format!("< {:.2}", min_conf())}
                </td>
            }
            .into_any()
        } else {
            view! {
                <td class="cd-missing" colspan="2" title="This model's list of classes does not have this species">
                    "Not in this model"
                </td>
            }
            .into_any()
        };
        view! {
            <tr class="detection-row-other" title=r.scientific.clone()>
                <td class="detection-col-kind">
                    <span class="cd-name-line">{species_swatch(None)}<span>{r.name.clone()}</span></span>
                </td>
                {flag}
                {cells}
                {species_pin(state, method, r.scientific.clone(), r.name.clone())}
            </tr>
        }
    };

    // The model's components picked for a row above the spectrogram.
    let component_rows = move || {
        if !has_components(method) {
            return Vec::new();
        }
        let picked = state.call_detection.strip_components().get();
        let shares = crate::components::score_strips::components_for(method)
            .map(|p| p.variance.clone())
            .unwrap_or_default();
        picked
            .into_iter()
            .map(|k| {
                let name = format!("PC{}", k + 1);
                let help = shares.get(k).map_or_else(String::new, |s| {
                    format!("{name}: {:.1}% of the variation between the model's classes", s * 100.0)
                });
                view! {
                    <tr title=help>
                        <td class="detection-col-kind" colspan=cols.to_string()>
                            <span class="cd-name-line">
                                <span class="map-key cd-swatch cd-swatch-pc"></span>
                                <span>{name.clone()}</span>
                            </span>
                        </td>
                        <td class="cd-col-pin">
                            {pin_button(name, move || component_picked(state, k), move || toggle_component(state, k))}
                        </td>
                    </tr>
                }
            })
            .collect::<Vec<_>>()
    };

    let clusters_pin = pin_button(
        "Sound clusters".to_string(),
        move || state.call_detection.sound_groups().get(),
        move || {
            state.call_detection.sound_groups().update(|v| *v = !*v);
            save_strips(state);
        },
    )
    .into_any();

    // The windows left out of the list, and why nothing is listed.
    let notes = move || {
        let s = summary.get()?;
        let min = min_conf();
        let mut notes = Vec::new();
        if s.species.is_empty() {
            notes.push(match s.highest {
                Some((class, conf)) => {
                    let name = state.classifier.results().with(|m| {
                        current_file
                            .get()
                            .and_then(|f| m.get(&(f, method)))
                            .map(|r| class_name(r.labels.get(class as usize)))
                            .unwrap_or_default()
                    });
                    format!(
                        "No species at confidence {min:.2} or above. Highest: {name}, {conf:.2}"
                    )
                }
                None => format!("No species at confidence {min:.2} or above."),
            });
        }
        if s.below_windows > 0 {
            notes.push(format!(
                "Below {min:.2}: {}",
                windows_or_time(&s, s.below_windows, s.below_secs)
            ));
        }
        if s.not_species_windows > 0 {
            notes.push(format!(
                "Noise and other sounds: {}",
                windows_or_time(&s, s.not_species_windows, s.not_species_secs)
            ));
        }
        (!notes.is_empty()).then(|| {
            view! {
                <div class="cd-notes">
                    {notes.into_iter().map(|t| view! { <div class="detection-status-note">{t}</div> }).collect::<Vec<_>>()}
                </div>
            }
        })
    };

    view! {
        <div class="cd-section cd-recording">
            {section_head("In this recording", move || head_fact(), head_title)}
            {move || results_notes().map(|t| view! { <div class="cd-notes"><div class="detection-status-note">{t}</div></div> })}
            {move || (!has_results.get()).then(|| view! {
                <div class="cd-notes"><div class="detection-status-note">"Analyse to list the species found in this recording."</div></div>
            })}
            // Hidden, not removed, before there are results: a row above the
            // spectrogram needs a run, so there is nothing to pin yet.
            <table class="detection-table cd-list" style:display=move || if has_results.get() { "" } else { "none" }>
                <thead>
                    <tr>
                        <th></th>
                        {flag_header(method)}
                        <th class="detection-col-num">{move || if overlapping() { "Time" } else { "Windows" }}</th>
                        <th class="detection-col-num" title="Best confidence">"Best"</th>
                        <th class="cd-col-pin" title="Shown above the spectrogram">
                            <span class="cd-pin-head" aria-label="Shown above the spectrogram"><Icon kind=Icon::Pin /></span>
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {(method == Method::BirdNetV3).then(|| bird_boxes::list_row(state, current_file, cols))}
                    <For each=move || species_rows.with(|r| r.1.clone()) key=SpeciesListRow::key children=species_row />
                    <For each=move || missing_rows.get() key=|r: &MissingRow| (r.scientific.clone(), r.in_model) children=missing_row />
                    {crate::components::sound_groups::list_rows(state, method, current_file, save_strips, cols, clusters_pin)}
                    {component_rows}
                </tbody>
            </table>
            {strip_search(state, method, current_file)}
            {top_three(state, method, current_file)}
            {notes}
        </div>
    }
}

/// The inside of the "Add N annotations" fold: one per window or merged,
/// what adding replaces, and the button.
fn annotate_body(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
    merge: RwSignal<bool>,
    to_add: Memo<(usize, String)>,
) -> impl IntoView {
    let note = move || {
        let file_id = current_file.get()?;
        let (n, tag) = to_add.get();
        let existing = state.annotations.store().with(|store| {
            store
                .get(file_id)
                .map(|set| {
                    set.annotations
                        .iter()
                        .filter(|a| summary::has_auto_tag(a, &tag))
                        .count()
                })
                .unwrap_or(0)
        });
        (existing > 0 && n > 0).then(|| {
            let text = format!(
                "Replaces {} tagged #auto={tag}, including any you edited. Undo with {}.",
                count(existing, "annotation", "annotations"),
                undo_keys()
            );
            view! { <div class="detection-status-note detection-annotate-row">{text}</div> }
        })
    };
    let option = move |value: bool, label: &'static str| {
        view! {
            <label class="setting-label detection-radio">
                <input
                    type="radio"
                    name="classifier-annotate-merge"
                    prop:checked=move || merge.get() == value
                    on:change=move |_| merge.set(value)
                />
                {label}
            </label>
        }
    };
    view! {
        <div class="detection-annotate-row detection-radio-group">
            {option(false, "One per window")}
            {option(true, "Merge neighbours")}
        </div>
        {note}
        <button
            class="psd-btn cd-add-btn"
            disabled=move || to_add.get().0 == 0
            on:click=move |_| {
                if let Some(file_id) = current_file.get_untracked() {
                    add_annotations(state, file_id, method, merge.get_untracked());
                }
            }
        >{move || match to_add.get().0 {
            0 => "Add annotations".to_string(),
            n => format!("Add {}", count(n, "annotation", "annotations")),
        }}</button>
    }
}

/// The line about the stored model (its size and Delete) and the
/// attribution the model's licence asks for, with Credits.
fn model_line(
    state: AppState,
    method: Method,
    model_size: u64,
    backbone: Memo<DownloadStatus>,
    show_credits: RwSignal<bool>,
) -> impl IntoView {
    let downloaded = backbone.get() == DownloadStatus::Downloaded;
    let a = catalog::attribution(method);
    let (byline, cant_run) = match method {
        Method::BirdNet => ("Powered by BirdNET V2.4.", "BirdNET and BattyBirdNET"),
        Method::Bats => (
            "BattyBirdNET by rdz-oss, powered by BirdNET.",
            "BirdNET and BattyBirdNET",
        ),
        Method::Perch => ("Perch 2.0 by Google DeepMind.", "Perch"),
        Method::BirdNetV3 => ("Powered by BirdNET+ V3.0.", "BirdNET+ V3.0"),
    };
    // BirdNET's backbone is shared with BattyBirdNET.
    let shared = matches!(method, Method::BirdNet | Method::Bats);
    // BirdBox's model, beside BirdNET+ V3.0: its size, Delete and its
    // licence (AGPL-3.0) go on the same lines.
    let v3 = method == Method::BirdNetV3;
    let boxes_stored = v3 && bird_boxes::model_stored(state);
    let boxes_credited = v3 && (boxes_stored || state.call_detection.bird_boxes().get());
    let boxes_size = catalog::JUST_BIRD.size;
    let b = catalog::JUST_BIRD_ATTRIBUTION;
    // The link text is the site, e.g. "birdnet.cornell.edu".
    let site = a
        .powered_by_url
        .trim_start_matches("https://")
        .trim_start_matches("www.")
        .trim_end_matches('/');
    view! {
        <div class="cd-model">
            {downloaded.then(|| view! {
                <div>
                    // Deleting BirdNET's backbone stops BattyBirdNET too, so
                    // the line says so where Delete is.
                    {if shared {
                        format!("Model on this device, {}, used by BirdNET and BattyBirdNET. ", mb(model_size))
                    } else if boxes_stored {
                        format!("Models on this device, {} and {}. ", mb(model_size), mb(boxes_size))
                    } else {
                        format!("Model on this device, {}. ", mb(model_size))
                    }}
                    <button
                        class="detection-link-btn"
                        on:click=move |_| {
                            let question = if boxes_stored {
                                format!(
                                    "Delete BirdNET+ V3.0 ({}) and BirdBox Just-Bird ({}) from this device? Neither model can run until it is downloaded again. Results already shown stay.",
                                    mb(model_size),
                                    mb(boxes_size)
                                )
                            } else {
                                format!(
                                    "Delete the model ({}) from this device? {cant_run} can't run until it is downloaded again. Results already shown stay.",
                                    mb(model_size)
                                )
                            };
                            let ok = web_sys::window()
                                .and_then(|w| w.confirm_with_message(&question).ok())
                                .unwrap_or(false);
                            if ok {
                                classifier::cancel(state);
                                let mut files = deletable_files(method);
                                if boxes_stored {
                                    files.push(&catalog::JUST_BIRD);
                                }
                                classifier::delete_files(state, files);
                            }
                        }
                    >"Delete"</button>
                </div>
            })}
            <div>
                {byline}" "
                <a href=a.licence_url target="_blank" rel="noopener">{a.licence}</a>
                ". "
                {boxes_credited.then(|| view! {
                    "Boxes by BirdBox Just-Bird. "
                    <a href=b.licence_url target="_blank" rel="noopener">{b.licence}</a>
                    ". "
                })}
                <button
                    class="detection-link-btn"
                    aria-expanded=move || show_credits.get().to_string()
                    on:click=move |_| show_credits.update(|v| *v = !*v)
                >"Credits"</button>
            </div>
            {move || show_credits.get().then(|| view! {
                <div class="detection-credits">
                    {a.credits.iter().map(|c| view! { <p>{*c}</p> }).collect::<Vec<_>>()}
                    <p>{a.citation}</p>
                    <p>
                        <a href=a.powered_by_url target="_blank" rel="noopener">{site}</a>
                    </p>
                    {boxes_credited.then(|| view! {
                        {b.credits.iter().map(|c| view! { <p>{*c}</p> }).collect::<Vec<_>>()}
                        <p>{b.citation}</p>
                        <p>
                            <a href=b.source_url target="_blank" rel="noopener">"github.com/birdnet-team/BirdBox"</a>
                            " "
                            <a href=b.licence_url target="_blank" rel="noopener">"AGPL-3.0 licence text"</a>
                        </p>
                    })}
                </div>
            })}
        </div>
    }
}

/// The flag beside a bird the location list says is unlikely at the
/// recording's place and month. One mark for every place a result is named
/// (the lists, the top 3, the spectrogram's label strip), so it costs one
/// character of width and the name stays on one line.
pub(super) fn unlikely_flag() -> impl IntoView {
    view! {
        <span
            class="unlikely-flag"
            role="img"
            aria-label="Unlikely at this place and month"
            title="Unlikely at this place and month"
        >
            <Icon kind=Icon::Flag />
        </span>
    }
}

/// The flag column of a results table: a narrow cell after the name, present
/// for the methods the location list applies to. A column of its own, so
/// turning "Mark birds unlikely here" on or off never re-wraps a name or
/// changes a row's height. Only the cell's content follows the setting; the
/// tables are not rebuilt.
fn flag_cell(
    state: AppState,
    method: Method,
    file: Option<u64>,
    label: Option<&classifier::SpeciesLabel>,
) -> Option<impl IntoView> {
    if !location::applies_to(method) {
        return None;
    }
    let label = label.cloned();
    Some(view! {
        <td class="detection-col-flag">
            {move || {
                file.zip(label.as_ref())
                    .is_some_and(|(f, l)| location::unlikely_here(state, f, method, l))
                    .then(unlikely_flag)
            }}
        </td>
    })
}

/// The header cell above [`flag_cell`].
fn flag_header(method: Method) -> Option<impl IntoView> {
    location::applies_to(method).then(|| view! { <th class="detection-col-flag"></th> })
}

/// How many of the species at or above the minimum confidence are birds
/// unlikely at the file's place and month. Reactive.
fn unlikely_species_count(state: AppState, file: u64, method: Method) -> usize {
    let min = classifier::min_confidence(state, method);
    state.classifier.results().with(|m| {
        m.get(&(file, method)).map_or(0, |run| {
            summary::summarise(run, min)
                .species
                .iter()
                .filter(|row| {
                    run.labels
                        .get(row.class as usize)
                        .is_some_and(|l| location::unlikely_here(state, file, method, l))
                })
                .count()
        })
    })
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The month of a location-list week (1 to 48), or `None` for any month (0).
fn month_of(week: u32) -> Option<&'static str> {
    (week > 0).then(|| MONTHS[((week - 1) / 4).min(11) as usize])
}

/// For the line of settings in force: the place and month birds are marked
/// unlikely for ("Antarctica in January", "Your location in June"), while
/// "Mark birds unlikely here" is on. Reactive.
fn place_fact(state: AppState, method: Method, file: Option<u64>) -> Option<String> {
    if !location::applies_to(method) || !state.call_detection.mark_unexpected().get() {
        return None;
    }
    let file = file?;
    let (own, country, week) = state.library.files().with(|fs| {
        fs.iter().find(|f| f.id == file).map(|f| {
            (
                place::file_place(f),
                place::recording_place(f).map(|p| p.country),
                place::file_week(f),
            )
        })
    })?;
    let device = state.classifier.device_position().get();
    let where_ = match (own, device) {
        (Some(p), _) => country.unwrap_or_else(|| format!("{:.2}, {:.2}", p.lat, p.lon)),
        (None, Some(_)) => "Your location".to_string(),
        // No place: no bird is marked, so nothing about it is in force.
        (None, None) => return None,
    };
    Some(match month_of(week) {
        Some(m) => format!("{where_} in {m}"),
        None => format!("{where_}, any month"),
    })
}

/// The Place row and "Mark birds unlikely here", in the settings. The place
/// is the recording's own GPS, or else this device's position once the user
/// asks for it with "Use my location" (kept in memory for this session).
fn place_rows(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
    locating: RwSignal<Option<Option<String>>>,
    has_results: Memo<bool>,
) -> Option<AnyView> {
    if !location::applies_to(method) {
        return None;
    }
    let file = current_file.get()?;
    let (own, week) = state.library.files().with(|fs| {
        fs.iter()
            .find(|f| f.id == file)
            .map(|f| (place::file_place(f), place::file_week(f)))
    })?;
    let device = state.classifier.device_position().get();
    let place = own.or_else(|| device.map(|(lat, lon)| place::Place { lat, lon, week }));
    let error = move || {
        locating
            .get()
            .flatten()
            .map(|msg| view! { <div class="setting-row setting-error" role="alert">{msg}</div> })
    };
    // Asks for this device's position: "Use my location", or "Update" once
    // it is known.
    let on_locate = move |_| {
        if locating.get_untracked() == Some(None) {
            return;
        }
        locating.set(Some(None));
        spawn_local(async move {
            use crate::device_location::{ask, LocationError};
            let msg = match ask(state).await {
                Ok(_) => None,
                Err(LocationError::Denied) => Some("Location permission denied".to_string()),
                Err(LocationError::Unavailable(m)) => Some(format!("Location unavailable: {m}")),
            };
            locating.set(msg.map(Some));
        });
    };
    // Beside the checkbox, so turning it on or off changes this one value
    // and moves nothing: blank when off or before there are results, then
    // "Checking..." until the location list is worked out, then how many
    // listed species are flagged.
    // Without a place there is no location list to wait for.
    let has_place = place.is_some();
    let marked = move || {
        if !state.call_detection.mark_unexpected().get() || !has_results.get() || !has_place {
            return String::new();
        }
        let checking = state
            .classifier
            .occurrence()
            .with(|m| m.get(&file).is_none_or(|o| o.scores.is_none()));
        if checking {
            "Checking\u{2026}".to_string()
        } else {
            count(
                unlikely_species_count(state, file, method),
                "species",
                "species",
            )
        }
    };
    let mark_row = view! {
        <div class="setting-row detection-annotate-row">
            <label class="setting-label">
                <input
                    type="checkbox"
                    prop:checked=move || state.call_detection.mark_unexpected().get()
                    on:change=move |ev| state.call_detection.mark_unexpected().set(event_target_checked(&ev))
                />
                " Mark birds unlikely here"
                {unlikely_flag()}
            </label>
            <span class="setting-value detection-marked-count">{marked}</span>
        </div>
    };
    let Some(place) = place else {
        return Some(
            view! {
                <div class="setting-row">
                    <span class="setting-label">"Place"</span>
                    <span class="setting-value">"No location data"</span>
                </div>
                <div class="setting-row">
                    <button
                        class="setting-button"
                        disabled=move || locating.get() == Some(None)
                        on:click=on_locate
                    >
                        {move || if locating.get() == Some(None) {
                            "Getting location\u{2026}"
                        } else {
                            "Use my location"
                        }}
                    </button>
                </div>
                {error}
                {mark_row}
            }
            .into_any(),
        );
    };
    let from_device = own.is_none();
    let month = month_of(place.week).unwrap_or("any month");
    let value = format!("{:.2}, {:.2} \u{b7} {month}", place.lat, place.lon);
    Some(
        view! {
            <div
                class="setting-row"
                title=if from_device { PLACE_HELP_DEVICE } else { PLACE_HELP_OWN }
            >
                <span class="setting-label">"Place"</span>
                <span class="setting-value">
                    {from_device.then(|| view! {
                        <span class="place-source">
                            "Your location "
                            <button
                                class="detection-link-btn"
                                title="Get this device's location again"
                                disabled=move || locating.get() == Some(None)
                                on:click=on_locate
                            >
                                {move || if locating.get() == Some(None) {
                                    "Getting location\u{2026}"
                                } else {
                                    "Update"
                                }}
                            </button>
                        </span>
                    })}
                    {value}
                </span>
            </div>
            {error}
            {mark_row}
        }
        .into_any(),
    )
}

const PLACE_HELP_OWN: &str = "From the recording's metadata. Birds the BirdNET Geomodel scores below 0.01 at this place and month are marked unlikely here. The place stays on this device.";
const PLACE_HELP_DEVICE: &str = "Your location, from this device, used for recordings without location data. Birds the BirdNET Geomodel scores below 0.01 at this place and month are marked unlikely here. Your location stays on this device and is not saved.";

/// One row of the "Every window" list.
#[derive(Clone, PartialEq)]
struct WindowRow {
    /// The window's number, when windows do not overlap.
    number: Option<usize>,
    start: f64,
    end: f64,
    label: Option<classifier::SpeciesLabel>,
    /// The species' place in `summary::colour_order`; `None` for a class
    /// that is not a species.
    slot: Option<usize>,
    other: bool,
    confidence: f32,
}

impl WindowRow {
    /// Everything the row shows, so a row is drawn again only when it changes.
    fn key(&self) -> (u64, Option<usize>, Option<usize>, bool, u32, String) {
        (
            self.start.to_bits(),
            self.number,
            self.slot,
            self.other,
            self.confidence.to_bits(),
            self.label
                .as_ref()
                .map_or_else(String::new, |l| l.scientific.clone()),
        )
    }
}

/// The "Every window" list: windows at or above the minimum, in time order.
/// The rows are worked out in a memo and drawn by key, so readings that add
/// no row (below the minimum) redraw nothing, and a new row adds one row.
fn window_table(state: AppState, method: Method, current_file: Memo<Option<u64>>) -> impl IntoView {
    let rows = Memo::new(move |_| {
        let min = classifier::min_confidence(state, method);
        state.classifier.results().with(|m| {
            let Some(run) = current_file.get().and_then(|f| m.get(&(f, method))) else {
                return (true, Vec::new());
            };
            // With overlapping windows the numbers change as a run adds
            // windows between others, so the column is left out.
            let numbered = !run.overlapping();
            let order = summary::colour_order(run);
            let rows = run
                .windows
                .iter()
                .enumerate()
                .filter_map(|(i, w)| {
                    let kind = summary::window_kind(run, i, min);
                    if kind == WindowKind::Below {
                        return None;
                    }
                    let top = w.top[0];
                    let other = kind == WindowKind::NotSpecies;
                    Some(WindowRow {
                        number: numbered.then_some(i + 1),
                        start: w.start_s,
                        end: w.end_s,
                        label: run.label(&top).cloned(),
                        slot: (!other)
                            .then(|| order.iter().position(|&c| c == top.class))
                            .flatten(),
                        other,
                        confidence: top.confidence,
                    })
                })
                .collect::<Vec<_>>();
            (numbered, rows)
        })
    });
    let numbered = move || rows.with(|r| r.0);
    let row = move |r: WindowRow| {
        let (start, end, other) = (r.start, r.end, r.other);
        let class = move || {
            let mut c = String::from("detection-row");
            if other {
                c.push_str(" detection-row-other");
            }
            if state.call_detection.selected_window().get() == Some(start) {
                c.push_str(" selected");
            }
            c
        };
        let scientific = r
            .label
            .as_ref()
            .map(|l| l.scientific.clone())
            .unwrap_or_default();
        let flag = flag_cell(
            state,
            method,
            current_file.get_untracked(),
            r.label.as_ref(),
        );
        view! {
            <tr class=class title=scientific on:click=move |_| show_window(state, start, end)>
                {r.number.map(|n| view! { <td class="detection-col-num">{n}</td> })}
                <td>{crate::format_time::format_time_display(start, 1)}</td>
                <td class="detection-col-kind">
                    <span class="cd-name-line">{species_swatch(r.slot)}<span>{class_name(r.label.as_ref())}</span></span>
                </td>
                {flag}
                <td class="detection-col-num">{format!("{:.2}", r.confidence)}</td>
            </tr>
        }
    };
    view! {
        <div class="detection-list">
            <table class="detection-table">
                <thead>
                    <tr>
                        {move || numbered().then(|| view! { <th class="detection-col-num">"#"</th> })}
                        <th>"Start"</th>
                        <th>"Species"</th>
                        {flag_header(method)}
                        <th class="detection-col-num" title="Confidence">"Conf."</th>
                    </tr>
                </thead>
                <tbody>
                    <For each=move || rows.with(|r| r.1.clone()) key=WindowRow::key children=row />
                </tbody>
            </table>
        </div>
    }
}

/// The selected window's top 3 classes, as the data they are drawn from, so
/// the table is drawn again only when it changes (not on every reading).
#[derive(Clone, PartialEq)]
struct TopThree {
    /// "Top 3 in window 4 (9.0s to 12.0s)"
    title: String,
    rows: Vec<(Option<classifier::SpeciesLabel>, f32)>,
}

/// The selected window's top 3 classes, under the list.
fn top_three(state: AppState, method: Method, current_file: Memo<Option<u64>>) -> impl IntoView {
    let data = Memo::new(move |_| {
        let start = state.call_detection.selected_window().get()?;
        state.classifier.results().with(|m| {
            let run = current_file.get().and_then(|f| m.get(&(f, method)))?;
            let i = run.window_at_start(start)?;
            let w = run.windows.get(i)?;
            let (a, b) = (
                crate::format_time::format_time_display(w.start_s, 1),
                crate::format_time::format_time_display(w.end_s, 1),
            );
            // With overlapping windows the numbers change as a run adds
            // windows between others, so only the times are given.
            let title = if run.overlapping() {
                format!("Top 3 in window {a} to {b}")
            } else {
                format!("Top 3 in window {} ({a} to {b})", i + 1)
            };
            Some(TopThree {
                title,
                rows: w
                    .top
                    .iter()
                    .take(3)
                    .map(|s| (run.label(s).cloned(), s.confidence))
                    .collect(),
            })
        })
    });
    move || {
        let t = data.get()?;
        let rows = t
            .rows
            .into_iter()
            .map(|(label, conf)| {
                let scientific = label
                    .as_ref()
                    .filter(|l| l.is_species && l.scientific != l.common)
                    .map(|l| l.scientific.clone());
                let flag = flag_cell(state, method, current_file.get_untracked(), label.as_ref());
                // BattyBirdNET's "Noise" and the like get no row above.
                let pin = match label.as_ref().filter(|l| l.is_species) {
                    Some(l) => {
                        species_pin(state, method, l.scientific.clone(), class_name(Some(l)))
                            .into_any()
                    }
                    None => view! { <td class="cd-col-pin"></td> }.into_any(),
                };
                view! {
                    <tr>
                        <td class="detection-col-kind">
                            {class_name(label.as_ref())}
                            {scientific.map(|s| view! { <div class="detection-scientific">{s}</div> })}
                        </td>
                        {flag}
                        <td class="detection-col-num">{format!("{conf:.2}")}</td>
                        {pin}
                    </tr>
                }
            })
            .collect::<Vec<_>>();
        Some(view! {
            <div class="detection-top3">
                <div class="detection-status-note">{t.title}</div>
                <table class="detection-table">
                    <tbody>{rows}</tbody>
                </table>
            </div>
        })
    }
}

/// Most matches the search lists at once.
const SEARCH_MATCHES: usize = 8;

/// A run's label list, compared by identity: a memo of it changes only when
/// another run's labels come in, not each time a window's scores do.
#[derive(Clone)]
struct LabelList(std::sync::Arc<Vec<classifier::SpeciesLabel>>);

impl LabelList {
    fn key(&self) -> usize {
        std::sync::Arc::as_ptr(&self.0) as usize
    }
}

/// Each label's common and scientific name, folded for matching
/// (`classifier::strips::fold_labels`).
type FoldedNames = std::sync::Arc<Vec<(String, String)>>;

/// True for the methods whose components can be rows above the spectrogram:
/// BirdNET V2.4 and BirdNET+ V3.0.
fn has_components(method: Method) -> bool {
    matches!(method, Method::BirdNet | Method::BirdNetV3)
}

/// One row the search offers: a species, or one of the model's
/// components.
#[derive(Clone, PartialEq)]
struct SearchMatch {
    name: String,
    /// Empty for a component.
    scientific: String,
    component: Option<usize>,
    added: bool,
}

/// The components a query asks for: "pc" lists them all, "pc1" those whose
/// number starts with 1. Empty for any other query.
fn component_matches(query: &str, n: usize) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let Some(rest) = q.strip_prefix("pc") else {
        return Vec::new();
    };
    let rest = rest.trim();
    if !rest.chars().all(|c| c.is_ascii_digit()) {
        return Vec::new();
    }
    (0..n)
        .filter(|k| (k + 1).to_string().starts_with(rest))
        .collect()
}

/// The last row of the list: type part of a common or scientific name (or
/// "PC" for a model's components), then pick a match (or press Enter
/// for the highlighted one) to add its row above the spectrogram. Searches
/// every class of the current file's run for this method, so a species the
/// run never scored highly can be added. Matches are listed in the panel
/// under the field rather than in a popup, which the narrow sidebar would
/// clip.
fn strip_search(state: AppState, method: Method, current_file: Memo<Option<u64>>) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let focused = RwSignal::new(false);
    let highlight = RwSignal::new(0usize);
    let with_components = has_components(method);
    let labels = Memo::new_with_compare(
        move |_| {
            let file = current_file.get()?;
            state
                .classifier
                .results()
                .with(|m| m.get(&(file, method)).map(|r| LabelList(r.labels.clone())))
        },
        |a, b| {
            a.map(|a| a.as_ref().map(LabelList::key)) != b.map(|b| b.as_ref().map(LabelList::key))
        },
    );
    // Folded names of the label list, built on the first search of each list
    // (Perch has 14,795 classes): (list key, names).
    let folded: StoredValue<Option<(usize, FoldedNames)>> = StoredValue::new(None);
    let matches = Memo::new(move |_| -> Vec<SearchMatch> {
        let q = query.get();
        if q.trim().is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if with_components {
            let n = crate::components::score_strips::components_for(method)
                .map_or(0, |p| p.variance.len());
            out.extend(component_matches(&q, n).into_iter().map(|k| SearchMatch {
                name: format!("PC{}", k + 1),
                scientific: String::new(),
                component: Some(k),
                added: component_picked(state, k),
            }));
        }
        let Some(list) = labels.get() else {
            return out;
        };
        let cached = folded.with_value(|f| {
            f.as_ref()
                .filter(|(k, _)| *k == list.key())
                .map(|(_, n)| n.clone())
        });
        let names = cached.unwrap_or_else(|| {
            let n = std::sync::Arc::new(classifier::strips::fold_labels(&list.0));
            folded.set_value(Some((list.key(), n.clone())));
            n
        });
        let room = SEARCH_MATCHES.saturating_sub(out.len());
        out.extend(
            classifier::strips::find_classes(&list.0, &names, &q, room)
                .into_iter()
                .filter_map(|c| list.0.get(c as usize))
                .map(|l| SearchMatch {
                    name: classifier::strips::display_name(l),
                    scientific: l.scientific.clone(),
                    component: None,
                    added: strip_picked(state, method, &l.scientific),
                }),
        );
        out
    });
    let open = move || focused.get() && !query.with(|q| q.trim().is_empty());
    let choose = move |m: &SearchMatch| {
        if !m.added {
            match m.component {
                Some(k) => toggle_component(state, k),
                None => toggle_strip(state, method, m.scientific.clone()),
            }
        }
        query.set(String::new());
        highlight.set(0);
    };
    let list_id = format!("strip-search-list-{method:?}");
    let option_id = move |i: usize| format!("strip-search-{method:?}-{i}");
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        let n = matches.with_untracked(Vec::len);
        match ev.key().as_str() {
            "ArrowDown" => {
                ev.prevent_default();
                highlight.update(|h| *h = (*h + 1).min(n.saturating_sub(1)));
            }
            "ArrowUp" => {
                ev.prevent_default();
                highlight.update(|h| *h = h.saturating_sub(1));
            }
            "Enter" => {
                ev.prevent_default();
                let m = matches.with_untracked(|v| v.get(highlight.get_untracked()).cloned());
                if let Some(m) = m {
                    choose(&m);
                }
            }
            "Escape" if !query.with_untracked(String::is_empty) => {
                ev.prevent_default();
                ev.stop_propagation();
                query.set(String::new());
                highlight.set(0);
            }
            _ => {}
        }
    };
    let list_view = {
        let list_id = list_id.clone();
        move || {
            if !open() {
                return None;
            }
            let found = matches.get();
            if found.is_empty() {
                return Some(
                    view! { <div class="strip-search-empty">"No species found"</div> }.into_any(),
                );
            }
            let rows = found
                .into_iter()
                .enumerate()
                .map(|(i, m)| {
                    let name = m.name.clone();
                    let scientific = (!m.scientific.is_empty() && m.scientific != m.name)
                        .then(|| m.scientific.clone());
                    let added = m.added;
                    let class = move || {
                        if highlight.get() == i {
                            "strip-search-option highlight"
                        } else {
                            "strip-search-option"
                        }
                    };
                    view! {
                        <div
                            id=option_id(i)
                            role="option"
                            class=class
                            aria-selected=move || (highlight.get() == i).to_string()
                            on:mousedown=move |ev: web_sys::MouseEvent| {
                                // Keeps the focus in the field, for another search.
                                ev.prevent_default();
                                choose(&m);
                            }
                            on:mousemove=move |_| {
                                if highlight.get_untracked() != i {
                                    highlight.set(i);
                                }
                            }
                        >
                            <span class="strip-search-names">
                                <span>{name}</span>
                                {scientific.map(|s| view! { <span class="detection-scientific">{s}</span> })}
                            </span>
                            {added.then(|| view! { <span class="strip-search-added">"Added"</span> })}
                        </div>
                    }
                })
                .collect::<Vec<_>>();
            Some(
                view! { <div id=list_id.clone() role="listbox" class="strip-search-list">{rows}</div> }
                    .into_any(),
            )
        }
    };
    let placeholder = if with_components {
        "Add a species or component"
    } else {
        "Add a species"
    };
    move || {
        labels
            .with(|l| l.as_ref().is_some_and(|l| !l.0.is_empty()))
            .then(|| {
                let list_id = list_id.clone();
                let list_view = list_view.clone();
                view! {
                    <div class="strip-search">
                        <input
                            type="text"
                            class="strip-search-input"
                            placeholder=placeholder
                            aria-label="Add a row above the spectrogram"
                            role="combobox"
                            aria-autocomplete="list"
                            aria-controls=list_id
                            aria-expanded=move || open().to_string()
                            aria-activedescendant=move || {
                                (open() && matches.with(|v| !v.is_empty()))
                                    .then(|| option_id(highlight.get()))
                            }
                            autocomplete="off"
                            spellcheck="false"
                            prop:value=move || query.get()
                            on:input=move |ev| {
                                query.set(event_target_value(&ev));
                                highlight.set(0);
                            }
                            on:focus=move |_| focused.set(true)
                            on:blur=move |_| focused.set(false)
                            on:keydown=on_keydown
                        />
                        {list_view}
                    </div>
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::component_matches;

    #[test]
    fn components_by_name() {
        assert_eq!(component_matches("PC", 12).len(), 12);
        assert_eq!(component_matches("pc1", 12), vec![0, 9, 10, 11]);
        assert_eq!(component_matches(" pc 3", 12), vec![2]);
        assert!(component_matches("pc1x", 12).is_empty());
        assert!(component_matches("pied", 12).is_empty());
        assert!(component_matches("p", 12).is_empty());
    }
}

//! The BirdNET, BattyBirdNET, Perch and BirdNET+ V3.0 part of the Call
//! detection tab: download
//! the model, analyse the current file, list the species found, and add them as
//! annotations. The calls it makes are in `crate::classifier`.
//!
//! Order on screen: the action (download, analyse, progress), the results
//! (minimum confidence, summary, list, the selected window's top 3), adding
//! annotations, then the stored model and the attribution the model's licence
//! asks for.

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListView {
    BySpecies,
    ByWindow,
}

const MB: f64 = 1_000_000.0;

/// `67 MB`, or `0.1 MB` for a small file.
fn mb(bytes: u64) -> String {
    let m = bytes as f64 / MB;
    if m >= 10.0 {
        format!("{m:.0} MB")
    } else {
        format!("{:.1} MB", m.max(0.1))
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

/// The Step choice under the Analyse button: the time between the starts of
/// windows for the next Analyse, as a group of buttons like the number of
/// Sound groups. Saved for every classifier. None for BattyBirdNET.
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
        <div class="detection-step-row" title=help>
            <span class="setting-label">"Step"</span>
            <div class="psd-btn-group" role="group" aria-label="Step">{buttons}</div>
        </div>
    })
}

/// A small square in the colour the spectrogram's label strip gives this
/// class, so a row in the lists can be matched to its label and call boxes.
/// `slot` is the species' place in `summary::colour_order`; None for classes
/// that are not a species ("Noise"). Inline style: the colour comes from
/// overlays.rs, the shape from the species map legend's `.map-key`.
fn species_swatch(slot: Option<usize>) -> impl IntoView {
    use crate::canvas::spectrogram_renderer::{species_colour, NOT_SPECIES_COLOUR};
    let colour = slot.map_or(NOT_SPECIES_COLOUR, |s| species_colour(s).0);
    view! {
        <span
            class="map-key"
            style=format!(
                "display:inline-block;vertical-align:-1px;margin-right:6px;border-radius:2px;background:{colour}",
            )
        ></span>
    }
}

/// A class name as shown: BattyBirdNET's lower-case classes get a capital.
fn class_name(label: Option<&classifier::SpeciesLabel>) -> String {
    label.map_or_else(|| "Unknown".to_string(), classifier::strips::display_name)
}

/// True when this species has a row above the spectrogram. Reactive.
fn strip_picked(state: AppState, method: Method, scientific: &str) -> bool {
    state.call_detection.strip_species().with(|v| {
        v.iter()
            .any(|s| s.method == method && s.scientific == scientific)
    })
}

/// Add or remove this species' row above the spectrogram.
fn toggle_strip(state: AppState, method: Method, scientific: String) {
    state.call_detection.strip_species().update(|v| {
        classifier::strips::toggle(v, classifier::strips::StripSpecies { method, scientific })
    });
    save_strips(state);
}

/// Save the picked rows (species, components and sound groups) for the next
/// session.
fn save_strips(state: AppState) {
    let cd = state.call_detection;
    crate::settings::set_score_strips(&classifier::strips::StripPicks {
        species: cd.strip_species().get_untracked(),
        components: cd.strip_components().get_untracked(),
        sound_groups: cd.sound_groups().get_untracked(),
        groups: cd.sound_group_count().get_untracked(),
    });
}

/// The cell with a species' "Show" checkbox: adds or removes its row above
/// the spectrogram. A click on it leaves the row's own click (show the
/// window) alone.
fn strip_cell(state: AppState, method: Method, scientific: String, name: String) -> impl IntoView {
    let sci = scientific.clone();
    view! {
        <td class="detection-col-strip" on:click=|ev| ev.stop_propagation()>
            // The label fills the cell, so a tap anywhere in it ticks the box.
            <label class="detection-strip-hit" title=format!("Show {name} scores above the spectrogram")>
                <input
                    type="checkbox"
                    aria-label=format!("Show {name} scores above the spectrogram")
                    prop:checked=move || strip_picked(state, method, &sci)
                    on:change=move |_| toggle_strip(state, method, scientific.clone())
                />
            </label>
        </td>
    }
}

/// The header above [`strip_cell`].
fn strip_header() -> impl IntoView {
    view! {
        <th class="detection-col-strip" title=STRIP_HEADER_HELP>"Show"</th>
    }
}

const STRIP_HEADER_HELP: &str = "Tick a species to show its scores in a row above the spectrogram";

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
    state.call_detection.overlay_enabled().set(true);
    state.call_detection.selected_window().set(None);
    if classifier::is_ready(state, choice) {
        classifier::run(state, file_id, choice, step_secs);
        return;
    }
    let method = choice.method();
    state
        .classifier
        .run()
        .set(RunStatus::LoadingModel { file_id, method });
    spawn_local(async move {
        match classifier::ensure_downloaded(state, choice).await {
            Ok(()) => classifier::run(state, file_id, choice, step_secs),
            Err(error) => state.classifier.run().set(RunStatus::Failed {
                file_id,
                method,
                error,
            }),
        }
    });
}

/// The large files the panel's download button and "Delete model" stand for:
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

/// The files that "Delete model" removes.
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

    let list_view = RwSignal::new(ListView::BySpecies);
    // Merging neighbouring windows suits bats: one pass is many 0.56 s windows.
    let merge = RwSignal::new(bats);
    let show_credits = RwSignal::new(false);
    let map_open = RwSignal::new(false);

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
    // This file's results for this method.
    let results_info = Memo::new(move |_| {
        let file = current_file.get()?;
        state.classifier.results().with(|m| {
            m.get(&(file, method)).map(|r| RunInfo {
                windows: r.windows.len(),
                complete: r.complete,
                whole_file: r.whole_file,
                region: r.region,
                window_secs: r.window_secs,
                step_secs: r.step_secs,
                spacing_secs: r.spacing_secs,
            })
        })
    });
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

    // Download, analyse, or progress.
    let action = move || {
        let Some(file_id) = current_file.get() else {
            return view! { <div class="sidebar-panel-empty">"No file selected"</div> }.into_any();
        };
        let total_size: u64 = choice().files().iter().map(|f| f.size).sum();
        match backbone.get() {
            DownloadStatus::Unknown => {
                return view! { <div class="detection-status"></div> }.into_any();
            }
            DownloadStatus::NotDownloaded => {
                return view! {
                    <div class="detection-status">
                        <button
                            class="sidebar-btn detection-primary-btn"
                            on:click=move |_| classifier::download(state, choice())
                        >{format!("Download model ({})", mb(total_size))}</button>
                        <div class="detection-status-note">"Runs on this device. Recordings are not uploaded."</div>
                    </div>
                }
                .into_any();
            }
            DownloadStatus::Downloading { fraction } => {
                let done = (fraction * model_size as f64 / MB).round();
                let pct = fraction * 100.0;
                return view! {
                    <div class="detection-status">
                        <div>{format!(
                            "Downloading model: {done:.0} of {:.0} MB",
                            model_size as f64 / MB
                        )}</div>
                        <div class="detection-progress">
                            <div class="detection-progress-fill" style=format!("width:{pct:.1}%")></div>
                        </div>
                    </div>
                }
                .into_any();
            }
            DownloadStatus::Failed(e) => {
                return view! {
                    <div class="detection-status">
                        <div class="detection-error">{format!("Download failed. {e}")}</div>
                        <button
                            class="sidebar-btn detection-primary-btn"
                            on:click=move |_| classifier::download(state, choice())
                        >{format!("Retry download ({})", mb(total_size))}</button>
                    </div>
                }
                .into_any();
            }
            DownloadStatus::Downloaded => {}
        }

        // Bats: the sample rate must reach 96 kHz.
        if let (Some(min), Some((rate, _))) = (method.min_source_rate(), source_rate.get()) {
            if rate < min {
                return view! {
                    <div class="detection-status">
                        <div class="detection-error">{format!(
                            "Sample rate too low: {} kHz. BattyBirdNET needs a sample rate of {} kHz or more.",
                            rate as f64 / 1000.0,
                            min / 1000
                        )}</div>
                    </div>
                }
                .into_any();
            }
        }

        let stop = move || {
            view! {
                <button class="psd-btn detection-stop-btn" on:click=move |_| classifier::cancel(state)>"Stop"</button>
            }
        };
        if busy_here() {
            let (text, pct) = match run_status() {
                RunStatus::LoadingModel { .. } => ("Loading the model\u{2026}".to_string(), None),
                RunStatus::Rescoring { scope, .. } => {
                    (format!("Relabelling for {}\u{2026}", scope.label()), None)
                }
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
                    let (done, len) = results_info
                        .get()
                        .map(|r| (r.windows, r.window_secs))
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
                <div class="detection-status">
                    <div class="detection-busy-row">
                        <span>{text}</span>
                        {stop()}
                    </div>
                    <div class="detection-progress">
                        {pct.map(|p| view! {
                            <div class="detection-progress-fill" style=format!("width:{p:.1}%")></div>
                        })}
                    </div>
                </div>
            }
            .into_any();
        }

        let label = match choice() {
            ModelChoice::BirdNet => "Analyse with BirdNET V2.4".to_string(),
            ModelChoice::Bats(r) => format!("Analyse with BattyBirdNET ({})", r.label()),
            ModelChoice::Perch => "Analyse with Perch 2.0".to_string(),
            ModelChoice::BirdNetV3 => "Analyse with BirdNET+ V3.0".to_string(),
        };
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
            } else {
                Some(format!(
                    "Stopped after {}. Analyse again for the whole recording.",
                    count(r.windows, "window", "windows")
                ))
            }
        });
        view! {
            <div class="detection-status">
                {error.map(|e: ClassifierError| view! { <div class="detection-error">{e.to_string()}</div> })}
                {partial.map(|p| view! { <div class="detection-status-note">{p}</div> })}
                <button
                    class="sidebar-btn detection-primary-btn"
                    on:click=move |_| {
                        let step = step_pick.get_untracked().secs(method);
                        analyse(state, file_id, choice(), step)
                    }
                >{label}</button>
                {step_row(method, step_pick)}
            </div>
        }
        .into_any()
    };

    // Minimum confidence, summary, overlay, list, and the selected window.
    // The slider is built once per set of results, so dragging it re-renders
    // only the summary and the list below it.
    let has_results = Memo::new(move |_| results_info.get().is_some_and(|r| r.windows > 0));
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
    // The method and region are picked just above, so the title names the
    // region only while the results are for another one (a relabel is going
    // or failed).
    // For the methods with a Step, the title gives the step between window
    // starts that the shown results have, since the Step above can differ
    // and a run stopped part way can have a longer one.
    let results_title = Memo::new(move |_| {
        let picked = bat_scope.get();
        let info = results_info.get();
        match info.as_ref().and_then(|r| r.region) {
            Some(r) if r != picked => format!("Results for {}", r.label()),
            _ => match info {
                Some(r) if !summary::step_choices(method).is_empty() => {
                    format!("Results ({} step)", summary::step_text(r.spacing_secs))
                }
                _ => "Results".to_string(),
            },
        }
    });
    let headline = move || {
        let s = summary.get()?;
        let min = min_conf();
        let main = if s.species.is_empty() {
            match s.highest {
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
            }
        } else {
            // Named sounds (Thunder, Dog) are listed with the species but
            // counted apart, so "3 species" never includes Thunder.
            let sounds = state.classifier.results().with(|m| {
                current_file
                    .get()
                    .and_then(|f| m.get(&(f, method)))
                    .map_or(0, |r| {
                        s.species
                            .iter()
                            .filter(|row| {
                                r.labels.get(row.class as usize).is_some_and(|l| l.is_sound)
                            })
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
            format!("{min:.2} or above: {}", parts.join(", "))
        };
        let mut notes = Vec::new();
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
        Some(view! {
            <div class="detection-status">
                <div class="detection-summary">{main}</div>
                {notes.into_iter().map(|t| view! { <div class="detection-status-note">{t}</div> }).collect::<Vec<_>>()}
            </div>
        })
    };
    // The lists, when any window reaches the minimum confidence.
    let lists = move || {
        let s = summary.get()?;
        if s.species_windows + s.not_species_windows == 0 {
            return None;
        }
        let view_button = move |value: ListView, label: &'static str| {
            view! {
                <button
                    class=move || if list_view.get() == value { "psd-btn psd-btn-active" } else { "psd-btn" }
                    on:click=move |_| list_view.set(value)
                >{label}</button>
            }
        };
        Some(view! {
            <div class="detection-annotate-row">
                <div class="psd-btn-group">
                    {view_button(ListView::BySpecies, "By species")}
                    {view_button(ListView::ByWindow, "By window")}
                </div>
            </div>
            {move || match list_view.get() {
                ListView::BySpecies => species_table(state, method, current_file).into_any(),
                ListView::ByWindow => window_table(state, method, current_file).into_any(),
            }}
        })
    };
    // "Use my location" in the Place row: None idle, Some(None) waiting,
    // Some(Some(msg)) failed.
    let locating: RwSignal<Option<Option<String>>> = RwSignal::new(None);
    let results = move || {
        if !has_results.get() {
            return None;
        }
        Some(view! {
            <div class="setting-group">
                <div class="setting-group-title">{move || results_title.get()}</div>
                <div class="setting-row" title="Shows only windows whose top species has at least this confidence. Applies to the summary, the list, the annotations and the overlay.">
                    <span class="setting-label">"Minimum confidence"</span>
                    <span class="setting-value">{move || format!("{:.2}", min_conf())}</span>
                </div>
                <div class="setting-row">
                    <input
                        type="range"
                        class="setting-range"
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
                {headline}
                {move || place_rows(state, method, current_file, locating)}
                <div class="setting-row detection-annotate-row">
                    <label class="setting-label">
                        <input
                            type="checkbox"
                            prop:checked=move || state.call_detection.overlay_enabled().get()
                            on:change=move |ev| state.call_detection.overlay_enabled().set(event_target_checked(&ev))
                        />
                        " Show overlay"
                    </label>
                </div>
                {lists}
                {move || top_three(state, method, current_file)}
                // BattyBirdNET's regional classifiers give no usable map
                // (research/species-map/README.md).
                {crate::classifier::species_map::map_spec(method).is_some().then(|| view! {
                    <div class="detection-annotate-row">
                        <button class="psd-btn" on:click=move |_| map_open.set(true)>"Species map"</button>
                    </div>
                })}
                // In a portal on <body>: the sidebar is hidden when closed on
                // phones, and the dialog must not go with it.
                {move || map_open.get().then(|| view! {
                    <leptos::portal::Portal>
                        <SpeciesMapDialog open=map_open current_file=current_file method=method />
                    </leptos::portal::Portal>
                })}
            </div>
        })
    };

    // Rows above the spectrogram: the species picked, and BirdNET's
    // components.
    let strips = move || {
        if !has_results.get() {
            return None;
        }
        Some(strips_group(state, method, current_file))
    };

    // Add as annotations.
    let annotate = move || {
        let file_id = current_file.get()?;
        let n = results_info.get()?.windows;
        let any_species = summary.with(|s| s.as_ref().is_some_and(|s| s.species_windows > 0));
        if n == 0 || !any_species || busy_here() {
            return None;
        }
        let min = min_conf();
        let m = merge.get();
        let (to_add, tag) = state.classifier.results().with(|r| {
            r.get(&(file_id, method))
                .map(|run| {
                    (
                        summary::spans(run, min, m).len(),
                        summary::auto_tag_value(run),
                    )
                })
                .unwrap_or_default()
        });
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
        let note = (existing > 0 && to_add > 0).then(|| {
            format!(
                "Replaces {} tagged #auto={tag}, including any you edited. Undo with {}.",
                count(existing, "annotation", "annotations"),
                undo_keys()
            )
        });
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
        Some(view! {
            <div class="setting-group">
                <div class="setting-group-title">"Add as annotations"</div>
                <div class="detection-annotate-row detection-radio-group">
                    {option(false, "One per window")}
                    {option(true, "Merge neighbours")}
                </div>
                {note.map(|t| view! { <div class="detection-status-note detection-annotate-row">{t}</div> })}
                <button
                    class="sidebar-btn detection-add-btn"
                    disabled=to_add == 0
                    on:click=move |_| add_annotations(state, file_id, method, merge.get_untracked())
                >{if to_add == 0 {
                    "Add annotations".to_string()
                } else {
                    format!("Add {}", count(to_add, "annotation", "annotations"))
                }}</button>
            </div>
        })
    };

    // The stored model, and the attribution.
    let model = move || {
        let downloaded = backbone.get() == DownloadStatus::Downloaded;
        let a = catalog::attribution(method);
        let (byline, used_by, cant_run) = match method {
            Method::BirdNet => (
                "Powered by BirdNET V2.4.",
                " Used by BirdNET and BattyBirdNET.",
                "BirdNET and BattyBirdNET",
            ),
            Method::Bats => (
                "BattyBirdNET by rdz-oss, powered by BirdNET.",
                " Used by BirdNET and BattyBirdNET.",
                "BirdNET and BattyBirdNET",
            ),
            Method::Perch => ("Perch 2.0 by Google DeepMind.", "", "Perch"),
            Method::BirdNetV3 => ("Powered by BirdNET+ V3.0.", "", "BirdNET+ V3.0"),
        };
        // The link text is the site, e.g. "birdnet.cornell.edu".
        let site = a
            .powered_by_url
            .trim_start_matches("https://")
            .trim_start_matches("www.")
            .trim_end_matches('/');
        view! {
            <div class="setting-group">
                <div class="setting-group-title">"Model"</div>
                {downloaded.then(|| view! {
                    <div class="detection-status">
                        <div class="detection-status-note">{format!(
                            "Downloaded: {}.{used_by}",
                            mb(model_size)
                        )}</div>
                    </div>
                    <button
                        class="psd-btn detection-delete-btn"
                        on:click=move |_| {
                            let ok = web_sys::window()
                                .and_then(|w| {
                                    w.confirm_with_message(&format!(
                                        "Delete the model ({}) from this device? {cant_run} can't run until it is downloaded again. Results already shown stay.",
                                        mb(model_size)
                                    ))
                                    .ok()
                                })
                                .unwrap_or(false);
                            if ok {
                                classifier::cancel(state);
                                classifier::delete_files(state, deletable_files(method));
                            }
                        }
                    >{format!("Delete model ({})", mb(model_size))}</button>
                })}
                <div class="detection-status detection-credit">
                    {byline}" "
                    <a href=a.licence_url target="_blank" rel="noopener">{a.licence}</a>
                    ". "
                    <button
                        class="detection-link-btn"
                        aria-expanded=move || show_credits.get().to_string()
                        on:click=move |_| show_credits.update(|v| *v = !*v)
                    >"Credits"</button>
                    {move || show_credits.get().then(|| view! {
                        <div class="detection-credits">
                            {a.credits.iter().map(|c| view! { <p>{*c}</p> }).collect::<Vec<_>>()}
                            <p>{a.citation}</p>
                            <p>
                                <a href=a.powered_by_url target="_blank" rel="noopener">{site}</a>
                            </p>
                        </div>
                    })}
                </div>
            </div>
        }
    };

    view! {
        <div>
            {action}
            {results}
            {strips}
            {annotate}
            {model}
        </div>
    }
}

/// The flag beside a bird the location list says is unlikely at the
/// recording's place and month. One mark for every place a result is named
/// (the lists, the top 3, the spectrogram's label strip), so it costs one
/// character of width and the name stays on one line. The summary line
/// "Unlikely here: 2 species" carries the same flag and so names it.
fn unlikely_flag() -> impl IntoView {
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

/// The Place row and "Mark birds unlikely here". The place is the
/// recording's own GPS, or else this device's position once the user asks for
/// it with "Use my location" (kept in memory for this session).
fn place_rows(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
    locating: RwSignal<Option<Option<String>>>,
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
            }
            .into_any(),
        );
    };
    let from_device = own.is_none();
    let month = match place.week {
        0 => "any month".to_string(),
        w => MONTHS[((w - 1) / 4).min(11) as usize].to_string(),
    };
    let value = format!("{:.2}, {:.2} \u{b7} {month}", place.lat, place.lon);
    // Beside the checkbox, so turning it on or off changes this one value
    // and moves nothing: blank when off, then "Checking..." until the
    // location list is worked out, then how many listed species are flagged.
    let marked = move || {
        if !state.call_detection.mark_unexpected().get() {
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
        }
        .into_any(),
    )
}

const PLACE_HELP_OWN: &str = "From the recording's metadata. Birds the BirdNET Geomodel scores below 0.01 at this place and month are marked unlikely here. The place stays on this device.";
const PLACE_HELP_DEVICE: &str = "Your location, from this device, used for recordings without location data. Birds the BirdNET Geomodel scores below 0.01 at this place and month are marked unlikely here. Your location stays on this device and is not saved.";

/// The "By species" list: one row per species, most windows first. A click
/// shows the species' best window.
fn species_table(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
) -> impl IntoView {
    let min = classifier::min_confidence(state, method);
    let mut overlapping = false;
    let rows = state.classifier.results().with(|m| {
        let Some(run) = current_file.get().and_then(|f| m.get(&(f, method))) else {
            return Vec::new();
        };
        let order = summary::colour_order(run);
        let s = summary::summarise(run, min);
        overlapping = s.overlapping;
        s.species
            .iter()
            .map(|row| {
                let slot = order.iter().position(|&c| c == row.class);
                let label = run.labels.get(row.class as usize);
                let name = class_name(label);
                let scientific = label.map(|l| l.scientific.clone()).unwrap_or_default();
                let w = &run.windows[row.best_window];
                let (start, end) = (w.start_s, w.end_s);
                let flag = flag_cell(state, method, current_file.get(), label);
                let strip = strip_cell(state, method, scientific.clone(), name.clone());
                let class = move || {
                    if state.call_detection.selected_window().get() == Some(start) {
                        "detection-row selected"
                    } else {
                        "detection-row"
                    }
                };
                // Windows that overlap would count the same audio twice.
                let amount = if s.overlapping {
                    summary::time_amount(row.secs)
                } else {
                    row.windows.to_string()
                };
                view! {
                    <tr class=class title=scientific on:click=move |_| show_window(state, start, end)>
                        {strip}
                        <td class="detection-col-kind">{species_swatch(slot)}{name}</td>
                        {flag}
                        <td class="detection-col-num">{amount}</td>
                        <td class="detection-col-num">{format!("{:.2}", row.best)}</td>
                    </tr>
                }
            })
            .collect::<Vec<_>>()
    });
    view! {
        <div class="detection-list">
            <table class="detection-table">
                <thead>
                    <tr>
                        {strip_header()}
                        <th>"Species"</th>
                        {flag_header(method)}
                        <th class="detection-col-num">{if overlapping { "Total time" } else { "Windows" }}</th>
                        <th class="detection-col-num" title="Best confidence">"Best"</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    }
}

/// The "By window" list: windows at or above the minimum, in time order.
fn window_table(state: AppState, method: Method, current_file: Memo<Option<u64>>) -> impl IntoView {
    let min = classifier::min_confidence(state, method);
    // With overlapping windows the numbers change as a run adds windows
    // between others, so the column is left out.
    let mut numbered = true;
    let rows = state.classifier.results().with(|m| {
        let Some(run) = current_file.get().and_then(|f| m.get(&(f, method))) else {
            return Vec::new();
        };
        numbered = !run.overlapping();
        let order = summary::colour_order(run);
        run.windows
            .iter()
            .enumerate()
            .filter_map(|(i, w)| {
                let kind = summary::window_kind(run, i, min);
                if kind == WindowKind::Below {
                    return None;
                }
                let top = w.top[0];
                let label = run.label(&top);
                let name = class_name(label);
                let scientific = label.map(|l| l.scientific.clone()).unwrap_or_default();
                let (start, end) = (w.start_s, w.end_s);
                let flag = flag_cell(state, method, current_file.get(), label);
                let other = kind == WindowKind::NotSpecies;
                let slot = (!other)
                    .then(|| order.iter().position(|&c| c == top.class))
                    .flatten();
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
                Some(view! {
                    <tr class=class title=scientific on:click=move |_| show_window(state, start, end)>
                        {numbered.then(|| view! { <td class="detection-col-num">{i + 1}</td> })}
                        <td>{crate::format_time::format_time_display(start, 1)}</td>
                        <td class="detection-col-kind">{species_swatch(slot)}{name}</td>
                        {flag}
                        <td class="detection-col-num">{format!("{:.2}", top.confidence)}</td>
                    </tr>
                })
            })
            .collect::<Vec<_>>()
    });
    view! {
        <div class="detection-list">
            <table class="detection-table">
                <thead>
                    <tr>
                        {numbered.then(|| view! { <th class="detection-col-num">"#"</th> })}
                        <th>"Start"</th>
                        <th>"Species"</th>
                        {flag_header(method)}
                        <th class="detection-col-num" title="Confidence">"Conf."</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    }
}

/// The selected window's top 3 classes.
fn top_three(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
) -> Option<impl IntoView> {
    let start = state.call_detection.selected_window().get()?;
    state.classifier.results().with(|m| {
        let run = current_file.get().and_then(|f| m.get(&(f, method)))?;
        let i = run.window_at_start(start)?;
        let w = run.windows.get(i)?;
        // With overlapping windows the numbers change as a run adds windows
        // between others, so only the times are given.
        let which = if run.overlapping() {
            String::new()
        } else {
            format!(" {}", i + 1)
        };
        let rows = w
            .top
            .iter()
            .take(3)
            .map(|s| {
                let label = run.label(s);
                let scientific = label
                    .filter(|l| l.is_species && l.scientific != l.common)
                    .map(|l| l.scientific.clone());
                let flag = flag_cell(state, method, current_file.get(), label);
                // BattyBirdNET's "Noise" and the like get no row.
                let strip = label.filter(|l| l.is_species).map(|l| {
                    strip_cell(state, method, l.scientific.clone(), class_name(Some(l)))
                });
                let strip = match strip {
                    Some(cell) => cell.into_any(),
                    None => view! { <td class="detection-col-strip"></td> }.into_any(),
                };
                view! {
                    <tr>
                        {strip}
                        <td class="detection-col-kind">
                            {class_name(label)}
                            {scientific.map(|s| view! { <div class="detection-scientific">{s}</div> })}
                        </td>
                        {flag}
                        <td class="detection-col-num">{format!("{:.2}", s.confidence)}</td>
                    </tr>
                }
            })
            .collect::<Vec<_>>();
        Some(view! {
            <div class="detection-top3">
                <div class="detection-status-note">{if which.is_empty() {
                    format!(
                        "Top 3 in window {} to {}",
                        crate::format_time::format_time_display(w.start_s, 1),
                        crate::format_time::format_time_display(w.end_s, 1)
                    )
                } else {
                    format!(
                        "Top 3 in window{which} ({} to {})",
                        crate::format_time::format_time_display(w.start_s, 1),
                        crate::format_time::format_time_display(w.end_s, 1)
                    )
                }}</div>
                <table class="detection-table">
                    <tbody>{rows}</tbody>
                </table>
            </div>
        })
    })
}

/// The "Rows above the spectrogram" group: each species picked for this
/// method, with a button to remove it (so a species that falls below the
/// minimum confidence, and leaves the lists, can still be removed), and for
/// BirdNET a button per principal component.
fn strips_group(state: AppState, method: Method, current_file: Memo<Option<u64>>) -> impl IntoView {
    let picked = move || {
        let file = current_file.get();
        let list: Vec<String> = state.call_detection.strip_species().with(|v| {
            v.iter()
                .filter(|s| s.method == method)
                .map(|s| s.scientific.clone())
                .collect()
        });
        if list.is_empty() {
            return ().into_any();
        }
        let rows = list
            .into_iter()
            .map(|scientific| {
                // The common name from this file's results; the scientific
                // name for a species this run does not have.
                let label = state.classifier.results().with(|m| {
                    file.and_then(|f| m.get(&(f, method))).and_then(|run| {
                        classifier::strips::class_of(run, &scientific)
                            .and_then(|c| run.labels.get(c as usize).cloned())
                    })
                });
                let name = label
                    .as_ref()
                    .map_or_else(|| scientific.clone(), classifier::strips::display_name);
                let flag = flag_cell(state, method, file, label.as_ref());
                let remove = scientific.clone();
                view! {
                    <tr title=scientific>
                        <td class="detection-col-kind">{name.clone()}</td>
                        {flag}
                        <td class="detection-col-num">
                            <button
                                class="detection-link-btn strip-remove-btn"
                                aria-label=format!("Remove {name} row")
                                title=format!("Remove {name} row")
                                on:click=move |_| toggle_strip(state, method, remove.clone())
                            >"\u{d7}"</button>
                        </td>
                    </tr>
                }
            })
            .collect::<Vec<_>>();
        view! {
            <table class="detection-table">
                <tbody>{rows}</tbody>
            </table>
        }
        .into_any()
    };
    // BirdNET only: the components of its embedding. The count comes from
    // the components file, loaded on first use.
    let components = move || {
        if method != Method::BirdNet {
            return None;
        }
        let pcs = crate::components::score_strips::birdnet_components()?;
        let buttons = pcs
            .variance
            .iter()
            .enumerate()
            .map(|(k, share)| {
                let on = move || {
                    state
                        .call_detection
                        .strip_components()
                        .with(|v| v.contains(&k))
                };
                let name = format!("PC{}", k + 1);
                let help = format!(
                    "{name}: {:.1}% of the variation between BirdNET's classes\nShows {name} in a row above the spectrogram",
                    share * 100.0
                );
                view! {
                    <button
                        class=move || if on() { "psd-btn psd-btn-active" } else { "psd-btn" }
                        aria-pressed=move || on().to_string()
                        title=help
                        on:click=move |_| {
                            state
                                .call_detection
                                .strip_components()
                                .update(|v| classifier::strips::toggle(v, k));
                            save_strips(state);
                        }
                    >{name}</button>
                }
            })
            .collect::<Vec<_>>();
        Some(view! {
            <div class="setting-row">
                <span class="setting-label">"Principal components"</span>
            </div>
            <div class="detection-annotate-row strip-pc-buttons">{buttons}</div>
        })
    };
    view! {
        <div class="setting-group">
            <div class="setting-group-title">"Rows above the spectrogram"</div>
            {picked}
            {strip_search(state, method, current_file)}
            {components}
            {crate::components::sound_groups::controls(state, method, current_file, save_strips)}
        </div>
    }
}

/// Most matches the species search lists at once.
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

/// One species the search offers.
#[derive(Clone, PartialEq)]
struct SearchMatch {
    name: String,
    scientific: String,
    added: bool,
}

/// The species search in "Rows above the spectrogram": type part of a
/// common or scientific name, then pick a match (or press Enter for the
/// highlighted one) to add its row. Searches every class of the current
/// file's run for this method, so a species the run never scored highly can
/// be added. Matches are listed in the panel under the field rather than in
/// a popup, which the narrow sidebar would clip.
fn strip_search(state: AppState, method: Method, current_file: Memo<Option<u64>>) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let focused = RwSignal::new(false);
    let highlight = RwSignal::new(0usize);
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
        let Some(list) = labels.get() else {
            return Vec::new();
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
        classifier::strips::find_classes(&list.0, &names, &q, SEARCH_MATCHES)
            .into_iter()
            .filter_map(|c| list.0.get(c as usize))
            .map(|l| SearchMatch {
                name: classifier::strips::display_name(l),
                scientific: l.scientific.clone(),
                added: strip_picked(state, method, &l.scientific),
            })
            .collect()
    });
    let open = move || focused.get() && !query.with(|q| q.trim().is_empty());
    let choose = move |m: &SearchMatch| {
        if !m.added {
            toggle_strip(state, method, m.scientific.clone());
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
                    let scientific = (m.scientific != m.name).then(|| m.scientific.clone());
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
                            placeholder="Add a species"
                            aria-label="Add a species row above the spectrogram"
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

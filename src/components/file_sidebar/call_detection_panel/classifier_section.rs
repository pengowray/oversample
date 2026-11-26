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
//!    unlikely here", boxes on the spectrogram; `settings.rs`).
//! 2. "At the play position" (`live_view.rs`), for the methods with live
//!    readings.
//! 3. "In this recording" (`in_recording.rs`): one list of the species
//!    found, the sound clusters and the picked components, each with a pin
//!    that shows it in a row above the spectrogram; then the search that
//!    adds a row (`species_search.rs`).
//! 4. Folds: "Every window" (`every_window.rs`) and "Add N annotations"
//!    (`annotate.rs`).
//! 5. One line about the stored model, and the attribution the model's
//!    licence asks for (`model_line.rs`).
//!
//! The cells and pins the lists share are in `rows.rs`.

use super::annotate::annotate_body;
use super::bird_boxes;
use super::every_window::window_table;
use super::in_recording::in_recording_section;
use super::live_view::LiveView;
use super::model_line::model_line;
use super::parts::{in_force_line, settings_button, Fold};
use super::settings::{place_fact, place_rows, step_row};
use super::species_map::SpeciesMapDialog;
use super::{count, event_target_checked, event_target_value};
use crate::classifier::summary;
use crate::classifier::{
    self, catalog, location, ClassifierError, DownloadStatus, Method, MethodFacts, ModelChoice,
    RunStatus,
};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;

const MB: f64 = 1_000_000.0;

/// `67 MB`, or `0.1 MB` for a small file.
pub(super) fn mb(bytes: u64) -> String {
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
pub(super) struct RunInfo {
    pub(super) windows: usize,
    /// Windows Analyse made; the rest are readings at the play position.
    pub(super) analysed: usize,
    /// Time the windows' cells cover, in seconds.
    pub(super) covered_secs: f64,
    pub(super) complete: bool,
    pub(super) whole_file: bool,
    pub(super) region: Option<classifier::BatScope>,
    pub(super) window_secs: f64,
    /// The step the Analyse asked for, and the step the whole recording has
    /// so far (`ClassifierRun::step_secs`, `spacing_secs`).
    pub(super) step_secs: f64,
    pub(super) spacing_secs: f64,
}

/// The longest recording Analyse starts on by itself, in seconds.
pub(super) const AUTO_ANALYSE_MAX_SECS: f64 = 600.0;

thread_local! {
    /// The files and methods Analyse has started on by itself this session:
    /// a run the user stopped, or that failed, is not started again.
    static AUTO_ANALYSED: std::cell::RefCell<std::collections::HashSet<(u64, Method)>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
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
pub(super) fn model_files(method: Method) -> Vec<&'static catalog::ModelFile> {
    match method {
        Method::BirdNet | Method::Bats => vec![&catalog::BACKBONE],
        Method::Perch => catalog::PERCH_FILES.to_vec(),
        Method::BirdNetV3 => vec![&catalog::BIRDNET_V3],
    }
}

/// One status for several files: failed or unknown if any is, downloading
/// (with the share of all their bytes) while any is, downloaded when all are.
pub(super) fn combined_status(
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

    // Analyse runs by itself, once for each file and method, when the model
    // is on this device and the file has no Analyse yet: results then cover
    // the whole recording without Play being pressed. Long recordings are
    // left to the button, since the run takes a while and can heat a phone.
    Effect::new(move |_| {
        let Some(file) = current_file.get() else {
            return;
        };
        let Some((rate, secs)) = source_rate.get() else {
            return;
        };
        if action_status.get() != DownloadStatus::Downloaded
            || analysed.get()
            || state.classifier.run().with(|r| r.is_busy())
            || secs > AUTO_ANALYSE_MAX_SECS
            || method.min_source_rate().is_some_and(|min| rate < min)
        {
            return;
        }
        let choice = choice();
        if !AUTO_ANALYSED.with(|a| a.borrow_mut().insert((file, method))) {
            return;
        }
        let step = step_pick.get_untracked().secs(method);
        analyse(state, file, choice, step);
    });

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
    let live = method.info().live_model.is_some().then(|| {
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

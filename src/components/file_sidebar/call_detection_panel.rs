//! The Call detection tab. Its first choice is the method: the in-built call
//! detector (this file), or BirdNET or BattyBirdNET (`call_detection_panel/classifier_section.rs`).
//!
//! The in-built detector finds bat calls in the current file with the call
//! detector (`oversample_core::dsp::call_detect`), lists every detection with
//! its kind, and adds calls or trains as annotations.
//!
//! The search runs when the tab is open and the file or the detector settings
//! change. It reads the file a piece at a time and yields to the browser
//! between pieces, so long files keep the page responsive. Results live in
//! `AppState::call_detection`, so they survive closing the tab and the overlay keeps
//! showing them.

mod classifier_section;
mod species_map;

use crate::annotations::{now_iso8601, AnnotationId, AnnotationSet};
use crate::call_annotations::{
    build_call_annotations, count_call_annotations, count_detector_annotations, detector_tags_in,
    kind_label, remove_detector_annotations, AnnotateWhat,
};
use crate::classifier::{BatRegion, Method};
use crate::dsp::call_detect::{
    group_trains, CallKind, CallScanParams, CallScanner, DetectedCall, TrainParams,
};
use crate::state::store_fields::*;
use crate::state::{AppState, DetectionMethod, RightSidebarTab};
use classifier_section::ClassifierSection;
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_core::audio::source::ChannelView;

/// Seconds of audio searched between yields to the browser.
const PIECE_SECS: f64 = 0.25;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListFilter {
    BatCalls,
    All,
}

/// The settings that need a new search when they change: everything except
/// the train settings, which only regroup.
fn search_key(params: &CallScanParams) -> CallScanParams {
    CallScanParams {
        trains: TrainParams::default(),
        ..params.clone()
    }
}

/// `1 bat call`, `2 bat calls`.
pub(super) fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn khz(hz: f32) -> String {
    format!("{:.0}", hz / 1000.0)
}

/// The undo shortcut on this platform.
pub(super) fn undo_keys() -> &'static str {
    let is_mac = web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.contains("Mac"));
    if is_mac {
        "Cmd+Z"
    } else {
        "Ctrl+Z"
    }
}

fn kind_tooltip(kind: CallKind, params: &CallScanParams) -> String {
    match kind {
        CallKind::Bat => {
            "A similar call (peak within 5 kHz) is within 0.6 s before or after this detection. \
             At least one of the two is 18 dB or more above the noise floor, or both are \
             constant-frequency (CF) calls with peaks within 1.5 kHz."
                .to_string()
        }
        CallKind::Single => "Probably a bat call, but no similar call (peak within 5 kHz) is \
                             within 0.6 s before or after this detection, or neither of the \
                             two is 18 dB or more above the noise floor."
            .to_string(),
        CallKind::Tone => "Part of a steady tone lasting longer than 150 ms, such as a rat's \
                           22 kHz alarm call or electrical hum."
            .to_string(),
        CallKind::PeriodicPulse => {
            "Probably electrical interference. Part of a series of 8 or more \
                             detections with peaks within 1.5 kHz and the same spacing between \
                             them (within 1 ms). Bats vary the spacing of their calls."
                .to_string()
        }
        CallKind::Broadband => format!(
            "Probably a click, rustle or footstep. Its frequency range reaches within 1 kHz of \
             the lowest frequency searched ({} kHz).",
            khz(params.band_lo_hz())
        ),
        CallKind::TooLow => format!(
            "Peak frequency is below the lowest bat frequency ({} kHz), set under Detection \
             Settings.",
            khz(params.min_bat_hz)
        ),
        CallKind::Audible => format!(
            "Probably a bird, voice or footstep. The audible range (2 to {} kHz) was more than \
             4 dB above its noise floor at the same moment.",
            khz(params.audible_band_hz().1)
        ),
    }
}

fn row_tooltip(n: usize, call: &DetectedCall, train: Option<usize>) -> String {
    let mut text = format!(
        "Detection {n}: {}\nStart: {}\nPeak frequency: {:.1} kHz\nFrequency range: {:.1} to {:.1} kHz\nDuration: {:.1} ms",
        kind_label(call.kind),
        crate::format_time::format_time_display(call.start_s, 3),
        call.event.peak_freq_hz / 1000.0,
        call.event.min_freq_hz / 1000.0,
        call.event.max_freq_hz / 1000.0,
        call.duration_ms(),
    );
    // How long a horseshoe or leaf-nosed bat's call held its constant frequency.
    if call.is_constant_frequency() {
        text.push_str(&format!("\nCF duration: {:.1} ms", call.steady_s * 1000.0));
    }
    text.push_str(&format!(
        "\nLevel: {:.1} dB above the noise floor",
        call.event.peak_db_above_floor
    ));
    if call.kind == CallKind::Bat {
        match train {
            Some(t) => text.push_str(&format!("\nCall train: {}", t + 1)),
            None => text.push_str("\nCall train: none"),
        }
    }
    text
}

/// Search the current file for calls, unless the results for this file and
/// these settings are already there or on their way.
fn start_search(state: AppState, file_id: u64, params: CallScanParams) {
    let key = (file_id, search_key(&params));
    let file = state.library.files().with_untracked(|files| {
        files
            .iter()
            .find(|f| f.id == file_id)
            .map(|f| (f.audio.source.clone(), f.audio.sample_rate))
    });
    let Some((source, rate)) = file else {
        return;
    };
    // Yield first, so the panel is not rebuilt while the event that changed a
    // setting is still being handled.
    spawn_local(async move {
        crate::web_util::yield_now().await;
        let generation = state
            .call_detection
            .scan_generation()
            .get_untracked()
            .wrapping_add(1);
        state.call_detection.scan_generation().set(generation);
        state.call_detection.scanning().set(Some(key.clone()));
        state.call_detection.scanned().set(None);
        state.call_detection.detected().set(Vec::new());
        state.call_detection.trains().set(Vec::new());
        state.call_detection.selected_index().set(None);
        state.call_detection.progress().set(0.0);
        let is_latest =
            move || state.call_detection.scan_generation().get_untracked() == generation;

        let mut scanner = CallScanner::new(rate, &params);
        if scanner.band_hz().is_some() {
            let total = source.total_samples();
            let piece = ((PIECE_SECS * rate as f64) as u64).max(4096);
            let mut at = 0u64;
            while at < total {
                let len = piece.min(total - at) as usize;
                let read = crate::audio::streaming_source::read_region_ready(
                    &*source,
                    ChannelView::MonoMix,
                    at,
                    len,
                )
                .await;
                if !is_latest() {
                    return;
                }
                match read {
                    // A piece that could not be read: later calls keep their times.
                    None => scanner.gap(len as u64),
                    Some(samples) => {
                        scanner.push(&samples);
                        if samples.len() < len {
                            if source.length_is_estimated() {
                                // The end of a file whose length was a guess.
                                break;
                            }
                            scanner.gap((len - samples.len()) as u64);
                        }
                    }
                }
                at += len as u64;
                state
                    .call_detection
                    .progress()
                    .set(at as f64 / total as f64);
                crate::web_util::yield_now().await;
                if !is_latest() {
                    return;
                }
            }
        }
        let band = scanner.band_hz();
        let result = scanner.finish();
        let train_params = state
            .call_detection
            .params()
            .with_untracked(|p| p.trains.clone());
        let trains = group_trains(&result.calls, &train_params);
        state.call_detection.band_hz().set(band);
        state.call_detection.detected().set(result.calls);
        state.call_detection.trains().set(trains);
        state.call_detection.scanned().set(Some(key));
        state.call_detection.scanning().set(None);
        state.call_detection.progress().set(1.0);
    });
}

/// Add the detections as annotations to the current file, replacing the ones
/// this detector added before.
fn add_annotations(state: AppState, what: AnnotateWhat, bat_calls_only: bool) {
    let Some(idx) = state.library.current_index().get_untracked() else {
        return;
    };
    let Some(file_id) = state.file_id_at(idx) else {
        return;
    };
    let results_file = state
        .call_detection
        .scanned()
        .with_untracked(|s| s.as_ref().map(|(id, _)| *id));
    if results_file != Some(file_id) {
        return;
    }
    let now = now_iso8601();
    let built = state.call_detection.detected().with_untracked(|calls| {
        state.call_detection.trains().with_untracked(|trains| {
            build_call_annotations(calls, trains, what, bat_calls_only, &now, AnnotationId::new)
        })
    });
    if built.annotations.is_empty() {
        return;
    }
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
    state.snapshot_annotations();
    let mut removed = 0;
    let mut remaining_ids: Vec<AnnotationId> = Vec::new();
    state.annotations.store().update(|store| {
        let set = store.entry_or_insert_with(file_id, || new_set);
        removed = remove_detector_annotations(&mut set.annotations);
        set.annotations.extend(built.annotations);
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

    let calls_text = if bat_calls_only {
        count(built.calls, "call", "calls")
    } else {
        count(built.calls, "detection", "detections")
    };
    let trains_text = count(built.trains, "call train", "call trains");
    let mut toast = match what {
        AnnotateWhat::Calls => format!("{calls_text} annotated"),
        AnnotateWhat::Trains => format!("{trains_text} annotated"),
        AnnotateWhat::TrainsWithCalls if built.calls == 0 => format!("{trains_text} annotated"),
        AnnotateWhat::TrainsWithCalls => format!("{trains_text} and {calls_text} annotated"),
    };
    if removed > 0 {
        toast.push_str(&format!(
            ". {} removed.",
            count(removed, "previous annotation", "previous annotations")
        ));
    }
    state.show_info_toast(toast);
}

#[component]
fn InbuiltDetection() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Settings as shown while a slider is dragged; copied to the detector's
    // settings when it is released.
    let draft = RwSignal::new(state.call_detection.params().get_untracked());
    let annotate_what = RwSignal::new(AnnotateWhat::Calls);
    let bat_calls_only = RwSignal::new(true);
    let list_filter = RwSignal::new(ListFilter::All);

    // Search when the tab is open and the file or the search settings change.
    Effect::new(move || {
        if state.panels.right_tab().get() != RightSidebarTab::CallDetection {
            return;
        }
        let Some(file_id) = state.current_file_id_tracked() else {
            return;
        };
        let params = state.call_detection.params().get();
        let key = Some((file_id, search_key(&params)));
        if state.call_detection.scanned().get_untracked() == key
            || state.call_detection.scanning().get_untracked() == key
        {
            return;
        }
        start_search(state, file_id, params);
    });

    // Regroup trains when the train settings change; no new search needed.
    let train_params = Memo::new(move |_| state.call_detection.params().with(|p| p.trains.clone()));
    Effect::new(move |prev: Option<TrainParams>| {
        let tp = train_params.get();
        if prev.is_some_and(|p| p != tp) {
            let trains = state
                .call_detection
                .detected()
                .with_untracked(|calls| group_trains(calls, &tp));
            state.call_detection.trains().set(trains);
        }
        tp
    });

    let commit = move || {
        let d = draft.get_untracked();
        if state.call_detection.params().get_untracked() != d {
            state.call_detection.params().set(d);
        }
    };

    // Memos, so the list is rebuilt (and its scroll position lost) only when
    // these values change, not on every change to the file list.
    let current_file = Memo::new(move |_| state.current_file_id_tracked());
    let has_results = Memo::new(move |_| {
        let scanned = state
            .call_detection
            .scanned()
            .with(|s| s.as_ref().map(|(id, _)| *id));
        state.call_detection.scanning().with(|s| s.is_none())
            && scanned.is_some()
            && scanned == current_file.get()
    });
    let bat_count = Memo::new(move |_| {
        state
            .call_detection
            .detected()
            .with(|d| d.iter().filter(|c| c.kind == CallKind::Bat).count())
    });
    let total_count = Memo::new(move |_| state.call_detection.detected().with(|d| d.len()));
    // Annotations in this file made by the detector earlier, and their tags
    // (an earlier version of the detector has its own tag).
    let existing_auto = Memo::new(move |_| {
        let Some(file_id) = current_file.get() else {
            return (0, Vec::new());
        };
        state.annotations.store().with(|store| {
            store
                .get(file_id)
                .map(|set| {
                    (
                        count_detector_annotations(&set.annotations),
                        detector_tags_in(&set.annotations),
                    )
                })
                .unwrap_or_default()
        })
    });
    let to_add = Memo::new(move |_| {
        let (what, bat_only) = (annotate_what.get(), bat_calls_only.get());
        state.call_detection.detected().with(|calls| {
            state
                .call_detection
                .trains()
                .with(|trains| count_call_annotations(calls, trains, what, bat_only))
        })
    });

    // Click a row: select it and scroll the spectrogram to it.
    let on_row_click = move |i: usize| {
        state.call_detection.selected_index().set(Some(i));
        let mid = state
            .call_detection
            .detected()
            .with_untracked(|d| d.get(i).map(|c| 0.5 * (c.start_s + c.end_s)));
        let Some(mid) = mid else { return };
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
    };

    let status = move || {
        if current_file.get().is_none() {
            return view! { <div class="sidebar-panel-empty">"No file selected"</div> }.into_any();
        }
        if state.call_detection.scanning().with(|s| s.is_some()) {
            let pct = (state.call_detection.progress().get() * 100.0).round();
            return view! {
                <div class="detection-status">
                    <div>{format!("Searching for bat calls\u{2026} {pct:.0}% done")}</div>
                    <div class="detection-progress">
                        <div class="detection-progress-fill" style=format!("width:{pct:.0}%")></div>
                    </div>
                </div>
            }
            .into_any();
        }
        if !has_results.get() {
            return view! { <div class="detection-status"></div> }.into_any();
        }
        let params = state.call_detection.params().get();
        let Some((lo, hi)) = state.call_detection.band_hz().get() else {
            let rate = state.library.files().with(|files| {
                state
                    .library
                    .current_index()
                    .get()
                    .and_then(|i| files.get(i))
                    .map(|f| f.audio.sample_rate)
                    .unwrap_or(0)
            });
            let min_rate = 2.0 * (params.band_lo_hz() + 1000.0);
            let half_rate = rate as f32 / 2.0;
            let hint = (half_rate > 8_000.0).then(|| {
                format!(
                    "Set the lowest bat frequency below {} kHz to search this recording.",
                    khz(half_rate)
                )
            });
            return view! {
                <div class="detection-status">
                    <div>{format!(
                        "Sample rate too low: {} kHz. The search needs a sample rate above {} kHz.",
                        khz(rate as f32),
                        khz(min_rate)
                    )}</div>
                    {hint.map(|h| view! { <div class="detection-status-note">{h}</div> })}
                </div>
            }
            .into_any();
        };
        let total = total_count.get();
        if total == 0 {
            return view! {
                <div class="detection-status">
                    <div>{format!("Nothing detected from {} to {} kHz.", khz(lo), khz(hi))}</div>
                    <div class="detection-status-note">"Lower the threshold to find fainter sounds."</div>
                </div>
            }
            .into_any();
        }
        let bats = bat_count.get();
        let trains = state.call_detection.trains().with(|t| t.len());
        let others = total - bats;
        view! {
            <div class="detection-status">
                <div class="detection-summary">{format!(
                    "{}, {}",
                    count(bats, "bat call", "bat calls"),
                    count(trains, "call train", "call trains")
                )}</div>
                {(others > 0).then(|| view! {
                    <div class="detection-status-note">{count(others, "other detection", "other detections")}</div>
                })}
            </div>
        }
        .into_any()
    };

    let annotate_section = move || {
        if !has_results.get() || total_count.get() == 0 {
            return None;
        }
        let what = annotate_what.get();
        let (calls, trains) = to_add.get();
        let n = calls + trains;
        let others = total_count.get() - bat_count.get();
        let reason = (n == 0).then(|| {
            if what == AnnotateWhat::Calls {
                if bat_calls_only.get() && others > 0 {
                    format!(
                        "No bat calls found. Turn off \u{201C}Bat calls only\u{201D} to add the {}.",
                        count(others, "other detection", "other detections")
                    )
                } else {
                    "No bat calls found".to_string()
                }
            } else {
                "No call trains found".to_string()
            }
        });
        let (existing, existing_tags) = existing_auto.get();
        let note = (existing > 0 && n > 0).then(|| {
            let tags = existing_tags.join(" or ");
            if existing == 1 {
                format!(
                    "Replaces 1 annotation tagged {tags}, even if you edited it. Undo with {}.",
                    undo_keys()
                )
            } else {
                format!(
                    "Replaces {existing} annotations tagged {tags}, including any you edited. Undo with {}.",
                    undo_keys()
                )
            }
        });
        let button_text = if n == 0 {
            "Add annotations".to_string()
        } else {
            format!("Add {}", count(n, "annotation", "annotations"))
        };
        // Radio buttons in a column: the sidebar is too narrow for the three
        // options side by side.
        let option = move |value: AnnotateWhat, label: &'static str| {
            view! {
                <label class="setting-label detection-radio">
                    <input
                        type="radio"
                        name="detection-annotate-what"
                        prop:checked=move || annotate_what.get() == value
                        on:change=move |_| annotate_what.set(value)
                    />
                    {label}
                </label>
            }
        };
        Some(view! {
            <div class="setting-group">
                <div class="setting-group-title">"Annotate Calls and Trains"</div>
                <div class="detection-annotate-row detection-radio-group">
                    {option(AnnotateWhat::Calls, "Calls")}
                    {option(AnnotateWhat::Trains, "Trains")}
                    {option(AnnotateWhat::TrainsWithCalls, "Trains with nested calls")}
                    {(what == AnnotateWhat::TrainsWithCalls).then(|| view! {
                        <div class="detection-status-note detection-radio-note">"Calls not in a train are added at the top level"</div>
                    })}
                </div>
                <div class="setting-row">
                    <label
                        class="setting-label"
                        title="Turn off to add every detection as a call annotation, with its kind in the label. Trains contain only bat calls."
                    >
                        <input
                            type="checkbox"
                            prop:checked=move || bat_calls_only.get()
                            on:change=move |ev| bat_calls_only.set(event_target_checked(&ev))
                        />
                        " Bat calls only"
                    </label>
                </div>
                {reason.map(|r| view! { <div class="detection-status-note detection-annotate-row">{r}</div> })}
                {note.map(|t| view! {
                    <div
                        class="detection-status-note detection-annotate-row"
                        title="Annotations without this tag are kept. Any that were inside a removed train move to the top level of the Annotations tab."
                    >{t}</div>
                })}
                <button
                    class="sidebar-btn detection-add-btn"
                    disabled=n == 0
                    on:click=move |_| add_annotations(state, annotate_what.get_untracked(), bat_calls_only.get_untracked())
                >{button_text}</button>
            </div>
        })
    };

    let list_section = move || {
        if !has_results.get() || total_count.get() == 0 {
            return None;
        }
        let filter = list_filter.get();
        let params = state.call_detection.params().get();
        // Which train each call is in.
        let train_of: std::collections::HashMap<usize, usize> =
            state.call_detection.trains().with(|trains| {
                trains
                    .iter()
                    .enumerate()
                    .flat_map(|(t, train)| train.calls.iter().map(move |&i| (i, t)))
                    .collect()
            });
        let rows = state.call_detection.detected().with(|calls| {
            calls
                .iter()
                .enumerate()
                .filter(|(_, c)| filter == ListFilter::All || c.kind == CallKind::Bat)
                .map(|(i, c)| {
                    let is_bat = c.kind == CallKind::Bat;
                    let tooltip = row_tooltip(i + 1, c, train_of.get(&i).copied());
                    let kind_title = kind_tooltip(c.kind, &params);
                    let row_class = move || {
                        let mut class = String::from("detection-row");
                        if !is_bat {
                            class.push_str(" detection-row-other");
                        }
                        if state.call_detection.selected_index().get() == Some(i) {
                            class.push_str(" selected");
                        }
                        class
                    };
                    view! {
                        <tr class=row_class title=tooltip on:click=move |_| on_row_click(i)>
                            <td class="detection-col-num">{i + 1}</td>
                            <td>{crate::format_time::format_time_display(c.start_s, 3)}</td>
                            <td class="detection-col-kind" title=kind_title>{kind_label(c.kind)}</td>
                            <td class="detection-col-num">{format!("{:.1}", c.event.peak_freq_hz / 1000.0)}</td>
                            <td class="detection-col-num">{format!("{:.1}", c.duration_ms())}</td>
                        </tr>
                    }
                })
                .collect::<Vec<_>>()
        });
        let filter_button = move |value: ListFilter, label: String| {
            view! {
                <button
                    class=move || if list_filter.get() == value { "psd-btn psd-btn-active" } else { "psd-btn" }
                    on:click=move |_| list_filter.set(value)
                >{label}</button>
            }
        };
        Some(view! {
            <div class="setting-group">
                <div class="detection-annotate-row">
                    <div class="psd-btn-group">
                        // The counts are in the status line above.
                        {filter_button(ListFilter::BatCalls, "Bat calls".to_string())}
                        {filter_button(ListFilter::All, "All detections".to_string())}
                    </div>
                </div>
                <div class="detection-list">
                    <table class="detection-table">
                        <thead>
                            <tr>
                                <th class="detection-col-num">"#"</th>
                                <th>"Start"</th>
                                <th>"Kind"</th>
                                <th class="detection-col-num" title="Peak frequency">"kHz"</th>
                                <th class="detection-col-num" title="Duration">"ms"</th>
                            </tr>
                        </thead>
                        <tbody>{rows}</tbody>
                    </table>
                </div>
            </div>
        })
    };

    // One slider row: label, current value, range input.
    let slider = move |label: &'static str,
                       tooltip: &'static str,
                       (min, max, step): (f64, f64, f64),
                       get: fn(&CallScanParams) -> f64,
                       set: fn(&mut CallScanParams, f64),
                       shown: fn(f64) -> String| {
        view! {
            <div class="setting-row" title=tooltip>
                <span class="setting-label">{label}</span>
                <span class="setting-value">{move || shown(draft.with(get))}</span>
            </div>
            <div class="setting-row" title=tooltip>
                <input
                    type="range"
                    class="setting-range"
                    min=min.to_string()
                    max=max.to_string()
                    step=step.to_string()
                    prop:value=move || draft.with(get).to_string()
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f64>() {
                            draft.update(|d| set(d, v));
                        }
                    }
                    on:change=move |_| commit()
                />
            </div>
        }
    };

    view! {
        <div>
            {status}
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
            {annotate_section}
            {list_section}
            <div class="setting-group">
                <div class="setting-group-title">"Detection Settings"</div>
                {slider(
                    "Lowest bat frequency",
                    "Detections with a peak frequency below this are marked Low frequency. Default 16 kHz. Lower it to find free-tailed bats calling near 11 kHz; expect more false detections from birds and insects.",
                    (8.0, 30.0, 1.0),
                    |p| p.min_bat_hz as f64 / 1000.0,
                    |p, v| p.min_bat_hz = (v * 1000.0) as f32,
                    |v| format!("{v:.0} kHz"),
                )}
                {slider(
                    "Threshold",
                    "Sounds must be at least this many dB above the noise floor to be detected. Default 12 dB. Lower values find fainter calls and more noise.",
                    (6.0, 24.0, 1.0),
                    |p| p.threshold_db as f64,
                    |p, v| {
                        // Keep the release level 3 dB under the threshold, as in the defaults.
                        p.threshold_db = v as f32;
                        p.release_db = v as f32 - 3.0;
                    },
                    |v| format!("{v:.0} dB"),
                )}
                {slider(
                    "Min duration",
                    "Sounds shorter than this are ignored. Default 1 ms.",
                    (0.5, 5.0, 0.5),
                    |p| p.min_call_ms as f64,
                    |p, v| p.min_call_ms = v as f32,
                    |v| format!("{v:.1} ms"),
                )}
                {slider(
                    "Max duration",
                    "Sounds longer than this are treated as background noise and ignored. Default 100 ms.",
                    (20.0, 200.0, 10.0),
                    |p| p.max_call_ms as f64,
                    |p, v| p.max_call_ms = v as f32,
                    |v| format!("{v:.0} ms"),
                )}
                {slider(
                    "Train gap",
                    "Longest gap between calls in one call train. A call joins a train if it starts within this time after the train's last call ends and its peak is within 5 kHz of that call's peak. Default 0.5 s. Changing it regroups the trains straight away.",
                    (0.1, 2.0, 0.1),
                    |p| p.trains.max_gap_s,
                    |p, v| p.trains.max_gap_s = v,
                    |v| format!("{v:.1} s"),
                )}
            </div>
        </div>
    }
}

/// The Call detection tab: a choice of method, then that method's panel.
#[component]
pub(crate) fn CallDetectionPanel() -> impl IntoView {
    let state = expect_context::<AppState>();
    let method = move || state.call_detection.method().get();

    let option = move |value: DetectionMethod, label: &'static str, help: &'static str| {
        view! {
            <label class="setting-label detection-radio detection-method">
                <input
                    type="radio"
                    name="detection-method"
                    prop:checked=move || method() == value
                    on:change=move |_| {
                        state.call_detection.method().set(value);
                        state.call_detection.selected_window().set(None);
                    }
                />
                <span class="detection-method-text">
                    <span>{label}</span>
                    <span class="detection-status-note">{help}</span>
                </span>
            </label>
        }
    };
    let region_select = move || {
        (method() == DetectionMethod::Bats).then(|| {
            let busy = move || state.classifier.run().with(|r| r.is_busy());
            view! {
                <label class="detection-region">
                    <span class="setting-label">"Region"</span>
                    <select
                        class="setting-select"
                        prop:disabled=busy
                        on:change=move |ev| {
                            let i: usize = event_target_select_value(&ev).parse().unwrap_or(0);
                            if let Some(r) = BatRegion::ALL.get(i) {
                                state.call_detection.bat_region().set(*r);
                            }
                        }
                    >
                        {BatRegion::ALL
                            .iter()
                            .enumerate()
                            .map(|(i, r)| {
                                let r = *r;
                                view! {
                                    <option
                                        value=i.to_string()
                                        prop:selected=move || state.call_detection.bat_region().get() == r
                                    >{format!("{}: {} species", r.label(), region_species(r))}</option>
                                }
                            })
                            .collect::<Vec<_>>()}
                    </select>
                </label>
            }
        })
    };

    view! {
        <div class="sidebar-panel">
            <div class="detection-annotate-row detection-radio-group detection-methods">
                {option(DetectionMethod::Inbuilt, "In-built bat call detection", "Finds bat calls. Runs automatically.")}
                {option(DetectionMethod::BirdNet, "BirdNET", "Bird species, plus some other sounds such as dog and engine")}
                // Before BattyBirdNET, so the region list stays under it.
                {option(DetectionMethod::Perch, "Perch 2.0", "Birds, frogs, insects and some mammals, plus sounds such as traffic and thunder")}
                {option(DetectionMethod::Bats, "BattyBirdNET", "Bat species for one region")}
                {region_select}
            </div>
            {move || match method() {
                DetectionMethod::Inbuilt => view! { <InbuiltDetection /> }.into_any(),
                DetectionMethod::BirdNet => view! { <ClassifierSection method=Method::BirdNet /> }.into_any(),
                DetectionMethod::Perch => view! { <ClassifierSection method=Method::Perch /> }.into_any(),
                DetectionMethod::Bats => view! { <ClassifierSection method=Method::Bats /> }.into_any(),
            }}
        </div>
    }
}

/// Bat species in each region's BattyBirdNET classifier (lines of its label
/// file with a scientific and a common name; crickets count as one).
fn region_species(region: BatRegion) -> usize {
    match region {
        BatRegion::Bavaria | BatRegion::BavariaHigh => 22,
        BatRegion::Eu => 29,
        BatRegion::Scotland => 10,
        BatRegion::SouthWales => 28,
        BatRegion::Sweden => 18,
        BatRegion::Uk => 15,
        BatRegion::Usa => 33,
        BatRegion::UsaEast | BatRegion::UsaEastHigh => 17,
        BatRegion::UsaWest => 23,
    }
}

fn event_target_select_value(ev: &web_sys::Event) -> String {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok())
        .map(|el| el.value())
        .unwrap_or_default()
}

pub(super) fn event_target_value(ev: &web_sys::Event) -> String {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.value())
        .unwrap_or_default()
}

pub(super) fn event_target_checked(ev: &web_sys::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.checked())
        .unwrap_or(false)
}

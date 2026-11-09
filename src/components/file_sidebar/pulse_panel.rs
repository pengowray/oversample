//! The Pulses tab: finds bat calls in the current file with the call
//! detector (`oversample_core::dsp::call_detect`), lists every detection with
//! its kind, and adds calls or trains as annotations.
//!
//! The search runs when the tab is open and the file or the detector settings
//! change. It reads the file a piece at a time and yields to the browser
//! between pieces, so long files keep the page responsive. Results live in
//! `AppState::pulse`, so they survive closing the tab and the overlay keeps
//! showing them.

use crate::annotations::{now_iso8601, AnnotationId, AnnotationSet};
use crate::call_annotations::{
    build_call_annotations, count_call_annotations, count_detector_annotations, kind_label,
    remove_detector_annotations, AnnotateWhat,
};
use crate::dsp::call_detect::{
    group_trains, CallKind, CallScanParams, CallScanner, DetectedCall, TrainParams,
};
use crate::state::store_fields::*;
use crate::state::{AppState, RightSidebarTab};
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
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn khz(hz: f32) -> String {
    format!("{:.0}", hz / 1000.0)
}

/// The undo shortcut on this platform.
fn undo_keys() -> &'static str {
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
            "A similar call (peak within 5 kHz) is within 0.6 s before or after this detection."
                .to_string()
        }
        CallKind::Single => "Probably a bat call, but no similar call (peak within 5 kHz) is \
                             within 0.6 s before or after this detection."
            .to_string(),
        CallKind::Tone => "Part of a steady tone lasting longer than 150 ms, such as a rat's \
                           22 kHz alarm call or electrical hum. Also used for 8 or more sounds \
                           with the same spacing (within 1 ms), usually electrical \
                           interference. Bats vary the spacing of their calls."
            .to_string(),
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
        "Detection {n}: {}\nStart: {}\nPeak frequency: {:.1} kHz\nFrequency range: {:.1} to {:.1} kHz\nDuration: {:.1} ms\nLevel: {:.1} dB above the noise floor",
        kind_label(call.kind),
        crate::format_time::format_time_display(call.start_s, 3),
        call.event.peak_freq_hz / 1000.0,
        call.event.min_freq_hz / 1000.0,
        call.event.max_freq_hz / 1000.0,
        call.duration_ms(),
        call.event.peak_db_above_floor,
    );
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
            .pulse
            .scan_generation()
            .get_untracked()
            .wrapping_add(1);
        state.pulse.scan_generation().set(generation);
        state.pulse.scanning().set(Some(key.clone()));
        state.pulse.scanned().set(None);
        state.pulse.detected().set(Vec::new());
        state.pulse.trains().set(Vec::new());
        state.pulse.selected_index().set(None);
        state.pulse.progress().set(0.0);
        let is_latest = move || state.pulse.scan_generation().get_untracked() == generation;

        let mut scanner = CallScanner::new(rate, &params);
        if scanner.band_hz().is_some() {
            let total = source.total_samples();
            let piece = ((PIECE_SECS * rate as f64) as u64).max(4096);
            let mut at = 0u64;
            while at < total {
                let len = piece.min(total - at) as usize;
                crate::audio::streaming_source::prefetch_streaming(&*source, at, len).await;
                if !is_latest() {
                    return;
                }
                let samples = source.read_region(ChannelView::MonoMix, at, len);
                scanner.push(&samples);
                if samples.len() < len {
                    if source.length_is_estimated() {
                        // The end of a file whose length was a guess.
                        break;
                    }
                    // A piece that could not be read: later calls keep their times.
                    scanner.gap((len - samples.len()) as u64);
                }
                at += len as u64;
                state.pulse.progress().set(at as f64 / total as f64);
                crate::web_util::yield_now().await;
                if !is_latest() {
                    return;
                }
            }
        }
        let band = scanner.band_hz();
        let result = scanner.finish();
        let train_params = state.pulse.params().with_untracked(|p| p.trains.clone());
        let trains = group_trains(&result.calls, &train_params);
        state.pulse.band_hz().set(band);
        state.pulse.detected().set(result.calls);
        state.pulse.trains().set(trains);
        state.pulse.scanned().set(Some(key));
        state.pulse.scanning().set(None);
        state.pulse.progress().set(1.0);
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
        .pulse
        .scanned()
        .with_untracked(|s| s.as_ref().map(|(id, _)| *id));
    if results_file != Some(file_id) {
        return;
    }
    let now = now_iso8601();
    let built = state.pulse.detected().with_untracked(|calls| {
        state.pulse.trains().with_untracked(|trains| {
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
pub(crate) fn PulsePanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Settings as shown while a slider is dragged; copied to the detector's
    // settings when it is released.
    let draft = RwSignal::new(state.pulse.params().get_untracked());
    let annotate_what = RwSignal::new(AnnotateWhat::Calls);
    let bat_calls_only = RwSignal::new(true);
    let list_filter = RwSignal::new(ListFilter::All);

    // Search when the tab is open and the file or the search settings change.
    Effect::new(move || {
        if state.panels.right_tab().get() != RightSidebarTab::Pulses {
            return;
        }
        let Some(file_id) = state.current_file_id_tracked() else {
            return;
        };
        let params = state.pulse.params().get();
        let key = Some((file_id, search_key(&params)));
        if state.pulse.scanned().get_untracked() == key
            || state.pulse.scanning().get_untracked() == key
        {
            return;
        }
        start_search(state, file_id, params);
    });

    // Regroup trains when the train settings change; no new search needed.
    let train_params = Memo::new(move |_| state.pulse.params().with(|p| p.trains.clone()));
    Effect::new(move |prev: Option<TrainParams>| {
        let tp = train_params.get();
        if prev.is_some_and(|p| p != tp) {
            let trains = state
                .pulse
                .detected()
                .with_untracked(|calls| group_trains(calls, &tp));
            state.pulse.trains().set(trains);
        }
        tp
    });

    let commit = move || {
        let d = draft.get_untracked();
        if state.pulse.params().get_untracked() != d {
            state.pulse.params().set(d);
        }
    };

    // Memos, so the list is rebuilt (and its scroll position lost) only when
    // these values change, not on every change to the file list.
    let current_file = Memo::new(move |_| state.current_file_id_tracked());
    let has_results = Memo::new(move |_| {
        let scanned = state
            .pulse
            .scanned()
            .with(|s| s.as_ref().map(|(id, _)| *id));
        state.pulse.scanning().with(|s| s.is_none())
            && scanned.is_some()
            && scanned == current_file.get()
    });
    let bat_count = Memo::new(move |_| {
        state
            .pulse
            .detected()
            .with(|d| d.iter().filter(|c| c.kind == CallKind::Bat).count())
    });
    let total_count = Memo::new(move |_| state.pulse.detected().with(|d| d.len()));
    // Annotations in this file made by the detector earlier.
    let existing_auto = Memo::new(move |_| {
        let Some(file_id) = current_file.get() else {
            return 0;
        };
        state.annotations.store().with(|store| {
            store
                .get(file_id)
                .map(|set| count_detector_annotations(&set.annotations))
                .unwrap_or(0)
        })
    });
    let to_add = Memo::new(move |_| {
        let (what, bat_only) = (annotate_what.get(), bat_calls_only.get());
        state.pulse.detected().with(|calls| {
            state
                .pulse
                .trains()
                .with(|trains| count_call_annotations(calls, trains, what, bat_only))
        })
    });

    // Click a row: select it and scroll the spectrogram to it.
    let on_row_click = move |i: usize| {
        state.pulse.selected_index().set(Some(i));
        let mid = state
            .pulse
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
        if state.pulse.scanning().with(|s| s.is_some()) {
            let pct = (state.pulse.progress().get() * 100.0).round();
            return view! {
                <div class="pulse-status">
                    <div>{format!("Searching for bat calls\u{2026} {pct:.0}% done")}</div>
                    <div class="pulse-progress">
                        <div class="pulse-progress-fill" style=format!("width:{pct:.0}%")></div>
                    </div>
                </div>
            }
            .into_any();
        }
        if !has_results.get() {
            return view! { <div class="pulse-status"></div> }.into_any();
        }
        let params = state.pulse.params().get();
        let Some((lo, hi)) = state.pulse.band_hz().get() else {
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
                <div class="pulse-status">
                    <div>{format!(
                        "Sample rate too low: {} kHz. The search needs a sample rate above {} kHz.",
                        khz(rate as f32),
                        khz(min_rate)
                    )}</div>
                    {hint.map(|h| view! { <div class="pulse-status-note">{h}</div> })}
                </div>
            }
            .into_any();
        };
        let total = total_count.get();
        if total == 0 {
            return view! {
                <div class="pulse-status">
                    <div>{format!("Nothing detected from {} to {} kHz.", khz(lo), khz(hi))}</div>
                    <div class="pulse-status-note">"Lower the threshold to find fainter sounds."</div>
                </div>
            }
            .into_any();
        }
        let bats = bat_count.get();
        let trains = state.pulse.trains().with(|t| t.len());
        let others = total - bats;
        view! {
            <div class="pulse-status">
                <div class="pulse-summary">{format!(
                    "{}, {}",
                    count(bats, "bat call", "bat calls"),
                    count(trains, "call train", "call trains")
                )}</div>
                {(others > 0).then(|| view! {
                    <div class="pulse-status-note">{count(others, "other detection", "other detections")}</div>
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
        let existing = existing_auto.get();
        let note = (existing > 0 && n > 0).then(|| {
            if existing == 1 {
                format!(
                    "Replaces 1 annotation tagged #auto=calls-1, even if you edited it. Undo with {}.",
                    undo_keys()
                )
            } else {
                format!(
                    "Replaces {existing} annotations tagged #auto=calls-1, including any you edited. Undo with {}.",
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
                <label class="setting-label pulse-radio">
                    <input
                        type="radio"
                        name="pulse-annotate-what"
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
                <div class="pulse-annotate-row pulse-radio-group">
                    {option(AnnotateWhat::Calls, "Calls")}
                    {option(AnnotateWhat::Trains, "Trains")}
                    {option(AnnotateWhat::TrainsWithCalls, "Trains with nested calls")}
                    {(what == AnnotateWhat::TrainsWithCalls).then(|| view! {
                        <div class="pulse-status-note pulse-radio-note">"Calls not in a train are added at the top level"</div>
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
                {reason.map(|r| view! { <div class="pulse-status-note pulse-annotate-row">{r}</div> })}
                {note.map(|t| view! {
                    <div
                        class="pulse-status-note pulse-annotate-row"
                        title="Annotations without this tag are kept. Any that were inside a removed train move to the top level of the Annotations tab."
                    >{t}</div>
                })}
                <button
                    class="sidebar-btn pulse-add-btn"
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
        let params = state.pulse.params().get();
        // Which train each call is in.
        let train_of: std::collections::HashMap<usize, usize> =
            state.pulse.trains().with(|trains| {
                trains
                    .iter()
                    .enumerate()
                    .flat_map(|(t, train)| train.calls.iter().map(move |&i| (i, t)))
                    .collect()
            });
        let rows = state.pulse.detected().with(|calls| {
            calls
                .iter()
                .enumerate()
                .filter(|(_, c)| filter == ListFilter::All || c.kind == CallKind::Bat)
                .map(|(i, c)| {
                    let is_bat = c.kind == CallKind::Bat;
                    let tooltip = row_tooltip(i + 1, c, train_of.get(&i).copied());
                    let kind_title = kind_tooltip(c.kind, &params);
                    let row_class = move || {
                        let mut class = String::from("pulse-row");
                        if !is_bat {
                            class.push_str(" pulse-row-other");
                        }
                        if state.pulse.selected_index().get() == Some(i) {
                            class.push_str(" selected");
                        }
                        class
                    };
                    view! {
                        <tr class=row_class title=tooltip on:click=move |_| on_row_click(i)>
                            <td class="pulse-col-num">{i + 1}</td>
                            <td>{crate::format_time::format_time_display(c.start_s, 3)}</td>
                            <td class="pulse-col-kind" title=kind_title>{kind_label(c.kind)}</td>
                            <td class="pulse-col-num">{format!("{:.1}", c.event.peak_freq_hz / 1000.0)}</td>
                            <td class="pulse-col-num">{format!("{:.1}", c.duration_ms())}</td>
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
                <div class="pulse-annotate-row">
                    <div class="psd-btn-group">
                        // The counts are in the status line above.
                        {filter_button(ListFilter::BatCalls, "Bat calls".to_string())}
                        {filter_button(ListFilter::All, "All detections".to_string())}
                    </div>
                </div>
                <div class="pulse-list">
                    <table class="pulse-table">
                        <thead>
                            <tr>
                                <th class="pulse-col-num">"#"</th>
                                <th>"Start"</th>
                                <th>"Kind"</th>
                                <th class="pulse-col-num" title="Peak frequency">"kHz"</th>
                                <th class="pulse-col-num" title="Duration">"ms"</th>
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
        <div class="sidebar-panel">
            {status}
            <div class="setting-row pulse-annotate-row">
                <label class="setting-label">
                    <input
                        type="checkbox"
                        prop:checked=move || state.pulse.overlay_enabled().get()
                        on:change=move |ev| state.pulse.overlay_enabled().set(event_target_checked(&ev))
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

fn event_target_value(ev: &web_sys::Event) -> String {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.value())
        .unwrap_or_default()
}

fn event_target_checked(ev: &web_sys::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.checked())
        .unwrap_or(false)
}

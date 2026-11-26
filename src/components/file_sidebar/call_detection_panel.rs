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

mod annotate;
mod bird_boxes;
mod classifier_section;
mod combined;
mod every_window;
mod in_recording;
mod live_view;
mod model_line;
mod parts;
mod rows;
mod settings;
mod species_map;
mod species_search;

use crate::annotations::{now_iso8601, AnnotationId, AnnotationSet};
use crate::call_annotations::{
    build_call_annotations, count_call_annotations, count_detector_annotations, detector_tags_in,
    kind_label, remove_detector_annotations, AnnotateWhat,
};
use crate::classifier::{bat_pick, BatPick, BatRegion, BatScope, MethodFacts};
use crate::dsp::call_detect::{
    group_trains, CallKind, CallScanParams, CallScanner, DetectedCall, TrainParams,
};
use crate::state::store_fields::*;
use crate::state::{AppState, DetectionMethod, RightSidebarTab};
use classifier_section::ClassifierSection;
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_core::audio::source::ChannelView;
use parts::{in_force_line, section_head, settings_button, Fold};

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
        state.call_detection.hovered_index().set(None);
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

/// Run the call detector on the current file while the Call detection tab
/// is open, again when the file or the search settings change, and regroup
/// the trains when only the train settings change. For the in-built
/// detector's panel and the combined scan's.
fn search_when_open(state: AppState) {
    // Search when the tab is open and the file or the search settings change.
    Effect::new(move || {
        if state.panels.right_tab().get() != RightSidebarTab::CallDetection {
            return;
        }
        let Some(file_id) = state.current_file_id_tracked() else {
            return;
        };
        let params = state.call_detection_params();
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

    search_when_open(state);

    let commit = move || {
        let d = draft.get_untracked();
        // The lowest bat frequency is shared with the loudest-frequency line,
        // ZC and HFR. The slider changes the value in force: the file's own
        // (Settings > Bats > this file) if it has one, else the app setting.
        if d.min_bat_hz != state.lowest_bat_hz_untracked() {
            let own = state
                .library
                .current_index()
                .get_untracked()
                .filter(|&i| state.file_lowest_bat_hz(i).is_some());
            match own {
                Some(i) => state.set_file_lowest_bat_hz(i, Some(d.min_bat_hz)),
                None => state.set_lowest_bat_hz(d.min_bat_hz),
            }
        }
        let old = state.call_detection.params().get_untracked();
        let new = CallScanParams {
            min_bat_hz: old.min_bat_hz,
            ..d
        };
        if old != new {
            state.call_detection.params().set(new);
        }
    };
    // Follow changes made elsewhere (Settings > Bats, or another file), so
    // the next commit here doesn't put an old value back.
    Effect::new(move || {
        let p = state.call_detection_params();
        if draft.get_untracked() != p {
            draft.set(p);
        }
    });

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

    // Click a row: pick it, make its box the selection on the spectrogram
    // (so it can be played, annotated or zoomed to like one drawn by hand)
    // and scroll the spectrogram to it.
    let on_row_click = move |i: usize| {
        state.call_detection.selected_index().set(Some(i));
        let found = state.call_detection.detected().with_untracked(|d| {
            d.get(i).map(|c| crate::state::Selection {
                time_start: c.start_s,
                time_end: c.end_s,
                freq_low: Some(c.event.min_freq_hz as f64),
                freq_high: Some(c.event.max_freq_hz as f64),
            })
        });
        let Some(sel) = found else { return };
        let mid = 0.5 * (sel.time_start + sel.time_end);
        state.interaction.selection().set(Some(sel));
        state
            .interaction
            .active_focus()
            .set(Some(crate::state::ActiveFocus::TransientSelection));
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

    let settings_open = RwSignal::new(false);
    // The settings in force: the two that decide what is found, then any
    // other that is not at its default.
    let facts = move || {
        let p = state.call_detection_params();
        let d = CallScanParams::default();
        let mut f = vec![
            format!("Lowest bat frequency {} kHz", khz(p.min_bat_hz)),
            format!("Threshold {:.0} dB", p.threshold_db),
        ];
        if p.min_call_ms != d.min_call_ms {
            f.push(format!("Min duration {:.1} ms", p.min_call_ms));
        }
        if p.max_call_ms != d.max_call_ms {
            f.push(format!("Max duration {:.0} ms", p.max_call_ms));
        }
        if p.trains.max_gap_s != d.trains.max_gap_s {
            f.push(format!("Train gap {:.1} s", p.trains.max_gap_s));
        }
        if list_filter.get() == ListFilter::BatCalls {
            f.push("Listed: bat calls".to_string());
        }
        if !state.call_detection.overlay_enabled().get() {
            f.push("Boxes off".to_string());
        }
        f
    };

    // Under the settings line: the search's progress, or why it cannot run.
    let progress = move || {
        current_file.get()?;
        if state.call_detection.scanning().with(|s| s.is_some()) {
            let pct = (state.call_detection.progress().get() * 100.0).round();
            return Some(
                view! {
                    <div class="cd-notes">
                        <div class="cd-progress-text">{format!("Searching for bat calls\u{2026} {pct:.0}% done")}</div>
                        <div class="detection-progress">
                            <div class="detection-progress-fill" style=format!("width:{pct:.0}%")></div>
                        </div>
                    </div>
                }
                .into_any(),
            );
        }
        if !has_results.get() || state.call_detection.band_hz().get().is_some() {
            return None;
        }
        let params = state.call_detection_params();
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
        Some(
            view! {
                <div class="cd-notes">
                    <div class="detection-error">{format!(
                        "Sample rate too low: {} kHz. The search needs a sample rate above {} kHz.",
                        khz(rate as f32),
                        khz(min_rate)
                    )}</div>
                    {hint.map(|h| view! { <div class="detection-status-note">{h}</div> })}
                </div>
            }
            .into_any(),
        )
    };

    // The heading's right side: what was found.
    let head_fact = move || {
        if !has_results.get() || state.call_detection.band_hz().get().is_none() {
            return String::new();
        }
        if total_count.get() == 0 {
            return "Nothing detected".to_string();
        }
        format!(
            "{}, {}",
            count(bat_count.get(), "bat call", "bat calls"),
            state
                .call_detection
                .trains()
                .with(|t| count(t.len(), "call train", "call trains"))
        )
    };
    let notes = move || {
        if !has_results.get() {
            return None;
        }
        let (lo, hi) = state.call_detection.band_hz().get()?;
        let total = total_count.get();
        if total == 0 {
            return Some(
                view! {
                    <div class="cd-notes">
                        <div class="detection-status-note">{format!("Nothing detected from {} to {} kHz.", khz(lo), khz(hi))}</div>
                        <div class="detection-status-note">"Lower the threshold to find fainter sounds."</div>
                    </div>
                }
                .into_any(),
            );
        }
        let others = total - bat_count.get();
        (others > 0).then(|| {
            view! {
                <div class="cd-notes">
                    <div class="detection-status-note">{count(others, "other detection", "other detections")}</div>
                </div>
            }
            .into_any()
        })
    };

    // Outline on the spectrogram the regions a choice would add, while the
    // pointer is over it or the keyboard focus is on it. Touch has no hover,
    // so a tap only picks the choice.
    let show_preview = move |what: AnnotateWhat| {
        state
            .call_detection
            .preview()
            .set(Some(crate::call_annotations::AnnotatePreview {
                what,
                bat_calls_only: bat_calls_only.get_untracked(),
            }));
    };
    let hide_preview = move || state.call_detection.preview().set(None);
    // True while the preview shown is for the radio button with keyboard
    // focus. Only then does a blur clear the preview: clicking a radio button
    // also moves the focus, which blurs the button that had it, and the
    // choice under the pointer must keep its preview.
    let preview_from_focus = StoredValue::new(false);
    // A new search or another file clears the results and hides the
    // choices, so clear the preview too.
    Effect::new(move |_| {
        if !has_results.get() {
            hide_preview();
        }
    });
    on_cleanup(hide_preview);

    let n_to_add = move || {
        let (calls, trains) = to_add.get();
        calls + trains
    };
    // The inside of the "Add N annotations" fold.
    let annotate_body = move || {
        let reason = move || {
            (n_to_add() == 0).then(|| {
                let others = total_count.get() - bat_count.get();
                let text = if annotate_what.get() == AnnotateWhat::Calls {
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
                };
                view! { <div class="detection-status-note detection-annotate-row">{text}</div> }
            })
        };
        let note = move || {
            let (existing, existing_tags) = existing_auto.get();
            (existing > 0 && n_to_add() > 0).then(|| {
                let tags = existing_tags.join(" or ");
                let text = if existing == 1 {
                    format!(
                        "Replaces 1 annotation tagged {tags}, even if you edited it. Undo with {}.",
                        undo_keys()
                    )
                } else {
                    format!(
                        "Replaces {existing} annotations tagged {tags}, including any you edited. Undo with {}.",
                        undo_keys()
                    )
                };
                view! {
                    <div
                        class="detection-status-note detection-annotate-row"
                        title="Annotations without this tag are kept. Any that were inside a removed train move to the top level of the Annotations tab."
                    >{text}</div>
                }
            })
        };
        // Radio buttons in a column: the sidebar is too narrow for the three
        // options side by side.
        let option = move |value: AnnotateWhat, label: &'static str| {
            view! {
                <label
                    class="setting-label detection-radio"
                    on:pointerenter=move |ev| {
                        if ev.pointer_type() != "touch" {
                            preview_from_focus.set_value(false);
                            show_preview(value);
                        }
                    }
                    on:pointerleave=move |ev| {
                        if ev.pointer_type() != "touch" {
                            preview_from_focus.set_value(false);
                            hide_preview();
                        }
                    }
                >
                    <input
                        type="radio"
                        name="detection-annotate-what"
                        prop:checked=move || annotate_what.get() == value
                        on:change=move |_| annotate_what.set(value)
                        on:focus=move |ev| {
                            // Keyboard focus only; a click or tap also focuses.
                            if focus_visible(&ev) {
                                preview_from_focus.set_value(true);
                                show_preview(value);
                            }
                        }
                        on:blur=move |_| {
                            if preview_from_focus.get_value() {
                                preview_from_focus.set_value(false);
                                hide_preview();
                            }
                        }
                    />
                    {label}
                </label>
            }
        };
        view! {
            <div class="detection-annotate-row detection-radio-group">
                {option(AnnotateWhat::Calls, "Calls")}
                {option(AnnotateWhat::Trains, "Trains")}
                {option(AnnotateWhat::TrainsWithCalls, "Trains with nested calls")}
                {move || (annotate_what.get() == AnnotateWhat::TrainsWithCalls).then(|| view! {
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
            {reason}
            {note}
            <button
                class="psd-btn cd-add-btn"
                disabled=move || n_to_add() == 0
                on:click=move |_| add_annotations(state, annotate_what.get_untracked(), bat_calls_only.get_untracked())
            >{move || match n_to_add() {
                0 => "Add annotations".to_string(),
                n => format!("Add {}", count(n, "annotation", "annotations")),
            }}</button>
        }
    };
    let annotate_section = move || {
        // Only these two are tracked here, so the radio buttons are not
        // rebuilt (losing their hover and focus) when a choice is picked.
        if !has_results.get() || total_count.get() == 0 {
            return None;
        }
        let title = Signal::derive(move || match n_to_add() {
            0 => "Add annotations".to_string(),
            n => format!("Add {}", count(n, "annotation", "annotations")),
        });
        Some(view! {
            <div class="cd-folds">
                <Fold name="annotate" title=title>{annotate_body()}</Fold>
            </div>
        })
    };

    let list_section = move || {
        if !has_results.get() || total_count.get() == 0 {
            return None;
        }
        let filter = list_filter.get();
        let params = state.call_detection_params();
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
                        <tr
                            class=row_class
                            title=tooltip
                            on:click=move |_| on_row_click(i)
                            on:pointerenter=move |ev: web_sys::PointerEvent| {
                                if ev.pointer_type() == "mouse" {
                                    state.call_detection.hovered_index().set(Some(i));
                                }
                            }
                            on:pointerleave=move |_| {
                                if state.call_detection.hovered_index().get_untracked() == Some(i) {
                                    state.call_detection.hovered_index().set(None);
                                }
                            }
                        >
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
        Some(view! {
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
                    aria-label=label
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
    let filter_button = move |value: ListFilter, label: &'static str| {
        view! {
            <button
                class=move || if list_filter.get() == value { "psd-btn psd-btn-active" } else { "psd-btn" }
                aria-pressed=move || (list_filter.get() == value).to_string()
                on:click=move |_| list_filter.set(value)
            >{label}</button>
        }
    };
    let settings = move || {
        settings_open.get().then(|| {
            view! {
                <div class="cd-settings">
                    {slider(
                        "Lowest bat frequency",
                        "Detections with a peak frequency below this are marked Low frequency. Default 16 kHz. Lower it to find free-tailed bats near 11 kHz; expect more false detections from birds and insects.\nThis is the Lowest bat frequency setting from Settings, which the Loudest-frequency line, ZC and HFR also use.",
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
                    // A filter of the list only: the boxes show every detection.
                    <div class="setting-row">
                        <span class="setting-label">"Listed"</span>
                        <div class="psd-btn-group" role="group" aria-label="Listed">
                            {filter_button(ListFilter::BatCalls, "Bat calls")}
                            {filter_button(ListFilter::All, "All detections")}
                        </div>
                    </div>
                    <div class="setting-row detection-annotate-row">
                        <label class="setting-label">
                            <input
                                type="checkbox"
                                prop:checked=move || state.call_detection.overlay_enabled().get()
                                on:change=move |ev| state.call_detection.overlay_enabled().set(event_target_checked(&ev))
                            />
                            " Boxes on the spectrogram"
                        </label>
                    </div>
                </div>
            }
        })
    };

    view! {
        <div class="cd-inbuilt">
            {move || match current_file.get() {
                None => view! { <div class="cd-no-file">"No file selected"</div> }.into_any(),
                Some(_) => view! {
                    <div class="cd-action-row">
                        <div class="cd-action">{in_force_line(settings_open, facts)}</div>
                        {settings_button(settings_open)}
                    </div>
                }
                .into_any(),
            }}
            {progress}
            {settings}
            {move || has_results.get().then(|| view! {
                <div class="cd-section">
                    {section_head("In this recording", head_fact, String::new)}
                    {notes}
                    {list_section}
                </div>
            })}
            {annotate_section}
        </div>
    }
}

/// The methods, in the order the drop-down lists them.
const METHODS: [DetectionMethod; 6] = [
    DetectionMethod::Combined,
    DetectionMethod::Inbuilt,
    DetectionMethod::BirdNet,
    DetectionMethod::BirdNetV3,
    DetectionMethod::Perch,
    DetectionMethod::Bats,
];

/// The name the method drop-down shows. The classifiers' names are in
/// `classifier::MethodInfo`.
fn method_name(method: DetectionMethod) -> &'static str {
    match method.classifier() {
        None if method == DetectionMethod::Combined => "Birds and bats",
        None => "In-built bat call detection",
        Some(m) => m.info().name,
    }
}

/// The line under the method's name in the drop-down.
fn method_description(method: DetectionMethod) -> &'static str {
    match method.classifier() {
        None if method == DetectionMethod::Combined => {
            // The models it uses are listed under its action button.
            "Draws a box round each bird and bat call and names the species."
        }
        None => "Finds bat calls. Runs automatically.",
        Some(m) => m.info().description,
    }
}

/// The method drop-down: a button with the picked method's name, and a menu
/// of the methods with their descriptions. The menu sits over the panel
/// inside the sidebar (not in a portal), so it is in the sidebar's layer on
/// phones too. It closes on a pick, on Escape and on a press outside it.
#[component]
fn MethodPicker() -> impl IntoView {
    let state = expect_context::<AppState>();
    let method = move || state.call_detection.method().get();
    let open = RwSignal::new(false);
    let wrap = NodeRef::<leptos::html::Div>::new();
    let outside = window_event_listener(leptos::ev::pointerdown, move |ev| {
        use wasm_bindgen::JsCast;
        if !open.get_untracked() {
            return;
        }
        let inside = wrap.get_untracked().is_some_and(|w| {
            let w: &web_sys::Node = w.as_ref();
            ev.target()
                .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                .is_some_and(|t| w.contains(Some(&t)))
        });
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || outside.remove());
    let on_key = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Escape" && open.get_untracked() {
            ev.stop_propagation();
            open.set(false);
        }
    };
    let options = METHODS
        .iter()
        .map(|&value| {
            let (label, help) = (method_name(value), method_description(value));
            let sel = move || method() == value;
            view! {
                <button
                    class="layer-panel-opt cd-method-opt"
                    class:sel=sel
                    role="option"
                    aria-selected=move || sel().to_string()
                    on:click=move |_| {
                        state.call_detection.method().set(value);
                        state.call_detection.selected_window().set(None);
                        open.set(false);
                    }
                >
                    <span class="preset-main">{label}</span>
                    <span class="preset-sub">{help}</span>
                </button>
            }
        })
        .collect::<Vec<_>>();
    view! {
        <div class="cd-method" node_ref=wrap on:keydown=on_key>
            <button
                class="cd-method-btn"
                aria-haspopup="listbox"
                aria-expanded=move || open.get().to_string()
                title="Method"
                on:click=move |_| open.update(|v| *v = !*v)
            >
                <span class="cd-method-name">{move || method_name(method())}</span>
                <span class="cd-caret">{"\u{25BE}"}</span>
            </button>
            <div
                class="layer-panel cd-method-menu"
                role="listbox"
                aria-label="Method"
                style:display=move || if open.get() { "" } else { "none" }
            >
                {options}
            </div>
            <div class="detection-status-note cd-method-help">{move || method_description(method())}</div>
        </div>
    }
}

/// The Call detection tab: a choice of method, then that method's panel.
#[component]
pub(crate) fn CallDetectionPanel() -> impl IntoView {
    let state = expect_context::<AppState>();
    let method = move || state.call_detection.method().get();

    // What "Auto" gives the current recording, and where that came from.
    let auto = Memo::new(move |_| bat_pick::current_auto_scope(state));
    let region_select = move || {
        // BattyBirdNET's region: for it on its own, and for the combined
        // scan of a recording it runs on (96 kHz or more).
        let ultrasonic = move || {
            let idx = state.library.current_index().get();
            state.library.files().with(|files| {
                idx.and_then(|i| files.get(i)).is_some_and(|f| {
                    f.audio.sample_rate >= oversample_ipc::classifier::BATS_MIN_SOURCE_RATE
                })
            })
        };
        let shown = method() == DetectionMethod::Bats
            || (method() == DetectionMethod::Combined && ultrasonic());
        shown.then(|| {
            let busy = move || state.classifier.run().with(|r| r.is_busy());
            let pick = move || state.call_detection.bat_pick().get();
            // Option values: "auto", "all", or a region's place in `BatRegion::ALL`.
            let fixed = move |scope: BatScope, value: String, species: usize| {
                view! {
                    <option
                        value=value
                        prop:selected=move || pick() == BatPick::Fixed(scope)
                    >{format!("{}: {species} species", scope.label())}</option>
                }
            };
            view! {
                <label class="detection-region">
                    <span class="setting-label">"Region"</span>
                    <select
                        class="setting-select"
                        prop:disabled=busy
                        on:change=move |ev| {
                            let value = event_target_select_value(&ev);
                            let new = match value.as_str() {
                                "auto" => Some(BatPick::Auto),
                                "all" => Some(BatPick::Fixed(BatScope::All)),
                                i => i
                                    .parse::<usize>()
                                    .ok()
                                    .and_then(|i| BatRegion::ALL.get(i))
                                    .map(|r| BatPick::Fixed(BatScope::Region(*r))),
                            };
                            if let Some(new) = new {
                                state.call_detection.bat_pick().set(new);
                            }
                        }
                    >
                        <option value="auto" prop:selected=move || pick() == BatPick::Auto>
                            {move || format!("Auto: {}", auto.get().scope.label())}
                        </option>
                        {fixed(BatScope::All, "all".to_string(), ALL_REGIONS_SPECIES)}
                        {BatRegion::ALL
                            .iter()
                            .enumerate()
                            .map(|(i, r)| fixed(BatScope::Region(*r), i.to_string(), region_species(*r)))
                            .collect::<Vec<_>>()}
                    </select>
                    // Why Auto picked this region.
                    {move || (pick() == BatPick::Auto).then(|| view! {
                        <span class="detection-status-note">{auto.get().note}</span>
                    })}
                </label>
            }
        })
    };

    view! {
        <div class="sidebar-panel cd-panel">
            <div class="cd-top">
                <MethodPicker />
                {region_select}
            </div>
            {move || match method().classifier() {
                None if method() == DetectionMethod::Combined => {
                    view! { <combined::CombinedSection /> }.into_any()
                }
                None => view! { <InbuiltDetection /> }.into_any(),
                Some(m) => view! { <ClassifierSection method=m /> }.into_any(),
            }}
        </div>
    }
}

/// Species in the 11 regional label files together, by scientific name
/// (`merge_bat_labels` in `oversample-ipc`).
const ALL_REGIONS_SPECIES: usize = 74;

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

/// True when the focus event's target shows a focus ring (`:focus-visible`):
/// the keyboard moved the focus, not a click or tap.
fn focus_visible(ev: &web_sys::FocusEvent) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .and_then(|el| el.matches(":focus-visible").ok())
        .unwrap_or(false)
}

pub(super) fn event_target_checked(ev: &web_sys::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.checked())
        .unwrap_or(false)
}

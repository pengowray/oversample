//! Bird sound boxes in the BirdNET+ V3.0 panel (`crate::classifier::boxes`):
//! the setting and its two options behind the settings button, the part of
//! the line of settings in force, the row in "In this recording", and the
//! note when the file was analysed without boxes. Nothing here is shown for
//! the other methods.

use super::event_target_checked;
use super::event_target_value;
use crate::classifier::{self, boxes, catalog, DownloadStatus};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;

/// True when the open recording gets no boxes (a heterodyne recording).
fn file_excluded(state: AppState, file: Option<u64>) -> bool {
    let Some(file) = file else {
        return false;
    };
    state.library.files().with(|fs| {
        fs.iter()
            .find(|f| f.id == file)
            .is_some_and(|f| !boxes::applies_to(f))
    })
}

/// The settings: "Draw a box round each bird sound", and while it is on the
/// minimum box confidence and the box size, nested under it.
pub(super) fn settings_rows(state: AppState, current_file: Memo<Option<u64>>) -> impl IntoView {
    let cd = state.call_detection;
    let on = move || cd.bird_boxes().get();
    let size_button = move |calls: bool, text: &'static str, help: &'static str| {
        let active = move || cd.box_calls().get() == calls;
        view! {
            <button
                class=move || if active() { "psd-btn psd-btn-active" } else { "psd-btn" }
                aria-pressed=move || active().to_string()
                title=help
                on:click=move |_| cd.box_calls().set(calls)
            >{text}</button>
        }
    };
    view! {
        <div class="setting-row detection-annotate-row" title="Also runs BirdBox Just-Bird to draw a box round each bird sound, named by BirdNET+ V3.0. One-off 21 MB download; Analyse takes about 2.5 times as long.">
            <label class="setting-label">
                <input
                    type="checkbox"
                    prop:checked=on
                    on:change=move |ev| {
                        let v = event_target_checked(&ev);
                        cd.bird_boxes().set(v);
                        crate::settings::set_bool(crate::settings::keys::BIRD_BOXES, v);
                    }
                />
                " Draw a box round each bird sound"
            </label>
        </div>
        {move || (on() && file_excluded(state, current_file.get())).then(|| view! {
            <div class="setting-row detection-status-note cd-box-sub">
                "Bird sound boxes are not made for heterodyne recordings like this one."
            </div>
        })}
        {move || on().then(|| view! {
            <div class="setting-row cd-box-sub">
                <span class="setting-label">"Minimum box confidence"</span>
                <span class="setting-value">{move || format!("{:.2}", cd.box_min_confidence().get())}</span>
            </div>
            <div class="setting-row cd-box-sub">
                <input
                    type="range"
                    class="setting-range"
                    aria-label="Minimum box confidence"
                    min="0.05"
                    max="0.95"
                    step="0.05"
                    prop:value=move || cd.box_min_confidence().get().to_string()
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                            cd.box_min_confidence().set(v);
                        }
                    }
                />
            </div>
            <div class="setting-row cd-box-sub">
                <span class="setting-label">"One box per"</span>
                <div class="psd-btn-group" role="group" aria-label="One box per">
                    {size_button(true, "Call", "A box round each call.")}
                    {size_button(false, "Phrase", "Calls in the same frequency band less than 0.05 s apart become one box.")}
                </div>
            </div>
        })}
    }
}

/// The parts of the line of settings in force, while boxes are on:
/// "One box per phrase" and "Min box confidence 0.30", in the style of
/// "Min confidence 0.25".
pub(super) fn facts(state: AppState) -> Vec<String> {
    let cd = state.call_detection;
    if !cd.bird_boxes().get() {
        return Vec::new();
    }
    let per = if cd.box_calls().get() {
        "call"
    } else {
        "phrase"
    };
    vec![
        format!("One box per {per}"),
        format!("Min box confidence {:.2}", cd.box_min_confidence().get()),
    ]
}

/// How many boxes there are with the settings now, the best confidence and
/// the box that has it (start, end, lowest frequency).
#[derive(Clone, Copy, PartialEq)]
struct Found {
    count: usize,
    best: f32,
    best_box: Option<(f64, f64, f64)>,
}

fn found(state: AppState, file: Option<u64>) -> Option<Found> {
    let cd = state.call_detection;
    if !cd.bird_boxes().get() {
        return None;
    }
    let file = file?;
    let rate = state.library.files().with(|fs| {
        fs.iter()
            .find(|f| f.id == file)
            .filter(|f| boxes::applies_to(f))
            .map(|f| f.audio.sample_rate)
    })?;
    let min = cd.box_min_confidence().get();
    let gap = boxes::phrase_gap(cd.box_calls().get());
    state.classifier.boxes().with(|m| {
        let run = m.get(&file).filter(|b| b.source_rate == rate)?;
        let merged = run.merged(min, gap);
        let best = merged
            .iter()
            .max_by(|a, b| a.max_conf.total_cmp(&b.max_conf));
        Some(Found {
            count: merged.len(),
            best: best.map_or(0.0, |b| b.max_conf as f32),
            best_box: best.map(|b| (b.t0, b.t1, b.f0)),
        })
    })
}

/// The row in "In this recording": "Bird sounds: 56 boxes" and the best box
/// confidence. A press shows the box with the highest confidence and its
/// card. `cols` is the number of columns before the pin.
pub(super) fn list_row(
    state: AppState,
    current_file: Memo<Option<u64>>,
    cols: usize,
) -> impl IntoView {
    let found = Memo::new(move |_| found(state, current_file.get()));
    move || {
        let f = found.get()?;
        let text = format!("Bird sounds: {}", super::count(f.count, "box", "boxes"));
        let show = move |_| {
            if let Some((t0, t1, f0)) = f.best_box {
                super::classifier_section::show_window(state, t0, t1);
                state.call_detection.selected_window().set(None);
                state.call_detection.picked_box().set(Some((t0, f0)));
            }
        };
        Some(view! {
            <tr
                class="detection-row"
                title="Boxes drawn by BirdBox Just-Bird. Best is the highest box confidence. Press to scroll to the box with the highest confidence."
                on:click=show
            >
                <td class="detection-col-kind" colspan=(cols - 1).to_string()>
                    <span class="cd-name-line">
                        <span class="map-key cd-swatch cd-swatch-box"></span>
                        <span>{text}</span>
                    </span>
                </td>
                <td class="detection-col-num">{if f.count > 0 { format!("{:.2}", f.best) } else { "\u{2013}".to_string() }}</td>
                <td class="cd-col-pin"></td>
            </tr>
        })
    }
}

/// The note when boxes are on but the file's last Analyse found none:
/// analysed without them.
pub(super) fn analyse_again_note(state: AppState, file: Option<u64>) -> Option<&'static str> {
    if !state.call_detection.bird_boxes().get() || file_excluded(state, file) {
        return None;
    }
    let file = file?;
    let has = state.classifier.boxes().with(|m| m.contains_key(&file));
    (!has).then_some("Analyse again to add bird sound boxes to this file.")
}

/// The bird box model's download, while it is going: "Downloading the bird
/// box model: 5 of 21 MB".
pub(super) fn download_text(state: AppState) -> Option<String> {
    let f = &catalog::JUST_BIRD;
    let status = state.classifier.downloads().with(|m| m.get(f.id).cloned());
    match status {
        Some(DownloadStatus::Downloading { fraction }) => Some(format!(
            "Downloading the bird box model: {:.0} of {:.0} MB",
            fraction * f.size as f64 / 1e6,
            f.size as f64 / 1e6
        )),
        _ => None,
    }
}

/// True when the bird box model is on this device. Reactive.
pub(super) fn model_stored(state: AppState) -> bool {
    classifier::files_ready(state, &[&catalog::JUST_BIRD])
}

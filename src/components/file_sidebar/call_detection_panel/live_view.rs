//! "At the play position" in the Call detection tab, for BirdNET V2.4 and
//! Perch 2.0: the model's top classes for the audio at the play position (a
//! few times a second while a file plays, or the window Play starts with
//! while it is paused), the strip of the last 20 s, and in the "Map and
//! components" fold the map and the component maps that
//! `crate::live_embedding` draws.
//!
//! The readings run while this section is shown and a file is open or the
//! microphone is on; leaving the tab stops them. There is no switch: the
//! owner asked (2026-11-21) for the play position to always be the top of
//! the results. The switch it replaced was never saved, so there was no
//! stored choice to carry over.

use super::parts::{pin_button, section_head, Fold};
use super::rows::{component_picked, species_swatch, strip_picked, toggle_component, toggle_strip};
use crate::classifier::{DownloadStatus, Method, MethodFacts};
use crate::format_time::format_time_display;
use crate::live_embedding::{self, LiveStatus, Readout};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use std::cell::{Cell, RefCell};

thread_local! {
    /// The sections shown now. Picking the other model builds a new section
    /// before the old one goes, so the readings follow a count rather than
    /// each section turning them on and off.
    static SHOWN: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
}

fn set_shown(state: AppState, id: u32, on: bool) {
    let any = SHOWN.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|&x| x != id);
        if on {
            s.push(id);
        }
        !s.is_empty()
    });
    live_embedding::set_user(state, live_embedding::LIVE_VIEW, any);
}

#[component]
pub(super) fn LiveView(
    /// The method picked in the tab, one with a live model
    /// (`MethodInfo::live_model`).
    method: Method,
    current_file: Memo<Option<u64>>,
    /// The file has results for this method (for the Species map button).
    has_results: Memo<bool>,
    /// Opens the Species map dialog (drawn by the classifier section).
    map_open: RwSignal<bool>,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let model = live_embedding::model_for(method);
    live_embedding::set_picked(method);
    // The name the method drop-down shows.
    let method_name = method.info().name;
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id.wrapping_add(1));
        id
    });
    // Readings while a file is open or the microphone is on.
    let wanted = Memo::new(move |_| {
        current_file.get().is_some() || state.mic.listening().get() || state.mic.recording().get()
    });
    Effect::new(move |_| set_shown(state, id, wanted.get()));
    on_cleanup(move || set_shown(state, id, false));

    // Start once the model has been downloaded.
    Effect::new(move |_| {
        let ready = state.classifier.downloads().with(|d| {
            model
                .parts
                .iter()
                .chain(&model.labels)
                .all(|f| matches!(d.get(f.id), Some(DownloadStatus::Downloaded)))
        });
        let waiting = state
            .classifier
            .live()
            .with_untracked(|s| *s == LiveStatus::NeedsDownload);
        if ready && waiting {
            live_embedding::restart(state);
        }
    });

    // The strip goes in `chart_ref`, the map and component maps in
    // `map_ref`, inside the fold.
    let chart_ref = NodeRef::<leptos::html::Div>::new();
    let map_ref = NodeRef::<leptos::html::Div>::new();
    let drawn = StoredValue::new_local(None::<web_sys::HtmlElement>);
    let pins_on = live_embedding::component_pins_apply(method);
    let send_pins = move |el: &web_sys::HtmlElement| {
        let json = pins_on.then(|| {
            let n = crate::components::score_strips::components_for(method)
                .map_or(12, |p| p.variance.len());
            let pins: Vec<bool> = (0..n).map(|k| component_picked(state, k)).collect();
            serde_json::json!(pins).to_string()
        });
        let json = json.map_or(wasm_bindgen::JsValue::NULL, |s| s.into());
        live_embedding::call("setPins", &[el, &json]);
    };
    Effect::new(move |_| {
        let (Some(el), Some(host)) = (chart_ref.get(), map_ref.get()) else {
            return;
        };
        let el = web_sys::HtmlElement::from(el);
        let host = web_sys::HtmlElement::from(host);
        live_embedding::call("setMapHost", &[&el, &host]);
        untrack(|| send_pins(&el));
        live_embedding::set_chart(Some(el.clone()));
        drawn.set_value(Some(el));
    });
    // The pins on the component maps follow the picks, without waiting for
    // a reading (a paused file sends none).
    Effect::new(move |_| {
        state.call_detection.strip_components().track();
        if let Some(el) = drawn.get_value() {
            send_pins(&el);
        }
    });
    on_cleanup(move || {
        if let Some(el) = drawn.get_value() {
            live_embedding::clear_chart(&el);
            live_embedding::call("dispose", &[&el]);
        }
    });

    // "Find recordings" beside a species in the map's list: live-embedding.js
    // sends the species' scientific name in an `oversample-find-species` event.
    let on_find = window_event_listener(
        leptos::ev::Custom::<web_sys::CustomEvent>::new("oversample-find-species"),
        move |ev| {
            if let Some(name) = ev.detail().as_string() {
                crate::components::find_recordings::open_with_search(state, &name);
            }
        },
    );
    on_cleanup(move || on_find.remove());
    // The pin on a component map.
    let on_pin = window_event_listener(
        leptos::ev::Custom::<web_sys::CustomEvent>::new("oversample-pc-pin"),
        move |ev| {
            if let Some(k) = ev.detail().as_f64() {
                toggle_component(state, k as usize);
            }
        },
    );
    on_cleanup(move || on_pin.remove());

    let readout = live_embedding::readout();
    let (readout1, readout2, readout3) = (readout.clone(), readout.clone(), readout);
    let sound_on = move || {
        state.playback.is_playing().get()
            || state.mic.listening().get()
            || state.mic.recording().get()
    };
    // Why there is no reading to show, or what is happening instead.
    let status = move || match state.classifier.live().get() {
        LiveStatus::Off => None,
        LiveStatus::NeedsDownload => Some(format!(
            "Download {} to see results for the play position.",
            method_name
        )),
        LiveStatus::Loading => Some(format!("Loading {}\u{2026}", method_name)),
        LiveStatus::Failed(e) => Some(format!("Readings at the play position stopped: {e}")),
        LiveStatus::Ready { .. } if readout1.with(Option::is_some) => None,
        LiveStatus::Ready { .. } if sound_on() => Some("Starting\u{2026}".to_string()),
        LiveStatus::Ready { .. } if current_file.get().is_some() => None,
        LiveStatus::Ready { .. } => Some("Play a file or listen with the microphone.".to_string()),
    };
    // The heading's right side: the part of the file a paused reading is of,
    // or where the playhead is.
    let where_ = move || {
        if let Some((a, b)) = readout2.with(|r| r.as_ref().and_then(|r| r.range)) {
            return format!(
                "{} to {}",
                format_time_display(a, 1),
                format_time_display(b, 1)
            );
        }
        if state.playback.is_playing().get() && current_file.get().is_some() {
            return format_time_display(state.playback.playhead_time().get(), 1);
        }
        String::new()
    };
    let readout4 = readout3.clone();
    let readout5 = readout3.clone();
    let picked = move || readout4.with(|r| r.as_ref().is_some_and(|r| r.picked));
    let head = move || {
        if readout5.with(|r| r.as_ref().is_some_and(|r| r.picked)) {
            "In the picked window"
        } else {
            "At the play position"
        }
    };
    let where_title = move || match state.classifier.live().get() {
        LiveStatus::Ready {
            interval_ms,
            model_ms,
        } if interval_ms > 0.0 && sound_on() => {
            format!(
                    "The last {} s of audio, updated every {:.2} s. {} takes {model_ms:.0} ms for each reading on this device.",
                    model.window_s,
                    interval_ms / 1000.0,
                    method_name
                )
        }
        _ if picked() => "The window you clicked (yellow ring on the map)".to_string(),
        _ => format!(
            "The {} s from where Play starts (blue triangle on the map)",
            model.window_s
        ),
    };

    // The reading's top classes, in the row format of "In this recording".
    let rows = move || {
        let r: Readout = readout3.get()?;
        let min = crate::classifier::min_confidence(state, method);
        let order = state.classifier.results().with(|m| {
            current_file
                .get()
                .and_then(|f| m.get(&(f, method)).map(|r| (f, r)))
                .map(|(f, r)| crate::classifier::kept_order::colour_order(f, method, r, min))
                .unwrap_or_default()
        });
        let below = r.top.first().is_some_and(|c| c.score < min);
        let rows = r
            .top
            .iter()
            .map(|c| {
                let slot = order.iter().position(|&o| o == c.class);
                let muted = c.score < min;
                let pin = if c.is_species && !c.scientific.is_empty() {
                    let sci = c.scientific.clone();
                    let sci2 = sci.clone();
                    pin_button(
                        c.common.clone(),
                        move || strip_picked(state, method, &sci),
                        move || toggle_strip(state, method, sci2.clone()),
                    )
                    .into_any()
                } else {
                    ().into_any()
                };
                view! {
                    <tr class=if muted { "detection-row-other" } else { "" } title=c.scientific.clone()>
                        <td class="detection-col-kind">
                            <span class="cd-name-line">
                                {species_swatch(slot)}
                                <span>{c.common.clone()}</span>
                                {c.unlikely.then(super::rows::unlikely_flag)}
                            </span>
                        </td>
                        <td class="detection-col-num" title="Confidence">{format!("{:.2}", c.score)}</td>
                        <td class="cd-col-pin">{pin}</td>
                    </tr>
                }
            })
            .collect::<Vec<_>>();
        let note = below.then(|| {
            let mut t = format!("No species at {min:.2} or above");
            if r.ultrasonic {
                t.push_str(". ");
                t.push_str(model.hearing);
            }
            view! { <div class="detection-status-note cd-live-note">{t}</div> }
        });
        Some(view! {
            {note}
            <table class="detection-table cd-list cd-live-rows">
                <tbody>{rows}</tbody>
            </table>
        })
    };

    // Opening the fold draws the map at once (a paused file sends no new
    // readings).
    let on_fold = Callback::new(move |open: bool| {
        if open {
            request_animation_redraw(state);
        }
    });

    view! {
        <div class="live-emb cd-section">
            {section_head(head, where_, where_title)}
            {move || status().map(|s| view! { <div class="detection-status-note cd-live-note">{s}</div> })}
            <div class="cd-live-readout" class:playing=sound_on>{rows}</div>
            // Hidden until the model is loaded: before that it would show
            // an empty frame. Hidden, not removed, so the element registered
            // with live_embedding stays the same.
            <div
                class="live-emb-chart"
                node_ref=chart_ref
                style:display=move || {
                    if matches!(state.classifier.live().get(), LiveStatus::Ready { .. }) {
                        ""
                    } else {
                        "none"
                    }
                }
            ></div>
            <Fold name="map" title="Map and components".to_string() tag="Experimental" keep=true on_toggle=on_fold>
                <div class="live-emb-maphost" node_ref=map_ref></div>
                {move || (has_results.get() && crate::classifier::species_map::map_spec(method).is_some()).then(|| view! {
                    <div class="cd-map-btn-row">
                        <button class="psd-btn" on:click=move |_| map_open.set(true)>"Species map"</button>
                    </div>
                })}
            </Fold>
        </div>
    }
}

/// Redraw after the fold has opened and the map's box has its size.
fn request_animation_redraw(state: AppState) {
    use wasm_bindgen::JsCast;
    let cb = wasm_bindgen::closure::Closure::once_into_js(move || live_embedding::redraw(state));
    if let Some(w) = web_sys::window() {
        let _ = w.request_animation_frame(cb.unchecked_ref());
    }
}

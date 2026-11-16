//! The species map dialog: a model's classes (BirdNET's or Perch's) as faint
//! points, placed so that classes the model hears as alike sit together
//! (`research/species-map`),
//! with this file's windows as highlighted dots. Drawn by `species-map.js`
//! with Apache ECharts; positions come from `crate::classifier::species_map`.

use super::classifier_section::show_window;
use crate::classifier::species_map::{map_spec, parse_map, place_windows, MapSpec};
use crate::classifier::summary::{window_kind, WindowKind};
use crate::classifier::{self, location, ClassifierRun, Method};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

type Map = Arc<Vec<Option<(f32, f32)>>>;

thread_local! {
    /// Loaded maps, by file name.
    static MAPS: std::cell::RefCell<Vec<(&'static str, Map)>> = const { std::cell::RefCell::new(Vec::new()) };
}

async fn load_map(spec: &'static MapSpec) -> Result<Map, String> {
    let cached = MAPS.with(|m| {
        m.borrow()
            .iter()
            .find(|(f, _)| *f == spec.file)
            .map(|(_, m)| m.clone())
    });
    if let Some(m) = cached {
        return Ok(m);
    }
    let bytes = super::super::fetch_bytes(spec.file).await?;
    let m = Arc::new(parse_map(&bytes));
    MAPS.with(|c| c.borrow_mut().push((spec.file, m.clone())));
    Ok(m)
}

/// The model's classes as `species-map.js` draws them: `[x, y, common,
/// scientific]`, or `["-", "-", "", ""]` (not drawn) for a class with no position.
fn species_json(run: &ClassifierRun, map: &[Option<(f32, f32)>]) -> serde_json::Value {
    let species: Vec<_> = map
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let Some((x, y)) = *p else {
                return serde_json::json!(["-", "-", "", ""]);
            };
            let l = run.labels.get(i);
            serde_json::json!([
                round4(x),
                round4(y),
                l.map_or("", |l| l.common.as_str()),
                l.map_or("", |l| l.scientific.as_str()),
            ])
        })
        .collect();
    serde_json::json!(species)
}

fn round4(v: f32) -> f32 {
    (v * 10_000.0).round() / 10_000.0
}

/// The parts of the map that change while a run is in progress: the found
/// species, the windows' dots and the selection.
///
/// `unlikely` says whether a class is a bird unlikely at the recording's
/// place and month; those get a flag after their name.
fn windows_json(
    run: &ClassifierRun,
    map: &[Option<(f32, f32)>],
    radius: f32,
    min: f32,
    selected: Option<usize>,
    unlikely: &dyn Fn(u16) -> bool,
) -> serde_json::Value {
    let placed = place_windows(run, map, radius, min);
    let mut found: Vec<u16> = placed
        .iter()
        .map(|p| run.windows[p.index].top[0].class)
        .collect();
    found.sort_unstable();
    found.dedup();
    let unlikely_found: Vec<u16> = found.iter().copied().filter(|&c| unlikely(c)).collect();
    let windows: Vec<_> = placed
        .iter()
        .map(|p| {
            let w = &run.windows[p.index];
            let top: Vec<_> = w
                .top
                .iter()
                .take(3)
                .map(|s| {
                    (
                        run.label(s).map_or("Unknown", |l| l.common.as_str()),
                        (s.confidence * 100.0).round() / 100.0,
                        unlikely(s.class),
                    )
                })
                .collect();
            (round4(p.x), round4(p.y), p.index, w.start_s, w.end_s, top)
        })
        .collect();
    serde_json::json!({
        "found": found,
        "unlikely": unlikely_found,
        "windows": windows,
        "selected": selected,
    })
}

fn js_api() -> Option<JsValue> {
    js_sys::Reflect::get(&js_sys::global(), &"oversampleSpeciesMap".into()).ok()
}

fn call(name: &str, args: &[&JsValue]) -> Option<JsValue> {
    let api = js_api()?;
    let f: js_sys::Function = js_sys::Reflect::get(&api, &name.into())
        .ok()?
        .dyn_into()
        .ok()?;
    let a = js_sys::Array::new();
    for v in args {
        a.push(v);
    }
    f.apply(&api, &a).ok()
}

#[component]
pub(super) fn SpeciesMapDialog(
    open: RwSignal<bool>,
    current_file: Memo<Option<u64>>,
    method: Method,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let spec = map_spec(method).expect("species map opened for a model without one");
    let chart_ref = NodeRef::<leptos::html::Div>::new();
    // None while loading; then the map, or why it could not load.
    let map: RwSignal<Option<Result<Map, String>>> = RwSignal::new(None);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    // True while a species in the results is flagged, so the legend names the flag.
    let any_unlikely = RwSignal::new(false);
    spawn_local(async move {
        let m = load_map(spec).await;
        map.set(Some(m));
    });

    // A click on a window's dot picks it, as a click in the lists does.
    let on_pick = Closure::<dyn Fn(u32)>::new(move |i: u32| {
        let i = i as usize;
        let Some(file) = current_file.get_untracked() else {
            return;
        };
        let times = state.classifier.results().with_untracked(|m| {
            m.get(&(file, method))
                .and_then(|r| r.windows.get(i))
                .map(|w| (w.start_s, w.end_s))
        });
        if let Some((s, e)) = times {
            show_window(state, s, e, i);
        }
    })
    .into_js_value();

    let drawn: StoredValue<Option<web_sys::HtmlElement>, LocalStorage> =
        StoredValue::new_local(None);
    // The file and windows JSON last sent to the chart.
    let last: StoredValue<Option<(u64, String)>, LocalStorage> = StoredValue::new_local(None);
    Effect::new(move |_| {
        let Some(el) = chart_ref.get() else { return };
        let Some(Ok(m)) = map.get() else { return };
        let Some(file) = current_file.get() else {
            return;
        };
        let min = classifier::min_confidence(state, method);
        let selected = state.call_detection.selected_window().get();
        // Results arrive in batches while a run is going; most batches change
        // nothing on the map, so redraw only when what is drawn changes, and
        // send the species points only when the file changes.
        let json = state.classifier.results().with(|r| {
            let run = r.get(&(file, method))?;
            let unlikely = |class: u16| {
                run.labels
                    .get(class as usize)
                    .is_some_and(|l| location::unlikely_here(state, file, method, l))
            };
            let mut d = windows_json(run, &m, spec.radius, min, selected, &unlikely);
            let any = d["unlikely"].as_array().is_some_and(|a| !a.is_empty());
            if any_unlikely.get_untracked() != any {
                any_unlikely.set(any);
            }
            let key = (file, d.to_string());
            if last.with_value(|l| l.as_ref() == Some(&key)) {
                return None;
            }
            let new_file = last.with_value(|l| l.as_ref().map(|l| l.0)) != Some(file);
            d["species"] = if new_file {
                species_json(run, &m)
            } else {
                serde_json::Value::Null
            };
            last.set_value(Some(key));
            Some(d.to_string())
        });
        let Some(json) = json else { return };
        let el: web_sys::HtmlElement = el.into();
        drawn.set_value(Some(el.clone()));
        let promise = call("render", &[&el, &JsValue::from_str(&json), &on_pick]);
        if let Some(p) = promise.and_then(|p| p.dyn_into::<js_sys::Promise>().ok()) {
            spawn_local(async move {
                if let Err(e) = wasm_bindgen_futures::JsFuture::from(p).await {
                    let msg = e
                        .dyn_ref::<js_sys::Error>()
                        .map(|e| String::from(e.message()))
                        .unwrap_or_else(|| "unknown error".into());
                    error.set(Some(msg));
                }
            });
        }
    });
    on_cleanup(move || {
        if let Some(el) = drawn.get_value() {
            call("dispose", &[&el]);
        }
    });

    let close = move || open.set(false);
    let keys = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" {
            close();
        }
    });
    on_cleanup(move || keys.remove());

    // Legend numbers from this run: how many windows are drawn, and whether
    // the selected window is one of them (only then is an amber dot shown).
    let drawn_windows = move || {
        let file = current_file.get()?;
        let min = classifier::min_confidence(state, method);
        let selected = state.call_detection.selected_window().get();
        state.classifier.results().with(|r| {
            r.get(&(file, method)).map(|run| {
                let shown = |i: usize| window_kind(run, i, min) == WindowKind::Species;
                let n = (0..run.windows.len()).filter(|&i| shown(i)).count();
                (
                    n,
                    min,
                    selected.is_some_and(|i| i < run.windows.len() && shown(i)),
                )
            })
        })
    };
    let windows_label = move || {
        drawn_windows().map(|(n, min, _)| {
            format!(
                "{} at {min:.2} or above, each beside its top species",
                super::count(n, "window", "windows")
            )
        })
    };
    let selected_shown = move || drawn_windows().is_some_and(|(_, _, s)| s);
    let status = move || match (map.get(), error.get()) {
        (_, Some(e)) | (Some(Err(e)), _) => Some(format!("Could not load the map: {e}")),
        (None, _) => Some("Loading the map\u{2026}".to_string()),
        _ => None,
    };

    view! {
        <div
            class="modal-overlay"
            on:click=move |ev| {
                if ev.target() == ev.current_target() {
                    close();
                }
            }
        >
            <div class="modal-dialog species-map-dialog" role="dialog" aria-label="Species map">
                <div class="modal-header">
                    <span class="modal-title">"Species map"</span>
                    <span class="species-map-actions">
                        <button
                            class="psd-btn"
                            on:click=move |_| {
                                if let Some(el) = drawn.get_value() {
                                    call("resetZoom", &[&el]);
                                }
                            }
                        >"Reset zoom"</button>
                        <button class="modal-close" title="Close (Esc)" aria-label="Close" on:click=move |_| close()>"\u{00D7}"</button>
                    </span>
                </div>
                <div class="species-map-body">
                    <div class="species-map-chart" node_ref=chart_ref></div>
                    {move || status().map(|s| view! { <div class="species-map-status">{s}</div> })}
                </div>
                <div class="species-map-notes">
                    <ul class="species-map-legend">
                        <li><span class="map-key map-key-species"></span>{spec.classes}</li>
                        <li><span class="map-key map-key-found"></span>"Species in the results"</li>
                        <li><span class="map-key map-key-window"></span>{windows_label}</li>
                        <Show when=move || any_unlikely.get()>
                            <li><span class="unlikely-flag map-key-flag">"\u{2691}"</span>"Unlikely at this place and month"</li>
                        </Show>
                        <Show when=selected_shown>
                            <li><span class="map-key map-key-selected"></span>"Selected window"</li>
                        </Show>
                    </ul>
                    <p>{format!("Species that {} hears as alike sit close together. Scroll or pinch to zoom. Click a window to select it.", spec.model)}</p>
                    <p class="detection-status-note">{spec.source}</p>
                </div>
            </div>
        </div>
    }
}

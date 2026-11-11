//! The species map dialog: BirdNET's 6,522 classes as faint points, placed so
//! that classes the model hears as alike sit together (`research/species-map`),
//! with this file's windows as highlighted dots. Drawn by `species-map.js`
//! with Apache ECharts; positions come from `crate::classifier::species_map`.

use super::classifier_section::show_window;
use crate::classifier::species_map::{parse_map, place_windows};
use crate::classifier::summary::{window_kind, WindowKind};
use crate::classifier::{ClassifierRun, Method};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

type Map = Arc<Vec<(f32, f32)>>;

thread_local! {
    static MAP: std::cell::RefCell<Option<Map>> = const { std::cell::RefCell::new(None) };
}

async fn load_map() -> Result<Map, String> {
    if let Some(m) = MAP.with(|m| m.borrow().clone()) {
        return Ok(m);
    }
    let bytes = super::fetch_bytes("birdnet-map.bin").await?;
    let m = Arc::new(parse_map(&bytes));
    MAP.with(|c| *c.borrow_mut() = Some(m.clone()));
    Ok(m)
}

/// The data `species-map.js` draws, as JSON.
fn map_json(run: &ClassifierRun, map: &[(f32, f32)], min: f32, selected: Option<usize>) -> String {
    let r = |v: f32| (v * 10_000.0).round() / 10_000.0;
    let species: Vec<_> = map
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| {
            let l = run.labels.get(i);
            (
                r(x),
                r(y),
                l.map_or("", |l| l.common.as_str()),
                l.map_or("", |l| l.scientific.as_str()),
            )
        })
        .collect();
    let placed = place_windows(run, map, min);
    let mut found: Vec<u16> = placed
        .iter()
        .map(|p| run.windows[p.index].top[0].class)
        .collect();
    found.sort_unstable();
    found.dedup();
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
                    )
                })
                .collect();
            (r(p.x), r(p.y), p.index, w.start_s, w.end_s, top)
        })
        .collect();
    serde_json::json!({
        "species": species,
        "found": found,
        "windows": windows,
        "selected": selected,
    })
    .to_string()
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
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let chart_ref = NodeRef::<leptos::html::Div>::new();
    // None while loading; then the map, or why it could not load.
    let map: RwSignal<Option<Result<Map, String>>> = RwSignal::new(None);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    spawn_local(async move {
        let m = load_map().await;
        map.set(Some(m));
    });

    // A click on a window's dot picks it, as a click in the lists does.
    let on_pick = Closure::<dyn Fn(u32)>::new(move |i: u32| {
        let i = i as usize;
        let Some(file) = current_file.get_untracked() else {
            return;
        };
        let times = state.classifier.results().with_untracked(|m| {
            m.get(&(file, Method::BirdNet))
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
    Effect::new(move |_| {
        let Some(el) = chart_ref.get() else { return };
        let Some(Ok(m)) = map.get() else { return };
        let Some(file) = current_file.get() else {
            return;
        };
        let min = state.call_detection.min_confidence().get();
        let selected = state.call_detection.selected_window().get();
        let json = state.classifier.results().with(|r| {
            r.get(&(file, Method::BirdNet))
                .map(|run| map_json(run, &m, min, selected))
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

    // What the green dots are, with the numbers from this run.
    let caption = move || {
        let file = current_file.get()?;
        let min = state.call_detection.min_confidence().get();
        let n = state.classifier.results().with(|r| {
            r.get(&(file, Method::BirdNet)).map(|run| {
                (0..run.windows.len())
                    .filter(|&i| window_kind(run, i, min) == WindowKind::Species)
                    .count()
            })
        })?;
        Some(format!(
            "Green dots: {} at confidence {min:.2} or above, each placed among the species it scored highest. Click one to select it.",
            super::call_detection_panel::count(n, "window", "windows")
        ))
    };
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
                    <p>"Grey dots are BirdNET's 6,522 species and sounds. Species that BirdNET hears as alike sit close together. Scroll or pinch to zoom."</p>
                    <p>{caption}</p>
                    <p class="detection-status-note">"Positions from BirdNET V2.4's classifier weights. CC BY-NC-SA 4.0."</p>
                </div>
            </div>
        </div>
    }
}

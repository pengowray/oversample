//! "View in 3D": a selection's spectrogram drawn as a surface whose height is
//! the level in dB, rotated by dragging. Drawn by Plotly (js/surface-3d.js),
//! which is loaded the first time the dialog opens.

use crate::audio::source::ChannelView;
use crate::audio::streaming_source::read_region_ready;
use crate::canvas::spectrogram_renderer::Colormap;
use crate::dsp::surface_grid::{surface_grid, SurfaceGrid};
use crate::format_time::{format_relative_label, format_time_display, offset_axis_ticks};
use crate::state::store_fields::*;
use crate::state::{AppState, Selection};
use js_sys::{Array, Float32Array, Float64Array, Object, Reflect};
use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};

/// Longest stretch read. Past a few seconds the surface turns into a comb of
/// narrow spikes, and the read and FFT start to take noticeable time.
pub const MAX_SECS: f64 = 10.0;
/// Grid size after max pooling: about 60k points, which Plotly rotates smoothly.
const MAX_COLS: usize = 300;
const MAX_ROWS: usize = 200;
/// Floor used until the grid is ready; replaced by `typical_floor`.
const DEFAULT_FLOOR_DB: f32 = -60.0;
const MIN_FLOOR_DB: f32 = -120.0;
const MAX_FLOOR_DB: f32 = -20.0;

#[derive(Clone, PartialEq)]
enum Status {
    Loading,
    Ready,
    Failed(String),
}

/// Opens the dialog for `sel`. A selection without a frequency range uses the
/// frequency range on screen.
pub fn open_surface_3d(state: AppState, sel: Selection, view_band: (f64, f64)) {
    let (lo, hi) = match (sel.freq_low, sel.freq_high) {
        (Some(lo), Some(hi)) if hi > lo => (lo, hi),
        _ => view_band,
    };
    state.dialogs.surface_3d().set(Some(Selection {
        freq_low: Some(lo),
        freq_high: Some(hi),
        ..sel
    }));
}

/// Starting floor: the median level, which is the background noise in a
/// selection that is mostly not call. Drawing the noise flat leaves the calls
/// standing up from it. Rounded to the slider's 5 dB step.
fn typical_floor(grid: &SurfaceGrid) -> f32 {
    let mut levels = grid.level_db.clone();
    if levels.is_empty() {
        return DEFAULT_FLOOR_DB;
    }
    let mid = levels.len() / 2;
    let (_, median, _) = levels.select_nth_unstable_by(mid, f32::total_cmp);
    ((*median / 5.0).round() * 5.0).clamp(MIN_FLOOR_DB, MAX_FLOOR_DB)
}

fn colorscale(cmap: Colormap) -> Array {
    let stops = Array::new();
    for i in 0..=10u32 {
        let [r, g, b] = cmap.apply((i * 255 / 10) as u8);
        let stop = Array::of2(
            &JsValue::from_f64(i as f64 / 10.0),
            &JsValue::from_str(&format!("rgb({r},{g},{b})")),
        );
        stops.push(&stop);
    }
    stops
}

fn set(obj: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), value);
}

async fn draw(
    el: &web_sys::HtmlElement,
    grid: &SurfaceGrid,
    start_s: f64,
    span_s: f64,
    floor_db: f32,
    cmap: Colormap,
) -> Result<(), String> {
    let z: Vec<f32> = grid.level_db.iter().map(|v| v.max(floor_db)).collect();
    let freqs_khz: Vec<f64> = grid.freqs_hz.iter().map(|f| f / 1000.0).collect();
    let d = Object::new();
    set(&d, "z", &Float32Array::from(z.as_slice()));
    set(&d, "nx", &JsValue::from(grid.cols as u32));
    set(&d, "ny", &JsValue::from(grid.rows as u32));
    set(&d, "times_s", &Float64Array::from(grid.times_s.as_slice()));
    set(&d, "freqs_khz", &Float64Array::from(freqs_khz.as_slice()));
    // Unclamped levels and each column's time, for the hover label.
    set(&d, "level", &Float32Array::from(grid.level_db.as_slice()));
    let strings = |v: Vec<String>| v.iter().map(|s| JsValue::from_str(s)).collect::<Array>();
    set(
        &d,
        "time_abs",
        &strings(
            grid.times_s
                .iter()
                .map(|t| format_time_display(start_s + t, 4))
                .collect(),
        ),
    );
    set(
        &d,
        "time_rel",
        &strings(
            grid.times_s
                .iter()
                .map(|&t| format_relative_label(t, 0.0001))
                .collect(),
        ),
    );
    // About one time tick per 110 px; at least 4, so a phone shows offsets too.
    let max_ticks = (el.client_width() / 110).clamp(4, 8) as usize;
    let (tick_vals, tick_text) = offset_axis_ticks(start_s, span_s, max_ticks);
    set(&d, "tick_vals", &Float64Array::from(tick_vals.as_slice()));
    set(&d, "tick_text", &strings(tick_text));
    set(&d, "floor_db", &JsValue::from_f64(floor_db as f64));
    set(&d, "colorscale", &colorscale(cmap));

    let api = Reflect::get(&js_sys::global(), &JsValue::from_str("oversampleSurface3d"))
        .ok()
        .filter(|v| v.is_object())
        .ok_or("surface-3d.js is missing")?;
    let show: js_sys::Function = Reflect::get(&api, &JsValue::from_str("show"))
        .map_err(|_| "surface-3d.js has no show()")?
        .unchecked_into();
    let promise: js_sys::Promise = show
        .call2(&api, el, &d)
        .map_err(|e| format!("{e:?}"))?
        .unchecked_into();
    JsFuture::from(promise).await.map(|_| ()).map_err(|e| {
        e.dyn_ref::<js_sys::Error>()
            .map(|err| String::from(err.message()))
            .or_else(|| e.as_string())
            .unwrap_or_else(|| "Couldn't load the 3D viewer".into())
    })
}

fn purge(el: &web_sys::HtmlElement) {
    let global = js_sys::global();
    let Ok(api) = Reflect::get(&global, &JsValue::from_str("oversampleSurface3d")) else {
        return;
    };
    if let Ok(f) = Reflect::get(&api, &JsValue::from_str("purge")) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call1(&api, el);
        }
    }
}

/// Mounted once in `App`; shows while `state.dialogs.surface_3d` is set.
#[component]
pub fn Surface3dDialog() -> impl IntoView {
    let state = expect_context::<AppState>();
    move || {
        state
            .dialogs
            .surface_3d()
            .get()
            .map(|region| view! { <Surface3dBody region=region /> })
    }
}

#[component]
fn Surface3dBody(region: Selection) -> impl IntoView {
    let state = expect_context::<AppState>();
    let plot_ref = NodeRef::<leptos::html::Div>::new();
    let status = RwSignal::new(Status::Loading);
    let grid: RwSignal<Option<SurfaceGrid>, LocalStorage> = RwSignal::new_local(None);
    let floor_db = RwSignal::new(DEFAULT_FLOOR_DB);
    let start_floor = StoredValue::new(DEFAULT_FLOOR_DB);
    let plot_el: StoredValue<Option<web_sys::HtmlElement>, LocalStorage> =
        StoredValue::new_local(None);

    let close = move || state.dialogs.surface_3d().set(None);
    // A rotation drag that ends outside the dialog fires a click on the
    // backdrop; only a press that also started on the backdrop closes it.
    let pressed_on_backdrop = StoredValue::new(false);

    let (file_name, file_secs) = state
        .library
        .current_index()
        .get_untracked()
        .and_then(|i| {
            state
                .library
                .files()
                .with_untracked(|f| f.get(i).map(|f| (f.name.clone(), f.audio.duration_secs)))
        })
        .unzip();
    // A selection can extend past the end of the recording.
    let region = Selection {
        time_start: region.time_start.max(0.0),
        time_end: region.time_end.min(file_secs.unwrap_or(f64::INFINITY)),
        ..region
    };
    let hint = if state.status.is_mobile().get_untracked() {
        "Drag to rotate"
    } else {
        "Drag to rotate, scroll to zoom"
    };
    let duration = (region.time_end - region.time_start).max(0.0);
    let truncated = duration > MAX_SECS;
    let shown_end = region.time_start + duration.min(MAX_SECS);
    let (lo, hi) = (
        region.freq_low.unwrap_or(0.0),
        region.freq_high.unwrap_or(0.0),
    );
    let summary = format!(
        "{}, {:.0}\u{2013}{:.0} kHz",
        crate::format_time::format_time_range(region.time_start, shown_end, 3),
        lo / 1000.0,
        hi / 1000.0,
    );

    // Read the audio and build the grid once per dialog.
    spawn_local(async move {
        crate::web_util::yield_now().await;
        let Some(source) = state
            .library
            .current_index()
            .try_get_untracked()
            .flatten()
            .and_then(|i| {
                state
                    .library
                    .files()
                    .with_untracked(|f| f.get(i).map(|f| f.audio.source.clone()))
            })
        else {
            status.try_set(Status::Failed("No recording open".into()));
            return;
        };
        let view = match state.viewmode.channel_view().get_untracked() {
            ChannelView::Stereo => ChannelView::MonoMix,
            other => other,
        };
        // The main spectrogram's FFT size at its widest zoom, so the 3D view has
        // the same time and frequency resolution as the 2D view.
        let fft = state
            .spect
            .fft_mode()
            .get_untracked()
            .fft_for_lod(0)
            .max(64);
        let sr = source.sample_rate();
        let start = (region.time_start * sr as f64) as u64;
        let len = ((shown_end - region.time_start) * sr as f64) as usize;
        if len == 0 {
            status.try_set(Status::Failed(
                "Selection is past the end of the recording".into(),
            ));
            return;
        }
        let Some(samples) = read_region_ready(&*source, view, start, len).await else {
            status.try_set(Status::Failed(
                "Couldn't read the audio for this selection".into(),
            ));
            return;
        };
        match surface_grid(&samples, sr, fft, (lo, hi), MAX_COLS, MAX_ROWS) {
            Some(g) => {
                let floor = typical_floor(&g);
                start_floor.try_set_value(floor);
                floor_db.try_set(floor);
                grid.try_set(Some(g));
            }
            None => {
                status.try_set(Status::Failed(format!(
                    "Selection is too short: {:.1} ms. The 3D view needs at least {:.1} ms.",
                    samples.len() as f64 / sr as f64 * 1000.0,
                    fft as f64 / sr as f64 * 1000.0
                )));
            }
        }
    });

    // Draw, and redraw when the floor changes (Plotly keeps the camera angle).
    Effect::new(move || {
        let floor = floor_db.get();
        // Same colormap as the 2D view: HFR mode has its own.
        let cmap = if state.viewmode.hfr_enabled().get() {
            state.spect.hfr_colormap_preference().get()
        } else {
            state.spect.colormap_preference().get()
        };
        let Some(el) = plot_ref.get() else { return };
        let el: web_sys::HtmlElement = el.into();
        let Some(g) = grid.with(|g| g.clone()) else {
            return;
        };
        plot_el.set_value(Some(el.clone()));
        spawn_local(async move {
            let result = draw(
                &el,
                &g,
                region.time_start,
                shown_end - region.time_start,
                floor,
                cmap,
            )
            .await;
            status.try_set(match result {
                Ok(()) => Status::Ready,
                Err(e) => Status::Failed(e),
            });
        });
    });

    on_cleanup(move || {
        if let Some(Some(el)) = plot_el.try_get_value() {
            purge(&el);
        }
    });

    let key_handle = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" {
            close();
        }
    });
    on_cleanup(move || key_handle.remove());

    view! {
        <div
            class="modal-overlay"
            on:mousedown=move |ev: web_sys::MouseEvent| {
                let on_backdrop = ev.target() == ev.current_target();
                pressed_on_backdrop.set_value(on_backdrop);
            }
            on:click=move |ev: web_sys::MouseEvent| {
                if ev.target() == ev.current_target() && pressed_on_backdrop.get_value() {
                    close();
                }
            }
        >
            <div class="modal-dialog surface-3d-dialog">
                <div class="modal-header">
                    <span class="modal-title">"3D spectrogram"</span>
                    <span class="surface-3d-summary">
                        {file_name.map(|n| view! { <span class="surface-3d-file">{n}</span> })}
                        <span class="surface-3d-range">{summary}</span>
                    </span>
                    <button class="modal-close" title="Close (Esc)" on:click=move |_| close()>
                        "\u{00D7}"
                    </button>
                </div>
                <div class="surface-3d-controls">
                    <label class="surface-3d-floor" title="Levels under this are drawn flat.">
                        <span>"Floor"</span>
                        <input
                            type="range"
                            class="setting-range"
                            min=MIN_FLOOR_DB.to_string()
                            max=MAX_FLOOR_DB.to_string()
                            step="5"
                            prop:value=move || floor_db.get().to_string()
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                    floor_db.set(v.clamp(MIN_FLOOR_DB, MAX_FLOOR_DB));
                                }
                            }
                            on:dblclick=move |_| floor_db.set(start_floor.get_value())
                        />
                        <span class="surface-3d-floor-value">
                            {move || format!("{:.0} dB", floor_db.get())}
                        </span>
                    </label>
                    {move || (floor_db.get() != start_floor.get_value()).then(|| view! {
                        <button
                            class="setting-button"
                            title="Back to the selection's median level"
                            on:click=move |_| floor_db.set(start_floor.get_value())
                        >
                            "Reset"
                        </button>
                    })}
                    {truncated.then(|| view! {
                        <span class="surface-3d-note">
                            {format!("Showing the first {MAX_SECS:.0} s of {duration:.1} s")}
                        </span>
                    })}
                    <span class="surface-3d-hint">{hint}</span>
                </div>
                <div class="surface-3d-plot-wrap">
                    <div class="surface-3d-plot" node_ref=plot_ref></div>
                    {move || {
                        let (text, class) = match status.get() {
                            Status::Ready => return None,
                            Status::Loading => ("Loading\u{2026}".to_string(), "surface-3d-status"),
                            Status::Failed(msg) => (msg, "surface-3d-status error"),
                        };
                        Some(view! { <div class=class>{text}</div> })
                    }}
                </div>
            </div>
        </div>
    }
}

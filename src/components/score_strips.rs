//! Rows above the spectrogram: the picked species' scores and the picked
//! principal components (BirdNET V2.4 or BirdNET+ V3.0) over the whole
//! recording, one cell per
//! window of the current file's classifier run (`crate::classifier::strips`).
//! The rows follow the method picked in the Call detection tab, as the label
//! strip on the spectrogram does, and share the spectrogram's time axis.
//!
//! Picking happens in the Call detection tab (`classifier_section.rs`); the
//! choices are `CallDetectionState::strip_species` and `strip_components`.

use crate::canvas::overlays::{draw_score_strips, SCORE_ROW_H};
use crate::classifier::strips::{
    class_of, component_cells, display_name, ordinal, species_cells, window_at, window_floors,
    ScoreRow, ScoreRowKind, ScoreStrips,
};
use crate::classifier::{self, location, Method, MethodFacts};
use crate::live_embedding::Components;
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

thread_local! {
    /// Each method's principal components, once loaded.
    static COMPONENTS: RefCell<HashMap<Method, Arc<Components>>> = RefCell::new(HashMap::new());
    static LOAD_STARTED: RefCell<HashSet<Method>> = RefCell::new(HashSet::new());
    /// Notified when a components file has loaded (or failed to).
    static LOADED: ArcTrigger = ArcTrigger::new();
}

/// The components file of `method`'s embedding, for rows above the
/// spectrogram: the live view's class-row components, for the methods with
/// [`MethodInfo::component_rows`](classifier::MethodInfo::component_rows).
fn components_file(method: Method) -> Option<&'static str> {
    method
        .info()
        .component_rows
        .then(|| crate::live_embedding::model_for(method).pc_file)
}

/// `method`'s principal components: the components of its class rows
/// (`birdnet-pc-rows.bin`, `birdnet-v3-pc-rows.bin`), the live view's
/// default. `None` for a method without them. The first call starts loading
/// the file and returns `None`; reactive callers run again once it is in. The
/// number of components comes from the file.
pub fn components_for(method: Method) -> Option<Arc<Components>> {
    let file = components_file(method)?;
    LOADED.with(|t| t.track());
    if let Some(c) = COMPONENTS.with(|c| c.borrow().get(&method).cloned()) {
        return Some(c);
    }
    if LOAD_STARTED.with(|s| s.borrow_mut().insert(method)) {
        spawn_local(async move {
            match crate::components::file_sidebar::fetch_bytes(file).await {
                // A file for another embedding would give wrong rows.
                Ok(bytes) => {
                    match Components::parse(&bytes).filter(|c| c.dim == method.embedding_len()) {
                        Some(c) => COMPONENTS.with(|p| {
                            p.borrow_mut().insert(method, Arc::new(c));
                        }),
                        None => log::warn!("score strips: {file} is not a components file"),
                    }
                }
                Err(e) => log::warn!("score strips: could not load {file}: {e}"),
            }
            LOADED.with(|t| t.notify());
        });
    }
    None
}

/// Widest the hover tip gets, in CSS pixels.
const TIP_W: f64 = 240.0;

/// Where the hover tip goes, in CSS pixels from the spectrogram box's edges:
/// its left side at `Left(x)`, or its right side at `Right(x)` from the box's
/// right edge.
#[derive(Clone, Copy, PartialEq)]
enum TipAt {
    Left(f64),
    Right(f64),
}

/// The rows for the current file. Reactive.
fn build(state: AppState) -> Option<ScoreStrips> {
    if state.timeline.active().with(|t| t.is_some()) {
        return None;
    }
    let method = state.call_detection.method().get().classifier()?;
    let file = state.current_file_id_tracked()?;
    let species: Vec<String> = state.call_detection.strip_species().with(|v| {
        v.iter()
            .filter(|s| s.method == method)
            .map(|s| s.scientific.clone())
            .collect()
    });
    let hidden: Vec<String> = state.call_detection.strip_hidden().with(|v| {
        v.iter()
            .filter(|s| s.method == method)
            .map(|s| s.scientific.clone())
            .collect()
    });
    // Components of the method's embedding, for the methods that have them
    // (`components_file`).
    let picked_pcs: Vec<usize> = if components_file(method).is_some() {
        state.call_detection.strip_components().get()
    } else {
        Vec::new()
    };
    let groups_on = state.call_detection.sound_groups().get();
    let min = classifier::min_confidence(state, method);
    let pcs = (!picked_pcs.is_empty())
        .then(|| components_for(method))
        .flatten();
    state.classifier.results().with(|m| {
        let run = m.get(&(file, method))?;
        let mut rows = Vec::new();
        let order = classifier::kept_order::colour_order(file, method, run, min);
        // A row without a pin for the species found (`auto_rows`), as the
        // live view's rows appeared by themselves, apart from those the
        // user removed. Then the pinned species.
        let mut classes: Vec<u16> = classifier::kept_order::auto_rows(file, method, run, min)
            .into_iter()
            .filter(|&c| !hidden.contains(&run.labels[c as usize].scientific))
            .collect();
        for scientific in &species {
            // Not among this run's classes (a BattyBirdNET region without
            // the species): no row.
            if let Some(class) = class_of(run, scientific) {
                if !classes.contains(&class) {
                    classes.push(class);
                }
            }
        }
        for class in classes {
            let label = &run.labels[class as usize];
            let scientific = &label.scientific;
            rows.push(ScoreRow {
                kind: ScoreRowKind::Species,
                label: display_name(label),
                scientific: scientific.clone(),
                unlikely: location::unlikely_here(state, file, method, label),
                colour: order.iter().position(|&c| c == class),
                cells: species_cells(run, class),
            });
        }
        if let Some(pcs) = &pcs {
            let picked: Vec<usize> = picked_pcs
                .iter()
                .copied()
                .filter(|&k| k < pcs.variance.len())
                .collect();
            let slots: Vec<u32> = run.windows.iter().map(|w| w.slot).collect();
            let cells = state.classifier.embeddings().with(|e| {
                let e = e.get(&(file, method)).map_or(&[][..], |v| &v[..]);
                component_cells(e, method, &slots, &picked, |x| pcs.percentiles(x))
            });
            for (k, cells) in picked.iter().zip(cells) {
                rows.push(ScoreRow {
                    kind: ScoreRowKind::Component,
                    label: format!("PC{}", k + 1),
                    scientific: String::new(),
                    unlikely: false,
                    colour: None,
                    cells,
                });
            }
        }
        let mut group_names = Vec::new();
        let mut quiet = Vec::new();
        if groups_on {
            if let Some((row, names, q)) =
                crate::components::sound_groups::row(state, file, method, run)
            {
                rows.push(row);
                group_names = names;
                quiet = q;
            }
        }
        (!rows.is_empty()).then(|| ScoreStrips {
            windows: run.cells(),
            heard: run.windows.iter().map(|w| (w.start_s, w.end_s)).collect(),
            numbered: !run.overlapping(),
            rows,
            floors: window_floors(run),
            min_confidence: min,
            group_names,
            quiet,
        })
    })
}

/// The hover text for row `r`, window `i`: two lines.
fn hover_text(strips: &ScoreStrips, r: usize, i: usize) -> Option<(String, String)> {
    let row = strips.rows.get(r)?;
    // The audio the window heard, which with a shorter step is longer than
    // its cell.
    let (s, e) = *strips.heard.get(i)?;
    let (s, e) = (
        crate::format_time::format_time_display(s, 1),
        crate::format_time::format_time_display(e, 1),
    );
    let window = if strips.numbered {
        format!("Window {} ({s} to {e})", i + 1)
    } else {
        format!("Window {s} to {e}")
    };
    let cell = row.cells.get(i).copied().flatten();
    let line = match (row.kind, cell) {
        (ScoreRowKind::Species, Some(v)) => format!("{}: confidence {v:.2}", row.label),
        (ScoreRowKind::Species, None) => match strips.floors.get(i).copied().flatten() {
            // Under 0.005 the bound would read "below 0.00".
            Some(floor) if floor >= 0.005 => {
                format!("{}: below {floor:.2} (not in the top 10)", row.label)
            }
            _ => format!("{}: not in the top 10 species", row.label),
        },
        (ScoreRowKind::Component, Some(p)) => format!(
            "{}: {} percentile",
            row.label,
            ordinal((p * 100.0).round() as u32)
        ),
        (ScoreRowKind::Component, None) => format!("{}: not analysed yet", row.label),
        (ScoreRowKind::Groups, Some(g)) => strips.group_names.get(g as usize)?.clone(),
        (ScoreRowKind::Groups, None) if strips.quiet.get(i) == Some(&true) => {
            "Quiet: not clustered".to_string()
        }
        (ScoreRowKind::Groups, None) => format!("{}: not clustered yet", row.label),
    };
    Some((line, window))
}

/// The rows, above the spectrogram's chart row. Nothing is shown while no row
/// applies to the current file.
#[component]
pub fn ScoreStripsRow() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    let strips = Memo::new(move |_| build(state));
    crate::components::sound_groups::drive(state);
    let row_count = move || strips.with(|s| s.as_ref().map_or(0, |s| s.rows.len()));
    // Hover: where the tip goes, and the two lines of text.
    let hover: RwSignal<Option<(TipAt, String, String)>> = RwSignal::new(None);
    let size_tick = RwSignal::new(0u32);

    // Seconds per CSS pixel of the spectrogram, and its left edge in seconds.
    let time_axis = move || -> Option<(f64, f64)> {
        let zoom = state.view.zoom_level().get();
        let scroll = state.view.scroll_offset().get();
        let idx = state.library.current_index().get()?;
        let time_res = state
            .library
            .files()
            .with(|f| f.get(idx).map(|f| f.spectrogram.time_resolution))?;
        (zoom > 0.0).then(|| (time_res / zoom, scroll))
    };

    // Redraw when the rows, the time axis, the selection or the size change.
    Effect::new(move |_| {
        let _ = size_tick.get();
        let _ = state.viewmode.spectrogram_canvas_width().get();
        let selected = state.call_detection.selected_window().get();
        let axis = time_axis();
        strips.with(|s| {
            let (Some(s), Some((sec_per_px, scroll))) = (s, axis) else {
                return;
            };
            let Some(canvas) = canvas_ref.get() else {
                return;
            };
            let canvas: &HtmlCanvasElement = canvas.as_ref();
            // The parent's box, not the canvas' own (see TimeGutter).
            let Some(parent) = canvas.parent_element() else {
                return;
            };
            let r = parent.get_bounding_client_rect();
            let (w, h) = (r.width().floor(), s.rows.len() as f64 * SCORE_ROW_H);
            if w <= 0.0 {
                return;
            }
            let dpr = web_sys::window()
                .map_or(1.0, |w| w.device_pixel_ratio())
                .max(1.0);
            let (pw, ph) = ((w * dpr).round() as u32, (h * dpr).round() as u32);
            if canvas.width() != pw || canvas.height() != ph {
                canvas.set_width(pw);
                canvas.set_height(ph);
            }
            let Ok(Some(ctx)) = canvas.get_context("2d") else {
                return;
            };
            let Ok(ctx) = ctx.dyn_into::<CanvasRenderingContext2d>() else {
                return;
            };
            let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
            let selected =
                selected.and_then(|t| crate::classifier::strips::start_index(&s.heard, t));
            draw_score_strips(&ctx, s, selected, scroll, w * sec_per_px, w);
        });
    });

    // Width changes (sidebars opening) redraw.
    Effect::new(move |_| {
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let canvas: &HtmlCanvasElement = canvas.as_ref();
        let Some(parent) = canvas.parent_element() else {
            return;
        };
        let cb = wasm_bindgen::closure::Closure::<dyn Fn(js_sys::Array)>::new(move |_| {
            if let Some(v) = size_tick.try_get_untracked() {
                size_tick.set(v.wrapping_add(1));
            }
        });
        if let Ok(observer) = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()) {
            observer.observe(&parent);
            let _ = js_sys::Reflect::set(
                &parent,
                &wasm_bindgen::JsValue::from_str("__score_strips_obs"),
                &observer,
            );
        }
        cb.forget();
    });

    // The row and window under a pointer, from its position on the canvas.
    let cell_at = move |ev: &web_sys::PointerEvent| -> Option<(f64, usize, usize)> {
        let canvas = canvas_ref.get_untracked()?;
        let canvas: &HtmlCanvasElement = canvas.as_ref();
        let r = canvas.get_bounding_client_rect();
        let x = ev.client_x() as f64 - r.left();
        let y = ev.client_y() as f64 - r.top();
        let (sec_per_px, scroll) = untrack(time_axis)?;
        let row = (y / SCORE_ROW_H).floor();
        if row < 0.0 {
            return None;
        }
        strips.with_untracked(|s| {
            let s = s.as_ref()?;
            let i = window_at(&s.windows, scroll + x * sec_per_px)?;
            Some((x, row as usize, i)).filter(|&(_, r, _)| r < s.rows.len())
        })
    };
    // The tip sits under the rows, at the pointer, inside the spectrogram's
    // box (whose `overflow: hidden` would cut it off at the right edge).
    // Near the right edge its right side is at the pointer instead, so a
    // short tip stays beside the pointer.
    let show_hover = move |ev: &web_sys::PointerEvent| {
        let at = || -> Option<TipAt> {
            let canvas = canvas_ref.get_untracked()?;
            let canvas: &HtmlCanvasElement = canvas.as_ref();
            let host = canvas.closest(".spectrogram-container").ok()??;
            let r = host.get_bounding_client_rect();
            let x = (ev.client_x() as f64 - r.left()).max(0.0);
            Some(if x + TIP_W <= r.width() {
                TipAt::Left(x)
            } else {
                // Never so far right that a full-width tip passes the left edge.
                TipAt::Right((r.width() - x).min(r.width() - TIP_W).max(0.0))
            })
        };
        let text = cell_at(ev).and_then(|(_, r, i)| {
            let (a, b) = strips.with_untracked(|s| s.as_ref().and_then(|s| hover_text(s, r, i)))?;
            Some((at()?, a, b))
        });
        hover.set(text);
    };
    // A tap has no hover: the browser sends pointerleave as the finger
    // lifts, which would hide the tip at once. After a tap the tip stays
    // until the next tap or click somewhere else.
    let outside_tap = window_event_listener(leptos::ev::pointerdown, move |ev| {
        let on_rows = canvas_ref.get_untracked().is_some_and(|c| {
            let c: &web_sys::Node = c.as_ref();
            ev.target()
                .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                .is_some_and(|t| c.contains(Some(&t)))
        });
        if !on_rows {
            hover.try_set(None);
        }
    });
    on_cleanup(move || outside_tap.remove());

    view! {
        <div
            class="score-strips-row"
            style:display=move || if row_count() == 0 { "none" } else { "flex" }
            style:height=move || format!("{}px", row_count() as f64 * SCORE_ROW_H)
        >
            <div class="view-bottom-corner score-strips-corner"></div>
            <div class="score-strips-stage">
                <canvas
                    node_ref=canvas_ref
                    on:pointermove=move |ev| show_hover(&ev)
                    on:pointerleave=move |ev: web_sys::PointerEvent| {
                        if ev.pointer_type() != "touch" {
                            hover.set(None);
                        }
                    }
                    on:pointerdown=move |ev: web_sys::PointerEvent| {
                        // A click (or tap) picks the window, as in the lists.
                        if let Some((_, _, i)) = cell_at(&ev) {
                            let start = strips.with_untracked(|s| {
                                s.as_ref().and_then(|s| s.heard.get(i)).map(|h| h.0)
                            });
                            if let Some(start) = start {
                                state.call_detection.selected_window().set(Some(start));
                            }
                        }
                        show_hover(&ev);
                    }
                />
            </div>
        </div>
        {move || hover.get().filter(|_| row_count() > 0).map(|(at, a, b)| view! {
            <div
                class="score-strips-tip"
                style:top=format!("{}px", row_count() as f64 * SCORE_ROW_H + 2.0)
                style:left=match at {
                    TipAt::Left(x) => format!("{x:.0}px"),
                    TipAt::Right(_) => "auto".to_string(),
                }
                style:right=match at {
                    TipAt::Left(_) => "auto".to_string(),
                    TipAt::Right(x) => format!("{x:.0}px"),
                }
                style:max-width=format!("{TIP_W}px")
            >
                <div>{a}</div>
                <div>{b}</div>
            </div>
        })}
    }
}

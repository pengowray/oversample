//! Typical calls strip: the selected bat book species' call sketch, drawn at
//! the left edge of the spectrogram on the same time scale (px per second) and
//! frequency scale (kHz per px) as the recording, so call lengths, spacing and
//! frequencies can be compared directly.
//!
//! It uses a different colour scheme from the recording (greyscale, or cividis
//! when the recording itself is shown in greyscale) and a framed panel, so the
//! sketch is never read as part of the recording.
//!
//! Redraws only on view-scale changes (zoom, frequency range, canvas size,
//! selection), never on playback position.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::bat_book::call_sketch::{self, CallSketch, Range};
use crate::canvas::spectrogram_renderer::{freq_to_y, Colormap};
use crate::state::store_fields::*;
use crate::state::AppState;

/// Widest the strip may get, as a fraction of the spectrogram's width.
const MAX_WIDTH_FRACTION: f64 = 0.35;

/// The main view's scale, which the strip copies.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ViewScale {
    /// CSS px per second.
    px_per_s: f64,
    /// Visible frequency range, Hz.
    min_hz: f64,
    max_hz: f64,
    /// Highest frequency in the recording (half its sample rate), Hz.
    file_max_hz: f64,
    /// Spectrogram size, CSS px.
    css_w: f64,
    css_h: f64,
}

/// Where the sketched calls are when none of them is in the visible
/// frequency range.
#[derive(Clone, Copy, Debug, PartialEq)]
enum OutOfView {
    /// Above the view; zooming out on frequency shows them.
    Above,
    /// Above the recording's highest frequency: the recording can't show them.
    AboveRecording {
        lo_khz: f64,
        hi_khz: f64,
        top_khz: f64,
    },
    /// Below the view; zooming out on frequency shows them.
    Below,
}

/// The first selected bat book species that has a sketch that can be drawn.
fn selected_sketch(state: AppState) -> Option<&'static CallSketch> {
    state.bat_book.selected_ids().with(|ids| {
        ids.iter()
            .filter_map(|id| call_sketch::for_species(id))
            .find(|s| s.drawable())
    })
}

/// Colour scheme for the sketch: one the recording isn't using.
fn sketch_colormap(state: AppState) -> Colormap {
    let main = state.spect.colormap_preference().get();
    let hfr = state.spect.hfr_colormap_preference().get();
    if main == Colormap::Greyscale || hfr == Colormap::Greyscale {
        Colormap::Cividis
    } else {
        Colormap::Greyscale
    }
}

fn rgba(c: [u8; 3], a: f64) -> String {
    format!("rgba({},{},{},{a:.3})", c[0], c[1], c[2])
}

fn range_text(r: Option<&Range>, unit: &str) -> Option<String> {
    let r = r?;
    let v = r.value()?;
    let typical = format!("{} {unit}", fmt(v));
    match (r.min, r.max) {
        (Some(a), Some(b)) if a < b => Some(format!("{typical} ({} to {} {unit})", fmt(a), fmt(b))),
        _ => Some(typical),
    }
}

fn fmt(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

fn out_of_view_text(o: OutOfView) -> String {
    match o {
        OutOfView::Above => "Calls are above the top of the spectrogram. Zoom out on frequency \
                             (Shift + scroll wheel) to see them."
            .to_string(),
        OutOfView::AboveRecording {
            lo_khz,
            hi_khz,
            top_khz,
        } => format!(
            "Calls are at {} to {} kHz, above the highest frequency in this recording ({} kHz). \
             Zooming will not show them.",
            fmt(lo_khz),
            fmt(hi_khz),
            fmt(top_khz)
        ),
        OutOfView::Below => "Calls are below the bottom of the spectrogram. Zoom out on frequency \
                             (Shift + scroll wheel) to see them."
            .to_string(),
    }
}

#[component]
pub fn CallSketchStrip(canvas_ref: NodeRef<leptos::html::Canvas>) -> impl IntoView {
    let state = expect_context::<AppState>();
    let strip_ref = NodeRef::<leptos::html::Canvas>::new();

    let sketch = Memo::new(move |_| {
        if !state.bat_book.open().get() || !state.bat_book.sketch_shown().get() {
            return None;
        }
        selected_sketch(state)
    });

    // None until the main canvas has a size.
    let scale = Memo::new(move |_| {
        let _ = state.viewmode.spectrogram_canvas_width().get();
        let zoom = state.view.zoom_level().get();
        let idx = state.library.current_index().get()?;
        let (time_res, file_max) = state.library.files().with(|files| {
            files
                .get(idx)
                .map(|f| (f.spectrogram.time_resolution, f.spectrogram.max_freq))
        })?;
        let max_f = state.view.max_display_freq().get().unwrap_or(file_max);
        let min_f = state.view.min_display_freq().get().unwrap_or(0.0);
        let canvas = canvas_ref.get()?;
        let el: &web_sys::HtmlCanvasElement = canvas.as_ref();
        let css_w = el.client_width() as f64;
        let css_h = el.client_height() as f64;
        let backing_w = el.width() as f64;
        if css_w <= 0.0 || css_h <= 0.0 || backing_w <= 0.0 || time_res <= 0.0 {
            return None;
        }
        // zoom is backing px per spectrogram column.
        let px_per_s = zoom / time_res * (css_w / backing_w);
        Some(ViewScale {
            px_per_s,
            min_hz: min_f,
            max_hz: max_f,
            file_max_hz: file_max,
            css_w,
            css_h,
        })
    });

    let layout = Memo::new(move |_| {
        let s = sketch.get()?;
        let sc = scale.get()?;
        Some(call_sketch::layout(
            s,
            sc.px_per_s,
            sc.css_w * MAX_WIDTH_FRACTION,
        ))
    });

    let out_of_view = Memo::new(move |_| {
        let (lo, hi) = sketch.get()?.khz_extent()?;
        let sc = scale.get()?;
        let (lo_hz, hi_hz) = (lo * 1000.0, hi * 1000.0);
        if lo_hz > sc.file_max_hz {
            Some(OutOfView::AboveRecording {
                lo_khz: lo,
                hi_khz: hi,
                top_khz: sc.file_max_hz / 1000.0,
            })
        } else if lo_hz > sc.max_hz {
            Some(OutOfView::Above)
        } else if hi_hz < sc.min_hz {
            Some(OutOfView::Below)
        } else {
            None
        }
    });

    // Left edge of the strip, CSS px. At the left of the view, unless the
    // recording ends inside the view with room for the strip after it: then
    // the strip goes there, so it covers none of the recording. Only this
    // memo follows scrolling, and it only moves the strip.
    let left_px = Memo::new(move |_| {
        let lay = layout.get()?;
        let sc = scale.get()?;
        let (pps, css_w) = (sc.px_per_s, sc.css_w);
        let idx = state.library.current_index().get()?;
        let duration = state
            .library
            .files()
            .with(|files| files.get(idx).map(|f| f.audio.duration_secs))?;
        let scroll = state.view.scroll_offset().get();
        let end_px = (duration - scroll) * pps;
        const GAP: f64 = 8.0;
        if end_px >= 0.0 && css_w - end_px >= lay.width + 2.0 * GAP {
            Some(end_px + GAP)
        } else {
            Some(0.0)
        }
    });

    Effect::new(move |_| {
        let (Some(s), Some(sc), Some(lay)) = (sketch.get(), scale.get(), layout.get()) else {
            return;
        };
        let (min_f, max_f, css_h) = (sc.min_hz, sc.max_hz, sc.css_h);
        let cmap = sketch_colormap(state);
        let Some(canvas) = strip_ref.get() else {
            return;
        };
        let el: &web_sys::HtmlCanvasElement = canvas.as_ref();
        let dpr = web_sys::window()
            .map(|w| w.device_pixel_ratio())
            .unwrap_or(1.0);
        el.set_width((lay.width * dpr).round() as u32);
        el.set_height((css_h * dpr).round() as u32);
        let Some(ctx) = el
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
        else {
            return;
        };
        let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        ctx.set_fill_style_str(&rgba(cmap.apply(16), 1.0));
        ctx.fill_rect(0.0, 0.0, lay.width, css_h);

        let y_of = |khz: f64| freq_to_y(khz * 1000.0, min_f, max_f, css_h);
        let line = cmap.apply(240);
        ctx.set_line_cap("round");
        ctx.set_line_join("round");

        for placed in &lay.calls {
            if lay.too_small {
                // Too small to draw the shape: mark where each call is, so the
                // spacing of the train still shows.
                let (lo, hi) = placed
                    .call
                    .shape
                    .iter()
                    .fold((f64::MAX, f64::MIN), |(lo, hi), p| {
                        (lo.min(p[1]), hi.max(p[1]))
                    });
                ctx.set_stroke_style_str(&rgba(line, 0.9));
                ctx.set_line_width(1.5);
                ctx.begin_path();
                ctx.move_to(placed.x, y_of(hi));
                ctx.line_to(placed.x, y_of(lo));
                ctx.stroke();
                continue;
            }
            let n = ((placed.width * 2.0).ceil() as usize).clamp(12, 400);
            let mut layers: Vec<(f64, f64)> = vec![(1.0, 1.0)];
            for h in &placed.call.harmonics {
                layers.push((
                    h.multiple,
                    10f64.powf(h.relative_db / 20.0).clamp(0.05, 1.0),
                ));
            }
            for (mult, gain) in layers {
                ctx.set_shadow_color(&rgba(line, 0.6 * gain));
                ctx.set_shadow_blur(6.0);
                ctx.set_line_width(2.5);
                for i in 0..n {
                    let t0 = i as f64 / n as f64;
                    let t1 = (i + 1) as f64 / n as f64;
                    // Softer at both ends of the call, as a call fades in and out.
                    let mid = (t0 + t1) / 2.0;
                    let env = (std::f64::consts::PI * mid).sin().powf(0.6).max(0.25);
                    ctx.set_stroke_style_str(&rgba(line, env * gain));
                    ctx.begin_path();
                    ctx.move_to(
                        placed.x + t0 * placed.width,
                        y_of(call_sketch::shape_at(&placed.call.shape, t0) * mult),
                    );
                    ctx.line_to(
                        placed.x + t1 * placed.width,
                        y_of(call_sketch::shape_at(&placed.call.shape, t1) * mult),
                    );
                    ctx.stroke();
                }
            }
            ctx.set_shadow_blur(0.0);
        }
        let _ = s;
    });

    let hide = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        state.bat_book.sketch_shown().set(false);
    };

    move || {
        let s = sketch.get()?;
        let lay = layout.get()?;
        let first = s.sequence().first().copied()?;
        // The bat book's name, so the strip and the bat book match.
        let name = crate::bat_book::auto_resolve::find_entry_any_book(&s.id)
            .map(|e| e.name.to_string())
            .or_else(|| s.common_name.clone())
            .unwrap_or_else(|| s.scientific_name.clone());
        let mut facts: Vec<String> = Vec::new();
        if let Some(t) = range_text(first.peak_khz.as_ref(), "kHz") {
            facts.push(format!("Peak frequency {t}"));
        } else if let Some(t) = range_text(first.characteristic_khz.as_ref(), "kHz") {
            facts.push(format!("Characteristic frequency {t}"));
        }
        if let Some(t) = range_text(first.duration_ms.as_ref(), "ms") {
            facts.push(format!("Call length {t}"));
        }
        if let Some(t) = range_text(s.train.as_ref().and_then(|t| t.interval_ms.as_ref()), "ms") {
            facts.push(format!("Call interval {t}"));
        }
        let sources_tip = std::iter::once("Sources:".to_string())
            .chain(s.sources.iter().map(|src| match &src.url {
                Some(u) => format!("{} {u}", src.citation),
                None => src.citation.clone(),
            }))
            .collect::<Vec<_>>()
            .join("\n");
        let n_sources = s.sources.len();
        let too_small = lay.too_small;
        Some(view! {
            <div
                class="call-sketch-strip"
                class=("call-sketch-floating", move || left_px.get().unwrap_or(0.0) > 0.0)
                style:width=format!("{}px", lay.width)
                style:left=move || format!("{}px", left_px.get().unwrap_or(0.0))
            >
                <canvas node_ref=strip_ref class="call-sketch-canvas"/>
                <div class="call-sketch-text">
                    <div class="call-sketch-title">
                        <span>{format!("{name}: typical calls")}</span>
                        <button class="call-sketch-close" title="Hide typical calls" on:click=hide>
                            "\u{00d7}"
                        </button>
                    </div>
                    {facts.into_iter().map(|f| view! { <div class="call-sketch-fact">{f}</div> }).collect_view()}
                    <div class="call-sketch-sources" title=sources_tip>
                        {format!("Sketch based on {n_sources} published sources")}
                    </div>
                    // Only one warning: when the calls are outside the
                    // frequency range, their width doesn't matter.
                    {move || (too_small && out_of_view.get().is_none()).then(|| view! {
                        <div class="call-sketch-warn">
                            "Calls are under 2 pixels wide at this zoom. Zoom in on time to see them."
                        </div>
                    })}
                    {move || out_of_view.get().map(|o| view! {
                        <div class="call-sketch-warn">{out_of_view_text(o)}</div>
                    })}
                </div>
            </div>
        })
    }
}

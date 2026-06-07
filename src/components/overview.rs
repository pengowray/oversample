use crate::state::store_fields::*;
use std::cell::RefCell;
use leptos::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData, MouseEvent};
use crate::state::{AppState, OverviewView};
use crate::types::PreviewImage;

/// Cached min/max envelope for overview waveform rendering.
/// Instead of iterating all samples (~57M for a 20MB MP3) on every frame,
/// we precompute a per-pixel min/max envelope once and render from that.
struct WaveformEnvelope {
    /// Interleaved [min0, max0, min1, max1, ...] for each pixel column.
    data: Vec<f32>,
    /// Cache key: (samples_ptr, samples_len, pixel_width, gain_bits)
    key: (usize, usize, u32, u64),
}

/// Cached off-screen canvas holding the fully rendered waveform bitmap.
/// On scroll we just `drawImage` from this instead of re-running ~1000
/// canvas path operations per frame.
struct WaveformCanvasCache {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    /// Cache key: (samples_ptr, samples_len, canvas_width, canvas_height, gain_bits)
    key: (usize, usize, u32, u32, u64),
}

thread_local! {
    /// Reusable off-screen canvas for the overview preview blit.
    static OVERVIEW_TMP: RefCell<Option<(HtmlCanvasElement, CanvasRenderingContext2d)>> =
        const { RefCell::new(None) };
    /// Cached identity of the last preview rendered to the tmp canvas.
    /// Stores (Arc data pointer, width, height) to detect when we need to re-render.
    static OVERVIEW_CACHED_PREVIEW: RefCell<(usize, u32, u32)> =
        const { RefCell::new((0, 0, 0)) };
    /// Cached min/max envelope for overview waveform.
    static OVERVIEW_ENVELOPE: RefCell<Option<WaveformEnvelope>> =
        const { RefCell::new(None) };
    /// Cached rendered waveform bitmap (off-screen canvas).
    static OVERVIEW_WAVEFORM_CANVAS: RefCell<Option<WaveformCanvasCache>> =
        const { RefCell::new(None) };
}

fn get_overview_tmp_canvas(w: u32, h: u32) -> Option<(HtmlCanvasElement, CanvasRenderingContext2d)> {
    OVERVIEW_TMP.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some((ref c, ref ctx)) = *slot {
            if c.width() >= w && c.height() >= h {
                return Some((c.clone(), ctx.clone()));
            }
        }
        let doc = web_sys::window()?.document()?;
        let c = doc.create_element("canvas").ok()?
            .dyn_into::<HtmlCanvasElement>().ok()?;
        c.set_width(w);
        c.set_height(h);
        let ctx = c.get_context("2d").ok()??.dyn_into::<CanvasRenderingContext2d>().ok()?;
        *slot = Some((c.clone(), ctx.clone()));
        Some((c, ctx))
    })
}

// ── Navigation helpers ────────────────────────────────────────────────────────

fn push_nav(state: &AppState) {
    state.push_nav();
}

// ── Rendering helpers ─────────────────────────────────────────────────────────

fn get_canvas_ctx(canvas: &HtmlCanvasElement) -> Option<CanvasRenderingContext2d> {
    canvas
        .get_context("2d")
        .ok()?
        .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok())
}

/// Blit a PreviewImage (RGBA) to the entire canvas at full width.
/// Also draws a viewport highlight rect (matching both time AND freq range of the main view)
/// and bookmark/playhead dots.
fn draw_overview_spectrogram(
    ctx: &CanvasRenderingContext2d,
    canvas: &HtmlCanvasElement,
    preview: &PreviewImage,
    scroll_offset: f64,       // main view left edge, seconds
    zoom: f64,                // main view zoom (px per spectrogram column)
    spec_time_res: f64,       // seconds per full-FFT spectrogram column (for viewport width)
    total_duration: f64,      // total file duration in seconds (from audio, always correct)
    main_canvas_width: f64,   // actual pixel width of main spectrogram canvas
    main_freq_crop_lo: f64,   // 0..1: low fraction of Nyquist shown in main view
    main_freq_crop_hi: f64,   // 0..1: high fraction of Nyquist shown in main view
    bookmarks: &[(f64,)],
    overview_freq_crop: f64,  // 0..1: fraction shown in the overview itself
    band_ff_range: Option<(f64, f64)>, // BandFF range as (lo_frac, hi_frac) of Nyquist
    clean_view: bool,         // hide all overlays (viewport rect, bookmarks, BandFF range)
) {
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, cw, ch);

    if preview.width == 0 || preview.height == 0 {
        return;
    }

    // Vertical crop: show overview_freq_crop fraction of the image (low-to-mid freqs hidden)
    let ofc = overview_freq_crop.clamp(0.01, 1.0);
    let full_h = preview.height as f64;
    let src_y = full_h * (1.0 - ofc);
    let src_h = full_h * ofc;

    // Blit via temporary canvas, caching the preview to avoid re-creating
    // ImageData on every scroll-triggered redraw.
    if let Some((tmp, tc)) = get_overview_tmp_canvas(preview.width, preview.height) {
        let preview_id = std::sync::Arc::as_ptr(&preview.pixels) as usize;
        let needs_render = OVERVIEW_CACHED_PREVIEW.with(|cell| {
            let cached = *cell.borrow();
            if cached != (preview_id, preview.width, preview.height) {
                *cell.borrow_mut() = (preview_id, preview.width, preview.height);
                true
            } else {
                false
            }
        });
        if needs_render {
            let clamped = Clamped(&preview.pixels[..]);
            if let Ok(img) = ImageData::new_with_u8_clamped_array_and_sh(
                clamped, preview.width, preview.height,
            ) {
                let _ = tc.put_image_data(&img, 0.0, 0.0);
            }
        }
        let _ = ctx.draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
            &tmp,
            0.0, src_y,
            preview.width as f64, src_h,
            0.0, 0.0,
            cw, ch,
        );
    }

    // Convert to px/sec using the true audio duration (not preview columns × FFT hop)
    if total_duration <= 0.0 { return; }
    let px_per_sec = cw / total_duration;

    if !clean_view {
        // ── Viewport highlight rect ───────────────────────────────────────────────
        // Horizontal: scroll_offset (left edge) + visible time
        // visible_time = (canvas_px / zoom) * spec_time_res  (zoom = px per FFT column)
        let visible_cols = main_canvas_width / zoom.max(0.001);
        let visible_time = visible_cols * spec_time_res;
        let vp_x = (scroll_offset * px_per_sec).max(0.0);
        let vp_w = (visible_time * px_per_sec).max(2.0);

        // Vertical: map main view freq range into the overview's freq coordinate space.
        // overview y=0 → top freq (ofc * Nyquist), y=ch → 0 Hz.
        let vp_y1 = (ch * (1.0 - main_freq_crop_hi / ofc)).clamp(0.0, ch);
        let vp_y2 = (ch * (1.0 - main_freq_crop_lo / ofc)).clamp(0.0, ch);
        let vp_h = vp_y2 - vp_y1;

        ctx.set_fill_style_str("rgba(80, 180, 130, 0.12)");
        ctx.fill_rect(vp_x, vp_y1, vp_w, vp_h);
        ctx.set_stroke_style_str("rgba(80, 180, 130, 0.55)");
        ctx.set_line_width(1.0);
        ctx.stroke_rect(vp_x, vp_y1, vp_w, vp_h);

        // BandFF range highlight (nested inside viewport rect)
        if let Some((band_ff_lo, band_ff_hi)) = band_ff_range {
            let band_ff_y1 = (ch * (1.0 - band_ff_hi / ofc)).clamp(0.0, ch);
            let band_ff_y2 = (ch * (1.0 - band_ff_lo / ofc)).clamp(0.0, ch);
            if band_ff_y2 - band_ff_y1 > 0.5 {
                ctx.set_fill_style_str("rgba(120, 200, 160, 0.15)");
                ctx.fill_rect(vp_x, band_ff_y1, vp_w, band_ff_y2 - band_ff_y1);
                ctx.set_stroke_style_str("rgba(120, 200, 160, 0.7)");
                ctx.set_line_width(1.0);
                ctx.stroke_rect(vp_x, band_ff_y1, vp_w, band_ff_y2 - band_ff_y1);
            }
        }

        // Bookmark dots (yellow, top edge)
        ctx.set_fill_style_str("rgba(255, 200, 50, 0.9)");
        for &(t,) in bookmarks {
            let x = t * px_per_sec;
            if x >= 0.0 && x <= cw {
                ctx.begin_path();
                let _ = ctx.arc(x, 5.0, 3.0, 0.0, std::f64::consts::TAU);
                ctx.fill();
            }
        }
    }

}

/// Compute a min/max envelope from samples: one (min, max) pair per pixel column.
/// This is O(total_samples) but only runs once; subsequent frames render from
/// the cached envelope in O(canvas_width).
fn compute_envelope(samples: &[f32], pixel_width: u32, gain_linear: f64) -> Vec<f32> {
    let pw = pixel_width as usize;
    if pw == 0 || samples.is_empty() {
        return Vec::new();
    }
    let mut envelope = vec![0.0f32; pw * 2]; // [min0, max0, min1, max1, ...]
    let samples_per_px = samples.len() as f64 / pw as f64;
    for px in 0..pw {
        let i0 = (px as f64 * samples_per_px) as usize;
        let i1 = (((px + 1) as f64 * samples_per_px) as usize).min(samples.len());
        if i0 >= i1 { continue; }
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for &s in &samples[i0..i1] {
            if s < lo { lo = s; }
            if s > hi { hi = s; }
        }
        envelope[px * 2] = lo * gain_linear as f32;
        envelope[px * 2 + 1] = hi * gain_linear as f32;
    }
    envelope
}

fn draw_overview_waveform(
    ctx: &CanvasRenderingContext2d,
    canvas: &HtmlCanvasElement,
    samples: &[f32],
    sample_rate: u32,
    time_resolution: f64,
    scroll_offset: f64,
    zoom: f64,
    main_canvas_width: f64,
    bookmarks: &[(f64,)],
    gain_db: f64,
    clean_view: bool,
) {
    let cw = canvas.width() as u32;
    let ch = canvas.height() as u32;
    if samples.is_empty() || cw == 0 || ch == 0 { return; }

    let total_duration = samples.len() as f64 / sample_rate as f64;

    // Cache key includes dimensions + gain so we re-render on resize or gain change.
    let cache_key = (samples.as_ptr() as usize, samples.len(), cw, ch, gain_db.to_bits());

    // Get or create the cached off-screen canvas with the rendered waveform.
    // The waveform bitmap only changes when the file, canvas size, or gain changes —
    // NOT on scroll. This turns per-frame cost from ~1000 path ops to a single drawImage.
    let cache_hit = OVERVIEW_WAVEFORM_CANVAS.with(|cell| {
        let slot = cell.borrow();
        if let Some(ref cached) = *slot {
            if cached.key == cache_key {
                // Cache hit — blit the pre-rendered waveform in one GPU call.
                let _ = ctx.draw_image_with_html_canvas_element(&cached.canvas, 0.0, 0.0);
                return true;
            }
        }
        false
    });

    if !cache_hit {
        let gain_linear = 10.0f64.powf(gain_db / 20.0);

        // Get or compute the min/max envelope (O(total_samples) once).
        let envelope_key = (samples.as_ptr() as usize, samples.len(), cw, gain_db.to_bits());
        let envelope = OVERVIEW_ENVELOPE.with(|cell| {
            let mut slot = cell.borrow_mut();
            if let Some(ref cached) = *slot {
                if cached.key == envelope_key {
                    return cached.data.clone();
                }
            }
            let env = compute_envelope(samples, cw, gain_linear);
            *slot = Some(WaveformEnvelope { data: env.clone(), key: envelope_key });
            env
        });

        // Create or reuse the off-screen canvas and render the waveform to it.
        OVERVIEW_WAVEFORM_CANVAS.with(|cell| {
            let mut slot = cell.borrow_mut();

            // Ensure we have an off-screen canvas of the right size.
            let needs_create = match *slot {
                Some(ref c) => c.canvas.width() != cw || c.canvas.height() != ch,
                None => true,
            };
            if needs_create {
                let doc = match web_sys::window().and_then(|w| w.document()) {
                    Some(d) => d,
                    None => return,
                };
                let c = match doc.create_element("canvas")
                    .ok()
                    .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
                {
                    Some(c) => c,
                    None => return,
                };
                c.set_width(cw);
                c.set_height(ch);
                let oc = match c.get_context("2d")
                    .ok()
                    .flatten()
                    .and_then(|o| o.dyn_into::<CanvasRenderingContext2d>().ok())
                {
                    Some(oc) => oc,
                    None => return,
                };
                *slot = Some(WaveformCanvasCache {
                    canvas: c,
                    ctx: oc,
                    key: cache_key,
                });
            }

            let cached = slot.as_mut().unwrap();

            // Render the static waveform to the off-screen canvas.
            let off_ctx = &cached.ctx;
            let mid_y = ch as f64 / 2.0;
            let scale = mid_y * 0.9;

            // Background
            off_ctx.set_fill_style_str("#0a0a0a");
            off_ctx.fill_rect(0.0, 0.0, cw as f64, ch as f64);

            // Center line
            off_ctx.set_stroke_style_str("#333");
            off_ctx.set_line_width(1.0);
            off_ctx.begin_path();
            off_ctx.move_to(0.0, mid_y);
            off_ctx.line_to(cw as f64, mid_y);
            off_ctx.stroke();

            // Waveform envelope
            off_ctx.set_stroke_style_str("#4a4");
            off_ctx.set_line_width(1.0);
            off_ctx.begin_path();
            let pw = cw as usize;
            for px in 0..pw {
                let lo = envelope[px * 2] as f64;
                let hi = envelope[px * 2 + 1] as f64;
                let y_top = mid_y - hi * scale;
                let y_bot = mid_y - lo * scale;
                let x = px as f64;
                off_ctx.move_to(x, y_top);
                off_ctx.line_to(x, y_bot);
            }
            off_ctx.stroke();

            // Update cache key and blit to the main canvas.
            cached.key = cache_key;
            let _ = ctx.draw_image_with_html_canvas_element(&cached.canvas, 0.0, 0.0);
        });
    }

    if !clean_view {
        let px_per_sec = cw as f64 / total_duration;
        let visible_cols = main_canvas_width / zoom.max(0.001);
        let visible_time = visible_cols * time_resolution;
        let vp_x = (scroll_offset * px_per_sec).max(0.0);
        let vp_w = (visible_time * px_per_sec).max(2.0);
        ctx.set_fill_style_str("rgba(80, 180, 130, 0.12)");
        ctx.fill_rect(vp_x, 0.0, vp_w, ch as f64);
        ctx.set_stroke_style_str("rgba(80, 180, 130, 0.55)");
        ctx.set_line_width(1.0);
        ctx.stroke_rect(vp_x, 0.0, vp_w, ch as f64);

        // Bookmark dots
        ctx.set_fill_style_str("rgba(255, 200, 50, 0.9)");
        for &(t,) in bookmarks {
            let x = t * px_per_sec;
            if x >= 0.0 && x <= cw as f64 {
                ctx.begin_path();
                let _ = ctx.arc(x, 5.0, 3.0, 0.0, std::f64::consts::TAU);
                ctx.fill();
            }
        }
    }
}

/// Draw the "● Recording/Listening …" status text + VU bar. Used in the live
/// overview when there's no waterfall data to render yet.
fn draw_live_status(
    ctx: &CanvasRenderingContext2d,
    w: u32,
    h: u32,
    file: &crate::state::LoadedFile,
    state: &AppState,
) {
    ctx.set_fill_style_str("#1a1a1a");
    ctx.fill_rect(0.0, 0.0, w as f64, h as f64);
    let is_listen = file.is_live_listen;
    let label = if is_listen { "Listening" } else { "Recording" };
    let elapsed = crate::canvas::live_waterfall::total_time().max(file.audio.duration_secs);
    let text = if elapsed >= 1.0 {
        format!("\u{25CF} {} {}:{:02}", label, elapsed as u32 / 60, elapsed as u32 % 60)
    } else {
        format!("\u{25CF} {}\u{2026}", label)
    };
    let color = if is_listen { "#6af" } else { "#f66" };
    ctx.set_fill_style_str(color);
    ctx.set_font("11px system-ui");
    ctx.set_text_align("center");
    ctx.set_text_baseline("middle");
    let _ = ctx.fill_text(&text, w as f64 / 2.0, h as f64 / 2.0);
    let peak = state.mic.peak_level().get_untracked();
    if peak > 0.01 {
        let bar_w = (peak as f64 * w as f64).min(w as f64);
        let vu_color = if is_listen { "#48f" } else { "#f44" };
        ctx.set_fill_style_str(vu_color);
        ctx.fill_rect(0.0, h as f64 - 2.0, bar_w, 2.0);
    }
}

/// The single time window the live overview displays, in absolute session
/// seconds: `(axis_start, span)` where the right edge is always `axis_start +
/// span == now`. Both overviews (waveform + spectrogram), the time markers, the
/// viewport rect, and click/drag mapping all derive from this one window so
/// they share one scale.
///
/// The window is the raw-sample ring we still hold (`now - audio_dur`), clamped
/// so we never claim more history than the waterfall retains. In the common
/// listen/stream case `audio_dur == preroll_buffer_secs + 2` (~12 s), so the
/// whole live overview shows the last ~12 s at one consistent scale. Returns
/// `None` when not live or there's no data yet.
pub(crate) fn live_overview_window(is_tauri: bool) -> Option<(f64, f64)> {
    use crate::canvas::live_waterfall as wf;
    if !wf::is_active() {
        return None;
    }
    let now = wf::total_time();
    if now <= 0.0 {
        return None;
    }
    let sr = wf::sample_rate() as f64;
    if sr <= 0.0 {
        return None;
    }
    let ring_len = crate::audio::mic_backend::with_live_samples(is_tauri, |s| s.len());
    let audio_dur = ring_len as f64 / sr;
    let axis_start = (now - audio_dur).max(wf::oldest_time()).max(0.0);
    let span = (now - axis_start).max(0.001);
    Some((axis_start, span))
}

/// Apply a scrub (overview click/drag) to the main-view scroll. The desired
/// scroll is clamped to the range the overview actually displays — during live
/// that's the shared ring window `[axis_start, now - visible]`, so clicking the
/// far-left lands on the far-left rather than centering past it — and the live
/// waterfall follow is suspended while scrubbing (no snap-back) but re-engaged
/// when scrubbed to the live edge. `visible` is the main view's visible span.
fn scrub_apply_scroll(state: &AppState, desired_scroll: f64, visible: f64, full_duration: f64) {
    let is_live = (state.mic.recording().get_untracked() || state.mic.listening().get_untracked())
        && state.timeline.active().get_untracked().is_none()
        && crate::canvas::live_waterfall::is_active();
    if is_live {
        if let Some((axis_start, span)) = live_overview_window(state.is_tauri) {
            let now = axis_start + span;
            let hi = (now - visible).max(axis_start);
            let s = desired_scroll.clamp(axis_start, hi);
            state.view.scroll_offset().set(s);
            if s >= hi - visible * 0.02 {
                state.resume_waterfall_follow();
            } else {
                state.suspend_waterfall_follow(2000.0);
            }
            return;
        }
    }
    let max_scroll = (full_duration - visible).max(0.0);
    state.view.scroll_offset().set(desired_scroll.clamp(0.0, max_scroll));
}

/// Draw a min/max waveform envelope for the live overview, recomputed fresh on
/// every call (no persistent cache) so it tracks the live ring at the capture
/// cadence rather than the ~1 Hz snapshot. Visual style matches the static
/// overview waveform.
fn draw_live_waveform(
    ctx: &CanvasRenderingContext2d,
    w: u32,
    h: u32,
    samples: &[f32],
    gain_db: f64,
) {
    if samples.is_empty() || w == 0 || h == 0 {
        return;
    }
    let gain_linear = 10.0f64.powf(gain_db / 20.0);
    let env = compute_envelope(samples, w, gain_linear);
    let cw = w as f64;
    let ch = h as f64;
    let mid_y = ch / 2.0;
    let scale = mid_y * 0.9;

    ctx.set_fill_style_str("#0a0a0a");
    ctx.fill_rect(0.0, 0.0, cw, ch);

    ctx.set_stroke_style_str("#333");
    ctx.set_line_width(1.0);
    ctx.begin_path();
    ctx.move_to(0.0, mid_y);
    ctx.line_to(cw, mid_y);
    ctx.stroke();

    ctx.set_stroke_style_str("#4a4");
    ctx.set_line_width(1.0);
    ctx.begin_path();
    for px in 0..w as usize {
        let lo = env[px * 2] as f64;
        let hi = env[px * 2 + 1] as f64;
        let x = px as f64;
        ctx.move_to(x, mid_y - hi * scale);
        ctx.line_to(x, mid_y - lo * scale);
    }
    ctx.stroke();
}

// ── Overview toggle button ────────────────────────────────────────────────────

#[component]
fn OverviewToggleButton() -> impl IntoView {
    let state = expect_context::<AppState>();

    let label = move || match state.viewmode.overview_view().get() {
        OverviewView::Spectrogram => "Spectrum",
        OverviewView::Waveform => "Waveform",
    };

    let toggle = move |_: MouseEvent| {
        let next = match state.viewmode.overview_view().get_untracked() {
            OverviewView::Spectrogram => OverviewView::Waveform,
            OverviewView::Waveform => OverviewView::Spectrogram,
        };
        state.viewmode.overview_view().set(next);
    };

    view! {
        <div
            style="position: absolute; bottom: 4px; left: 64px; pointer-events: none;"
            on:click=|ev: MouseEvent| ev.stop_propagation()
            on:touchstart=|ev: web_sys::TouchEvent| ev.stop_propagation()
        >
            <div style="position: relative; pointer-events: auto;">
                <button
                    class="layer-btn"
                    style="font-size: 10px; padding: 3px 7px;"
                    on:click=toggle
                    title="Toggle overview display"
                >{label}</button>
            </div>
        </div>
    }
}

// ── Helpers for sizing a canvas to its CSS pixel dimensions ──────────────────

/// Resize a canvas element's bitmap to match its CSS layout size.
/// Returns (width, height) on success, or None if the canvas has zero dimensions.
fn size_canvas_to_display(canvas: &HtmlCanvasElement) -> Option<(u32, u32)> {
    let w = canvas.client_width() as u32;
    let h = canvas.client_height() as u32;
    if w == 0 || h == 0 { return None; }
    if canvas.width() != w { canvas.set_width(w); }
    if canvas.height() != h { canvas.set_height(h); }
    Some((w, h))
}

// ── Main OverviewPanel component ──────────────────────────────────────────────

#[component]
pub fn OverviewPanel() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    let overlay_ref = NodeRef::<leptos::html::Canvas>::new();

    // Dragging state: (is_dragging, initial_client_x, initial_scroll)
    let drag_active = RwSignal::new(false);
    let drag_start_x = RwSignal::new(0.0f64);
    let drag_start_scroll = RwSignal::new(0.0f64);

    // Smoothed live "now" (right edge of the overview window), eased toward the
    // real latest-data time on each overlay redraw so the time markers slide
    // smoothly instead of stepping at the ~20 Hz capture tick — matching the
    // smooth main-view time axis. Non-reactive: it's interpolation state, not a
    // signal anything subscribes to.
    let smooth_now = StoredValue::new(0.0f64);

    // ── Background Effect ── draws waveform/spectrogram content.
    // Does NOT subscribe to scroll_offset or zoom_level, so panning/zooming
    // only triggers the cheap overlay Effect below instead of re-blitting
    // the entire waveform from the off-screen cache on every frame.
    Effect::new(move || {
        state.library.files().track();
        let files = state.library.files().get_untracked();
        let _timeline_trigger = state.timeline.active().get();
        let idx = state.library.current_index().get();
        let overview_view = state.viewmode.overview_view().get();
        let cv = state.viewmode.channel_view().get();
        let _mic_recording = state.mic.recording().get();
        let _mic_listening = state.mic.listening().get();
        // Re-render the live overview on each capture tick. `live_data_cols` is
        // bumped by the processing loop while recording/listening and stays 0
        // otherwise, so static files don't pay for this.
        let _live_cols = state.mic.live_data_cols().get();
        let auto_gain = state.gain.auto().get();
        let gain_db = if auto_gain { state.compute_auto_gain_untracked() } else { state.gain.db().get() };
        // Re-read canvas dimensions when sidebar layout changes
        let _sidebar = state.panels.left_collapsed().get();
        let _sidebar_width = state.panels.left_width().get();
        let _rsidebar = state.panels.right_collapsed().get();
        let _rsidebar_width = state.panels.right_width().get();

        let Some(canvas_el) = canvas_ref.get() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();

        let Some((w, h)) = size_canvas_to_display(canvas) else { return };

        let Some(ctx) = get_canvas_ctx(canvas) else { return };
        ctx.set_fill_style_str("#000");
        ctx.fill_rect(0.0, 0.0, w as f64, h as f64);

        let timeline = state.timeline.active().get_untracked();

        if let Some(ref tl) = timeline {
            // ── Timeline mode: render segment previews ──
            let total_duration = tl.total_duration_secs;
            if total_duration <= 0.0 { return; }
            let cw = w as f64;
            let ch = h as f64;
            let px_per_sec = cw / total_duration;

            for seg in &tl.segments {
                let seg_file = match files.get(seg.file_index) {
                    Some(f) => f,
                    None => continue,
                };
                let overview_src = seg_file.overview_image.as_ref().or(seg_file.preview.as_ref());
                let Some(preview) = overview_src else { continue };

                let seg_x = seg.timeline_offset_secs * px_per_sec;
                let seg_w = seg.duration_secs * px_per_sec;
                if seg_w < 1.0 { continue; }

                ctx.save();
                ctx.begin_path();
                ctx.rect(seg_x, 0.0, seg_w, ch);
                ctx.clip();

                if let Some((tmp, tc)) = get_overview_tmp_canvas(preview.width, preview.height) {
                    let clamped = wasm_bindgen::Clamped(preview.pixels.as_slice());
                    if let Ok(img) = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
                        clamped, preview.width, preview.height,
                    ) {
                        let _ = tc.put_image_data(&img, 0.0, 0.0);
                        let _ = ctx.draw_image_with_html_canvas_element_and_dw_and_dh(
                            &tmp, seg_x, 0.0, seg_w, ch,
                        );
                    }
                }

                ctx.restore();
            }
            // Viewport rect, gap indicators, and time markers are drawn by the overlay Effect.
        } else {
            // ── Single file overview ──
            let file_opt = idx.and_then(|i| files.get(i));

            if file_opt.is_none() {
                let is_rec = state.mic.recording().get_untracked();
                let is_lis = state.mic.listening().get_untracked();
                if is_rec || is_lis {
                    ctx.set_fill_style_str("#1a1a1a");
                    ctx.fill_rect(0.0, 0.0, w as f64, h as f64);
                    let label = if is_rec { "Recording" } else { "Listening" };
                    let color = if is_rec { "#f66" } else { "#6cf" };
                    ctx.set_fill_style_str(color);
                    ctx.set_font("11px system-ui");
                    ctx.set_text_align("center");
                    ctx.set_text_baseline("middle");
                    let _ = ctx.fill_text(&format!("\u{25CF} {}\u{2026}", label), w as f64 / 2.0, h as f64 / 2.0);
                    let peak = state.mic.peak_level().get_untracked();
                    if peak > 0.01 {
                        let bar_w = (peak as f64 * w as f64).min(w as f64);
                        ctx.set_fill_style_str(color);
                        ctx.fill_rect(0.0, h as f64 - 2.0, bar_w, 2.0);
                    }
                }
                return;
            }
            let file = file_opt.unwrap();

            // During live listening/recording, the overview shows ONLY the
            // shared raw-sample-ring window (the last ~audio_dur s we still hold,
            // ~12 s by default), not the full elapsed session — and the
            // spectrogram, waveform, and time markers all use that same window so
            // toggling never changes the horizontal scale.
            let is_live = (file.is_recording || file.is_live_listen)
                && crate::canvas::live_waterfall::is_active();
            if is_live {
                // Both views span the SAME window — the raw-sample ring we still
                // hold ([now - audio_dur, now], ~12 s by default) — so toggling
                // between them never changes the horizontal scale.
                let window = live_overview_window(state.is_tauri);
                match overview_view {
                    OverviewView::Spectrogram => {
                        // Crop the waterfall to the same span the waveform shows.
                        let recent_cols = window.map(|(_, span)| {
                            (span / crate::canvas::live_waterfall::time_resolution()).ceil() as usize
                        });
                        if let Some(img) = crate::canvas::live_waterfall::render_overview(w, h, recent_cols) {
                            draw_overview_spectrogram(
                                &ctx, canvas, &img,
                                0.0, 1.0, file.spectrogram.time_resolution,
                                0.0, 0.0, 0.0, 1.0, &[], 1.0, None, true,
                            );
                        } else {
                            draw_live_status(&ctx, w, h, file, &state);
                        }
                    }
                    OverviewView::Waveform => {
                        // Read the live ring directly (fresh each tick) and draw
                        // exactly the window's worth of its newest samples.
                        let drew = match window {
                            Some((_, span)) => crate::audio::mic_backend::with_live_samples(
                                state.is_tauri,
                                |ring| {
                                    if ring.is_empty() { return false; }
                                    let n = ((span * file.audio.sample_rate as f64) as usize)
                                        .clamp(1, ring.len());
                                    draw_live_waveform(&ctx, w, h, &ring[ring.len() - n..], gain_db);
                                    true
                                },
                            ),
                            None => false,
                        };
                        if !drew {
                            draw_live_status(&ctx, w, h, file, &state);
                        }
                    }
                }
            } else {
            match overview_view {
                OverviewView::Spectrogram => {
                    let overview_src = file.overview_image.as_ref().or(file.preview.as_ref());
                    if let Some(preview) = overview_src {
                        // Draw spectrogram image only (clean_view=true skips all overlays).
                        draw_overview_spectrogram(
                            &ctx, canvas, preview,
                            0.0, 1.0,
                            file.spectrogram.time_resolution,
                            file.audio.duration_secs,
                            0.0,
                            0.0, 1.0,
                            &[],
                            1.0,
                            None,
                            true, // clean_view — overlays drawn by overlay Effect
                        );
                    } else if file.is_recording && !file.audio.samples.is_empty() {
                        let scroll = state.view.scroll_offset().get_untracked();
                        let is_live_wf = (file.is_live_listen || file.is_recording)
                            && crate::canvas::live_waterfall::is_active();
                        let buf_scroll = if is_live_wf {
                            let wf_total = crate::canvas::live_waterfall::total_time();
                            let offset = (wf_total - file.audio.duration_secs).max(0.0);
                            (scroll - offset).clamp(0.0, file.audio.duration_secs)
                        } else {
                            scroll
                        };
                        draw_overview_waveform(
                            &ctx, canvas, &file.audio.samples,
                            file.audio.sample_rate,
                            file.spectrogram.time_resolution,
                            buf_scroll, 1.0, 0.0,
                            &[], gain_db, true,
                        );
                    } else if file.is_recording {
                        ctx.set_fill_style_str("#1a1a1a");
                        ctx.fill_rect(0.0, 0.0, w as f64, h as f64);
                        let is_listen = file.is_live_listen;
                        let label = if is_listen { "Listening" } else { "Recording" };
                        let elapsed = if is_listen && crate::canvas::live_waterfall::is_active() {
                            crate::canvas::live_waterfall::total_time()
                        } else {
                            file.audio.duration_secs
                        };
                        let text = if elapsed >= 1.0 {
                            let mins = elapsed as u32 / 60;
                            let secs = elapsed as u32 % 60;
                            format!("\u{25CF} {} {}:{:02}", label, mins, secs)
                        } else {
                            format!("\u{25CF} {}\u{2026}", label)
                        };
                        let color = if is_listen { "#6af" } else { "#f66" };
                        ctx.set_fill_style_str(color);
                        ctx.set_font("11px system-ui");
                        ctx.set_text_align("center");
                        ctx.set_text_baseline("middle");
                        let _ = ctx.fill_text(&text, w as f64 / 2.0, h as f64 / 2.0);
                        let peak = state.mic.peak_level().get_untracked();
                        if peak > 0.01 {
                            let bar_w = (peak as f64 * w as f64).min(w as f64);
                            let vu_color = if is_listen { "#48f" } else { "#f44" };
                            ctx.set_fill_style_str(vu_color);
                            ctx.fill_rect(0.0, h as f64 - 2.0, bar_w, 2.0);
                        }
                    } else if file.loading_id.is_some() {
                        // A real file still being decoded — show a loading hint.
                        ctx.set_fill_style_str("#333");
                        ctx.fill_rect(0.0, 0.0, w as f64, h as f64);
                        ctx.set_fill_style_str("#666");
                        ctx.set_font("11px system-ui");
                        ctx.set_text_align("center");
                        ctx.set_text_baseline("middle");
                        let _ = ctx.fill_text("Loading\u{2026}", w as f64 / 2.0, h as f64 / 2.0);
                    }
                    // else: an empty / armed live placeholder (no samples, no
                    // preview, not loading) — leave the strip blank rather than
                    // showing a misleading "Loading…".
                }
                OverviewView::Waveform => {
                    let ov_buf;
                    // Cap non-MonoMix reads to file.audio.samples.len() so
                    // streaming sources don't allocate gigabytes for
                    // multi-hour files.
                    let read_len = file.audio.samples.len();
                    let ov_samples: &[f32] = match cv {
                        crate::audio::source::ChannelView::MonoMix => &file.audio.samples,
                        _ => {
                            ov_buf = file.audio.source.read_region(cv, 0, read_len);
                            &ov_buf
                        }
                    };
                    draw_overview_waveform(
                        &ctx, canvas,
                        ov_samples,
                        file.audio.sample_rate,
                        file.spectrogram.time_resolution,
                        0.0, 1.0, 0.0,
                        &[], gain_db, true, // clean_view — overlays drawn by overlay Effect
                    );
                }
            }
            } // end else (not live)
            // Time markers are drawn by the overlay Effect.
        }
    });

    // ── Overlay Effect ── draws viewport rect, bookmarks, time markers on a
    // transparent canvas layered on top. This subscribes to scroll_offset and
    // zoom_level but is very cheap (no drawImage blit, just a few shapes).
    Effect::new(move || {
        let scroll = state.view.scroll_offset().get();
        let zoom = state.view.zoom_level().get();
        let bookmarks = state.viewmode.bookmarks().get();
        let clean_view = state.viewmode.clean_view().get();
        let main_canvas_w = state.viewmode.spectrogram_canvas_width().get();
        let min_display_freq = state.view.min_display_freq().get();
        let max_display_freq = state.view.max_display_freq().get();
        let band_ff_lo_hz = state.filter.band_ff_freq_lo().get();
        let band_ff_hi_hz = state.filter.band_ff_freq_hi().get();
        let overview_view = state.viewmode.overview_view().get();
        // Re-sync overlay dimensions when sidebar layout changes
        let _sidebar = state.panels.left_collapsed().get();
        let _sidebar_width = state.panels.left_width().get();
        let _rsidebar = state.panels.right_collapsed().get();
        let _rsidebar_width = state.panels.right_width().get();
        // Redraw when file changes (duration, freq info)
        state.library.files().track();
        let _idx = state.library.current_index().get();
        // During live capture the displayed window advances every tick; track
        // live_data_cols so the markers/viewport advance with the data (not only
        // when the follow-scroll animation happens to nudge scroll_offset).
        let _live_cols = state.mic.live_data_cols().get();

        let Some(canvas_el) = overlay_ref.get() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();

        let Some((w, h)) = size_canvas_to_display(canvas) else { return };

        let Some(ctx) = get_canvas_ctx(canvas) else { return };
        ctx.clear_rect(0.0, 0.0, w as f64, h as f64);

        if clean_view { return; }

        let cw = w as f64;
        let ch = h as f64;

        let timeline = state.timeline.active().get_untracked();

        if let Some(ref tl) = timeline {
            // ── Timeline overlay ──
            let total_duration = tl.total_duration_secs;
            if total_duration <= 0.0 { return; }
            let px_per_sec = cw / total_duration;

            let files = state.library.files().get_untracked();
            let primary_file = tl.segments.first().and_then(|s| files.get(s.file_index));
            let max_freq = primary_file.map(|f| f.spectrogram.max_freq).unwrap_or(96_000.0);
            let spec_time_res = primary_file.map(|f| f.spectrogram.time_resolution).unwrap_or(1.0);

            let main_freq_crop_hi = max_display_freq
                .map(|mdf| (mdf / max_freq).clamp(0.001, 1.0))
                .unwrap_or(1.0);
            let main_freq_crop_lo = min_display_freq
                .map(|mdf| (mdf / max_freq).clamp(0.0, 1.0))
                .unwrap_or(0.0);

            // Viewport rectangle
            let visible_cols = main_canvas_w / zoom.max(0.001);
            let visible_time_span = visible_cols * spec_time_res;
            let vp_x = (scroll * px_per_sec).max(0.0);
            let vp_w = (visible_time_span * px_per_sec).max(2.0);
            let vp_y1 = (ch * (1.0 - main_freq_crop_hi)).clamp(0.0, ch);
            let vp_y2 = (ch * (1.0 - main_freq_crop_lo)).clamp(0.0, ch);
            let vp_h = (vp_y2 - vp_y1).max(1.0);

            ctx.set_stroke_style_str("rgba(255, 255, 255, 0.7)");
            ctx.set_line_width(1.5);
            ctx.stroke_rect(vp_x, vp_y1, vp_w, vp_h);

            // Gap indicators
            for i in 0..tl.segments.len() {
                let seg_end = tl.segments[i].timeline_offset_secs + tl.segments[i].duration_secs;
                if i + 1 < tl.segments.len() {
                    let next_start = tl.segments[i + 1].timeline_offset_secs;
                    if next_start > seg_end + 0.001 {
                        let gap_x = seg_end * px_per_sec;
                        let gap_w = (next_start - seg_end) * px_per_sec;
                        ctx.set_fill_style_str("rgba(50, 50, 80, 0.5)");
                        ctx.fill_rect(gap_x, 0.0, gap_w, ch);
                    }
                }
            }

            // Time markers
            let clock_cfg = if tl.origin_epoch_ms > 0.0 {
                Some(crate::canvas::time_markers::ClockTimeConfig {
                    recording_start_epoch_ms: tl.origin_epoch_ms,
                })
            } else {
                None
            };
            crate::canvas::time_markers::draw_time_markers(
                &ctx,
                0.0,
                total_duration,
                cw,
                ch,
                total_duration,
                clock_cfg,
                state.timeline.show_clock_time().get(),
                1.0,
            );
        } else {
            // ── Single file overlay ──
            let files = state.library.files().get_untracked();
            let idx = state.library.current_index().get_untracked();
            let file_opt = idx.and_then(|i| files.get(i));
            let Some(file) = file_opt else { return };

            let max_freq = if file.spectrogram.max_freq > 0.0 {
                file.spectrogram.max_freq
            } else {
                file.audio.sample_rate as f64 / 2.0
            };
            // During live, the overview spans the shared raw-sample-ring window
            // [now - audio_dur, now] (same as the waveform/spectrogram drawn
            // beneath); otherwise the whole file. axis_start offsets the viewport
            // rect / markers so they line up with the background.
            let is_live = (file.is_recording || file.is_live_listen)
                && crate::canvas::live_waterfall::is_active();
            let (axis_start, total_duration) = if is_live {
                match live_overview_window(state.is_tauri) {
                    Some((axis_real, span)) => {
                        // Ease the displayed right edge toward the real latest-
                        // data time so markers don't step at the capture tick.
                        // Snap on first use / large gaps (new session, long
                        // pause), interpolate otherwise. The window keeps the
                        // real ring width and slides as a whole.
                        let now_real = axis_real + span;
                        let prev = smooth_now.get_value();
                        let now_smooth = if prev <= 0.0 || (now_real - prev).abs() > span {
                            now_real
                        } else {
                            prev + (now_real - prev) * 0.35
                        };
                        smooth_now.set_value(now_smooth);
                        ((now_smooth - span).max(0.0), span)
                    }
                    None => (0.0, file.audio.duration_secs),
                }
            } else {
                smooth_now.set_value(0.0); // reset between live sessions
                (0.0, file.audio.duration_secs)
            };
            if total_duration <= 0.0 { return; }
            let spec_time_res = file.spectrogram.time_resolution;
            let px_per_sec = cw / total_duration;

            // Viewport rectangle (offset by axis_start for the live window)
            let visible_cols = main_canvas_w / zoom.max(0.001);
            let visible_time = visible_cols * spec_time_res;
            let vp_x = ((scroll - axis_start) * px_per_sec).max(0.0);
            let vp_w = (visible_time * px_per_sec).max(2.0);

            match overview_view {
                OverviewView::Spectrogram => {
                    // Fractions of Nyquist shown in the main view
                    let main_freq_crop_hi = max_display_freq
                        .map(|mdf| (mdf / max_freq).clamp(0.001, 1.0))
                        .unwrap_or(1.0);
                    let main_freq_crop_lo = min_display_freq
                        .map(|mdf| (mdf / max_freq).clamp(0.0, 1.0))
                        .unwrap_or(0.0);

                    let ofc = 1.0; // overview always shows full frequency range
                    let vp_y1 = (ch * (1.0 - main_freq_crop_hi / ofc)).clamp(0.0, ch);
                    let vp_y2 = (ch * (1.0 - main_freq_crop_lo / ofc)).clamp(0.0, ch);
                    let vp_h = vp_y2 - vp_y1;

                    ctx.set_fill_style_str("rgba(80, 180, 130, 0.12)");
                    ctx.fill_rect(vp_x, vp_y1, vp_w, vp_h);
                    ctx.set_stroke_style_str("rgba(80, 180, 130, 0.55)");
                    ctx.set_line_width(1.0);
                    ctx.stroke_rect(vp_x, vp_y1, vp_w, vp_h);

                    // BandFF range highlight
                    if band_ff_hi_hz > band_ff_lo_hz {
                        let lo_frac = (band_ff_lo_hz / max_freq).clamp(0.0, 1.0);
                        let hi_frac = (band_ff_hi_hz.min(max_freq) / max_freq).clamp(0.0, 1.0);
                        let band_ff_y1 = (ch * (1.0 - hi_frac / ofc)).clamp(0.0, ch);
                        let band_ff_y2 = (ch * (1.0 - lo_frac / ofc)).clamp(0.0, ch);
                        if band_ff_y2 - band_ff_y1 > 0.5 {
                            ctx.set_fill_style_str("rgba(120, 200, 160, 0.15)");
                            ctx.fill_rect(vp_x, band_ff_y1, vp_w, band_ff_y2 - band_ff_y1);
                            ctx.set_stroke_style_str("rgba(120, 200, 160, 0.7)");
                            ctx.set_line_width(1.0);
                            ctx.stroke_rect(vp_x, band_ff_y1, vp_w, band_ff_y2 - band_ff_y1);
                        }
                    }
                }
                OverviewView::Waveform => {
                    // Full-height viewport rect
                    ctx.set_fill_style_str("rgba(80, 180, 130, 0.12)");
                    ctx.fill_rect(vp_x, 0.0, vp_w, ch);
                    ctx.set_stroke_style_str("rgba(80, 180, 130, 0.55)");
                    ctx.set_line_width(1.0);
                    ctx.stroke_rect(vp_x, 0.0, vp_w, ch);
                }
            }

            // Bookmark dots (offset by axis_start for the live window)
            ctx.set_fill_style_str("rgba(255, 200, 50, 0.9)");
            for bm in bookmarks.iter() {
                let x = (bm.time - axis_start) * px_per_sec;
                if x >= 0.0 && x <= cw {
                    ctx.begin_path();
                    let _ = ctx.arc(x, 5.0, 3.0, 0.0, std::f64::consts::TAU);
                    ctx.fill();
                }
            }

            // Time markers — span the live retained window [axis_start, now] or
            // the whole file. (scroll_offset arg = left-edge time, visible_time =
            // span shown, duration = end time.)
            let clock_cfg = file.recording_start_epoch_ms()
                .map(|ms| crate::canvas::time_markers::ClockTimeConfig {
                    recording_start_epoch_ms: ms,
                });
            crate::canvas::time_markers::draw_time_markers(
                &ctx,
                axis_start,
                total_duration,
                cw,
                ch,
                axis_start + total_duration,
                clock_cfg,
                state.timeline.show_clock_time().get(),
                1.0,
            );
        }
    });

    // ── Mouse handlers ────────────────────────────────────────────────────────

    // Get the true total duration (timeline, live waterfall, or single file)
    let file_duration = move || -> f64 {
        if let Some(ref tl) = state.timeline.active().get_untracked() {
            return tl.total_duration_secs;
        }
        let is_live = state.mic.recording().get_untracked() || state.mic.listening().get_untracked();
        if is_live && crate::canvas::live_waterfall::is_active() {
            return crate::canvas::live_waterfall::total_time();
        }
        let files = state.library.files().get_untracked();
        let idx = state.library.current_index().get_untracked();
        idx.and_then(|i| files.get(i))
            .map(|f| f.audio.duration_secs)
            .unwrap_or(0.0)
    };

    // Convert a click x-coordinate to a time offset (seconds). During live the
    // overview spans only the shared ring window [axis_start, now], so map
    // within that window rather than [0, now].
    let x_to_time = move |canvas_x: f64, canvas_w: f64| -> Option<f64> {
        if canvas_w <= 0.0 { return None; }
        let is_live = (state.mic.recording().get_untracked()
            || state.mic.listening().get_untracked())
            && state.timeline.active().get_untracked().is_none();
        if is_live {
            if let Some((axis_start, span)) = live_overview_window(state.is_tauri) {
                return Some(axis_start + (canvas_x / canvas_w) * span);
            }
        }
        let dur = file_duration();
        if dur <= 0.0 { return None; }
        Some((canvas_x / canvas_w) * dur)
    };

    // The time span the overview currently displays across its full width:
    // the shared ring window during live, else the whole file/timeline.
    let overview_span = move || -> f64 {
        if state.timeline.active().get_untracked().is_none() {
            let is_live = state.mic.recording().get_untracked()
                || state.mic.listening().get_untracked();
            if is_live {
                if let Some((_, span)) = live_overview_window(state.is_tauri) {
                    return span;
                }
            }
        }
        file_duration()
    };

    // Compute half the visible time window for centering clicks
    let half_visible_time = move || -> f64 {
        let files = state.library.files().get_untracked();
        let idx = state.library.current_index().get_untracked();
        idx.and_then(|i| files.get(i)).map(|f| {
            let zoom = state.view.zoom_level().get_untracked();
            let canvas_w = state.viewmode.spectrogram_canvas_width().get_untracked();
            (canvas_w / zoom) * f.spectrogram.time_resolution / 2.0
        }).unwrap_or(0.0)
    };

    let on_pointerdown = move |ev: web_sys::PointerEvent| {
        if state.status.viewport_zoomed().get_untracked() { return; }
        ev.prevent_default();
        let Some(canvas_el) = overlay_ref.get_untracked() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let canvas_x = ev.client_x() as f64 - rect.left();
        let cw = rect.width();
        if let Some(t) = x_to_time(canvas_x, cw) {
            push_nav(&state);
            let visible = half_visible_time() * 2.0;
            state.suspend_follow();
            scrub_apply_scroll(&state, t - half_visible_time(), visible, file_duration());
        }
        drag_active.set(true);
        drag_start_x.set(ev.client_x() as f64);
        drag_start_scroll.set(state.view.scroll_offset().get_untracked());
        // Capture pointer so drag continues when cursor leaves the overview strip
        if let Some(target) = ev.target() {
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                let _ = el.set_pointer_capture(ev.pointer_id());
            }
        }
    };

    let on_pointermove = move |ev: web_sys::PointerEvent| {
        if !drag_active.get_untracked() { return; }
        let Some(canvas_el) = overlay_ref.get_untracked() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let cw = rect.width();
        let full_duration = file_duration();
        if full_duration <= 0.0 || cw <= 0.0 { return; }
        let dx = ev.client_x() as f64 - drag_start_x.get_untracked();
        // Map per-pixel motion against the span the overview actually displays.
        let dt = (dx / cw) * overview_span();
        let visible_time = {
            let files = state.library.files().get_untracked();
            let idx = state.library.current_index().get_untracked();
            idx.and_then(|i| files.get(i)).map(|f| {
                let zoom = state.view.zoom_level().get_untracked();
                let canvas_w = state.viewmode.spectrogram_canvas_width().get_untracked();
                (canvas_w / zoom) * f.spectrogram.time_resolution
            }).unwrap_or(0.0)
        };
        state.suspend_follow();
        scrub_apply_scroll(&state, drag_start_scroll.get_untracked() + dt, visible_time, full_duration);
    };

    let on_pointerup = move |_: web_sys::PointerEvent| {
        drag_active.set(false);
    };

    // ── Touch event handlers (mobile) ──────────────────────────────────────────
    let on_touchstart = move |ev: web_sys::TouchEvent| {
        if state.status.viewport_zoomed().get_untracked() { return; }
        let touches = ev.touches();
        if touches.length() != 1 { return; }
        ev.prevent_default();
        let touch = touches.get(0).unwrap();
        let Some(canvas_el) = overlay_ref.get_untracked() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let canvas_x = touch.client_x() as f64 - rect.left();
        let cw = rect.width();
        if let Some(t) = x_to_time(canvas_x, cw) {
            push_nav(&state);
            let visible = half_visible_time() * 2.0;
            state.suspend_follow();
            scrub_apply_scroll(&state, t - half_visible_time(), visible, file_duration());
        }
        drag_active.set(true);
        drag_start_x.set(touch.client_x() as f64);
        drag_start_scroll.set(state.view.scroll_offset().get_untracked());
    };

    let on_touchmove = move |ev: web_sys::TouchEvent| {
        if !drag_active.get_untracked() { return; }
        let touches = ev.touches();
        if touches.length() != 1 { return; }
        ev.prevent_default();
        let touch = touches.get(0).unwrap();
        let Some(canvas_el) = overlay_ref.get_untracked() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let cw = rect.width();
        let full_duration = file_duration();
        if full_duration <= 0.0 || cw <= 0.0 { return; }
        let dx = touch.client_x() as f64 - drag_start_x.get_untracked();
        // Map per-pixel motion against the span the overview actually displays.
        let dt = (dx / cw) * overview_span();
        let visible_time = {
            let files = state.library.files().get_untracked();
            let idx = state.library.current_index().get_untracked();
            idx.and_then(|i| files.get(i)).map(|f| {
                let zoom = state.view.zoom_level().get_untracked();
                let canvas_w = state.viewmode.spectrogram_canvas_width().get_untracked();
                (canvas_w / zoom) * f.spectrogram.time_resolution
            }).unwrap_or(0.0)
        };
        state.suspend_follow();
        scrub_apply_scroll(&state, drag_start_scroll.get_untracked() + dt, visible_time, full_duration);
    };

    let on_touchend = move |_ev: web_sys::TouchEvent| {
        drag_active.set(false);
    };

    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        let raw_delta = ev.delta_y() + ev.delta_x();
        let total_duration = file_duration();
        let visible_time = {
            let files = state.library.files().get_untracked();
            let idx = state.library.current_index().get_untracked();
            idx.and_then(|i| files.get(i)).map(|f| {
                let zoom = state.view.zoom_level().get_untracked();
                let canvas_w = state.viewmode.spectrogram_canvas_width().get_untracked();
                (canvas_w / zoom) * f.spectrogram.time_resolution
            }).unwrap_or(0.0)
        };
        let delta = raw_delta.signum() * visible_time * 0.1 * (raw_delta.abs() / 100.0).min(3.0);
        state.suspend_follow();
        let cur = state.view.scroll_offset().get_untracked();
        scrub_apply_scroll(&state, cur + delta, visible_time, total_duration);
    };

    view! {
        <div class="overview-strip">
            // Background canvas — static waveform/spectrogram (no scroll redraw)
            <canvas
                node_ref=canvas_ref
                style="cursor: crosshair; touch-action: none; pointer-events: none;"
            />

            // Overlay canvas — viewport rect, bookmarks, time markers (cheap per-frame)
            <canvas
                node_ref=overlay_ref
                on:pointerdown=on_pointerdown
                on:pointermove=on_pointermove
                on:pointerup=on_pointerup
                on:wheel=on_wheel
                on:touchstart=on_touchstart
                on:touchmove=on_touchmove
                on:touchend=on_touchend
                style=move || {
                    let ta = if state.status.viewport_zoomed().get() { "pinch-zoom" } else { "none" };
                    let pe = if state.status.viewport_zoomed().get() { "none" } else { "auto" };
                    format!("position: absolute; top: 0; left: 0; cursor: crosshair; touch-action: {ta}; pointer-events: {pe};")
                }
            />

            // DOM playhead dot overlay — decoupled from heavy canvas redraws
            <div
                class="playhead-dot"
                style:left=move || {
                    let playhead = state.playback.playhead_time().get();
                    let duration = if let Some(ref tl) = state.timeline.active().get_untracked() {
                        tl.total_duration_secs
                    } else {
                        let is_live = state.mic.recording().get_untracked()
                            || state.mic.listening().get_untracked();
                        if is_live && crate::canvas::live_waterfall::is_active() {
                            crate::canvas::live_waterfall::total_time()
                        } else {
                            let files = state.library.files().get_untracked();
                            let idx = state.library.current_index().get_untracked();
                            idx.and_then(|i| files.get(i))
                                .map(|f| f.audio.duration_secs)
                                .unwrap_or(0.0)
                        }
                    };
                    let pct = if duration > 0.0 {
                        (playhead / duration * 100.0).clamp(0.0, 100.0)
                    } else { 0.0 };
                    format!("{:.2}%", pct)
                }
                style:display=move || if state.playback.is_playing().get() && !state.viewmode.clean_view().get() { "block" } else { "none" }
            />

            // Layers button (bottom-left, after nav buttons)
            <Show when=move || !state.viewmode.clean_view().get()>
                <OverviewToggleButton />
            </Show>
        </div>
    }
}

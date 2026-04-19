// Gutter components — dedicated drag surfaces for range selection that
// live alongside (not on top of) the spectrogram / waveform axes.
//
// `BandGutter` is a narrow vertical canvas showing the frequency-band
// selection. The time gutter is drawn as an overlay strip on the
// waveform canvas itself (see waveform.rs) rather than as its own
// component — the user asked for it to be "part of the main canvas".

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};
use crate::canvas::gutter_renderer;
use crate::components::spectrogram_events::{
    apply_axis_drag, finalize_axis_drag, freq_snap, select_all_frequencies,
};
use crate::state::AppState;

/// Vertical band-selection gutter. Interactions mirror the spectrogram's
/// left y-axis so the two feel like one control surface: single tap
/// toggles HFR off, drag paints a new band (auto-enabling HFR), shift+
/// drag extends the existing band from its far edge, and double-click
/// selects the full Nyquist range. All three gestures route through the
/// shared `apply_axis_drag` / `finalize_axis_drag` helpers so snapping,
/// focus, and selection-upgrade behaviour stay in lockstep with the axis.
#[component]
pub fn BandGutter() -> impl IntoView {
    let state = expect_context::<AppState>();
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    // Start-of-drag anchor in Hz; None when not dragging.
    let drag_anchor: StoredValue<Option<f64>> = StoredValue::new(None);
    // Tooltip position (canvas-local y, in px) — drives the drag tooltip.
    // None while not dragging.
    let tooltip_y = RwSignal::new_local(Option::<f64>::None);

    // Resolve the visible frequency window for the gutter. On the
    // spectrogram this tracks min/max_display_freq so the gutter ticks
    // line up 1:1 with the spectrogram's y-axis; on views that don't set
    // those signals it falls back to 0..Nyquist.
    let display_range = move || -> (f64, f64) {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        let nyquist = idx
            .and_then(|i| files.get(i))
            .map(|f| f.audio.sample_rate as f64 / 2.0)
            .unwrap_or(0.0);
        let lo = state.min_display_freq.get().unwrap_or(0.0);
        let hi = state.max_display_freq.get().unwrap_or(nyquist);
        (lo, hi)
    };

    // Redraw when any relevant signal changes.
    Effect::new(move |_| {
        let band_lo = state.band_ff_freq_lo.get();
        let band_hi = state.band_ff_freq_hi.get();
        let hfr_on = state.hfr_enabled.get();
        let shield_style = state.shield_style.get();
        // Live drag range from either this gutter or the spectrogram's
        // y-axis — when Some, overrides the stored band so the shield
        // lights up mid-drag even before the band has been committed.
        let drag_range = match (
            state.axis_drag_start_freq.get(),
            state.axis_drag_current_freq.get(),
        ) {
            (Some(s), Some(c)) => Some((s, c)),
            _ => None,
        };
        let (min_freq, max_freq) = display_range();
        let _sidebar = state.sidebar_collapsed.get();
        let _sidebar_width = state.sidebar_width.get();
        let _rsidebar = state.right_sidebar_collapsed.get();
        let _rsidebar_width = state.right_sidebar_width.get();
        let _tile_ready = state.tile_ready_signal.get();

        let Some(canvas_el) = canvas_ref.get() else { return };
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let display_w = rect.width() as u32;
        let display_h = rect.height() as u32;
        if display_w == 0 || display_h == 0 { return; }
        if canvas.width() != display_w || canvas.height() != display_h {
            canvas.set_width(display_w);
            canvas.set_height(display_h);
        }

        let Ok(Some(obj)) = canvas.get_context("2d") else { return };
        let Ok(ctx) = obj.dyn_into::<CanvasRenderingContext2d>() else { return };

        gutter_renderer::draw_band_gutter(
            &ctx,
            display_w as f64,
            display_h as f64,
            min_freq,
            max_freq,
            band_lo,
            band_hi,
            hfr_on,
            shield_style,
            drag_range,
        );
    });

    // Resolve (local_y, canvas_height, min_freq, max_freq) for a pointer
    // event — frequency bounds reflect the host view's current display
    // range so drag math uses the same mapping the gutter renders with.
    let pointer_context = move |ev: &web_sys::PointerEvent| -> Option<(f64, f64, f64, f64)> {
        let canvas_el = canvas_ref.get()?;
        let canvas: &HtmlCanvasElement = canvas_el.as_ref();
        let rect = canvas.get_bounding_client_rect();
        let h = rect.height();
        if h <= 0.0 { return None; }
        let y = ev.client_y() as f64 - rect.top();
        let (min_freq, max_freq) = display_range();
        if max_freq <= min_freq { return None; }
        Some((y, h, min_freq, max_freq))
    };

    let on_pointerdown = move |ev: web_sys::PointerEvent| {
        if ev.button() != 0 { return; }
        let Some((y, h, min_freq, max_freq)) = pointer_context(&ev) else { return };
        ev.prevent_default();

        let freq = gutter_renderer::y_to_freq(y, min_freq, max_freq, h);
        let shift = ev.shift_key();
        let band_lo = state.band_ff_freq_lo.get_untracked();
        let band_hi = state.band_ff_freq_hi.get_untracked();
        let has_range = band_hi > band_lo;

        // Shift+click extend: anchor at the edge of the existing range
        // farthest from the click, so dragging grows the band from there.
        let raw_start = if shift && has_range {
            if (freq - band_lo).abs() < (freq - band_hi).abs() { band_hi } else { band_lo }
        } else {
            freq
        };

        drag_anchor.set_value(Some(raw_start));
        tooltip_y.set(Some(y));
        // Flag the drag so heavy consumers (waveform band-split) can cache.
        state.band_ff_dragging.set(true);

        // Seed the shared axis-drag state so the spectrogram's y-axis
        // shields light up in sync, and so finalize_axis_drag can detect
        // a tap (start ≈ current).
        let snap_s = freq_snap(raw_start, shift);
        let snap_e = freq_snap(freq, shift);
        state.axis_drag_start_freq.set(Some((raw_start / snap_s).round() * snap_s));
        state.axis_drag_current_freq.set(Some((freq / snap_e).round() * snap_e));
        state.is_dragging.set(true);

        // Shift-extend should update the band immediately; a fresh drag
        // waits for pointermove so a pure tap leaves the existing band
        // intact (tap = toggle HFR, handled in finalize_axis_drag).
        if shift && has_range {
            let lo = raw_start.min(freq);
            let hi = raw_start.max(freq);
            if hi - lo > 500.0 {
                state.set_band_ff_range(lo, hi);
            }
        }

        if let Some(target) = ev.target() {
            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                let _ = el.set_pointer_capture(ev.pointer_id());
            }
        }
    };

    let on_pointermove = move |ev: web_sys::PointerEvent| {
        let Some(raw_start) = drag_anchor.get_value() else { return };
        let Some((y, h, min_freq, max_freq)) = pointer_context(&ev) else { return };
        tooltip_y.set(Some(y.clamp(0.0, h)));
        let freq = gutter_renderer::y_to_freq(y, min_freq, max_freq, h);
        apply_axis_drag(state, raw_start, freq, ev.shift_key());
    };

    let on_pointerup = move |_ev: web_sys::PointerEvent| {
        if drag_anchor.get_value().is_some() {
            drag_anchor.set_value(None);
            tooltip_y.set(None);
            state.band_ff_dragging.set(false);
            // Shared finalize: taps toggle HFR off, meaningful drags
            // auto-enable HFR and promote focus to FrequencyFocus.
            finalize_axis_drag(state);
        }
    };

    let on_dblclick = move |_ev: web_sys::MouseEvent| {
        select_all_frequencies(state);
    };

    // Format "40.0 – 72.5 kHz" for the drag tooltip.
    let format_range = move || {
        let lo = state.band_ff_freq_lo.get();
        let hi = state.band_ff_freq_hi.get();
        if hi <= lo { return String::new(); }
        format!("{:.1} – {:.1} kHz", lo / 1000.0, hi / 1000.0)
    };

    view! {
        <div class="band-gutter">
            <canvas
                node_ref=canvas_ref
                on:pointerdown=on_pointerdown
                on:pointermove=on_pointermove
                on:pointerup=on_pointerup
                on:dblclick=on_dblclick
            />
            // Drag tooltip: floats next to the pointer while dragging, shows the
            // current lo–hi range. Hidden when not dragging.
            <div
                class="band-gutter-tooltip"
                style:top=move || tooltip_y.get().map(|y| format!("{:.0}px", y)).unwrap_or_default()
                style:display=move || if tooltip_y.get().is_some() && !format_range().is_empty() { "block" } else { "none" }
            >
                {format_range}
            </div>
        </div>
    }
}

// Output Range combo — sits at the end of the band-affected group in the
// Hearing bar, after the mode picker. Shows + edits how the active
// playback mode maps the input frequency focus into the 0–2000 Hz
// target listening range.
//
//   [Output ÷8 ▾]
//
// The popup is a two-column layout: mode-aware preset/slider controls
// on the left, and a vertical 0–2000 Hz "output gutter" on the right
// that visualises the current output band and is itself draggable.
//
// Two playback styles are handled in Phase 1:
//   • Divide (TE / PS / PV / ZC) — output_freq = input_freq / factor
//   • Linear shift (Heterodyne)  — output_freq = |input_freq − carrier|
//
// Each style edits the same underlying signals the mode's own popup
// edits; this component just exposes them through an output-side
// perspective. The Snap toggle constrains dragging to canonical stops
// (powers of 2 / 10 for factors; multiples of 5 kHz for carriers).

use leptos::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::combo_button::ComboButton;
use crate::components::mode_button::{format_factor_value, format_freq_khz, output_freq};
use crate::state::{AppState, LayerPanel, OutputSnap, PlaybackMode};

/// Top of the gutter scale (Hz). Bottom is always 0.
const GUTTER_MAX_HZ: f64 = 2000.0;

/// Canonical divide factors for the Standard snap mode.
const STANDARD_FACTORS: [f64; 6] = [2.0, 4.0, 8.0, 10.0, 16.0, 32.0];
/// Equal-chroma snap: powers of 2 only (preserves pitch class).
const CHROMA_FACTORS: [f64; 5] = [2.0, 4.0, 8.0, 16.0, 32.0];

/// Carrier snap step (Hz) for heterodyne mode when Snap is on.
const HET_SNAP_STEP_HZ: f64 = 5_000.0;

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

/// Logical style picked by the current playback mode.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Style {
    /// No mapping — output is the same as input (Normal mode).
    Passthrough,
    /// output = input / factor (TE, PS, PV, ZC).
    Divide,
    /// output = |input − carrier| (Heterodyne).
    LinearShift,
}

fn style_for(mode: PlaybackMode) -> Style {
    match mode {
        PlaybackMode::Normal => Style::Passthrough,
        PlaybackMode::Heterodyne => Style::LinearShift,
        PlaybackMode::TimeExpansion
        | PlaybackMode::PitchShift
        | PlaybackMode::PhaseVocoder
        | PlaybackMode::ZeroCrossing => Style::Divide,
    }
}

/// Factor signal for the current playback mode (None for Normal/Het).
fn factor_signal(state: &AppState, mode: PlaybackMode) -> Option<RwSignal<f64>> {
    match mode {
        PlaybackMode::TimeExpansion => Some(state.te_factor),
        PlaybackMode::PitchShift => Some(state.ps_factor),
        PlaybackMode::PhaseVocoder => Some(state.pv_factor),
        PlaybackMode::ZeroCrossing => Some(state.zc_factor),
        _ => None,
    }
}

/// Auto-derive-from-BandFF signal for the current mode (None where N/A).
fn auto_signal(state: &AppState, mode: PlaybackMode) -> Option<RwSignal<bool>> {
    match mode {
        PlaybackMode::TimeExpansion => Some(state.te_factor_auto),
        PlaybackMode::PitchShift => Some(state.ps_factor_auto),
        PlaybackMode::PhaseVocoder => Some(state.pv_factor_auto),
        PlaybackMode::Heterodyne => Some(state.het_freq_auto),
        _ => None,
    }
}

fn band(state: &AppState) -> (f64, f64) {
    (state.band_ff_freq_lo.get(), state.band_ff_freq_hi.get())
}

/// Whether this mode supports a compound scale + shift mapping.
/// Only the PS bucket (PitchShift / PhaseVocoder) does today — the
/// `ps_shift_hz` signal is honoured by `apply_dsp_mode`'s PS/PV
/// branches via a heterodyne pre-shift stage.
fn supports_shift(mode: PlaybackMode) -> bool {
    matches!(mode, PlaybackMode::PitchShift | PlaybackMode::PhaseVocoder)
}

/// Compute the current effective output range derived from the active
/// mode + its parameters + the BandFF input range. Returns None when
/// there's no meaningful range to draw (e.g. BandFF is empty).
fn current_output_range(state: &AppState, mode: PlaybackMode) -> Option<(f64, f64)> {
    let (in_lo, in_hi) = band(state);
    if in_hi <= in_lo { return None; }
    let style = style_for(mode);
    match style {
        Style::Passthrough => Some((in_lo, in_hi)),
        Style::Divide => {
            let f = factor_signal(state, mode)?.get();
            // Optional pre-shift for PS/PV: apply additive shift first,
            // then divide. `|in - shift|` folds at zero so the abs() is
            // important when shift > in_lo.
            let shift = if supports_shift(mode) { state.ps_shift_hz.get() } else { 0.0 };
            let post_a = (in_lo - shift).abs();
            let post_b = (in_hi - shift).abs();
            let a = output_freq(post_a, f);
            let b = output_freq(post_b, f);
            Some(if a < b { (a, b) } else { (b, a) })
        }
        Style::LinearShift => {
            let c = state.het_frequency.get();
            let a = (in_lo - c).abs();
            let b = (in_hi - c).abs();
            Some(if a < b { (a, b) } else { (b, a) })
        }
    }
}

fn divide_shorthand(f: f64) -> String {
    let s = format_factor_value(f);
    if s.starts_with('\u{00f7}') { s } else { format!("\u{00f7}{s}") }
}

/// Short mapping label for the toolbar button left side.
fn mapping_shorthand(state: &AppState, mode: PlaybackMode) -> String {
    match style_for(mode) {
        Style::Passthrough => "1:1".into(),
        Style::Divide => {
            let Some(sig) = factor_signal(state, mode) else { return "\u{2014}".into(); };
            let div = divide_shorthand(sig.get());
            if supports_shift(mode) {
                let shift = state.ps_shift_hz.get();
                if shift.abs() >= 100.0 {
                    return format!("{div} \u{2212}{}", format_freq_khz(shift));
                }
            }
            div
        }
        Style::LinearShift => {
            let c = state.het_frequency.get();
            // Negative because heterodyne shifts the band *down*.
            format!("\u{2212}{}", format_freq_khz(c))
        }
    }
}

/// Snap a free-form factor according to the current snap policy.
fn snap_factor(raw: f64, snap: OutputSnap) -> f64 {
    let abs = raw.abs().max(1.5);
    let snapped_abs = match snap {
        OutputSnap::Free => ((abs * 10.0).round() / 10.0).clamp(1.5, 64.0),
        OutputSnap::Standard => STANDARD_FACTORS
            .iter()
            .copied()
            .min_by(|a, b| (a - abs).abs().partial_cmp(&(b - abs).abs()).unwrap())
            .unwrap_or(8.0),
        OutputSnap::EqualChroma => CHROMA_FACTORS
            .iter()
            .copied()
            .min_by(|a, b| (a - abs).abs().partial_cmp(&(b - abs).abs()).unwrap())
            .unwrap_or(8.0),
    };
    if raw < 0.0 { -snapped_abs } else { snapped_abs }
}

fn snap_carrier(raw_hz: f64, snap: OutputSnap) -> f64 {
    let clamped = raw_hz.max(0.0);
    match snap {
        OutputSnap::Free => (clamped / 100.0).round() * 100.0,
        // Both Standard and EqualChroma snap carriers to 5 kHz — there's
        // no meaningful "musical interval" quantisation for an additive
        // shift, so EqualChroma falls back to the same step.
        OutputSnap::Standard | OutputSnap::EqualChroma => {
            (clamped / HET_SNAP_STEP_HZ).round() * HET_SNAP_STEP_HZ
        }
    }
}

#[component]
pub fn OutputRangeCombo() -> impl IntoView {
    let state = expect_context::<AppState>();

    let is_open = Signal::derive(move || state.layer_panel_open.get() == Some(LayerPanel::OutputRange));
    let no_file = move || {
        state.current_file_index.get().is_none() && state.active_timeline.get().is_none()
    };

    let mode = Signal::derive(move || state.playback_mode.get());

    let left_class = Signal::derive(move || {
        if no_file() { "layer-btn combo-btn-left disabled" }
        else { "layer-btn combo-btn-left no-annotation active" }
    });
    let right_class = Signal::derive(move || {
        if no_file() { return "layer-btn combo-btn-right disabled"; }
        if is_open.get() { "layer-btn combo-btn-right open" } else { "layer-btn combo-btn-right" }
    });

    let left_value = Signal::derive(move || mapping_shorthand(&state, mode.get()));

    let right_value = Signal::derive(move || {
        match current_output_range(&state, mode.get()) {
            Some((lo, hi)) => format!("{}\u{2013}{}", format_freq_khz(lo), format_freq_khz(hi)),
            None => "\u{2014}".into(),
        }
    });

    // Left click: identical to right click for now — opens the popup. The
    // mode-specific actions live inside.
    let left_click = Callback::new(move |_: web_sys::MouseEvent| {
        if no_file() { return; }
        toggle_panel(&state, LayerPanel::OutputRange);
    });
    let toggle_menu = Callback::new(move |()| {
        toggle_panel(&state, LayerPanel::OutputRange);
    });

    view! {
        <ComboButton
            left_label="OUT"
            left_value=left_value
            left_click=left_click
            left_class=left_class
            right_value=right_value
            right_class=right_class
            is_open=is_open
            toggle_menu=toggle_menu
            left_title="Output range — how input frequencies are mapped to audible output"
            right_title="Output range settings"
            panel_style="min-width: 320px;"
        >
            <OutputRangePopup/>
        </ComboButton>
    }
}

#[component]
fn OutputRangePopup() -> impl IntoView {
    let state = expect_context::<AppState>();

    view! {
        <div class="output-range-popup">
            <div class="output-range-controls">
                <div class="layer-panel-title">"Output range"</div>
                {move || {
                    let mode = state.playback_mode.get();
                    match style_for(mode) {
                        Style::Passthrough => view! {
                            <div class="output-range-empty">
                                "1:1 \u{2014} input plays back unchanged."
                                <br/>
                                "Pick HET, TE, PS or ZC to remap above-audible content."
                            </div>
                        }.into_any(),
                        Style::Divide => view! { <DivideControls mode/> }.into_any(),
                        Style::LinearShift => view! { <LinearShiftControls/> }.into_any(),
                    }
                }}
                <SnapPicker/>
                <BandSummary/>
            </div>
            <OutputGutter/>
        </div>
    }
}

#[component]
fn DivideControls(#[prop(into)] mode: Signal<PlaybackMode>) -> impl IntoView {
    let state = expect_context::<AppState>();

    let preset_values: [(f64, &str); 6] = [
        (2.0, "\u{00f7}2"),
        (4.0, "\u{00f7}4"),
        (8.0, "\u{00f7}8"),
        (10.0, "\u{00f7}10"),
        (16.0, "\u{00f7}16"),
        (32.0, "\u{00f7}32"),
    ];

    let click_preset = move |value: f64| {
        let m = mode.get_untracked();
        if let Some(f_sig) = factor_signal(&state, m) {
            if let Some(a_sig) = auto_signal(&state, m) { a_sig.set(false); }
            f_sig.set(value);
        }
    };

    view! {
        <div class="output-range-section-label">"Divide factor"</div>
        <div class="output-range-presets">
            {move || {
                let m = mode.get();
                let a_sig = auto_signal(&state, m);
                let on_auto = a_sig.map(|s| s.get()).unwrap_or(false);
                view! {
                    <button
                        class=if on_auto { "factor-preset auto on" } else { "factor-preset auto" }
                        on:click=move |_| {
                            if let Some(s) = a_sig { s.update(|v| *v = !*v); }
                        }
                        title="Auto-pick factor from current band"
                    >"Auto"</button>
                }
            }}
            {preset_values.iter().map(|&(val, label)| {
                let sel = Signal::derive(move || {
                    let m = mode.get();
                    let f = factor_signal(&state, m).map(|s| s.get()).unwrap_or(0.0);
                    let auto = auto_signal(&state, m).map(|s| s.get()).unwrap_or(false);
                    (f.abs() - val).abs() < 0.01 && !auto
                });
                let click = move |_: web_sys::MouseEvent| click_preset(val);
                view! {
                    <button
                        class=move || if sel.get() { "factor-preset sel" } else { "factor-preset" }
                        on:click=click
                    >{label}</button>
                }
            }).collect::<Vec<_>>()}
        </div>
        // ── Optional pre-shift (PS / PV only) ──
        // Lets the user offset the input band before the divide stage,
        // turning the pitch shift into a true scale + shift mapping
        // (out = |in − shift| / factor).
        {move || {
            if !supports_shift(mode.get()) { return view! { <span></span> }.into_any(); }
            let on_shift_input = move |ev: web_sys::Event| {
                use wasm_bindgen::JsCast;
                let el: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
                if let Ok(khz) = el.value().parse::<f64>() {
                    state.ps_shift_hz.set((khz * 1000.0).max(0.0));
                }
            };
            view! {
                <div class="output-range-section-label">"Pre-shift"</div>
                <div class="output-range-shift-row">
                    <input type="number"
                        class="output-range-num"
                        min="0" max="500" step="0.5"
                        prop:value=move || format!("{:.1}", state.ps_shift_hz.get() / 1000.0)
                        on:input=on_shift_input
                    />
                    <span class="output-range-num-suffix">"kHz"</span>
                    <button
                        class="auto-toggle"
                        on:click=move |_| state.ps_shift_hz.set(0.0)
                        title="Clear pre-shift (back to pure divide)"
                    >"0"</button>
                </div>
                <div class="output-range-presets">
                    {[0.0, 10.0, 20.0, 40.0, 80.0].iter().map(|&khz| {
                        let click = move |_: web_sys::MouseEvent| {
                            state.ps_shift_hz.set(khz * 1000.0);
                        };
                        let sel = Signal::derive(move || {
                            (state.ps_shift_hz.get() - khz * 1000.0).abs() < 100.0
                        });
                        let label = if khz == 0.0 {
                            "0".to_string()
                        } else {
                            format!("\u{2212}{}k", khz as i32)
                        };
                        view! {
                            <button
                                class=move || if sel.get() { "factor-preset sel" } else { "factor-preset" }
                                on:click=click
                            >{label}</button>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            }.into_any()
        }}
    }
}

#[component]
fn LinearShiftControls() -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_input = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let el: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(khz) = el.value().parse::<f64>() {
            state.het_freq_auto.set(false);
            state.het_frequency.set(khz * 1000.0);
        }
    };

    view! {
        <div class="output-range-section-label">"Carrier (shift)"</div>
        <div class="output-range-shift-row">
            <input type="number"
                class="output-range-num"
                min="0" max="500" step="0.5"
                prop:value=move || format!("{:.1}", state.het_frequency.get() / 1000.0)
                on:input=on_input
            />
            <span class="output-range-num-suffix">"kHz"</span>
            <button
                class=move || if state.het_freq_auto.get() { "auto-toggle on" } else { "auto-toggle" }
                on:click=move |_| state.het_freq_auto.update(|v| *v = !*v)
                title="Auto-derive carrier from current band"
            >"A"</button>
        </div>
        <div class="output-range-presets">
            {[10.0, 20.0, 40.0, 80.0].iter().map(|&khz| {
                let click = move |_: web_sys::MouseEvent| {
                    state.het_freq_auto.set(false);
                    state.het_frequency.set(khz * 1000.0);
                };
                let sel = Signal::derive(move || {
                    !state.het_freq_auto.get()
                        && (state.het_frequency.get() - khz * 1000.0).abs() < 100.0
                });
                view! {
                    <button
                        class=move || if sel.get() { "factor-preset sel" } else { "factor-preset" }
                        on:click=click
                    >{format!("\u{2212}{}", khz as i32)}{"k"}</button>
                }
            }).collect::<Vec<_>>()}
        </div>
    }
}

#[component]
fn BandSummary() -> impl IntoView {
    let state = expect_context::<AppState>();
    view! {
        <div class="output-range-summary">
            <div>
                <span class="dim">"in  "</span>
                {move || {
                    let (lo, hi) = band(&state);
                    if hi > lo { format!("{}\u{2013}{}", format_freq_khz(lo), format_freq_khz(hi)) }
                    else { "\u{2014}".into() }
                }}
            </div>
            <div>
                <span class="dim">"out "</span>
                {move || {
                    let mode = state.playback_mode.get();
                    match current_output_range(&state, mode) {
                        Some((lo, hi)) => format!("{}\u{2013}{}", format_freq_khz(lo), format_freq_khz(hi)),
                        None => "\u{2014}".into(),
                    }
                }}
            </div>
        </div>
    }
}

/// Which input frequency the drag pins to the cursor's output Hz.
/// Lo / Hi anchor a single endpoint; Center moves the whole band.
#[derive(Copy, Clone)]
enum DragAnchor { Lo, Hi, Center }

#[component]
fn SnapPicker() -> impl IntoView {
    let state = expect_context::<AppState>();
    let opts: [(OutputSnap, &str, &str); 3] = [
        (OutputSnap::Free, "Free", "No snap \u{2014} continuous factor / carrier"),
        (OutputSnap::Standard, "Std", "Snap to standard factors (\u{00f7}2 \u{00f7}4 \u{00f7}8 \u{00f7}10 \u{00f7}16 \u{00f7}32) or 5 kHz carrier steps"),
        (OutputSnap::EqualChroma, "Chroma", "Snap to powers of 2 only \u{2014} preserves pitch intervals"),
    ];
    view! {
        <div class="output-range-snap-row">
            <span class="output-range-snap-label">"Snap"</span>
            <div class="output-range-snap-group">
                {opts.iter().map(|&(val, label, title)| {
                    let sel = Signal::derive(move || state.output_snap.get() == val);
                    let click = move |_: web_sys::MouseEvent| state.output_snap.set(val);
                    view! {
                        <button
                            class=move || if sel.get() { "snap-btn sel" } else { "snap-btn" }
                            on:click=click
                            title=title
                        >{label}</button>
                    }
                }).collect::<Vec<_>>()}
            </div>
        </div>
    }
}

#[component]
fn OutputGutter() -> impl IntoView {
    let state = expect_context::<AppState>();
    let gutter_ref = NodeRef::<leptos::html::Div>::new();

    let highlight_style = move || {
        let mode = state.playback_mode.get();
        let Some((lo, hi)) = current_output_range(&state, mode) else {
            return "display: none;".to_string();
        };
        let lo_pct = (lo.clamp(0.0, GUTTER_MAX_HZ) / GUTTER_MAX_HZ * 100.0).min(100.0);
        let hi_pct = (hi.clamp(0.0, GUTTER_MAX_HZ) / GUTTER_MAX_HZ * 100.0).min(100.0);
        format!(
            "bottom: {lo_pct:.2}%; height: {:.2}%;",
            (hi_pct - lo_pct).max(0.5)
        )
    };

    let band_label = move || mapping_shorthand(&state, state.playback_mode.get());

    let pin_input_to = move |target_hz: f64, anchor: f64| {
        let mode = state.playback_mode.get_untracked();
        let snap = state.output_snap.get_untracked();
        let target = target_hz.clamp(1.0, GUTTER_MAX_HZ);
        match style_for(mode) {
            Style::Passthrough => {}
            Style::Divide => {
                if let Some(f_sig) = factor_signal(&state, mode) {
                    if let Some(a_sig) = auto_signal(&state, mode) { a_sig.set(false); }
                    // Honour any active pre-shift so the back-solve
                    // matches the compound mapping the user sees.
                    let shift = if supports_shift(mode) {
                        state.ps_shift_hz.get_untracked()
                    } else { 0.0 };
                    let post_shift = (anchor - shift).abs().max(1.0);
                    let raw = post_shift / target;
                    f_sig.set(snap_factor(raw, snap));
                }
            }
            Style::LinearShift => {
                state.het_freq_auto.set(false);
                let raw = anchor - target;
                state.het_frequency.set(snap_carrier(raw, snap));
            }
        }
    };

    let start_drag = move |ev: web_sys::PointerEvent, which: DragAnchor| {
        ev.prevent_default();
        ev.stop_propagation();
        let Some(el) = gutter_ref.get_untracked() else { return; };
        let rect = el.get_bounding_client_rect();
        let height = rect.height().max(1.0);
        let top = rect.top();

        let (in_lo, in_hi) = band(&state);
        if in_hi <= in_lo { return; }
        let anchor = match which {
            DragAnchor::Lo => in_lo,
            DragAnchor::Hi => in_hi,
            DragAnchor::Center => (in_lo + in_hi) / 2.0,
        };

        let hz_from_client_y = move |client_y: f64| -> f64 {
            let rel = ((client_y - top) / height).clamp(0.0, 1.0);
            (1.0 - rel) * GUTTER_MAX_HZ
        };

        pin_input_to(hz_from_client_y(ev.client_y() as f64), anchor);

        let win = web_sys::window().unwrap();
        let move_slot: Rc<RefCell<Option<Closure<dyn FnMut(web_sys::PointerEvent)>>>> =
            Rc::new(RefCell::new(None));
        let up_slot: Rc<RefCell<Option<Closure<dyn FnMut(web_sys::PointerEvent)>>>> =
            Rc::new(RefCell::new(None));

        let move_cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
            pin_input_to(hz_from_client_y(e.client_y() as f64), anchor);
        });
        let win_clone = win.clone();
        let move_slot_clone = Rc::clone(&move_slot);
        let up_slot_clone = Rc::clone(&up_slot);
        let up_cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |_: web_sys::PointerEvent| {
            if let Some(m) = move_slot_clone.borrow_mut().take() {
                let _ = win_clone.remove_event_listener_with_callback(
                    "pointermove", m.as_ref().unchecked_ref(),
                );
            }
            if let Some(u) = up_slot_clone.borrow_mut().take() {
                let _ = win_clone.remove_event_listener_with_callback(
                    "pointerup", u.as_ref().unchecked_ref(),
                );
            }
        });
        let _ = win.add_event_listener_with_callback(
            "pointermove", move_cb.as_ref().unchecked_ref(),
        );
        let _ = win.add_event_listener_with_callback(
            "pointerup", up_cb.as_ref().unchecked_ref(),
        );
        *move_slot.borrow_mut() = Some(move_cb);
        *up_slot.borrow_mut() = Some(up_cb);
    };

    let on_gutter_down = move |ev: web_sys::PointerEvent| start_drag(ev, DragAnchor::Center);
    let on_lo_down = move |ev: web_sys::PointerEvent| start_drag(ev, DragAnchor::Lo);
    let on_hi_down = move |ev: web_sys::PointerEvent| start_drag(ev, DragAnchor::Hi);
    let on_band_down = move |ev: web_sys::PointerEvent| start_drag(ev, DragAnchor::Center);

    view! {
        <div class="output-gutter-wrap">
            <div class="output-gutter"
                node_ref=gutter_ref
                on:pointerdown=on_gutter_down
            >
                <div class="output-gutter-ticks">
                    {[0i32, 500, 1000, 1500, 2000].iter().map(|&hz| {
                        let pct = hz as f64 / GUTTER_MAX_HZ * 100.0;
                        let label = if hz >= 1000 {
                            format!("{}k", hz / 1000)
                        } else {
                            hz.to_string()
                        };
                        view! {
                            <div class="output-gutter-tick" style=format!("bottom: {pct:.2}%;")>
                                <span class="output-gutter-tick-label">{label}</span>
                            </div>
                        }
                    }).collect::<Vec<_>>()}
                </div>
                <div class="output-gutter-band"
                    style=highlight_style
                    on:pointerdown=on_band_down
                >
                    <span class="output-gutter-band-label">{band_label}</span>
                    <div class="output-gutter-handle output-gutter-handle-hi"
                        on:pointerdown=on_hi_down
                        title="Drag to anchor the high end of the input band to this output Hz"
                    ></div>
                    <div class="output-gutter-handle output-gutter-handle-lo"
                        on:pointerdown=on_lo_down
                        title="Drag to anchor the low end of the input band to this output Hz"
                    ></div>
                </div>
            </div>
        </div>
    }
}

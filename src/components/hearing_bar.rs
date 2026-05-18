// Hearing Bar — the "what comes out the speakers" strip between the
// Overview and the main canvas.
//
// Layout:  [HFR | Band] [Mode | HET] │ [Bandpass] [Gain] [NR] [Notch] │ [Listen | …]
//
//          ^HfrButton    ^ModeButton    ^filter combos                  ^ListenButton
//
// The HFR cell wraps `HfrButton` in a class that drives per-letter
// brightness on the "HFR" label (H dims when the active band sits entirely
// below 24 kHz — i.e. an audible-only filter). HfrButton's right half is
// the band-presets dropdown; ModeButton is a separate combo so each half
// of HfrButton stays single-purpose. Listen lives at the right end
// because its DSP pipeline is unified with HFR/Mode. Filter combos in
// the middle wrap onto a second row on narrow viewports.

use leptos::prelude::*;

use crate::audio::streaming_playback::PV_MODE_BOOST_DB;
use crate::components::combo_button::ComboButton;
use crate::components::hfr_button::HfrButton;
use crate::components::listen_button::ListenButton;
use crate::components::mode_button::ModeButton;
use crate::components::noise_combos::{NotchCombo, NrCombo};
use crate::state::{
    AppState, Bar, BandpassMode, BandpassRange, FilterQuality, GainMode, LayerPanel,
    PeakSource, PlaybackMode,
};

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

// Band presets and their helpers moved into `hfr_button.rs` — they're
// the body of HfrButton's right-half dropdown now.

fn layer_opt_class_simple(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

/// Gain combo (migrated from the bottom toolbar). Controls the playback
/// gain mode (Off / Manual / AutoPeak / Adaptive) plus a manual-boost dB
/// slider that doubles as a quick override when in Auto modes.
#[component]
fn GainCombo() -> impl IntoView {
    let state = expect_context::<AppState>();

    let is_open = Signal::derive(move || state.layer_panel_open.get() == Some(LayerPanel::Gain));
    let no_file = move || {
        state.current_file_index.get().is_none() && state.active_timeline.get().is_none()
    };

    let left_class = Signal::derive(move || {
        if no_file() {
            "layer-btn combo-btn-left disabled"
        } else if state.gain_mode.get() != GainMode::Off {
            "layer-btn combo-btn-left active"
        } else {
            "layer-btn combo-btn-left no-annotation"
        }
    });
    let right_class = Signal::derive(move || {
        if no_file() { return "layer-btn combo-btn-right disabled"; }
        let dim = if state.gain_mode.get() == GainMode::Off { " dim" } else { "" };
        if is_open.get() {
            if dim.is_empty() { "layer-btn combo-btn-right open" } else { "layer-btn combo-btn-right dim open" }
        } else if dim.is_empty() { "layer-btn combo-btn-right" } else { "layer-btn combo-btn-right dim" }
    });

    let left_value = Signal::derive(move || {
        let mode = state.gain_mode.get();
        let manual_db = state.gain_db.get();
        let pv_boost = if state.playback_mode.get() == PlaybackMode::PhaseVocoder { PV_MODE_BOOST_DB } else { 0.0 };
        match mode {
            GainMode::Off => {
                if pv_boost > 0.0 { format!("+{:.0}dB", pv_boost) }
                else { String::new() }
            }
            GainMode::Manual => {
                let total = manual_db + pv_boost;
                if total > 0.0 { format!("+{:.0}dB", total) }
                else { format!("{:.0}dB", total) }
            }
            GainMode::AutoPeak => {
                let auto_db = state.compute_auto_gain();
                let total = auto_db + manual_db + pv_boost;
                format!("+{:.0}dB", total)
            }
            GainMode::Adaptive => {
                if manual_db > 0.0 || pv_boost > 0.0 {
                    format!("A+{:.0}", manual_db + pv_boost)
                } else {
                    "Auto".to_string()
                }
            }
        }
    });
    let right_value = Signal::derive(move || {
        match state.gain_mode.get() {
            GainMode::Off => "OFF".to_string(),
            mode => mode.label().to_string(),
        }
    });

    let left_click = Callback::new(move |_: web_sys::MouseEvent| {
        let mode = state.gain_mode.get_untracked();
        if mode == GainMode::Off {
            let last = state.gain_mode_last_auto.get_untracked();
            state.gain_mode.set(last);
            state.auto_gain.set(last.is_auto());
        } else {
            if mode.is_auto() {
                state.gain_mode_last_auto.set(mode);
            }
            state.gain_mode.set(GainMode::Off);
            state.auto_gain.set(false);
        }
    });
    let toggle_menu = Callback::new(move |()| {
        toggle_panel(&state, LayerPanel::Gain);
    });

    view! {
        <ComboButton
            left_label="Gain"
            left_value=left_value
            left_click=left_click
            left_class=left_class
            right_value=right_value
            right_class=right_class
            is_open=is_open
            toggle_menu=toggle_menu
            left_title="Toggle gain"
            right_title="Gain mode"
            menu_direction="below"
            panel_align="left"
            panel_style="min-width: 210px;"
        >
            <button class=move || layer_opt_class_simple(state.gain_mode.get() == GainMode::Off)
                on:click=move |_| {
                    state.gain_mode.set(GainMode::Off);
                    state.auto_gain.set(false);
                    state.layer_panel_open.set(None);
                }
            >"Off"</button>
            <button class=move || layer_opt_class_simple(state.gain_mode.get() == GainMode::Manual)
                on:click=move |_| {
                    state.gain_mode.set(GainMode::Manual);
                    state.auto_gain.set(false);
                    state.layer_panel_open.set(None);
                }
            >"Manual \u{2014} Slider boost only"</button>
            <button class=move || layer_opt_class_simple(state.gain_mode.get() == GainMode::AutoPeak)
                on:click=move |_| {
                    state.gain_mode.set(GainMode::AutoPeak);
                    state.gain_mode_last_auto.set(GainMode::AutoPeak);
                    state.auto_gain.set(true);
                    state.layer_panel_open.set(None);
                }
            >"Peak \u{2014} Normalize to peak"</button>
            <button class=move || layer_opt_class_simple(state.gain_mode.get() == GainMode::Adaptive)
                on:click=move |_| {
                    state.gain_mode.set(GainMode::Adaptive);
                    state.gain_mode_last_auto.set(GainMode::Adaptive);
                    state.auto_gain.set(true);
                    state.layer_panel_open.set(None);
                }
            >"AGC \u{2014} Automatic gain control"</button>
            <Show when=move || state.gain_mode.get() == GainMode::AutoPeak>
                <div class="peak-source-row">
                    <span class="peak-source-label">"Peak from:"</span>
                    <button class=move || if state.peak_source.get() == PeakSource::First30s { "peak-src-btn sel" } else { "peak-src-btn" }
                        on:click=move |_| state.peak_source.set(PeakSource::First30s)
                        title="Peak from first 30 seconds"
                    >"30s"</button>
                    <button class=move || if state.peak_source.get() == PeakSource::FullWave { "peak-src-btn sel" } else { "peak-src-btn" }
                        on:click=move |_| state.peak_source.set(PeakSource::FullWave)
                        title="Peak from entire file"
                    >"Full"</button>
                    <button class=move || {
                        let base = if state.peak_source.get() == PeakSource::Selection { "peak-src-btn sel" } else { "peak-src-btn" };
                        if state.selection.get().is_none() { format!("{} disabled", base) } else { base.to_string() }
                    }
                        on:click=move |_| {
                            if state.selection.get_untracked().is_some() {
                                state.peak_source.set(PeakSource::Selection);
                            }
                        }
                        title="Peak from current selection"
                    >"Sel"</button>
                    <button class=move || if state.peak_source.get() == PeakSource::Processed { "peak-src-btn sel" } else { "peak-src-btn" }
                        on:click=move |_| state.peak_source.set(PeakSource::Processed)
                        title="Peak after DSP processing"
                    >"DSP"</button>
                </div>
            </Show>
            <div class="layer-panel-slider-row" style="margin-top: 6px;">
                <span class="slider-label">"Boost"</span>
                <label>{move || {
                    let db = state.gain_db.get();
                    let pv = if state.playback_mode.get() == PlaybackMode::PhaseVocoder { PV_MODE_BOOST_DB } else { 0.0 };
                    let total = db + pv;
                    if total > 0.0 { format!("+{:.0}dB", total) }
                    else { format!("{:.0}dB", total) }
                }}</label>
                <input type="range" min="-12" max="60" step="1"
                    prop:value=move || state.gain_db.get().to_string()
                    on:input=move |ev| {
                        let val: f64 = leptos::prelude::event_target_value(&ev).parse().unwrap_or(0.0);
                        state.gain_db.set(val);
                        if state.gain_mode.get_untracked() == GainMode::Off && val > 0.0 {
                            state.gain_mode.set(GainMode::Manual);
                        }
                    }
                    on:dblclick=move |_| {
                        state.gain_db.set(0.0);
                    }
                />
            </div>
        </ComboButton>
    }
}

/// Bandpass + EQ combo. Migrated from HfrButton's dropdown body. Owns
/// `bandpass_mode` (Off/Auto/On), `bandpass_range` (FollowFocus/Custom),
/// the four `filter_db_*` band sliders, `filter_quality`, and
/// `filter_band_mode` (3 vs 4 band split).
#[component]
fn BandpassCombo() -> impl IntoView {
    let state = expect_context::<AppState>();

    let is_open = Signal::derive(move || state.layer_panel_open.get() == Some(LayerPanel::Bandpass));
    let no_file = move || {
        state.current_file_index.get().is_none() && state.active_timeline.get().is_none()
    };

    let active = Signal::derive(move || state.bandpass_mode.get() != BandpassMode::Off);

    let left_class = Signal::derive(move || {
        if no_file() {
            "layer-btn combo-btn-left disabled"
        } else if active.get() {
            "layer-btn combo-btn-left no-annotation active"
        } else {
            "layer-btn combo-btn-left no-annotation"
        }
    });
    let right_class = Signal::derive(move || {
        if no_file() { return "layer-btn combo-btn-right disabled"; }
        let dim = if !active.get() { " dim" } else { "" };
        if is_open.get() {
            if dim.is_empty() { "layer-btn combo-btn-right open" } else { "layer-btn combo-btn-right dim open" }
        } else if dim.is_empty() { "layer-btn combo-btn-right" } else { "layer-btn combo-btn-right dim" }
    });

    // Left half just labels the button. Right half shows the bandpass
    // mode (OFF/AUTO/ON); when the configured range *differs* from the
    // FF/HFR band (i.e. locked to something else), the differing range
    // appears as a small caption above the mode word.
    let left_value = Signal::derive(|| "PASS".to_string());

    let range_differs = Signal::derive(move || {
        let lo = state.filter_freq_low.get();
        let hi = state.filter_freq_high.get();
        if hi <= lo { return false; }
        let ff_lo = state.band_ff_freq_lo.get();
        let ff_hi = state.band_ff_freq_hi.get();
        if ff_hi <= ff_lo { return true; } // no FF set, but bandpass has a range
        (lo - ff_lo).abs() > 50.0 || (hi - ff_hi).abs() > 50.0
    });
    let right_label = Signal::derive(move || {
        if !range_differs.get() { return String::new(); }
        let lo = state.filter_freq_low.get();
        let hi = state.filter_freq_high.get();
        format!("{:.1}\u{2013}{:.1}", lo / 1000.0, hi / 1000.0)
    });
    let right_value = Signal::derive(move || match state.bandpass_mode.get() {
        BandpassMode::Off => "OFF".to_string(),
        BandpassMode::Auto => "AUTO".to_string(),
        BandpassMode::On => "ON".to_string(),
    });

    let left_click = Callback::new(move |_: web_sys::MouseEvent| {
        if no_file() { return; }
        let mode = state.bandpass_mode.get_untracked();
        if mode == BandpassMode::Off {
            // Turn on: prefer Auto when HFR is on (band-following), else On.
            let next = if state.focus_stack.get_untracked().hfr_enabled() {
                BandpassMode::Auto
            } else {
                BandpassMode::On
            };
            state.bandpass_mode.set(next);
        } else {
            state.bandpass_mode.set(BandpassMode::Off);
        }
    });
    let toggle_menu = Callback::new(move |()| {
        toggle_panel(&state, LayerPanel::Bandpass);
    });

    let make_db_handler = |signal: RwSignal<f64>| {
        move |ev: web_sys::Event| {
            use wasm_bindgen::JsCast;
            let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
            if let Ok(val) = input.value().parse::<f64>() {
                if state.bandpass_mode.get_untracked() == BandpassMode::Auto {
                    state.bandpass_mode.set(BandpassMode::On);
                }
                signal.set(val);
            }
        }
    };
    let on_above_change = make_db_handler(state.filter_db_above);
    let on_selected_change = make_db_handler(state.filter_db_selected);
    let on_harmonics_change = make_db_handler(state.filter_db_harmonics);
    let on_below_change = make_db_handler(state.filter_db_below);

    let on_quality_click = move |q: FilterQuality| {
        move |_: web_sys::MouseEvent| {
            if state.bandpass_mode.get_untracked() == BandpassMode::Auto {
                state.bandpass_mode.set(BandpassMode::On);
            }
            state.filter_quality.set(q);
        }
    };
    let on_band_click = move |b: u8| {
        move |_: web_sys::MouseEvent| {
            if state.bandpass_mode.get_untracked() == BandpassMode::Auto {
                state.bandpass_mode.set(BandpassMode::On);
            }
            state.filter_band_mode.set(b);
        }
    };

    view! {
        <ComboButton
            left_label=""
            left_value=left_value
            left_click=left_click
            left_class=left_class
            right_value=right_value
            right_class=right_class
            right_label=right_label
            is_open=is_open
            toggle_menu=toggle_menu
            left_title="Toggle bandpass / EQ"
            right_title="Bandpass mode"
            menu_direction="below"
            panel_align="left"
            panel_style="min-width: 240px;"
        >
            <div class="layer-panel-title">"Bandpass"</div>
            <div style="display: flex; gap: 2px; padding: 0 6px 4px;">
                <Show when=move || state.hfr_enabled.get()>
                    <button class=move || layer_opt_class_simple(state.bandpass_mode.get() == BandpassMode::Auto)
                        on:click=move |_| state.bandpass_mode.set(BandpassMode::Auto)
                    >"AUTO"</button>
                </Show>
                <button class=move || layer_opt_class_simple(state.bandpass_mode.get() == BandpassMode::Off)
                    on:click=move |_| state.bandpass_mode.set(BandpassMode::Off)
                >"OFF"</button>
                <button class=move || layer_opt_class_simple(state.bandpass_mode.get() == BandpassMode::On)
                    on:click=move |_| {
                        if !state.focus_stack.get_untracked().hfr_enabled() {
                            state.focus_stack.update(|s| s.set_saved_playback_mode(Some(PlaybackMode::Normal)));
                            state.toggle_hfr();
                        }
                        state.bandpass_mode.set(BandpassMode::On);
                    }
                >"ON"</button>
            </div>
            <Show when=move || {
                let bp = state.bandpass_mode.get();
                bp == BandpassMode::On
                    || (bp == BandpassMode::Auto && state.band_ff_freq_hi.get() > state.band_ff_freq_lo.get())
            }>
                <div style="display: flex; gap: 2px; padding: 0 6px 2px;">
                    <button class=move || layer_opt_class_simple(state.bandpass_range.get() == BandpassRange::FollowFocus)
                        on:click=move |_| state.bandpass_range.set(BandpassRange::FollowFocus)
                        title="Range tracks the active FF/HFR focus"
                    >"Follow"</button>
                    <button class=move || layer_opt_class_simple(state.bandpass_range.get() == BandpassRange::Locked)
                        on:click=move |_| state.bandpass_range.set(BandpassRange::Locked)
                        title="Lock the range here \u{2014} won't track focus changes"
                    >"Locked"</button>
                </div>
                <div class="bandpass-range-readout">
                    {move || format!("{:.1}\u{2013}{:.1} kHz",
                        state.filter_freq_low.get() / 1000.0,
                        state.filter_freq_high.get() / 1000.0
                    )}
                    <Show when=move || state.bandpass_range.get() == BandpassRange::Locked>
                        <span class="bandpass-lock-icon" title="Locked">{"\u{00A0}\u{1F512}"}</span>
                    </Show>
                </div>
                <div style="display: flex; gap: 2px; padding: 0 6px 2px;">
                    <button class=move || layer_opt_class_simple(state.filter_quality.get() == FilterQuality::Fast)
                        on:click=on_quality_click(FilterQuality::Fast)
                        title="IIR band-split \u{2014} low latency, softer edges"
                    >"Fast"</button>
                    <button class=move || layer_opt_class_simple(state.filter_quality.get() == FilterQuality::Spectral)
                        on:click=on_quality_click(FilterQuality::Spectral)
                        title="FFT spectral EQ \u{2014} sharp edges, higher latency"
                    >"HQ"</button>
                    <span style="width: 8px;"></span>
                    <button class=move || layer_opt_class_simple(state.filter_band_mode.get() == 3)
                        on:click=on_band_click(3)
                    >"3"</button>
                    <button class=move || layer_opt_class_simple(state.filter_band_mode.get() == 4)
                        on:click=on_band_click(4)
                    >"4"</button>
                </div>
                <div class="layer-panel-slider-row"
                    on:mouseenter=move |_| state.filter_hovering_band.set(Some(3))
                    on:mouseleave=move |_| state.filter_hovering_band.set(None)
                >
                    <label>"Above"</label>
                    <input type="range" min="-60" max="6" step="1"
                        prop:value=move || state.filter_db_above.get().to_string()
                        on:input=on_above_change
                    />
                    <span>{move || format!("{:.0}", state.filter_db_above.get())}</span>
                </div>
                <Show when=move || { state.filter_band_mode.get() >= 4 }>
                    <div class="layer-panel-slider-row"
                        on:mouseenter=move |_| state.filter_hovering_band.set(Some(2))
                        on:mouseleave=move |_| state.filter_hovering_band.set(None)
                    >
                        <label>"Harm"</label>
                        <input type="range" min="-60" max="6" step="1"
                            prop:value=move || state.filter_db_harmonics.get().to_string()
                            on:input=on_harmonics_change
                        />
                        <span>{move || format!("{:.0}", state.filter_db_harmonics.get())}</span>
                    </div>
                </Show>
                <div class="layer-panel-slider-row"
                    on:mouseenter=move |_| state.filter_hovering_band.set(Some(1))
                    on:mouseleave=move |_| state.filter_hovering_band.set(None)
                >
                    <label>"Band"</label>
                    <input type="range" min="-60" max="6" step="1"
                        prop:value=move || state.filter_db_selected.get().to_string()
                        on:input=on_selected_change
                    />
                    <span>{move || format!("{:.0}", state.filter_db_selected.get())}</span>
                </div>
                <div class="layer-panel-slider-row"
                    on:mouseenter=move |_| state.filter_hovering_band.set(Some(0))
                    on:mouseleave=move |_| state.filter_hovering_band.set(None)
                >
                    <label>"Below"</label>
                    <input type="range" min="-60" max="6" step="1"
                        prop:value=move || state.filter_db_below.get().to_string()
                        on:input=on_below_change
                    />
                    <span>{move || format!("{:.0}", state.filter_db_below.get())}</span>
                </div>
            </Show>
        </ComboButton>
    }
}

// NotchCombo and NrCombo live in `noise_combos.rs` — they contain the
// Detect / Sensitivity / Bands / Learn / Strength controls migrated
// from the deleted right-sidebar Noise Filter tab.

/// Hearing Bar. Mounted between the OverviewPanel and the main view.
///
/// Per-letter brightness on the "HFR" cell encodes whether HFR is on, and
/// (when on) whether the active band is audible-only (H dim) or includes
/// ultrasound (H bright). The classes are applied to the wrapper div around
/// `HfrButton` so the existing `.hearing-hfr-cell ... .layer-btn-value`
/// CSS rules paint it.
#[component]
pub fn HearingBar() -> impl IntoView {
    let state = expect_context::<AppState>();
    let cell_class = Signal::derive(move || {
        if !state.hfr_enabled.get() {
            "hearing-hfr-cell"
        } else {
            let lo = state.band_ff_freq_lo.get();
            let hi = state.band_ff_freq_hi.get();
            if hi > lo && hi < 24_000.0 {
                "hearing-hfr-cell hfr-on hfr-h-dim"
            } else {
                "hearing-hfr-cell hfr-on"
            }
        }
    });
    // Label reads "HEARING" by default; when HFR is on and a band is
    // set, it switches to show the active frequency range — gives the
    // user the numeric answer at a glance.
    let bar_label = Signal::derive(move || {
        if state.hfr_enabled.get() {
            let lo = state.band_ff_freq_lo.get();
            let hi = state.band_ff_freq_hi.get();
            if hi > lo {
                return format!("{:.1}\u{2013}{:.1} kHz", lo / 1000.0, hi / 1000.0);
            }
        }
        "HEARING".to_string()
    });
    view! {
        <div class="hearing-bar"
            class:panel-open=move || matches!(state.layer_panel_open.get().map(LayerPanel::bar), Some(Bar::Hearing))
        >
            <span class="bar-label" class:bar-label-range=move || state.hfr_enabled.get()>
                {move || bar_label.get()}
            </span>
            <div class="bar-controls">
                <div class=move || cell_class.get()>
                    <HfrButton/>
                </div>
                <ModeButton/>
                <div class="bar-sep"></div>
                <BandpassCombo/>
                <GainCombo/>
                <NrCombo/>
                <NotchCombo/>
                <div class="bar-spacer"></div>
                <ListenButton/>
            </div>
        </div>
    }
}

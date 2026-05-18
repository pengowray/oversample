// Hearing Bar — the "what comes out the speakers" strip between the
// Overview and the main canvas.
//
// Layout:  [HFR ▾ HET] │ [Band] [EQ] [Notch] [Gain] │ [🎤 Listen]
//
//          ^brightness   ^DSP filter combos          ^live mic
//           encoded
//
// The HFR cell wraps the full `HfrButton` in a class that drives per-letter
// brightness on the "HFR" label (H dims when the active band sits entirely
// below 24 kHz — i.e. an audible-only filter). Listen lives at the right
// end because its DSP pipeline is unified with HFR. Filter combos in the
// middle wrap onto a second row on narrow viewports.

use leptos::prelude::*;

use crate::audio::streaming_playback::PV_MODE_BOOST_DB;
use crate::components::combo_button::ComboButton;
use crate::components::hfr_button::HfrButton;
use crate::components::listen_button::ListenButton;
use crate::state::{
    ActiveFocus, AppState, BandpassMode, BandpassRange, FilterQuality, GainMode, LayerPanel,
    PeakSource, PlaybackMode, RightSidebarTab,
};

fn layer_opt_class(active: bool, disabled: bool) -> &'static str {
    if disabled {
        "layer-panel-opt disabled"
    } else if active {
        "layer-panel-opt sel"
    } else {
        "layer-panel-opt"
    }
}

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

fn nyquist_for_current(state: AppState) -> f64 {
    // When listening or recording, the live waterfall has the most up-to-date
    // sample rate (USB devices can re-negotiate after the AppState signal is
    // first set). Fall back to AppState's view otherwise — that handles the
    // armed-but-not-streaming case too.
    let is_mic_active = state.mic_recording.get_untracked() || state.mic_listening.get_untracked();
    if is_mic_active && crate::canvas::live_waterfall::is_active() {
        crate::canvas::live_waterfall::max_freq()
    } else {
        state.active_nyquist()
    }
}

/// Apply a band preset: set the BandFF range and ensure HFR is enabled.
/// Used by every preset button except "None".
fn apply_band(state: AppState, lo: f64, hi: f64) {
    state.set_band_ff_range(lo, hi);
    if !state.focus_stack.get_untracked().hfr_enabled() {
        state.toggle_hfr();
    }
    state.layer_panel_open.set(None);
}

fn clear_band(state: AppState) {
    state.set_band_ff_range(0.0, 0.0);
    if state.focus_stack.get_untracked().hfr_enabled() {
        state.toggle_hfr();
    }
    state.layer_panel_open.set(None);
}

/// Range of the bat-book species auto-resolved from the file's metadata.
/// Returns None when there's no file, no species match, or the entry has
/// no useful frequency bounds.
fn file_species_range(state: AppState) -> Option<(String, f64, f64)> {
    let files = state.files.get_untracked();
    let idx = state.current_file_index.get_untracked()?;
    let file = files.get(idx)?;
    let favourites = state.bat_book_favourites.get_untracked();
    let resolved = crate::bat_book::auto_resolve::resolve_auto(Some(file), &favourites);
    let species_id = resolved.matched_species_id?;
    let entry = crate::bat_book::auto_resolve::find_entry_in_manifest(resolved.region, &species_id)
        .or_else(|| crate::bat_book::auto_resolve::find_entry_any_book(&species_id))?;
    if entry.freq_lo_hz <= 0.0 || entry.freq_hi_hz <= entry.freq_lo_hz {
        return None;
    }
    let nyq = file.spectrogram.max_freq;
    Some((entry.name.to_string(), entry.freq_lo_hz, entry.freq_hi_hz.min(nyq)))
}

/// Range covering all currently-selected bat book species (min lo, max hi).
fn selected_species_range(state: AppState) -> Option<(f64, f64)> {
    let ids = state.bat_book_selected_ids.get_untracked();
    if ids.is_empty() {
        return None;
    }
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    let mut found = false;
    for id in &ids {
        if let Some(entry) = crate::bat_book::auto_resolve::find_entry_any_book(id) {
            if entry.freq_lo_hz > 0.0 && entry.freq_hi_hz > entry.freq_lo_hz {
                lo = lo.min(entry.freq_lo_hz);
                hi = hi.max(entry.freq_hi_hz);
                found = true;
            }
        }
    }
    if found && hi > lo { Some((lo, hi)) } else { None }
}

/// Range from whichever of selection / annotation / frequency-focus is
/// the active focus right now (only one can be active at a time).
fn focused_range(state: AppState) -> Option<(f64, f64)> {
    match state.active_focus.get_untracked() {
        Some(ActiveFocus::TransientSelection) => {
            let sel = state.selection.get_untracked()?;
            match (sel.freq_low, sel.freq_high) {
                (Some(lo), Some(hi)) if hi > lo => Some((lo, hi)),
                _ => None,
            }
        }
        Some(ActiveFocus::Annotations) => state.selected_annotation_focus_range(),
        Some(ActiveFocus::FrequencyFocus) => {
            let r = state.focus_stack.get_untracked().effective_range();
            if r.is_active() { Some((r.lo, r.hi)) } else { None }
        }
        None => None,
    }
}

/// Band Presets combo — sits next to the HFR/FR toggle.
#[component]
fn BandPresetsCombo() -> impl IntoView {
    let state = expect_context::<AppState>();

    let is_open = Signal::derive(move || {
        state.layer_panel_open.get() == Some(LayerPanel::BandPresets)
    });
    let no_file = move || {
        state.current_file_index.get().is_none() && state.active_timeline.get().is_none()
    };

    // Detect which preset is active so the right-side label reflects state.
    let preset_label = Signal::derive(move || {
        if !state.hfr_enabled.get() {
            return "None".to_string();
        }
        let lo = state.band_ff_freq_lo.get();
        let hi = state.band_ff_freq_hi.get();
        if hi <= lo { return "None".to_string(); }
        let nyq = nyquist_for_current(state);
        let close = |a: f64, b: f64| (a - b).abs() < 100.0;
        if close(lo, 0.0) && close(hi, nyq) { return "All".to_string(); }
        if close(lo, 20_000.0) && close(hi, nyq) { return "Ultrasound".to_string(); }
        if close(lo, 0.0) && close(hi, 24_000.0) { return "Audible".to_string(); }
        "Custom".to_string()
    });

    let left_class = Signal::derive(move || {
        if no_file() {
            "layer-btn combo-btn-left no-annotation disabled"
        } else if state.hfr_enabled.get() {
            "layer-btn combo-btn-left no-annotation active"
        } else {
            "layer-btn combo-btn-left no-annotation"
        }
    });
    let right_class = Signal::derive(move || {
        if no_file() {
            "layer-btn combo-btn-right disabled"
        } else if is_open.get() {
            "layer-btn combo-btn-right open"
        } else {
            "layer-btn combo-btn-right"
        }
    });

    let left_click = Callback::new(move |_: web_sys::MouseEvent| {
        if no_file() { return; }
        toggle_panel(&state, LayerPanel::BandPresets);
    });
    let toggle_menu = Callback::new(move |()| {
        toggle_panel(&state, LayerPanel::BandPresets);
    });

    let left_value = Signal::derive(move || preset_label.get());
    let right_value = Signal::derive(String::new);

    view! {
        <ComboButton
            left_label="Band"
            left_value=left_value
            left_click=left_click
            left_class=left_class
            right_value=right_value
            right_class=right_class
            is_open=is_open
            toggle_menu=toggle_menu
            left_title="Band presets"
            right_title="Band presets"
            menu_direction="below"
            panel_align="left"
            panel_style="min-width: 220px;"
        >
            <div class="layer-panel-title">"Band presets"</div>

            // ── All ──
            <button
                class="layer-panel-opt"
                on:click=move |_| {
                    let nyq = nyquist_for_current(state);
                    apply_band(state, 0.0, nyq);
                }
            >"All"</button>

            // ── Bat book: file species ──
            {move || {
                let r = file_species_range(state);
                let label = match &r {
                    Some((name, lo, hi)) => format!(
                        "Bat book: {} ({:.0}\u{2013}{:.0} kHz)",
                        name, lo / 1000.0, hi / 1000.0,
                    ),
                    None => "Bat book: file species".to_string(),
                };
                let disabled = r.is_none();
                view! {
                    <button
                        class=move || layer_opt_class(false, disabled)
                        disabled=disabled
                        on:click=move |_| {
                            if let Some((_, lo, hi)) = file_species_range(state) {
                                apply_band(state, lo, hi);
                            }
                        }
                    >{label}</button>
                }
            }}

            // ── Bat book: selected species ──
            {move || {
                let r = selected_species_range(state);
                let label = match r {
                    Some((lo, hi)) => format!(
                        "Bat book: selected ({:.0}\u{2013}{:.0} kHz)",
                        lo / 1000.0, hi / 1000.0,
                    ),
                    None => "Bat book: selected species".to_string(),
                };
                let disabled = r.is_none();
                view! {
                    <button
                        class=move || layer_opt_class(false, disabled)
                        disabled=disabled
                        on:click=move |_| {
                            if let Some((lo, hi)) = selected_species_range(state) {
                                apply_band(state, lo, hi);
                            }
                        }
                    >{label}</button>
                }
            }}

            // ── Selection or annotation (whichever is focused) ──
            {move || {
                let r = focused_range(state);
                let (label, hint) = match (state.active_focus.get(), &r) {
                    (Some(ActiveFocus::TransientSelection), Some((lo, hi))) => (
                        "Selection".to_string(),
                        format!(" ({:.0}\u{2013}{:.0} kHz)", lo / 1000.0, hi / 1000.0),
                    ),
                    (Some(ActiveFocus::Annotations), Some((lo, hi))) => (
                        "Annotation".to_string(),
                        format!(" ({:.0}\u{2013}{:.0} kHz)", lo / 1000.0, hi / 1000.0),
                    ),
                    (Some(ActiveFocus::FrequencyFocus), Some((lo, hi))) => (
                        "Focus".to_string(),
                        format!(" ({:.0}\u{2013}{:.0} kHz)", lo / 1000.0, hi / 1000.0),
                    ),
                    _ => ("Selection / annotation".to_string(), String::new()),
                };
                let disabled = r.is_none();
                view! {
                    <button
                        class=move || layer_opt_class(false, disabled)
                        disabled=disabled
                        on:click=move |_| {
                            if let Some((lo, hi)) = focused_range(state) {
                                apply_band(state, lo, hi);
                            }
                        }
                    >{label}{hint}</button>
                }
            }}

            // ── Ultrasound ──
            <button
                class="layer-panel-opt"
                on:click=move |_| {
                    let nyq = nyquist_for_current(state);
                    if nyq > 20_000.0 {
                        apply_band(state, 20_000.0, nyq);
                    }
                }
            >"Ultrasound (≥20 kHz)"</button>

            // ── Audible ──
            <button
                class="layer-panel-opt"
                on:click=move |_| { apply_band(state, 0.0, 24_000.0); }
            >"Audible (\u{2264}24 kHz)"</button>

            <hr/>

            // ── None ──
            <button
                class="layer-panel-opt"
                on:click=move |_| { clear_band(state); }
            >"None"</button>
        </ComboButton>
    }
}

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

    let left_value = Signal::derive(move || {
        if state.bandpass_mode.get() == BandpassMode::Off { return String::new(); }
        let lo = state.filter_freq_low.get();
        let hi = state.filter_freq_high.get();
        if hi > lo {
            format!("{:.0}\u{2013}{:.0}", lo / 1000.0, hi / 1000.0)
        } else {
            String::new()
        }
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
            left_label="Bandpass"
            left_value=left_value
            left_click=left_click
            left_class=left_class
            right_value=right_value
            right_class=right_class
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
                    >"Band"</button>
                    <button class=move || layer_opt_class_simple(state.bandpass_range.get() == BandpassRange::Custom)
                        on:click=move |_| state.bandpass_range.set(BandpassRange::Custom)
                    >"Custom"</button>
                </div>
                <div style="padding: 0 8px 2px; font-size: 10px; opacity: 0.7;">
                    {move || format!("{:.1}\u{2013}{:.1} kHz",
                        state.filter_freq_low.get() / 1000.0,
                        state.filter_freq_high.get() / 1000.0
                    )}
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

/// Notch combo — compact toggle + summary. Full editor stays in the
/// right sidebar's Notch tab; a button in the dropdown jumps there.
#[component]
fn NotchCombo() -> impl IntoView {
    let state = expect_context::<AppState>();

    let is_open = Signal::derive(move || state.layer_panel_open.get() == Some(LayerPanel::Notch));
    let no_file = move || {
        state.current_file_index.get().is_none() && state.active_timeline.get().is_none()
    };

    let band_count = Signal::derive(move || state.notch_bands.get().len());
    let enabled = Signal::derive(move || state.notch_enabled.get());

    let left_class = Signal::derive(move || {
        if no_file() {
            "layer-btn combo-btn-left disabled"
        } else if enabled.get() {
            "layer-btn combo-btn-left no-annotation active"
        } else {
            "layer-btn combo-btn-left no-annotation"
        }
    });
    let right_class = Signal::derive(move || {
        if no_file() { return "layer-btn combo-btn-right disabled"; }
        let dim = if !enabled.get() { " dim" } else { "" };
        if is_open.get() {
            if dim.is_empty() { "layer-btn combo-btn-right open" } else { "layer-btn combo-btn-right dim open" }
        } else if dim.is_empty() { "layer-btn combo-btn-right" } else { "layer-btn combo-btn-right dim" }
    });
    let left_value = Signal::derive(move || {
        let n = band_count.get();
        if n == 0 { String::new() }
        else if n == 1 { "1 band".to_string() }
        else { format!("{} bands", n) }
    });
    let right_value = Signal::derive(move || if enabled.get() { "ON".to_string() } else { "OFF".to_string() });

    let left_click = Callback::new(move |_: web_sys::MouseEvent| {
        if no_file() { return; }
        state.notch_enabled.update(|v| *v = !*v);
    });
    let toggle_menu = Callback::new(move |()| {
        toggle_panel(&state, LayerPanel::Notch);
    });

    view! {
        <ComboButton
            left_label="Notch"
            left_value=left_value
            left_click=left_click
            left_class=left_class
            right_value=right_value
            right_class=right_class
            is_open=is_open
            toggle_menu=toggle_menu
            left_title="Toggle notch / noise filter"
            right_title="Notch options"
            menu_direction="below"
            panel_align="left"
            panel_style="min-width: 200px;"
        >
            <button
                class=move || layer_opt_class_simple(state.notch_enabled.get())
                on:click=move |_| {
                    state.notch_enabled.set(true);
                    state.layer_panel_open.set(None);
                }
            >"On"</button>
            <button
                class=move || layer_opt_class_simple(!state.notch_enabled.get())
                on:click=move |_| {
                    state.notch_enabled.set(false);
                    state.layer_panel_open.set(None);
                }
            >"Off"</button>
            <hr/>
            <div style="padding: 4px 8px; font-size: 10px; color: #999;">
                {move || {
                    let n = band_count.get();
                    if n == 0 { "No bands defined".to_string() }
                    else if n == 1 { "1 band defined".to_string() }
                    else { format!("{} bands defined", n) }
                }}
            </div>
            <button
                class="layer-panel-opt"
                on:click=move |_| {
                    state.right_sidebar_tab.set(RightSidebarTab::Notch);
                    state.right_sidebar_collapsed.set(false);
                    state.layer_panel_open.set(None);
                }
            >"Open noise filter editor \u{2192}"</button>
        </ComboButton>
    }
}

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
    view! {
        <div class="hearing-bar"
            class:panel-open=move || state.layer_panel_open.get().is_some()
        >
            <span class="bar-label">"HEARING"</span>
            <div class="bar-controls">
                <div class=move || cell_class.get()>
                    <HfrButton/>
                </div>
                <div class="bar-sep"></div>
                <BandPresetsCombo/>
                <BandpassCombo/>
                <NotchCombo/>
                <GainCombo/>
                <div class="bar-spacer"></div>
                <ListenButton/>
            </div>
        </div>
    }
}

// Range dropdown button — sits to the right of the BAND/HFR cell on the
// Hearing Bar.
//
//   • When HFR is off, shows "OFF" in dim grey (matches the dim "HFR" on
//     the toggle cell).
//   • When HFR is on, shows the active band range (e.g. "45.0–120.0 kHz"
//     — the text that used to live in the BAND cell itself).
//   • Clicking opens the band-presets dropdown (All, bat-book file
//     species, bat-book selected species, focus/selection/annotation,
//     lowest bat frequency and up, Audible, None).
//
// Replaces the previous `HfrButton` combo, whose left half (the HFR
// toggle) is the top row of [HfrSwitch] below.

use crate::components::layer_panel::toggle_panel;
use crate::state::store_fields::*;
use leptos::prelude::*;

use crate::components::switch_button::SwitchButton;
use crate::focus_stack::FocusRange;
use crate::state::{ActiveFocus, AppState, LayerPanel};

fn layer_opt_class(active: bool, disabled: bool) -> &'static str {
    if disabled {
        "layer-panel-opt disabled"
    } else if active {
        "layer-panel-opt sel"
    } else {
        "layer-panel-opt"
    }
}

/// Format Hz as a kHz string for the Range button label: drop ".0" for
/// whole-kHz values (40 kHz, not 40.0 kHz); keep one decimal otherwise.
fn fmt_khz(hz: f64) -> String {
    let khz = hz / 1000.0;
    if (khz - khz.round()).abs() < 0.05 {
        format!("{}", khz.round() as i32)
    } else {
        format!("{:.1}", khz)
    }
}

/// Nyquist for the *currently relevant* signal source. When listening or
/// recording, the live waterfall has the most up-to-date sample rate (USB
/// devices can re-negotiate after the AppState signal is first set).
pub(crate) fn nyquist_for_current(state: AppState) -> f64 {
    let is_mic_active =
        state.mic.recording().get_untracked() || state.mic.listening().get_untracked();
    if is_mic_active && crate::canvas::live_waterfall::is_active() {
        crate::canvas::live_waterfall::max_freq()
    } else {
        state.active_nyquist()
    }
}

/// Apply a band preset: set the BandFF range and ensure HFR is enabled.
fn apply_band(state: AppState, lo: f64, hi: f64) {
    state.set_band_ff_range(lo, hi);
    if !state.viewmode.focus_stack().get_untracked().hfr_enabled() {
        state.toggle_hfr();
    }
    state.panels.layer_panel_open().set(None);
}

fn clear_band(state: AppState) {
    state.set_band_ff_range(0.0, 0.0);
    if state.viewmode.focus_stack().get_untracked().hfr_enabled() {
        state.toggle_hfr();
    }
    state.panels.layer_panel_open().set(None);
}

/// The band HFR uses when it is turned on with no band set: the range of the
/// species named in the file's metadata when that fits the file, otherwise
/// from the lowest bat frequency up (`FocusRange::hfr_default`). Tracks the
/// open file. With no file open it assumes a 192 kHz recording.
pub fn hfr_default_band(state: AppState) -> Option<FocusRange> {
    let idx = state.library.current_index().get();
    let (nyquist, species_range) = state.library.files().with(|files| {
        let file = idx.and_then(|i| files.get(i));
        let nyquist = file.map(|f| f.spectrogram.max_freq).unwrap_or(96_000.0);
        let species_range = file.and_then(|f| {
            use crate::bat_book::auto_resolve;
            let favourites = state.bat_book.favourites().get_untracked();
            let resolved = auto_resolve::resolve_auto(Some(f), &favourites);
            let species_id = resolved.matched_species_id?;
            let entry = auto_resolve::find_entry_in_manifest(resolved.region, &species_id)
                .or_else(|| auto_resolve::find_entry_any_book(&species_id))?;
            (entry.freq_lo_hz > 0.0 && entry.freq_hi_hz > entry.freq_lo_hz)
                .then_some((entry.freq_lo_hz, entry.freq_hi_hz))
        });
        (nyquist, species_range)
    });
    let lowest = state.lowest_bat_hz_untracked() as f64;
    FocusRange::hfr_default(species_range, lowest, nyquist)
}

/// Range of the bat-book species auto-resolved from the file's metadata.
fn file_species_range(state: AppState) -> Option<(String, f64, f64)> {
    let files = state.library.files().get_untracked();
    let idx = state.library.current_index().get_untracked()?;
    let file = files.get(idx)?;
    let favourites = state.bat_book.favourites().get_untracked();
    let resolved = crate::bat_book::auto_resolve::resolve_auto(Some(file), &favourites);
    let species_id = resolved.matched_species_id?;
    let entry = crate::bat_book::auto_resolve::find_entry_in_manifest(resolved.region, &species_id)
        .or_else(|| crate::bat_book::auto_resolve::find_entry_any_book(&species_id))?;
    if entry.freq_lo_hz <= 0.0 || entry.freq_hi_hz <= entry.freq_lo_hz {
        return None;
    }
    // The range is empty when the species calls above the file's range.
    // Applying an empty band makes HFR pick its default band
    // (FocusRange::hfr_default).
    let r = FocusRange::new(entry.freq_lo_hz, entry.freq_hi_hz).clip_to(file.spectrogram.max_freq);
    Some((entry.name.to_string(), r.lo, r.hi))
}

/// Range from whichever of selection / annotation / frequency-focus is
/// the active focus right now (only one can be active at a time).
fn focused_range(state: AppState) -> Option<(f64, f64)> {
    match state.interaction.active_focus().get_untracked() {
        Some(ActiveFocus::TransientSelection) => {
            let sel = state.interaction.selection().get_untracked()?;
            match (sel.freq_low, sel.freq_high) {
                (Some(lo), Some(hi)) if hi > lo => Some((lo, hi)),
                _ => None,
            }
        }
        Some(ActiveFocus::Annotations) => state.selected_annotation_focus_range(),
        Some(ActiveFocus::FrequencyFocus) => {
            let r = state
                .viewmode
                .focus_stack()
                .get_untracked()
                .effective_range();
            if r.is_active() {
                Some((r.lo, r.hi))
            } else {
                None
            }
        }
        None => None,
    }
}

/// Where the file's auto-matched species name came from, for the preset
/// sub-line. Mirrors `auto_resolve::get_scientific_name`'s priority order.
fn file_species_source(state: AppState) -> Option<String> {
    let files = state.library.files().get_untracked();
    let idx = state.library.current_index().get_untracked()?;
    let file = files.get(idx)?;
    if let Some(meta) = &file.site_metadata {
        if meta
            .fields
            .iter()
            .any(|(k, v)| k == "Scientific name" && !v.is_empty())
        {
            return Some(format!("{}: species", meta.site.name()));
        }
    }
    if let Some(guano) = &file.audio.metadata.guano {
        if guano
            .fields
            .iter()
            .any(|(k, v)| k == "Species|Manual" && !v.is_empty())
        {
            return Some("GUANO: Species (Manual)".to_string());
        }
        if guano
            .fields
            .iter()
            .any(|(k, v)| k == "Species|Auto" && !v.is_empty())
        {
            return Some("GUANO: Species (Auto)".to_string());
        }
    }
    None
}

/// The bat-book "current page" selection as (label, lo, hi): the species name
/// when a single species is selected, otherwise an "N species" summary over the
/// union of their ranges.
fn selected_species_info(state: AppState) -> Option<(String, f64, f64)> {
    let ids = state.bat_book.selected_ids().get_untracked();
    if ids.is_empty() {
        return None;
    }
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    let mut first_name = String::new();
    let mut count = 0usize;
    for id in &ids {
        if let Some(entry) = crate::bat_book::auto_resolve::find_entry_any_book(id) {
            if entry.freq_lo_hz > 0.0 && entry.freq_hi_hz > entry.freq_lo_hz {
                lo = lo.min(entry.freq_lo_hz);
                hi = hi.max(entry.freq_hi_hz);
                if count == 0 {
                    first_name = entry.name.to_string();
                }
                count += 1;
            }
        }
    }
    if count == 0 || hi <= lo {
        return None;
    }
    let label = if count == 1 {
        first_name
    } else {
        format!("{count} species")
    };
    Some((label, lo, hi))
}

/// The band the selected species preset applies: the selected species'
/// range clipped to the file, like the file species preset. The label keeps
/// the species' own range.
fn selected_species_band(state: AppState) -> Option<(f64, f64)> {
    let (_, lo, hi) = selected_species_info(state)?;
    let r = FocusRange::new(lo, hi).clip_to(nyquist_for_current(state));
    Some((r.lo, r.hi))
}

/// True when HFR is on and the current BandFF range matches `(lo, hi)` — used
/// to tick the preset the active band came from. 50 Hz tolerance absorbs
/// rounding in display/storage.
fn band_selected(state: AppState, lo: f64, hi: f64) -> bool {
    if !state.viewmode.hfr_enabled().get() {
        return false;
    }
    let clo = state.filter.band_ff_freq_lo().get();
    let chi = state.filter.band_ff_freq_hi().get();
    (clo - lo).abs() < 50.0 && (chi - hi).abs() < 50.0
}

/// The HFR control at the start of the Hear bar. Top row: turns HFR on or
/// off. Bottom row: the band HFR plays, and opens the Band presets menu.
#[component]
pub fn HfrSwitch() -> impl IntoView {
    let state = expect_context::<AppState>();

    let is_open = Signal::derive(move || {
        state.panels.layer_panel_open().get() == Some(LayerPanel::BandPresets)
    });
    let no_file = move || state.no_recording_open();

    // The band in use. While HFR is off the row shows, dimmed, the band HFR
    // will use: the band still set, or the one HFR picks when none is set.
    let value = Signal::derive(move || {
        let lo = state.filter.band_ff_freq_lo().get();
        let hi = state.filter.band_ff_freq_hi().get();
        let band = if hi > lo {
            Some((lo, hi))
        } else if state.no_recording_open() {
            None
        } else {
            hfr_default_band(state).map(|r| (r.lo, r.hi))
        };
        match band {
            Some((lo, hi)) => format!("{}\u{2013}{} kHz", fmt_khz(lo), fmt_khz(hi)),
            None => "Band presets".to_string(),
        }
    });

    let toggle = Callback::new(move |()| state.toggle_hfr());
    let toggle_menu = Callback::new(move |()| toggle_panel(&state, LayerPanel::BandPresets));

    view! {
            <SwitchButton
                name="HFR"
                class="hfr-switch"
                on=Signal::derive(move || state.viewmode.hfr_enabled().get())
                toggle=toggle
                value=value
                is_open=is_open
                toggle_menu=toggle_menu
                disabled=Signal::derive(no_file)
                width="108px"
                switch_title="HFR (High Frequency Range): hear the band selected on the frequency strip at a lower pitch"
                menu_title="Band presets"
                panel_style="min-width: 220px;"
            >
                <div class="layer-panel-title">"Band presets"</div>

                // ── All ──
                <button
                    class=move || layer_opt_class(band_selected(state, 0.0, nyquist_for_current(state)), false)
                    on:click=move |_| {
                        let nyq = nyquist_for_current(state);
                        apply_band(state, 0.0, nyq);
                    }
                >"All"</button>

                // ── File species (auto-matched from metadata) ──
                {move || {
                    let r = file_species_range(state);
                    let disabled = r.is_none();
                    let (main, sub) = match &r {
                        Some((name, lo, hi)) => (
                            format!("{} ({}\u{2013}{} kHz)", name, fmt_khz(*lo), fmt_khz(*hi)),
                            file_species_source(state),
                        ),
                        None => ("Bat book: file species".to_string(), None),
                    };
                    let sel_range = r.as_ref().map(|(_, lo, hi)| (*lo, *hi));
                    view! {
                        <button
                            class=move || layer_opt_class(
                                sel_range.is_some_and(|(lo, hi)| band_selected(state, lo, hi)),
                                disabled,
                            )
                            disabled=disabled
                            on:click=move |_| {
                                if let Some((_, lo, hi)) = file_species_range(state) {
                                    apply_band(state, lo, hi);
                                }
                            }
                        >
                            <span class="preset-main">{main}</span>
                            {sub.map(|s| view! { <span class="preset-sub">{s}</span> })}
                        </button>
                    }
                }}

                // ── Bat book selection (current page) ──
                {move || {
                    let info = selected_species_info(state);
                    let disabled = info.is_none();
                    let (main, sub) = match &info {
                        Some((name, lo, hi)) => (
                            format!("{} ({}\u{2013}{} kHz)", name, fmt_khz(*lo), fmt_khz(*hi)),
                            Some("Bat book: current page".to_string()),
                        ),
                        None => ("Bat book: selected species".to_string(), None),
                    };
                    let sel_range = selected_species_band(state);
                    view! {
                        <button
                            class=move || layer_opt_class(
                                sel_range.is_some_and(|(lo, hi)| band_selected(state, lo, hi)),
                                disabled,
                            )
                            disabled=disabled
                            on:click=move |_| {
                                if let Some((lo, hi)) = selected_species_band(state) {
                                    apply_band(state, lo, hi);
                                }
                            }
                        >
                            <span class="preset-main">{main}</span>
                            {sub.map(|s| view! { <span class="preset-sub">{s}</span> })}
                        </button>
                    }
                }}

                // ── Selection / annotation / focus ──
                {move || {
                    let r = focused_range(state);
                    let (label, hint) = match (state.interaction.active_focus().get(), &r) {
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
                    let sel_range = r;
                    view! {
                        <button
                            class=move || layer_opt_class(
                                sel_range.is_some_and(|(lo, hi)| band_selected(state, lo, hi)),
                                disabled,
                            )
                            disabled=disabled
                            on:click=move |_| {
                                if let Some((lo, hi)) = focused_range(state) {
                                    apply_band(state, lo, hi);
                                }
                            }
                        >{label}{hint}</button>
                    }
                }}

                // ── Bat calls: lowest bat frequency (Settings > Bats) to Nyquist ──
                <button
                    class=move || {
                        let lo = state.lowest_bat_hz() as f64;
                        layer_opt_class(band_selected(state, lo, nyquist_for_current(state)), false)
                    }
                    on:click=move |_| {
                        let nyq = nyquist_for_current(state);
                        let lo = state.lowest_bat_hz_untracked() as f64;
                        if nyq > lo {
                            apply_band(state, lo, nyq);
                        }
                    }
                >{move || format!("From lowest bat frequency (\u{2265}{:.0} kHz)", state.lowest_bat_hz() / 1000.0)}</button>

                // ── Audible ──
                <button
                    class=move || layer_opt_class(band_selected(state, 0.0, crate::dsp::ultrasonic::HEARING_LIMIT_HZ), false)
                    on:click=move |_| { apply_band(state, 0.0, crate::dsp::ultrasonic::HEARING_LIMIT_HZ); }
                >"Audible (\u{2264}20 kHz)"</button>

                <hr/>

                // ── None ──
                <button
                    class=move || layer_opt_class(!state.viewmode.hfr_enabled().get(), false)
                    on:click=move |_| { clear_band(state); }
                >"None"</button>

                <hr/>
                // Dimmed like other unavailable menu items while the mic is
                // live (the tutorial ends on pressing Play).
                <button
                    class=move || if crate::components::hfr_tour::tour_available(state) {
                        "layer-panel-opt"
                    } else {
                        "layer-panel-opt disabled"
                    }
                    disabled=move || !crate::components::hfr_tour::tour_available(state)
                    on:click=move |_| crate::components::hfr_tour::start_tour(state)
                >{crate::components::hfr_tour::TOUR_NAME}</button>
            </SwitchButton>
    }
}

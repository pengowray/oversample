// XForm toggle — a switch button in the View bar. The top row turns XForm
// display-processing on or off; the bottom row opens its menu: a sentence
// saying what the spectrogram shows now, then a grid with a row for each
// Hear bar control (PASS filter, Notch filter, Noise reduction, Playback
// mode, Gain, Resampling) and a column for each choice (Off, Auto, As heard,
// Custom), then the custom rate.
//
// The rows use the Hear bar's names and the columns are whole words
// (2026-11-15; they were EQ / Notch / NR / Xform / Gain / Resam and
// off / aut / sam / cst). The "As heard" cell shows what the Hear bar is set
// to for that row, which replaced a column of dots.
//
// XForm used to be a separate `MainView::XformedSpec` that duplicated all the
// spectrogram settings. It is now an independent toggle layered over any
// spectro-like view (Spectrogram / Flow / Chromagram / Resonators): the
// spectrogram settings stay in the Spectrogram menu and apply to both.

use crate::components::layer_panel::toggle_panel;
use crate::state::store_fields::*;
use leptos::prelude::*;

use crate::components::app::{disable_xform, enable_xform};
use crate::components::display_filter_button::{DspFilterHeader, DspFilterRow};
use crate::components::switch_button::SwitchButton;
use crate::state::{AppState, DisplayFilterMode, GainMode, LayerPanel, PlaybackMode};

#[component]
pub fn XformButton() -> impl IntoView {
    let state = expect_context::<AppState>();

    let is_open =
        Signal::derive(move || state.panels.layer_panel_open().get() == Some(LayerPanel::Xform));
    let no_file = move || state.no_recording_open();
    // XForm only transforms the spectrogram, so it's inert on Wave / ZC views.
    let supported = move || state.viewmode.main_view().get().is_spectrogram();
    let enabled = Signal::derive(move || state.display.xform_enabled().get());

    // Toggle XForm on/off (no-op when no file or on a non-spectro view).
    let toggle_xform = move || {
        if no_file() || !supported() {
            return;
        }
        if state.display.xform_enabled().get_untracked() {
            disable_xform(&state);
        } else {
            enable_xform(&state);
        }
    };

    // Compact summary of the stages XForm applies, shown on the bottom row.
    // "Raw" while XForm is off or applies nothing.
    let value = Signal::derive(move || {
        let mut parts: Vec<&'static str> = Vec::new();
        if state.display.eq().get() {
            parts.push("EQ");
        }
        if state.display.noise_filter().get() {
            parts.push("NF");
        }
        if state.display.transform().get() {
            parts.push(match state.playback.mode().get() {
                PlaybackMode::Heterodyne => "HET",
                PlaybackMode::TimeExpansion => "TE",
                PlaybackMode::PitchShift => "PS",
                PlaybackMode::PhaseVocoder => "PV",
                PlaybackMode::ZeroCrossing => "ZC",
                PlaybackMode::Normal => "XF",
            });
        }
        if state.display.gain_boost().get().abs() >= 0.5 {
            parts.push("G");
        }
        if display_resample_rate(&state).is_some() {
            parts.push("RS");
        }
        if parts.is_empty() {
            "Raw".to_string()
        } else {
            parts.join("+")
        }
    });
    let toggle_menu = Callback::new(move |()| toggle_panel(&state, LayerPanel::Xform));
    let toggle = Callback::new(move |()| toggle_xform());
    let disabled = Signal::derive(move || no_file() || !supported());

    // ── What the Hear bar is set to for each row: its "As heard" cell ──
    let on_off = |on: bool| if on { "on" } else { "off" }.to_string();
    let pass_heard = Signal::derive(move || on_off(state.filter.enabled().get()));
    let notch_heard = Signal::derive(move || on_off(state.notch.enabled().get()));
    let nr_heard = Signal::derive(move || on_off(state.noise_reduce.enabled().get()));
    let mode_heard = Signal::derive(move || {
        match state.playback.mode().get() {
            PlaybackMode::Normal => "1:1",
            PlaybackMode::Heterodyne => "HET",
            PlaybackMode::TimeExpansion => "TE",
            PlaybackMode::PitchShift => "PS",
            PlaybackMode::PhaseVocoder => "PV",
            PlaybackMode::ZeroCrossing => "ZC",
        }
        .to_string()
    });
    let gain_heard = Signal::derive(move || on_off(state.gain.mode().get() != GainMode::Off));
    // "As heard" resamples to the rate the browser plays at.
    let resample_heard = Signal::derive(move || match state.display.browser_sample_rate().get() {
        0 => "off".to_string(),
        rate => crate::format_units::sample_rate_khz(rate),
    });

    let resam_tooltip = Signal::derive(move || {
        let bsr = state.display.browser_sample_rate().get();
        if bsr == 0 {
            return String::new();
        }
        let files = state.library.files().get();
        let idx = state.library.current_index().get();
        let file_rate = idx
            .and_then(|i| files.get(i))
            .map(|f| f.audio.sample_rate)
            .unwrap_or(0);
        if file_rate == 0 {
            return String::new();
        }
        let decim = state.display.decimate_effective().get();
        let effective = if decim > 0 && decim < file_rate {
            crate::dsp::filters::decimated_rate(file_rate, decim)
        } else {
            file_rate
        };
        if effective != bsr {
            format!("Browser resampling {}Hz to {}Hz output", effective, bsr)
        } else {
            format!("Output matches browser rate ({}Hz)", bsr)
        }
    });

    let show_decim_custom = Signal::derive(move || {
        state.display.xform_enabled().get()
            && state.display.filter_decimate().get() == DisplayFilterMode::Custom
    });

    // Dynamic blurb describing what the transformed spectrogram is showing,
    // composed from the *resolved* display_* signals so it tracks reality.
    let blurb = Signal::derive(move || {
        if !state.display.xform_enabled().get() {
            return "Off — the spectrogram shows the raw signal. Turn XForm on to \
                    view the signal as transformed for playback (heterodyne, gain, \
                    filtering, …)."
                .to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        if state.display.eq().get() {
            parts.push("bandpass/EQ".to_string());
        }
        if state.display.noise_filter().get() {
            parts.push("noise filtering".to_string());
        }
        if state.display.transform().get() {
            match state.playback.mode().get() {
                PlaybackMode::Heterodyne => {
                    let f = state.transform.het_frequency().get();
                    parts.push(format!("heterodyne @ {:.0} kHz", f / 1000.0));
                }
                // TE leaves the picture as it is and relabels the axes: see
                // the sentence added after the list.
                PlaybackMode::TimeExpansion => {}
                PlaybackMode::PitchShift => parts.push("pitch-shift".to_string()),
                PlaybackMode::PhaseVocoder => parts.push("phase-vocoder shift".to_string()),
                PlaybackMode::ZeroCrossing => parts.push("zero-crossing".to_string()),
                PlaybackMode::Normal => {}
            }
        }
        let boost = state.display.gain_boost().get();
        if boost.abs() >= 0.5 {
            parts.push(format!("{:+.0} dB gain", boost));
        }
        // With heard axes the rate is given as heard, like the axis labels.
        let heard = crate::heard_axis::heard_axis(&state);
        if let Some(rate) = display_resample_rate(&state) {
            let shown = rate as f64 * heard.map_or(1.0, |h| h.label_scale());
            parts.push(format!("resample to {:.0} kHz", shown / 1000.0));
        }
        let picture = if parts.is_empty() {
            None
        } else {
            Some(format!(
                "Spectrogram of the signal after {}.",
                join_clauses(&parts)
            ))
        };
        match (picture, heard) {
            (Some(picture), Some(heard)) => format!("{picture} {}", heard.blurb()),
            (None, Some(heard)) => format!("The recording's own spectrogram. {}", heard.blurb()),
            (Some(picture), None) => picture,
            (None, None) => "Showing the spectrogram of the unmodified signal — no processing \
                             stages are active. A row set to \"As heard\" uses that control's \
                             setting from the Hear bar."
                .to_string(),
        }
    });

    view! {
        <SwitchButton
            name="XForm"
            on=enabled
            toggle=toggle
            value=value
            is_open=is_open
            toggle_menu=toggle_menu
            disabled=disabled
            width="92px"
            switch_title="XForm: show the spectrogram of the signal as transformed for playback"
            menu_title="XForm settings"
            // A fixed width, like the Noise menu: nothing moves when a cell's
            // text changes.
            panel_style="width: min(340px, calc(100vw - 16px));"
        >
            <div class="xform-blurb">{move || blurb.get()}</div>
            <div class="layer-panel-title">"Show in the spectrogram"</div>
            <DspFilterHeader />
            <DspFilterRow label="PASS filter" signal=state.display.filter_eq() heard=pass_heard custom_available=false />
            <DspFilterRow label="Notch filter" signal=state.display.filter_notch() heard=notch_heard custom_available=false auto_available=false />
            <DspFilterRow label="Noise reduction" signal=state.display.filter_nr() heard=nr_heard custom_available=false />
            <DspFilterRow label="Playback mode" signal=state.display.filter_transform() heard=mode_heard custom_available=false auto_available=false />
            <DspFilterRow label="Gain" signal=state.display.filter_gain() heard=gain_heard custom_available=true />
            <DspFilterRow label="Resampling" signal=state.display.filter_decimate() heard=resample_heard custom_available=true heard_tooltip=resam_tooltip />

            // Display Gain Auto takes its peak from the shared "Peak from".
            {move || state.display.auto_gain().get().then(|| view! {
                <crate::components::hearing_bar::PeakSourceRow />
            })}

            // Custom resampling rate
            {move || show_decim_custom.get().then(|| {
                let rate = state.display.decimate_rate();
                let rates: [(u32, &str); 4] = [
                    (44100, "44.1k"),
                    (48000, "48k"),
                    (96000, "96k"),
                    (192000, "192k"),
                ];
                view! {
                    <div class="dsp-custom-section">
                        <div class="dsp-custom-title">"Custom resampling rate"</div>
                        <div class="dsp-filter-seg" style="justify-content: center; gap: 2px; padding: 2px 4px;">
                            {rates.into_iter().map(|(r, label)| {
                                view! {
                                    <button
                                        class=move || if rate.get() == r { "sel" } else { "" }
                                        on:click=move |_| rate.set(r)
                                    >{label}</button>
                                }
                            }).collect_view()}
                        </div>
                    </div>
                }
            })}
        </SwitchButton>
    }
}

/// Join clause fragments into "a", "a and b", or "a, b and c".
/// The rate the display is resampled to, in the recording's Hz, or `None`
/// when Resam is off or its target is at or above the file's rate (so
/// nothing is resampled). Tracked.
fn display_resample_rate(state: &AppState) -> Option<u32> {
    let decim = state.display.decimate_effective().get();
    let files = state.library.files().get();
    let idx = state.library.current_index().get();
    let file_rate = idx
        .and_then(|i| files.get(i))
        .map(|f| f.audio.sample_rate)?;
    (decim > 0 && decim < file_rate).then(|| crate::dsp::filters::decimated_rate(file_rate, decim))
}

fn join_clauses(parts: &[String]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        2 => format!("{} and {}", parts[0], parts[1]),
        _ => {
            let (last, head) = parts.split_last().unwrap();
            format!("{} and {}", head.join(", "), last)
        }
    }
}

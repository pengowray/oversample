// View button: the View bar's Spec / Wave / Dots buttons and the settings
// menu of the view in use.
//
// The spectrogram kinds (Spectrogram, Flow, Chromagram, Resonators) share the
// first button and are tabs at the top of its menu.

use crate::components::layer_panel::{layer_opt_class, toggle_panel};
use crate::state::store_fields::*;
use crate::state::{
    resonator_bw_to_slider, resonator_slider_to_bw, AppState, ChromaColormap, ChromaRange,
    ChromaSource, FftMode, FlowColorScheme, LayerPanel, MainView, ResonatorAlphaMode,
    ResonatorDensity, ResonatorFftMode, ResonatorHybridMode, ResonatorLayout, SpectrogramDisplay,
    WaveformView, RESONATOR_BW_SLIDER_MAX,
};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// Tooltip for the Waveform Gain row's Auto toggle. Auto gain follows the
/// peak source picked in the Hearing bar's Gain menu (`compute_auto_gain`).
fn wave_auto_title(auto_on: bool, source: crate::state::PeakSource) -> String {
    use crate::state::PeakSource;
    let peak_of = match source {
        PeakSource::FullWave => "the whole file",
        PeakSource::Selection => "the selection",
        PeakSource::First30s => "the first 30 s",
        PeakSource::Processed => "the selection (or the first 30 s) after playback processing",
    };
    if auto_on {
        format!(
            "Auto: Gain brings the peak of {peak_of} to \u{2212}3 dBFS, by up to +60 dB. Click to turn off."
        )
    } else {
        format!("Auto is off. Click to set Gain from the peak of {peak_of}.")
    }
}

/// Tooltip for the Range row's Auto toggle. `noise_floor_dbfs` is the
/// selected file's noise floor, when measured; `range_db` is the range shown.
fn range_auto_title(auto_on: bool, noise_floor_dbfs: Option<f32>, range_db: f32) -> String {
    match (auto_on, noise_floor_dbfs) {
        (true, Some(nf)) => format!(
            "Auto: Range reaches 10 dB below the noise floor ({} dBFS). Click to turn off.",
            format!("{:.0}", nf).replace('-', "\u{2212}")
        ),
        (true, None) => format!(
            "No measured noise floor, so Auto uses {:.0} dB. Click to turn Auto off.",
            range_db
        ),
        (false, _) => "Auto is off. Click to set Range from the file's noise floor.".to_string(),
    }
}

/// Current effective sample rate for the Resonators readouts — the live
/// waterfall's rate when mic is active, otherwise the current file's, with a
/// sensible default so the label stays populated with no file loaded.
fn resonator_quick_sample_rate(state: AppState) -> f64 {
    if crate::canvas::live_waterfall::is_active() {
        return crate::canvas::live_waterfall::max_freq() * 2.0;
    }
    let files = state.library.files().get();
    let idx = state.library.current_index().get();
    idx.and_then(|i| files.get(i))
        .map(|f| f.spectrogram.sample_rate as f64)
        .unwrap_or(192_000.0)
}

/// Class for an on/off toggle beside the value it sets, such as Range's
/// Auto. Styled by `.opt-toggle` in style.css.
fn opt_toggle_class(on: bool) -> &'static str {
    if on {
        "layer-panel-opt opt-toggle sel"
    } else {
        "layer-panel-opt opt-toggle"
    }
}

/// Split-button: click cycles Spec/Wave, down-arrow opens a dropdown with all
/// view modes + DSP settings. Rendered in the bottom toolbar.
#[component]
pub fn MainViewButton() -> impl IntoView {
    use crate::components::popup::{Align, PopupPanel, Side};
    let state = expect_context::<AppState>();
    let is_open =
        Signal::derive(move || state.panels.layer_panel_open().get() == Some(LayerPanel::MainView));
    let no_file = move || state.no_recording_open();

    // Helper: handle all side-effects of a view switch synchronously,
    // so the spectrogram render Effect always sees consistent state.
    let switch_view = move |new_view: MainView| {
        let old_view = state.viewmode.main_view().get_untracked();
        if new_view == old_view {
            return;
        }
        // XForm is an independent toggle now (see XformButton) — the view
        // switch only changes which renderer is shown. The xform_enabled
        // state persists across view changes; it's gated to spectro-like
        // views by the XForm button being inert elsewhere.
        state.viewmode.main_view().set(new_view);
    };

    // The View bar has three buttons: the spectrogram kinds (Spectrogram,
    // Flow, Chromagram, Resonators) share the first one, then Wave and Dots.
    // `spec_kind` is the kind the first button names and switches to: the
    // current view when it is a spectrogram kind, otherwise the kind shown
    // most recently.
    Effect::new(move |_| {
        let view = state.viewmode.main_view().get();
        if view.is_spectrogram() {
            state.viewmode.last_spectrogram_kind().set(view);
        }
    });
    let spec_kind = Signal::derive(move || {
        let view = state.viewmode.main_view().get();
        if view.is_spectrogram() {
            return view;
        }
        let last = state.viewmode.last_spectrogram_kind().get();
        if state.current_is_zc() && !last.is_sensible_for_zc() {
            MainView::Spectrogram
        } else {
            last
        }
    });
    let slot_selected = move |slot: MainView| {
        let cur = state.viewmode.main_view().get();
        if slot == MainView::Spectrogram {
            cur.is_spectrogram()
        } else {
            cur == slot
        }
    };

    // Click on a non-selected button: switch to its view. Click on the
    // selected button: toggle its settings popup. Mirrors the playback
    // mode radio group's "click again to open settings".
    let select_slot = move |slot: MainView| {
        if no_file() {
            return;
        }
        if slot_selected(slot) {
            toggle_panel(&state, LayerPanel::MainView);
            return;
        }
        switch_view(if slot == MainView::Spectrogram {
            spec_kind.get_untracked()
        } else {
            slot
        });
        state.panels.layer_panel_open().set(None);
    };

    let row_ref = NodeRef::<leptos::html::Div>::new();

    // Per-button class (radio-group look, reused from .mode-radio-*).
    let view_btn_class = move |slot: MainView| {
        Signal::derive(move || {
            let is_sel = slot_selected(slot);
            let mut s = String::from("layer-btn mode-radio-btn");
            if is_sel {
                s.push_str(" selected has-settings");
                if is_open.get() {
                    s.push_str(" open");
                }
            }
            if no_file() {
                s.push_str(" disabled");
            }
            s
        })
    };

    view! {
        // Reuses .mode-radio-group styling for visual consistency with the
        // playback Mode radio group on the Hearing Bar.
        <div node_ref=row_ref class="mode-radio-group view-radio-group"
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:touchstart=|ev: web_sys::TouchEvent| ev.stop_propagation()
        >
            {[MainView::Spectrogram, MainView::Waveform, MainView::ZcChart].into_iter().map(|slot| {
                let class_sig = view_btn_class(slot);
                // The first button names the spectrogram kind it shows.
                let shown = move || if slot == MainView::Spectrogram { spec_kind.get() } else { slot };
                view! {
                    <button class=move || class_sig.get()
                        title=move || shown().label()
                        on:click=move |_: web_sys::MouseEvent| select_slot(slot)
                    >
                        <span class="mode-radio-label">{move || shown().short_label()}</span>
                        <span class="mode-radio-corner">{"\u{25E2}"}</span>
                    </button>
                }
            }).collect_view()}

            <PopupPanel
                is_open=is_open
                anchor=row_ref
                preferred_side=Side::Below
                preferred_align=Align::Start
                extra_style="min-width: 240px;"
            >
            // Spectrogram kinds as tabs. A tab switches the view and leaves
            // the menu open, so the settings under the tabs change in place.
            // A plain title instead in Wave and Dots, and for ZC files, which
            // offer one kind: they store dot-plot data, and the other kinds'
            // DSP would measure the synthesised waveform, not the recording.
            {move || {
                let cur = state.viewmode.main_view().get();
                let is_zc = state.current_is_zc();
                let tabs: Vec<MainView> = MainView::SPECTROGRAM_KINDS
                    .iter()
                    .copied()
                    .filter(|m| !is_zc || m.is_sensible_for_zc())
                    .collect();
                if cur.is_spectrogram() && tabs.len() > 1 {
                    view! {
                        <div class="view-kind-tabs" role="tablist">
                            {tabs.into_iter().map(|kind| view! {
                                <button
                                    role="tab"
                                    aria-selected=(cur == kind).to_string()
                                    class=if cur == kind { "view-kind-tab sel" } else { "view-kind-tab" }
                                    on:click=move |_: web_sys::MouseEvent| switch_view(kind)
                                >{kind.label()}</button>
                            }).collect_view()}
                        </div>
                    }.into_any()
                } else {
                    view! { <div class="layer-panel-title">{cur.label()}</div> }.into_any()
                }
            }}

            // Waveform sub-view (when Waveform is the active main view)
            {move || (state.viewmode.main_view().get() == MainView::Waveform).then(|| {
                view! {
                    <hr />
                    <div class="layer-panel-title">"Waveform Mode"</div>
                    {WaveformView::ALL.iter().map(|&wv| {
                        view! {
                            <button
                                class=move || layer_opt_class(state.viewmode.waveform_view().get() == wv)
                                on:click=move |_: web_sys::MouseEvent| {
                                    state.viewmode.waveform_view().set(wv);
                                }
                            >
                                {wv.label()}
                            </button>
                        }
                    }).collect_view()}
                }
            })}

            // Loudest-frequency line checkbox (spectrogram view), drawn with the
            // reassignment method (canvas::reassign_line)
            {move || (state.viewmode.main_view().get() == MainView::Spectrogram).then(|| {
                view! {
                    <hr />
                    <label style="display:flex;align-items:center;gap:4px;cursor:pointer;padding:4px 8px;font-size:12px;"
                        title="Draws a line over the spectrogram at the loudest frequency in each column, where that sound is 15 dB or more above the background noise (a steady hum or tone counts as background).\nThe line's position is calculated with the reassignment method, more precisely than the spectrogram's pixel grid.\nAt sample rates of 96 kHz or higher, sounds below 15 kHz are ignored (wind, insects, handling noise).\nHidden when zoomed far out.">
                        <input
                            type="checkbox"
                            prop:checked=move || state.spect.loudest_freq_line().get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.spect.loudest_freq_line().set(input.checked());
                            }
                        />
                        "Loudest-frequency line"
                    </label>
                }
            })}

            // Resonator quick controls (shown only when Resonators view is active)
            {move || (state.viewmode.main_view().get() == MainView::Resonators).then(|| {
                view! {
                    <div class="dsp-custom-section">
                        <div class="dsp-custom-title">{move || {
                            let bw = state.resonator.bandwidth_hz().get().max(0.001);
                            let tau_ms = 1000.0 / (2.0 * std::f32::consts::PI * bw);
                            let bw_str = if bw < 10.0 {
                                format!("{:.1}", bw)
                            } else {
                                format!("{:.0}", bw.round())
                            };
                            format!("Bandwidth: {} Hz (\u{03c4} \u{2248} {:.1} ms)", bw_str, tau_ms)
                        }}</div>
                        <div class="dsp-custom-slider-row">
                            // Log-scale slider: 0..RESONATOR_BW_SLIDER_MAX ↦ 1..2000 Hz.
                            <input
                                type="range"
                                class="setting-range"
                                min="0"
                                max=RESONATOR_BW_SLIDER_MAX.to_string()
                                step="1"
                                prop:value=move || {
                                    resonator_bw_to_slider(state.resonator.bandwidth_hz().get())
                                        .round()
                                        .to_string()
                                }
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(pos) = input.value().parse::<f32>() {
                                        state.resonator.bandwidth_hz().set(resonator_slider_to_bw(pos));
                                    }
                                }
                                on:dblclick=move |_| state.resonator.bandwidth_hz().set(20.0)
                            />
                        </div>
                    </div>
                    <div class="setting-row" style="padding: 4px 8px;">
                        <span class="setting-label">{move || {
                            let mode = state.resonator.fft_mode().get();
                            let sr = resonator_quick_sample_rate(state);
                            let current_lod = crate::canvas::tile_cache::select_lod(
                                state.view.zoom_level().get(),
                            );
                            let f = mode.fft_for_lod(current_lod).max(2);
                            let spacing = sr / f as f64;
                            let spacing_str = if spacing >= 1000.0 {
                                format!("{:.2} kHz", spacing / 1000.0)
                            } else if spacing >= 100.0 {
                                format!("{:.0} Hz", spacing)
                            } else {
                                format!("{:.1} Hz", spacing)
                            };
                            format!("Bins ({}/bin)", spacing_str)
                        }}</span>
                        <select
                            class="setting-select"
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let select: web_sys::HtmlSelectElement = target.unchecked_into();
                                let v = select.value();
                                let new_mode = match v.as_str() {
                                    "adaptive" => ResonatorFftMode::Adaptive(ResonatorDensity::Full),
                                    "adaptive-50" => ResonatorFftMode::Adaptive(ResonatorDensity::Half),
                                    "adaptive-25" => ResonatorFftMode::Adaptive(ResonatorDensity::Quarter),
                                    _ => match v.parse::<usize>() {
                                        Ok(sz) => ResonatorFftMode::Single(sz.max(16)),
                                        Err(_) => return,
                                    },
                                };
                                state.resonator.fft_mode().set(new_mode);
                            }
                            prop:value=move || match state.resonator.fft_mode().get() {
                                ResonatorFftMode::Adaptive(d) => d.as_value().to_string(),
                                ResonatorFftMode::Single(sz) => sz.to_string(),
                            }
                        >
                            <option value="adaptive">"Adaptive 100%"</option>
                            <option value="adaptive-50">"Adaptive 50%"</option>
                            <option value="adaptive-25">"Adaptive 25%"</option>
                            <option value="64">"33"</option>
                            <option value="128">"65"</option>
                            <option value="256">"129"</option>
                            <option value="512">"257"</option>
                            <option value="1024">"513"</option>
                        </select>
                    </div>
                    <div class="setting-row" style="padding: 4px 8px;">
                        <span class="setting-label">"Layout"</span>
                        <select
                            class="setting-select"
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let select: web_sys::HtmlSelectElement = target.unchecked_into();
                                let new_layout = match select.value().as_str() {
                                    "log" => ResonatorLayout::Log,
                                    _ => ResonatorLayout::Linear,
                                };
                                state.resonator.layout().set(new_layout);
                            }
                            prop:value=move || match state.resonator.layout().get() {
                                ResonatorLayout::Linear => "linear",
                                ResonatorLayout::Log => "log",
                            }
                        >
                            <option value="linear">"Linear"</option>
                            <option value="log">"Log"</option>
                        </select>
                    </div>
                    <div class="setting-row" style="padding: 4px 8px;">
                        <span class="setting-label"
                            title="Const bandwidth: one bandwidth for every bin (uniform; uses the Bandwidth slider above). Const Q: bw = f / Q — sharp at low freq, fast at high freq (suits FM bat calls; uses the Q slider).">
                            "Alpha"
                        </span>
                        <select
                            class="setting-select"
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let select: web_sys::HtmlSelectElement = target.unchecked_into();
                                let new_mode = match select.value().as_str() {
                                    "constq" => ResonatorAlphaMode::ConstQ,
                                    _ => ResonatorAlphaMode::ConstBandwidth,
                                };
                                state.resonator.alpha_mode().set(new_mode);
                            }
                            prop:value=move || match state.resonator.alpha_mode().get() {
                                ResonatorAlphaMode::ConstBandwidth => "constbw",
                                ResonatorAlphaMode::ConstQ => "constq",
                            }
                        >
                            <option value="constbw">"Const bandwidth"</option>
                            <option value="constq">"Const Q"</option>
                        </select>
                    </div>
                    <div class="dsp-custom-section">
                        <div class="dsp-custom-title">{move || {
                            let q = state.resonator.q().get().max(0.1);
                            format!("Q: {:.0} (bw \u{2248} {:.0} Hz @ 40 kHz)", q, 40_000.0 / q)
                        }}</div>
                        <div class="dsp-custom-slider-row">
                            // Linear Q slider; only affects the Const Q schedule.
                            <input
                                type="range"
                                class="setting-range"
                                min="10"
                                max="1000"
                                step="5"
                                prop:value=move || state.resonator.q().get().round().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        state.resonator.q().set(v.max(1.0));
                                    }
                                }
                                on:dblclick=move |_| state.resonator.q().set(200.0)
                            />
                        </div>
                    </div>
                    <div class="setting-row" style="padding: 4px 8px;">
                        <span class="setting-label"
                            title="FFT-steered hybrid (files & live). Clean: gate the resonator with an FFT to remove its leakage skirts (crisp line, clean background). Adaptive: blend a sharp + fast bank by FFT tonalness — sharp on sustained tones, crisp on transients (≈2× cost live). Off: plain resonator.">
                            "Hybrid"
                        </span>
                        <select
                            class="setting-select"
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let select: web_sys::HtmlSelectElement = target.unchecked_into();
                                let m = match select.value().as_str() {
                                    "clean" => ResonatorHybridMode::Clean,
                                    "adaptive" => ResonatorHybridMode::Adaptive,
                                    _ => ResonatorHybridMode::Off,
                                };
                                state.resonator.hybrid_mode().set(m);
                            }
                            prop:value=move || match state.resonator.hybrid_mode().get() {
                                ResonatorHybridMode::Off => "off",
                                ResonatorHybridMode::Clean => "clean",
                                ResonatorHybridMode::Adaptive => "adaptive",
                            }
                        >
                            <option value="off">"Off"</option>
                            <option value="clean">"Clean (de-leak)"</option>
                            <option value="adaptive">"Adaptive (sharp+fast)"</option>
                        </select>
                    </div>
                    <label style="display:flex;align-items:center;gap:4px;cursor:pointer;padding:4px 8px;font-size:12px;"
                        title="Concentrate all bins on the visible freq range for finer vertical zoom. Rebuilds ~0.5s after you stop zooming vertically.">
                        <input
                            type="checkbox"
                            prop:checked=move || state.resonator.viewport_bins().get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.resonator.viewport_bins().set(input.checked());
                            }
                        />
                        "Viewport zoom"
                    </label>
                    <label style="display:flex;align-items:center;gap:4px;cursor:pointer;padding:4px 8px;font-size:12px;"
                        title="Live recording/listening only: automatically lower the resonator density when the device can't keep up (and raise it back when it can), capped at the density chosen above. Keeps the live waterfall smooth at high sample rates.">
                        <input
                            type="checkbox"
                            prop:checked=move || state.resonator.auto_quality().get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.resonator.auto_quality().set(input.checked());
                            }
                        />
                        "Auto quality (fps, live)"
                    </label>
                }
            })}

            // FFT size selector (when any spectrogram/flow view is active)
            {move || matches!(state.viewmode.main_view().get(), MainView::Spectrogram | MainView::Flow).then(|| {
                view! {
                    <hr />
                    <div class="layer-panel-title">"FFT Size"</div>
                    <select
                        class="setting-select"
                        style="margin: 4px 8px; width: calc(100% - 16px);"
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let select: web_sys::HtmlSelectElement = target.unchecked_into();
                            let val = select.value();
                            let mode = match val.as_str() {
                                "ax" => FftMode::AdaptiveXS,
                                "as" => FftMode::AdaptiveS,
                                "am" => FftMode::AdaptiveM,
                                "al" => FftMode::AdaptiveL,
                                _ => {
                                    if let Ok(v) = val.parse::<usize>() {
                                        FftMode::Single(v)
                                    } else {
                                        return;
                                    }
                                }
                            };
                            state.spect.fft_mode().set(mode);
                        }
                    >
                        {move || {
                            let current = state.spect.fft_mode().get();
                            let options: [(&str, &str); 13] = [
                                ("ax", "Adaptive XS"),
                                ("as", "Adaptive S"),
                                ("am", "Adaptive M"),
                                ("al", "Adaptive L"),
                                ("32", "32"),
                                ("64", "64"),
                                ("128", "128"),
                                ("256", "256"),
                                ("512", "512"),
                                ("1024", "1024"),
                                ("2048", "2048"),
                                ("4096", "4096"),
                                ("8192", "8192"),
                            ];
                            options.into_iter().map(|(value, label)| {
                                let is_selected = match (value, current) {
                                    ("ax", FftMode::AdaptiveXS) => true,
                                    ("as", FftMode::AdaptiveS) => true,
                                    ("am", FftMode::AdaptiveM) => true,
                                    ("al", FftMode::AdaptiveL) => true,
                                    (v, FftMode::Single(sz)) => v.parse::<usize>().ok() == Some(sz),
                                    _ => false,
                                };
                                let v = value.to_string();
                                let l = label.to_string();
                                view! { <option value={v} selected=move || is_selected>{l}</option> }
                            }).collect::<Vec<_>>()
                        }}
                    </select>
                }
            })}

            // Frequency range selector (for spectrogram/flow/resonator views)
            {move || matches!(state.viewmode.main_view().get(), MainView::Spectrogram | MainView::Flow | MainView::Resonators).then(|| {
                let file_max = move || {
                    let files = state.library.files().get();
                    let idx = state.library.current_index().get();
                    idx.and_then(|i| files.get(i))
                        .map(|f| f.spectrogram.max_freq)
                        .unwrap_or(96_000.0)
                };
                let is_range = move |lo: Option<f64>, hi: Option<f64>| -> bool {
                    let cur_min = state.view.min_display_freq().get();
                    let cur_max = state.view.max_display_freq().get();
                    match (lo, hi) {
                        (None, None) => {
                            let fm = file_max();
                            (cur_min.is_none() || cur_min == Some(0.0))
                                && (cur_max.is_none() || cur_max.is_some_and(|m| (m - fm).abs() < 100.0))
                        }
                        (_, Some(h)) => cur_max.is_some_and(|m| (m - h).abs() < 100.0)
                            && (lo.is_none() || cur_min.is_none() || cur_min == lo),
                        _ => false,
                    }
                };
                view! {
                    <hr />
                    <div class="layer-panel-title">"Freq Range"</div>
                    <select
                        class="setting-select"
                        style="margin: 4px 8px; width: calc(100% - 16px);"
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let select: web_sys::HtmlSelectElement = target.unchecked_into();
                            let val = select.value();
                            match val.as_str() {
                                "full" => {
                                    state.view.min_display_freq().set(None);
                                    state.view.max_display_freq().set(None);
                                }
                                "20k" => {
                                    state.view.min_display_freq().set(Some(0.0));
                                    state.view.max_display_freq().set(Some(crate::dsp::ultrasonic::HEARING_LIMIT_HZ));
                                }
                                "50k" => {
                                    state.view.min_display_freq().set(Some(0.0));
                                    state.view.max_display_freq().set(Some(50_000.0));
                                }
                                "100k" => {
                                    state.view.min_display_freq().set(Some(0.0));
                                    state.view.max_display_freq().set(Some(100_000.0));
                                }
                                "192k" => {
                                    state.view.min_display_freq().set(Some(0.0));
                                    state.view.max_display_freq().set(Some(192_000.0));
                                }
                                _ => {}
                            }
                        }
                    >
                        {move || {
                            let options: [(&str, &str, Option<f64>, Option<f64>); 5] = [
                                ("full", "Full", None, None),
                                ("20k", "0 \u{2013} 20 kHz", Some(0.0), Some(crate::dsp::ultrasonic::HEARING_LIMIT_HZ)),
                                ("50k", "0 \u{2013} 50 kHz", Some(0.0), Some(50_000.0)),
                                ("100k", "0 \u{2013} 100 kHz", Some(0.0), Some(100_000.0)),
                                ("192k", "0 \u{2013} 192 kHz", Some(0.0), Some(192_000.0)),
                            ];
                            options.into_iter().map(|(value, label, lo, hi)| {
                                let selected = is_range(lo, hi);
                                let v = value.to_string();
                                let l = label.to_string();
                                view! { <option value={v} selected=move || selected>{l}</option> }
                            }).collect::<Vec<_>>()
                        }}
                    </select>
                }
            })}

            // Intensity sliders (for Spectrogram, Flow, or Resonators). These
            // are spectrogram settings — they also drive the XForm view, which
            // shares the Spectrogram menu rather than duplicating these.
            {move || matches!(state.viewmode.main_view().get(), MainView::Spectrogram | MainView::Flow | MainView::Resonators).then(|| {
                let gain_get = move || state.spect.gain_db().get();
                let gain_set = move |v: f32| state.spect.gain_db().set(v);
                // The range in use for the selected file (see spect_floor_range).
                let range_get = move || state.spect_floor_range().1;
                let range_set = move |v: f32| state.spect.range_db().set(v);
                let floor_set = move |v: f32| state.spect.floor_db().set(v);
                let auto_range = move || state.spect.auto_range().get();
                let auto_range_set = move |on: bool| state.spect.auto_range().set(on);
                // The Range slider reaches the selected file's bit-depth limit:
                // 120 dB for 16-bit audio, 170 dB for 24-bit, 32-bit and float.
                let range_max = move || state.spect_max_range_db();
                let gamma_get = move || state.spect.gamma().get();
                let gamma_set = move |v: f32| state.spect.gamma().set(v);
                view! {
                    <hr />
                    <div class="dsp-custom-section">
                        <div class="dsp-custom-title">"Intensity"</div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Gain"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="-40" max="40" step="1"
                                prop:value=move || gain_get().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        gain_set(v);
                                        state.display.auto_gain().set(false);
                                    }
                                }
                                on:dblclick=move |_| gain_set(0.0)
                            />
                            <span class="dsp-custom-value">{move || format!("{:+.0} dB", gain_get())}</span>
                        </div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Range"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="20" step="5"
                                prop:max=move || range_max().to_string()
                                prop:value=move || range_get().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        range_set(v);
                                        floor_set(-v);
                                        auto_range_set(false);
                                    }
                                }
                                on:dblclick=move |_| auto_range_set(true)
                            />
                            <span class="dsp-custom-value">{move || format!("{:.0} dB", range_get())}</span>
                            // Beside the value it sets; range_auto_title says
                            // what Auto does.
                            <button
                                class=move || opt_toggle_class(auto_range())
                                aria-pressed=move || auto_range().to_string()
                                title=move || range_auto_title(
                                    auto_range(),
                                    state.current_noise_floor_dbfs(),
                                    range_get(),
                                )
                                on:click=move |_| {
                                    if auto_range() {
                                        // Keep the range Auto chose, so turning
                                        // Auto off doesn't change the picture.
                                        let r = range_get();
                                        range_set(r);
                                        floor_set(-r);
                                        auto_range_set(false);
                                    } else {
                                        auto_range_set(true);
                                    }
                                }
                            >"Auto"</button>
                        </div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Contrast"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="0.2" max="3.0" step="0.05"
                                prop:value=move || gamma_get().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        gamma_set(v);
                                    }
                                }
                                on:dblclick=move |_| gamma_set(1.0)
                            />
                            <span class="dsp-custom-value">{move || {
                                let g = gamma_get();
                                if g == 1.0 { "linear".to_string() } else { format!("{:.2}", g) }
                            }}</span>
                        </div>
                        <div style="text-align: right; padding-top: 4px;">
                            <button
                                class="layer-panel-opt opt-small-action"
                                on:click=move |_| {
                                    gain_set(0.0);
                                    floor_set(-120.0);
                                    range_set(120.0);
                                    auto_range_set(true);
                                    gamma_set(1.0);
                                    state.display.auto_gain().set(false);
                                }
                            >"Reset"</button>
                        </div>
                    </div>
                }
            })}

            // Waveform view gain (when Waveform is active)
            {move || (state.viewmode.main_view().get() == MainView::Waveform).then(|| {
                let wave_gain_db = move || if state.gain.wave_view_auto().get() {
                    state.compute_auto_gain()
                } else {
                    state.gain.wave_view_db().get()
                };
                view! {
                    <hr />
                    <div class="dsp-custom-section">
                        <div class="dsp-custom-title">"Waveform Gain"</div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Gain"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="-12" max="60" step="1"
                                prop:value=move || wave_gain_db().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f64>() {
                                        state.gain.wave_view_db().set(v);
                                        state.gain.wave_view_auto().set(false);
                                    }
                                }
                                on:dblclick=move |_| {
                                    state.gain.wave_view_db().set(0.0);
                                    state.gain.wave_view_auto().set(false);
                                }
                            />
                            <span class="dsp-custom-value">{move || format!("{:+.0} dB", wave_gain_db())}</span>
                            <button
                                class=move || opt_toggle_class(state.gain.wave_view_auto().get())
                                aria-pressed=move || state.gain.wave_view_auto().get().to_string()
                                title=move || wave_auto_title(
                                    state.gain.wave_view_auto().get(),
                                    state.gain.peak_source().get(),
                                )
                                on:click=move |_| {
                                    let auto = state.gain.wave_view_auto();
                                    if auto.get_untracked() {
                                        // Keep the gain Auto chose, so turning Auto
                                        // off doesn't change the waveform.
                                        state.gain.wave_view_db().set(state.compute_auto_gain_untracked());
                                        auto.set(false);
                                    } else {
                                        auto.set(true);
                                    }
                                }
                            >"Auto"</button>
                        </div>
                        {move || state.gain.wave_view_auto().get().then(|| view! {
                            <crate::components::hearing_bar::PeakSourceRow />
                        })}
                    </div>
                }
            })}

            // Flow algorithm options (when Flow is active)
            {move || (state.viewmode.main_view().get() == MainView::Flow).then(|| {
                view! {
                    <hr />
                    <div class="layer-panel-title">"Algorithm"</div>
                    <button
                        class=move || layer_opt_class(state.spect.display().get() == SpectrogramDisplay::FlowOptical)
                        on:click=move |_| state.spect.display().set(SpectrogramDisplay::FlowOptical)
                    >"Optical"</button>
                    <button
                        class=move || layer_opt_class(state.spect.display().get() == SpectrogramDisplay::PhaseCoherence)
                        on:click=move |_| state.spect.display().set(SpectrogramDisplay::PhaseCoherence)
                    >"Phase Coherence"</button>
                    <button
                        class=move || layer_opt_class(state.spect.display().get() == SpectrogramDisplay::FlowCentroid)
                        on:click=move |_| state.spect.display().set(SpectrogramDisplay::FlowCentroid)
                    >"Centroid"</button>
                    <button
                        class=move || layer_opt_class(state.spect.display().get() == SpectrogramDisplay::FlowGradient)
                        on:click=move |_| state.spect.display().set(SpectrogramDisplay::FlowGradient)
                    >"Gradient"</button>
                    <button
                        class=move || layer_opt_class(state.spect.display().get() == SpectrogramDisplay::Phase)
                        on:click=move |_| state.spect.display().set(SpectrogramDisplay::Phase)
                    >"Phase"</button>

                    // Color scheme (only for non-phase flow algorithms)
                    {move || {
                        let display = state.spect.display().get();
                        matches!(display,
                            SpectrogramDisplay::FlowOptical |
                            SpectrogramDisplay::FlowCentroid |
                            SpectrogramDisplay::FlowGradient
                        ).then(|| view! {
                            <hr />
                            <div class="layer-panel-title">"Color Scheme"</div>
                            <select
                                class="setting-select"
                                style="margin: 4px 8px; width: calc(100% - 16px);"
                                on:change=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let select: web_sys::HtmlSelectElement = target.unchecked_into();
                                    let scheme = match select.value().as_str() {
                                        "coolwarm" => FlowColorScheme::CoolWarm,
                                        "tealorange" => FlowColorScheme::TealOrange,
                                        "purplegreen" => FlowColorScheme::PurpleGreen,
                                        "spectral" => FlowColorScheme::Spectral,
                                        _ => FlowColorScheme::RedBlue,
                                    };
                                    state.flow.color_scheme().set(scheme);
                                }
                                prop:value=move || match state.flow.color_scheme().get() {
                                    FlowColorScheme::RedBlue => "redblue",
                                    FlowColorScheme::CoolWarm => "coolwarm",
                                    FlowColorScheme::TealOrange => "tealorange",
                                    FlowColorScheme::PurpleGreen => "purplegreen",
                                    FlowColorScheme::Spectral => "spectral",
                                }
                            >
                                <option value="redblue">"Red-Blue"</option>
                                <option value="coolwarm">"Cool-Warm"</option>
                                <option value="tealorange">"Teal-Orange"</option>
                                <option value="purplegreen">"Purple-Green"</option>
                                <option value="spectral">"Spectral"</option>
                            </select>
                        })
                    }}

                    // Flow-specific sliders
                    <hr />
                    <div class="dsp-custom-section">
                        <div class="dsp-custom-title">"Flow Controls"</div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Intensity gate"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="0" max="100" step="1"
                                prop:value=move || (state.flow.intensity_gate().get() * 100.0).round().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        state.flow.intensity_gate().set(v / 100.0);
                                    }
                                }
                                on:dblclick=move |_| state.flow.intensity_gate().set(0.0)
                            />
                            <span class="dsp-custom-value">{move || format!("{}%", (state.flow.intensity_gate().get() * 100.0).round() as u32)}</span>
                        </div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Flow gate"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="0" max="100" step="1"
                                prop:value=move || (state.flow.gate().get() * 100.0).round().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        state.flow.gate().set(v / 100.0);
                                    }
                                }
                                on:dblclick=move |_| state.flow.gate().set(0.0)
                            />
                            <span class="dsp-custom-value">{move || format!("{}%", (state.flow.gate().get() * 100.0).round() as u32)}</span>
                        </div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Color gain"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="0.5" max="10.0" step="0.5"
                                prop:value=move || state.flow.shift_gain().get().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        state.flow.shift_gain().set(v);
                                    }
                                }
                                on:dblclick=move |_| state.flow.shift_gain().set(3.0)
                            />
                            <span class="dsp-custom-value">{move || format!("{:.1}x", state.flow.shift_gain().get())}</span>
                        </div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Contrast"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="0.2" max="3.0" step="0.05"
                                prop:value=move || state.flow.color_gamma().get().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        state.flow.color_gamma().set(v);
                                    }
                                }
                                on:dblclick=move |_| state.flow.color_gamma().set(1.0)
                            />
                            <span class="dsp-custom-value">{move || {
                                let g = state.flow.color_gamma().get();
                                if g == 1.0 { "linear".to_string() } else { format!("{:.2}", g) }
                            }}</span>
                        </div>
                    </div>
                }
            })}

            // Chromagram options (when Chromagram is active)
            {move || (state.viewmode.main_view().get() == MainView::Chromagram).then(|| {
                view! {
                    <hr />
                    <div class="layer-panel-title">"Colormap"</div>
                    {ChromaColormap::ALL.iter().map(|&mode| {
                        view! {
                            <button
                                class=move || layer_opt_class(state.chroma.colormap().get() == mode)
                                on:click=move |_| state.chroma.colormap().set(mode)
                            >
                                {mode.label()}
                            </button>
                        }
                    }).collect_view()}

                    <hr />
                    <div class="layer-panel-title">"Freq Range"</div>
                    {ChromaRange::ALL.iter().map(|&range| {
                        view! {
                            <button
                                class=move || layer_opt_class(state.chroma.range().get() == range)
                                on:click=move |_| state.chroma.range().set(range)
                            >
                                {range.label()}
                            </button>
                        }
                    }).collect_view()}

                    <hr />
                    <div class="layer-panel-title">"Source"</div>
                    {ChromaSource::ALL.iter().map(|&src| {
                        view! {
                            <button
                                class=move || layer_opt_class(state.chroma.source().get() == src)
                                on:click=move |_| state.chroma.source().set(src)
                            >
                                {src.label()}
                            </button>
                        }
                    }).collect_view()}

                    <hr />
                    <div class="dsp-custom-section">
                        <div class="dsp-custom-title">"Chromagram Controls"</div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Gain"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="-20" max="60" step="1"
                                prop:value=move || state.chroma.gain().get().round().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        state.chroma.gain().set(v);
                                    }
                                }
                                on:dblclick=move |_| state.chroma.gain().set(0.0)
                            />
                            <span class="dsp-custom-value">{move || {
                                let db = state.chroma.gain().get();
                                if db == 0.0 { "0 dB".to_string() } else { format!("{:+.0} dB", db) }
                            }}</span>
                        </div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Contrast"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="0.2" max="3.0" step="0.05"
                                prop:value=move || state.chroma.gamma().get().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        state.chroma.gamma().set(v);
                                    }
                                }
                                on:dblclick=move |_| state.chroma.gamma().set(1.0)
                            />
                            <span class="dsp-custom-value">{move || {
                                let g = state.chroma.gamma().get();
                                if g == 1.0 { "linear".to_string() } else { format!("{:.2}", g) }
                            }}</span>
                        </div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Adapt"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="0" max="100" step="1"
                                prop:value=move || (state.chroma.adapt().get() * 100.0).round().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        state.chroma.adapt().set((v / 100.0).clamp(0.0, 1.0));
                                    }
                                }
                                on:dblclick=move |_| state.chroma.adapt().set(0.0)
                            />
                            <span class="dsp-custom-value">{move || {
                                let pct = (state.chroma.adapt().get() * 100.0).round() as i32;
                                if pct == 0 { "off".to_string() } else { format!("{}%", pct) }
                            }}</span>
                        </div>
                        <div class="dsp-custom-slider-row">
                            <span class="dsp-slider-label">"Floor"</span>
                            <input
                                type="range"
                                class="setting-range"
                                min="-80" max="0" step="1"
                                prop:value=move || state.chroma.floor_db().get().round().to_string()
                                on:input=move |ev: web_sys::Event| {
                                    let target = ev.target().unwrap();
                                    let input: web_sys::HtmlInputElement = target.unchecked_into();
                                    if let Ok(v) = input.value().parse::<f32>() {
                                        state.chroma.floor_db().set(v.clamp(-80.0, 0.0));
                                    }
                                }
                                on:dblclick=move |_| state.chroma.floor_db().set(-80.0)
                            />
                            <span class="dsp-custom-value">{move || {
                                let db = state.chroma.floor_db().get().round() as i32;
                                if db <= -80 { "off".to_string() } else { format!("{} dB", db) }
                            }}</span>
                        </div>
                    </div>
                }
            })}
            </PopupPanel>
        </div>
    }
}

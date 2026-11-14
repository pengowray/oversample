// Noise button: the Hearing Bar's one control for the noise filters.
//
// Noise reduction (spectral subtraction) and the notch filter used to be two
// split buttons. They are now one switch button: its top row turns noise
// filtering on or off, its bottom row names the filters in use, and its menu
// has a section per filter, each with its own switch and settings. A new
// noise filter gets a section here, not another button in the bar.
//
// Profile management (save/load Tauri presets) was dropped earlier; the
// Tauri-side commands are still there if it's reinstated.

use crate::components::layer_panel::toggle_panel;
use crate::state::store_fields::*;
use leptos::prelude::*;
use std::sync::Arc;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::audio::source::ChannelView;
use crate::components::switch_button::{SwitchButton, SwitchRow};
use crate::dsp::notch::{self, DetectionConfig};
use crate::state::{AppState, LayerPanel};

/// Notch detection threshold (prominence, as a ratio) until the Sensitivity
/// slider is moved.
const DEFAULT_SENSITIVITY: f64 = 6.0;

async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback(&resolve);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Detect the notch filter's noise bands, with `threshold` as the prominence
/// a band needs. Turns the notch filter on when it finds any.
fn run_detect(state: AppState, threshold: f64) {
    // While live, detect from the circular buffer so the bands reflect
    // current ambient noise rather than stale file content.
    let is_live = state.mic.listening().get_untracked() || state.mic.recording().get_untracked();
    if is_live {
        let is_tauri = state.is_tauri;
        let sample_rate = state.mic.sample_rate().get_untracked();
        let snapshot = crate::audio::mic_backend::with_live_samples(is_tauri, |s| s.to_vec());
        if sample_rate == 0 || snapshot.is_empty() {
            state.show_error_toast("Not enough live audio yet");
            return;
        }
        state.notch.detecting().set(true);
        let samples = Arc::new(snapshot);
        spawn_local(async move {
            yield_to_browser().await;
            let duration = samples.len() as f64 / sample_rate as f64;
            let config = DetectionConfig {
                analysis_duration_secs: duration.min(10.0),
                prominence_threshold: threshold,
                ..DetectionConfig::default()
            };
            let bands = notch::detect_noise_bands_async(
                &samples,
                sample_rate,
                &config,
                crate::canvas::tile_cache::yield_to_browser,
            )
            .await;
            let count = bands.len();
            state.notch.bands().set(bands);
            if count > 0 {
                state.notch.enabled().set(true);
                state.show_info_toast(format!(
                    "Found {} noise band{} from live",
                    count,
                    if count == 1 { "" } else { "s" }
                ));
            } else {
                state.show_info_toast("No persistent noise bands detected");
            }
            state.notch.detecting().set(false);
        });
        return;
    }

    let files = state.library.files().get_untracked();
    let idx = state.library.current_index().get_untracked();
    let Some(file) = idx.and_then(|i| files.get(i).cloned()) else {
        state.show_error_toast("No file loaded");
        return;
    };
    state.notch.detecting().set(true);
    let total = file.audio.source.total_samples() as usize;
    let samples = Arc::new(
        file.audio
            .source
            .read_region(ChannelView::MonoMix, 0, total),
    );
    let sample_rate = file.audio.sample_rate;
    let duration = file.audio.duration_secs;

    spawn_local(async move {
        yield_to_browser().await;
        let config = DetectionConfig {
            analysis_duration_secs: if duration > 30.0 { 10.0 } else { duration },
            prominence_threshold: threshold,
            ..DetectionConfig::default()
        };
        let bands = notch::detect_noise_bands_async(
            &samples,
            sample_rate,
            &config,
            crate::canvas::tile_cache::yield_to_browser,
        )
        .await;
        let count = bands.len();
        state.notch.bands().set(bands);
        if count > 0 {
            state.notch.enabled().set(true);
            state.show_info_toast(format!(
                "Found {} noise band{}",
                count,
                if count == 1 { "" } else { "s" }
            ));
        } else {
            state.show_info_toast("No persistent noise bands detected");
        }
        state.notch.detecting().set(false);
    });
}

/// Learn the noise floor that noise reduction subtracts, then turn noise
/// reduction on.
fn run_learn(state: AppState) {
    // While live, source samples from the circular capture buffer rather
    // than the file (which lags the buffer by the periodic snapshot
    // interval, and is short on history right after Listen starts).
    let is_live = state.mic.listening().get_untracked() || state.mic.recording().get_untracked();
    if is_live {
        let is_tauri = state.is_tauri;
        let sample_rate = state.mic.sample_rate().get_untracked();
        let snapshot = crate::audio::mic_backend::with_live_samples(is_tauri, |s| s.to_vec());
        if sample_rate == 0 || snapshot.is_empty() {
            state.show_error_toast("Not enough live audio yet");
            return;
        }
        let samples = Arc::new(snapshot);
        state.noise_reduce.learning().set(true);
        spawn_local(async move {
            yield_to_browser().await;
            // The newest LEARN_SECS: what was just heard.
            let learn_secs = crate::dsp::spectral_sub::LEARN_SECS;
            let keep = (learn_secs * sample_rate as f64).ceil() as usize;
            let recent = &samples[samples.len().saturating_sub(keep)..];
            let floor = crate::dsp::spectral_sub::learn_noise_floor_async(
                recent,
                sample_rate,
                learn_secs,
                crate::canvas::tile_cache::yield_to_browser,
            )
            .await;
            if let Some(f) = floor {
                state.noise_reduce.floor().set(Some(f));
                state.noise_reduce.enabled().set(true);
                state.show_info_toast("Noise floor learned from live");
            } else {
                state.show_error_toast("Not enough live audio to learn noise floor");
            }
            state.noise_reduce.learning().set(false);
        });
        return;
    }

    let files = state.library.files().get_untracked();
    let idx = state.library.current_index().get_untracked();
    let Some(file) = idx.and_then(|i| files.get(i).cloned()) else {
        state.show_error_toast("No file loaded");
        return;
    };
    state.noise_reduce.learning().set(true);
    // The first LEARN_SECS, the same stretch the display's automatic
    // floor uses.
    let sample_rate = file.audio.sample_rate;
    let learn_secs = crate::dsp::spectral_sub::LEARN_SECS;
    let total = file.audio.source.total_samples() as usize;
    let wanted = (learn_secs * sample_rate as f64).ceil() as usize;
    let samples = Arc::new(file.audio.source.read_region(
        ChannelView::MonoMix,
        0,
        total.min(wanted),
    ));

    spawn_local(async move {
        yield_to_browser().await;
        let floor = crate::dsp::spectral_sub::learn_noise_floor_async(
            &samples,
            sample_rate,
            learn_secs,
            crate::canvas::tile_cache::yield_to_browser,
        )
        .await;
        if let Some(f) = floor {
            state.noise_reduce.floor().set(Some(f));
            state.noise_reduce.enabled().set(true);
            state.show_info_toast("Noise floor learned");
        } else {
            state.show_error_toast("Not enough audio to learn noise floor");
        }
        state.noise_reduce.learning().set(false);
    });
}

/// Turn the notch filter on or off. Turning it on with no bands defined runs
/// detection first (live or file, whichever applies), so one press is enough.
fn set_notch_enabled(state: AppState, on: bool, threshold: f64) {
    state.notch.enabled().set(on);
    if on
        && state.notch.bands().get_untracked().is_empty()
        && !state.notch.detecting().get_untracked()
    {
        run_detect(state, threshold);
    }
}

/// Turn noise reduction on or off. Turning it on with no noise floor learned
/// learns one first (live or file), so one press is enough.
fn set_nr_enabled(state: AppState, on: bool) {
    state.noise_reduce.enabled().set(on);
    if on
        && state.noise_reduce.floor().get_untracked().is_none()
        && !state.noise_reduce.learning().get_untracked()
    {
        run_learn(state);
    }
}

/// "NR + Notch": the short names of the chosen filters, for the bar.
fn filter_names(nr: bool, notch: bool) -> String {
    match (nr, notch) {
        (true, true) => "NR + Notch",
        (true, false) => "NR",
        (false, true) => "Notch",
        (false, false) => "None",
    }
    .to_string()
}

#[component]
pub fn NoiseButton() -> impl IntoView {
    let state = expect_context::<AppState>();

    // The notch Sensitivity slider's value (prominence threshold). Kept here,
    // not in the menu, because the menu is unmounted each time it closes.
    let sensitivity = RwSignal::new(DEFAULT_SENSITIVITY);

    let is_open =
        Signal::derive(move || state.panels.layer_panel_open().get() == Some(LayerPanel::Noise));
    let no_file = Signal::derive(move || state.no_recording_open());
    let nr_on = Signal::derive(move || state.noise_reduce.enabled().get());
    let notch_on = Signal::derive(move || state.notch.enabled().get());
    let any_on = Signal::derive(move || nr_on.get() || notch_on.get());

    // The filters that were on most recently, as (noise reduction, notch).
    // The bar switch turns these back on, and the bottom row names them
    // while everything is off. Starts as noise reduction alone.
    let last_on = RwSignal::new((true, false));
    Effect::new(move |_| {
        let now = (nr_on.get(), notch_on.get());
        if now.0 || now.1 {
            last_on.set(now);
        }
    });

    let value = Signal::derive(move || {
        let (nr, notch) = if any_on.get() {
            (nr_on.get(), notch_on.get())
        } else {
            last_on.get()
        };
        filter_names(nr, notch)
    });

    let toggle = Callback::new(move |()| {
        if any_on.get_untracked() {
            state.noise_reduce.enabled().set(false);
            state.notch.enabled().set(false);
        } else {
            let (nr, notch) = last_on.get_untracked();
            if nr {
                set_nr_enabled(state, true);
            }
            if notch {
                set_notch_enabled(state, true, sensitivity.get_untracked());
            }
        }
    });
    let toggle_menu = Callback::new(move |()| toggle_panel(&state, LayerPanel::Noise));
    let toggle_nr = Callback::new(move |()| {
        set_nr_enabled(state, !state.noise_reduce.enabled().get_untracked())
    });
    let toggle_notch = Callback::new(move |()| {
        set_notch_enabled(
            state,
            !state.notch.enabled().get_untracked(),
            sensitivity.get_untracked(),
        )
    });

    view! {
        <SwitchButton
            name="Noise"
            on=any_on
            toggle=toggle
            value=value
            is_open=is_open
            toggle_menu=toggle_menu
            disabled=no_file
            width="88px"
            switch_title="Turn noise filtering on or off"
            menu_title="Noise filter settings"
            panel_align="right"
            panel_style="min-width: 260px;"
        >
            <SwitchRow name="Noise reduction" on=nr_on toggle=toggle_nr />
            <NrSettings />
            <hr />
            <SwitchRow name="Notch filter" on=notch_on toggle=toggle_notch />
            <NotchSettings sensitivity=sensitivity />
        </SwitchButton>
    }
}

// ─── Notch filter settings ──────────────────────────────────────────────
//
// Detection (button + sensitivity), the per-band list (with bulk toggles),
// and harmonic suppression.

#[component]
fn NotchSettings(sensitivity: RwSignal<f64>) -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_detect = move |_: web_sys::MouseEvent| {
        run_detect(state, sensitivity.get_untracked());
    };

    let on_sensitivity_change = move |ev: web_sys::Event| {
        let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = target.value().parse::<f64>() {
            sensitivity.set((150.0 - val) / 10.0);
        }
    };

    let on_harmonic_change = move |ev: web_sys::Event| {
        let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = target.value().parse::<f64>() {
            state.notch.harmonic_suppression().set(val / 100.0);
        }
    };

    let toggle_band = move |index: usize| {
        state.notch.bands().update(|bands| {
            if let Some(band) = bands.get_mut(index) {
                band.enabled = !band.enabled;
            }
        });
    };
    let remove_band = move |index: usize| {
        state.notch.bands().update(|bands| {
            if index < bands.len() {
                bands.remove(index);
            }
        });
    };
    let set_all_enabled = move |enabled: bool| {
        state.notch.bands().update(|bands| {
            for band in bands.iter_mut() {
                band.enabled = enabled;
            }
        });
    };

    view! {
        // ── Detection ──
        <div class="layer-panel-title">"Detection"</div>
        <div style="display: flex; gap: 4px; padding: 0 6px 4px;">
            <button
                class="layer-panel-opt"
                style="flex: 1;"
                on:click=on_detect
                disabled=move || {
                    if state.notch.detecting().get() { return true; }
                    let live = state.mic.listening().get() || state.mic.recording().get();
                    !live && state.library.current_index().get().is_none()
                }
            >
                {move || {
                    if state.notch.detecting().get() { return "Detecting\u{2026}".to_string(); }
                    let live = state.mic.listening().get() || state.mic.recording().get();
                    if live { "Detect from Live".to_string() } else { "Detect Noise".to_string() }
                }}
            </button>
        </div>
        <div class="layer-panel-slider-row">
            <label>"Sensitivity"</label>
            <input type="range" min="30" max="120" step="5"
                prop:value=move || (150.0 - sensitivity.get() * 10.0) as i32
                on:input=on_sensitivity_change
                title=move || format!("Threshold: {:.1}x ({:.0} dB)", sensitivity.get(), 20.0 * sensitivity.get().log10())
            />
        </div>

        <hr />

        // ── Bands ──
        <div class="layer-panel-title">{move || {
            let bands = state.notch.bands().get();
            let on = bands.iter().filter(|b| b.enabled).count();
            if bands.is_empty() { "Bands".to_string() }
            else { format!("Bands ({}/{})", on, bands.len()) }
        }}</div>
        {move || {
            let bands = state.notch.bands().get();
            if bands.is_empty() {
                view! {
                    <div style="padding: 4px 8px; font-size: 11px; opacity: 0.55;">
                        "No bands detected yet"
                    </div>
                }.into_any()
            } else {
                let items: Vec<_> = bands.iter().enumerate().map(|(i, band)| {
                    let center = band.center_hz;
                    let strength = band.strength_db;
                    let band_enabled = band.enabled;
                    let bandwidth = band.bandwidth_hz;
                    view! {
                        <div
                            style="display: flex; align-items: center; gap: 6px; padding: 2px 8px; font-size: 11px;"
                            on:mouseenter=move |_| state.notch.hovering_band().set(Some(i))
                            on:mouseleave=move |_| state.notch.hovering_band().set(None)
                        >
                            <input type="checkbox"
                                checked=band_enabled
                                on:change=move |_| toggle_band(i)
                                style="margin: 0;"
                            />
                            <span style="flex: 1; white-space: nowrap;">
                                {if center >= 1000.0 { format!("{:.1} kHz", center / 1000.0) }
                                 else { format!("{:.0} Hz", center) }}
                            </span>
                            <span style="opacity: 0.6; font-size: 10px; white-space: nowrap;"
                                  title=format!("BW: {:.0} Hz", bandwidth)>
                                {format!("+{:.0}dB", strength)}
                            </span>
                            <button
                                style="background: none; border: none; color: inherit; opacity: 0.4; cursor: pointer; padding: 0 2px; font-size: 12px;"
                                on:click=move |_: web_sys::MouseEvent| remove_band(i)
                                title="Remove band"
                            >{"\u{00D7}"}</button>
                        </div>
                    }
                }).collect();
                view! {
                    <div style="max-height: 220px; overflow-y: auto;">{items}</div>
                    <div style="display: flex; gap: 4px; padding: 4px 6px 0;">
                        <button class="layer-panel-opt" style="flex: 1; font-size: 10px;"
                            on:click=move |_: web_sys::MouseEvent| set_all_enabled(true)
                        >"All On"</button>
                        <button class="layer-panel-opt" style="flex: 1; font-size: 10px;"
                            on:click=move |_: web_sys::MouseEvent| set_all_enabled(false)
                        >"All Off"</button>
                        <button class="layer-panel-opt" style="flex: 1; font-size: 10px;"
                            on:click=move |_: web_sys::MouseEvent| {
                                state.notch.bands().set(Vec::new());
                                state.notch.enabled().set(false);
                            }
                        >"Clear"</button>
                    </div>
                }.into_any()
            }
        }}

        // ── Harmonic suppression (only when there are bands or a learned floor) ──
        {move || {
            let has_bands = !state.notch.bands().get().is_empty();
            let has_floor = state.noise_reduce.floor().get().is_some();
            if has_bands || has_floor {
                view! {
                    <hr />
                    <div class="layer-panel-slider-row">
                        <label>"Harm. supp."</label>
                        <input type="range" min="0" max="100" step="5"
                            prop:value=move || (state.notch.harmonic_suppression().get() * 100.0) as i32
                            on:input=on_harmonic_change
                            title=move || {
                                let v = state.notch.harmonic_suppression().get();
                                if v == 0.0 { "Off".to_string() }
                                else { format!("{:.0}% ({:.0} dB at 2\u{00D7} & 3\u{00D7})", v * 100.0, -48.0 * v) }
                            }
                        />
                        <span style="min-width: 30px; text-align: right; font-size: 10px; opacity: 0.7;">
                            {move || format!("{:.0}%", state.notch.harmonic_suppression().get() * 100.0)}
                        </span>
                    </div>
                }.into_any()
            } else {
                view! { <span></span> }.into_any()
            }
        }}
    }
}

// ─── Noise reduction settings ───────────────────────────────────────────
//
// Spectral-subtraction noise reduction: the "learn" button, strength slider,
// and floor status.

#[component]
fn NrSettings() -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_learn_floor = move |_: web_sys::MouseEvent| {
        run_learn(state);
    };

    let on_strength_change = move |ev: web_sys::Event| {
        let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = target.value().parse::<f64>() {
            state.noise_reduce.strength().set(val / 100.0);
        }
    };

    view! {
        // ── Learn ──
        <div style="display: flex; gap: 4px; padding: 0 6px 4px;">
            <button
                class="layer-panel-opt"
                style="flex: 1;"
                on:click=on_learn_floor
                disabled=move || {
                    if state.noise_reduce.learning().get() { return true; }
                    let live = state.mic.listening().get() || state.mic.recording().get();
                    !live && state.library.current_index().get().is_none()
                }
            >
                {move || {
                    if state.noise_reduce.learning().get() { return "Learning\u{2026}".to_string(); }
                    let live = state.mic.listening().get() || state.mic.recording().get();
                    if live { "Learn from Live".to_string() } else { "Learn Noise Floor".to_string() }
                }}
            </button>
        </div>

        // ── Strength ──
        <div class="layer-panel-slider-row">
            <label>"Strength"</label>
            <input type="range" min="0" max="300" step="5"
                prop:value=move || (state.noise_reduce.strength().get() * 100.0) as i32
                on:input=on_strength_change
                title=move || format!("{:.0}%", state.noise_reduce.strength().get() * 100.0)
            />
            <span style="min-width: 36px; text-align: right; font-size: 10px; opacity: 0.7;">
                {move || format!("{:.0}%", state.noise_reduce.strength().get() * 100.0)}
            </span>
        </div>

        // ── Floor status ──
        {move || {
            let floor = state.noise_reduce.floor().get();
            if let Some(f) = floor {
                let bins = f.bin_magnitudes.len();
                let dur = f.analysis_duration_secs;
                view! {
                    <hr />
                    <div style="padding: 2px 8px; font-size: 11px; opacity: 0.7;">
                        {format!("{} bins, {:.1}s analyzed", bins, dur)}
                    </div>
                    <div style="display: flex; gap: 4px; padding: 2px 6px 0;">
                        <button class="layer-panel-opt" style="flex: 1; font-size: 10px;"
                            on:click=move |_: web_sys::MouseEvent| {
                                state.noise_reduce.floor().set(None);
                                state.noise_reduce.enabled().set(false);
                            }
                        >"Clear Floor"</button>
                    </div>
                }.into_any()
            } else {
                view! {
                    <div style="padding: 2px 8px; font-size: 11px; opacity: 0.5;">
                        "No noise floor learned"
                    </div>
                }.into_any()
            }
        }}

        <DenoiseSection />
    }
}

// ─── The learned bat denoiser ───────────────────────────────────────────
//
// A second method, in the same menu: one button that makes a denoised copy of
// the open recording (`crate::denoise`). The line under the button says what
// the press will do, why it can't be pressed, or how far the work has got.

#[component]
fn DenoiseSection() -> impl IntoView {
    use crate::denoise::{self, text};
    use crate::state::{DenoiseStateStoreFields, DenoiseStatus};
    let state = expect_context::<AppState>();
    denoise::check_model_stored(state);

    // Sample rate, length in samples and channels of the open recording.
    let open = move || {
        state.current_file().map(|f| {
            (
                f.audio.sample_rate,
                f.audio.source.total_samples() as usize,
                f.audio.channels,
            )
        })
    };
    let refusal = move || open().and_then(|(rate, samples, _)| denoise::refusal(rate, samples));
    let busy = move || denoise::is_busy(state);

    view! {
        <hr />
        <div class="layer-panel-title">{text::TITLE}</div>
        <div style="display: flex; gap: 4px; padding: 0 6px 4px;">
            {move || if busy() {
                view! {
                    <button class="layer-panel-opt" style="flex: 1;"
                        on:click=move |_: web_sys::MouseEvent| denoise::cancel(state)
                    >{text::CANCEL}</button>
                }.into_any()
            } else {
                view! {
                    <button class="layer-panel-opt" style="flex: 1;"
                        title=text::TOOLTIP
                        disabled=move || open().is_none() || refusal().is_some()
                        on:click=move |_: web_sys::MouseEvent| denoise::start(state)
                    >{text::BUTTON}</button>
                }.into_any()
            }}
        </div>
        <div class="denoise-status">
        {move || {
            let muted = |line: String| view! {
                <div class="denoise-note">{line}</div>
            }.into_any();
            match state.denoise.status().get() {
                DenoiseStatus::Downloading(f) => progress(f, text::downloading(percent(f))),
                DenoiseStatus::Starting => muted(text::STARTING.to_string()),
                DenoiseStatus::Running(f) => progress(f, text::running(percent(f))),
                DenoiseStatus::Failed(error) => view! {
                    <div class="denoise-note denoise-error">{error}</div>
                }.into_any(),
                DenoiseStatus::Idle => match refusal() {
                    Some(why) => muted(why),
                    None => {
                        let stored = state.denoise.model_stored().get().unwrap_or(true);
                        let stereo = open().is_some_and(|(_, _, channels)| channels > 1);
                        view! {
                            <div class="denoise-note">
                                {text::help(stored, &denoise::model_size_text())}
                            </div>
                            {stereo.then(|| view! {
                                <div class="denoise-note">{text::STEREO}</div>
                            })}
                        }.into_any()
                    }
                },
            }
        }}
        </div>
    }
}

fn percent(fraction: f64) -> u32 {
    (fraction * 100.0).round().clamp(0.0, 100.0) as u32
}

/// A progress bar with its line of text, in the export panel's style.
fn progress(fraction: f64, line: String) -> AnyView {
    view! {
        <div class="export-progress" style="padding: 0 8px 2px;">
            <div class="export-progress-bar">
                <div class="export-progress-fill"
                    style:width=format!("{:.1}%", fraction * 100.0)></div>
            </div>
            <div class="export-progress-text">{line}</div>
        </div>
    }.into_any()
}

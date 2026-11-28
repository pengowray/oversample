// Noise button: the Hearing Bar's one control for the noise filters.
//
// Noise reduction (spectral subtraction) and the notch filter used to be two
// split buttons. They are now one switch button: its top row turns noise
// filtering on or off, its bottom row names the filters in use, and its menu
// has a section per filter, each with its own switch and settings. A new
// noise filter gets a section here, not another button in the bar.
//
// The menu has two groups (2026-11-15, after the user found it "a confusing
// mix"): "Filters" holds noise reduction and the notch filter, which change
// what is heard and seen and can both be on; under a rule, "Denoised copy"
// makes a new recording and is not affected by either switch. Each filter's
// settings are indented under its switch and stay visible while it is off,
// so nothing moves when a switch is pressed.
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

/// What started a detection of the notch filter's noise bands.
#[derive(Clone, Copy, PartialEq)]
enum DetectFrom {
    /// The Detect button, or a switch turned on with no bands yet. Says what
    /// it found in a toast and turns the notch filter on when it finds bands.
    Press,
    /// The Sensitivity slider moved. Updates the bands and nothing else: no
    /// toast, the filter stays on or off, and a band the user turned off
    /// stays off.
    Slider,
}

thread_local! {
    /// Counts moves of the Sensitivity slider, so that only the last move of
    /// a drag starts a detection. Not a signal: the menu, and any signal it
    /// owns, is gone as soon as it closes.
    static SLIDER_MOVES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A band that was detected before keeps the user's on/off choice. Two
/// detections give slightly different centres for one tone, so a band counts
/// as the same when the centres are within half the wider bandwidth.
fn keep_band_switches(old: &[notch::NoiseBand], new: &mut [notch::NoiseBand]) {
    for band in new.iter_mut() {
        let same = old.iter().find(|o| {
            (o.center_hz - band.center_hz).abs() <= o.bandwidth_hz.max(band.bandwidth_hz) / 2.0
        });
        if let Some(o) = same {
            band.enabled = o.enabled;
        }
    }
}

/// Detect the notch filter's noise bands, with the Sensitivity slider's value
/// as the prominence a band needs. While one detection runs, another does
/// not start: the running one detects again at its end if the slider has
/// moved since it began.
fn run_detect(state: AppState, sensitivity: RwSignal<f64>, from: DetectFrom) {
    if state.notch.detecting().get_untracked() {
        return;
    }
    let quiet = from == DetectFrom::Slider;
    // While live, detect from the circular buffer so the bands reflect
    // current ambient noise rather than stale file content.
    let is_live = state.mic.listening().get_untracked() || state.mic.recording().get_untracked();
    let (samples, sample_rate, analysis_secs) = if is_live {
        let sample_rate = state.mic.sample_rate().get_untracked();
        let snapshot = crate::audio::mic_backend::with_live_samples(state.is_tauri, |s| s.to_vec());
        if sample_rate == 0 || snapshot.is_empty() {
            if !quiet {
                state.show_error_toast("Not enough live audio yet");
            }
            return;
        }
        let duration = snapshot.len() as f64 / sample_rate as f64;
        (snapshot, sample_rate, duration.min(10.0))
    } else {
        let Some(file) = state.current_file() else {
            if !quiet {
                state.show_error_toast("No file loaded");
            }
            return;
        };
        let sample_rate = file.audio.sample_rate;
        let duration = file.audio.duration_secs;
        let analysis_secs = if duration > 30.0 { 10.0 } else { duration };
        // Detection looks at the first `analysis_secs` only, so read no more.
        let total = file.audio.source.total_samples() as usize;
        let wanted = (analysis_secs * sample_rate as f64).ceil() as usize;
        let samples = file
            .audio
            .source
            .read_region(ChannelView::MonoMix, 0, total.min(wanted));
        (samples, sample_rate, analysis_secs)
    };
    let Some(threshold) = sensitivity.try_get_untracked() else {
        return;
    };

    state.notch.detecting().set(true);
    spawn_local(async move {
        yield_to_browser().await;
        let config = DetectionConfig {
            analysis_duration_secs: analysis_secs,
            prominence_threshold: threshold,
            ..DetectionConfig::default()
        };
        let mut bands = notch::detect_noise_bands_async(
            &samples,
            sample_rate,
            &config,
            crate::canvas::tile_cache::yield_to_browser,
        )
        .await;
        let count = bands.len();
        if quiet {
            keep_band_switches(&state.notch.bands().get_untracked(), &mut bands);
        }
        state.notch.bands().set(bands);
        if !quiet {
            if count > 0 {
                state.notch.enabled().set(true);
                state.show_info_toast(format!(
                    "Found {} noise band{}{}",
                    count,
                    if count == 1 { "" } else { "s" },
                    if is_live { " from live" } else { "" }
                ));
            } else {
                state.show_info_toast("No persistent noise bands detected");
            }
        }
        state.notch.detecting().set(false);
        if sensitivity
            .try_get_untracked()
            .is_some_and(|now| now != threshold)
        {
            run_detect(state, sensitivity, DetectFrom::Slider);
        }
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
fn set_notch_enabled(state: AppState, on: bool, sensitivity: RwSignal<f64>) {
    state.notch.enabled().set(on);
    if on
        && state.notch.bands().get_untracked().is_empty()
        && !state.notch.detecting().get_untracked()
    {
        run_detect(state, sensitivity, DetectFrom::Press);
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
                set_notch_enabled(state, true, sensitivity);
            }
        }
    });
    let toggle_menu = Callback::new(move |()| toggle_panel(&state, LayerPanel::Noise));
    let toggle_nr = Callback::new(move |()| {
        set_nr_enabled(state, !state.noise_reduce.enabled().get_untracked())
    });
    let toggle_notch = Callback::new(move |()| {
        set_notch_enabled(state, !state.notch.enabled().get_untracked(), sensitivity)
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
            // A fixed width: the menu is placed when it opens, and a row that
            // grew afterwards ("Learn again") pushed it off a phone's screen.
            panel_style="width: min(340px, calc(100vw - 16px));"
        >
            <div class="layer-panel-title">"Filters: use one or both"</div>
            <SwitchRow name="Noise reduction" on=nr_on toggle=toggle_nr />
            <div class="noise-settings"><NrSettings /></div>
            <SwitchRow name="Notch filter" on=notch_on toggle=toggle_notch />
            <div class="noise-settings"><NotchSettings sensitivity=sensitivity /></div>
            <DenoiseSection />
        </SwitchButton>
    }
}

// ─── Notch filter settings ──────────────────────────────────────────────
//
// What the filter removes, the Bands row (how many bands are on, and the
// Detect button), the band list with its bulk buttons, and the Sensitivity
// slider, which detects again as it moves.

#[component]
fn NotchSettings(sensitivity: RwSignal<f64>) -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_detect = move |_: web_sys::MouseEvent| {
        run_detect(state, sensitivity, DetectFrom::Press);
    };

    let on_sensitivity_change = move |ev: web_sys::Event| {
        let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(val) = target.value().parse::<f64>() {
            sensitivity.set((150.0 - val) / 10.0);
            // Detect again 200 ms after the slider's last move, so the Bands
            // row and the marks on the spectrogram follow the slider.
            let n = SLIDER_MOVES.with(|c| {
                c.set(c.get().wrapping_add(1));
                c.get()
            });
            spawn_local(async move {
                crate::web_util::sleep_ms(200).await;
                if SLIDER_MOVES.with(|c| c.get()) == n {
                    run_detect(state, sensitivity, DetectFrom::Slider);
                }
            });
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
    let live = move || state.mic.listening().get() || state.mic.recording().get();

    view! {
        <div class="noise-note">"Removes constant tones"</div>

        // ── Bands: how many are on, and Detect ──
        <div class="noise-row">
            <span class="noise-row-label">"Bands"</span>
            {move || {
                let bands = state.notch.bands().get();
                if bands.is_empty() {
                    view! { <span class="noise-row-value none">"none yet"</span> }.into_any()
                } else {
                    let on = bands.iter().filter(|b| b.enabled).count();
                    view! {
                        <span class="noise-row-value">{format!("{} of {} on", on, bands.len())}</span>
                    }.into_any()
                }
            }}
            <button
                class="noise-btn"
                on:click=on_detect
                disabled=move || {
                    state.notch.detecting().get()
                        || (!live() && state.library.current_index().get().is_none())
                }
            >
                {move || {
                    if state.notch.detecting().get() {
                        "Detecting\u{2026}"
                    } else if live() {
                        "Detect from live"
                    } else if state.notch.bands().with(|b| b.is_empty()) {
                        "Detect"
                    } else {
                        "Detect again"
                    }
                }}
            </button>
        </div>

        // ── The bands ──
        {move || {
            let bands = state.notch.bands().get();
            (!bands.is_empty()).then(|| {
                let items: Vec<_> = bands.iter().enumerate().map(|(i, band)| {
                    let center = band.center_hz;
                    let strength = band.strength_db;
                    let band_enabled = band.enabled;
                    let bandwidth = band.bandwidth_hz;
                    view! {
                        <div
                            class="noise-band"
                            on:mouseenter=move |_| state.notch.hovering_band().set(Some(i))
                            on:mouseleave=move |_| state.notch.hovering_band().set(None)
                        >
                            <input type="checkbox"
                                checked=band_enabled
                                on:change=move |_| toggle_band(i)
                            />
                            <span class="noise-band-freq">
                                {if center >= 1000.0 { format!("{:.1} kHz", center / 1000.0) }
                                 else { format!("{:.0} Hz", center) }}
                            </span>
                            <span class="noise-band-level"
                                  title=format!("{:.0} dB above the sound around it; {:.0} Hz wide", strength, bandwidth)>
                                {format!("+{:.0} dB", strength)}
                            </span>
                            <button
                                class="noise-band-remove"
                                on:click=move |_: web_sys::MouseEvent| remove_band(i)
                                title="Remove band"
                                aria-label="Remove band"
                            >{"\u{00D7}"}</button>
                        </div>
                    }
                }).collect();
                view! {
                    <div class="noise-bands">{items}</div>
                    <div class="noise-row noise-band-actions">
                        <button class="noise-btn"
                            on:click=move |_: web_sys::MouseEvent| set_all_enabled(true)
                        >"All on"</button>
                        <button class="noise-btn"
                            on:click=move |_: web_sys::MouseEvent| set_all_enabled(false)
                        >"All off"</button>
                        <button class="noise-btn"
                            title="Removes every band and turns the notch filter off"
                            on:click=move |_: web_sys::MouseEvent| {
                                state.notch.bands().set(Vec::new());
                                state.notch.enabled().set(false);
                            }
                        >"Clear"</button>
                    </div>
                }
            })
        }}

        // ── Sensitivity: moving it detects again ──
        <div class="layer-panel-slider-row">
            <label>"Sensitivity"</label>
            <input type="range" min="30" max="120" step="5"
                prop:value=move || (150.0 - sensitivity.get() * 10.0) as i32
                on:input=on_sensitivity_change
                title=move || format!(
                    "Finds tones at least {:.0} dB above the sound around them",
                    20.0 * sensitivity.get().log10()
                )
            />
        </div>
    }
}

// ─── Noise reduction settings ───────────────────────────────────────────
//
// Spectral-subtraction noise reduction: what it removes, the strength
// slider, and the Noise floor row (how much audio the floor was learned
// from, Learn, Clear).

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
    let live = move || state.mic.listening().get() || state.mic.recording().get();
    let has_floor = move || state.noise_reduce.floor().with(|f| f.is_some());

    view! {
        <div class="noise-note">"Removes steady background noise"</div>

        // ── Strength ──
        <div class="layer-panel-slider-row">
            <label>"Strength"</label>
            // 0 to 200%, so the 100% default is the middle of the slider. A
            // recording saved with more keeps its value: the slider sits at
            // its end and the number beside it shows the value.
            <input type="range" min="0" max="200" step="5"
                prop:value=move || (state.noise_reduce.strength().get() * 100.0) as i32
                on:input=on_strength_change
            />
            <span>{move || format!("{:.0}%", state.noise_reduce.strength().get() * 100.0)}</span>
        </div>

        // ── Noise floor: what it was learned from, Learn, Clear ──
        <div class="noise-row">
            <span class="noise-row-label">"Noise floor"</span>
            {move || match state.noise_reduce.floor().get() {
                Some(f) => view! {
                    <span class="noise-row-value"
                        title=format!("{} frequency bins", f.bin_magnitudes.len())>
                        {format!("{:.1} s of audio", f.analysis_duration_secs)}
                    </span>
                }.into_any(),
                None => view! { <span class="noise-row-value none">"not learned"</span> }.into_any(),
            }}
            <button
                class="noise-btn"
                on:click=on_learn_floor
                disabled=move || {
                    state.noise_reduce.learning().get()
                        || (!live() && state.library.current_index().get().is_none())
                }
            >
                {move || {
                    if state.noise_reduce.learning().get() {
                        "Learning\u{2026}"
                    } else if live() {
                        "Learn from live"
                    } else if has_floor() {
                        "Learn again"
                    } else {
                        "Learn"
                    }
                }}
            </button>
            <button
                class="noise-btn"
                title="Clears the noise floor and turns noise reduction off"
                disabled=move || !has_floor()
                on:click=move |_: web_sys::MouseEvent| {
                    state.noise_reduce.floor().set(None);
                    state.noise_reduce.enabled().set(false);
                }
            >"Clear"</button>
        </div>
    }
}

// ─── The learned denoiser ───────────────────────────────────────────────
//
// The menu's second group, under a rule: a Strength choice (Normal or Heavy)
// and one button that makes a denoised copy of the open recording at that
// strength (`crate::denoise`). It is not one of the filters above, and their
// switches do not change the copy. The lines under the button name the
// model and say what the press will do, why it can't be pressed, or how far
// the work has got. The "Model:" line is where a choice of model would go.

#[component]
fn DenoiseSection() -> impl IntoView {
    use crate::denoise::{self, text};
    use crate::state::{DenoiseStateStoreFields, DenoiseStatus, InteractionStateStoreFields};
    let state = expect_context::<AppState>();
    denoise::check_model_stored(state);

    // Sample rate, length in samples and channels of what the button would
    // copy: the selection when there is one, else the open recording.
    let range = move || {
        state.interaction.selection().track();
        state.current_file().map(|f| denoise::copy_range(state, &f))
    };
    let open = move || {
        let frames = range()?.frames;
        state
            .current_file()
            .map(|f| (f.audio.sample_rate, frames, f.audio.channels))
    };
    let of_selection = move || range().is_some_and(|r| r.selection);
    let refusal = move || {
        open().and_then(|(rate, samples, channels)| {
            denoise::refusal(rate, samples, channels, denoise::sample_limit(state))
        })
    };
    let busy = move || denoise::is_busy(state);
    let can_start = move || open().is_some() && refusal().is_none() && !busy();

    view! {
        <hr />
        <div class="noise-group-title">{text::TITLE}</div>
        <StrengthRow can_start=can_start />
        <div class="noise-row">
            {move || if busy() {
                view! {
                    <button class="noise-btn"
                        on:click=move |_: web_sys::MouseEvent| denoise::cancel(state)
                    >{text::CANCEL}</button>
                }.into_any()
            } else {
                view! {
                    <button class="noise-btn"
                        title=text::TOOLTIP
                        disabled=move || !can_start()
                        on:click=move |_: web_sys::MouseEvent| denoise::start(state)
                    >{move || if of_selection() { text::BUTTON_SELECTION } else { text::BUTTON }}</button>
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
                DenoiseStatus::Running { fraction, seconds_left } => {
                    progress(fraction, text::running(percent(fraction), seconds_left))
                }
                DenoiseStatus::Failed(error) => view! {
                    <div class="denoise-note denoise-error">{error}</div>
                }.into_any(),
                DenoiseStatus::Idle => match refusal() {
                    Some(why) => muted(why),
                    None => {
                        let stored = state.denoise.model_stored().get().unwrap_or(true);
                        // More than two channels are mixed to one.
                        let mono_mix = open().is_some_and(|(_, _, channels)| channels > 2);
                        view! {
                            <div class="denoise-note">
                                {text::model_line(denoise::model().trained_on)}
                            </div>
                            <div class="denoise-note">{text::result_line(mono_mix, of_selection())}</div>
                            {(!stored).then(|| view! {
                                <div class="denoise-note">
                                    {text::download_line(&denoise::model_size_text())}
                                </div>
                            })}
                        }.into_any()
                    }
                },
            }
        }}
        </div>
    }
}

/// The Strength row, Normal or Heavy, and the line under it that says what
/// the picked strength does. The pick is for the next copy and is saved in
/// the browser. It is a radio group: Tab reaches the picked option and the
/// arrow keys move the pick. Double-clicking an option picks it and makes the
/// copy, as the button would when `can_start`. While a copy is made the
/// options are disabled, so they show the strength in use.
#[component]
fn StrengthRow(can_start: impl Fn() -> bool + Copy + Send + Sync + 'static) -> impl IntoView {
    use crate::denoise::{self, text};
    use crate::state::{DenoiseStateStoreFields, DenoiseStrength};
    let state = expect_context::<AppState>();

    let picked = move || state.denoise.strength().get();
    let pick = move |strength: DenoiseStrength| {
        if state.denoise.strength().get_untracked() != strength {
            state.denoise.strength().set(strength);
            crate::settings::set_raw(crate::settings::keys::DENOISE_STRENGTH, strength.key());
        }
    };
    let busy = move || denoise::is_busy(state);

    // Arrow keys move the pick to the other option, and the focus with it.
    // These keys and Space stop here: the app's own shortcuts use them to
    // scroll the recording and to play it.
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        let step: isize = match ev.key().as_str() {
            "ArrowLeft" | "ArrowUp" => -1,
            "ArrowRight" | "ArrowDown" => 1,
            " " => {
                ev.stop_propagation();
                return;
            }
            _ => return,
        };
        ev.prevent_default();
        ev.stop_propagation();
        let all = DenoiseStrength::ALL;
        let at = all
            .iter()
            .position(|s| *s == state.denoise.strength().get_untracked())
            .unwrap_or(0) as isize;
        let next = all[(at + step).rem_euclid(all.len() as isize) as usize];
        pick(next);
        let option = ev
            .current_target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|group| {
                group
                    .query_selector(&format!("[data-strength=\"{}\"]", next.key()))
                    .ok()
                    .flatten()
            })
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok());
        if let Some(option) = option {
            let _ = option.focus();
        }
    };

    view! {
        <div class="noise-row">
            <span class="noise-row-label">{text::STRENGTH}</span>
            <div
                class="denoise-strength"
                role="radiogroup"
                aria-label=text::STRENGTH_GROUP
                on:keydown=on_keydown
            >
                {DenoiseStrength::ALL.into_iter().map(|strength| view! {
                    <button
                        type="button"
                        class="peak-src-btn"
                        class:sel=move || picked() == strength
                        role="radio"
                        aria-checked=move || (picked() == strength).to_string()
                        tabindex=move || if picked() == strength { "0" } else { "-1" }
                        data-strength=strength.key()
                        title=text::STRENGTH_OPTION_TOOLTIP
                        disabled=busy
                        on:click=move |_| pick(strength)
                        on:dblclick=move |_| {
                            pick(strength);
                            if can_start() {
                                denoise::start(state);
                            }
                        }
                    >{text::strength_name(strength)}</button>
                }).collect_view()}
            </div>
        </div>
        <div class="denoise-note denoise-strength-line">
            {move || text::strength_line(picked())}
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
    }
    .into_any()
}

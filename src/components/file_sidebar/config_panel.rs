use crate::canvas::spectrogram_renderer::Colormap;
use crate::state::store_fields::*;
use crate::state::{AppState, ChromaColormap, ShieldStyle};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

fn parse_colormap_pref(s: &str) -> Colormap {
    match s {
        "inferno" => Colormap::Inferno,
        "magma" => Colormap::Magma,
        "plasma" => Colormap::Plasma,
        "cividis" => Colormap::Cividis,
        "turbo" => Colormap::Turbo,
        "greyscale" => Colormap::Greyscale,
        _ => Colormap::Viridis,
    }
}

/// Label and tooltip of the checkbox that gives the open file its own lowest
/// bat frequency.
const FILE_BAT_LABEL: &str = "This recording only";
const FILE_BAT_TIP: &str = "Sets a lowest bat frequency for this recording only, saved in its .batm file. Untick to use the Lowest bat frequency setting above again.";

/// Tooltip of the "Lowest bat frequency" setting.
const LOWEST_BAT_TIP: &str = "Lowest frequency to look for bat calls. Default 16 kHz. Call detection, the Loudest-frequency line, ZC playback and HFR all use this value.\nLower it to find free-tailed bats near 11 kHz. Birds, insects and wind noise will also be picked up.";

#[component]
pub(super) fn ConfigPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_follow_cursor = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        let checked = input.checked();
        state.view.follow_cursor().set(checked);
        if checked {
            state.view.follow_suspended().set(false);
            state.view.follow_visible_since().set(None);
        }
    };

    let on_colormap_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let select: web_sys::HtmlSelectElement = target.unchecked_into();
        state
            .spect
            .colormap_preference()
            .set(parse_colormap_pref(&select.value()));
        state
            .viewmode
            .tile_ready_signal()
            .update(|n| *n = n.wrapping_add(1));
    };

    let on_hfr_colormap_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let select: web_sys::HtmlSelectElement = target.unchecked_into();
        state
            .spect
            .hfr_colormap_preference()
            .set(parse_colormap_pref(&select.value()));
        state
            .viewmode
            .tile_ready_signal()
            .update(|n| *n = n.wrapping_add(1));
    };

    let is_tauri = state.is_tauri;
    let _ = is_tauri; // used in the view

    // The slider shows its value while dragging; the setting (and everything
    // that follows it) changes when it is released.
    let lowest_bat_khz = RwSignal::new(0.0f32);
    Effect::new(move || lowest_bat_khz.set(state.lowest_bat_hz_setting() / 1000.0));
    // The open file's own value, while its slider is dragged.
    let file_bat_khz = RwSignal::new(0.0f32);
    Effect::new(move || {
        if let Some(hz) = state.current_file_lowest_bat_hz() {
            file_bat_khz.set(hz / 1000.0);
        }
    });

    view! {
        <div class="sidebar-panel">
            // Recording settings moved to Record menu in bottom toolbar

            <div class="setting-group">
                <div class="setting-group-title">"Playback"</div>
                <div class="setting-row">
                    <span class="setting-label">"Follow cursor"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.view.follow_cursor().get()
                        on:change=on_follow_cursor
                    />
                </div>
            </div>

            <div class="setting-group">
                <div class="setting-group-title">"Bats"</div>
                <div class="setting-row" title=LOWEST_BAT_TIP>
                    <span class="setting-label">"Lowest bat frequency"</span>
                    <span class="setting-value">
                        {move || format!("{:.0} kHz", lowest_bat_khz.get())}
                    </span>
                </div>
                <div class="setting-row" title=LOWEST_BAT_TIP>
                    <input
                        type="range"
                        class="setting-range"
                        min="8"
                        max="30"
                        step="1"
                        prop:value=move || lowest_bat_khz.get().to_string()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                lowest_bat_khz.set(v);
                            }
                        }
                        on:change=move |_| {
                            state.set_lowest_bat_hz(lowest_bat_khz.get_untracked() * 1000.0);
                        }
                    />
                </div>
                // The open file's own value, saved in its .batm.
                {move || state.library.current_index().get().map(|idx| {
                    let own = state.current_file_lowest_bat_hz().is_some();
                    view! {
                        <div class="setting-row" title=FILE_BAT_TIP>
                            <label class="setting-label">
                                <input
                                    type="checkbox"
                                    prop:checked=own
                                    on:change=move |ev| {
                                        let on = event_target_checked(&ev);
                                        let hz = on.then(|| lowest_bat_khz.get_untracked() * 1000.0);
                                        state.set_file_lowest_bat_hz(idx, hz);
                                    }
                                />
                                " "{FILE_BAT_LABEL}
                            </label>
                            <span class="setting-value">
                                {move || own.then(|| format!("{:.0} kHz", file_bat_khz.get()))}
                            </span>
                        </div>
                        {own.then(|| view! {
                            <div class="setting-row" title=FILE_BAT_TIP>
                                <input
                                    type="range"
                                    class="setting-range"
                                    min="8"
                                    max="30"
                                    step="1"
                                    prop:value=move || file_bat_khz.get().to_string()
                                    on:input=move |ev| {
                                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                            file_bat_khz.set(v);
                                        }
                                    }
                                    on:change=move |_| {
                                        let hz = file_bat_khz.get_untracked() * 1000.0;
                                        state.set_file_lowest_bat_hz(idx, Some(hz));
                                    }
                                />
                            </div>
                        })}
                    }
                })}
                // Box first, as in "This recording only": the label is too
                // long to share a line with a box at the right edge.
                <div class="setting-row">
                    <label class="setting-label">
                        <input
                            id="hfr-nudge-enabled"
                            type="checkbox"
                            class="setting-checkbox"
                            prop:checked=move || state.dialogs.hfr_nudge_enabled().get()
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                state.dialogs.hfr_nudge_enabled().set(on);
                                crate::settings::set_bool(crate::settings::keys::HFR_NUDGE_ENABLED, on);
                            }
                        />
                        " "{crate::components::hfr_tour::NUDGE_SETTING}
                    </label>
                </div>
                <div class="setting-row">
                    <button
                        class="setting-button"
                        disabled=move || !crate::components::hfr_tour::tour_available(state)
                        title=move || if crate::components::hfr_tour::tour_available(state) {
                            ""
                        } else {
                            crate::components::hfr_tour::TOUR_NEEDS_FILE
                        }
                        on:click=move |_| crate::components::hfr_tour::start_tour(state)
                    >{crate::components::hfr_tour::TOUR_NAME}</button>
                </div>
            </div>

            <div class="setting-group">
                <div class="setting-group-title">"Display"</div>
                <div class="setting-row">
                    <span class="setting-label">"Color scheme"</span>
                    <select
                        class="setting-select"
                        on:change=on_colormap_change
                    >
                        <option value="viridis" selected=move || state.spect.colormap_preference().get() == Colormap::Viridis>"Viridis"</option>
                        <option value="inferno" selected=move || state.spect.colormap_preference().get() == Colormap::Inferno>"Inferno"</option>
                        <option value="magma" selected=move || state.spect.colormap_preference().get() == Colormap::Magma>"Magma"</option>
                        <option value="plasma" selected=move || state.spect.colormap_preference().get() == Colormap::Plasma>"Plasma"</option>
                        <option value="cividis" selected=move || state.spect.colormap_preference().get() == Colormap::Cividis>"Cividis"</option>
                        <option value="turbo" selected=move || state.spect.colormap_preference().get() == Colormap::Turbo>"Turbo"</option>
                        <option value="greyscale" selected=move || state.spect.colormap_preference().get() == Colormap::Greyscale>"Greyscale"</option>
                    </select>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"HFR color scheme"</span>
                    <select
                        class="setting-select"
                        on:change=on_hfr_colormap_change
                    >
                        <option value="viridis" selected=move || state.spect.hfr_colormap_preference().get() == Colormap::Viridis>"Viridis"</option>
                        <option value="inferno" selected=move || state.spect.hfr_colormap_preference().get() == Colormap::Inferno>"Inferno"</option>
                        <option value="magma" selected=move || state.spect.hfr_colormap_preference().get() == Colormap::Magma>"Magma"</option>
                        <option value="plasma" selected=move || state.spect.hfr_colormap_preference().get() == Colormap::Plasma>"Plasma"</option>
                        <option value="cividis" selected=move || state.spect.hfr_colormap_preference().get() == Colormap::Cividis>"Cividis"</option>
                        <option value="turbo" selected=move || state.spect.hfr_colormap_preference().get() == Colormap::Turbo>"Turbo"</option>
                        <option value="greyscale" selected=move || state.spect.hfr_colormap_preference().get() == Colormap::Greyscale>"Greyscale"</option>
                    </select>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Chromagram colors"</span>
                    <select
                        class="setting-select"
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let select: web_sys::HtmlSelectElement = target.unchecked_into();
                            let mode = match select.value().as_str() {
                                "warm" => ChromaColormap::Warm,
                                "solid" => ChromaColormap::Solid,
                                "octave" => ChromaColormap::Octave,
                                "flow" => ChromaColormap::Flow,
                                _ => ChromaColormap::PitchClass,
                            };
                            state.chroma.colormap().set(mode);
                        }
                    >
                        <option value="pitch_class" selected=move || state.chroma.colormap().get() == ChromaColormap::PitchClass>"Pitch Class"</option>
                        <option value="solid" selected=move || state.chroma.colormap().get() == ChromaColormap::Solid>"Solid"</option>
                        <option value="warm" selected=move || state.chroma.colormap().get() == ChromaColormap::Warm>"Warm"</option>
                        <option value="octave" selected=move || state.chroma.colormap().get() == ChromaColormap::Octave>"Octave"</option>
                        <option value="flow" selected=move || state.chroma.colormap().get() == ChromaColormap::Flow>"Flow"</option>
                    </select>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Show clock time"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.timeline.show_clock_time().get()
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            state.timeline.show_clock_time().set(input.checked());
                        }
                        prop:disabled=move || {
                            state.current_file()
                                .and_then(|f| f.recording_start_epoch_ms())
                                .is_none()
                        }
                    />
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Freq flags"</span>
                    <select
                        class="setting-select"
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let select: web_sys::HtmlSelectElement = target.unchecked_into();
                            let style = ShieldStyle::from_key(&select.value());
                            state.viewmode.shield_style().set(style);
                            crate::settings::set_raw(crate::settings::keys::SHIELD_STYLE, style.key());
                        }
                    >
                        {ShieldStyle::ALL.iter().map(|&s| view! {
                            <option
                                value=s.key()
                                selected=move || state.viewmode.shield_style().get() == s
                            >{s.label()}</option>
                        }).collect::<Vec<_>>()}
                    </select>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Status bar"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.panels.show_status_bar().get()
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            let show = input.checked();
                            state.panels.show_status_bar().set(show);
                            crate::settings::set_bool(crate::settings::keys::SHOW_STATUS_BAR, show);
                        }
                    />
                </div>
            </div>

            {move || {
                if state.is_tauri {
                    view! {
                        <div class="setting-group">
                            <div class="setting-group-title">"Location"</div>
                            <div class="setting-row">
                                <span class="setting-label">"Location tags"</span>
                                <input
                                    type="checkbox"
                                    class="setting-checkbox"
                                    prop:checked=move || state.recording_meta.gps_enabled().get()
                                    on:change=move |ev: web_sys::Event| {
                                        let target = ev.target().unwrap();
                                        let input: web_sys::HtmlInputElement = target.unchecked_into();
                                        let checked = input.checked();
                                        state.recording_meta.gps_enabled().set(checked);
                                        crate::settings::set_bool(crate::settings::keys::GPS_ENABLED, checked);
                                    }
                                />
                            </div>
                            <div class="setting-hint">"Add location data to recorded audio"</div>
                            {move || {
                                if !state.recording_meta.gps_enabled().get() {
                                    return view! { <span></span> }.into_any();
                                }
                                let count = state.recording_meta.home_wifi_ssids().with(|list| list.len());
                                if count > 0 {
                                    view! {
                                        <div class="setting-hint" style="color: #8cf;">
                                            {format!("{} privacy zone{} configured", count, if count == 1 { "" } else { "s" })}
                                        </div>
                                    }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            <div style="padding: 4px 12px 8px;">
                                <button
                                    class="analysis-full-btn privacy-settings-btn"
                                    on:click=move |_| { state.dialogs.privacy_settings().set(true); }
                                >"Privacy settings\u{2026}"</button>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}

            <div class="setting-group">
                <div class="setting-group-title">"Selection"</div>
                <div class="setting-row">
                    <span class="setting-label">"Auto-band: Selection"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.annotations.selection_auto_focus().get()
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            state.annotations.selection_auto_focus().set(input.checked());
                        }
                    />
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Auto-band: Annotation"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.annotations.auto_focus().get()
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            state.annotations.auto_focus().set(input.checked());
                        }
                    />
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Auto-band: Bat Book"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.bat_book.auto_focus().get()
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            state.bat_book.auto_focus().set(input.checked());
                        }
                    />
                </div>
            </div>

            <div class="setting-group">
                <div class="setting-group-title">"Beta"</div>
                <div class="setting-row">
                    <span class="setting-label">"Enable projects"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.project.enabled().get()
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            let checked = input.checked();
                            state.project.enabled().set(checked);
                            crate::settings::set_bool(crate::settings::keys::PROJECTS_ENABLED, checked);
                        }
                    />
                </div>
            </div>

        </div>
    }
}

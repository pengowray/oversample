use leptos::prelude::*;
use wasm_bindgen::JsCast;
use crate::canvas::spectrogram_renderer::Colormap;
use crate::state::{AppState, ChromaColormap};

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

#[component]
pub(super) fn ConfigPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_follow_cursor = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        let checked = input.checked();
        state.follow_cursor.set(checked);
        if checked {
            state.follow_suspended.set(false);
            state.follow_visible_since.set(None);
        }
    };

    let on_always_show_view_range = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let input: web_sys::HtmlInputElement = target.unchecked_into();
        state.always_show_view_range.set(input.checked());
    };

    let on_colormap_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let select: web_sys::HtmlSelectElement = target.unchecked_into();
        state.colormap_preference.set(parse_colormap_pref(&select.value()));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    };

    let on_hfr_colormap_change = move |ev: web_sys::Event| {
        let target = ev.target().unwrap();
        let select: web_sys::HtmlSelectElement = target.unchecked_into();
        state.hfr_colormap_preference.set(parse_colormap_pref(&select.value()));
        state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
    };

    let is_tauri = state.is_tauri;
    let _ = is_tauri; // used in the view

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
                        prop:checked=move || state.follow_cursor.get()
                        on:change=on_follow_cursor
                    />
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
                        <option value="viridis" selected=move || state.colormap_preference.get() == Colormap::Viridis>"Viridis"</option>
                        <option value="inferno" selected=move || state.colormap_preference.get() == Colormap::Inferno>"Inferno"</option>
                        <option value="magma" selected=move || state.colormap_preference.get() == Colormap::Magma>"Magma"</option>
                        <option value="plasma" selected=move || state.colormap_preference.get() == Colormap::Plasma>"Plasma"</option>
                        <option value="cividis" selected=move || state.colormap_preference.get() == Colormap::Cividis>"Cividis"</option>
                        <option value="turbo" selected=move || state.colormap_preference.get() == Colormap::Turbo>"Turbo"</option>
                        <option value="greyscale" selected=move || state.colormap_preference.get() == Colormap::Greyscale>"Greyscale"</option>
                    </select>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"HFR color scheme"</span>
                    <select
                        class="setting-select"
                        on:change=on_hfr_colormap_change
                    >
                        <option value="viridis" selected=move || state.hfr_colormap_preference.get() == Colormap::Viridis>"Viridis"</option>
                        <option value="inferno" selected=move || state.hfr_colormap_preference.get() == Colormap::Inferno>"Inferno"</option>
                        <option value="magma" selected=move || state.hfr_colormap_preference.get() == Colormap::Magma>"Magma"</option>
                        <option value="plasma" selected=move || state.hfr_colormap_preference.get() == Colormap::Plasma>"Plasma"</option>
                        <option value="cividis" selected=move || state.hfr_colormap_preference.get() == Colormap::Cividis>"Cividis"</option>
                        <option value="turbo" selected=move || state.hfr_colormap_preference.get() == Colormap::Turbo>"Turbo"</option>
                        <option value="greyscale" selected=move || state.hfr_colormap_preference.get() == Colormap::Greyscale>"Greyscale"</option>
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
                            state.chroma_colormap.set(mode);
                        }
                    >
                        <option value="pitch_class" selected=move || state.chroma_colormap.get() == ChromaColormap::PitchClass>"Pitch Class"</option>
                        <option value="solid" selected=move || state.chroma_colormap.get() == ChromaColormap::Solid>"Solid"</option>
                        <option value="warm" selected=move || state.chroma_colormap.get() == ChromaColormap::Warm>"Warm"</option>
                        <option value="octave" selected=move || state.chroma_colormap.get() == ChromaColormap::Octave>"Octave"</option>
                        <option value="flow" selected=move || state.chroma_colormap.get() == ChromaColormap::Flow>"Flow"</option>
                    </select>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Always show view range"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.always_show_view_range.get()
                        on:change=on_always_show_view_range
                    />
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Show clock time"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.show_clock_time.get()
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            state.show_clock_time.set(input.checked());
                        }
                        prop:disabled=move || {
                            state.current_file()
                                .and_then(|f| f.recording_start_epoch_ms())
                                .is_none()
                        }
                    />
                </div>
                <div class="setting-row">
                    <span class="setting-label">"Max freq"</span>
                    <select
                        class="setting-select"
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let select: web_sys::HtmlSelectElement = target.unchecked_into();
                            let freq = match select.value().as_str() {
                                "auto" => None,
                                v => v.parse::<f64>().ok().map(|khz| khz * 1000.0),
                            };
                            state.max_display_freq.set(freq);
                            state.min_display_freq.set(None);
                        }
                        prop:value=move || match state.max_display_freq.get() {
                            None => "auto".to_string(),
                            Some(hz) => format!("{}", (hz / 1000.0) as u32),
                        }
                    >
                        <option value="auto">"Auto"</option>
                        <option value="50">"50 kHz"</option>
                        <option value="100">"100 kHz"</option>
                        <option value="150">"150 kHz"</option>
                        <option value="200">"200 kHz"</option>
                        <option value="250">"250 kHz"</option>
                    </select>
                </div>
            </div>

            {move || {
                if state.is_mobile.get() {
                    view! {
                        <div class="setting-group">
                            <div class="setting-group-title">"Recording"</div>
                            <div class="setting-row">
                                <span class="setting-label">"Embed GPS location"</span>
                                <input
                                    type="checkbox"
                                    class="setting-checkbox"
                                    prop:checked=move || state.gps_location_enabled.get()
                                    on:change=move |ev: web_sys::Event| {
                                        let target = ev.target().unwrap();
                                        let input: web_sys::HtmlInputElement = target.unchecked_into();
                                        let checked = input.checked();
                                        state.gps_location_enabled.set(checked);
                                        if let Some(ls) = web_sys::window()
                                            .and_then(|w| w.local_storage().ok().flatten())
                                        {
                                            let _ = ls.set_item("oversample_gps_enabled", if checked { "true" } else { "false" });
                                        }
                                    }
                                />
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}

            <div class="setting-group">
                <div class="setting-group-title">"Beta"</div>
                <div class="setting-row">
                    <span class="setting-label">"Enable projects"</span>
                    <input
                        type="checkbox"
                        class="setting-checkbox"
                        prop:checked=move || state.projects_enabled.get()
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                            let checked = input.checked();
                            state.projects_enabled.set(checked);
                            if let Some(ls) = web_sys::window()
                                .and_then(|w| w.local_storage().ok().flatten())
                            {
                                let _ = ls.set_item("oversample_projects_enabled", if checked { "true" } else { "false" });
                            }
                        }
                    />
                </div>
            </div>

        </div>
    }
}

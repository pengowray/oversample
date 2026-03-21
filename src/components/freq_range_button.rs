use leptos::prelude::*;
use crate::state::{AppState, LayerPanel};

fn layer_opt_class(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

/// Short label for the current freq range display.
fn range_label(min_f: Option<f64>, max_f: Option<f64>, file_max: f64) -> &'static str {
    match (min_f, max_f) {
        (None, None) | (Some(0.0), None) => "Full",
        (_, Some(m)) if (m - 22_000.0).abs() < 100.0 => "22k",
        (_, Some(m)) if (m - 50_000.0).abs() < 100.0 => "50k",
        (_, Some(m)) if (m - 100_000.0).abs() < 100.0 => "100k",
        (_, Some(m)) if (m - file_max).abs() < 100.0 => "Full",
        _ => "Custom",
    }
}

#[component]
pub fn FreqRangeButton() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::FreqRange);

    let file_max = move || {
        let files = state.files.get();
        let idx = state.current_file_index.get();
        idx.and_then(|i| files.get(i))
            .map(|f| f.spectrogram.max_freq)
            .unwrap_or(96_000.0)
    };

    let visible = move || {
        if state.always_show_view_range.get() {
            return true;
        }
        let min_f = state.min_display_freq.get();
        let max_f = state.max_display_freq.get();
        let fm = file_max();
        let is_full = match (min_f, max_f) {
            (None, None) | (Some(0.0), None) => true,
            (_, Some(m)) if (m - fm).abs() < 100.0 => {
                min_f.is_none() || min_f == Some(0.0)
            }
            _ => false,
        };
        !is_full
    };

    let set_range = move |lo: Option<f64>, hi: Option<f64>| {
        move |_: web_sys::MouseEvent| {
            state.min_display_freq.set(lo);
            state.max_display_freq.set(hi);
        }
    };

    view! {
        <div
            style=move || format!("position: absolute; top: 46px; left: 56px; pointer-events: none; z-index: 20; opacity: {}; transition: opacity 0.1s;{}",
                if state.mouse_in_label_area.get() { "0" } else { "1" },
                if visible() { "" } else { " display: none;" })
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:touchstart=|ev: web_sys::TouchEvent| ev.stop_propagation()
        >
            <div style=move || format!("position: relative; pointer-events: {};",
                if state.mouse_in_label_area.get() { "none" } else { "auto" })>
                <button
                    class=move || if is_open() { "layer-btn open" } else { "layer-btn" }
                    on:click=move |_| toggle_panel(&state, LayerPanel::FreqRange)
                    title="Frequency range (Shift+scroll to zoom)"
                >
                    <span class="layer-btn-category">"Range"</span>
                    <span class="layer-btn-value">{move || {
                        range_label(
                            state.min_display_freq.get(),
                            state.max_display_freq.get(),
                            file_max(),
                        )
                    }}</span>
                </button>
                <Show when=move || is_open()>
                    <div class="layer-panel" style="left: 0; top: 34px; min-width: 140px;">
                        <div class="layer-panel-title">"Freq Range"</div>
                        <button class=move || {
                            let cur_max = state.max_display_freq.get();
                            let fm = file_max();
                            let full = cur_max.is_none() || cur_max == Some(fm);
                            layer_opt_class(full)
                        }
                            on:click=set_range(None, None)
                        >"Full"</button>
                        <button class=move || {
                            let is_22k = state.max_display_freq.get().is_some_and(|m| (m - 22_000.0).abs() < 100.0);
                            layer_opt_class(is_22k)
                        }
                            on:click=set_range(Some(0.0), Some(22_000.0))
                        >"0 – 22 kHz"</button>
                        <button class=move || {
                            let is_50k = state.max_display_freq.get().is_some_and(|m| (m - 50_000.0).abs() < 100.0);
                            layer_opt_class(is_50k)
                        }
                            on:click=set_range(Some(0.0), Some(50_000.0))
                        >"0 – 50 kHz"</button>
                        <button class=move || {
                            let is_100k = state.max_display_freq.get().is_some_and(|m| (m - 100_000.0).abs() < 100.0);
                            layer_opt_class(is_100k)
                        }
                            on:click=set_range(Some(0.0), Some(100_000.0))
                        >"0 – 100 kHz"</button>
                    </div>
                </Show>
            </div>
        </div>
    }
}

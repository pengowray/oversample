use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use crate::state::{AppState, CanvasTool, LayerPanel, PlayStartMode, RecordMode};
use crate::audio::{microphone, playback};
use crate::components::hfr_button::HfrButton;
use crate::components::combo_button::ComboButton;

fn layer_opt_class(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

#[component]
pub fn BottomToolbar() -> impl IntoView {
    let state = expect_context::<AppState>();
    let has_file = move || state.current_file_index.get().is_some();
    let is_mobile = state.is_mobile.get_untracked();

    // ── Recording timer ──
    let interval_id: StoredValue<Option<i32>> = StoredValue::new(None);
    Effect::new(move |_| {
        let recording = state.mic_recording.get();
        if recording {
            let cb = Closure::<dyn FnMut()>::new(move || {
                state.mic_timer_tick.update(|n| *n = n.wrapping_add(1));
            });
            if let Some(window) = web_sys::window() {
                if let Ok(id) = window.set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(), 100,
                ) {
                    interval_id.set_value(Some(id));
                }
            }
            cb.forget();
        } else {
            if let Some(id) = interval_id.get_value() {
                if let Some(window) = web_sys::window() {
                    window.clear_interval_with_handle(id);
                }
                interval_id.set_value(None);
            }
        }
    });

    // ── Play ComboButton setup ──
    let play_is_open = Signal::derive(move || state.layer_panel_open.get() == Some(LayerPanel::PlayMode));

    let play_left_class = Signal::derive(move || {
        let playing = state.is_playing.get();
        let open = play_is_open.get();
        match (playing, open) {
            (true, true) => "layer-btn combo-btn-left active open",
            (true, false) => "layer-btn combo-btn-left active",
            (false, true) => "layer-btn combo-btn-left open",
            (false, false) => "layer-btn combo-btn-left",
        }
    });
    let play_right_class = Signal::derive(move || {
        if play_is_open.get() { "layer-btn combo-btn-right open" } else { "layer-btn combo-btn-right" }
    });

    let play_left_value = Signal::derive(move || "\u{25B6}".to_string()); // ▶
    let play_right_value = Signal::derive(move || {
        match state.play_start_mode.get() {
            PlayStartMode::All => "All".to_string(),
            PlayStartMode::FromHere => "Here".to_string(),
            PlayStartMode::Selected => "Sel".to_string(),
        }
    });

    let play_left_click = Callback::new(move |_: web_sys::MouseEvent| {
        if state.is_playing.get_untracked() {
            playback::stop(&state);
        } else {
            match state.play_start_mode.get_untracked() {
                PlayStartMode::All => playback::play_from_start(&state),
                PlayStartMode::FromHere => playback::play_from_here(&state),
                PlayStartMode::Selected => {
                    if state.selection.get_untracked().is_some() {
                        playback::play(&state);
                    } else {
                        playback::play_from_start(&state);
                    }
                }
            }
        }
    });
    let play_toggle_menu = Callback::new(move |()| {
        toggle_panel(&state, LayerPanel::PlayMode);
    });

    // ── Record ComboButton setup ──
    let rec_is_open = Signal::derive(move || state.layer_panel_open.get() == Some(LayerPanel::RecordMode));

    let rec_left_class = Signal::derive(move || {
        let recording = state.mic_recording.get();
        let listen_only = state.record_mode.get() == RecordMode::ListenOnly;
        let open = rec_is_open.get();
        if recording {
            if open { "layer-btn combo-btn-left mic-recording open" } else { "layer-btn combo-btn-left mic-recording" }
        } else if listen_only {
            if open { "layer-btn combo-btn-left disabled open" } else { "layer-btn combo-btn-left disabled" }
        } else {
            if open { "layer-btn combo-btn-left open" } else { "layer-btn combo-btn-left" }
        }
    });
    let rec_right_class = Signal::derive(move || {
        if rec_is_open.get() { "layer-btn combo-btn-right open" } else { "layer-btn combo-btn-right" }
    });

    let rec_left_value = Signal::derive(move || {
        if state.mic_recording.get() {
            let _ = state.mic_timer_tick.get();
            let start = state.mic_recording_start_time.get_untracked().unwrap_or(0.0);
            let now = js_sys::Date::now();
            let secs = (now - start) / 1000.0;
            format!("Rec {:.1}s", secs)
        } else {
            "\u{23FA}".to_string() // ⏺
        }
    });
    let rec_right_value = Signal::derive(move || {
        match state.record_mode.get() {
            RecordMode::ToFile => "File".to_string(),
            RecordMode::ToMemory => "Mem".to_string(),
            RecordMode::ListenOnly => "Listen".to_string(),
        }
    });

    let rec_left_click = Callback::new(move |_: web_sys::MouseEvent| {
        if state.record_mode.get_untracked() == RecordMode::ListenOnly {
            return; // greyed out
        }
        let st = state;
        wasm_bindgen_futures::spawn_local(async move {
            microphone::toggle_record(&st).await;
        });
    });
    let rec_toggle_menu = Callback::new(move |()| {
        toggle_panel(&state, LayerPanel::RecordMode);
    });

    view! {
        <div class=if is_mobile { "bottom-toolbar mobile" } else { "bottom-toolbar" }
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            // ── HFR combo button ──
            <HfrButton />

            <div class="bottom-toolbar-sep"></div>

            // ── Play combo button ──
            {move || has_file().then(|| view! {
                <ComboButton
                    left_label=""
                    left_value=play_left_value
                    left_click=play_left_click
                    left_class=play_left_class
                    right_value=play_right_value
                    right_class=play_right_class
                    is_open=play_is_open
                    toggle_menu=play_toggle_menu
                    left_title="Play / Stop"
                    right_title="Play mode"
                    menu_direction="above"
                    panel_style="min-width: 180px;"
                >
                    <button class=move || layer_opt_class(state.play_start_mode.get() == PlayStartMode::All)
                        on:click=move |_| {
                            state.play_start_mode.set(PlayStartMode::All);
                            state.layer_panel_open.set(None);
                        }
                    >"All \u{2014} Play from start"</button>
                    <button class=move || layer_opt_class(state.play_start_mode.get() == PlayStartMode::FromHere)
                        on:click=move |_| {
                            state.play_start_mode.set(PlayStartMode::FromHere);
                            state.layer_panel_open.set(None);
                        }
                    >"From here \u{2014} Current position"</button>
                    <button
                        class=move || {
                            let active = state.play_start_mode.get() == PlayStartMode::Selected;
                            let enabled = state.canvas_tool.get() == CanvasTool::Selection
                                && state.selection.get().is_some();
                            if !enabled {
                                "layer-panel-opt disabled"
                            } else if active {
                                "layer-panel-opt sel"
                            } else {
                                "layer-panel-opt"
                            }
                        }
                        on:click=move |_| {
                            let enabled = state.canvas_tool.get_untracked() == CanvasTool::Selection
                                && state.selection.get_untracked().is_some();
                            if enabled {
                                state.play_start_mode.set(PlayStartMode::Selected);
                                state.layer_panel_open.set(None);
                            }
                        }
                    >"Selected \u{2014} Play selection"</button>
                </ComboButton>
            })}

            // Gain toggle
            {move || has_file().then(|| {
                let auto = state.auto_gain.get();
                let db = if auto {
                    state.compute_auto_gain()
                } else {
                    state.gain_db.get()
                };
                let label = if auto {
                    "Auto".to_string()
                } else if db > 0.0 {
                    format!("+{:.0}dB", db)
                } else {
                    format!("{:.0}dB", db)
                };
                view! {
                    <button
                        class=move || if state.auto_gain.get() { "layer-btn active" } else { "layer-btn" }
                        on:click=move |_| state.auto_gain.update(|v| *v = !*v)
                        title="Toggle auto gain"
                    >
                        <span class="layer-btn-category">"Gain"</span>
                        <span class="layer-btn-value">{label}</span>
                    </button>
                }
            })}

            <div class="bottom-toolbar-sep"></div>

            // ── Record combo button ──
            <ComboButton
                left_label=""
                left_value=rec_left_value
                left_click=rec_left_click
                left_class=rec_left_class
                right_value=rec_right_value
                right_class=rec_right_class
                is_open=rec_is_open
                toggle_menu=rec_toggle_menu
                left_title="Record"
                right_title="Record mode"
                menu_direction="above"
                panel_style="min-width: 160px;"
            >
                <button
                    class=move || {
                        let active = state.record_mode.get() == RecordMode::ToFile;
                        if !state.is_tauri {
                            "layer-panel-opt disabled"
                        } else if active {
                            "layer-panel-opt sel"
                        } else {
                            "layer-panel-opt"
                        }
                    }
                    on:click=move |_| {
                        if state.is_tauri {
                            state.record_mode.set(RecordMode::ToFile);
                            state.layer_panel_open.set(None);
                        }
                    }
                >"To file"</button>
                <button class=move || layer_opt_class(state.record_mode.get() == RecordMode::ToMemory)
                    on:click=move |_| {
                        state.record_mode.set(RecordMode::ToMemory);
                        state.layer_panel_open.set(None);
                    }
                >"To memory"</button>
                <button class=move || layer_opt_class(state.record_mode.get() == RecordMode::ListenOnly)
                    on:click=move |_| {
                        // If currently recording, finish and switch to listening
                        if state.mic_recording.get_untracked() {
                            let st = state;
                            wasm_bindgen_futures::spawn_local(async move {
                                microphone::toggle_record(&st).await; // stops recording
                                microphone::toggle_listen(&st).await; // starts listening
                            });
                        }
                        state.record_mode.set(RecordMode::ListenOnly);
                        state.layer_panel_open.set(None);
                    }
                >"Listen only"</button>
            </ComboButton>

            // ── Listen button ──
            <button
                class=move || if state.mic_listening.get() { "layer-btn mic-armed" } else { "layer-btn" }
                on:click=move |_| {
                    let st = state;
                    wasm_bindgen_futures::spawn_local(async move {
                        microphone::toggle_listen(&st).await;
                    });
                }
                title=move || if state.mic_needs_permission.get() && state.is_tauri {
                    "Grant USB mic permission to start listening"
                } else {
                    "Toggle live listening (L)"
                }
            >
                <span class="layer-btn-category">"Mic"</span>
                <span class="layer-btn-value">{move || if state.mic_needs_permission.get() && state.is_tauri && !state.mic_listening.get() {
                    "USB mic"
                } else {
                    "Listen"
                }}</span>
            </button>

            <div class="bottom-toolbar-sep"></div>

            // ── Tool button (Hand / Selection) ──
            <ToolButtonInline />
        </div>
    }
}

/// Tool button adapted for inline use in the bottom toolbar (no absolute positioning).
#[component]
fn ToolButtonInline() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::Tool);

    view! {
        <div style="position: relative;">
            <button
                class=move || if is_open() { "layer-btn open" } else { "layer-btn" }
                on:click=move |_| toggle_panel(&state, LayerPanel::Tool)
                title="Tool"
            >
                <span class="layer-btn-category">"Tool"</span>
                <span class="layer-btn-value">{move || match state.canvas_tool.get() {
                    CanvasTool::Hand => "Hand",
                    CanvasTool::Selection => "Select",
                }}</span>
            </button>
            {move || is_open().then(|| view! {
                <div class="layer-panel" style="bottom: calc(100% + 4px); right: 0;">
                    <div class="layer-panel-title">"Tool"</div>
                    <button
                        class=move || layer_opt_class(state.canvas_tool.get() == CanvasTool::Hand)
                        on:click=move |_| {
                            state.canvas_tool.set(CanvasTool::Hand);
                            state.layer_panel_open.set(None);
                        }
                    >"Hand (pan)"</button>
                    <button
                        class=move || layer_opt_class(state.canvas_tool.get() == CanvasTool::Selection)
                        on:click=move |_| {
                            state.canvas_tool.set(CanvasTool::Selection);
                            state.layer_panel_open.set(None);
                        }
                    >"Selection"</button>
                </div>
            })}
        </div>
    }
}

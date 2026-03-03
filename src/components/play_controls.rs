use leptos::prelude::*;
use crate::state::{AppState, StatusLevel};
use crate::audio::playback;

/// App-level toast display — always visible regardless of whether a file is open.
#[component]
pub fn ToastDisplay() -> impl IntoView {
    let state = expect_context::<AppState>();

    view! {
        <div class="app-toast-container">
            {move || state.status_message.get().map(|msg| {
                let state2 = state;
                wasm_bindgen_futures::spawn_local(async move {
                    let p = js_sys::Promise::new(&mut |resolve, _| {
                        if let Some(w) = web_sys::window() {
                            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 3000);
                        }
                    });
                    wasm_bindgen_futures::JsFuture::from(p).await.ok();
                    state2.status_message.set(None);
                });
                let cls = if state.status_level.get_untracked() == StatusLevel::Info {
                    "status-toast status-toast-info"
                } else {
                    "status-toast"
                };
                view! {
                    <span class=cls>{msg}</span>
                }
            })}
        </div>
    }
}

#[component]
pub fn PlayControls() -> impl IntoView {
    let state = expect_context::<AppState>();
    let has_file = move || state.current_file_index.get().is_some();
    let is_playing = move || state.is_playing.get();

    let state_play = state.clone();
    let on_play_start = move |_| {
        playback::play_from_start(&state_play);
    };

    let state_here = state.clone();
    let on_play_here = move |_| {
        playback::play_from_here(&state_here);
    };

    let state_stop = state.clone();
    let on_stop = move |_| {
        playback::stop(&state_stop);
    };

    view! {
        <div class="play-controls"
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            // Play/Stop buttons (when a file is loaded)
            {move || if !has_file() {
                view! { <span></span> }.into_any()
            } else if is_playing() {
                view! {
                    <button class="layer-btn" on:click=on_stop.clone()>"Stop"</button>
                }.into_any()
            } else {
                view! {
                    <button class="layer-btn" on:click=on_play_start.clone()
                        title="Play from start of file"
                    >"Play start"</button>
                    <button class="layer-btn" on:click=on_play_here.clone()
                        title="Play from current position"
                    >"Play here"</button>
                }.into_any()
            }}

            // Gain toggle (auto / manual)
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

            // Bookmark popup
            {move || state.show_bookmark_popup.get().then(|| {
                let bms = state.bookmarks.get();
                let recent: Vec<_> = bms.iter().rev().take(8).cloned().collect();
                view! {
                    <div class="bookmark-popup"
                        on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                    >
                        <div class="bookmark-popup-title">"Bookmarks"</div>
                        {recent.into_iter().map(|bm| {
                            let t = bm.time;
                            let state2 = state.clone();
                            view! {
                                <button class="bookmark-item"
                                    on:click=move |_| {
                                        let zoom = state2.zoom_level.get_untracked();
                                        let files = state2.files.get_untracked();
                                        let idx = state2.current_file_index.get_untracked();
                                        let time_res = idx.and_then(|i| files.get(i))
                                            .map(|f| f.spectrogram.time_resolution)
                                            .unwrap_or(0.001);
                                        let canvas_w = 800.0_f64;
                                        let visible_time = (canvas_w / zoom) * time_res;
                                        let new_scroll = (t - visible_time * 0.1).max(0.0);
                                        state2.suspend_follow();
                                        state2.scroll_offset.set(new_scroll);
                                        state2.show_bookmark_popup.set(false);
                                    }
                                >{format!("{:.2}s", t)}</button>
                            }
                        }).collect_view()}
                        <button class="bookmark-popup-close"
                            on:click=move |_| state.show_bookmark_popup.set(false)
                        >"Dismiss"</button>
                    </div>
                }
            })}
        </div>
    }
}

use crate::state::store_fields::*;
use crate::state::{AppState, StatusLevel};
use crate::viewport;
use leptos::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// How long a status message stays on screen.
const TOAST_MS: i32 = 3000;

/// App-level toast display — always visible regardless of whether a file is open.
#[component]
pub fn ToastDisplay() -> impl IntoView {
    let state = expect_context::<AppState>();
    let toast_count = Arc::new(AtomicU64::new(0));

    view! {
        <div class="app-toast-container">
            // Persistent "Buffering…" indicator while the streaming decoder
            // is behind the playhead. Distinct from transient status toasts
            // so it doesn't auto-dismiss.
            {move || state.playback.is_buffering().get().then(|| {
                view! {
                    <span class="status-toast status-toast-info">"Buffering\u{2026}"</span>
                }
            })}
            {move || state.dialogs.notif_hidden_notice().get().then(|| {
                // Names match the notification's own titles (ForegroundAudioService.kt).
                let title = if state.mic.recording().get() {
                    "\u{201c}Recording audio\u{201d}"
                } else {
                    "\u{201c}Listening (live)\u{201d}"
                };
                let on_open = move |_: web_sys::MouseEvent| {
                    state.dialogs.notif_hidden_notice().set(false);
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ = crate::tauri_bridge::tauri_invoke_no_args(
                            "plugin:audio-service|openNotificationSettings",
                        )
                        .await;
                    });
                };
                let on_close = move |_: web_sys::MouseEvent| {
                    state.dialogs.notif_hidden_notice().set(false);
                    state.dialogs.notif_hidden_notice_dismissed().set(true);
                    crate::settings::set_bool(
                        crate::settings::keys::NOTIF_HIDDEN_NOTICE_DISMISSED,
                        true,
                    );
                };
                view! {
                    <div class="capture-notice" role="status">
                        <span class="capture-notice-text">"Cannot show " {title} " notification"</span>
                        <button class="capture-notice-btn" on:click=on_open>"Open settings"</button>
                        <button class="capture-notice-close"
                            title="Close and don\u{2019}t show again"
                            aria-label="Close and don\u{2019}t show again"
                            on:click=on_close>"\u{00d7}"</button>
                    </div>
                }
            })}
            {move || state.dialogs.mic_access_notice().get().then(|| {
                let on_open = move |_: web_sys::MouseEvent| {
                    state.dialogs.mic_access_notice().set(false);
                    wasm_bindgen_futures::spawn_local(async move {
                        let _ = crate::tauri_bridge::tauri_invoke_no_args(
                            "plugin:audio-service|openAppSettings",
                        )
                        .await;
                    });
                };
                let on_close = move |_: web_sys::MouseEvent| {
                    state.dialogs.mic_access_notice().set(false);
                };
                view! {
                    <div class="capture-notice" role="alert">
                        <span class="capture-notice-text">"Microphone not allowed for Oversample"</span>
                        <button class="capture-notice-btn" on:click=on_open>"Open settings"</button>
                        <button class="capture-notice-close"
                            title="Close"
                            aria-label="Close"
                            on:click=on_close>"\u{00d7}"</button>
                    </div>
                }
            })}
            {move || {
                // Every change to the status message, from any caller, runs
                // this closure and gets a new number. A message's timer clears
                // the message only if no newer message has been set since, so
                // a newer message is shown for its full 3 seconds.
                let shown = toast_count.fetch_add(1, Ordering::Relaxed) + 1;
                let msg = state.status.message().get()?;
                let count = toast_count.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    crate::web_util::sleep_ms(TOAST_MS).await;
                    if count.load(Ordering::Relaxed) == shown {
                        state.status.message().set(None);
                    }
                });
                let cls = if state.status.level().get_untracked() == StatusLevel::Info {
                    "status-toast status-toast-info"
                } else {
                    "status-toast"
                };
                Some(view! {
                    <span class=cls>{msg}</span>
                })
            }}
        </div>
    }
}

/// Bookmark popup — floated in main-overlays so it appears over the canvas.
#[component]
pub fn BookmarkPopup() -> impl IntoView {
    let state = expect_context::<AppState>();

    view! {
        {move || state.dialogs.bookmark_popup().get().then(|| {
            let bms = state.viewmode.bookmarks().get();
            let recent: Vec<_> = bms.iter().rev().take(8).cloned().collect();
            view! {
                <div class="bookmark-popup"
                    on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                >
                    <div class="bookmark-popup-title">"Bookmarks"</div>
                    {recent.into_iter().map(|bm| {
                        let t = bm.time;
                        let state2 = state;
                        view! {
                            <button class="bookmark-item"
                                on:click=move |_| {
                                    let zoom = state2.view.zoom_level().get_untracked();
                                    let files = state2.library.files().get_untracked();
                                    let idx = state2.library.current_index().get_untracked();
                                    let time_res = idx.and_then(|i| files.get(i))
                                        .map(|f| f.spectrogram.time_resolution)
                                        .unwrap_or(0.001);
                                    let canvas_w = 800.0_f64;
                                    let visible_time = viewport::visible_time(canvas_w, zoom, time_res);
                                    let new_scroll = (t - visible_time * 0.1).max(0.0);
                                    state2.suspend_follow();
                                    state2.view.scroll_offset().set(new_scroll);
                                    state2.dialogs.bookmark_popup().set(false);
                                }
                            >{crate::format_time::format_time_display(t, 2)}</button>
                        }
                    }).collect_view()}
                    <button class="bookmark-popup-close"
                        on:click=move |_| state.dialogs.bookmark_popup().set(false)
                    >"Dismiss"</button>
                </div>
            }
        })}
    }
}

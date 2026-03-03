use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use crate::state::AppState;
use crate::audio::microphone;

#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<AppState>();
    let show_about = RwSignal::new(false);

    let is_mobile = state.is_mobile.get_untracked();

    // Recording timer: start/stop a 100ms setInterval to tick the timer signal
    let interval_id: StoredValue<Option<i32>> = StoredValue::new(None);
    Effect::new(move |_| {
        let recording = state.mic_recording.get();
        if recording {
            // Start 100ms interval to update timer
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
            cb.forget(); // leak the closure — cleared when recording stops
        } else {
            // Clear interval
            if let Some(id) = interval_id.get_value() {
                if let Some(window) = web_sys::window() {
                    window.clear_interval_with_handle(id);
                }
                interval_id.set_value(None);
            }
        }
    });

    view! {
        <div class="toolbar">
            {if is_mobile {
                Some(view! {
                    <button
                        class="toolbar-menu-btn"
                        on:click=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            state.sidebar_collapsed.update(|c| *c = !*c);
                        }
                        title="Menu"
                    >"\u{2630}"</button>
                })
            } else {
                None
            }}
            <span
                class="toolbar-brand"
                style=move || if !is_mobile && state.sidebar_collapsed.get() { "margin-left: 24px; cursor: pointer" } else { "cursor: pointer" }
                on:click=move |_| show_about.set(true)
                title="About"
            ><b>"Batmonic"</b></span>

            // Spacer
            <div style="flex: 1;"></div>

            // Listen button
            <button
                class=move || if state.mic_listening.get() { "toolbar-listen-btn active" } else { "toolbar-listen-btn" }
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
            >{move || if state.mic_needs_permission.get() && state.is_tauri && !state.mic_listening.get() {
                "Allow USB mic"
            } else {
                "Listen"
            }}</button>

            // Record button
            <button
                class=move || if state.mic_recording.get() { "toolbar-record-btn active" } else { "toolbar-record-btn" }
                on:click=move |_| {
                    let st = state;
                    wasm_bindgen_futures::spawn_local(async move {
                        microphone::toggle_record(&st).await;
                    });
                }
                title=move || if state.mic_needs_permission.get() && state.is_tauri {
                    "Grant USB mic permission to start recording"
                } else {
                    "Toggle recording (R)"
                }
            >
                {move || if state.mic_recording.get() {
                    let _ = state.mic_timer_tick.get(); // subscribe to 100ms tick
                    let start = state.mic_recording_start_time.get_untracked().unwrap_or(0.0);
                    let now = js_sys::Date::now();
                    let secs = (now - start) / 1000.0;
                    format!("Rec {:.1}s", secs)
                } else if state.mic_needs_permission.get() && state.is_tauri {
                    "Allow USB mic".to_string()
                } else {
                    "Record".to_string()
                }}
            </button>

            // Right sidebar button (mobile only, after Record)
            {if is_mobile {
                Some(view! {
                    <button
                        class="toolbar-menu-btn"
                        on:click=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            state.right_sidebar_collapsed.update(|c| *c = !*c);
                            // Close left sidebar when opening right
                            if !state.right_sidebar_collapsed.get_untracked() {
                                state.sidebar_collapsed.set(true);
                            }
                        }
                        title="Info panel"
                    >"\u{2630}"</button>
                })
            } else {
                None
            }}

            {move || show_about.get().then(|| view! {
                <div class="about-overlay" on:click=move |_| show_about.set(false)>
                    <div class="about-dialog" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="about-header">
                            <span class="about-title"><b>"Batmonic"</b>" by Pengo Wray"</span>
                            <span class="about-version">{concat!("v", env!("CARGO_PKG_VERSION"))}</span>
                        </div>
                        <p class="about-desc">"Bat call viewer and acoustic analysis tool."</p>
                        <div style="margin-top: 12px; font-size: 11px; color: #999; line-height: 1.8;">
                            "Thanks to the libraries that make this possible:"
                            <div style="margin-top: 6px; columns: 2; column-gap: 16px;">
                                <div><a href="https://leptos.dev" target="_blank" style="color: #8cf; text-decoration: none;">"Leptos"</a>""</div>
                                <div><a href="https://crates.io/crates/realfft" target="_blank" style="color: #8cf; text-decoration: none;">"RealFFT"</a></div>
                                <div><a href="https://crates.io/crates/hound" target="_blank" style="color: #8cf; text-decoration: none;">"Hound"</a></div>
                                <div><a href="https://crates.io/crates/claxon" target="_blank" style="color: #8cf; text-decoration: none;">"Claxon"</a></div>
                                <div><a href="https://crates.io/crates/lewton" target="_blank" style="color: #8cf; text-decoration: none;">"Lewton"</a></div>
                                <div><a href="https://crates.io/crates/symphonia" target="_blank" style="color: #8cf; text-decoration: none;">"Symphonia"</a></div>
                                <div><a href="https://github.com/jmears63/batgizmo-app-public" target="_blank" style="color: #8cf; text-decoration: none;">"batgizmo-app"</a></div>
                            </div>
                        </div>
                        <button class="about-close" on:click=move |_| show_about.set(false)>"Close"</button>
                    </div>
                </div>
            })}
        </div>
    }
}

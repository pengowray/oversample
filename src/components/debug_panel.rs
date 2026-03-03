use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use crate::state::AppState;

#[component]
pub fn DebugPanel() -> impl IntoView {
    let state = expect_context::<AppState>();
    let container_ref = NodeRef::<leptos::html::Div>::new();

    // Auto-scroll to bottom when entries change
    Effect::new(move |_| {
        let entries = state.debug_log_entries.get();
        let _ = entries.len(); // subscribe
        if let Some(el) = container_ref.get() {
            let el: &web_sys::HtmlElement = &el;
            el.set_scroll_top(el.scroll_height());
        }
    });

    let on_copy = move |_| {
        let entries = state.debug_log_entries.get_untracked();
        let text: String = entries.iter().map(|(ts, level, msg)| {
            let secs = (ts / 1000.0) % 100000.0;
            format!("[{:.1}s] [{}] {}", secs, level, msg)
        }).collect::<Vec<_>>().join("\n");
        if let Some(window) = web_sys::window() {
            if let Ok(nav) = js_sys::Reflect::get(&window, &JsValue::from_str("navigator")) {
                if let Ok(clip) = js_sys::Reflect::get(&nav, &JsValue::from_str("clipboard")) {
                    let _ = js_sys::Reflect::get(&clip, &JsValue::from_str("writeText"))
                        .ok()
                        .and_then(|f| f.dyn_ref::<js_sys::Function>().cloned())
                        .map(|f| f.call1(&clip, &JsValue::from_str(&text)));
                    state.show_info_toast("Debug log copied");
                }
            }
        }
    };

    let on_clear = move |_| {
        state.debug_log_entries.update(|e| e.clear());
    };

    // Compute start_time from first entry (or now) for relative timestamps
    let start_time = js_sys::Date::now();

    view! {
        <div class="sidebar-panel debug-panel">
            <div class="debug-panel-toolbar">
                <button class="setting-btn" on:click=on_copy>"Copy All"</button>
                <button class="setting-btn" on:click=on_clear>"Clear"</button>
            </div>
            <div class="debug-panel-log" node_ref=container_ref>
                {move || {
                    let entries = state.debug_log_entries.get();
                    if entries.is_empty() {
                        return view! {
                            <div class="debug-panel-empty">"No log entries yet"</div>
                        }.into_any();
                    }
                    let items: Vec<_> = entries.iter().map(|(ts, level, msg)| {
                        let relative = (*ts - start_time) / 1000.0;
                        let time_str = format!("{:+.1}s", relative);
                        let level_cls = match level.as_str() {
                            "error" => "debug-level-error",
                            "warn" => "debug-level-warn",
                            _ => "debug-level-info",
                        };
                        view! {
                            <div class="debug-entry">
                                <span class="debug-time">{time_str}</span>
                                <span class=level_cls>{format!("[{}]", level)}</span>
                                <span class="debug-msg">{msg.clone()}</span>
                            </div>
                        }
                    }).collect();
                    view! { <div>{items}</div> }.into_any()
                }}
            </div>
        </div>
    }
}

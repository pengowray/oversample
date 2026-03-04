use leptos::prelude::*;
use crate::state::AppState;
use crate::bat_book::data::get_manifest;

/// Floating reference panel on the right side of the main view.
/// Shows info about the selected bat family.
#[component]
pub fn BatBookRefPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let selected_entry = Memo::new(move |_| {
        let sel_id = state.bat_book_selected_id.get()?;
        let region = state.bat_book_region.get();
        let manifest = get_manifest(region);
        manifest.entries.into_iter().find(|e| e.id == sel_id)
    });

    let on_close = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        state.bat_book_ref_open.set(false);
    };

    // Swipe-up to dismiss
    let touch_start_y = RwSignal::new(0.0f64);
    let on_touchstart = move |ev: web_sys::TouchEvent| {
        if let Some(touch) = ev.touches().get(0) {
            touch_start_y.set(touch.client_y() as f64);
        }
    };
    let on_touchend = move |ev: web_sys::TouchEvent| {
        if let Some(touch) = ev.changed_touches().get(0) {
            let dy = touch_start_y.get_untracked() - touch.client_y() as f64;
            if dy > 60.0 {
                state.bat_book_ref_open.set(false);
            }
        }
    };

    view! {
        <div
            class="bat-book-ref-panel"
            on:touchstart=on_touchstart
            on:touchend=on_touchend
        >
            <div class="ref-panel-header">
                <span class="ref-panel-name">
                    {move || selected_entry.get().map(|e| e.name.to_string()).unwrap_or_default()}
                </span>
                <button class="ref-panel-close" on:click=on_close title="Close">
                    "\u{00d7}"
                </button>
            </div>
            <div class="ref-panel-body">
                {move || selected_entry.get().map(|entry| {
                    view! {
                        <div class="ref-panel-family">{entry.family}</div>
                        <div class="ref-panel-freq">{entry.freq_range_label()}</div>
                        <div class="ref-panel-call-type">"Call type: " {entry.call_type}</div>
                        <div class="ref-panel-desc">{entry.description}</div>
                    }
                })}
            </div>
        </div>
    }
}

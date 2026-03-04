use leptos::prelude::*;
use crate::state::AppState;

/// Small vertical tab handle on the left edge of the main area.
/// Toggles the bat book strip open/closed.
#[component]
pub fn BatBookTab() -> impl IntoView {
    let state = expect_context::<AppState>();

    let on_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        state.bat_book_open.update(|v| *v = !*v);
    };

    let class = move || {
        if state.bat_book_open.get() {
            "bat-book-tab open"
        } else {
            "bat-book-tab"
        }
    };

    view! {
        <button
            class=class
            on:click=on_click
            title="Bat Book"
        >
            <span class="bat-book-tab-label">"Bat Book"</span>
        </button>
    }
}

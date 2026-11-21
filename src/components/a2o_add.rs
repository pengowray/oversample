//! "Add before" and "Add after" for an Australian Acoustic Observatory clip:
//! two buttons in the transport bar that fetch the audio just before or just
//! after what is open and put it on the timeline (`crate::a2o::add_adjacent`).
//!
//! Shown only while the open file, or every file on the open timeline, is a
//! clip of one A2O recording.

use crate::a2o;
use crate::state::AppState;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn A2oAddButtons() -> impl IntoView {
    let state = expect_context::<AppState>();
    // True while a clip is being fetched: both buttons wait for it.
    let busy = RwSignal::new(false);
    let add = move |before: bool| {
        move |_: web_sys::MouseEvent| {
            if busy.get_untracked() {
                return;
            }
            busy.set(true);
            spawn_local(async move {
                if let Err(e) = a2o::add_adjacent(state, before).await {
                    state.show_error_toast(e);
                }
                busy.try_set(false);
            });
        }
    };
    let secs = a2o::ADD_SECS as u32;
    move || {
        let stretch = a2o::open_stretch(&state)?;
        // At the start or the end of the recording there is nothing more to
        // add on that side, so that button is left out.
        let button = |before: bool| {
            stretch.can_add(before).then(|| {
                let (label, tip) = if before {
                    (
                        format!("Add {secs}\u{a0}s before"),
                        format!("Get the {secs} s before this audio from A2O and add it to the timeline"),
                    )
                } else {
                    (
                        format!("Add {secs}\u{a0}s after"),
                        format!("Get the {secs} s after this audio from A2O and add it to the timeline"),
                    )
                };
                view! {
                    <button class="a2o-add-btn" title=tip
                        disabled=move || busy.get()
                        on:click=add(before)
                    >{label}</button>
                }
            })
        };
        Some(view! {
            <div class="a2o-add" class:busy=move || busy.get()>
                {button(true)}
                {button(false)}
            </div>
        })
    }
}

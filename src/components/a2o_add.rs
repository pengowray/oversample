//! "Add before" and "Add after" for an Australian Acoustic Observatory clip:
//! two buttons in the transport bar that fetch the audio just before or just
//! after what is open and put it on the timeline (`crate::a2o::add_adjacent`),
//! with a menu between them for how much audio to fetch.
//!
//! Shown only while the open file, or every file on the open timeline, is a
//! clip of one A2O recording.

use crate::a2o;
use crate::state::AppState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;

#[component]
pub fn A2oAddButtons() -> impl IntoView {
    let state = expect_context::<AppState>();
    // True while a clip is being fetched: both buttons wait for it.
    let busy = RwSignal::new(false);
    // How much audio one press fetches, in seconds. Kept between sessions.
    let add_secs = RwSignal::new(a2o::add_secs());
    let add = move |before: bool| {
        move |_: web_sys::MouseEvent| {
            if busy.get_untracked() {
                return;
            }
            busy.set(true);
            let secs = add_secs.get_untracked();
            spawn_local(async move {
                if let Err(e) = a2o::add_adjacent(state, before, secs).await {
                    state.show_error_toast(e);
                }
                busy.try_set(false);
            });
        }
    };
    let on_length = move |ev: web_sys::Event| {
        let Some(select) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok())
        else {
            return;
        };
        if let Ok(secs) = select.value().parse::<f64>() {
            add_secs.set(secs);
            a2o::set_add_secs(secs);
        }
    };
    move || {
        let stretch = a2o::open_stretch(&state)?;
        // At the start or the end of the recording there is nothing more to
        // add on that side, so that button is left out.
        let button = |before: bool| {
            stretch.can_add(before).then(|| {
                let (label, side) = if before {
                    ("Add before", "before")
                } else {
                    ("Add after", "after")
                };
                view! {
                    <button class="a2o-add-btn"
                        title=move || format!(
                            "Get the {} {side} this audio from A2O and add it to the timeline",
                            a2o::length_text(add_secs.get())
                        )
                        disabled=move || busy.get()
                        on:click=add(before)
                    >{label}</button>
                }
            })
        };
        // The length sits between the two buttons it applies to.
        let lengths: Vec<_> = a2o::ADD_CHOICES_SECS
            .iter()
            .map(|&secs| {
                view! {
                    <option value=secs.to_string() selected=move || add_secs.get() == secs>
                        {a2o::length_text(secs)}
                    </option>
                }
            })
            .collect();
        Some(view! {
            <div class="a2o-add" class:busy=move || busy.get()>
                {button(true)}
                <select class="a2o-add-length" on:change=on_length
                    aria-label="How much audio to add"
                    title="How much audio Add before and Add after get"
                >{lengths}</select>
                {button(false)}
            </div>
        })
    }
}

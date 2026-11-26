// The card that opens above the Play button to suggest a playback setting:
// the "Playing 1:1" HFR card (hfr_tour.rs) and the quiet-playback Gain card
// (gain_nudge.rs). Both look the same and open in the same place; the
// .play-card classes in style.css give the look.
//
// `PlayCard` is the panel above Play. `PlayCardBody` is what is inside it
// (close button, text, a row of buttons), so a card can also be shown in
// another panel (the HFR card moves under the HFR switch).

use crate::components::popup::{Align, PopupPanel, Side};
use leptos::prelude::*;

const CLOSE: &str = "Close";

/// A card above the Play button. Mount it in the bottom toolbar, where its
/// anchor (.play-card-anchor) lines up with Play. `class` is the panel's
/// whole class list: "layer-panel play-card" and the card's own name.
#[component]
pub fn PlayCard(
    is_open: Signal<bool>,
    class: &'static str,
    children: ChildrenFn,
) -> impl IntoView {
    let anchor = NodeRef::<leptos::html::Div>::new();
    view! {
        <div class="play-card-anchor" node_ref=anchor></div>
        <PopupPanel
            is_open=is_open
            anchor=anchor
            preferred_side=Side::Above
            preferred_align=Align::Start
            class=class
        >
            {children()}
        </PopupPanel>
    }
}

/// The inside of a card: a close button, `text`, and `children` (the
/// buttons, .play-card-btn and .play-card-link) in a row under it.
#[component]
pub fn PlayCardBody(
    #[prop(into)] text: Signal<String>,
    on_close: Callback<()>,
    /// The body element, for callers that check where keyboard focus is.
    #[prop(optional)]
    body_ref: NodeRef<leptos::html::Div>,
    /// True while a mouse pointer is over the card. Mouse only: a tap sends
    /// pointerenter too, and on touch screens the matching pointerleave is
    /// not reliable.
    #[prop(optional)]
    hover: Option<RwSignal<bool>>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="play-card-body" role="status" node_ref=body_ref
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:pointerenter=move |ev: web_sys::PointerEvent| {
                if ev.pointer_type() == "mouse" {
                    if let Some(h) = hover {
                        h.set(true);
                    }
                }
            }
            on:pointerleave=move |_| {
                if let Some(h) = hover {
                    h.set(false);
                }
            }
        >
            <button class="play-card-close" title=CLOSE aria-label=CLOSE
                on:click=move |_| on_close.run(())
            >"\u{00D7}"</button>
            <p class="play-card-text">{move || text.get()}</p>
            <div class="play-card-actions">{children()}</div>
        </div>
    }
}

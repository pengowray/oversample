// Phone "Hear" sheet: the playback modes as a bottom sheet.
//
// On narrow viewports the Hearing Bar's mode radio group (1:1 HET TE PS ZC)
// has no room to say what each abbreviation means. On mobile it is replaced
// by one chip showing the current playback mode; tapping it opens this
// sheet, which lists every mode with its full name and a one-line
// consequence. The bar's other controls (HFR, Range, Output range, PASS,
// Gain, NR, Notch) stay in the bar beside the chip.
//
// The sheet is always mounted (hidden by CSS when closed) because
// `ModeRadioGroup` owns effects that keep filter settings in sync. They
// must run even while the sheet is closed.
//
// Open state is its own store field (`panels.hear_sheet_open`) rather than a
// `LayerPanel` variant: a mode's settings open as a layer panel, and sharing
// the single `layer_panel_open` slot would close the sheet the moment one
// of them opened.

use crate::state::store_fields::*;
use leptos::portal::Portal;
use leptos::prelude::*;

use crate::components::mode_button::{ModeBucket, ModeRadioGroup};
use crate::state::AppState;

fn close(state: &AppState) {
    state.panels.hear_sheet_open().set(false);
    state.panels.layer_panel_open().set(None);
}

/// The chip that stands in for the Hearing Bar on phones. Shows the mode
/// currently in effect; tapping toggles the sheet.
#[component]
pub fn HearChip() -> impl IntoView {
    let state = expect_context::<AppState>();
    let bucket = Signal::derive(move || {
        let on = state.viewmode.hfr_enabled().get();
        if on {
            ModeBucket::from_mode(state.playback.mode().get())
        } else {
            ModeBucket::Normal
        }
    });
    let open = move || state.panels.hear_sheet_open().get();
    view! {
        <button
            class=move || if open() { "layer-btn hear-chip open" } else { "layer-btn hear-chip" }
            title="Playback mode"
            on:click=move |ev: web_sys::MouseEvent| {
                ev.stop_propagation();
                state.panels.hear_sheet_open().update(|o| *o = !*o);
            }
        >
            <span class="layer-btn-category">"Hear"</span>
            <span class="layer-btn-value">
                {move || bucket.get().name()}
                <span class="hear-chip-abbr">{move || bucket.get().label()}</span>
            </span>
        </button>
    }
}

#[component]
pub fn HearSheet() -> impl IntoView {
    let state = expect_context::<AppState>();
    let open = move || state.panels.hear_sheet_open().get();
    view! {
        <Portal>
            <div
                class=move || if open() { "hear-sheet-backdrop open" } else { "hear-sheet-backdrop" }
                on:click=move |_| close(&state)
            ></div>
            <div
                class=move || if open() { "hear-sheet open" } else { "hear-sheet" }
                on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                on:touchstart=|ev: web_sys::TouchEvent| ev.stop_propagation()
            >
                <div class="hear-sheet-handle" on:click=move |_| close(&state)></div>
                <div class="hear-sheet-title">"Playback mode"</div>
                <ModeRadioGroup list_layout=true/>
            </div>
        </Portal>
    }
}

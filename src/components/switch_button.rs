// Switch button: the bar control for something that is on or off and has
// settings (XForm, PASS, Gain, Noise).
//
//   ┌───────────────┐
//   │ Gain      ◖●  │  top row: the name and a switch. Pressing it only
//   ├───────────────┤  turns the control on or off.
//   │ Peak +12 dB ▾ │  bottom row: the setting in use. Pressing it only
//   └───────────────┘  opens the settings menu.
//
// It replaces the split ComboButton for these controls: there, one half
// toggled and the other half showed "OFF" and opened the menu, and nothing
// showed which half did what. ComboButton stays for Play, Record and Listen,
// whose left half is an action, not an on/off state.
//
// The control has a fixed width, and the bottom row keeps showing the setting
// (dimmed) while the control is off, so turning it on or off moves nothing.

use crate::state::store_fields::*;
use leptos::prelude::*;

use crate::components::popup::{Align, PopupPanel, Side};
use crate::state::AppState;

#[component]
pub fn SwitchButton(
    /// Name on the top row (e.g. "Gain").
    name: &'static str,
    /// Whether the control is on.
    #[prop(into)]
    on: Signal<bool>,
    /// Called when the top row is pressed.
    toggle: Callback<()>,
    /// The setting in use, shown on the bottom row (e.g. "Peak +12 dB").
    #[prop(into)]
    value: Signal<String>,
    /// Whether the settings menu is open.
    #[prop(into)]
    is_open: Signal<bool>,
    /// Called when the bottom row is pressed.
    toggle_menu: Callback<()>,
    /// Greys out the whole control and ignores presses (no file loaded).
    #[prop(into)]
    disabled: Signal<bool>,
    /// The switch alone cannot be changed (live gain is always applied).
    /// The bottom row still opens the menu.
    #[prop(optional, into)]
    switch_locked: Option<Signal<bool>>,
    /// CSS width of the control. Fixed so that a changing value moves nothing.
    width: &'static str,
    /// Tooltip for the top row.
    switch_title: &'static str,
    /// Tooltip for the bottom row.
    menu_title: &'static str,
    /// "left" or "right": the edge of the control the menu lines up with.
    #[prop(default = "left")]
    panel_align: &'static str,
    /// Extra inline style for the menu (e.g. "min-width: 210px;").
    #[prop(default = "")]
    panel_style: &'static str,
    /// Menu content. `ChildrenFn` because the menu is unmounted when closed.
    children: ChildrenFn,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let locked = move || switch_locked.is_some_and(|s| s.get());
    let row_ref = NodeRef::<leptos::html::Div>::new();
    let preferred_align = if panel_align == "right" {
        Align::End
    } else {
        Align::Start
    };

    view! {
        <div
            node_ref=row_ref
            class="switch-btn"
            class:on=move || on.get()
            class:open=move || is_open.get()
            class:disabled=move || disabled.get()
            style=move || format!(
                "width: {}; pointer-events: {};",
                width,
                if state.interaction.mouse_in_label_area().get() { "none" } else { "auto" }
            )
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:touchstart=|ev: web_sys::TouchEvent| ev.stop_propagation()
        >
            <button
                class="switch-btn-top"
                class:locked=locked
                role="switch"
                aria-checked=move || on.get().to_string()
                title=switch_title
                on:click=move |_: web_sys::MouseEvent| {
                    if !disabled.get_untracked() && !locked() {
                        toggle.run(());
                    }
                }
            >
                <span class="switch-btn-name">{name}</span>
                <span class="switch-mark"></span>
            </button>
            <button
                class="switch-btn-value"
                aria-expanded=move || is_open.get().to_string()
                title=menu_title
                on:click=move |_: web_sys::MouseEvent| {
                    if !disabled.get_untracked() {
                        toggle_menu.run(());
                    }
                }
            >
                <span class="switch-btn-value-text fit-text" data-fit-max="11" data-fit-min="8">
                    {move || value.get()}
                </span>
                <span class="switch-btn-caret">{"\u{25BE}"}</span>
            </button>

            // Portalled under <body> and placed against the viewport, so a
            // scrolling bar cannot clip the menu (see popup.rs).
            <PopupPanel
                is_open=is_open
                anchor=row_ref
                preferred_side=Side::Below
                preferred_align=preferred_align
                extra_style=panel_style
            >
                {children()}
            </PopupPanel>
        </div>
    }
}

/// A heading row inside a settings menu with the same switch as the bar
/// control: the section's name on the left, its on/off switch on the right.
#[component]
pub fn SwitchRow(
    name: &'static str,
    #[prop(into)] on: Signal<bool>,
    toggle: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            class="switch-row"
            class:on=move || on.get()
            role="switch"
            aria-checked=move || on.get().to_string()
            on:click=move |_: web_sys::MouseEvent| toggle.run(())
        >
            <span class="switch-row-name">{name}</span>
            <span class="switch-mark"></span>
        </button>
    }
}

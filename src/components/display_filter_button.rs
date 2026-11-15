use leptos::prelude::*;

use crate::state::DisplayFilterMode;

/// One row of the XForm menu's grid: the control's name, then a cell for
/// each choice (Off, Auto, As heard, Custom) under the header's columns.
///
/// A choice the row does not have leaves its cell empty. The "As heard" cell
/// always shows what the Hear bar is set to for this control ("on", "HET"),
/// because that is what the choice gives. The other cells show their name
/// only while selected.
#[component]
pub fn DspFilterRow<S>(
    /// The control's name, as the Hear bar and its menus give it.
    label: &'static str,
    /// The mode signal — generic so it accepts both `RwSignal<DisplayFilterMode>`
    /// and a `Store<DisplayState>` subfield (distinct types).
    signal: S,
    /// What the Hear bar is set to for this control, shown in the "As heard"
    /// cell: "on", "off", "HET", "48 kHz".
    #[prop(into)]
    heard: Signal<String>,
    /// Whether the row has a Custom choice.
    custom_available: bool,
    /// Whether the row has an Auto choice.
    #[prop(default = true)]
    auto_available: bool,
    /// Tooltip for the "As heard" cell, in place of the choice's name.
    #[prop(optional, into)]
    heard_tooltip: Option<Signal<String>>,
) -> impl IntoView
where
    S: leptos::prelude::Get<Value = DisplayFilterMode>
        + leptos::prelude::Set<Value = DisplayFilterMode>
        + Copy
        + Send
        + Sync
        + 'static,
{
    view! {
        <div class="dsp-filter-row">
            <span class="dsp-filter-label">{label}</span>
            {DisplayFilterMode::ALL.iter().copied().map(|mode| {
                let is_same = mode == DisplayFilterMode::Same;
                let available = match mode {
                    DisplayFilterMode::Custom => custom_available,
                    DisplayFilterMode::Auto => auto_available,
                    DisplayFilterMode::Off | DisplayFilterMode::Same => true,
                };
                if !available {
                    return view! { <span class="dsp-cell-none"></span> }.into_any();
                }
                let selected = move || signal.get() == mode;
                view! {
                    <button
                        class="dsp-cell"
                        class:sel=selected
                        aria-label=mode.label()
                        aria-pressed=move || selected().to_string()
                        title=move || match (is_same, heard_tooltip) {
                            (true, Some(tip)) if !tip.get().is_empty() => tip.get(),
                            _ => mode.label().to_string(),
                        }
                        on:click=move |_| signal.set(mode)
                    >
                        {move || {
                            if is_same {
                                heard.get()
                            } else if selected() {
                                mode.label().to_lowercase()
                            } else {
                                String::new()
                            }
                        }}
                    </button>
                }.into_any()
            }).collect_view()}
        </div>
    }
}

/// The grid's header: the names of the four choices over their columns.
#[component]
pub fn DspFilterHeader() -> impl IntoView {
    view! {
        <div class="dsp-filter-row dsp-filter-header">
            <span class="dsp-filter-label"></span>
            {DisplayFilterMode::ALL.iter().map(|mode| view! {
                <span class="dsp-filter-col">{mode.label()}</span>
            }).collect_view()}
        </div>
    }
}

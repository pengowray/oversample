use crate::components::layer_panel::{layer_opt_class, toggle_panel};
use crate::components::popup::{Align, PopupPanel, Side};
use crate::state::store_fields::*;
use crate::state::{AppState, CanvasTool, LayerPanel};
use leptos::prelude::*;

#[component]
pub fn ToolButton() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open: Signal<bool> =
        Signal::derive(move || state.panels.layer_panel_open().get() == Some(LayerPanel::Tool));
    let anchor = NodeRef::<leptos::html::Div>::new();

    view! {
        // Anchored bottom-right of main-overlays
        <div
            style="position: absolute; bottom: 50px; right: 12px; pointer-events: none;"
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:touchstart=|ev: web_sys::TouchEvent| ev.stop_propagation()
        >
            <div node_ref=anchor style="position: relative; pointer-events: auto;">
                <button
                    class=move || if is_open.get() { "layer-btn open" } else { "layer-btn" }
                    on:click=move |_| toggle_panel(&state, LayerPanel::Tool)
                    title="Tool"
                >
                    <span class="layer-btn-category">"Tool"</span>
                    <span class="layer-btn-value">{move || match state.interaction.canvas_tool().get() {
                        CanvasTool::Hand => "Hand",
                        CanvasTool::Selection => "Select",
                    }}</span>
                </button>
                <PopupPanel
                    is_open=is_open
                    anchor=anchor
                    preferred_side=Side::Above
                    preferred_align=Align::End
                >
                    <div class="layer-panel-title">"Tool"</div>
                    <button
                        class=move || layer_opt_class(state.interaction.canvas_tool().get() == CanvasTool::Hand)
                        on:click=move |_| {
                            state.interaction.canvas_tool().set(CanvasTool::Hand);
                            state.panels.layer_panel_open().set(None);
                        }
                    >"Hand (pan)"</button>
                    <button
                        class=move || layer_opt_class(state.interaction.canvas_tool().get() == CanvasTool::Selection)
                        on:click=move |_| {
                            state.interaction.canvas_tool().set(CanvasTool::Selection);
                            state.panels.layer_panel_open().set(None);
                        }
                    >"Selection"</button>
                </PopupPanel>
            </div>
        </div>
    }
}

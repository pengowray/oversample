// View Bar — sits below the Hearing Bar (above the main canvas) and owns
// the visualization-layer controls: which view fills the canvas, which
// overlays (annotations, bat book) are visible, and which canvas tool is
// active for click/drag interactions.
//
// Kept deliberately flexible — flex-wrap + a `.bar-spacer` separator lets
// new toggles be added on either end without breaking narrow layouts.

use leptos::prelude::*;

use crate::components::app::MainViewButton;
use crate::state::{ActiveFocus, AppState, Bar, CanvasTool, LayerPanel};

fn layer_opt_class(active: bool) -> &'static str {
    if active { "layer-panel-opt sel" } else { "layer-panel-opt" }
}

fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.layer_panel_open.update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

/// Annotations visibility toggle.
#[component]
fn AnnoToggle() -> impl IntoView {
    let state = expect_context::<AppState>();
    view! {
        <button
            class=move || if state.annotations_visible.get() { "layer-btn active" } else { "layer-btn" }
            on:click=move |_| {
                let new_visible = !state.annotations_visible.get_untracked();
                state.annotations_visible.set(new_visible);
                if !new_visible {
                    // Drop annotation focus/selection and clear interaction state.
                    if state.active_focus.get_untracked() == Some(ActiveFocus::Annotations) {
                        state.active_focus.set(None);
                    }
                    if !state.selected_annotation_ids.get_untracked().is_empty() {
                        state.selected_annotation_ids.set(Vec::new());
                    }
                    state.annotation_hover_handle.set(None);
                    state.annotation_drag_handle.set(None);
                    state.annotation_editing.set(false);
                    state.annotation_is_new_edit.set(false);
                }
            }
            title=move || if state.annotations_visible.get() { "Hide annotations" } else { "Show annotations" }
        >
            <span class="layer-btn-category">"\u{00A0}"</span>
            <span class="layer-btn-value">"Anno"</span>
        </button>
    }
}

/// Bat book toggle — shows/hides both the strip below the main view
/// and the floating reference panel together. "On" means either is open;
/// clicking when on closes both, clicking when off opens both.
#[component]
fn BookToggle() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_on = move || state.bat_book_open.get() || state.bat_book_ref_open.get();
    view! {
        <button
            class=move || if is_on() { "layer-btn active" } else { "layer-btn" }
            on:click=move |_| {
                let on = is_on();
                state.bat_book_open.set(!on);
                state.bat_book_ref_open.set(!on);
            }
            title=move || if is_on() { "Hide bat book" } else { "Show bat book" }
        >
            <span class="layer-btn-category">"Bat"</span>
            <span class="layer-btn-value">"Book"</span>
        </button>
    }
}

/// Canvas tool selector (Hand / Selection). Dropdown opens downward since
/// this lives in the top-positioned View Bar (not the bottom toolbar).
#[component]
fn ToolCombo() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = move || state.layer_panel_open.get() == Some(LayerPanel::Tool);
    let no_file = move || state.current_file_index.get().is_none() && state.active_timeline.get().is_none();

    view! {
        <div style="position: relative;">
            <button
                class=move || {
                    if no_file() { "layer-btn disabled" }
                    else if is_open() { "layer-btn open" }
                    else { "layer-btn" }
                }
                on:click=move |_| { if !no_file() { toggle_panel(&state, LayerPanel::Tool); } }
                title="Tool"
            >
                <span class="layer-btn-category">"Tool"</span>
                <span class="layer-btn-value">{move || match state.canvas_tool.get() {
                    CanvasTool::Hand => "Hand",
                    CanvasTool::Selection => "Select",
                }}</span>
            </button>
            <Show when=move || is_open()>
                <div class="layer-panel" style="top: calc(100% + 4px); right: 0;">
                    <div class="layer-panel-title">"Tool"</div>
                    <button
                        class=move || layer_opt_class(state.canvas_tool.get() == CanvasTool::Hand)
                        on:click=move |_| {
                            state.canvas_tool.set(CanvasTool::Hand);
                            state.layer_panel_open.set(None);
                        }
                    >"Hand (pan)"</button>
                    <button
                        class=move || layer_opt_class(state.canvas_tool.get() == CanvasTool::Selection)
                        on:click=move |_| {
                            state.canvas_tool.set(CanvasTool::Selection);
                            state.layer_panel_open.set(None);
                        }
                    >"Selection"</button>
                </div>
            </Show>
        </div>
    }
}

#[component]
pub fn ViewBar() -> impl IntoView {
    let state = expect_context::<AppState>();
    let has_file = move || state.current_file_index.get().is_some() || state.active_timeline.get().is_some();

    view! {
        // Stop clicks/taps inside the bar from bubbling to .main's
        // "close all panels" handler — without this, opening a custom
        // button's panel (Tool, etc.) immediately re-closes it.
        // ComboButton has its own stop_propagation, so the combos in
        // here used to be fine; the Tool button is a plain <button>
        // that doesn't, hence the menu was getting eaten.
        <div class="view-bar"
            class:panel-open=move || matches!(state.layer_panel_open.get().map(LayerPanel::bar), Some(Bar::View))
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:touchstart=|ev: web_sys::TouchEvent| ev.stop_propagation()
        >
            <span class="bar-label">"VIEW"</span>
            <div class="bar-controls">
                <MainViewButton />
                <div class="bar-sep"></div>
                {move || has_file().then(|| view! { <AnnoToggle /> })}
                <BookToggle />
                <div class="bar-spacer"></div>
                {move || (!state.is_mobile.get() && has_file()).then(|| view! { <ToolCombo /> })}
            </div>
        </div>
    }
}

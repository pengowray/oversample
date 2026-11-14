// Helpers shared by the bar buttons that open a menu (a "layer panel").

use crate::state::store_fields::*;
use leptos::prelude::*;

use crate::state::{AppState, LayerPanel};

/// Open `panel`, or close it if it is the one open. Only one layer panel is
/// open at a time.
pub fn toggle_panel(state: &AppState, panel: LayerPanel) {
    state.panels.layer_panel_open().update(|p| {
        *p = if *p == Some(panel) { None } else { Some(panel) };
    });
}

/// Class for a menu option, marked when it is the one selected.
pub fn layer_opt_class(active: bool) -> &'static str {
    if active {
        "layer-panel-opt sel"
    } else {
        "layer-panel-opt"
    }
}

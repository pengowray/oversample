//! Pieces the result lists of the classifier section share: a class's
//! swatch and name, the pin that gives a species or component a row above
//! the spectrogram, the flag column for birds unlikely here, and showing a
//! window on the spectrogram.

use super::parts::pin_button;
use crate::classifier::{self, location, Method};
use crate::components::icons::Icon;
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;

/// A small square in the colour the spectrogram's label strip gives this
/// class, so a row in the lists can be matched to its label and call boxes.
/// `slot` is the species' place in `summary::colour_order`; None for classes
/// that are not a species ("Noise"), or that no window has as its top class.
/// Inline style: the colour comes from overlays.rs, the shape from the
/// species map legend's `.map-key`.
pub(super) fn species_swatch(slot: Option<usize>) -> impl IntoView {
    use crate::canvas::spectrogram_renderer::{species_colour, NOT_SPECIES_COLOUR};
    let colour = slot.map_or(NOT_SPECIES_COLOUR, |s| species_colour(s).0);
    view! {
        <span
            class="map-key cd-swatch"
            style=format!("background:{colour}")
        ></span>
    }
}

/// A class name as shown: BattyBirdNET's lower-case classes get a capital.
pub(super) fn class_name(label: Option<&classifier::SpeciesLabel>) -> String {
    label.map_or_else(|| "Unknown".to_string(), classifier::strips::display_name)
}

/// True when this species has a row above the spectrogram. Reactive.
pub(super) fn strip_picked(state: AppState, method: Method, scientific: &str) -> bool {
    state.call_detection.strip_species().with(|v| {
        v.iter()
            .any(|s| s.method == method && s.scientific == scientific)
    })
}

/// Add or remove this species' row above the spectrogram.
pub(super) fn toggle_strip(state: AppState, method: Method, scientific: String) {
    state.call_detection.strip_species().update(|v| {
        classifier::strips::toggle(v, classifier::strips::StripSpecies { method, scientific })
    });
    save_strips(state);
}

/// True when component `k` (0 for PC1) has a row above the spectrogram.
/// Reactive.
pub(super) fn component_picked(state: AppState, k: usize) -> bool {
    state
        .call_detection
        .strip_components()
        .with(|v| v.contains(&k))
}

/// Add or remove component `k`'s row above the spectrogram.
pub(super) fn toggle_component(state: AppState, k: usize) {
    state
        .call_detection
        .strip_components()
        .update(|v| classifier::strips::toggle(v, k));
    save_strips(state);
}

/// Save the picked rows (species, components and sound clusters) for the
/// next session.
pub(super) fn save_strips(state: AppState) {
    let cd = state.call_detection;
    crate::settings::set_score_strips(&classifier::strips::StripPicks {
        species: cd.strip_species().get_untracked(),
        components: cd.strip_components().get_untracked(),
        sound_groups: cd.sound_groups().get_untracked(),
        groups: cd.sound_group_count().get_untracked(),
    });
}

/// The pin cell of a species row.
pub(super) fn species_pin(
    state: AppState,
    method: Method,
    scientific: String,
    name: String,
) -> impl IntoView {
    let sci = scientific.clone();
    let picked = move || strip_picked(state, method, &sci);
    view! {
        <td class="cd-col-pin">
            {pin_button(name, picked, move || toggle_strip(state, method, scientific.clone()))}
        </td>
    }
}

/// Scroll the spectrogram so the window from `start_s` to `end_s` has its
/// middle in the centre, and pick it.
pub(super) fn show_window(state: AppState, start_s: f64, end_s: f64) {
    state.call_detection.selected_window().set(Some(start_s));
    let mid = 0.5 * (start_s + end_s);
    let files = state.library.files().get_untracked();
    let idx = state.library.current_index().get_untracked();
    if let Some(file) = idx.and_then(|i| files.get(i)) {
        let canvas_w = state.viewmode.spectrogram_canvas_width().get_untracked();
        let zoom = state.view.zoom_level().get_untracked();
        let visible_time = (canvas_w / zoom) * file.spectrogram.time_resolution;
        state
            .view
            .scroll_offset()
            .set((mid - visible_time / 2.0).max(0.0));
    }
}

/// The flag beside a bird the location list says is unlikely at the
/// recording's place and month. One mark for every place a result is named
/// (the lists, the top 3, the spectrogram's label strip), so it costs one
/// character of width and the name stays on one line.
pub(super) fn unlikely_flag() -> impl IntoView {
    view! {
        <span
            class="unlikely-flag"
            role="img"
            aria-label="Unlikely at this place and month"
            title="Unlikely at this place and month"
        >
            <Icon kind=Icon::Flag />
        </span>
    }
}

/// The flag column of a results table: a narrow cell after the name, present
/// for the methods the location list applies to. A column of its own, so
/// turning "Mark birds unlikely here" on or off never re-wraps a name or
/// changes a row's height. Only the cell's content follows the setting; the
/// tables are not rebuilt.
pub(super) fn flag_cell(
    state: AppState,
    method: Method,
    file: Option<u64>,
    label: Option<&classifier::SpeciesLabel>,
) -> Option<impl IntoView> {
    if !location::applies_to(method) {
        return None;
    }
    let label = label.cloned();
    Some(view! {
        <td class="detection-col-flag">
            {move || {
                file.zip(label.as_ref())
                    .is_some_and(|(f, l)| location::unlikely_here(state, f, method, l))
                    .then(unlikely_flag)
            }}
        </td>
    })
}

/// The header cell above [`flag_cell`].
pub(super) fn flag_header(method: Method) -> Option<impl IntoView> {
    location::applies_to(method).then(|| view! { <th class="detection-col-flag"></th> })
}

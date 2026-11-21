use crate::canvas::spectrogram_renderer;
use crate::state::store_fields::*;
use crate::state::AppState;
use crate::viewport;
use leptos::prelude::*;
use web_sys::HtmlCanvasElement;

thread_local! {
    /// The timeline row the pointer was last over while nothing was being
    /// dragged. A drag stays in this row.
    static POINTER_ROW: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The height in px of one row of the spectrogram: `canvas_height` unless a
/// timeline with clips on several rows is open. Pointer handlers and hit
/// tests work in a row's own coordinates, with this as the height.
pub fn row_height(state: &AppState, canvas_height: f64) -> f64 {
    let rows = state
        .timeline
        .active()
        .with_untracked(|t| t.as_ref().map_or(1, |tl| tl.row_count));
    crate::timeline::row_rect(0, rows, canvas_height).1
}

/// A pointer's y on the canvas as `(y inside its row, the row's height)`.
/// With one row that is `(px_y, canvas_height)`. During a drag the row is
/// the one the drag started in, and y is kept inside it, so a drag that
/// leaves the row stops at the row's top or bottom frequency.
fn y_in_row(state: &AppState, px_y: f64, canvas_height: f64) -> (f64, f64) {
    let rows = state
        .timeline
        .active()
        .with_untracked(|t| t.as_ref().map_or(1, |tl| tl.row_count));
    if rows <= 1 {
        return (px_y, canvas_height);
    }
    let (here, _, height) = crate::timeline::row_at(px_y, rows, canvas_height);
    let row = if state.interaction.is_dragging().get_untracked() {
        POINTER_ROW.with(|r| r.get()).min(rows - 1)
    } else {
        POINTER_ROW.with(|r| r.set(here));
        here
    };
    let (top, _) = crate::timeline::row_rect(row, rows, canvas_height);
    ((px_y - top).clamp(0.0, height), height)
}

/// Convert a pointer position (client_x, client_y) relative to the canvas
/// into (px_x, px_y, time, freq). On a timeline with several rows, `px_y`
/// is measured from the top of the pointer's row: see [`row_height`].
///
/// Works for both mouse and touch events — the caller extracts client_x/client_y
/// from whichever event type they have.
pub fn pointer_to_xtf(
    client_x: f64,
    client_y: f64,
    canvas_ref: &NodeRef<leptos::html::Canvas>,
    state: &AppState,
) -> Option<(f64, f64, f64, f64)> {
    let canvas_el = canvas_ref.get()?;
    let canvas: &HtmlCanvasElement = canvas_el.as_ref();
    let rect = canvas.get_bounding_client_rect();
    let px_x = client_x - rect.left();
    let cw = canvas.width() as f64;
    let (px_y, ch) = y_in_row(state, client_y - rect.top(), canvas.height() as f64);

    let files = state.library.files().get_untracked();
    let timeline = state.timeline.active().get_untracked();

    // When the waterfall is active, use its parameters so interactions work
    // even without a file loaded (listening/recording mode).
    let is_mic_active =
        state.mic.recording().get_untracked() || state.mic.listening().get_untracked();
    let waterfall_active = is_mic_active && crate::canvas::live_waterfall::is_active();

    let (time_res, file_max_freq) = if waterfall_active {
        (
            crate::canvas::live_waterfall::time_resolution(),
            crate::canvas::live_waterfall::max_freq(),
        )
    } else if let Some(ref tl) = timeline {
        let primary_file = tl.primary_segment().and_then(|s| files.get(s.file_index))?;
        (
            primary_file.spectrogram.time_resolution,
            primary_file.spectrogram.max_freq,
        )
    } else {
        let idx = state.library.current_index().get_untracked()?;
        let file = files.get(idx)?;
        (file.spectrogram.time_resolution, file.spectrogram.max_freq)
    };
    let max_freq = state
        .view
        .max_display_freq()
        .get_untracked()
        .unwrap_or(file_max_freq);
    let min_freq = state.view.min_display_freq().get_untracked().unwrap_or(0.0);
    let scroll = state.view.scroll_offset().get_untracked();
    let zoom = state.view.zoom_level().get_untracked();
    let visible_time = viewport::visible_time(cw, zoom, time_res);
    if visible_time <= 0.0 {
        return None;
    }

    let (t, f) = spectrogram_renderer::pixel_to_time_freq(
        px_x, px_y, min_freq, max_freq, scroll, time_res, zoom, cw, ch,
    );
    Some((px_x, px_y, t, f))
}

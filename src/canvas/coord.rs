use leptos::prelude::*;
use web_sys::HtmlCanvasElement;
use crate::canvas::spectrogram_renderer;
use crate::state::AppState;
use crate::viewport;

/// Convert a pointer position (client_x, client_y) relative to the canvas
/// into (px_x, px_y, time, freq).
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
    let px_y = client_y - rect.top();
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;

    let files = state.files.get_untracked();
    let timeline = state.active_timeline.get_untracked();
    let (time_res, file_max_freq) = if let Some(ref tl) = timeline {
        let primary_file = tl.segments.first().and_then(|s| files.get(s.file_index))?;
        (primary_file.spectrogram.time_resolution, primary_file.spectrogram.max_freq)
    } else {
        let idx = state.current_file_index.get_untracked()?;
        let file = files.get(idx)?;
        (file.spectrogram.time_resolution, file.spectrogram.max_freq)
    };
    let max_freq = state.max_display_freq.get_untracked()
        .unwrap_or(file_max_freq);
    let min_freq = state.min_display_freq.get_untracked()
        .unwrap_or(0.0);
    let scroll = state.scroll_offset.get_untracked();
    let zoom = state.zoom_level.get_untracked();
    let visible_time = viewport::visible_time(cw, zoom, time_res);
    if visible_time <= 0.0 {
        return None;
    }

    let (t, f) = spectrogram_renderer::pixel_to_time_freq(
        px_x, px_y, min_freq, max_freq, scroll, time_res, zoom, cw, ch,
    );
    Some((px_x, px_y, t, f))
}

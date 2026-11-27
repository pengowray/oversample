//! Page zoom for the desktop app: Ctrl (Cmd on a Mac) with `=` or `+` to zoom
//! in, `-` to zoom out and `0` for 100%, as in a web browser. A browser
//! already does this for the web build, and Android has pinch zoom, so this
//! only acts in the desktop Tauri app.
//!
//! The zoom is kept in localStorage and applied again at start-up.
//!
//! Tauri's own `zoomHotkeysEnabled` is not used: it also takes Ctrl+wheel,
//! which the spectrogram uses to zoom the time axis.

use std::cell::Cell;
use wasm_bindgen::JsValue;

const KEY: &str = "oversample_page_zoom";

/// The zoom levels of Chrome and Firefox.
const STEPS: [f64; 15] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

thread_local! {
    static ZOOM: Cell<f64> = const { Cell::new(1.0) };
}

/// The next step above `zoom`, or the highest step.
fn step_up(zoom: f64) -> f64 {
    STEPS
        .iter()
        .copied()
        .find(|s| *s > zoom + 1e-6)
        .unwrap_or(STEPS[STEPS.len() - 1])
}

/// The next step below `zoom`, or the lowest step.
fn step_down(zoom: f64) -> f64 {
    STEPS
        .iter()
        .rev()
        .copied()
        .find(|s| *s < zoom - 1e-6)
        .unwrap_or(STEPS[0])
}

/// The zoom after a key press, or `None` when the key is not a zoom key.
fn zoom_for_key(key: &str, zoom: f64) -> Option<f64> {
    match key {
        "=" | "+" => Some(step_up(zoom)),
        "-" | "_" => Some(step_down(zoom)),
        "0" => Some(1.0),
        _ => None,
    }
}

fn is_desktop_app() -> bool {
    crate::tauri_bridge::get_tauri_internals().is_some() && !crate::tauri_bridge::is_android()
}

fn apply(zoom: f64) {
    ZOOM.with(|z| z.set(zoom));
    wasm_bindgen_futures::spawn_local(async move {
        let args = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&args, &"value".into(), &JsValue::from_f64(zoom));
        if let Err(e) =
            crate::tauri_bridge::tauri_invoke("plugin:webview|set_webview_zoom", &args.into()).await
        {
            log::warn!("page zoom: {e}");
        }
    });
}

/// Apply the zoom saved in an earlier session. Call once at start-up.
pub fn restore() {
    if !is_desktop_app() {
        return;
    }
    let saved = crate::settings::get_raw(KEY).and_then(|s| s.parse::<f64>().ok());
    if let Some(zoom) = saved.filter(|z| (STEPS[0]..=STEPS[STEPS.len() - 1]).contains(z)) {
        if (zoom - 1.0).abs() > 1e-6 {
            apply(zoom);
        }
    }
}

/// Handle a key press. Returns true when it was a zoom shortcut, which is
/// then done and must not be handled further.
pub fn handle_key(ev: &web_sys::KeyboardEvent) -> bool {
    if !(ev.ctrl_key() || ev.meta_key()) || ev.alt_key() || !is_desktop_app() {
        return false;
    }
    let Some(zoom) = zoom_for_key(&ev.key(), ZOOM.with(|z| z.get())) else {
        return false;
    };
    ev.prevent_default();
    apply(zoom);
    if (zoom - 1.0).abs() < 1e-6 {
        crate::settings::remove_raw(KEY);
    } else {
        crate::settings::set_raw(KEY, &zoom.to_string());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_like_a_browser() {
        assert_eq!(zoom_for_key("=", 1.0), Some(1.1));
        assert_eq!(zoom_for_key("+", 1.1), Some(1.25));
        assert_eq!(zoom_for_key("-", 1.0), Some(0.9));
        assert_eq!(zoom_for_key("0", 2.5), Some(1.0));
        assert_eq!(zoom_for_key("a", 1.0), None);
    }

    #[test]
    fn stops_at_the_ends() {
        assert_eq!(zoom_for_key("=", 3.0), Some(3.0));
        assert_eq!(zoom_for_key("-", 0.25), Some(0.25));
    }
}

//! Tells the desktop app which recordings are not saved, so it can ask
//! before the window closes (`src-tauri/src/close_warning.rs`).
//!
//! A finished recording that is not saved has `is_recording` set (it shows
//! "* File unsaved" and Save). A "To memory" recording still running is sent
//! separately, because it has to be stopped before it can be saved.
//!
//! In a browser the same check runs before the tab closes or reloads.

use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;

/// The finished recordings that are not saved, and the "To memory" recording
/// still running, if any. A running "To file" recording is left out: its
/// audio is on disk and is recovered at the next start.
fn unsaved(state: AppState) -> (Vec<String>, Option<String>) {
    let live = state.mic.live_file_idx().get();
    let to_memory =
        !state.is_tauri || state.playback.record_mode().get() == crate::state::RecordMode::ToMemory;
    state.library.files().with(|files| {
        let names = files
            .iter()
            .enumerate()
            .filter(|(i, f)| f.is_recording && !f.is_live_listen && Some(*i) != live)
            .map(|(_, f)| f.name.clone())
            .collect();
        let running = live
            .and_then(|i| files.get(i))
            .filter(|f| f.is_recording && !f.is_live_listen && to_memory)
            .map(|f| f.name.clone());
        (names, running)
    })
}

/// Warn before a recording that is not saved is lost. The desktop app is
/// sent the recordings' names whenever they change, starting with the list
/// at startup. A browser shows its own "Leave site?" box when the tab is
/// closed or reloaded. Android: nothing.
pub fn watch(state: AppState) {
    if !state.is_tauri {
        watch_browser(state);
        return;
    }
    if !crate::recording_folder::is_desktop(state) {
        return;
    }
    let unsaved = Memo::new(move |_| unsaved(state));
    // Numbered, so the native side keeps the newest list if two calls arrive
    // out of order. Starts from the clock (in thousandths of a millisecond),
    // so after a page reload the new page's lists replace the old page's.
    let mut seq: u64 = (js_sys::Date::now() as u64).saturating_mul(1000);
    Effect::new(move |_| {
        let (names, running) = unsaved.get();
        seq += 1;
        let args = Args {
            names,
            running,
            seq,
        };
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) =
                crate::tauri_bridge::tauri_invoke_args("set_unsaved_recordings", &args).await
            {
                log::warn!("set_unsaved_recordings: {e}");
            }
        });
    });
}

/// Browser: ask before the tab closes or reloads while a recording is not
/// saved. The browser writes the box's text itself.
fn watch_browser(state: AppState) {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else {
        return;
    };
    let on_unload = Closure::<dyn Fn(web_sys::BeforeUnloadEvent)>::new(
        move |ev: web_sys::BeforeUnloadEvent| {
            let any = state.library.files().try_with_untracked(|files| {
                files.iter().any(|f| f.is_recording && !f.is_live_listen)
            });
            if any == Some(true) {
                ev.prevent_default();
                ev.set_return_value("unsaved recording");
            }
        },
    );
    let _ =
        window.add_event_listener_with_callback("beforeunload", on_unload.as_ref().unchecked_ref());
    on_unload.forget();
}

#[derive(serde::Serialize)]
struct Args {
    names: Vec<String>,
    running: Option<String>,
    seq: u64,
}

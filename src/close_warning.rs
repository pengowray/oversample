//! Tells the desktop app which recordings are not saved, so it can ask
//! before the window closes (`src-tauri/src/close_warning.rs`).
//!
//! A finished recording that is not saved has `is_recording` set (it shows
//! "* File unsaved" and Save). The recording in progress is left out, as the
//! toolbar leaves it out of Save: it has no finished audio yet.

use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;

/// Desktop only: send the unsaved recordings' names whenever they change,
/// starting with the list at startup.
pub fn watch(state: AppState) {
    if !crate::recording_folder::is_desktop(state) {
        return;
    }
    let unsaved = Memo::new(move |_| {
        let live = state.mic.live_file_idx().get();
        state.library.files().with(|files| {
            files
                .iter()
                .enumerate()
                .filter(|(i, f)| f.is_recording && !f.is_live_listen && Some(*i) != live)
                .map(|(_, f)| f.name.clone())
                .collect::<Vec<String>>()
        })
    });
    // Numbered, so the native side keeps the newest list if two calls arrive
    // out of order. Starts from the clock (in thousandths of a millisecond),
    // so after a page reload the new page's lists replace the old page's.
    let mut seq: u64 = (js_sys::Date::now() as u64).saturating_mul(1000);
    Effect::new(move |_| {
        let names = unsaved.get();
        seq += 1;
        let args = Args { names, seq };
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) =
                crate::tauri_bridge::tauri_invoke_args("set_unsaved_recordings", &args).await
            {
                log::warn!("set_unsaved_recordings: {e}");
            }
        });
    });
}

#[derive(serde::Serialize)]
struct Args {
    names: Vec<String>,
    seq: u64,
}

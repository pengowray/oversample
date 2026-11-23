pub mod a2o;
pub mod annota;
pub mod annotations;
pub mod audio;
pub mod bat_book;
pub mod call_annotations;
pub mod canvas;
pub mod classifier;
pub mod cloud;
pub mod components;
pub mod denoise;
pub mod denoise_copy;
pub mod device_location;
pub mod dsp;
pub mod file_identity;
pub mod focus_stack;
pub mod format_time;
pub mod format_units;
pub mod guano_write;
pub mod heard_axis;
pub mod heterodyne;
pub mod license;
pub mod live_embedding;
pub mod live_explorer;
pub mod markdown;
pub mod native_save;
pub mod note_line;
pub mod opfs;
pub mod opfs_audio;
pub mod pca;
pub mod project;
pub mod project_store;
pub mod rate_notes;
pub mod recording_place;
pub mod scope;
pub mod search_client;
pub mod settings;
pub mod source_site;
pub mod species_check;
pub mod state;
pub mod tauri_bridge;
pub mod te_correction;
pub mod test_hook;
pub mod timeline;
pub mod types;
pub mod viewport;
pub mod web_util;
pub mod wikimedia;
pub mod xc_client;

use components::app::App;
use leptos::prelude::*;

/// Milliseconds since the page started loading.
fn page_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    // Reached once the WASM is downloaded and compiled.
    let wasm_ready = page_ms();
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Debug);

    if cfg!(debug_assertions) {
        log::warn!(
            "Oversample is running in DEBUG WASM mode. Audio rendering is much \
             slower and the app can hit spurious WASM panics that don't happen \
             in release. Run `trunk serve --release` (or `trunk build --release`)."
        );
    }

    mount_to_body(App);
    let app_built = page_ms();

    // The start-up screen from index.html, shown while the WASM loaded.
    if let Some(splash) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("boot-splash"))
    {
        splash.remove();
    }

    // One line in the console (logcat on Android) saying how long each
    // start-up stage took, for comparing builds and devices.
    wasm_bindgen_futures::spawn_local(async move {
        web_util::after_next_paint().await;
        let first_frame = page_ms();
        log::info!(
            "Start-up: {:.0} ms. Loading and compiling the app {:.0} ms, building the screen {:.0} ms, first frame {:.0} ms.",
            first_frame,
            wasm_ready,
            app_built - wasm_ready,
            first_frame - app_built,
        );
        // Read by e2e/startup.spec.ts.
        if let Some(window) = web_sys::window() {
            let times = js_sys::Array::of3(
                &wasm_ready.into(),
                &app_built.into(),
                &first_frame.into(),
            );
            let _ = js_sys::Reflect::set(&window, &"__oversampleStartup".into(), &times);
        }
    });
}

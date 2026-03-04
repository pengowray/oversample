use leptos::prelude::*;
use crate::state::AppState;
use crate::bat_book::data::get_manifest;

/// Floating reference panel on the right side of the main view.
/// Shows info about the selected bat family/families.
/// Scroll wheel navigates through entries.
#[component]
pub fn BatBookRefPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let selected_entries = Memo::new(move |_| {
        let sel_ids = state.bat_book_selected_ids.get();
        if sel_ids.is_empty() {
            return Vec::new();
        }
        let region = state.bat_book_region.get();
        let manifest = get_manifest(region);
        manifest.entries.into_iter()
            .filter(|e| sel_ids.iter().any(|id| id == e.id))
            .collect::<Vec<_>>()
    });

    let on_close = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        state.bat_book_ref_open.set(false);
    };

    // Scroll wheel: navigate to prev/next entry in the full manifest
    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        ev.stop_propagation();
        let delta = ev.delta_y();
        if delta.abs() < 1.0 { return; }

        let region = state.bat_book_region.get_untracked();
        let manifest = get_manifest(region);
        let ids = state.bat_book_selected_ids.get_untracked();
        if ids.is_empty() || manifest.entries.is_empty() { return; }

        // Find the index of the last selected entry
        let last_id = &ids[ids.len() - 1];
        let cur_idx = manifest.entries.iter().position(|e| e.id == last_id.as_str());
        let Some(cur) = cur_idx else { return };

        let next = if delta > 0.0 {
            // scroll down = next
            if cur + 1 < manifest.entries.len() { cur + 1 } else { return }
        } else {
            // scroll up = prev
            if cur > 0 { cur - 1 } else { return }
        };

        let new_id = manifest.entries[next].id.to_string();

        // Save FF state if this is the first selection
        if ids.is_empty() {
            state.bat_book_saved_ff_lo.set(state.ff_freq_lo.get_untracked());
            state.bat_book_saved_ff_hi.set(state.ff_freq_hi.get_untracked());
            state.bat_book_saved_hfr.set(state.hfr_enabled.get_untracked());
        }

        state.bat_book_selected_ids.set(vec![new_id]);

        // Apply FF for the new entry
        let entry = &manifest.entries[next];
        if let Some(idx) = state.current_file_index.get_untracked() {
            let files = state.files.get_untracked();
            if let Some(file) = files.get(idx) {
                let nyquist = file.audio.sample_rate as f64 / 2.0;
                if entry.freq_lo_hz < nyquist {
                    state.ff_freq_lo.set(entry.freq_lo_hz);
                    state.ff_freq_hi.set(entry.freq_hi_hz.min(nyquist));
                    state.hfr_enabled.set(true);
                }
            }
        }
    };

    // Swipe-up to dismiss
    let touch_start_y = RwSignal::new(0.0f64);
    let on_touchstart = move |ev: web_sys::TouchEvent| {
        if let Some(touch) = ev.touches().get(0) {
            touch_start_y.set(touch.client_y() as f64);
        }
    };
    let on_touchend = move |ev: web_sys::TouchEvent| {
        if let Some(touch) = ev.changed_touches().get(0) {
            let dy = touch_start_y.get_untracked() - touch.client_y() as f64;
            if dy > 60.0 {
                state.bat_book_ref_open.set(false);
            }
        }
    };

    view! {
        <div
            class="bat-book-ref-panel"
            on:touchstart=on_touchstart
            on:touchend=on_touchend
            on:wheel=on_wheel
        >
            <div class="ref-panel-header">
                <span class="ref-panel-name">
                    {move || {
                        let entries = selected_entries.get();
                        match entries.len() {
                            0 => String::new(),
                            1 => entries[0].name.to_string(),
                            n => format!("{} (+{})", entries[0].name, n - 1),
                        }
                    }}
                </span>
                <button class="ref-panel-close" on:click=on_close title="Close">
                    "\u{00d7}"
                </button>
            </div>
            <div class="ref-panel-body">
                {move || {
                    let entries = selected_entries.get();
                    entries.into_iter().map(|entry| {
                        view! {
                            <div class="ref-panel-entry">
                                <div class="ref-panel-entry-name">{entry.name}</div>
                                <div class="ref-panel-family">{entry.family}</div>
                                <div class="ref-panel-freq">{entry.freq_range_label()}</div>
                                <div class="ref-panel-call-type">"Call type: " {entry.call_type}</div>
                                <div class="ref-panel-desc">{entry.description}</div>
                            </div>
                        }
                    }).collect_view()
                }}
                <div class="ref-panel-draft-notice">
                    "Draft \u{2014} not verified. Ranges are approximate."
                </div>
            </div>
        </div>
    }
}

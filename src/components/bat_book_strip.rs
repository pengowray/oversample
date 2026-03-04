use leptos::prelude::*;
use crate::state::AppState;
use crate::bat_book::data::get_manifest;
use crate::bat_book::types::{BatBookRegion, BatBookEntry};

/// Horizontal scrolling strip of bat family chips.
/// Sits between the main view and the bottom toolbar.
#[component]
pub fn BatBookStrip() -> impl IntoView {
    let state = expect_context::<AppState>();
    let region_menu_open = RwSignal::new(false);

    let manifest = Memo::new(move |_| {
        let region = state.bat_book_region.get();
        get_manifest(region)
    });

    let on_close = move |_: web_sys::MouseEvent| {
        state.bat_book_open.set(false);
    };

    let on_config = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        region_menu_open.update(|v| *v = !*v);
    };

    view! {
        <div class="bat-book-strip">
            <div class="bat-book-header">
                <span class="bat-book-title">"Bat Book"</span>
                <span class="bat-book-region-label">
                    {move || state.bat_book_region.get().short_label()}
                </span>
                <div class="bat-book-config-wrap">
                    <button
                        class="bat-book-config-btn"
                        on:click=on_config
                        title="Choose region"
                    >
                        "\u{2699}"
                    </button>
                    {move || region_menu_open.get().then(|| {
                        view! {
                            <div class="bat-book-region-menu">
                                {BatBookRegion::ALL.iter().map(|&r| {
                                    let is_active = move || state.bat_book_region.get() == r;
                                    view! {
                                        <button
                                            class=move || if is_active() { "bat-book-region-opt active" } else { "bat-book-region-opt" }
                                            on:click=move |ev: web_sys::MouseEvent| {
                                                ev.stop_propagation();
                                                state.bat_book_region.set(r);
                                                region_menu_open.set(false);
                                            }
                                        >
                                            {r.label()}
                                        </button>
                                    }
                                }).collect_view()}
                            </div>
                        }
                    })}
                </div>
                <div style="flex:1"></div>
                <button
                    class="bat-book-close-btn"
                    on:click=on_close
                    title="Close bat book"
                >
                    "\u{00d7}"
                </button>
            </div>
            <div class="bat-book-scroll">
                {move || {
                    let m = manifest.get();
                    m.entries.iter().map(|entry| {
                        view! { <BatBookChip entry=entry.clone() /> }
                    }).collect_view()
                }}
            </div>
        </div>
    }
}

#[component]
fn BatBookChip(entry: BatBookEntry) -> impl IntoView {
    let state = expect_context::<AppState>();
    let entry_id = entry.id.to_string();
    let entry_id_click = entry_id.clone();
    let freq_lo = entry.freq_lo_hz;
    let freq_hi = entry.freq_hi_hz;
    let name = entry.name;
    let freq_label = entry.freq_range_label();
    let call_type = entry.call_type;

    let is_selected = move || {
        state.bat_book_selected_id.get().as_deref() == Some(entry_id.as_str())
    };

    let on_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        state.bat_book_selected_id.set(Some(entry_id_click.clone()));
        state.bat_book_ref_open.set(true);

        // Set FF range if a file is loaded
        if state.current_file_index.get_untracked().is_some() {
            // Clamp to Nyquist if needed
            let files = state.files.get_untracked();
            let nyquist = state.current_file_index.get_untracked()
                .and_then(|i| files.get(i))
                .map(|f| f.audio.sample_rate as f64 / 2.0)
                .unwrap_or(freq_hi);
            state.ff_freq_lo.set(freq_lo);
            state.ff_freq_hi.set(freq_hi.min(nyquist));
            state.hfr_enabled.set(true);
        }
    };

    let class = move || {
        if is_selected() {
            "bat-book-chip selected"
        } else {
            "bat-book-chip"
        }
    };

    view! {
        <button class=class on:click=on_click>
            <span class="bat-book-chip-name">{name}</span>
            <span class="bat-book-chip-freq">{freq_label} " " {call_type}</span>
        </button>
    }
}

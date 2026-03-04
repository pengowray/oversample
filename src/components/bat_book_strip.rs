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
    let scroll_ref = NodeRef::<leptos::html::Div>::new();

    let manifest = Memo::new(move |_| {
        let region = state.bat_book_region.get();
        get_manifest(region)
    });

    let on_close = move |_: web_sys::MouseEvent| {
        state.bat_book_open.set(false);
    };

    // Clicking the title also closes the strip
    let on_title_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        state.bat_book_open.set(false);
    };

    let on_config = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        region_menu_open.update(|v| *v = !*v);
    };

    // Convert vertical scroll to horizontal scroll in the chip area
    let on_wheel = move |ev: web_sys::WheelEvent| {
        if let Some(el) = scroll_ref.get() {
            let el: &web_sys::HtmlElement = el.as_ref();
            let delta = ev.delta_y();
            if delta.abs() > 0.0 {
                ev.prevent_default();
                el.set_scroll_left(el.scroll_left() + delta as i32);
            }
        }
    };

    view! {
        <div class="bat-book-strip" on:click=move |_| { region_menu_open.set(false); }>
            <div class="bat-book-header">
                <span class="bat-book-title" on:click=on_title_click style="cursor:pointer">"Bat Book"</span>
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
            <div class="bat-book-scroll" node_ref=scroll_ref on:wheel=on_wheel>
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

/// Compute the combined FF range from all selected entries.
/// Returns (lo, hi) or None if nothing selected.
fn combined_ff_range(state: &AppState) -> Option<(f64, f64)> {
    let ids = state.bat_book_selected_ids.get_untracked();
    if ids.is_empty() {
        return None;
    }
    let region = state.bat_book_region.get_untracked();
    let manifest = get_manifest(region);
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for entry in &manifest.entries {
        if ids.iter().any(|id| id == entry.id) {
            lo = lo.min(entry.freq_lo_hz);
            hi = hi.max(entry.freq_hi_hz);
        }
    }
    if lo < hi { Some((lo, hi)) } else { None }
}

/// Apply the combined FF range from selected bat book entries.
/// Shows toasts for out-of-range conditions.
/// If all selected entries lack echolocation (e.g. flying foxes), clears HFR.
fn apply_bat_book_ff(state: &AppState) {
    let Some((lo, hi)) = combined_ff_range(state) else {
        // No valid frequency range — clear HFR if it was set by the bat book
        if !state.bat_book_hfr_suppressed.get_untracked() {
            state.ff_freq_lo.set(0.0);
            state.ff_freq_hi.set(0.0);
            state.hfr_enabled.set(false);
        }
        return;
    };

    // Only apply if a file is loaded
    let files = state.files.get_untracked();
    let Some(idx) = state.current_file_index.get_untracked() else { return };
    let Some(file) = files.get(idx) else { return };
    let nyquist = file.audio.sample_rate as f64 / 2.0;

    if lo >= nyquist {
        // Entirely out of range
        state.show_info_toast(format!(
            "Frequency range {}\u{2013}{} kHz is above this file's Nyquist ({} kHz)",
            (lo / 1000.0) as u32,
            (hi / 1000.0) as u32,
            (nyquist / 1000.0) as u32,
        ));
        return;
    }

    let clamped_hi = hi.min(nyquist);
    if clamped_hi < hi {
        state.show_info_toast(format!(
            "Frequency range clamped to {}\u{2013}{} kHz (file Nyquist: {} kHz)",
            (lo / 1000.0) as u32,
            (clamped_hi / 1000.0) as u32,
            (nyquist / 1000.0) as u32,
        ));
    }

    if state.bat_book_hfr_suppressed.get_untracked() {
        // HFR was manually turned off while bat book is active.
        // Update hfr_saved so re-enabling HFR will apply this range,
        // but don't touch ff or hfr_enabled.
        state.hfr_saved_ff_lo.set(Some(lo));
        state.hfr_saved_ff_hi.set(Some(clamped_hi));
        return;
    }

    // Save the user's previous hfr_saved values before we overwrite them,
    // so deselecting the bat book entry can restore them.
    state.bat_book_saved_hfr_ff_lo.set(state.hfr_saved_ff_lo.get_untracked());
    state.bat_book_saved_hfr_ff_hi.set(state.hfr_saved_ff_hi.get_untracked());
    // Set saved HFR values so that Effect A in HfrButton picks them up
    // instead of using defaults (18 kHz–nyquist).
    state.hfr_saved_ff_lo.set(Some(lo));
    state.hfr_saved_ff_hi.set(Some(clamped_hi));
    state.ff_freq_lo.set(lo);
    state.ff_freq_hi.set(clamped_hi);
    state.hfr_enabled.set(true);
}

#[component]
fn BatBookChip(entry: BatBookEntry) -> impl IntoView {
    let state = expect_context::<AppState>();
    let entry_id = entry.id.to_string();
    let entry_id_for_click = entry_id.clone();
    let name = entry.name;
    let freq_label = entry.freq_range_label();
    let call_type = entry.call_type;

    let is_selected = {
        let eid = entry_id.clone();
        move || {
            state.bat_book_selected_ids.get().iter().any(|id| id == &eid)
        }
    };

    let on_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let ctrl = ev.ctrl_key() || ev.meta_key();
        let shift = ev.shift_key();
        let eid = entry_id_for_click.clone();

        let was_selected = state.bat_book_selected_ids.get_untracked().iter().any(|id| id == &eid);

        if was_selected && !ctrl && !shift {
            // Click selected bat again: deselect and restore previous FF
            let was_suppressed = state.bat_book_hfr_suppressed.get_untracked();
            state.bat_book_selected_ids.set(Vec::new());
            state.bat_book_ref_open.set(false);
            state.bat_book_last_clicked_id.set(None);
            state.bat_book_hfr_suppressed.set(false);
            // Restore saved FF state
            if state.current_file_index.get_untracked().is_some() {
                // Always restore hfr_saved to the user's original values.
                state.hfr_saved_ff_lo.set(state.bat_book_saved_hfr_ff_lo.get_untracked());
                state.hfr_saved_ff_hi.set(state.bat_book_saved_hfr_ff_hi.get_untracked());
                if was_suppressed {
                    // HFR was manually turned off — keep it off, ff stays 0/0.
                    // The restored hfr_saved values are ready if they re-enable HFR.
                } else {
                    let saved_lo = state.bat_book_saved_ff_lo.get_untracked();
                    let saved_hi = state.bat_book_saved_ff_hi.get_untracked();
                    let saved_hfr = state.bat_book_saved_hfr.get_untracked();
                    state.ff_freq_lo.set(saved_lo);
                    state.ff_freq_hi.set(saved_hi);
                    state.hfr_enabled.set(saved_hfr);
                }
            }
            return;
        }

        if ctrl && was_selected {
            // Ctrl/Cmd-click an already-selected bat: remove from selection
            state.bat_book_selected_ids.update(|ids| ids.retain(|id| id != &eid));
            if state.bat_book_selected_ids.get_untracked().is_empty() {
                let was_suppressed = state.bat_book_hfr_suppressed.get_untracked();
                state.bat_book_ref_open.set(false);
                state.bat_book_last_clicked_id.set(None);
                state.bat_book_hfr_suppressed.set(false);
                // Restore saved FF state
                if state.current_file_index.get_untracked().is_some() {
                    state.hfr_saved_ff_lo.set(state.bat_book_saved_hfr_ff_lo.get_untracked());
                    state.hfr_saved_ff_hi.set(state.bat_book_saved_hfr_ff_hi.get_untracked());
                    if !was_suppressed {
                        state.ff_freq_lo.set(state.bat_book_saved_ff_lo.get_untracked());
                        state.ff_freq_hi.set(state.bat_book_saved_ff_hi.get_untracked());
                        state.hfr_enabled.set(state.bat_book_saved_hfr.get_untracked());
                    }
                }
            } else {
                // Recalculate combined range
                apply_bat_book_ff(&state);
            }
            state.bat_book_last_clicked_id.set(Some(eid));
            return;
        }

        // Save current FF state before first selection
        let currently_empty = state.bat_book_selected_ids.get_untracked().is_empty();
        if currently_empty {
            state.bat_book_saved_ff_lo.set(state.ff_freq_lo.get_untracked());
            state.bat_book_saved_ff_hi.set(state.ff_freq_hi.get_untracked());
            state.bat_book_saved_hfr.set(state.hfr_enabled.get_untracked());
        }

        if shift {
            // Shift-click: range select from last clicked to this entry
            let region = state.bat_book_region.get_untracked();
            let manifest = get_manifest(region);
            let last_id = state.bat_book_last_clicked_id.get_untracked();
            let anchor = last_id.as_deref().unwrap_or("");
            let anchor_idx = manifest.entries.iter().position(|e| e.id == anchor);
            let click_idx = manifest.entries.iter().position(|e| e.id == eid.as_str());

            if let (Some(a), Some(b)) = (anchor_idx, click_idx) {
                let lo = a.min(b);
                let hi = a.max(b);
                let range_ids: Vec<String> = manifest.entries[lo..=hi]
                    .iter()
                    .map(|e| e.id.to_string())
                    .collect();
                if ctrl {
                    // Shift+Ctrl: add range to existing selection
                    state.bat_book_selected_ids.update(|ids| {
                        for rid in &range_ids {
                            if !ids.iter().any(|id| id == rid) {
                                ids.push(rid.clone());
                            }
                        }
                    });
                } else {
                    // Shift only: replace selection with range
                    state.bat_book_selected_ids.set(range_ids);
                }
            } else {
                // No anchor or entry not found — treat as normal click
                state.bat_book_selected_ids.set(vec![eid.clone()]);
            }
        } else if ctrl {
            // Ctrl/Cmd-click: add to selection
            state.bat_book_selected_ids.update(|ids| {
                if !ids.iter().any(|id| id == &eid) {
                    ids.push(eid.clone());
                }
            });
        } else {
            // Normal click: replace selection
            state.bat_book_selected_ids.set(vec![eid.clone()]);
        }

        state.bat_book_last_clicked_id.set(Some(eid));
        state.bat_book_ref_open.set(true);
        apply_bat_book_ff(&state);
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

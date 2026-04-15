use leptos::prelude::*;
use wasm_bindgen::JsCast;
use crate::state::{ActiveFocus, AppState, Selection};
use crate::canvas::spectrogram_renderer::freq_to_y;
use crate::components::file_sidebar::settings_panel::{
    toggle_annotation_lock, delete_annotation,
    update_annotation_label, update_annotation_tags,
};
use crate::components::selection_combo_button::annotate_selection;

/// Compute the pixel position of the top-right corner of the transient selection
/// relative to the spectrogram canvas area.
fn selection_top_right(
    sel: &Selection,
    scroll_offset: f64,
    time_resolution: f64,
    zoom: f64,
    canvas_width: f64,
    canvas_height: f64,
    min_freq: f64,
    max_freq: f64,
) -> (f64, f64) {
    let visible_time = (canvas_width / zoom) * time_resolution;
    let start_time = scroll_offset;
    let px_per_sec = canvas_width / visible_time;

    let x1 = ((sel.time_end - start_time) * px_per_sec).min(canvas_width);

    let y0 = match sel.freq_high {
        Some(fh) => freq_to_y(fh, min_freq, max_freq, canvas_height).max(0.0),
        None => 0.0,
    };

    (x1, y0)
}

/// Compute the pixel position of the top-right corner of an annotation.
fn annotation_top_right(
    time_end: f64,
    freq_high: Option<f64>,
    scroll_offset: f64,
    time_resolution: f64,
    zoom: f64,
    canvas_width: f64,
    canvas_height: f64,
    min_freq: f64,
    max_freq: f64,
) -> (f64, f64) {
    let visible_time = (canvas_width / zoom) * time_resolution;
    let start_time = scroll_offset;
    let px_per_sec = canvas_width / visible_time;

    let x1 = ((time_end - start_time) * px_per_sec).min(canvas_width);

    let y0 = match freq_high {
        Some(fh) => freq_to_y(fh, min_freq, max_freq, canvas_height).max(0.0),
        None => 0.0,
    };

    (x1, y0)
}

/// Renders the "..." overflow menus for the transient selection and annotations.
/// Mounted inside `main-overlays` in app.rs.
#[component]
pub fn CanvasOverflowMenus() -> impl IntoView {
    let state = expect_context::<AppState>();

    view! {
        {move || {
            let focus = state.active_focus.get();
            // Close overflow menus when focus doesn't match
            if focus != Some(ActiveFocus::TransientSelection) {
                state.selection_overflow_open.set(false);
            }
            if focus != Some(ActiveFocus::Annotations) {
                state.annotation_overflow_open.set(false);
            }
            match focus {
                Some(ActiveFocus::TransientSelection) => {
                    if state.selection.get().is_some() {
                        Some(view! { <SelectionOverflowMenu /> }.into_any())
                    } else {
                        None
                    }
                }
                Some(ActiveFocus::Annotations) => {
                    let ids = state.selected_annotation_ids.get();
                    if !ids.is_empty() {
                        Some(view! { <AnnotationOverflowMenu /> }.into_any())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }}
    }
}

const BTN_SIZE: f64 = 22.0;
const BTN_MARGIN: f64 = 4.0;

/// "..." overflow button + dropdown for transient selection.
#[component]
fn SelectionOverflowMenu() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = state.selection_overflow_open;

    // Reactive position: top-right corner of selection
    let pos = Signal::derive(move || {
        let sel = state.selection.get()?;
        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let canvas_w = state.spectrogram_canvas_width.get();

        let files = state.files.get();
        let idx = state.current_file_index.get()?;
        let file = files.get(idx)?;
        let time_res = file.spectrogram.time_resolution;
        let file_max_freq = file.spectrogram.max_freq;
        let min_freq = state.min_display_freq.get().unwrap_or(0.0);
        let max_freq = state.max_display_freq.get().unwrap_or(file_max_freq);

        // Estimate canvas height from the spectrogram_canvas_width and aspect
        // We don't have a height signal, so compute from the viewport
        let canvas_h = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector(".spectrogram-container canvas").ok().flatten())
            .map(|el| el.get_bounding_client_rect().height())
            .unwrap_or(400.0);

        let (x, y) = selection_top_right(
            &sel, scroll, time_res, zoom, canvas_w, canvas_h, min_freq, max_freq,
        );
        Some((x, y))
    });

    let sel_details = Signal::derive(move || {
        let sel = state.selection.get()?;
        let d = sel.time_end - sel.time_start;
        if d < 0.0001 { return None; }
        let dur = crate::format_time::format_duration(d, 3);
        let freq = match (sel.freq_low, sel.freq_high) {
            (Some(fl), Some(fh)) => Some(format!("{:.0} \u{2013} {:.0} kHz", fl / 1000.0, fh / 1000.0)),
            _ => None,
        };
        let has_freq = sel.freq_low.is_some() && sel.freq_high.is_some();
        Some((dur, freq, has_freq))
    });

    // Check if selection already matches FF
    let sel_matches_ff = Signal::derive(move || {
        let sel = state.selection.get();
        let ff_lo = state.ff_freq_lo.get();
        let ff_hi = state.ff_freq_hi.get();
        match sel {
            Some(s) => {
                if ff_hi <= ff_lo { return false; }
                match (s.freq_low, s.freq_high) {
                    (Some(sl), Some(sh)) => (sl - ff_lo).abs() < 1.0 && (sh - ff_hi).abs() < 1.0,
                    _ => false,
                }
            }
            None => false,
        }
    });

    let ff_active = Signal::derive(move || {
        state.ff_freq_hi.get() > state.ff_freq_lo.get()
    });

    view! {
        {move || {
            let (x, y) = pos.get().unwrap_or((0.0, 0.0));
            if x <= 0.0 && y <= 0.0 { return None; }

            // Position the button above and to the left of the top-right corner
            let btn_left = (x - BTN_SIZE - BTN_MARGIN).max(0.0);
            let btn_top = (y + BTN_MARGIN).max(0.0);

            Some(view! {
                <div
                    class="canvas-overflow-anchor"
                    style=format!(
                        "position: absolute; left: {:.0}px; top: {:.0}px; pointer-events: auto; z-index: 25;",
                        btn_left, btn_top
                    )
                >
                    <button
                        class="canvas-overflow-btn"
                        title="Selection options"
                        on:click=move |ev| {
                            ev.stop_propagation();
                            is_open.update(|v| *v = !*v);
                        }
                    >
                        "\u{22EF}"
                    </button>

                    // Dropdown menu
                    {move || is_open.get().then(|| {
                        view! {
                            <div
                                class="canvas-overflow-backdrop"
                                on:click=move |_| is_open.set(false)
                            ></div>
                            <div class="canvas-overflow-menu">
                                {move || {
                                    if let Some((dur, freq, has_freq)) = sel_details.get() {
                                        view! {
                                            <div class="canvas-overflow-info">
                                                <div>"Duration: " {dur}</div>
                                                {freq.map(|f| view! { <div>"Freq: " {f}</div> }.into_any())}
                                            </div>
                                            <div class="canvas-overflow-btn-row">
                                                // Remove freq range
                                                <button
                                                    class="canvas-overflow-action-btn"
                                                    class:disabled={!has_freq}
                                                    disabled=move || !has_freq
                                                    title="Remove frequency range"
                                                    on:click=move |_| {
                                                        if let Some(sel) = state.selection.get_untracked() {
                                                            state.selection.set(Some(Selection {
                                                                freq_low: None,
                                                                freq_high: None,
                                                                ..sel
                                                            }));
                                                        }
                                                    }
                                                >
                                                    {"\u{00D7}"}
                                                </button>
                                                // Add freq range from FF
                                                <button
                                                    class="canvas-overflow-action-btn"
                                                    class:disabled={has_freq}
                                                    disabled=move || has_freq
                                                    title="Add frequency range from focus"
                                                    on:click=move |_| {
                                                        if let Some(sel) = state.selection.get_untracked() {
                                                            let ff_lo = state.ff_freq_lo.get_untracked();
                                                            let ff_hi = state.ff_freq_hi.get_untracked();
                                                            if ff_hi > ff_lo {
                                                                state.selection.set(Some(Selection {
                                                                    freq_low: Some(ff_lo),
                                                                    freq_high: Some(ff_hi),
                                                                    ..sel
                                                                }));
                                                            } else {
                                                                // Fall back to display range
                                                                let files = state.files.get_untracked();
                                                                let idx = state.current_file_index.get_untracked().unwrap_or(0);
                                                                let file_max = files.get(idx).map(|f| f.spectrogram.max_freq).unwrap_or(96_000.0);
                                                                let lo = state.min_display_freq.get_untracked().unwrap_or(0.0);
                                                                let hi = state.max_display_freq.get_untracked().unwrap_or(file_max);
                                                                state.selection.set(Some(Selection {
                                                                    freq_low: Some(lo),
                                                                    freq_high: Some(hi),
                                                                    ..sel
                                                                }));
                                                            }
                                                        }
                                                    }
                                                >
                                                    {"+"}
                                                </button>
                                                // Refresh: reset selection freq to FF
                                                <button
                                                    class="canvas-overflow-action-btn"
                                                    class:disabled={move || !ff_active.get() || sel_matches_ff.get()}
                                                    disabled=move || !ff_active.get() || sel_matches_ff.get()
                                                    title="Reset frequency range to match focus"
                                                    on:click=move |_| {
                                                        let ff_lo = state.ff_freq_lo.get_untracked();
                                                        let ff_hi = state.ff_freq_hi.get_untracked();
                                                        if ff_hi > ff_lo {
                                                            if let Some(sel) = state.selection.get_untracked() {
                                                                state.selection.set(Some(Selection {
                                                                    freq_low: Some(ff_lo),
                                                                    freq_high: Some(ff_hi),
                                                                    ..sel
                                                                }));
                                                            }
                                                        }
                                                    }
                                                >
                                                    {"\u{21BB}"}
                                                </button>
                                            </div>
                                            <div class="canvas-overflow-separator"></div>
                                            <button
                                                class="canvas-overflow-item"
                                                on:click=move |_| {
                                                    annotate_selection(&state);
                                                    is_open.set(false);
                                                }
                                            >
                                                {if has_freq { "Annotate Region" } else { "Annotate Segment" }}
                                            </button>
                                        }.into_any()
                                    } else {
                                        view! { <span></span> }.into_any()
                                    }
                                }}
                            </div>
                        }
                    })}
                </div>
            })
        }}
    }
}

/// "..." overflow button + dropdown for selected annotations.
#[component]
fn AnnotationOverflowMenu() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_open = state.annotation_overflow_open;
    let inline_editing = RwSignal::new(false);

    // Reset inline editing when menu closes
    Effect::new(move |_| {
        if !is_open.get() {
            inline_editing.set(false);
        }
    });

    // Reactive position: top-right corner of first selected annotation
    let pos = Signal::derive(move || {
        let ids = state.selected_annotation_ids.get();
        if ids.is_empty() { return None; }
        let idx = state.current_file_index.get()?;
        let store = state.annotation_store.get();
        let set = store.sets.get(idx)?.as_ref()?;
        let ann = set.annotations.iter().find(|a| ids.contains(&a.id))?;

        let region = match &ann.kind {
            crate::annotations::AnnotationKind::Region(r) => r,
            _ => return None,
        };

        let scroll = state.scroll_offset.get();
        let zoom = state.zoom_level.get();
        let canvas_w = state.spectrogram_canvas_width.get();

        let files = state.files.get();
        let file = files.get(idx)?;
        let time_res = file.spectrogram.time_resolution;
        let file_max_freq = file.spectrogram.max_freq;
        let min_freq = state.min_display_freq.get().unwrap_or(0.0);
        let max_freq = state.max_display_freq.get().unwrap_or(file_max_freq);

        let canvas_h = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector(".spectrogram-container canvas").ok().flatten())
            .map(|el| el.get_bounding_client_rect().height())
            .unwrap_or(400.0);

        let (x, y) = annotation_top_right(
            region.time_end, region.freq_high,
            scroll, time_res, zoom, canvas_w, canvas_h, min_freq, max_freq,
        );
        Some((x, y))
    });

    // Get annotation info for display
    let ann_info = Signal::derive(move || {
        let ids = state.selected_annotation_ids.get();
        if ids.len() != 1 { return None; }
        let idx = state.current_file_index.get()?;
        let store = state.annotation_store.get();
        let set = store.sets.get(idx)?.as_ref()?;
        let ann = set.annotations.iter().find(|a| a.id == ids[0])?;
        match &ann.kind {
            crate::annotations::AnnotationKind::Region(r) => {
                Some((
                    ann.id.clone(),
                    r.label.clone(),
                    r.is_locked(),
                    true, // is_region
                ))
            }
            _ => Some((ann.id.clone(), None, false, false)),
        }
    });

    view! {
        {move || {
            let (x, y) = pos.get().unwrap_or((0.0, 0.0));
            if x <= 0.0 && y <= 0.0 { return None; }

            let btn_left = (x - BTN_SIZE - BTN_MARGIN).max(0.0);
            let btn_top = (y + BTN_MARGIN).max(0.0);

            Some(view! {
                <div
                    class="canvas-overflow-anchor"
                    style=format!(
                        "position: absolute; left: {:.0}px; top: {:.0}px; pointer-events: auto; z-index: 25;",
                        btn_left, btn_top
                    )
                >
                    <button
                        class="canvas-overflow-btn"
                        title="Annotation options"
                        on:click=move |ev| {
                            ev.stop_propagation();
                            is_open.update(|v| *v = !*v);
                        }
                    >
                        "\u{22EF}"
                    </button>

                    {move || is_open.get().then(|| {
                        view! {
                            <div
                                class="canvas-overflow-backdrop"
                                on:click=move |_| is_open.set(false)
                            ></div>
                            <div class="canvas-overflow-menu">
                                {move || {
                                    let info = ann_info.get();
                                    if let Some((id, label, is_locked, is_region)) = info {
                                        let id_lock = id.clone();
                                        let id_del = id.clone();
                                        let id_edit = id.clone();
                                        let id_edit_confirm = id.clone();
                                        let lock_label = if is_locked { "\u{1F512} Unlock" } else { "\u{1F513} Lock" };
                                        let new_locked = !is_locked;

                                        if inline_editing.get() {
                                            // Inline editing mode
                                            let label_ref = NodeRef::<leptos::html::Input>::new();
                                            let label_value = RwSignal::new(label.clone().unwrap_or_default());
                                            let initial_tags = {
                                                // Get current tags (tags are on the Annotation, not Region)
                                                let idx = state.current_file_index.get_untracked();
                                                let store = state.annotation_store.get_untracked();
                                                idx.and_then(|i| store.sets.get(i)?.as_ref())
                                                    .and_then(|set| set.annotations.iter().find(|a| a.id == id_edit))
                                                    .map(|ann| ann.tags.join(", "))
                                                    .unwrap_or_default()
                                            };
                                            let tags_value = RwSignal::new(initial_tags.clone());
                                            // Store the annotation id in a signal so closures can share it
                                            let edit_id = StoredValue::new(id_edit_confirm.clone());
                                            // Auto-focus the label input
                                            Effect::new(move |_| {
                                                if let Some(el) = label_ref.get() {
                                                    let _ = el.focus();
                                                }
                                            });
                                            let save_and_close = move || {
                                                let aid = edit_id.get_value();
                                                let lbl = label_value.get_untracked();
                                                let label_opt = if lbl.trim().is_empty() { None } else { Some(lbl.trim().to_string()) };
                                                update_annotation_label(state, &aid, label_opt);
                                                let tval = tags_value.get_untracked();
                                                let tags: Vec<String> = tval.split(',')
                                                    .map(|s: &str| s.trim().to_string())
                                                    .filter(|s: &String| !s.is_empty())
                                                    .collect();
                                                update_annotation_tags(state, &aid, tags);
                                                inline_editing.set(false);
                                            };
                                            let save_close_enter = save_and_close.clone();
                                            let save_close_btn = save_and_close.clone();
                                            view! {
                                                <div class="canvas-overflow-info">
                                                    <div style="font-weight: 600; color: #ccc; font-size: 10px;">"Edit Annotation"</div>
                                                </div>
                                                <div style="padding: 4px 8px;">
                                                    <div style="font-size: 10px; color: #888; margin-bottom: 2px;">"Label"</div>
                                                    <input
                                                        class="sel-combo-input"
                                                        type="text"
                                                        node_ref=label_ref
                                                        prop:value=label.clone().unwrap_or_default()
                                                        on:input=move |ev: web_sys::Event| {
                                                            let input = ev.target().unwrap().unchecked_into::<web_sys::HtmlInputElement>();
                                                            label_value.set(input.value());
                                                        }
                                                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                            if ev.key() == "Escape" {
                                                                ev.prevent_default();
                                                                ev.stop_propagation();
                                                                inline_editing.set(false);
                                                            } else if ev.key() == "Enter" {
                                                                ev.prevent_default();
                                                                save_close_enter();
                                                            }
                                                        }
                                                    />
                                                    <div style="font-size: 10px; color: #888; margin-top: 4px; margin-bottom: 2px;">"Tags (comma separated)"</div>
                                                    <input
                                                        class="sel-combo-input"
                                                        type="text"
                                                        prop:value=initial_tags
                                                        on:input=move |ev: web_sys::Event| {
                                                            let input = ev.target().unwrap().unchecked_into::<web_sys::HtmlInputElement>();
                                                            tags_value.set(input.value());
                                                        }
                                                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                            if ev.key() == "Escape" {
                                                                ev.prevent_default();
                                                                ev.stop_propagation();
                                                                inline_editing.set(false);
                                                            } else if ev.key() == "Enter" {
                                                                ev.prevent_default();
                                                                save_and_close();
                                                            }
                                                        }
                                                    />
                                                </div>
                                                <div style="padding: 4px 8px; display: flex; gap: 4px;">
                                                    <button class="canvas-overflow-item" style="flex: 1; text-align: center;"
                                                        on:click=move |_| save_close_btn()
                                                    >
                                                        "\u{2713} Done"
                                                    </button>
                                                    <button class="canvas-overflow-item" style="flex: 1; text-align: center;"
                                                        on:click=move |_| inline_editing.set(false)
                                                    >
                                                        "Cancel"
                                                    </button>
                                                </div>
                                            }.into_any()
                                        } else {
                                        // Normal menu mode
                                        view! {
                                            {label.map(|l| view! {
                                                <div class="canvas-overflow-info">
                                                    <div style="font-weight: 600; color: #ccc;">{l}</div>
                                                </div>
                                            })}
                                            <button
                                                class="canvas-overflow-item"
                                                on:click=move |_| {
                                                    inline_editing.set(true);
                                                }
                                            >
                                                "\u{270E} Edit label & tags"
                                            </button>
                                            {is_region.then(move || {
                                                view! {
                                                    <button
                                                        class="canvas-overflow-item"
                                                        on:click=move |_| {
                                                            toggle_annotation_lock(state, &id_lock, new_locked);
                                                            is_open.set(false);
                                                        }
                                                    >
                                                        {lock_label}
                                                    </button>
                                                }
                                            })}
                                            <div class="canvas-overflow-separator"></div>
                                            <button
                                                class="canvas-overflow-item danger"
                                                on:click=move |_| {
                                                    delete_annotation(state, &id_del);
                                                    is_open.set(false);
                                                }
                                            >
                                                "\u{00D7} Delete"
                                            </button>
                                        }.into_any()
                                        }
                                    } else {
                                        view! { <span></span> }.into_any()
                                    }
                                }}
                            </div>
                        }
                    })}
                </div>
            })
        }}
    }
}

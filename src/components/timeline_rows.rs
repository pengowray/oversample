//! Labels on the rows of a timeline whose clips overlap in time. Each row
//! shows its file's name and a Solo button that plays that row alone.
//!
//! These are DOM elements over the spectrogram canvas, not drawn on it: the
//! canvas's own pointer handlers take every press on the canvas.

use crate::components::icons::Icon;
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;

/// What a row's label says: its file's name, or how many files it has.
fn row_name(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        many => format!("{} files", many.len()),
    }
}

#[component]
pub fn TimelineRowLabels() -> impl IntoView {
    let state = expect_context::<AppState>();
    move || {
        let tl = state.timeline.active().get()?;
        if tl.row_count < 2 {
            return None;
        }
        let row_count = tl.row_count;
        // With five or more rows a row is short, and its label would cover
        // the start of the strip: those rows get the see-through label
        // (see style.css).
        let compact = row_count >= 5;
        let label_class = if compact {
            "timeline-row-label compact"
        } else {
            "timeline-row-label"
        };
        let labels: Vec<_> = (0..row_count)
            .map(|row| {
                let names: Vec<String> = state.library.files().with(|files| {
                    tl.segments
                        .iter()
                        .filter(|seg| seg.row == row)
                        .filter_map(|seg| files.get(seg.file_index).map(|f| f.name.clone()))
                        .collect()
                });
                let name = row_name(&names);
                let title = names.join(", ");
                let top = format!("{}%", row as f64 * 100.0 / row_count as f64);
                let is_only = move || state.timeline.only_row().get() == Some(row);
                view! {
                    <div class=label_class style:top=top>
                        <span class="timeline-row-name" title=title>{name}</span>
                        <button
                            class=move || if is_only() { "timeline-row-solo on" } else { "timeline-row-solo" }
                            title="Play only this row"
                            aria-pressed=move || if is_only() { "true" } else { "false" }
                            on:click=move |_| {
                                let next = if is_only() { None } else { Some(row) };
                                state.timeline.only_row().set(next);
                                crate::audio::playback::schedule_replay_live(&state);
                            }
                        >"Solo"</button>
                    </div>
                }
            })
            .collect();
        Some(view! { <div class="timeline-rows">{labels}</div> })
    }
}

/// A file's place on the open timeline and two buttons that move it one
/// place. Shown on the file's card in the Files tab and on its row in the
/// Project tab.
///
/// `pos` is the file's place, counted from 0, and `last` the last place.
/// The arrows point up and down, the way the card moves in the list. The
/// tooltips say what the move does on the timeline: earlier or later when
/// the files play one after another, up or down when each has a row.
#[component]
pub fn TimelineMoveButtons(pos: usize, last: usize) -> impl IntoView {
    let state = expect_context::<AppState>();
    let rows = matches!(
        state.timeline.arrangement().get_untracked(),
        crate::project::Arrangement::Together
    );
    let (up_tip, down_tip) = if rows {
        ("Move up", "Move down")
    } else {
        ("Play earlier", "Play later")
    };
    // The card or row under these buttons has its own click: a press on a
    // button must not reach it.
    let move_to = move |to: usize| {
        move |ev: web_sys::MouseEvent| {
            ev.stop_propagation();
            crate::timeline::move_clip(&state, pos, to);
        }
    };
    view! {
        <span class="timeline-order-num" title="Place on the timeline">{pos + 1}</span>
        <button class="timeline-order-btn" title=up_tip aria-label=up_tip
            disabled={pos == 0}
            on:click=move_to(pos.saturating_sub(1))
        ><Icon kind=Icon::ArrowUp /></button>
        <button class="timeline-order-btn" title=down_tip aria-label=down_tip
            disabled={pos >= last}
            on:click=move_to(pos + 1)
        ><Icon kind=Icon::ArrowDown /></button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_named_after_its_file_or_its_file_count() {
        assert_eq!(row_name(&["a.wav".to_string()]), "a.wav");
        assert_eq!(
            row_name(&["a.wav".to_string(), "b.wav".to_string()]),
            "2 files"
        );
        assert_eq!(row_name(&[]), "");
    }
}

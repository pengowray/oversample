//! Labels on the rows of a timeline whose clips overlap in time. Each row
//! shows its file's name and a Solo button that plays that row alone.
//!
//! These are DOM elements over the spectrogram canvas, not drawn on it: the
//! canvas's own pointer handlers take every press on the canvas.

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
                    <div class="timeline-row-label" style:top=top>
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

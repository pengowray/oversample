//! Draws the notes under the Info panel's Sample rate row
//! ([`crate::rate_notes`]).

use crate::rate_notes::{NoteKind, RateNote};
use leptos::prelude::*;

/// The CSS selector of the note of one kind, such as
/// `.rate-note[data-note="heterodyne"]`.
pub(crate) fn note_selector(kind: NoteKind) -> String {
    format!(".rate-note[data-note=\"{}\"]", kind.key())
}

/// The notes, each one a `.metadata-note.rate-note` element with one `div`
/// per line. The notes are siblings of the Sample rate row with no element
/// around them, so the first note's previous sibling is the row.
///
/// - `data-note` names the note's kind ([`NoteKind::key`]).
/// - Each line's class names its role: `rate-note-warning`,
///   `rate-note-plain`, `rate-note-muted` or `rate-note-quote`
///   (`style.css`).
#[component]
pub(crate) fn RateNotes(notes: Vec<RateNote>) -> impl IntoView {
    notes
        .into_iter()
        .map(|note| {
            let lines = note
                .lines
                .into_iter()
                .map(|line| {
                    view! { <div class=format!("rate-note-{}", line.role.key())>{line.text}</div> }
                })
                .collect_view();
            view! { <div class="metadata-note rate-note" data-note=note.kind.key()>{lines}</div> }
        })
        .collect_view()
}

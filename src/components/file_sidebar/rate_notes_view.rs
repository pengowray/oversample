//! Draws the notes under the Info panel's Sample rate row
//! ([`crate::rate_notes`]).

use crate::rate_notes::{NoteAction, NoteKind, RateNote};
use crate::state::AppState;
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
/// - A note's button ([`NoteAction`]) is a `button.rate-note-action` after
///   the first line, with `data-action` naming it. It is disabled while the
///   file is still loading (`loading`).
///
/// `file_id` and `stored_rate` are the file's id and the rate its header
/// states.
#[component]
pub(crate) fn RateNotes(
    notes: Vec<RateNote>,
    file_id: u64,
    stored_rate: u32,
    loading: bool,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    notes
        .into_iter()
        .map(|note| {
            let button = note
                .action
                .map(|action| action_button(state, action, file_id, stored_rate, loading));
            let mut lines = note.lines.into_iter().map(|line| {
                view! { <div class=format!("rate-note-{}", line.role.key())>{line.text}</div> }
            });
            let first = lines.next();
            let rest = lines.collect_view();
            view! {
                <div class="metadata-note rate-note" data-note=note.kind.key()>
                    {first}
                    {button}
                    {rest}
                </div>
            }
        })
        .collect_view()
}

/// The button of a note. The change runs in the click handler. It rebuilds
/// the Info panel, this button included, but the rebuild runs after the
/// handler returns, before the next click can arrive. A second press of the
/// same button sets the same rate and changes nothing.
///
/// The handler holds only `state` and the file's id, so it reads nothing that
/// the rebuild disposes of. It sets no `disabled` property by hand: the
/// rebuild reuses the button element for the next note's button and keeps
/// any property it did not set.
fn action_button(
    state: AppState,
    action: NoteAction,
    file_id: u64,
    stored_rate: u32,
    loading: bool,
) -> impl IntoView {
    let on_click = move |_: web_sys::MouseEvent| {
        crate::te_correction::set_user_time_expansion(state, file_id, action.factor());
    };
    view! {
        <button
            class="hash-calc-btn rate-note-action"
            data-action=action.key()
            title=action.tooltip(stored_rate)
            disabled=loading
            on:click=on_click
        >
            {action.label()}
        </button>
    }
}

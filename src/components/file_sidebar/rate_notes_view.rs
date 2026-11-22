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
///   the note's lines, with `data-action` naming it. It is disabled while
///   the file is still loading (`loading`).
///
/// `file_id` and `stored_rate` are the file's id and the rate its header
/// states.
#[component]
pub(crate) fn RateNotes(
    notes: Vec<RateNote>,
    file_id: u64,
    stored_rate: u32,
    loading: bool,
    kept: crate::rate_notes::Kept,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    notes
        .into_iter()
        .map(|note| {
            let button = note
                .action
                .map(|action| action_button(state, action, file_id, stored_rate, loading, kept));
            let lines = note
                .lines
                .into_iter()
                .map(|line| {
                    view! { <div class=format!("rate-note-{}", line.role.key())>{line.text}</div> }
                })
                .collect_view();
            view! {
                <div class="metadata-note rate-note" data-note=note.kind.key()>
                    {lines}
                    {button}
                </div>
            }
        })
        .collect_view()
}

/// The "Time expansion" row of the Info panel's File group, above the
/// Sample rate row: the factor the file is read with, in a field the user
/// can type another factor into (1 for the rate in the file header). The
/// change is made when the field loses focus or Enter is pressed, and is
/// saved with the file's annotations
/// ([`crate::te_correction::set_factor_by_user`]). A value that can't be
/// used leaves the rate alone and shows why under the row.
///
/// - `factor`: the factor in use, 1 with no correction.
/// - `stored_rate`: the rate the file's header states.
/// - `saved`: whether the file's annotations are saved (false in view-only
///   mode), which the tooltip says.
#[component]
pub(crate) fn TimeExpansionRow(
    file_id: u64,
    factor: f64,
    stored_rate: u32,
    loading: bool,
    saved: bool,
) -> impl IntoView {
    use wasm_bindgen::JsCast;
    let state = expect_context::<AppState>();
    let error = RwSignal::new(None::<String>);
    let shown = crate::rate_notes::factor_text(factor);
    let shown_for_reset = shown.clone();
    let on_change = move |ev: web_sys::Event| {
        let Some(input) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        match crate::rate_notes::parse_factor(&input.value(), stored_rate) {
            Ok(typed) => {
                error.set(None);
                if !crate::te_correction::set_factor_by_user(state, file_id, Some(typed)) {
                    // Nothing changed (the same factor, or the file is busy):
                    // show the factor in use again.
                    input.set_value(&shown_for_reset);
                }
            }
            Err(why) => error.set(Some(why)),
        }
    };
    let tip = crate::rate_notes::factor_tooltip(saved);
    view! {
        <div class="setting-row metadata-row metadata-row-inline te-factor-row">
            <span class="setting-label" title=tip.clone()>{crate::rate_notes::FACTOR_LABEL}</span>
            <label class="te-factor-value" title=tip>
                "\u{d7}"
                <input
                    class="factor-input te-factor-input"
                    type="text"
                    inputmode="decimal"
                    autocomplete="off"
                    aria-label=crate::rate_notes::FACTOR_ARIA
                    prop:value=shown
                    disabled=loading
                    on:change=on_change
                />
            </label>
        </div>
        {move || error.get().map(|why| view! {
            <div class="metadata-note te-factor-error" role="alert">{why}</div>
        })}
    }
}

/// The button of a note. The change runs in the click handler. It rebuilds
/// the Info panel, this button included, but the rebuild runs after the
/// handler returns, before the next click can arrive.
///
/// The rebuild reuses the button element for the next note's button, so
/// "Correct for ×10 time expansion" becomes "Remove ×10 correction" under
/// the pointer and keeps keyboard focus; "Remove ×N correction" becomes
/// "Correct for ×N time expansion" the same way. So that one double-click,
/// or Enter held down, does not apply the correction and take it back, the
/// second click of a double-click (`detail` 2 or more) and a key repeat do
/// nothing. A separate press of the button does what its label says.
///
/// The handler holds only `state` and the file's id, so it reads nothing that
/// the rebuild disposes of. It sets no `disabled` property by hand: the
/// rebuilt button would keep it.
fn action_button(
    state: AppState,
    action: NoteAction,
    file_id: u64,
    stored_rate: u32,
    loading: bool,
    kept: crate::rate_notes::Kept,
) -> impl IntoView {
    let on_click = move |ev: web_sys::MouseEvent| {
        if ev.detail() > 1 {
            return;
        }
        if crate::te_correction::set_time_expansion(state, file_id, action.target()) {
            crate::te_correction::save_factor(state, file_id);
        }
    };
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.repeat() {
            ev.prevent_default();
        }
    };
    view! {
        <button
            class="hash-calc-btn rate-note-action"
            data-action=action.key()
            title=action.tooltip(stored_rate, kept)
            disabled=loading
            on:click=on_click
            on:keydown=on_keydown
        >
            {action.label()}
        </button>
    }
}

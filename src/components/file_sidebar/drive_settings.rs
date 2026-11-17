//! The "Google Drive" group in the settings panel (`crate::cloud`). Shown in
//! the browser only, and only when the server offers Google sign-in.

use crate::cloud::sync;
use crate::state::store_fields::*;
use crate::state::{AppState, DriveProblem};
use leptos::prelude::*;

const SIGNED_OUT_NOTE: &str = "Sign in to keep copies of your annotations and projects in an \"Oversample\" folder in your Google Drive.";
const SIGNING_IN_NOTE: &str = "Use the Google window to finish signing in.";
const POPUP_BLOCKED: &str = "Google sign-in pop-up blocked. Allow pop-ups for this site, then press \"Sign in with Google\".";
const SESSION_ENDED: &str = "Signed out of Google Drive. Changes are saved on this device.";
const UNREACHABLE: &str =
    "Can't reach Google Drive. Changes are saved on this device and will upload later.";
const FULL: &str =
    "Google Drive storage is full. Changes are saved on this device. Free up space, then press Retry.";
const REFUSED: &str =
    "Google Drive rejected a change. Changes are saved on this device and will upload later.";

fn problem_text(p: DriveProblem) -> &'static str {
    match p {
        DriveProblem::Unreachable => UNREACHABLE,
        DriveProblem::Full => FULL,
        DriveProblem::Refused => REFUSED,
    }
}

/// The one-line state of the Drive copy while signed in.
fn status_text(uploading: bool, waiting: usize) -> String {
    if uploading {
        "Saving\u{2026}".to_string()
    } else if waiting == 1 {
        "1 file waiting to upload".to_string()
    } else if waiting > 1 {
        format!("{waiting} files waiting to upload")
    } else {
        "Saved to Google Drive".to_string()
    }
}

#[component]
pub(super) fn DriveSettings() -> impl IntoView {
    let state = expect_context::<AppState>();
    if state.is_tauri {
        return view! { <span></span> }.into_any();
    }

    // The note says what signing in does, so it comes before the button.
    let signed_out = move || {
        let signing_in = state.drive.signing_in().get();
        // While the Google window is open, its line replaces the description.
        let note = if signing_in {
            view! { <div class="drive-note">{SIGNING_IN_NOTE}</div> }.into_any()
        } else if state.drive.session_ended().get() {
            view! { <div class="drive-note">{SESSION_ENDED}</div> }.into_any()
        } else {
            view! { <div class="drive-note">{SIGNED_OUT_NOTE}</div> }.into_any()
        };
        view! {
            {note}
            <div class="setting-row">
                <button
                    class="setting-button drive-button"
                    disabled=signing_in
                    on:click=move |_| sync::sign_in(state)
                >{if signing_in { "Signing in\u{2026}" } else { "Sign in with Google" }}</button>
            </div>
            {(!signing_in && state.drive.popup_blocked().get()).then(|| view! {
                <div class="drive-note setting-error">{POPUP_BLOCKED}</div>
            })}
        }
    };

    // The account gets a line of its own so a full address fits the narrow
    // sidebar; the state line sits beside Sign out.
    let signed_in = move || {
        view! {
            <div class="setting-row">
                <span
                    class="drive-account"
                    title=move || state.drive.email().get().unwrap_or_default()
                >
                    {move || state.drive.email().get().unwrap_or_else(|| "Signed in".to_string())}
                </span>
            </div>
            {move || match state.drive.problem().get() {
                Some(p) => view! {
                    <div class="drive-note drive-warning">{problem_text(p)}</div>
                    <div class="setting-row">
                        <button
                            class="setting-button drive-button"
                            on:click=move |_| sync::retry()
                        >"Retry"</button>
                        <button
                            class="setting-button drive-button"
                            on:click=move |_| sync::sign_out(state)
                        >"Sign out"</button>
                    </div>
                }.into_any(),
                None => view! {
                    <div class="setting-row">
                        <span class="drive-status">
                            {move || status_text(
                                state.drive.uploading().get(),
                                state.drive.waiting().get(),
                            )}
                        </span>
                        <button
                            class="setting-button drive-button"
                            on:click=move |_| sync::sign_out(state)
                        >"Sign out"</button>
                    </div>
                }.into_any(),
            }}
        }
    };

    view! {
        <Show when=move || state.drive.available().get()>
            <div class="setting-group drive-settings">
                <div class="setting-group-title">"Google Drive"</div>
                {move || if state.drive.signed_in().get() {
                    signed_in().into_any()
                } else {
                    signed_out().into_any()
                }}
            </div>
        </Show>
    }
    .into_any()
}

//! The "Google Drive" group in the settings panel (`crate::cloud`). Shown
//! only when Google sign-in is offered: on the web when the server has it set
//! up, on the desktop when the build has a Google client (never on Android).

use crate::cloud::sync;
use crate::state::store_fields::*;
use crate::state::{AppState, DriveProblem, DriveSignInProblem};
use leptos::prelude::*;

const SIGNED_OUT_NOTE: &str = "Sign in to keep copies of your annotations and projects in an \"Oversample\" folder in your Google Drive.";
const SIGNING_IN_NOTE: &str = "Use the Google window to finish signing in.";
/// Desktop: sign-in happens in the system's web browser.
const SIGNING_IN_NOTE_DESKTOP: &str = "Google's sign-in page is open in your web browser. Sign in there and allow access to Google Drive.";
const SIGN_IN_DENIED: &str = "Sign-in cancelled.";
const SIGN_IN_TIMED_OUT: &str = "Sign-in timed out after 5 minutes.";
const SIGN_IN_FAILED: &str = "Sign-in failed.";
/// Desktop with no credential store (some Linux set-ups).
const NOT_KEPT: &str = "Signed in until Oversample closes, because no keyring is available.";
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

    // The note says what signing in does, so it comes before the button.
    let signed_out = move || {
        let signing_in = state.drive.signing_in().get();
        // While the Google window is open, its line replaces the description.
        let desktop = state.drive.desktop().get();
        let note = if signing_in {
            let text = if desktop {
                SIGNING_IN_NOTE_DESKTOP
            } else {
                SIGNING_IN_NOTE
            };
            view! { <div class="drive-note">{text}</div> }.into_any()
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
                // Desktop: the wait for the browser can be stopped.
                {(signing_in && desktop).then(|| view! {
                    <button
                        class="setting-button drive-button"
                        on:click=move |_| sync::cancel_sign_in()
                    >"Cancel"</button>
                })}
            </div>
            {(!signing_in && state.drive.popup_blocked().get()).then(|| view! {
                <div class="drive-note setting-error">{POPUP_BLOCKED}</div>
            })}
            {(!signing_in).then(|| state.drive.sign_in_problem().get()).flatten().map(|p| {
                let (text, class) = match p {
                    DriveSignInProblem::Denied => (SIGN_IN_DENIED, "drive-note"),
                    DriveSignInProblem::TimedOut => (SIGN_IN_TIMED_OUT, "drive-note drive-warning"),
                    DriveSignInProblem::Failed => (SIGN_IN_FAILED, "drive-note drive-warning"),
                };
                view! { <div class=class>{text}</div> }
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
            {move || (!state.drive.persistent().get()).then(|| view! {
                <div class="drive-note">{NOT_KEPT}</div>
            })}
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

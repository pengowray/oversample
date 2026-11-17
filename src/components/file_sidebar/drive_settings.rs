//! The "Google Drive" group in the settings panel (`crate::cloud`). Shown
//! only when Google sign-in is offered: on the web when the server has it set
//! up, on the desktop when the build has a Google client (never on Android).

use crate::cloud::sync;
use crate::state::store_fields::*;
use crate::state::{AppState, DesktopDriveCheck, DriveProblem, DriveSignInProblem};
use leptos::prelude::*;

const SIGNED_OUT_NOTE: &str = "Sign in to keep copies of your annotations and projects in an \"Oversample\" folder in your Google Drive.";
const SIGNING_IN_NOTE: &str = "Use the Google window to finish signing in.";
/// Desktop: sign-in happens in the system's web browser.
const SIGNING_IN_NOTE_DESKTOP: &str = "Google's sign-in page is open in your web browser. Sign in there and allow access to Google Drive.";
const SIGN_IN_DENIED: &str = "Sign-in cancelled.";
const SIGN_IN_TIMED_OUT: &str = "Sign-in timed out after 5 minutes.";
const SIGN_IN_FAILED: &str = "Sign-in failed.";
/// Desktop, while it makes no Drive copies (see `cloud::sync`).
const DESKTOP_NOT_SAVING: &str = "The desktop app does not save to Google Drive yet.";
const CHECK_RUNNING: &str = "Looking for the Oversample folder in Google Drive\u{2026}";
const CHECK_NOT_FOUND: &str = "No Oversample folder is visible to the desktop app.";
const CHECK_FAILED: &str = "Couldn't check Google Drive.";

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The result of the desktop app's read-only check of Google Drive.
fn check_text(c: DesktopDriveCheck) -> String {
    match c {
        DesktopDriveCheck::Checking => CHECK_RUNNING.to_string(),
        DesktopDriveCheck::Found {
            annotations,
            projects,
        } => format!(
            "Oversample folder found in Google Drive: {}, {}.",
            count(annotations, "annotation file", "annotation files"),
            count(projects, "project", "projects")
        ),
        DesktopDriveCheck::NotFound => CHECK_NOT_FOUND.to_string(),
        DesktopDriveCheck::Failed => CHECK_FAILED.to_string(),
    }
}

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

/// Drive no longer shows the Oversample folder (see `DriveProblem`).
const FOLDER_MISSING: &str =
    "Can't find the Oversample folder in Google Drive. Copying to Drive is paused. Everything is still saved in this browser. If you deleted the folder, it may be in Drive's trash.";

fn problem_text(p: DriveProblem) -> String {
    match p {
        DriveProblem::Unreachable => UNREACHABLE.to_string(),
        DriveProblem::Full => FULL.to_string(),
        DriveProblem::Refused => REFUSED.to_string(),
        DriveProblem::FolderMissing => FOLDER_MISSING.to_string(),
        DriveProblem::FilesMissing(n) => format!(
            "{n} files are missing from the Oversample folder in Google Drive. They are still saved in \
             this browser. If you didn't delete them, check Drive's trash."
        ),
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

    // Web: the state of the Drive copies, or the last problem.
    let web_status = move || {
        view! {
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
            {move || if state.drive.desktop().get() {
                // Desktop: no copies yet; the read-only check's result.
                let check = state.drive.desktop_check().get();
                view! {
                    {check.map(|c| {
                        let class = if c == DesktopDriveCheck::Failed {
                            "drive-note drive-warning"
                        } else {
                            "drive-note"
                        };
                        view! { <div class=class>{check_text(c)}</div> }
                    })}
                    <div class="drive-note">{DESKTOP_NOT_SAVING}</div>
                    <div class="setting-row">
                        {(check == Some(DesktopDriveCheck::Failed)).then(|| view! {
                            <button
                                class="setting-button drive-button"
                                on:click=move |_| sync::retry()
                            >"Retry"</button>
                        })}
                        <button
                            class="setting-button drive-button"
                            on:click=move |_| sync::sign_out(state)
                        >"Sign out"</button>
                    </div>
                }.into_any()
            } else {
                view! { {web_status()} }.into_any()
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

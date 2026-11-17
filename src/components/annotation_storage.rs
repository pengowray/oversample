//! The line that says where the open file's annotations are saved, shown
//! under the Annotations heading in the right sidebar and in an
//! annotation's "⋯" menu on the spectrogram.
//!
//! Row 1 is the copy on this device: in this browser (web), on this device
//! (desktop and Android), or not saved for a view-only file. Row 2 is the
//! second copy, when there is one: Google Drive (web, signed in), with the
//! state of this file's upload, or the .batm file beside the recording
//! (desktop and Android). Row 2 keeps its height through every Drive state,
//! so the list below does not move after each save. The Drive row is a link
//! to the Google Drive settings, not a grey button: its text is a state, and
//! a button would read as "do this now".

use crate::cloud::sync::{self, FileDriveState};
use crate::state::store_fields::*;
use crate::state::{AppState, DriveProblem, LeftSidebarTab};
use leptos::prelude::*;

const SAVED_IN_BROWSER: &str = "Saved in this browser";
const SAVED_ON_DEVICE: &str = "Saved on this device";
const NOT_SAVED_VIEW_ONLY: &str = "Not saved: this file is view-only";
const DRIVE_BUTTON_TIP: &str = "Open the Google Drive settings";
const NOT_COPIED_TIP: &str =
    "Google Drive has annotations for a different recording with the same name and size.";

/// The text of the Google Drive row, and whether it is a warning.
fn drive_text(s: FileDriveState) -> (&'static str, bool) {
    match s {
        FileDriveState::Saved => ("Saved in Google Drive", false),
        FileDriveState::Checking => ("Checking Google Drive\u{2026}", false),
        FileDriveState::Waiting => ("Waiting to upload to Google Drive", false),
        FileDriveState::Uploading => ("Uploading to Google Drive\u{2026}", false),
        FileDriveState::Problem(DriveProblem::Unreachable) => ("Can't reach Google Drive", true),
        FileDriveState::Problem(DriveProblem::Full) => ("Google Drive is full", true),
        FileDriveState::Problem(DriveProblem::Refused) => ("Google Drive rejected a change", true),
        FileDriveState::NotCopied => ("Not copied to Google Drive", true),
    }
}

/// Show the Google Drive group in Settings (left sidebar).
fn open_drive_settings(state: AppState) {
    state.panels.left_tab().set(LeftSidebarTab::Settings);
    state.panels.left_collapsed().set(false);
    if state.status.is_mobile().get_untracked() {
        // On a phone the two sidebars cover each other.
        state.panels.right_collapsed().set(true);
    }
    wasm_bindgen_futures::spawn_local(async {
        crate::web_util::sleep_ms(50).await;
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector(".drive-settings").ok().flatten())
        {
            el.scroll_into_view();
        }
    });
}

/// What the line needs to know about the open file.
#[derive(Clone, PartialEq)]
struct OpenFile {
    read_only: bool,
    had_sidecar: bool,
    /// The audio file's name, without folders.
    name: String,
    /// Key of its saved annotations, once its hash is known.
    key: Option<String>,
}

#[component]
pub fn AnnotationStorage() -> impl IntoView {
    let state = expect_context::<AppState>();
    let open_file = Memo::new(move |_| {
        let idx = state.library.current_index().get()?;
        state.library.files().with(|files| {
            let f = files.get(idx)?;
            let name = f.name.rsplit(['/', '\\']).next().unwrap_or(&f.name);
            Some(OpenFile {
                read_only: f.read_only,
                had_sidecar: f.had_sidecar,
                name: name.to_string(),
                key: f
                    .identity
                    .as_ref()
                    .filter(|_| !crate::file_identity::spot_hash_pending(f.id))
                    .map(crate::opfs::opfs_key),
            })
        })
    });

    move || {
        let file = open_file.get()?;
        if file.read_only {
            return Some(
                view! {
                    <div class="annotation-storage">
                        <div class="annotation-storage-row drive-warning">{NOT_SAVED_VIEW_ONLY}</div>
                    </div>
                }
                .into_any(),
            );
        }
        if state.is_tauri {
            let batm = format!("{}.batm", file.name);
            return Some(
                view! {
                    <div class="annotation-storage">
                        <div class="annotation-storage-row">{SAVED_ON_DEVICE}</div>
                        {file.had_sidecar.then(|| view! {
                            <div class="annotation-storage-row with-name" title=format!("{batm} is in the same folder as the recording")>
                                "Also saved in "
                                <span class="annotation-storage-name">{batm.clone()}</span>
                                " beside the recording"
                            </div>
                        })}
                    </div>
                }
                .into_any(),
            );
        }
        let key = file.key.clone();
        Some(
            view! {
                <div class="annotation-storage">
                    <div class="annotation-storage-row">{SAVED_IN_BROWSER}</div>
                    {move || {
                        let s = sync::file_drive_state(state, key.as_deref()?)?;
                        let (text, warning) = drive_text(s);
                        let class = if warning {
                            "annotation-storage-drive drive-warning"
                        } else {
                            "annotation-storage-drive"
                        };
                        let title = if s == FileDriveState::NotCopied {
                            format!("{NOT_COPIED_TIP} {DRIVE_BUTTON_TIP}")
                        } else {
                            DRIVE_BUTTON_TIP.to_string()
                        };
                        Some(view! {
                            <button
                                class=class
                                title=title
                                on:click=move |_| open_drive_settings(state)
                            >{text}</button>
                        })
                    }}
                </div>
            }
            .into_any(),
        )
    }
}

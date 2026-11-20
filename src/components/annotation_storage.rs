//! The line that says where the open file's annotations are saved, shown
//! under the Annotations heading in the right sidebar and in an
//! annotation's "⋯" menu on the spectrogram.
//!
//! Row 1 is the copy on this device: in this browser (web), on this device
//! (desktop and Android), or not saved for a view-only file. Then, on the
//! desktop and Android, the .batm file beside the recording when there is
//! one. Last, when signed in (web and desktop), Google Drive with the state
//! of this file's upload. The Drive row keeps its height through every
//! state, so the list below does not move after each save. The Drive row is a link
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
const NOT_IN_DRIVE: &str = "Not in Google Drive";
/// Desktop: the per-folder setting (the folder's `.oversample` file).
const BESIDE_LABEL: &str = "Keep .batm beside recordings in this folder";
const BESIDE_TIP: &str = "Annotations of the recordings in this folder are saved only in a .batm file beside each recording, not in the app's folder or Google Drive. The setting is kept in a .oversample file in the folder.";
const NOT_IN_DRIVE_TIP: &str =
    "Made while signed out. Move it with \"Move to Google Drive\" in the Google Drive settings.";
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
        FileDriveState::Problem(DriveProblem::FolderMissing) => {
            ("Google Drive copying paused", true)
        }
        // Not shown per file (`sync::file_drive_state`).
        FileDriveState::Problem(DriveProblem::FilesMissing(_)) => {
            ("Waiting to upload to Google Drive", false)
        }
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

/// Desktop: turn the folder setting on or off, then save every open file
/// of that folder so its annotations move to their new home (on: the .batm
/// beside the recording; off: the app's folder, with the .batm kept up to
/// date as before).
fn set_folder_beside(state: AppState, audio_path: String, on: bool) {
    fn folder(p: &str) -> &str {
        p.rfind(['/', '\\']).map_or("", |i| &p[..i])
    }
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = crate::opfs::tauri_set_folder_beside(&audio_path, on).await {
            state.show_error_toast(&format!("Couldn't save the folder setting: {e}"));
            return;
        }
        let dir = folder(&audio_path).to_string();
        let has_set = |id: u64| state.annotations.store().with_untracked(|s| s.get(id).is_some());
        let mut to_save = Vec::new();
        state.library.files().update(|files| {
            for (idx, f) in files.iter_mut().enumerate() {
                let path = f.identity.as_ref().and_then(|i| i.file_path.as_deref());
                if !path.is_some_and(|p| folder(p) == dir) {
                    continue;
                }
                f.beside_only = on;
                if has_set(f.id) {
                    // Off: the .batm written while it was on stays and is
                    // kept up to date.
                    f.had_sidecar = true;
                    to_save.push(idx);
                }
            }
        });
        for idx in to_save {
            crate::opfs::save_annotations(state, idx);
        }
    });
}

/// What the line needs to know about the open file.
#[derive(Clone, PartialEq)]
struct OpenFile {
    read_only: bool,
    had_sidecar: bool,
    /// The folder keeps annotations beside its recordings (desktop).
    beside_only: bool,
    /// Full path of the audio file (desktop, opened from disk).
    path: Option<String>,
    /// The audio file's name, without folders.
    name: String,
    /// Key of its saved annotations, once its hash is known.
    key: Option<String>,
    /// `LoadedFile.id`.
    id: u64,
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
                id: f.id,
                read_only: f.read_only,
                had_sidecar: f.had_sidecar,
                beside_only: f.beside_only,
                path: f.identity.as_ref().and_then(|i| i.file_path.clone()),
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
        // Row 1: this device. Desktop: then the .batm beside the recording,
        // when there is one. Last: the Google Drive copy, when signed in.
        let device = if state.is_tauri {
            SAVED_ON_DEVICE
        } else {
            SAVED_IN_BROWSER
        };
        let desktop = state.is_tauri && !crate::cloud::store::android_app();
        let beside = (state.is_tauri && (file.had_sidecar || file.beside_only))
            .then(|| format!("{}.batm", file.name));
        // With the folder setting on, the .batm is the only copy.
        let only_beside = desktop && file.beside_only;
        let key = file.key.clone();
        let folder_toggle = file.path.clone().filter(|_| desktop).map(|path| {
            let on = file.beside_only;
            view! {
                <label class="annotation-storage-row annotation-storage-toggle" title=BESIDE_TIP>
                    <input
                        type="checkbox"
                        checked=on
                        on:change=move |ev| set_folder_beside(state, path.clone(), event_target_checked(&ev))
                    />
                    {BESIDE_LABEL}
                </label>
            }
        });
        Some(
            view! {
                <div class="annotation-storage">
                    {(!only_beside).then(|| view! {
                        <div class="annotation-storage-row">{device}</div>
                    })}
                    {beside.map(|batm| view! {
                        <div class="annotation-storage-row with-name" title=format!("{batm} is in the same folder as the recording")>
                            {if only_beside { "Saved in " } else { "Also saved in " }}
                            <span class="annotation-storage-name">{batm.clone()}</span>
                            " beside the recording"
                        </div>
                    })}
                    {folder_toggle}
                    {move || {
                        // Only a set whose home is the signed-in account has a
                        // Drive copy. One from the device store (made while
                        // signed out) says so, until "Move to Google Drive".
                        // (The count changes when a move is done.)
                        let _ = state.drive.device_only().get();
                        let home = crate::cloud::store::home_of(file.id);
                        if home == crate::cloud::store::Store::Device {
                            let copies_here = state.drive.signed_in().get()
                                && !state.drive.check_only().get();
                            return copies_here.then(|| view! {
                                <button
                                    class="annotation-storage-drive"
                                    title=NOT_IN_DRIVE_TIP
                                    on:click=move |_| open_drive_settings(state)
                                >{NOT_IN_DRIVE}</button>
                            }.into_any());
                        }
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
                        }.into_any())
                    }}
                </div>
            }
            .into_any(),
        )
    }
}

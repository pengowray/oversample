//! The folder for new recordings on the desktop (`crate::recording_folder`):
//! the "Recordings" group in Settings, and its one-line form under "To file"
//! in the Record menu.

use crate::recording_folder::{self as rf, RecordingFolder};
use crate::state::AppState;
use leptos::prelude::*;

/// Shown when the folder the user chose can't be used now.
fn unusable_note(chosen: &str) -> String {
    format!("Can't save to {chosen}, so new recordings are saved in the default folder for now.")
}
const MUSIC_MISSING_NOTE: &str =
    "Can't save to the Music folder, so new recordings are saved in Oversample's data folder.";

/// Settings > Recordings (desktop only).
#[component]
pub fn RecordingFolderSettings() -> impl IntoView {
    let state = expect_context::<AppState>();
    let folder = rf::folder();
    rf::refresh();
    move || {
        let f: RecordingFolder = folder.get()?;
        let reset = f.is_custom.then(|| {
            let label = rf::reset_label(&f);
            view! {
                <button class="setting-button folder-button" title=f.default_path.clone()
                    on:click=move |_| rf::reset(state)>{label}</button>
            }
        });
        let note = if let Some(chosen) = &f.unusable_custom {
            Some(("drive-note drive-warning", unusable_note(chosen)))
        } else if f.music_missing && !f.is_custom {
            Some(("drive-note drive-warning", MUSIC_MISSING_NOTE.to_string()))
        } else {
            None
        }
        .map(|(class, text)| view! { <div class=class>{text}</div> });
        Some(view! {
            <div class="setting-group" data-testid="recording-folder-settings">
                <div class="setting-group-title">"Recordings"</div>
                <div class="setting-row">
                    <span class="setting-label">"Folder for new recordings"</span>
                </div>
                <div class="folder-path path-text" title=f.path.clone()>{rf::path_view(&f.path)}</div>
                {note}
                <div class="setting-row folder-buttons">
                    <button class="setting-button folder-button"
                        on:click=move |_| rf::choose(state)>"Change folder\u{2026}"</button>
                    {reset}
                </div>
            </div>
        })
    }
}

/// Record menu, under "To file": the folder in one line, shortened in the
/// middle, with the full path on hover.
#[component]
pub fn RecordMenuFolderLine() -> impl IntoView {
    let state = expect_context::<AppState>();
    let folder = rf::folder();
    rf::refresh();
    move || {
        let f: RecordingFolder = folder.get()?;
        let (head, tail) = rf::head_and_tail(&f.short_path);
        let warn = f.unusable_custom.is_some();
        let title = match &f.unusable_custom {
            Some(chosen) => format!("{}\n{}", f.path, unusable_note(chosen)),
            None => f.path.clone(),
        };
        Some(view! {
            <div class="rec-folder-line" class:rec-folder-warn=warn title=title
                data-testid="rec-folder-line">
                <span class="rec-folder-label">"Saves to"</span>
                <span class="rec-folder-path">
                    <span class="path-head">{head}</span>
                    <span class="path-tail">{tail}</span>
                </span>
                <button class="setting-button folder-button"
                    on:click=move |ev| {
                        ev.stop_propagation();
                        rf::choose(state);
                    }>"Change\u{2026}"</button>
            </div>
        })
    }
}

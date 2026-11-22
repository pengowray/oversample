//! Writing the time expansion factor into the audio file itself: the TE
//! field of a WAV file's GUANO metadata.
//!
//! The factor the user sets in the Info panel is saved in the recording's
//! .batm file ([`crate::te_correction`]), which only Oversample reads. This
//! module puts the factor where other programs read it too.
//!
//! - **Desktop app:** the file on disk is changed in place
//!   ([`write_in_place`]), after the user confirms. The .batm file gets a
//!   [`FileEdit`]: the file's size and hashes before and after, and the bytes
//!   that put the original file back. [`undo_in_place`] applies them, and the
//!   desktop command writes the file only when the result has the original
//!   size and SHA-256.
//! - **Browser:** a page can't change the file the user opened, so the
//!   edited file is downloaded as a copy ([`save_copy`]). Nothing is recorded,
//!   since the opened file is unchanged.
//!
//! The change is worked out by `oversample_core::audio::guano_edit`. Only the
//! `guan` chunk and the RIFF size change, never the audio.
//!
//! A changed file has a new size and new file hashes, and it opens corrected
//! from its own metadata. So its saved annotations are moved with it: they
//! are converted to the seconds and Hz the changed file opens in, stored
//! under the changed file's name and size, and the file is opened again.

use crate::annotations::{AnnotationSet, FileEdit, SavedTimeExpansion};
use crate::state::{AppState, LoadedFile};
use crate::te_correction::RateChange;
use crate::types::{AudioFormat, TimeExpansionOrigin};
use oversample_ipc::file_edit::{FileEditOutcome, FileHashes, SavedSplice};

/// [`FileEdit::what`] for a change of GUANO's TE field.
pub const KIND_GUANO_TE: &str = "guano_te";

/// What the Info panel offers for a file's GUANO.
#[derive(Clone, Debug, PartialEq)]
pub enum GuanoAction {
    /// Desktop: set TE in the file on disk.
    WriteInPlace { factor: f64 },
    /// Browser: download a copy of the file with TE set.
    SaveCopy { factor: f64 },
    /// Desktop: put back the bytes the file had before Oversample set TE.
    Undo { factor: f64, made_at: String },
}

/// The facts [`action_for`] decides from.
#[derive(Clone, Debug, Default)]
pub struct GuanoFacts {
    pub is_tauri: bool,
    /// The file is at a path on disk the desktop app can write.
    pub has_path: bool,
    /// The browser still holds the file the user opened.
    pub has_web_file: bool,
    pub is_wav: bool,
    /// View-only: nothing about the file is saved.
    pub read_only: bool,
    pub file_size: u64,
    /// The factor the file is read with, and whether the file's own
    /// metadata gave it.
    pub factor: Option<(f64, bool)>,
    /// The latest change saved with the file's annotations.
    pub last_edit: Option<FileEdit>,
}

impl GuanoFacts {
    pub fn of(is_tauri: bool, file: &LoadedFile, last_edit: Option<FileEdit>) -> Self {
        use crate::audio::streaming_source::FileHandle;
        let path = file.identity.as_ref().and_then(|id| id.file_path.as_ref());
        GuanoFacts {
            is_tauri,
            has_path: path.is_some(),
            has_web_file: matches!(file.hash_source, Some(FileHandle::WebFile(_))),
            is_wav: file.audio.metadata.format == AudioFormat::Wav,
            read_only: file.read_only,
            file_size: file.audio.metadata.file_size as u64,
            factor: file
                .audio
                .metadata
                .time_expansion
                .map(|te| (te.factor, te.origin == TimeExpansionOrigin::File)),
            last_edit,
        }
    }
}

/// What to offer. Undo comes first: a file Oversample changed, still at the
/// size it was changed to. Otherwise a WAV file read with a factor that is
/// not in its own metadata can have the factor written.
pub fn action_for(f: &GuanoFacts) -> Option<GuanoAction> {
    if !f.is_wav {
        return None;
    }
    let can_write_disk = f.is_tauri && f.has_path && !f.read_only;
    if can_write_disk {
        if let Some(edit) = f
            .last_edit
            .as_ref()
            .filter(|e| e.what == KIND_GUANO_TE && e.changed.file_size == f.file_size)
        {
            return Some(GuanoAction::Undo {
                factor: edit.factor,
                made_at: edit.made_at.clone(),
            });
        }
    }
    let (factor, from_file) = f.factor?;
    if from_file {
        return None;
    }
    if can_write_disk {
        Some(GuanoAction::WriteInPlace { factor })
    } else if !f.is_tauri && f.has_web_file {
        Some(GuanoAction::SaveCopy { factor })
    } else {
        None
    }
}

fn te(factor: f64) -> String {
    oversample_core::audio::guano_edit::te_value(factor)
}

/// The factor as the panel writes it: "×10".
fn times(factor: f64) -> String {
    crate::wikimedia::format_factor(factor)
}

/// The label of the Undo button, which the confirmation before a change
/// names.
const UNDO_LABEL: &str = "Restore original audio file";

impl GuanoAction {
    /// The button's label. The factor is written "×10", as in the "Remove
    /// ×10 correction" button above it. The tooltips, the questions and the
    /// note name the GUANO line itself, "TE: 10", which is what the reader
    /// will look for in other programs.
    pub fn label(&self) -> String {
        match self {
            GuanoAction::WriteInPlace { factor } => {
                format!("Write {} to GUANO metadata", times(*factor))
            }
            GuanoAction::SaveCopy { factor } => {
                format!("Download copy with {} in GUANO", times(*factor))
            }
            GuanoAction::Undo { .. } => UNDO_LABEL.to_string(),
        }
    }

    /// The button's tooltip.
    pub fn tooltip(&self) -> String {
        match self {
            GuanoAction::WriteInPlace { factor } => format!(
                "Writes the {} time expansion factor into the audio file's GUANO metadata, \
                 as the TE field (TE: {}), so other programs read the factor too. The audio \
                 samples stay the same; the audio file's size and hashes change. The change \
                 can be undone.",
                times(*factor),
                te(*factor)
            ),
            GuanoAction::SaveCopy { factor } => format!(
                "Downloads a copy of the audio file with the {} time expansion factor \
                 written into the copy's GUANO metadata, as the TE field (TE: {}), so other \
                 programs read the factor too. The audio file you opened is unchanged.",
                times(*factor),
                te(*factor)
            ),
            GuanoAction::Undo { factor, .. } => format!(
                "Restores the audio file's original bytes from the recording's .batm file, \
                 and checks the restored file against the original SHA-256 hash. Oversample \
                 still reads the recording with {} time expansion, because the factor stays \
                 saved in the recording's .batm file.",
                times(*factor)
            ),
        }
    }

    /// The line shown above the Undo button: what Oversample changed, and
    /// when. `None` for the other actions, whose button says it all.
    pub fn note(&self) -> Option<String> {
        match self {
            GuanoAction::Undo { factor, made_at } => {
                let day = made_at.get(..10).unwrap_or(made_at);
                Some(format!(
                    "Oversample changed this audio file on {day}: it set TE: {} ({}) in the \
                     GUANO metadata. The original file's size and hashes are saved in the \
                     recording's .batm file.",
                    te(*factor),
                    times(*factor)
                ))
            }
            _ => None,
        }
    }

    /// The name of the action in the button's `data-action` attribute.
    pub fn key(&self) -> &'static str {
        match self {
            GuanoAction::WriteInPlace { .. } => "guano-write",
            GuanoAction::SaveCopy { .. } => "guano-copy",
            GuanoAction::Undo { .. } => "guano-undo",
        }
    }

    /// The question asked before the desktop app changes the file on disk.
    pub fn confirm_text(&self, name: &str) -> Option<String> {
        match self {
            GuanoAction::WriteInPlace { factor } => Some(format!(
                "Write {} time expansion to {name}?\n\nOversample will set TE: {} in the \
                 audio file's GUANO metadata. The audio samples stay the same. The audio \
                 file's size and hashes will change.\n\nYou can undo the change with \
                 \"{UNDO_LABEL}\". The original file's size and hashes, and the bytes needed \
                 to restore the file, are saved in the recording's .batm file.",
                times(*factor),
                te(*factor)
            )),
            GuanoAction::Undo { factor, made_at } => {
                let day = made_at.get(..10).unwrap_or(made_at);
                Some(format!(
                    "Restore the original {name}?\n\nOversample will put back the audio \
                     file's original bytes and check the restored file against the original \
                     SHA-256 hash. The GUANO metadata will be as it was before {day}.\n\n\
                     Oversample will still read this recording with {} time expansion.",
                    times(*factor)
                ))
            }
            GuanoAction::SaveCopy { .. } => None,
        }
    }
}

/// The name of the downloaded copy: "rec.wav" with TE 10 is "rec TE10.wav".
pub fn copy_name(name: &str, factor: f64) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    format!("{stem} TE{}{ext}", te(factor))
}

/// What both desktop actions need to know about the open file.
struct OnDisk {
    path: String,
    name: String,
    /// The rate the file's header states.
    stored_rate: u32,
    /// The rate the file's annotation files are in now
    /// ([`crate::te_correction::to_opening_rate`]).
    opening_rate: u32,
    had_sidecar: bool,
    beside_only: bool,
    content_hash: Option<String>,
    full_blake3: Option<String>,
    /// The key the file's annotations are stored under now.
    old_key: Option<String>,
    /// The key under the file's name and size, where they are stored for a
    /// file opened before its hash was known, or moved here by this module.
    old_fallback_key: String,
    set: AnnotationSet,
}

fn on_disk(state: AppState, file_id: u64) -> Result<OnDisk, String> {
    use crate::state::store_fields::*;
    use leptos::prelude::*;
    let idx = state
        .file_idx_for_id(file_id)
        .ok_or("The file is no longer open.")?;
    let mut found = state.library.files().with_untracked(|files| {
        let f = files.get(idx)?;
        if f.loading_id.is_some() {
            return None;
        }
        let id = f.identity.as_ref()?;
        let stored_rate = f.audio.stored_rate();
        Some(OnDisk {
            path: id.file_path.clone()?,
            name: f.name.clone(),
            stored_rate,
            opening_rate: f
                .opened_time_expansion
                .map_or(stored_rate, |te| te.corrected_rate()),
            had_sidecar: f.had_sidecar,
            beside_only: f.beside_only,
            content_hash: id.content_hash.clone(),
            full_blake3: id.full_blake3.clone(),
            old_key: Some(crate::opfs::opfs_key(id)),
            old_fallback_key: crate::opfs::opfs_fallback_key(
                &f.name,
                f.audio.metadata.file_size as u64,
            ),
            set: AnnotationSet::new_with_metadata(
                id.clone(),
                &f.audio,
                f.cached_peak_db,
                f.cached_full_peak_db,
            ),
        })
    });
    let on_disk = found
        .as_mut()
        .ok_or("The file is still loading, or is not a file on disk.")?;
    if let Some(set) = state
        .annotations
        .store()
        .with_untracked(|store| store.get(file_id).cloned())
    {
        on_disk.set = set;
    }
    // In the seconds and Hz the file opens in, as every saved set is.
    on_disk.set = crate::te_correction::to_file_units_at(state, idx, on_disk.set.clone());
    found.ok_or_else(|| "The file is not a file on disk.".to_string())
}

/// Store `set` for the file as it is on disk now: `file_size` bytes,
/// opening at `opening_rate`. The set is converted to those seconds and Hz
/// and stored under the changed file's name and size, where the loader
/// looks when it finds nothing under the file's hash, and under `also_key`
/// when given. The copy stored for the file as it was is removed: it is out
/// of date, and it would be found first if the file had those bytes again.
async fn save_moved_set(
    file: &OnDisk,
    mut set: AnnotationSet,
    file_size: u64,
    opening_rate: u32,
    also_key: Option<&str>,
) -> Result<(), String> {
    if opening_rate != file.opening_rate {
        RateChange::new(file.opening_rate, opening_rate).annotation_set(&mut set);
    }
    // The hashes of the whole file are those of the file before. The audio
    // is the same, so the content hash still identifies it.
    let mut identity = crate::file_identity::identity_layer1(&file.name, file_size);
    identity.content_hash = file.content_hash.clone();
    identity.file_path = Some(file.path.clone());
    set.file_identity = identity;
    set.touch();

    let new_key = crate::opfs::opfs_fallback_key(&file.name, file_size);
    let mut kept = vec![new_key.clone()];
    if !file.beside_only {
        let yaml = yaml_serde::to_string(&set).map_err(|e| e.to_string())?;
        crate::opfs::tauri_save_central(&new_key, &yaml).await?;
        if let Some(key) = also_key.filter(|k| *k != new_key) {
            crate::opfs::tauri_save_central(key, &yaml).await?;
            kept.push(key.to_string());
        }
    }
    if file.had_sidecar || file.beside_only {
        let yaml = crate::opfs::sidecar_yaml_without_full_path(&set);
        crate::opfs::tauri_save_sidecar(&file.path, &yaml).await?;
    }
    for old_key in file.old_key.iter().chain([&file.old_fallback_key]) {
        if kept.contains(old_key) {
            continue;
        }
        if let Err(e) = crate::opfs::tauri_delete_central(old_key).await {
            log::warn!("Could not remove the annotations stored under {old_key}: {e}");
        }
    }
    Ok(())
}

/// Close the open file without saving anything more for it. Its set is
/// already stored for the file as it is on disk now, and a save made as it
/// closes would write the old set back.
fn close_without_saving(state: AppState, file_id: u64) {
    use crate::state::store_fields::*;
    use leptos::prelude::*;
    state.library.files().update(|files| {
        if let Some(f) = files.iter_mut().find(|f| f.id == file_id) {
            f.read_only = true;
        }
    });
    if let Some(idx) = state.file_idx_for_id(file_id) {
        crate::components::file_sidebar::files_panel::remove_file_at(&state, idx);
    }
    state.annotations.store().update(|store| {
        store.remove(file_id);
    });
}

/// Open the file at `path` (`file_size` bytes) again and make it the
/// current file, so the app reads it and its set from disk.
async fn reopen(state: AppState, path: &str, name: &str, file_size: u64) -> Result<(), String> {
    use crate::state::store_fields::*;
    use leptos::prelude::*;
    let load_id = state.loading_start(name);
    let loaded = crate::components::file_sidebar::load_native_wav(
        path.to_string(),
        file_size,
        state,
        load_id,
    )
    .await;
    state.loading_done(load_id);
    loaded?;
    let count = state.library.files().with_untracked(|f| f.len());
    if count > 0 {
        state.library.current_index().set(Some(count - 1));
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct SetTeArgs<'a> {
    path: &'a str,
    factor: f64,
}

#[derive(serde::Serialize)]
struct UndoArgs<'a> {
    path: &'a str,
    undo: &'a [SavedSplice],
    original: &'a FileHashes,
}

async fn undo_on_disk(
    path: &str,
    undo: &[SavedSplice],
    original: &FileHashes,
) -> Result<FileHashes, String> {
    crate::tauri_bridge::tauri_invoke_typed_args(
        "file_edit_undo",
        &UndoArgs {
            path,
            undo,
            original,
        },
    )
    .await
}

/// What a change to the file on disk came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    /// The file was changed and is open again.
    Reopened,
    /// The file was changed and its annotations stored, but it could not be
    /// opened again. Holds why.
    NotReopened(String),
}

/// Desktop: set GUANO's TE field of file `file_id` on disk to `factor`,
/// record the change with the file's annotations, and open the file again.
/// The caller has asked the user ([`GuanoAction::confirm_text`]).
///
/// If the annotations can't be stored for the changed file, the file is put
/// back as it was, so the bytes that undo the change are never held only in
/// memory. `Err` says which of the two states the file is in.
pub async fn write_in_place(state: AppState, file_id: u64, factor: f64) -> Result<Done, String> {
    let file = on_disk(state, file_id)?;
    let outcome: FileEditOutcome = crate::tauri_bridge::tauri_invoke_typed_args(
        "guano_set_te",
        &SetTeArgs {
            path: &file.path,
            factor,
        },
    )
    .await?;
    let mut set = file.set.clone();
    // The desktop command hashed the file it read. A BLAKE3 the app worked
    // out earlier is used only if the command gave none.
    let mut original = outcome.original.clone();
    if original.blake3.is_none() {
        original.blake3 = file.full_blake3.clone();
    }
    set.file_edits.push(FileEdit {
        what: KIND_GUANO_TE.to_string(),
        factor,
        made_at: crate::annotations::now_iso8601(),
        original,
        changed: outcome.changed.clone(),
        undo: outcome.undo.clone(),
        opened_rate: file.opening_rate,
        key_before: file.old_key.clone(),
    });
    // The file's own metadata gives the factor from now on.
    set.time_expansion = None;
    let opening_rate = (f64::from(file.stored_rate) * factor).round() as u32;
    let size = outcome.changed.file_size;
    if let Err(why) = save_moved_set(&file, set, size, opening_rate, None).await {
        return Err(
            match undo_on_disk(&file.path, &outcome.undo, &outcome.original).await {
                Ok(_) => format!(
                    "its annotations could not be saved ({why}), so the audio file was put \
                     back as it was"
                ),
                Err(undo_why) => format!(
                    "the audio file was changed, but its annotations could not be saved \
                     ({why}) and the file could not be put back ({undo_why})"
                ),
            },
        );
    }
    close_without_saving(state, file_id);
    Ok(match reopen(state, &file.path, &file.name, size).await {
        Ok(()) => Done::Reopened,
        Err(why) => Done::NotReopened(why),
    })
}

/// Desktop: put back the bytes file `file_id` had before the latest change
/// Oversample made to it. The recording keeps the factor, as one the user
/// set, so it is read the same way afterwards. The annotations go back to
/// the seconds and Hz the original file opened in, under the key they had.
pub async fn undo_in_place(state: AppState, file_id: u64) -> Result<Done, String> {
    let file = on_disk(state, file_id)?;
    let edit = file
        .set
        .file_edits
        .last()
        .cloned()
        .ok_or("no change to this file is saved with its annotations")?;
    let restored = undo_on_disk(&file.path, &edit.undo, &edit.original).await?;
    let mut set = file.set.clone();
    set.file_edits.pop();
    set.time_expansion = Some(SavedTimeExpansion {
        from_source: None,
        set_by_user: Some(edit.factor),
    });
    // A record from before the rate was saved: the rate in the header.
    let opening_rate = if edit.opened_rate > 0 {
        edit.opened_rate
    } else {
        file.stored_rate
    };
    let size = restored.file_size;
    save_moved_set(&file, set, size, opening_rate, edit.key_before.as_deref())
        .await
        .map_err(|why| {
            format!(
                "the audio file has its original bytes again, but its annotations could \
                 not be saved ({why})"
            )
        })?;
    close_without_saving(state, file_id);
    Ok(match reopen(state, &file.path, &file.name, size).await {
        Ok(()) => Done::Reopened,
        Err(why) => Done::NotReopened(why),
    })
}

/// Browser: download a copy of file `file_id` with GUANO's TE field set to
/// `factor`. Returns the copy's name.
pub async fn save_copy(state: AppState, file_id: u64, factor: f64) -> Result<String, String> {
    use crate::audio::streaming_source::FileHandle;
    use crate::state::store_fields::*;
    use leptos::prelude::*;
    let (name, web_file) = state
        .library
        .files()
        .with_untracked(|files| {
            let f = files.iter().find(|f| f.id == file_id)?;
            match &f.hash_source {
                Some(FileHandle::WebFile(file)) => Some((f.name.clone(), file.clone())),
                _ => None,
            }
        })
        .ok_or("The browser no longer holds the file you opened.")?;
    let bytes = crate::components::file_sidebar::read_file_bytes(&web_file).await?;
    let patch = oversample_core::audio::guano_edit::set_te(&bytes, factor)?;
    let changed = patch.apply(&bytes)?;
    let copy = copy_name(&name, factor);
    crate::audio::export::trigger_browser_download(&changed, &copy);
    Ok(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> GuanoFacts {
        GuanoFacts {
            is_tauri: true,
            has_path: true,
            has_web_file: false,
            is_wav: true,
            read_only: false,
            file_size: 1_000,
            factor: Some((10.0, false)),
            last_edit: None,
        }
    }

    fn edit(changed_size: u64) -> FileEdit {
        FileEdit {
            what: KIND_GUANO_TE.to_string(),
            factor: 10.0,
            made_at: "2026-11-22T05:00:00.000Z".to_string(),
            original: FileHashes {
                file_size: 966,
                ..Default::default()
            },
            changed: FileHashes {
                file_size: changed_size,
                ..Default::default()
            },
            undo: Vec::new(),
            opened_rate: 38_400,
            key_before: None,
        }
    }

    #[test]
    fn the_desktop_app_offers_to_write_a_factor_the_file_does_not_state() {
        assert_eq!(
            action_for(&facts()),
            Some(GuanoAction::WriteInPlace { factor: 10.0 })
        );
        // The file's own metadata already gives the factor.
        let mut f = facts();
        f.factor = Some((10.0, true));
        assert_eq!(action_for(&f), None);
        // No factor in use, not a WAV file, or view-only.
        for change in [
            (|f: &mut GuanoFacts| f.factor = None) as fn(&mut GuanoFacts),
            |f| f.is_wav = false,
            |f| f.read_only = true,
            |f| f.has_path = false,
        ] {
            let mut f = facts();
            change(&mut f);
            assert_eq!(action_for(&f), None);
        }
    }

    #[test]
    fn a_file_oversample_changed_offers_undo_while_it_is_that_size() {
        let mut f = facts();
        f.factor = Some((10.0, true));
        f.last_edit = Some(edit(1_000));
        assert_eq!(
            action_for(&f),
            Some(GuanoAction::Undo {
                factor: 10.0,
                made_at: "2026-11-22T05:00:00.000Z".to_string()
            })
        );
        // Another size: not the file the change left.
        f.last_edit = Some(edit(1_002));
        assert_eq!(action_for(&f), None);
    }

    #[test]
    fn the_browser_offers_a_copy() {
        let mut f = facts();
        f.is_tauri = false;
        f.has_path = false;
        assert_eq!(action_for(&f), None);
        f.has_web_file = true;
        assert_eq!(action_for(&f), Some(GuanoAction::SaveCopy { factor: 10.0 }));
    }

    #[test]
    fn texts_name_the_factor_and_the_day() {
        assert_eq!(
            GuanoAction::WriteInPlace { factor: 2.5 }.label(),
            "Write \u{d7}2.5 to GUANO metadata"
        );
        assert_eq!(copy_name("bat call.wav", 10.0), "bat call TE10.wav");
        assert_eq!(copy_name("noext", 2.5), "noext TE2.5");
        let undo = GuanoAction::Undo {
            factor: 10.0,
            made_at: "2026-11-22T05:00:00.000Z".to_string(),
        };
        assert_eq!(
            undo.note().as_deref(),
            Some(
                "Oversample changed this audio file on 2026-11-22: it set TE: 10 (\u{d7}10) in \
                 the GUANO metadata. The original file's size and hashes are saved in the \
                 recording's .batm file."
            )
        );
        assert!(undo.confirm_text("a.wav").is_some());
        assert!(GuanoAction::SaveCopy { factor: 10.0 }
            .confirm_text("a.wav")
            .is_none());
    }
}

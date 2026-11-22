//! Commands that change an audio file on disk: setting the time expansion
//! factor in a WAV file's GUANO metadata, and taking such a change back.
//!
//! Each command works out the new bytes in memory, checks them, writes them
//! to a temporary file beside the audio file, and renames that over the audio
//! file. A failure at any point before the rename leaves the audio file as
//! it was.

use oversample_core::audio::guano_edit::{self, FilePatch, Splice};
use oversample_ipc::file_edit::{from_hex, to_hex, FileEditOutcome, FileHashes, SavedSplice};
use sha2::{Digest, Sha256};
use std::path::Path;

/// The largest file these commands change. The file is held in memory twice.
const MAX_EDIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

fn hashes(bytes: &[u8]) -> FileHashes {
    FileHashes {
        file_size: bytes.len() as u64,
        sha256: Some(sha256_hex(bytes)),
        blake3: Some(blake3::hash(bytes).to_hex().to_string()),
    }
}

fn read_for_edit(path: &Path) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("Could not read the file: {e}"))?
        .len();
    if size > MAX_EDIT_BYTES {
        return Err("Files over 2 GB can't be changed yet.".into());
    }
    std::fs::read(path).map_err(|e| format!("Could not read the file: {e}"))
}

/// Replace `path` with `bytes`: written to a temporary file in the same
/// folder, flushed to disk, then renamed over `path`. The file keeps its
/// permissions.
fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".oversample-tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        std::fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Could not write the file: {e}")
    })
}

fn saved(patch: &FilePatch) -> Vec<SavedSplice> {
    patch
        .splices
        .iter()
        .map(|s| SavedSplice {
            offset: s.offset,
            old: to_hex(&s.old),
            new: to_hex(&s.new),
        })
        .collect()
}

fn patch_from(saved: &[SavedSplice]) -> Result<FilePatch, String> {
    let splices = saved
        .iter()
        .map(|s| {
            Some(Splice {
                offset: s.offset,
                old: from_hex(&s.old)?,
                new: from_hex(&s.new)?,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or("The saved undo data is damaged.")?;
    Ok(FilePatch { splices })
}

/// Set GUANO's TE field of the WAV file at `path` to `factor`, in place.
/// Before the file is written, the change is checked: the new bytes hold the
/// factor, and the undo patch gives the original bytes back.
pub fn set_te_on_disk(path: &Path, factor: f64) -> Result<FileEditOutcome, String> {
    if !(factor.is_finite() && factor > 0.0) {
        return Err("The time expansion factor must be a number over 0.".into());
    }
    let original = read_for_edit(path)?;
    let patch = guano_edit::set_te(&original, factor)?;
    let changed = patch.apply(&original)?;
    let undo = patch.inverse();
    let reads_back = oversample_core::audio::guano::parse_guano(&changed).is_some_and(|g| {
        g.fields
            .iter()
            .any(|(k, v)| k == "TE" && v.trim() == guano_edit::te_value(factor))
    });
    if !reads_back || undo.apply(&changed).as_deref() != Ok(&original[..]) {
        return Err("The change could not be checked, so the file is left as it is.".into());
    }
    let outcome = FileEditOutcome {
        original: hashes(&original),
        changed: hashes(&changed),
        undo: saved(&undo),
    };
    replace_file(path, &changed)?;
    Ok(outcome)
}

/// Take back a change to the file at `path` with its saved undo patch. The
/// file is written only when the result has the size, SHA-256 and BLAKE3
/// the file had before the change (`original`).
pub fn undo_on_disk(
    path: &Path,
    undo: &[SavedSplice],
    original: &FileHashes,
) -> Result<FileHashes, String> {
    let current = read_for_edit(path)?;
    let restored = patch_from(undo)?.apply(&current).map_err(|_| {
        "The file has changed since Oversample changed it, so the change can't be undone."
            .to_string()
    })?;
    let now = hashes(&restored);
    if now.file_size != original.file_size
        || (original.sha256.is_some() && now.sha256 != original.sha256)
        || (original.blake3.is_some() && now.blake3 != original.blake3)
    {
        return Err(
            "Undoing would not give the original file back, so the file is left as it is.".into(),
        );
    }
    replace_file(path, &restored)?;
    Ok(now)
}

#[tauri::command]
pub fn guano_set_te(path: String, factor: f64) -> Result<FileEditOutcome, String> {
    set_te_on_disk(Path::new(&path), factor)
}

#[tauri::command]
pub fn file_edit_undo(
    path: String,
    undo: Vec<SavedSplice>,
    original: FileHashes,
) -> Result<FileHashes, String> {
    undo_on_disk(Path::new(&path), &undo, &original)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav() -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&[1, 0, 1, 0]);
        b.extend_from_slice(&38_400u32.to_le_bytes());
        b.extend_from_slice(&76_800u32.to_le_bytes());
        b.extend_from_slice(&[2, 0, 16, 0]);
        b.extend_from_slice(b"data");
        b.extend_from_slice(&64u32.to_le_bytes());
        b.extend((0..64u8).map(|i| i.wrapping_mul(7)));
        let size = (b.len() - 8) as u32;
        b[4..8].copy_from_slice(&size.to_le_bytes());
        b
    }

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("oversample-edit-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rec.wav");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn sets_the_factor_and_undo_gives_the_original_file_back() {
        let original = wav();
        let path = temp_file("roundtrip", &original);
        let outcome = set_te_on_disk(&path, 10.0).unwrap();
        let changed = std::fs::read(&path).unwrap();
        assert_ne!(changed, original);
        assert_eq!(outcome.original, hashes(&original));
        assert_eq!(outcome.changed, hashes(&changed));
        // Both hashes of both files, as the Info panel shows them.
        assert_eq!(
            outcome.original.blake3.as_deref(),
            Some(blake3::hash(&original).to_hex().as_str())
        );
        assert!(outcome.changed.blake3.is_some() && outcome.changed.sha256.is_some());
        assert!(changed.ends_with(b"GUANO|Version: 1.0\nTE: 10\n"));
        // No temporary file is left beside it.
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );

        let restored = undo_on_disk(&path, &outcome.undo, &outcome.original).unwrap();
        assert_eq!(restored, outcome.original);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn undo_leaves_a_file_changed_since() {
        let path = temp_file("changed-since", &wav());
        let outcome = set_te_on_disk(&path, 10.0).unwrap();
        let mut edited = std::fs::read(&path).unwrap();
        let last = edited.len() - 2;
        edited[last] = b'9';
        std::fs::write(&path, &edited).unwrap();
        assert!(undo_on_disk(&path, &outcome.undo, &outcome.original).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), edited);
        // A patch that applies but gives another file is refused too.
        std::fs::write(&path, wav()).unwrap();
        let outcome = set_te_on_disk(&path, 10.0).unwrap();
        let mut wrong = outcome.original.clone();
        wrong.sha256 = Some("00".repeat(32));
        assert!(undo_on_disk(&path, &outcome.undo, &wrong).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_file_that_is_not_a_wav_is_left_alone() {
        let path = temp_file("not-wav", b"OggS not a wav file at all");
        assert!(set_te_on_disk(&path, 10.0).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"OggS not a wav file at all");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}

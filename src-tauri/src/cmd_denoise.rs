//! Denoised copies of long recordings, written to the folder recordings are
//! saved in (`crate::save_folder`) as the webview makes them.
//!
//! The webview sends the WAV in parts, as raw IPC bodies, because the copy is
//! too large to hold whole (and WebKitGTK has no OPFS to write it to). Parts
//! go to `<name>.<run>.part`, which [`denoise_copy_commit`] renames to
//! `<name>` and [`denoise_copy_discard`] deletes. The folder is picked when a
//! copy's first part is written and kept for that copy ([`CopyDirs`]), so
//! changing the save folder part way through does not split a copy.
//!
//! Only names that end in " (denoised).wav" and have no folder in them are
//! accepted, so no command can touch another file.

use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

/// The end of every copy's name: `DENOISED_SUFFIX` in the frontend's
/// `file_groups.rs`, with the extension.
const NAME_END: &str = " (denoised).wav";

/// The folder each copy being written is in, by name and run.
#[derive(Default)]
pub struct CopyDirs(Mutex<HashMap<(String, u32), PathBuf>>);

/// The folder of the copy `name`, `run`: the one its first part went to, or
/// the first save folder that exists or can be made.
fn copy_dir(app: &tauri::AppHandle, name: &str, run: u32) -> Result<PathBuf, String> {
    let key = (name.to_string(), run);
    if let Some(dir) = app.state::<CopyDirs>().0.lock().unwrap().get(&key) {
        return Ok(dir.clone());
    }
    crate::save_folder::dirs(app)?
        .into_iter()
        .find(|d| std::fs::create_dir_all(d).is_ok())
        .ok_or_else(|| "no folder to save the copy in".to_string())
}

/// Start the part file of copy `name`, `run` in the first save folder where
/// that works, and remember the folder.
fn start_part(app: &tauri::AppHandle, name: &str, run: u32) -> Result<std::fs::File, String> {
    let part_name = format!("{}.{run}.part", checked_name(name)?);
    let mut errors = Vec::new();
    for dir in crate::save_folder::dirs(app)? {
        let created = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::File::create(dir.join(&part_name)));
        match created {
            Ok(f) => {
                app.state::<CopyDirs>()
                    .0
                    .lock()
                    .unwrap()
                    .insert((name.to_string(), run), dir);
                return Ok(f);
            }
            Err(e) => errors.push(format!("{}: {e}", dir.display())),
        }
    }
    Err(errors.join("; "))
}

fn forget(app: &tauri::AppHandle, name: &str, run: u32) {
    app.state::<CopyDirs>()
        .0
        .lock()
        .unwrap()
        .remove(&(name.to_string(), run));
}

fn checked_name(name: &str) -> Result<&str, String> {
    let plain = !name.contains(['/', '\\', '\0']) && name != NAME_END;
    if plain && name.ends_with(NAME_END) {
        Ok(name)
    } else {
        Err(format!("'{name}' is not a denoised copy's name"))
    }
}

fn part_path(app: &tauri::AppHandle, name: &str, run: u32) -> Result<PathBuf, String> {
    Ok(copy_dir(app, name, run)?.join(format!("{}.{run}.part", checked_name(name)?)))
}

/// The file name from its UTF-8 bytes in hexadecimal. Header values are
/// ASCII, and a file name need not be.
fn name_from_hex(hex: &str) -> Result<String, String> {
    let bad = || "bad x-copy-name".to_string();
    if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return Err(bad());
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|_| bad())?;
    String::from_utf8(bytes).map_err(|_| bad())
}

/// Write the next part of a copy. Bytes from [`crate::ipc_bytes::body`]; headers `x-copy-name` (the file
/// name's UTF-8 bytes in hexadecimal), `x-copy-run` (a number that tells this
/// copy from an earlier, cancelled one of the same name) and `x-copy-offset`
/// (byte offset). Offset 0 starts the file.
#[tauri::command]
pub fn denoise_copy_write(
    app: tauri::AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let bytes = crate::ipc_bytes::body(&request)?;
    let header = |name: &str| crate::ipc_bytes::header(&request, name);
    let name = name_from_hex(&header("x-copy-name")?)?;
    let run: u32 = header("x-copy-run")?
        .parse()
        .map_err(|_| "bad x-copy-run".to_string())?;
    let offset: u64 = header("x-copy-offset")?
        .parse()
        .map_err(|_| "bad x-copy-offset".to_string())?;
    let mut f = if offset == 0 {
        start_part(&app, &name, run)?
    } else {
        let path = part_path(&app, &name, run)?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let len = f.metadata().map_err(|e| e.to_string())?.len();
        if len != offset {
            return Err(format!("expected offset {len}, got {offset}"));
        }
        f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        f
    };
    f.write_all(&bytes).map_err(|e| e.to_string())
}

/// Finish a copy: check the part file's size and give it its name, replacing
/// an older copy of that name. Returns the copy's path.
#[tauri::command]
pub fn denoise_copy_commit(
    app: tauri::AppHandle,
    name: String,
    run: u32,
    size: u64,
) -> Result<String, String> {
    let part = part_path(&app, &name, run)?;
    let dir = copy_dir(&app, &name, run)?;
    forget(&app, &name, run);
    let len = std::fs::metadata(&part).map_err(|e| e.to_string())?.len();
    if len != size {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{name} has {len} bytes; expected {size}"));
    }
    let path = dir.join(&name);
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

/// Delete an unfinished copy. No part file is not an error.
#[tauri::command]
pub fn denoise_copy_discard(app: tauri::AppHandle, name: String, run: u32) -> Result<(), String> {
    let part = part_path(&app, &name, run);
    forget(&app, &name, run);
    match std::fs::remove_file(part?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_denoised_copys_name_is_accepted() {
        assert!(checked_name("bat (denoised).wav").is_ok());
        assert!(checked_name("XC1 \u{2013} Bät (denoised).wav").is_ok());
        assert!(checked_name("bat.wav").is_err());
        assert!(checked_name(" (denoised).wav").is_err());
        assert!(checked_name("../bat (denoised).wav").is_err());
        assert!(checked_name("a\\b (denoised).wav").is_err());
    }

    #[test]
    fn the_name_header_is_the_names_bytes_in_hexadecimal() {
        assert_eq!(name_from_hex("62c3a474").unwrap(), "b\u{e4}t");
        assert!(name_from_hex("6").is_err());
        assert!(name_from_hex("zz").is_err());
        assert!(name_from_hex("ff").is_err());
    }
}

//! Denoised copies of long recordings, written to the app data folder's
//! `recordings/` as the webview makes them.
//!
//! The webview sends the WAV in parts, as raw IPC bodies, because the copy is
//! too large to hold whole (and WebKitGTK has no OPFS to write it to). Parts
//! go to `<name>.<run>.part`, which [`denoise_copy_commit`] renames to
//! `<name>` and [`denoise_copy_discard`] deletes.
//!
//! Only names that end in " (denoised).wav" and have no folder in them are
//! accepted, so no command can touch another file.

use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use tauri::Manager;

/// The end of every copy's name: `DENOISED_SUFFIX` in the frontend's
/// `file_groups.rs`, with the extension.
const NAME_END: &str = " (denoised).wav";

fn copies_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let dir = crate::recovery::recordings_dir(&app_data);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
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
    Ok(copies_dir(app)?.join(format!("{}.{run}.part", checked_name(name)?)))
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
    let path = part_path(&app, &name, run)?;
    let mut f = if offset == 0 {
        std::fs::File::create(&path).map_err(|e| e.to_string())?
    } else {
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
    let len = std::fs::metadata(&part).map_err(|e| e.to_string())?.len();
    if len != size {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{name} has {len} bytes; expected {size}"));
    }
    let path = copies_dir(&app)?.join(&name);
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

/// Delete an unfinished copy. No part file is not an error.
#[tauri::command]
pub fn denoise_copy_discard(app: tauri::AppHandle, name: String, run: u32) -> Result<(), String> {
    match std::fs::remove_file(part_path(&app, &name, run)?) {
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

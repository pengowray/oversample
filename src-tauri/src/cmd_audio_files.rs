//! File-byte IPC commands. Audio *decoding* lives entirely in `oversample-core`
//! and runs in the WASM frontend (and in `native_playback` for the cpal path);
//! these commands only ferry raw bytes across the boundary.

/// Read raw file bytes — returns binary data via efficient IPC (no JSON serialization).
#[tauri::command]
pub fn read_file_bytes(path: String) -> Result<tauri::ipc::Response, String> {
    let bytes =
        std::fs::read(&path).map_err(|e| format!("Failed to read file '{}': {}", path, e))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Read a byte range from a file, for streaming large files without loading
/// them whole. Returns fewer bytes at the end of the file.
#[tauri::command]
pub fn read_file_range(
    path: String,
    offset: u64,
    length: u64,
) -> Result<tauri::ipc::Response, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f =
        std::fs::File::open(&path).map_err(|e| format!("Failed to open '{}': {}", path, e))?;
    f.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Seek failed: {}", e))?;
    let mut buf = Vec::with_capacity(length as usize);
    f.take(length)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Read failed: {}", e))?;
    Ok(tauri::ipc::Response::new(buf))
}

/// Largest range [`read_fd_range`] reads in one call.
const MAX_FD_READ: u64 = 64 * 1024 * 1024;

/// Read a byte range from a file descriptor that the media-store plugin opened
/// (`openRecordingForRead`), as raw bytes. Returns fewer bytes at the end of
/// the file. The fd stays open for later reads.
#[tauri::command]
pub fn read_fd_range(fd: i32, offset: u64, length: u64) -> Result<tauri::ipc::Response, String> {
    if length > MAX_FD_READ {
        return Err(format!(
            "read of {length} bytes is over the {MAX_FD_READ} byte limit"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        use std::os::unix::fs::FileExt;
        if fd < 0 {
            return Err(format!("bad fd {fd}"));
        }
        // ManuallyDrop: borrowing the fd, not closing it.
        let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
        let mut buf = vec![0u8; length as usize];
        let mut got = 0usize;
        while got < buf.len() {
            match file.read_at(&mut buf[got..], offset + got as u64) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("Read failed: {e}")),
            }
        }
        buf.truncate(got);
        Ok(tauri::ipc::Response::new(buf))
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, offset);
        Err("read_fd_range needs a Unix file descriptor".into())
    }
}

//! The file that the denoised copy of a long recording is written to, a piece
//! at a time, so the copy is never held in memory (`crate::denoise`).
//!
//! In the browser the file is in the Origin Private File System
//! (`crate::opfs_audio`). In the Tauri app it is in the app data folder's
//! `recordings/`, written by the `denoise_copy_*` commands
//! (`src-tauri/src/cmd_denoise.rs`).
//!
//! The file is a 32-bit float WAV. The copy has as many samples as the
//! recording, so the header is written first with its final sizes.

use crate::audio::wav::{header_bytes, WavWriteFormat, WRITE_HEADER_LEN};
use crate::opfs_audio;
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;
use std::cell::RefCell;

/// A copy being written.
pub struct CopyFile {
    name: String,
    /// Bytes the finished file has.
    size: u64,
    to: Target,
}

enum Target {
    Browser(opfs_audio::Writer),
    /// `run` tells this copy's part file from an earlier, cancelled one of
    /// the same name. `written` is the bytes sent so far.
    Tauri {
        run: u32,
        written: u64,
    },
}

/// A finished copy, for [`open`].
pub enum Written {
    Browser(web_sys::File, opfs_audio::StoredFile),
    Tauri { path: String, size: u64 },
}

/// Why a copy cannot be started.
pub enum CreateError {
    /// The browser's storage has `free` bytes and the copy needs `needed`.
    NoSpace {
        needed: u64,
        free: u64,
    },
    Other(String),
}

thread_local! {
    /// The copies in the browser's storage that are open in the file list,
    /// by the file's id, so that closing the file deletes its copy.
    static STORED: RefCell<Vec<(u64, opfs_audio::StoredFile)>> = const { RefCell::new(Vec::new()) };
}

/// True when this device can write a copy a piece at a time. Without it a
/// copy has to fit in memory.
pub fn supported(state: AppState) -> bool {
    state.is_tauri || opfs_audio::supported()
}

/// Bytes of the WAV for `frames` samples in each of `channels` channels.
pub fn file_size(frames: u64, channels: u32) -> u64 {
    WRITE_HEADER_LEN as u64 + frames * u64::from(channels) * 4
}

fn name_hex(name: &str) -> String {
    name.bytes().map(|b| format!("{b:02x}")).collect()
}

impl CopyFile {
    /// Starts the file `name` and writes its header.
    pub async fn create(
        state: AppState,
        name: &str,
        frames: u64,
        channels: u32,
        sample_rate: u32,
        run: u32,
    ) -> Result<CopyFile, CreateError> {
        let size = file_size(frames, channels);
        let to = if state.is_tauri {
            Target::Tauri { run, written: 0 }
        } else {
            if let Some(free) = opfs_audio::free_bytes().await {
                if free < size {
                    return Err(CreateError::NoSpace { needed: size, free });
                }
            }
            Target::Browser(
                opfs_audio::Writer::create(name)
                    .await
                    .map_err(CreateError::Other)?,
            )
        };
        let mut copy = CopyFile {
            name: name.to_string(),
            size,
            to,
        };
        let fmt = WavWriteFormat::for_f32_samples(sample_rate, channels as u16, 32, true);
        let header = header_bytes(&fmt, size - WRITE_HEADER_LEN as u64, 0);
        let header = js_sys::Uint8Array::from(header.as_slice());
        if let Err(e) = copy.write_bytes(&header).await {
            copy.discard().await;
            return Err(CreateError::Other(e));
        }
        Ok(copy)
    }

    /// Adds samples to the end of the file. The machine's own byte order is
    /// little-endian in every browser, which is the WAV's.
    pub async fn write(&mut self, samples: &js_sys::Float32Array) -> Result<(), String> {
        let bytes = js_sys::Uint8Array::new_with_byte_offset_and_length(
            &samples.buffer(),
            samples.byte_offset(),
            samples.byte_length(),
        );
        self.write_bytes(&bytes).await
    }

    async fn write_bytes(&mut self, bytes: &js_sys::Uint8Array) -> Result<(), String> {
        match &mut self.to {
            Target::Browser(writer) => writer.write(bytes).await,
            Target::Tauri { run, written } => {
                crate::tauri_bridge::tauri_invoke_raw(
                    "denoise_copy_write",
                    bytes,
                    &[
                        ("x-copy-name", name_hex(&self.name)),
                        ("x-copy-run", run.to_string()),
                        ("x-copy-offset", written.to_string()),
                    ],
                )
                .await?;
                *written += u64::from(bytes.byte_length());
                Ok(())
            }
        }
    }

    /// Closes the file. A file that has not got all its bytes is deleted.
    pub async fn finish(self) -> Result<Written, String> {
        match self.to {
            Target::Browser(writer) => {
                let (file, stored) = writer.finish().await?;
                if file.size() as u64 != self.size {
                    opfs_audio::delete(&stored).await;
                    return Err(format!(
                        "the file has {} bytes; expected {}",
                        file.size(),
                        self.size
                    ));
                }
                Ok(Written::Browser(file, stored))
            }
            Target::Tauri { run, .. } => {
                let path: String = crate::tauri_bridge::tauri_invoke_typed_args(
                    "denoise_copy_commit",
                    &CopyArgs {
                        name: &self.name,
                        run,
                        size: Some(self.size),
                    },
                )
                .await?;
                Ok(Written::Tauri {
                    path,
                    size: self.size,
                })
            }
        }
    }

    /// Stops writing and deletes what was written.
    pub async fn discard(self) {
        match self.to {
            Target::Browser(writer) => writer.discard().await,
            Target::Tauri { run, .. } => {
                let _ = crate::tauri_bridge::tauri_invoke_args(
                    "denoise_copy_discard",
                    &CopyArgs {
                        name: &self.name,
                        run,
                        size: None,
                    },
                )
                .await;
            }
        }
    }
}

/// Arguments of `denoise_copy_commit` and `denoise_copy_discard`.
#[derive(serde::Serialize)]
struct CopyArgs<'a> {
    name: &'a str,
    run: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

/// Adds a finished copy to the file list, the way the app opens any file of
/// its size: read from the file a part at a time when it is large, decoded
/// into memory otherwise.
pub async fn open(state: AppState, written: Written, name: &str) -> Result<(), String> {
    let load_id = state.loading_start(name);
    let opened = match written {
        Written::Browser(file, stored) => {
            let opened =
                crate::components::file_sidebar::read_and_load_file(file, state, load_id).await;
            // A copy that is read from its file keeps it until it is closed.
            // One that was decoded into memory needs its file no longer, and
            // the file is deleted: a copy can be hundreds of megabytes, and
            // the browser's storage is limited.
            let copy = state.library.files().with_untracked(|files| {
                files
                    .last()
                    .filter(|f| f.name == name)
                    .map(|f| (f.id, f.file_handle.is_some()))
            });
            match (&opened, copy) {
                (Ok(()), Some((id, true))) => STORED.with(|s| s.borrow_mut().push((id, stored))),
                (Ok(()), Some((id, false))) => {
                    // Before the file goes, compute the hashes that need it:
                    // a large copy has only its spot hash so far, and no
                    // other hash could be computed once the file is deleted.
                    // (A small copy has them all, and this does nothing.)
                    wasm_bindgen_futures::spawn_local(async move {
                        crate::file_identity::compute_hashes_now(
                            state,
                            id,
                            vec![
                                crate::file_identity::HashKind::Blake3,
                                crate::file_identity::HashKind::Content,
                                crate::file_identity::HashKind::Sha256,
                            ],
                        )
                        .await;
                        state.library.files().update(|files| {
                            if let Some(f) = files.iter_mut().find(|f| f.id == id) {
                                f.hash_source = None;
                            }
                        });
                        opfs_audio::delete(&stored).await;
                    });
                }
                _ => opfs_audio::delete(&stored).await,
            }
            opened
        }
        Written::Tauri { path, size } => {
            crate::components::file_sidebar::load_native_wav(path, size, state, load_id).await
        }
    };
    state.loading_done(load_id);
    opened
}

/// Call when the file with this id leaves the file list: deletes its copy
/// from the browser's storage, if it has one there.
pub fn file_closed(file_id: u64) {
    let stored = STORED.with(|s| {
        let mut s = s.borrow_mut();
        let at = s.iter().position(|(id, _)| *id == file_id)?;
        Some(s.remove(at).1)
    });
    if let Some(stored) = stored {
        wasm_bindgen_futures::spawn_local(async move { opfs_audio::delete(&stored).await });
    }
}

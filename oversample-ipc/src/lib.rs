//! Shared IPC wire types for the Oversample/batchi app.
//!
//! These types form the contract between the WASM frontend and the Tauri native
//! backend. Both sides depend on this crate so the *same* struct/enum definition
//! is used on each end: the frontend serialises command arguments with
//! `serde_wasm_bindgen::to_value` and deserialises results with
//! `serde_wasm_bindgen::from_value`, giving a compile-time-checked boundary
//! instead of hand-mirrored structs parsed field-by-field with `Reflect::get`.
//!
//! The crate is intentionally dependency-light (serde only) so it can compile
//! into the WASM frontend — unlike `xc-lib` (pulls `reqwest`) or `src-tauri`
//! (pulls `tauri`/`cpal`). That dependency weight is exactly why types like
//! [`SidecarHashes`] used to be hand-mirrored; this crate is their canonical home.

use serde::{Deserialize, Serialize};

pub mod mic;
pub mod plugins;
pub mod xc;

/// Hash data extracted from an XC sidecar JSON file (stored under the `_app`
/// key, with a fallback to legacy top-level keys).
///
/// Canonical definition: previously hand-mirrored between `xc_lib::cache` and
/// the frontend `state` module (which couldn't depend on `xc-lib` because it
/// pulls `reqwest`). Both now re-export this type.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SidecarHashes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    /// Multi-point spot hash (16×1MB chunks, matches the main app's Layer 2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spot_hash_b3: Option<String>,
    /// Content hash (BLAKE3 with the header bytes zeroed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Audio data region byte offset within the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_offset: Option<u64>,
    /// Audio data region byte length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_size: Option<u64>,
}

impl SidecarHashes {
    /// True when no usable hash is present.
    pub fn is_empty(&self) -> bool {
        self.blake3.is_none()
            && self.sha256.is_none()
            && self.file_size.is_none()
            && self.spot_hash_b3.is_none()
    }
}

use sha2::{Sha256, Digest};
use leptos::prelude::{Update, WithUntracked};
use crate::annotations::FileIdentity;
use crate::state::AppState;

const SPOT_CHUNK_SIZE: u64 = 4096;

/// Create a Layer 1 identity (filename + size). Instant.
pub fn identity_layer1(filename: &str, file_size: u64) -> FileIdentity {
    FileIdentity {
        filename: filename.to_string(),
        file_size,
        spot_hash: None,
        audio_hash: None,
        full_sha256: None,
        file_path: None,
    }
}

/// Compute Layer 2 spot-check hash from raw file bytes.
/// Hashes first 4KB + middle 4KB + last 4KB.
pub fn compute_spot_hash(file_bytes: &[u8]) -> String {
    let len = file_bytes.len() as u64;
    let mut hasher = Sha256::new();

    // First chunk
    let first_end = SPOT_CHUNK_SIZE.min(len) as usize;
    hasher.update(&file_bytes[..first_end]);

    // Middle chunk
    if len > SPOT_CHUNK_SIZE * 2 {
        let mid_start = ((len / 2) - (SPOT_CHUNK_SIZE / 2)) as usize;
        let mid_end = (mid_start + SPOT_CHUNK_SIZE as usize).min(file_bytes.len());
        hasher.update(&file_bytes[mid_start..mid_end]);
    }

    // Last chunk
    if len > SPOT_CHUNK_SIZE {
        let last_start = (len - SPOT_CHUNK_SIZE) as usize;
        hasher.update(&file_bytes[last_start..]);
    }

    format!("{:x}", hasher.finalize())
}

/// Compute Layer 2 spot-check hash from a blob reader function (for large files).
/// Takes three 4KB ranges via async reads.
pub async fn compute_spot_hash_from_ranges(
    read_range: impl AsyncRangeReader,
    file_size: u64,
) -> Result<String, String> {
    let mut hasher = Sha256::new();

    // First chunk
    let first_end = SPOT_CHUNK_SIZE.min(file_size);
    let first = read_range.read(0, first_end).await?;
    hasher.update(&first);

    // Middle chunk
    if file_size > SPOT_CHUNK_SIZE * 2 {
        let mid_start = (file_size / 2) - (SPOT_CHUNK_SIZE / 2);
        let mid_len = SPOT_CHUNK_SIZE.min(file_size - mid_start);
        let mid = read_range.read(mid_start, mid_len).await?;
        hasher.update(&mid);
    }

    // Last chunk
    if file_size > SPOT_CHUNK_SIZE {
        let last_start = file_size - SPOT_CHUNK_SIZE;
        let last = read_range.read(last_start, SPOT_CHUNK_SIZE).await?;
        hasher.update(&last);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Trait for async range reading (implemented for web File blobs and Tauri paths).
pub trait AsyncRangeReader {
    fn read(&self, offset: u64, length: u64) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>>;
}

/// Web File blob range reader.
pub struct BlobRangeReader {
    pub file: web_sys::File,
}

impl AsyncRangeReader for BlobRangeReader {
    fn read(&self, offset: u64, length: u64) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>> {
        let start = offset as f64;
        let end = (offset + length) as f64;
        Box::pin(async move {
            crate::audio::streaming_source::read_blob_range(&self.file, start, end).await
        })
    }
}

/// Tauri file path range reader.
pub struct TauriRangeReader {
    pub path: String,
}

impl AsyncRangeReader for TauriRangeReader {
    fn read(&self, offset: u64, length: u64) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>> {
        Box::pin(async move {
            crate::tauri_bridge::read_file_range(&self.path, offset, length).await
        })
    }
}

/// Compute Layer 1 identity and kick off async Layer 2 (spot-check) computation.
/// Call this after a file is added to state.files.
pub fn start_identity_computation(
    state: AppState,
    file_index: usize,
    filename: String,
    file_size: u64,
    file_bytes: Option<Vec<u8>>,
) {
    // Skip if identity already computed (e.g. by load_named_bytes)
    let already_has_identity = state.files.with_untracked(|files| {
        files.get(file_index).map_or(false, |f| f.identity.is_some())
    });
    if already_has_identity {
        return;
    }

    // Layer 1: set immediately
    let identity = identity_layer1(&filename, file_size);
    state.files.update(|files| {
        if let Some(f) = files.get_mut(file_index) {
            f.identity = Some(identity);
        }
    });

    // Layer 2: compute spot hash async
    wasm_bindgen_futures::spawn_local(async move {
        let spot_hash = if let Some(bytes) = file_bytes {
            // We have the full bytes in memory — compute directly
            Some(compute_spot_hash(&bytes))
        } else {
            // No bytes available (streaming load), skip for now
            None
        };

        if let Some(hash) = spot_hash {
            state.files.update(|files| {
                if let Some(f) = files.get_mut(file_index) {
                    if let Some(ref mut id) = f.identity {
                        id.spot_hash = Some(hash);
                    }
                }
            });
        }
    });
}

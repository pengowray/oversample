// SPDX-License-Identifier: GPL-3.0-only OR MIT OR Apache-2.0
use crate::annotations::FileIdentity;
use crate::state::store_fields::*;
use crate::state::AppState;
use crate::state::SidecarHashes;
use crate::web_util::yield_now;
use leptos::prelude::{ArcRwSignal, Set, Update, WithUntracked};
use sha2::{Digest, Sha256};

/// Size of each chunk for the multi-point spot hash (1 MB).
const SPOT_CHUNK_SIZE: u64 = 1_048_576;
/// Maximum number of chunks for the spot hash.
const NUM_SPOT_CHUNKS: u64 = 16;

/// Create a Layer 1 identity (filename + size). Instant.
pub fn identity_layer1(filename: &str, file_size: u64) -> FileIdentity {
    FileIdentity {
        filename: filename.to_string(),
        file_size,
        spot_hash_b3: None,
        content_hash: None,
        full_blake3: None,
        full_sha256: None,
        legacy_spot_hash: None,
        data_offset: None,
        data_size: None,
        last_modified: None,
        file_path: None,
    }
}

/// Determine audio region boundaries for hashing.
/// Returns (audio_start, audio_len) in bytes.
fn audio_region(file_size: u64, data_offset: Option<u64>, data_size: Option<u64>) -> (u64, u64) {
    let audio_start = data_offset.unwrap_or(0);
    let audio_len = data_size.unwrap_or(file_size.saturating_sub(audio_start));
    (audio_start, audio_len)
}

/// Compute the chunk positions for the multi-point spot hash.
/// Returns Vec of (chunk_start, chunk_len) pairs.
fn spot_chunk_positions(audio_start: u64, audio_len: u64) -> Vec<(u64, u64)> {
    if audio_len == 0 {
        return Vec::new();
    }
    let num_chunks = NUM_SPOT_CHUNKS.min((audio_len / SPOT_CHUNK_SIZE).max(1));
    (0..num_chunks)
        .map(|i| {
            let chunk_start = audio_start + i * (audio_len / num_chunks);
            let remaining = audio_len - (chunk_start - audio_start);
            let chunk_len = SPOT_CHUNK_SIZE.min(remaining);
            (chunk_start, chunk_len)
        })
        .collect()
}

/// Combine individual chunk hashes into the final spot hash.
fn finalize_spot_hash(chunk_hashes: &[blake3::Hash]) -> String {
    let mut combined = Vec::with_capacity(chunk_hashes.len() * 32);
    for h in chunk_hashes {
        combined.extend_from_slice(h.as_bytes());
    }
    blake3::hash(&combined).to_hex().to_string()
}

/// Compute Layer 2 BLAKE3 multi-point spot hash from in-memory file bytes (sync).
pub fn compute_spot_hash_b3_sync(
    file_bytes: &[u8],
    data_offset: Option<u64>,
    data_size: Option<u64>,
) -> String {
    let (audio_start, audio_len) = audio_region(file_bytes.len() as u64, data_offset, data_size);
    let positions = spot_chunk_positions(audio_start, audio_len);

    let chunk_hashes: Vec<blake3::Hash> = positions
        .iter()
        .map(|&(start, len)| {
            let s = start as usize;
            let e = (start + len).min(file_bytes.len() as u64) as usize;
            blake3::hash(&file_bytes[s..e])
        })
        .collect();

    finalize_spot_hash(&chunk_hashes)
}

/// Compute Layer 2 BLAKE3 multi-point spot hash via async range reader.
pub async fn compute_spot_hash_b3(
    reader: &(impl AsyncRangeReader + ?Sized),
    file_size: u64,
    data_offset: Option<u64>,
    data_size: Option<u64>,
) -> Result<String, String> {
    let (audio_start, audio_len) = audio_region(file_size, data_offset, data_size);
    let positions = spot_chunk_positions(audio_start, audio_len);

    let mut chunk_hashes = Vec::with_capacity(positions.len());
    for (i, &(start, len)) in positions.iter().enumerate() {
        let bytes = reader.read(start, len).await?;
        chunk_hashes.push(blake3::hash(&bytes));

        // Yield to browser every 4 chunks
        if (i + 1) % 4 == 0 {
            yield_now().await;
        }
    }

    Ok(finalize_spot_hash(&chunk_hashes))
}

/// The hashes of one file. A hash that was not computed is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComputedHashes {
    pub spot: Option<String>,
    pub blake3: Option<String>,
    pub sha256: Option<String>,
    pub content: Option<String>,
}

/// Compute the hashes in `kinds` that need the whole file: the full BLAKE3,
/// the full SHA-256 and the content hash. The file is read once, in 1 MB
/// pieces, whichever of the three are asked for.
///
/// The content hash covers only `file[data_offset .. data_offset+data_size]`,
/// skipping both the header and any trailing metadata (e.g. GUANO). This means
/// metadata edits don't change the content hash.
///
/// `progress` is called with the number of bytes read so far. The read stops
/// with `Err("Cancelled")` when `cancelled` returns true.
pub async fn compute_whole_file_hashes(
    reader: &(impl AsyncRangeReader + ?Sized),
    file_size: u64,
    data_offset: Option<u64>,
    data_size: Option<u64>,
    kinds: &[HashKind],
    progress: impl Fn(u64),
    cancelled: impl Fn() -> bool,
) -> Result<ComputedHashes, String> {
    let audio_start = data_offset.unwrap_or(0);
    let audio_end = match data_size {
        Some(sz) => (audio_start + sz).min(file_size),
        None => file_size,
    };
    let audio_start = audio_start.min(audio_end);

    let mut content_hasher = kinds.contains(&HashKind::Content).then(blake3::Hasher::new);
    let mut full_hasher = kinds.contains(&HashKind::Blake3).then(blake3::Hasher::new);
    let mut sha_hasher = kinds.contains(&HashKind::Sha256).then(Sha256::new);

    let chunk_size: u64 = SPOT_CHUNK_SIZE; // 1 MB
    let mut offset: u64 = 0;

    while offset < file_size {
        if cancelled() {
            return Err("Cancelled".into());
        }

        let len = chunk_size.min(file_size - offset);
        let bytes = reader.read(offset, len).await?;
        if bytes.len() as u64 != len {
            return Err(format!(
                "Read {} of {len} bytes at offset {offset}",
                bytes.len()
            ));
        }

        if let Some(h) = full_hasher.as_mut() {
            h.update(&bytes);
        }
        if let Some(h) = sha_hasher.as_mut() {
            h.update(&bytes);
        }
        // Content hash: only the part of this piece inside [audio_start, audio_end)
        if let Some(h) = content_hasher.as_mut() {
            let slice_start = audio_start.max(offset);
            let slice_end = audio_end.min(offset + len);
            if slice_start < slice_end {
                let lo = (slice_start - offset) as usize;
                let hi = (slice_end - offset) as usize;
                h.update(&bytes[lo..hi]);
            }
        }

        offset += len;
        progress(offset);

        // Yield every 4 MB
        if (offset / chunk_size).is_multiple_of(4) {
            yield_now().await;
        }
    }

    Ok(ComputedHashes {
        spot: None,
        blake3: full_hasher.map(|h| h.finalize().to_hex().to_string()),
        sha256: sha_hasher.map(|h| h.finalize().iter().map(|b| format!("{b:02x}")).collect()),
        content: content_hasher.map(|h| h.finalize().to_hex().to_string()),
    })
}

/// Trait for async range reading (implemented for web File blobs and Tauri paths).
pub trait AsyncRangeReader {
    fn read(
        &self,
        offset: u64,
        length: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>>;
}

/// Web File blob range reader.
pub struct BlobRangeReader {
    pub file: web_sys::File,
}

impl AsyncRangeReader for BlobRangeReader {
    fn read(
        &self,
        offset: u64,
        length: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>> {
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
    fn read(
        &self,
        offset: u64,
        length: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>> {
        Box::pin(
            async move { crate::tauri_bridge::read_file_range(&self.path, offset, length).await },
        )
    }
}

/// Android MediaStore `content://` range reader (reads back via the media-store
/// plugin). Used to hash a recording that lives only in public shared storage.
pub struct MediaStoreRangeReader {
    pub uri: String,
}

impl AsyncRangeReader for MediaStoreRangeReader {
    fn read(
        &self,
        offset: u64,
        length: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>> {
        Box::pin(async move {
            crate::audio::streaming_source::read_media_store_range(&self.uri, offset, length).await
        })
    }
}

/// Construct an AsyncRangeReader from a FileHandle.
pub fn reader_from_handle(
    handle: &crate::audio::streaming_source::FileHandle,
) -> Box<dyn AsyncRangeReader + '_> {
    match handle {
        crate::audio::streaming_source::FileHandle::WebFile(file) => {
            Box::new(BlobRangeReader { file: file.clone() })
        }
        crate::audio::streaming_source::FileHandle::TauriPath(path) => {
            Box::new(TauriRangeReader { path: path.clone() })
        }
        crate::audio::streaming_source::FileHandle::MediaStoreUri(uri) => {
            Box::new(MediaStoreRangeReader { uri: uri.clone() })
        }
        crate::audio::streaming_source::FileHandle::Bytes(b) => {
            Box::new(BytesRangeReader { bytes: b.clone() })
        }
    }
}

/// In-memory range reader (host tests; mirrors `FileHandle::Bytes`).
pub struct BytesRangeReader {
    pub bytes: std::sync::Arc<Vec<u8>>,
}

impl AsyncRangeReader for BytesRangeReader {
    fn read(
        &self,
        offset: u64,
        length: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>> {
        Box::pin(async move {
            let len = self.bytes.len() as u64;
            let s = offset.min(len) as usize;
            let e = (offset + length).min(len) as usize;
            Ok(self.bytes[s..e].to_vec())
        })
    }
}

/// The size, in bytes, that divides small files from large ones. It decides
/// two things, and they use this one number:
///
/// - Which hash the Info panel shows as the file's main hash: the full BLAKE3
///   below this size, and the spot hash at this size and above.
/// - Whether every hash is computed without being asked for. Below this size
///   they all are, so a saved hash is compared with the file as soon as it is
///   shown. At this size and above only the spot hash is, and the others are
///   computed when the user asks.
///
/// The spot hash reads up to 16 x 1 MB = 16 MB, so below about 24 MB the full
/// BLAKE3 costs about the same and covers every byte.
pub const SMALL_FILE_THRESHOLD: u64 = 24_000_000;

/// The kinds of hash kept for a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashKind {
    /// BLAKE3 of up to 16 pieces of 1 MB spread over the audio data.
    Spot,
    /// BLAKE3 of the whole file.
    Blake3,
    /// SHA-256 of the whole file.
    Sha256,
    /// BLAKE3 of the audio data only, without the header and trailing metadata.
    Content,
}

impl HashKind {
    pub const ALL: [HashKind; 4] = [
        HashKind::Spot,
        HashKind::Blake3,
        HashKind::Sha256,
        HashKind::Content,
    ];

    /// The name the Info panel shows.
    pub fn label(self) -> &'static str {
        match self {
            HashKind::Spot => "Spot hash",
            HashKind::Blake3 => "Full BLAKE3",
            HashKind::Sha256 => "Full SHA-256",
            HashKind::Content => "Content hash",
        }
    }

    /// A short name for the `data-hash` attribute of the Info panel's row.
    pub fn key(self) -> &'static str {
        match self {
            HashKind::Spot => "spot",
            HashKind::Blake3 => "blake3",
            HashKind::Sha256 => "sha256",
            HashKind::Content => "content",
        }
    }

    /// This kind of hash in a file's identity.
    pub fn in_identity(self, id: &FileIdentity) -> Option<&String> {
        match self {
            HashKind::Spot => id.spot_hash_b3.as_ref(),
            HashKind::Blake3 => id.full_blake3.as_ref(),
            HashKind::Sha256 => id.full_sha256.as_ref(),
            HashKind::Content => id.content_hash.as_ref(),
        }
    }

    fn in_identity_mut(self, id: &mut FileIdentity) -> &mut Option<String> {
        match self {
            HashKind::Spot => &mut id.spot_hash_b3,
            HashKind::Blake3 => &mut id.full_blake3,
            HashKind::Sha256 => &mut id.full_sha256,
            HashKind::Content => &mut id.content_hash,
        }
    }

    fn in_saved(self, saved: &SidecarHashes) -> Option<&String> {
        match self {
            HashKind::Spot => saved.spot_hash_b3.as_ref(),
            HashKind::Blake3 => saved.blake3.as_ref(),
            HashKind::Sha256 => saved.sha256.as_ref(),
            HashKind::Content => saved.content_hash.as_ref(),
        }
    }

    fn in_computed(self, hashes: &ComputedHashes) -> Option<&String> {
        match self {
            HashKind::Spot => hashes.spot.as_ref(),
            HashKind::Blake3 => hashes.blake3.as_ref(),
            HashKind::Sha256 => hashes.sha256.as_ref(),
            HashKind::Content => hashes.content.as_ref(),
        }
    }
}

/// Where a saved hash was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedSource {
    /// The metadata file that came with a Xeno-canto recording
    /// (`LoadedFile::xc_hashes`).
    XenoCanto,
    /// The annotations saved for the file in an earlier session
    /// (`LoadedFile::saved_hashes`).
    Annotations,
}

/// How a hash computed from the file compares with the saved hash of the same
/// kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HashState {
    /// Computed from the file. There is no saved hash of this kind.
    Computed,
    /// Read from saved data. Not computed from the file, so not compared.
    Saved,
    /// Computed from the file, and equal to the saved hash.
    Match,
    /// Computed from the file, and different from the saved hash.
    Mismatch,
}

/// One kind of hash of a file: the value computed from the file in this
/// session, and the value read from saved data. Either can be missing.
#[derive(Clone, Debug, PartialEq)]
pub struct HashRow {
    pub kind: HashKind,
    pub computed: Option<String>,
    pub saved: Option<(String, SavedSource)>,
}

impl HashRow {
    /// `None` when the file has no hash of this kind, computed or saved.
    pub fn state(&self) -> Option<HashState> {
        match (&self.computed, &self.saved) {
            (None, None) => None,
            (Some(_), None) => Some(HashState::Computed),
            (None, Some(_)) => Some(HashState::Saved),
            (Some(c), Some((s, _))) if c.eq_ignore_ascii_case(s) => Some(HashState::Match),
            (Some(_), Some(_)) => Some(HashState::Mismatch),
        }
    }
}

/// The four kinds of hash of a file, in `HashKind::ALL` order. A hash from
/// the Xeno-canto metadata file is used before one from the saved
/// annotations: it describes the recording as it was published.
pub fn hash_rows(
    identity: Option<&FileIdentity>,
    xc_hashes: Option<&SidecarHashes>,
    saved_hashes: Option<&SidecarHashes>,
) -> Vec<HashRow> {
    HashKind::ALL
        .into_iter()
        .map(|kind| {
            let from = |saved: Option<&SidecarHashes>, source| {
                saved
                    .and_then(|s| kind.in_saved(s))
                    .map(|h| (h.clone(), source))
            };
            HashRow {
                kind,
                computed: identity.and_then(|id| kind.in_identity(id)).cloned(),
                saved: from(xc_hashes, SavedSource::XenoCanto)
                    .or_else(|| from(saved_hashes, SavedSource::Annotations)),
            }
        })
        .collect()
}

/// The four kinds of hash of an open file.
pub fn file_hash_rows(f: &crate::state::LoadedFile) -> Vec<HashRow> {
    hash_rows(
        f.identity.as_ref(),
        f.xc_hashes.as_ref(),
        f.saved_hashes.as_ref(),
    )
}

/// The hash the Info panel shows for a file of `file_size` bytes; the other
/// kinds go in its list of other hashes.
///
/// It is the spot hash for a large file and the full BLAKE3 for a small one
/// (`SMALL_FILE_THRESHOLD`). When the file has no hash of that kind and
/// `preferred_is_coming` is false (the hash is not being computed and cannot
/// be), it is the first kind the file does have, of full SHA-256, content
/// hash, full BLAKE3 and spot hash.
pub fn main_hash(file_size: u64, rows: &[HashRow], preferred_is_coming: bool) -> HashKind {
    let preferred = if file_size < SMALL_FILE_THRESHOLD {
        HashKind::Blake3
    } else {
        HashKind::Spot
    };
    if preferred_is_coming {
        return preferred;
    }
    [
        preferred,
        HashKind::Sha256,
        HashKind::Content,
        HashKind::Blake3,
        HashKind::Spot,
    ]
    .into_iter()
    .find(|kind| rows.iter().any(|r| r.kind == *kind && r.state().is_some()))
    .unwrap_or(preferred)
}

/// Whether the file's audio data is the saved one while the rest of the file
/// is not: the content hash is equal to the saved content hash, and a hash
/// of the file (spot, full BLAKE3 or full SHA-256) differs from the saved one.
/// An edit to the file's header or to metadata stored in it does this.
pub fn only_audio_matches(rows: &[HashRow]) -> bool {
    let state = |kind| {
        rows.iter()
            .find(|r| r.kind == kind)
            .and_then(HashRow::state)
    };
    state(HashKind::Content) == Some(HashState::Match)
        && [HashKind::Spot, HashKind::Blake3, HashKind::Sha256]
            .into_iter()
            .any(|kind| state(kind) == Some(HashState::Mismatch))
}

/// The hashes and size in an identity read from saved annotations.
fn saved_hashes_of(id: &FileIdentity) -> SidecarHashes {
    SidecarHashes {
        blake3: id.full_blake3.clone(),
        sha256: id.full_sha256.clone(),
        file_size: Some(id.file_size),
        spot_hash_b3: id.spot_hash_b3.clone(),
        content_hash: id.content_hash.clone(),
        data_offset: id.data_offset,
        data_size: id.data_size,
    }
}

/// Keep the hashes of the annotation set read from disk for file `file_id`,
/// as `LoadedFile::saved_hashes`. Called when the set is put in the store.
///
/// The store's own copy of the identity does not serve: every save replaces
/// it with the identity computed from the file, and a hash compared with that
/// copy is compared with itself.
pub fn remember_saved_hashes(state: AppState, file_id: u64, saved: &FileIdentity) {
    let saved = saved_hashes_of(saved);
    state.library.files().update(|files| {
        // The first set read is the one from disk. A later one can be the
        // Google Drive copy of what this session saved, which holds the
        // computed hashes.
        if let Some(f) = files.iter_mut().find(|f| f.id == file_id) {
            if f.saved_hashes.is_none() {
                f.saved_hashes = Some(saved);
            }
        }
    });
}

/// The identity to save with a file's annotations: the one computed from the
/// file, with each kind of hash that has not been computed taken from the
/// annotations read from disk. Without this a save would drop a full hash
/// that an earlier session computed for a large file.
pub fn identity_to_save(f: &crate::state::LoadedFile) -> Option<FileIdentity> {
    let mut id = f.identity.clone()?;
    if let Some(saved) = &f.saved_hashes {
        for kind in HashKind::ALL {
            let slot = kind.in_identity_mut(&mut id);
            if slot.is_none() {
                *slot = kind.in_saved(saved).cloned();
            }
        }
    }
    Some(id)
}

/// Whether the identity stored in a saved annotation set can be that of
/// `file`, going by the hashes both hold.
///
/// The kinds of hash are compared in turn: content hash, full BLAKE3, spot
/// hash, and the old SHA-256 spot hash. One equal pair is enough: an edit to
/// a file's header changes its file hashes but not its content hash. The
/// answer is false only when both sides hold at least one kind in common and
/// no common kind is equal.
///
/// With no kind in common there is nothing to compare, and the answer is
/// true. That is the case for a set saved before the file's hash was known
/// (or by a version that stored none): it goes to a file of the same name
/// and size, as it always did.
pub fn saved_identity_agrees(saved: &FileIdentity, file: &FileIdentity) -> bool {
    let pairs = [
        (&saved.content_hash, &file.content_hash),
        (&saved.full_blake3, &file.full_blake3),
        (&saved.spot_hash_b3, &file.spot_hash_b3),
        (&saved.legacy_spot_hash, &file.legacy_spot_hash),
    ];
    let mut compared = false;
    for (a, b) in pairs {
        if let (Some(a), Some(b)) = (a, b) {
            if a == b {
                return true;
            }
            compared = true;
        }
    }
    !compared
}

thread_local! {
    /// Files (by `LoadedFile.id`) whose spot hash is being computed.
    static SPOT_HASH_PENDING: std::cell::RefCell<std::collections::HashSet<u64>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// Whether the spot hash of file `file_id` is still being computed. Its
/// saved annotations are looked up when the hash is known
/// (`start_identity_computation`), so a lookup asked for before then is
/// left to that one: without the hash, a set stored under the file's name
/// and size cannot be told from the set of another file with that name and
/// size.
pub fn spot_hash_pending(file_id: u64) -> bool {
    SPOT_HASH_PENDING.with(|s| s.borrow().contains(&file_id))
}

/// Compute Layer 1 identity and kick off async Layer 2 (spot-check) computation.
/// Call this after a file is added to state.library.files().
pub fn start_identity_computation(
    state: AppState,
    file_index: usize,
    filename: String,
    file_size: u64,
    file_bytes: Option<Vec<u8>>,
    data_offset: Option<u64>,
    data_size: Option<u64>,
    last_modified_ms: Option<f64>,
) {
    // The async work below outlives changes to the file list, so it finds
    // the file again by its id after every await. Closing another file moves
    // this one to a new position, and writing by the old position would put
    // this file's hashes (the key its annotations are saved under) on a
    // different file.
    let Some(file_id) = state.file_id_at(file_index) else {
        return;
    };

    // The file's annotations are not saved until its saved annotations have
    // been looked up under its hash, below (see `opfs::SaveHold`).
    let save_hold = crate::opfs::hold_saves(state, file_id);

    // Skip if identity already computed (e.g. by load_named_bytes)
    let already_has_identity = state
        .library
        .files()
        .with_untracked(|files| files.get(file_index).is_some_and(|f| f.identity.is_some()));
    if already_has_identity {
        // Still try loading saved annotations even if identity was already set.
        // (Does nothing while the first call's spot hash is being computed:
        // that call reads them when it has the hash.)
        let identity = state
            .library
            .files()
            .with_untracked(|files| files.get(file_index).and_then(|f| f.identity.clone()));
        if let Some(id) = identity {
            crate::opfs::load_annotations(state, file_index, id);
        }
        return;
    }

    // Layer 1: set immediately
    let mut identity = identity_layer1(&filename, file_size);
    identity.data_offset = data_offset;
    identity.data_size = data_size;
    identity.last_modified = last_modified_ms.map(|ms| ms.to_string());

    state.library.files().update(|files| {
        if let Some(f) = files.get_mut(file_index) {
            f.identity = Some(identity.clone());
        }
    });

    // The saved annotations are read once, after the spot hash below: they
    // are stored under the hash. (With no hash, under the name and size.)

    // Layer 2: compute BLAKE3 spot hash async (+ Layers 3+4 when bytes available)
    SPOT_HASH_PENDING.with(|s| s.borrow_mut().insert(file_id));
    wasm_bindgen_futures::spawn_local(async move {
        let save_hold = save_hold;
        let spot_hash = if let Some(ref bytes) = file_bytes {
            // In-memory: compute synchronously
            Some(compute_spot_hash_b3_sync(bytes, data_offset, data_size))
        } else {
            // No bytes available (streaming load) — need file handle for range reads
            let handle = state
                .library
                .files()
                .with_untracked(|files| files.get(file_index).and_then(|f| f.file_handle.clone()));
            if let Some(handle) = handle {
                let reader = reader_from_handle(&handle);
                compute_spot_hash_b3(reader.as_ref(), file_size, data_offset, data_size)
                    .await
                    .ok()
            } else {
                None
            }
        };

        // The hash is known, or cannot be computed: lookups go ahead.
        SPOT_HASH_PENDING.with(|s| s.borrow_mut().remove(&file_id));

        let Some(file_index) = state.file_idx_for_id(file_id) else {
            return;
        };
        if let Some(hash) = spot_hash {
            state.library.files().update(|files| {
                if let Some(f) = files.get_mut(file_index) {
                    if let Some(ref mut id) = f.identity {
                        id.spot_hash_b3 = Some(hash);
                    }
                }
            });
        }

        // Read the saved annotations: under the spot hash, or under the name
        // and size if the hash could not be computed. The read holds the
        // file's saves itself until it is done.
        let identity = state
            .library
            .files()
            .with_untracked(|files| files.get(file_index).and_then(|f| f.identity.clone()));
        if let Some(id) = identity {
            crate::opfs::load_annotations(state, file_index, id);
        }
        drop(save_hold);

        let is_small = file_size < SMALL_FILE_THRESHOLD;
        let bytes = file_bytes.map(std::sync::Arc::new);

        // A small file gets every hash now. A large file whose bytes are in
        // memory gets the kinds it has a saved hash of: the bytes are not
        // kept, and this is the one time those hashes can be compared.
        let kinds: Vec<HashKind> = if is_small {
            vec![HashKind::Blake3, HashKind::Content, HashKind::Sha256]
        } else if bytes.is_some() {
            saved_kinds_not_computed(state, file_id)
        } else {
            Vec::new()
        };
        run_hash_job(state, file_id, kinds, bytes).await;

        // A large file whose spot hash differs from the saved one: compute
        // the content hash, which tells whether the audio data is still the
        // saved one and only the header or metadata changed.
        let needs_content = state.library.files().with_untracked(|files| {
            files.iter().find(|f| f.id == file_id).is_some_and(|f| {
                let rows = file_hash_rows(f);
                let state_of = |kind| {
                    rows.iter()
                        .find(|r| r.kind == kind)
                        .and_then(HashRow::state)
                };
                state_of(HashKind::Spot) == Some(HashState::Mismatch)
                    && state_of(HashKind::Content) == Some(HashState::Saved)
            })
        });
        if needs_content {
            run_hash_job(state, file_id, vec![HashKind::Content], None).await;
        }
    });
}

/// A hash computation in progress for one file.
#[derive(Clone, Debug, PartialEq)]
pub struct HashJob {
    id: u64,
    pub file_id: u64,
    pub kinds: Vec<HashKind>,
    /// Bytes of the file read so far, and the file's size.
    pub read: u64,
    pub total: u64,
}

impl HashJob {
    /// How much of the file has been read, from 0 to 100.
    pub fn percent(&self) -> u32 {
        if self.total == 0 {
            return 100;
        }
        ((self.read as f64 / self.total as f64) * 100.0).floor() as u32
    }
}

/// Hash computations in progress, and the ones that failed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HashJobs {
    pub running: Vec<HashJob>,
    /// File id, kind of hash, and the error.
    pub failed: Vec<(u64, HashKind, String)>,
}

impl HashJobs {
    /// The computation of `kind` in progress for file `file_id`.
    pub fn running(&self, file_id: u64, kind: HashKind) -> Option<&HashJob> {
        self.running
            .iter()
            .find(|j| j.file_id == file_id && j.kinds.contains(&kind))
    }

    /// The error from the last computation of `kind` for file `file_id`, if
    /// it failed.
    pub fn failure(&self, file_id: u64, kind: HashKind) -> Option<&str> {
        self.failed
            .iter()
            .find(|(id, k, _)| *id == file_id && *k == kind)
            .map(|(_, _, e)| e.as_str())
    }
}

thread_local! {
    // An `Arc` signal: it is made on first use, under whatever reactive owner
    // is current then, and has to outlive that owner.
    static HASH_JOBS: ArcRwSignal<HashJobs> = ArcRwSignal::new(HashJobs::default());
    static NEXT_HASH_JOB: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Hash computations in progress and failed, for the Info panel's hash rows.
/// It changes as a file is read, so the rows' status lines read it and the
/// file list does not.
pub fn hash_jobs() -> ArcRwSignal<HashJobs> {
    HASH_JOBS.with(Clone::clone)
}

/// The handle hashes are computed through after a file has opened.
fn reread_handle(
    f: &crate::state::LoadedFile,
) -> Option<crate::audio::streaming_source::FileHandle> {
    f.file_handle.clone().or_else(|| f.hash_source.clone())
}

/// Whether hashes of file `f` can still be computed: its bytes can be read
/// again.
pub fn can_compute_hashes(f: &crate::state::LoadedFile) -> bool {
    f.file_handle.is_some() || f.hash_source.is_some()
}

/// The kinds of hash file `file_id` has a saved value of and no computed one.
fn saved_kinds_not_computed(state: AppState, file_id: u64) -> Vec<HashKind> {
    state.library.files().with_untracked(|files| {
        files
            .iter()
            .find(|f| f.id == file_id)
            .map(|f| {
                file_hash_rows(f)
                    .iter()
                    .filter(|r| r.state() == Some(HashState::Saved))
                    .map(|r| r.kind)
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Compute `kinds` of hash for file `file_id` and put them in its identity.
/// Kinds the file already has, and kinds being computed, are left out. The
/// file is read from `bytes` when given, and otherwise through its handle.
///
/// The full BLAKE3 and the content hash are computed together: both are
/// BLAKE3, and the second costs little once the file is being read.
async fn run_hash_job(
    state: AppState,
    file_id: u64,
    mut kinds: Vec<HashKind>,
    bytes: Option<std::sync::Arc<Vec<u8>>>,
) {
    if kinds.contains(&HashKind::Blake3) || kinds.contains(&HashKind::Content) {
        kinds.extend([HashKind::Blake3, HashKind::Content]);
    }
    let jobs = hash_jobs();
    let found = state.library.files().with_untracked(|files| {
        files
            .iter()
            .find(|f| f.id == file_id)
            .and_then(|f| Some((f.identity.clone()?, reread_handle(f))))
    });
    let Some((identity, handle)) = found else {
        return;
    };
    let mut wanted: Vec<HashKind> = Vec::new();
    for kind in kinds {
        let have = kind.in_identity(&identity).is_some();
        let running = jobs.with_untracked(|j| j.running(file_id, kind).is_some());
        if !have && !running && !wanted.contains(&kind) {
            wanted.push(kind);
        }
    }
    if wanted.is_empty() {
        return;
    }
    let from_memory = bytes.is_some();
    let reader: Box<dyn AsyncRangeReader + '_> = match (bytes, handle.as_ref()) {
        (Some(bytes), _) => Box::new(BytesRangeReader { bytes }),
        (None, Some(handle)) => reader_from_handle(handle),
        (None, None) => return,
    };

    let job_id = NEXT_HASH_JOB.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    jobs.update(|j| {
        j.failed
            .retain(|(id, kind, _)| !(*id == file_id && wanted.contains(kind)));
        j.running.push(HashJob {
            id: job_id,
            file_id,
            kinds: wanted.clone(),
            read: 0,
            total: identity.file_size,
        });
    });
    // The status bar's "Computing file identity..." is for reads of the file.
    // Hashing bytes already in memory as a file opens is over too soon to
    // announce.
    let reads_file = !from_memory;
    if reads_file {
        state.status.hash_computing().set(true);
    }

    let result = async {
        let mut hashes = if wanted.iter().any(|k| *k != HashKind::Spot) {
            compute_whole_file_hashes(
                reader.as_ref(),
                identity.file_size,
                identity.data_offset,
                identity.data_size,
                &wanted,
                |read| {
                    jobs.update(|j| {
                        if let Some(job) = j.running.iter_mut().find(|job| job.id == job_id) {
                            job.read = read;
                        }
                    })
                },
                // The file was closed.
                || state.file_idx_for_id(file_id).is_none(),
            )
            .await?
        } else {
            ComputedHashes::default()
        };
        if wanted.contains(&HashKind::Spot) {
            hashes.spot = Some(
                compute_spot_hash_b3(
                    reader.as_ref(),
                    identity.file_size,
                    identity.data_offset,
                    identity.data_size,
                )
                .await?,
            );
        }
        Ok::<_, String>(hashes)
    }
    .await;

    jobs.update(|j| {
        j.running.retain(|job| job.id != job_id);
        if let Err(e) = &result {
            if e != "Cancelled" {
                for kind in &wanted {
                    j.failed.push((file_id, *kind, e.clone()));
                }
            }
        }
    });
    if reads_file {
        state
            .status
            .hash_computing()
            .set(jobs.with_untracked(|j| !j.running.is_empty()));
    }

    let hashes = match result {
        Ok(hashes) => hashes,
        Err(e) => {
            if e != "Cancelled" {
                log::warn!("Hash computation failed: {e}");
            }
            return;
        }
    };
    let Some(file_index) = state.file_idx_for_id(file_id) else {
        return;
    };
    state.library.files().update(|files| {
        if let Some(id) = files.get_mut(file_index).and_then(|f| f.identity.as_mut()) {
            for kind in &wanted {
                if let Some(hash) = kind.in_computed(&hashes) {
                    *kind.in_identity_mut(id) = Some(hash.clone());
                }
            }
        }
    });
    // The saved annotations hold the file's hashes.
    crate::opfs::save_annotations_to_opfs(state, file_index);
}

/// Compute `kinds` of hash for file `file_id` from the file, when the user
/// asks for them in the Info panel. A kind that has a saved hash is compared
/// with it once computed (`HashRow::state`).
pub fn compute_hashes(state: AppState, file_id: u64, kinds: Vec<HashKind>) {
    wasm_bindgen_futures::spawn_local(run_hash_job(state, file_id, kinds, None));
}

/// For a small file (`SMALL_FILE_THRESHOLD`), compute every kind of hash it
/// has a saved value of and no computed one, so the saved value is compared
/// with the file. The Info panel calls this when it shows the file's hashes.
/// A kind whose computation failed is not tried again here; the panel's
/// Check button tries it again.
pub fn check_saved_hashes_of_small_file(state: AppState, file_id: u64) {
    let small_and_readable = state.library.files().with_untracked(|files| {
        files.iter().find(|f| f.id == file_id).is_some_and(|f| {
            can_compute_hashes(f)
                && f.identity
                    .as_ref()
                    .is_some_and(|id| id.file_size < SMALL_FILE_THRESHOLD)
        })
    });
    if !small_and_readable {
        return;
    }
    let jobs = hash_jobs();
    let kinds: Vec<HashKind> = saved_kinds_not_computed(state, file_id)
        .into_iter()
        .filter(|kind| {
            jobs.with_untracked(|j| {
                j.failure(file_id, *kind).is_none() && j.running(file_id, *kind).is_none()
            })
        })
        .collect();
    if !kinds.is_empty() {
        compute_hashes(state, file_id, kinds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_set_goes_to_the_file_whose_hash_it_holds() {
        let with = |spot: Option<&str>, content: Option<&str>| {
            let mut id = identity_layer1("rec.wav", 12345);
            id.spot_hash_b3 = spot.map(str::to_string);
            id.content_hash = content.map(str::to_string);
            id
        };
        // Same spot hash.
        assert!(saved_identity_agrees(
            &with(Some("aa"), None),
            &with(Some("aa"), None)
        ));
        // Same name and size, different spot hash: another file.
        assert!(!saved_identity_agrees(
            &with(Some("aa"), None),
            &with(Some("bb"), None)
        ));
        // The header was edited: the spot hash changed, the audio did not.
        assert!(saved_identity_agrees(
            &with(Some("aa"), Some("cc")),
            &with(Some("bb"), Some("cc"))
        ));
        assert!(!saved_identity_agrees(
            &with(Some("aa"), Some("cc")),
            &with(Some("bb"), Some("dd"))
        ));
        // Nothing to compare: a set saved with no hash, or a file with none.
        assert!(saved_identity_agrees(
            &with(None, None),
            &with(Some("bb"), None)
        ));
        assert!(saved_identity_agrees(
            &with(Some("aa"), None),
            &with(None, None)
        ));
        // The saved set has only a content hash and the file only a spot hash.
        assert!(saved_identity_agrees(
            &with(None, Some("cc")),
            &with(Some("bb"), None)
        ));
    }

    fn saved(kind: HashKind, hash: &str) -> SidecarHashes {
        let mut s = SidecarHashes::default();
        let slot = match kind {
            HashKind::Spot => &mut s.spot_hash_b3,
            HashKind::Blake3 => &mut s.blake3,
            HashKind::Sha256 => &mut s.sha256,
            HashKind::Content => &mut s.content_hash,
        };
        *slot = Some(hash.to_string());
        s
    }

    fn state_of(rows: &[HashRow], kind: HashKind) -> Option<HashState> {
        rows.iter()
            .find(|r| r.kind == kind)
            .and_then(HashRow::state)
    }

    #[test]
    fn a_hash_row_says_where_its_hash_is_from_and_whether_it_matches() {
        let mut id = identity_layer1("rec.wav", 1000);
        id.spot_hash_b3 = Some("aa".into());
        id.full_blake3 = Some("bb".into());
        id.full_sha256 = Some("cc".into());
        let mut xc = saved(HashKind::Blake3, "BB");
        xc.sha256 = Some("dd".into());
        let annotations = saved(HashKind::Content, "ee");
        let rows = hash_rows(Some(&id), Some(&xc), Some(&annotations));

        // Computed, and nothing saved to compare it with.
        assert_eq!(state_of(&rows, HashKind::Spot), Some(HashState::Computed));
        // Equal to the saved hash, whatever the case of its letters.
        assert_eq!(state_of(&rows, HashKind::Blake3), Some(HashState::Match));
        assert_eq!(state_of(&rows, HashKind::Sha256), Some(HashState::Mismatch));
        // Saved, and not computed from the file.
        assert_eq!(state_of(&rows, HashKind::Content), Some(HashState::Saved));
        assert_eq!(
            rows[3].saved,
            Some(("ee".to_string(), SavedSource::Annotations))
        );

        // No hash of a kind, computed or saved.
        let rows = hash_rows(Some(&identity_layer1("rec.wav", 1000)), None, None);
        assert!(rows.iter().all(|r| r.state().is_none()));
    }

    #[test]
    fn a_xeno_canto_hash_is_used_before_one_from_saved_annotations() {
        let xc = saved(HashKind::Sha256, "from-xc");
        let annotations = saved(HashKind::Sha256, "from-annotations");
        let rows = hash_rows(None, Some(&xc), Some(&annotations));
        assert_eq!(
            rows[2].saved,
            Some(("from-xc".to_string(), SavedSource::XenoCanto))
        );
    }

    #[test]
    fn the_main_hash_is_the_full_blake3_below_the_threshold_and_the_spot_hash_from_it() {
        let mut id = identity_layer1("rec.wav", 1000);
        id.spot_hash_b3 = Some("aa".into());
        id.full_blake3 = Some("bb".into());
        let rows = hash_rows(Some(&id), None, None);
        assert_eq!(
            main_hash(SMALL_FILE_THRESHOLD - 1, &rows, false),
            HashKind::Blake3
        );
        assert_eq!(
            main_hash(SMALL_FILE_THRESHOLD, &rows, false),
            HashKind::Spot
        );

        // A small file with no full BLAKE3: the SHA-256 if there is one, then
        // the content hash, then the spot hash.
        id.full_blake3 = None;
        let rows = hash_rows(Some(&id), None, None);
        assert_eq!(main_hash(1000, &rows, false), HashKind::Spot);
        let rows = hash_rows(Some(&id), None, Some(&saved(HashKind::Content, "cc")));
        assert_eq!(main_hash(1000, &rows, false), HashKind::Content);
        let rows = hash_rows(Some(&id), Some(&saved(HashKind::Sha256, "dd")), None);
        assert_eq!(main_hash(1000, &rows, false), HashKind::Sha256);
        // While the full BLAKE3 is being computed, its row is the main one.
        assert_eq!(main_hash(1000, &rows, true), HashKind::Blake3);
    }

    #[test]
    fn only_the_audio_matches_when_the_content_hash_is_equal_and_a_file_hash_differs() {
        let mut id = identity_layer1("rec.wav", 1000);
        id.full_blake3 = Some("new".into());
        id.content_hash = Some("audio".into());
        let mut before = saved(HashKind::Blake3, "old");
        before.content_hash = Some("audio".into());
        assert!(only_audio_matches(&hash_rows(
            Some(&id),
            None,
            Some(&before)
        )));

        // The whole file is the saved one.
        before.blake3 = Some("new".into());
        assert!(!only_audio_matches(&hash_rows(
            Some(&id),
            None,
            Some(&before)
        )));
        // The audio differs too.
        before.blake3 = Some("old".into());
        before.content_hash = Some("other".into());
        assert!(!only_audio_matches(&hash_rows(
            Some(&id),
            None,
            Some(&before)
        )));
    }

    #[test]
    fn identity_layer1_only_carries_filename_and_size() {
        let id = identity_layer1("rec.wav", 12345);
        assert_eq!(id.filename, "rec.wav");
        assert_eq!(id.file_size, 12345);
        assert!(id.spot_hash_b3.is_none());
        assert!(id.content_hash.is_none());
        assert!(id.full_blake3.is_none());
        assert!(id.full_sha256.is_none());
        assert!(id.data_offset.is_none());
        assert!(id.data_size.is_none());
        assert!(id.file_path.is_none());
    }

    #[test]
    fn audio_region_defaults_to_whole_file() {
        assert_eq!(audio_region(1000, None, None), (0, 1000));
    }

    #[test]
    fn audio_region_respects_explicit_offset_and_size() {
        assert_eq!(audio_region(1000, Some(44), Some(800)), (44, 800));
    }

    #[test]
    fn audio_region_derives_size_from_offset_when_size_unset() {
        // file=1000, offset=44 → audio_len = 1000 - 44 = 956
        assert_eq!(audio_region(1000, Some(44), None), (44, 956));
    }

    #[test]
    fn audio_region_saturates_when_offset_exceeds_file() {
        // Pathological case: offset larger than file_size should not underflow.
        assert_eq!(audio_region(100, Some(200), None), (200, 0));
    }

    #[test]
    fn spot_chunk_positions_empty_audio() {
        assert!(spot_chunk_positions(0, 0).is_empty());
    }

    #[test]
    fn spot_chunk_positions_small_audio_emits_one_chunk() {
        // audio_len smaller than SPOT_CHUNK_SIZE → exactly one chunk covering it all.
        let pos = spot_chunk_positions(0, 100_000);
        assert_eq!(pos.len(), 1);
        assert_eq!(pos[0], (0, 100_000));
    }

    #[test]
    fn spot_chunk_positions_evenly_spaces_chunks() {
        // 32 MB of audio → 16 chunks of 1 MB, evenly spaced.
        let audio_len = 32 * SPOT_CHUNK_SIZE;
        let pos = spot_chunk_positions(0, audio_len);
        assert_eq!(pos.len(), NUM_SPOT_CHUNKS as usize);
        let stride = audio_len / NUM_SPOT_CHUNKS;
        for (i, &(start, len)) in pos.iter().enumerate() {
            assert_eq!(start, i as u64 * stride);
            assert_eq!(len, SPOT_CHUNK_SIZE);
        }
    }

    #[test]
    fn finalize_spot_hash_is_deterministic_and_order_sensitive() {
        let h1 = blake3::hash(b"a");
        let h2 = blake3::hash(b"b");

        let ab = finalize_spot_hash(&[h1, h2]);
        let ab_again = finalize_spot_hash(&[h1, h2]);
        let ba = finalize_spot_hash(&[h2, h1]);

        assert_eq!(ab, ab_again);
        assert_ne!(ab, ba, "swapping chunk order must change the hash");
        // BLAKE3 hex digest is 64 characters.
        assert_eq!(ab.len(), 64);
    }

    #[test]
    fn compute_spot_hash_b3_sync_matches_for_identical_bytes() {
        // Two byte arrays with the same content should hash identically.
        let mut bytes = vec![0u8; 4 * SPOT_CHUNK_SIZE as usize];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let h1 = compute_spot_hash_b3_sync(&bytes, None, None);
        let h2 = compute_spot_hash_b3_sync(&bytes, None, None);
        assert_eq!(h1, h2);

        // Changing a byte inside the audio region must change the hash.
        let mut tampered = bytes.clone();
        tampered[100] ^= 0xff;
        assert_ne!(h1, compute_spot_hash_b3_sync(&tampered, None, None));
    }

    #[test]
    fn compute_spot_hash_b3_sync_skips_header_when_data_offset_provided() {
        // Same audio region, different "header" bytes — hash should be unchanged.
        let mut a = vec![0u8; 1000];
        let mut b = vec![0u8; 1000];
        for (i, byte) in a.iter_mut().enumerate() {
            *byte = (i % 251) as u8;
        }
        b.copy_from_slice(&a);
        // Vary the first 44 bytes (the "header"); audio starts at offset 44.
        a[..44].fill(0xAA);
        b[..44].fill(0x55);
        let ha = compute_spot_hash_b3_sync(&a, Some(44), Some(956));
        let hb = compute_spot_hash_b3_sync(&b, Some(44), Some(956));
        assert_eq!(ha, hb, "header bytes must not affect spot hash");
    }
}

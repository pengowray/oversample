// SPDX-License-Identifier: GPL-3.0-only OR MIT OR Apache-2.0
//! Streaming audio sources — streams decoded samples from a file on demand.
//!
//! For audio files whose decoded size exceeds a threshold (~512 MB), only the
//! header is parsed and the first 30 seconds are decoded upfront. Remaining
//! samples are decoded on demand into an LRU chunk cache.
//!
//! - `StreamingWavSource`: random-access PCM via `File.slice()` byte ranges
//! - `StreamingFlacSource`: progressive sequential FLAC decode in 4 MB windows
//!
//! `read_samples()` is sync and returns silence for frames that aren't cached,
//! so callers read through [`read_region_ready`] / [`read_views_ready`], which
//! load the region and return `None` if any of it is still missing.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use crate::audio::loader::{FlacHeader, WavHeader};

// Re-export MP3, OGG, and M4A sources from their own modules
pub use super::streaming_m4a::StreamingM4aSource;
pub use super::streaming_mp3::StreamingMp3Source;
pub use super::streaming_ogg::StreamingOggSource;
use crate::audio::source::{AudioSource, ChannelView};

/// Number of per-channel frames per cache chunk (~256K frames).
/// At 384 kHz stereo 32-bit float, one chunk ≈ 2 MB raw + 1 MB mono = 3 MB.
pub(crate) const CHUNK_FRAMES: usize = 262_144;

/// Maximum cache size in bytes (128 MB).
const CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;

/// Format info extracted from the WAV header, needed for PCM decoding.
#[derive(Clone, Debug)]
struct WavFormatInfo {
    sample_rate: u32,
    channels: u32,
    encoding: crate::audio::wav::SampleEncoding,
    data_offset: u64,
    bytes_per_frame: u32,
}

/// A single cached chunk of decoded audio.
pub(crate) struct CachedChunk {
    /// Mono-mixed samples for this chunk.
    pub(crate) mono: Vec<f32>,
    /// Interleaved raw samples (if multi-channel). `None` for mono files.
    pub(crate) raw: Option<Vec<f32>>,
}

impl CachedChunk {
    pub(crate) fn byte_size(&self) -> usize {
        self.mono.len() * 4 + self.raw.as_ref().map_or(0, |r| r.len() * 4)
    }
}

/// LRU chunk cache for decoded audio regions.
pub(crate) struct ChunkCache {
    /// Decoded chunks keyed by chunk index (frame / CHUNK_FRAMES). `Rc` so a
    /// prefetch can hold its chunks while it awaits further reads, and put
    /// back any that were evicted in the meantime.
    chunks: HashMap<u64, Rc<CachedChunk>>,
    /// LRU order: least recently used at the front, most recent at the back.
    /// `VecDeque` keeps eviction (`pop_front`) O(1); the bounded budget means
    /// the deque never grows large enough to make the linear `iter().position`
    /// touch-promotion meaningfully slow.
    lru: VecDeque<u64>,
    /// Current total memory usage in bytes.
    total_bytes: usize,
}

impl ChunkCache {
    pub(crate) fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            lru: VecDeque::new(),
            total_bytes: 0,
        }
    }

    /// Move `chunk_idx` to the back of the LRU list (most recently used).
    fn lru_move_to_back(&mut self, chunk_idx: u64) {
        if let Some(pos) = self.lru.iter().position(|&k| k == chunk_idx) {
            self.lru.remove(pos);
        }
        self.lru.push_back(chunk_idx);
    }

    /// Get a cached chunk, updating LRU order.
    pub(crate) fn get(&mut self, chunk_idx: u64) -> Option<&CachedChunk> {
        if self.chunks.contains_key(&chunk_idx) {
            self.lru_move_to_back(chunk_idx);
            self.chunks.get(&chunk_idx).map(|c| &**c)
        } else {
            None
        }
    }

    /// Get a shared handle to a cached chunk, updating LRU order.
    pub(crate) fn get_rc(&mut self, chunk_idx: u64) -> Option<Rc<CachedChunk>> {
        if self.chunks.contains_key(&chunk_idx) {
            self.lru_move_to_back(chunk_idx);
        }
        self.chunks.get(&chunk_idx).cloned()
    }

    /// Insert a chunk, evicting oldest entries if over budget.
    pub(crate) fn insert(&mut self, chunk_idx: u64, chunk: CachedChunk) {
        self.insert_rc(chunk_idx, Rc::new(chunk));
    }

    /// Insert a shared chunk, evicting oldest entries if over budget.
    pub(crate) fn insert_rc(&mut self, chunk_idx: u64, chunk: Rc<CachedChunk>) {
        let size = chunk.byte_size();

        // Remove existing entry first to fix memory accounting on duplicate inserts
        if let Some(existing) = self.chunks.remove(&chunk_idx) {
            self.total_bytes -= existing.byte_size();
            if let Some(pos) = self.lru.iter().position(|&k| k == chunk_idx) {
                self.lru.remove(pos);
            }
        }

        // Evict until we have room
        while self.total_bytes + size > CACHE_MAX_BYTES {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(removed) = self.chunks.remove(&oldest) {
                self.total_bytes -= removed.byte_size();
            }
        }

        self.total_bytes += size;
        self.chunks.insert(chunk_idx, chunk);
        self.lru.push_back(chunk_idx);
    }

    /// Update LRU position for a cached chunk (mark as recently used).
    pub(crate) fn touch(&mut self, chunk_idx: u64) {
        if self.chunks.contains_key(&chunk_idx) {
            self.lru_move_to_back(chunk_idx);
        }
    }

    /// Check if a chunk is cached (without updating LRU).
    pub(crate) fn contains(&self, chunk_idx: u64) -> bool {
        self.chunks.contains_key(&chunk_idx)
    }

    /// True when every chunk holding frames `[start, end)` is cached. Frames
    /// before `head_frames` live in the source's head buffer and always count.
    pub(crate) fn covers(&self, head_frames: u64, start: u64, end: u64) -> bool {
        let start = start.max(head_frames);
        if end <= start {
            return true;
        }
        let first = start / CHUNK_FRAMES as u64;
        let last = (end - 1) / CHUNK_FRAMES as u64;
        (first..=last).all(|ci| self.chunks.contains_key(&ci))
    }
}

/// Handle to the underlying file — either a browser `web_sys::File` or a native
/// path string (used via Tauri IPC `read_file_range` command).
#[derive(Clone)]
pub enum FileHandle {
    /// Browser / webview: uses `File.slice()` + `FileReader`.
    WebFile(web_sys::File),
    /// Tauri desktop/mobile: uses native `read_file_range` IPC command.
    TauriPath(String),
    /// Android MediaStore `content://` URI: ranges are read back via the
    /// `plugin:media-store|readRecordingRange` Kotlin command (base64). Used to
    /// display a recording that lives only in public shared storage.
    MediaStoreUri(String),
    /// In-memory bytes — holds the whole file; range reads just slice it. Used
    /// by host tests of the streaming sources (the WebFile / Tauri handles can't
    /// run off-WASM). Not constructed in production.
    Bytes(std::sync::Arc<Vec<u8>>),
}

/// Read a byte range from an Android MediaStore `content://` URI via the
/// media-store plugin (which returns base64, since Kotlin plugin commands can't
/// return raw buffers). Used by [`FileHandle::MediaStoreUri`].
pub async fn read_media_store_range(
    uri: &str,
    byte_start: u64,
    byte_len: u64,
) -> Result<Vec<u8>, String> {
    let args = oversample_ipc::plugins::ReadRecordingRangeArgs {
        uri: uri.to_string(),
        offset: byte_start as f64,
        length: byte_len as f64,
    };
    let res: oversample_ipc::plugins::ReadRecordingRangeResult =
        crate::tauri_bridge::tauri_invoke_typed_args(
            "plugin:media-store|readRecordingRange",
            &args,
        )
        .await
        .map_err(|e| format!("readRecordingRange failed: {:?}", e))?;
    base64_decode(&res.data)
}

/// Minimal standard-alphabet base64 decoder (no external dep). Ignores
/// whitespace; accepts with or without `=` padding.
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = val(c).ok_or_else(|| format!("invalid base64 char: {}", c as char))?;
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

impl FileHandle {
    /// Read a byte range `[byte_start, byte_start + byte_len)` via whichever
    /// backend this handle uses. The single dispatch point for all sources.
    pub async fn read_range(&self, byte_start: u64, byte_len: u64) -> Result<Vec<u8>, String> {
        match self {
            FileHandle::WebFile(file) => {
                read_blob_range(file, byte_start as f64, (byte_start + byte_len) as f64).await
            }
            FileHandle::TauriPath(path) => {
                crate::tauri_bridge::read_file_range(path, byte_start, byte_len).await
            }
            FileHandle::MediaStoreUri(uri) => {
                read_media_store_range(uri, byte_start, byte_len).await
            }
            FileHandle::Bytes(b) => slice_bytes(b, byte_start, byte_start + byte_len),
        }
    }
}

/// Clamp-and-slice a byte range `[start, end)` from an in-memory buffer, used by
/// [`FileHandle::Bytes`] to mimic a successful range read.
pub(crate) fn slice_bytes(bytes: &[u8], start: u64, end: u64) -> Result<Vec<u8>, String> {
    let len = bytes.len() as u64;
    let s = start.min(len) as usize;
    let e = end.min(len) as usize;
    Ok(bytes[s..e].to_vec())
}

impl std::fmt::Debug for FileHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileHandle::WebFile(file) => write!(f, "WebFile(\"{}\")", file.name()),
            FileHandle::TauriPath(path) => write!(f, "TauriPath(\"{}\")", path),
            FileHandle::MediaStoreUri(uri) => write!(f, "MediaStoreUri(\"{}\")", uri),
            FileHandle::Bytes(b) => write!(f, "Bytes({} B)", b.len()),
        }
    }
}

// SAFETY: WASM is single-threaded; Send+Sync required for storage in RwSignal<Vec<LoadedFile>>.
unsafe impl Send for FileHandle {}
unsafe impl Sync for FileHandle {}

/// Streaming audio source backed by a file handle (browser File or native path).
///
/// The first ~30 seconds are decoded during construction and kept in memory
/// permanently. Beyond that, samples are decoded on demand via file range reads
/// and cached in an LRU chunk cache.
pub struct StreamingWavSource {
    /// The file handle — browser File or native path.
    handle: FileHandle,
    /// WAV format info for PCM decoding.
    info: WavFormatInfo,
    /// Total per-channel frames in the file.
    total_frames: u64,
    /// Pre-decoded first 30s (mono-mixed). Always available.
    head_mono: Arc<Vec<f32>>,
    /// Pre-decoded first 30s (raw interleaved, if multi-channel).
    head_raw: Option<Arc<Vec<f32>>>,
    /// Number of head frames (per-channel).
    head_frames: usize,
    /// LRU cache for chunks beyond the head region.
    cache: RefCell<ChunkCache>,
}

// SAFETY: required by AudioSource: Send + Sync, but this holds RefCell
// caches/cursors so it is NOT inherently Sync. Sound ONLY because the app is
// single-threaded (WASM) and the source is touched from that one thread; unsound
// from a real worker thread. Invariant: never call a read_* method while an
// internal cache.borrow_mut() is alive (BorrowMutError — see the
// ChannelView::Difference recursion in read_samples).
unsafe impl Send for StreamingWavSource {}
unsafe impl Sync for StreamingWavSource {}

impl std::fmt::Debug for StreamingWavSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingWavSource")
            .field("total_frames", &self.total_frames)
            .field("head_frames", &self.head_frames)
            .field("sample_rate", &self.info.sample_rate)
            .field("channels", &self.info.channels)
            .finish()
    }
}

impl StreamingWavSource {
    /// Create a new streaming source from a parsed header and pre-decoded head samples.
    ///
    /// - `handle`: file handle (browser File or native path)
    /// - `header`: parsed WAV header with format info and data offset
    /// - `head_mono`: mono-mixed samples for the first ~30s
    /// - `head_raw`: raw interleaved samples for the first ~30s (None for mono)
    pub fn new(
        handle: FileHandle,
        header: &WavHeader,
        head_mono: Vec<f32>,
        head_raw: Option<Vec<f32>>,
    ) -> Self {
        let head_frames = head_mono.len();
        Self {
            handle,
            info: WavFormatInfo {
                sample_rate: header.sample_rate,
                channels: header.channels as u32,
                encoding: header.encoding,
                data_offset: header.data_offset,
                bytes_per_frame: header.block_align as u32,
            },
            total_frames: header.total_frames,
            head_mono: Arc::new(head_mono),
            head_raw: head_raw.map(Arc::new),
            head_frames,
            cache: RefCell::new(ChunkCache::new()),
        }
    }

    /// Async: ensure all chunks covering `[start_frame, start_frame + len)` are cached.
    ///
    /// Must be called before `read_samples()` for regions beyond the head.
    /// Safe to call for head-region frames (no-op).
    pub async fn prefetch_region(&self, start_frame: u64, len: usize) {
        let end_frame = (start_frame + len as u64).min(self.total_frames);
        if end_frame <= self.head_frames as u64 {
            return; // Entirely within head — no fetch needed
        }

        // Determine which chunks we need
        let fetch_start = start_frame.max(self.head_frames as u64);
        let first_chunk = fetch_start / CHUNK_FRAMES as u64;
        let last_chunk = end_frame.saturating_sub(1) / CHUNK_FRAMES as u64;

        // Other prefetches (more tiles, background scans, playback) insert
        // chunks while this one awaits its reads, and can evict the chunks it
        // already has: eight coarse tiles at once need more than the cache
        // holds for a long stereo file. Hold every chunk until the end and put
        // back any that were evicted, so the caller's read right after this
        // returns gets real samples rather than silence.
        let mut held: Vec<(u64, Rc<CachedChunk>)> = Vec::new();

        for chunk_idx in first_chunk..=last_chunk {
            if let Some(chunk) = self.cache.borrow_mut().get_rc(chunk_idx) {
                held.push((chunk_idx, chunk));
                continue;
            }
            // Compute byte range for this chunk
            let chunk_start_frame = chunk_idx * CHUNK_FRAMES as u64;
            let chunk_end_frame = (chunk_start_frame + CHUNK_FRAMES as u64).min(self.total_frames);
            let frames_in_chunk = (chunk_end_frame - chunk_start_frame) as usize;

            let byte_start =
                self.info.data_offset + chunk_start_frame * self.info.bytes_per_frame as u64;
            let byte_len = frames_in_chunk as u64 * self.info.bytes_per_frame as u64;

            // Read raw bytes from file via the appropriate backend.
            let bytes = self.handle.read_range(byte_start, byte_len).await;
            let bytes = match bytes {
                Ok(b) => b,
                Err(e) => {
                    log::warn!(
                        "StreamingWavSource: prefetch chunk {} failed: {}",
                        chunk_idx,
                        e
                    );
                    continue;
                }
            };

            // Decode PCM bytes to f32 samples
            let interleaved = crate::audio::wav::decode_pcm(&bytes, self.info.encoding);
            let channels = self.info.channels as usize;

            let (mono, raw) = if channels == 1 {
                (interleaved, None)
            } else {
                let mono = mix_to_mono(&interleaved, channels);
                (mono, Some(interleaved))
            };

            let chunk = Rc::new(CachedChunk { mono, raw });
            self.cache.borrow_mut().insert_rc(chunk_idx, chunk.clone());
            held.push((chunk_idx, chunk));
        }

        // No await from here on.
        let mut cache = self.cache.borrow_mut();
        for (chunk_idx, chunk) in held {
            if cache.contains(chunk_idx) {
                cache.touch(chunk_idx);
            } else {
                cache.insert_rc(chunk_idx, chunk);
            }
        }
    }

    /// True when `[start_frame, start_frame + len)` can be read without
    /// returning silence for missing chunks.
    pub fn region_ready(&self, start_frame: u64, len: usize) -> bool {
        let end = (start_frame + len as u64).min(self.total_frames);
        self.cache
            .borrow()
            .covers(self.head_frames as u64, start_frame, end)
    }

    /// Read mono samples from the head buffer.
    fn read_head_mono(&self, start: u64, buf: &mut [f32]) -> usize {
        let start = start as usize;
        let avail = self.head_frames.saturating_sub(start);
        let n = buf.len().min(avail);
        buf[..n].copy_from_slice(&self.head_mono[start..start + n]);
        n
    }

    /// Read mono samples from the cache (or return 0s for uncached regions).
    fn read_cached_mono(&self, start: u64, buf: &mut [f32]) -> usize {
        let end = (start + buf.len() as u64).min(self.total_frames);
        let total_to_read = (end - start) as usize;
        let mut written = 0usize;
        let mut cache = self.cache.borrow_mut();

        while written < total_to_read {
            let frame = start + written as u64;
            let chunk_idx = frame / CHUNK_FRAMES as u64;
            let offset_in_chunk = (frame % CHUNK_FRAMES as u64) as usize;

            let chunk_end_frame = ((chunk_idx + 1) * CHUNK_FRAMES as u64).min(self.total_frames);
            let avail_in_chunk = (chunk_end_frame - frame) as usize;
            let to_read = avail_in_chunk.min(total_to_read - written);

            if let Some(chunk) = cache.get(chunk_idx) {
                let src = &chunk.mono[offset_in_chunk..offset_in_chunk + to_read];
                buf[written..written + to_read].copy_from_slice(src);
            } else {
                // Not cached — fill with silence
                for s in &mut buf[written..written + to_read] {
                    *s = 0.0;
                }
            }

            written += to_read;
        }

        total_to_read
    }

    /// Read raw interleaved samples from cache for a specific channel.
    fn read_cached_channel(&self, ch: u32, start: u64, buf: &mut [f32]) -> usize {
        let end = (start + buf.len() as u64).min(self.total_frames);
        let total_to_read = (end - start) as usize;
        let channels = self.info.channels as usize;
        let mut written = 0usize;
        let mut cache = self.cache.borrow_mut();

        while written < total_to_read {
            let frame = start + written as u64;
            let chunk_idx = frame / CHUNK_FRAMES as u64;
            let offset_in_chunk = (frame % CHUNK_FRAMES as u64) as usize;

            let chunk_end_frame = ((chunk_idx + 1) * CHUNK_FRAMES as u64).min(self.total_frames);
            let avail_in_chunk = (chunk_end_frame - frame) as usize;
            let to_read = avail_in_chunk.min(total_to_read - written);

            if let Some(chunk) = cache.get(chunk_idx) {
                if let Some(raw) = &chunk.raw {
                    for i in 0..to_read {
                        buf[written + i] = raw[(offset_in_chunk + i) * channels + ch as usize];
                    }
                } else {
                    // Mono file — raw not stored, use mono
                    let src = &chunk.mono[offset_in_chunk..offset_in_chunk + to_read];
                    buf[written..written + to_read].copy_from_slice(src);
                }
            } else {
                for s in &mut buf[written..written + to_read] {
                    *s = 0.0;
                }
            }

            written += to_read;
        }

        total_to_read
    }

    /// Read head raw interleaved samples for a specific channel.
    fn read_head_channel(&self, ch: u32, start: u64, buf: &mut [f32]) -> usize {
        let start = start as usize;
        let avail = self.head_frames.saturating_sub(start);
        let n = buf.len().min(avail);

        if let Some(raw) = &self.head_raw {
            let channels = self.info.channels as usize;
            for i in 0..n {
                buf[i] = raw[(start + i) * channels + ch as usize];
            }
        } else {
            // Mono — same as mono mix
            buf[..n].copy_from_slice(&self.head_mono[start..start + n]);
        }
        n
    }
}

impl AudioSource for StreamingWavSource {
    fn total_samples(&self) -> u64 {
        self.total_frames
    }

    fn sample_rate(&self) -> u32 {
        self.info.sample_rate
    }

    fn channel_count(&self) -> u32 {
        self.info.channels
    }

    fn is_fully_loaded(&self) -> bool {
        false
    }

    fn read_samples(&self, channel: ChannelView, start: u64, buf: &mut [f32]) -> usize {
        let end = (start + buf.len() as u64).min(self.total_frames);
        if end <= start {
            return 0;
        }
        let total_len = (end - start) as usize;
        let buf = &mut buf[..total_len];

        let head_end = self.head_frames as u64;

        match channel {
            ChannelView::Stereo | ChannelView::MonoMix => {
                if end <= head_end {
                    // Entirely within head
                    self.read_head_mono(start, buf)
                } else if start >= head_end {
                    // Entirely in cache region
                    self.read_cached_mono(start, buf)
                } else {
                    // Spans head and cache
                    let head_part = (head_end - start) as usize;
                    self.read_head_mono(start, &mut buf[..head_part]);
                    self.read_cached_mono(head_end, &mut buf[head_part..]);
                    total_len
                }
            }
            ChannelView::Channel(ch) => {
                if ch >= self.info.channels {
                    for s in buf.iter_mut() {
                        *s = 0.0;
                    }
                    return total_len;
                }
                if end <= head_end {
                    self.read_head_channel(ch, start, buf)
                } else if start >= head_end {
                    self.read_cached_channel(ch, start, buf)
                } else {
                    let head_part = (head_end - start) as usize;
                    self.read_head_channel(ch, start, &mut buf[..head_part]);
                    self.read_cached_channel(ch, head_end, &mut buf[head_part..]);
                    total_len
                }
            }
            ChannelView::Difference => {
                if self.info.channels < 2 {
                    for s in buf.iter_mut() {
                        *s = 0.0;
                    }
                    return total_len;
                }
                // Read L and R separately, compute difference.
                // RE-ENTRANCY INVARIANT: read_samples recurses here. This is only
                // sound because every `self.cache.borrow_mut()` inside the channel
                // reads is released BEFORE returning — so no RefCell borrow is
                // alive across these recursive calls. Never call a read_* method
                // while holding a cache borrow, or it panics with BorrowMutError
                // (the RefCell can't catch it at compile time). See the unsafe
                // Send/Sync impls for the single-threaded (WASM) invariant.
                let mut left = vec![0.0f32; total_len];
                let mut right = vec![0.0f32; total_len];
                self.read_samples(ChannelView::Channel(0), start, &mut left);
                self.read_samples(ChannelView::Channel(1), start, &mut right);
                for i in 0..total_len {
                    buf[i] = left[i] - right[i];
                }
                total_len
            }
        }
    }

    fn as_contiguous(&self) -> Option<&[f32]> {
        None
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ─── PCM decoding ───────────────────────────────────────────────────────────

/// Mix interleaved multi-channel samples to mono.
pub(crate) fn mix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    let inv = 1.0 / channels as f32;
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() * inv)
        .collect()
}

// ─── File.slice() async helper ──────────────────────────────────────────────

// ─── Streaming FLAC source ──────────────────────────────────────────────────

/// Streaming audio source for FLAC files.
///
/// FLAC is compressed and frame-based, so we can't do random byte-offset access.
/// Instead we decode sequentially in 4 MB windows, storing decoded chunks in the
/// same LRU cache used by WAV streaming. A background task progressively decodes
/// the entire file; on-demand prefetch fast-forwards the decode cursor if needed.
pub struct StreamingFlacSource {
    handle: FileHandle,
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u16,
    total_frames: u64,
    #[allow(dead_code)]
    first_frame_offset: u64,
    max_frame_size: u32,
    head_mono: Arc<Vec<f32>>,
    head_raw: Option<Arc<Vec<f32>>>,
    head_frames: usize,
    cache: RefCell<ChunkCache>,
    /// File byte offset for the next compressed read window.
    decode_byte_cursor: RefCell<u64>,
    /// Per-channel frames decoded so far (beyond the head).
    decode_frame_cursor: RefCell<u64>,
}

// SAFETY: required by AudioSource: Send + Sync, but this holds RefCell
// caches/cursors so it is NOT inherently Sync. Sound ONLY because the app is
// single-threaded (WASM) and the source is touched from that one thread; unsound
// from a real worker thread. Invariant: never call a read_* method while an
// internal cache.borrow_mut() is alive (BorrowMutError — see the
// ChannelView::Difference recursion in read_samples).
unsafe impl Send for StreamingFlacSource {}
unsafe impl Sync for StreamingFlacSource {}

impl std::fmt::Debug for StreamingFlacSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingFlacSource")
            .field("total_frames", &self.total_frames)
            .field("head_frames", &self.head_frames)
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("decode_frame_cursor", &*self.decode_frame_cursor.borrow())
            .finish()
    }
}

/// Size of each compressed read window for FLAC streaming (4 MB).
const FLAC_WINDOW_BYTES: u64 = 4 * 1024 * 1024;

impl StreamingFlacSource {
    /// Create a new streaming FLAC source.
    ///
    /// - `handle`: browser File or native path
    /// - `header`: parsed FLAC header
    /// - `head_mono`: mono-mixed samples for the first ~30s
    /// - `head_raw`: raw interleaved samples (None for mono)
    /// - `initial_byte_cursor`: byte offset where head decoding stopped
    /// - `initial_frame_cursor`: frames decoded in the head region
    pub fn new(
        handle: FileHandle,
        header: &FlacHeader,
        head_mono: Vec<f32>,
        head_raw: Option<Vec<f32>>,
        initial_byte_cursor: u64,
        initial_frame_cursor: u64,
    ) -> Self {
        let head_frames = head_mono.len();
        Self {
            handle,
            sample_rate: header.sample_rate,
            channels: header.channels as u32,
            bits_per_sample: header.bits_per_sample,
            total_frames: header.total_frames,
            first_frame_offset: header.first_frame_offset,
            max_frame_size: header.max_frame_size,
            head_mono: Arc::new(head_mono),
            head_raw: head_raw.map(Arc::new),
            head_frames,
            cache: RefCell::new(ChunkCache::new()),
            decode_byte_cursor: RefCell::new(initial_byte_cursor),
            decode_frame_cursor: RefCell::new(initial_frame_cursor),
        }
    }

    /// Async: ensure all chunks covering `[start_frame, start_frame + len)` are cached.
    ///
    /// For FLAC this may need to decode forward from the current cursor position.
    pub async fn prefetch_region(&self, start_frame: u64, len: usize) {
        let end_frame = (start_frame + len as u64).min(self.total_frames);
        if end_frame <= self.head_frames as u64 {
            return; // Entirely within head
        }

        // Check if everything requested is already cached
        let fetch_start = start_frame.max(self.head_frames as u64);
        let first_chunk = fetch_start / CHUNK_FRAMES as u64;
        let last_chunk = end_frame.saturating_sub(1) / CHUNK_FRAMES as u64;

        let all_cached = {
            let mut cache = self.cache.borrow_mut();
            let mut ok = true;
            for ci in first_chunk..=last_chunk {
                if cache.contains(ci) {
                    cache.touch(ci);
                } else {
                    ok = false;
                    break;
                }
            }
            ok
        };
        if all_cached {
            return;
        }

        // Need to decode forward until we've covered end_frame
        while *self.decode_frame_cursor.borrow() < end_frame {
            if self.decode_one_window().await.is_err() {
                break;
            }
            crate::canvas::tile_cache::yield_to_browser().await;
        }
    }

    /// Decode one window of compressed FLAC data (~4 MB), storing results in cache.
    /// Returns Ok(frames_decoded) or Err if at EOF or read failed.
    async fn decode_one_window(&self) -> Result<usize, String> {
        let byte_cursor = *self.decode_byte_cursor.borrow();
        let frame_cursor = *self.decode_frame_cursor.borrow();

        if frame_cursor >= self.total_frames {
            return Err("Already fully decoded".into());
        }

        // Safety overlap: re-read some bytes before the cursor to catch frames
        // that straddled the previous window boundary.
        let overlap = if self.max_frame_size > 0 {
            self.max_frame_size as u64
        } else {
            65535 // max possible FLAC frame size
        };
        let read_start = byte_cursor.saturating_sub(overlap);
        let read_end = read_start + FLAC_WINDOW_BYTES + overlap;

        let bytes = self
            .handle
            .read_range(read_start, read_end - read_start)
            .await;
        let bytes = match bytes {
            Ok(b) if b.is_empty() => return Err("EOF: no bytes read".into()),
            Ok(b) => b,
            Err(e) => return Err(format!("FLAC window read failed: {}", e)),
        };

        let bytes_len = bytes.len() as u64;

        // Create a FrameReader over the raw bytes.
        // FrameReader scans for FLAC sync codes (0xFFF8/0xFFF9), so it can
        // start decoding from the middle of a stream without a STREAMINFO header.
        let buf_reader = claxon::input::BufferedReader::new(std::io::Cursor::new(bytes));
        let mut frame_reader = claxon::frame::FrameReader::new(buf_reader);

        let channels = self.channels as usize;
        let max_val = (1u32 << (self.bits_per_sample - 1)) as f32;
        let mut total_new_frames = 0usize;
        let mut frames_since_yield = 0usize;
        const YIELD_EVERY_FRAMES: usize = 65_536;

        // Accumulate interleaved f32 samples, then flush to cache in CHUNK_FRAMES-sized pieces
        let mut pending_interleaved: Vec<f32> = Vec::new();
        let mut pending_start_frame = frame_cursor;

        let mut block_buf = Vec::new();
        loop {
            match frame_reader.read_next_or_eof(block_buf) {
                Ok(Some(block)) => {
                    let block_time = block.time();
                    let block_duration = block.duration() as u64;

                    // Skip blocks from the overlap region that we already decoded
                    if block_time + block_duration <= frame_cursor {
                        block_buf = block.into_buffer();
                        continue;
                    }

                    // Convert block samples to interleaved f32
                    let n_frames = block.duration() as usize;
                    for frame_idx in 0..n_frames {
                        for ch in 0..channels {
                            let sample = block.sample(ch as u32, frame_idx as u32);
                            pending_interleaved.push(sample as f32 / max_val);
                        }
                    }
                    total_new_frames += n_frames;
                    frames_since_yield += n_frames;

                    if frames_since_yield >= YIELD_EVERY_FRAMES {
                        frames_since_yield = 0;
                        crate::canvas::tile_cache::yield_to_browser().await;
                    }

                    // Flush complete CHUNK_FRAMES-sized chunks to cache
                    loop {
                        let pending_mono_frames = pending_interleaved.len() / channels;
                        if pending_mono_frames < CHUNK_FRAMES {
                            break;
                        }
                        let take_samples = CHUNK_FRAMES * channels;
                        let chunk_interleaved: Vec<f32> =
                            pending_interleaved.drain(..take_samples).collect();

                        let (mono, raw) = if channels == 1 {
                            (chunk_interleaved, None)
                        } else {
                            let mono = mix_to_mono(&chunk_interleaved, channels);
                            (mono, Some(chunk_interleaved))
                        };

                        let chunk_idx = pending_start_frame / CHUNK_FRAMES as u64;
                        self.cache
                            .borrow_mut()
                            .insert(chunk_idx, CachedChunk { mono, raw });
                        pending_start_frame += CHUNK_FRAMES as u64;
                    }

                    block_buf = block.into_buffer();
                }
                Ok(None) => break, // Clean EOF
                Err(_) => break,   // Partial frame at window boundary — expected
            }
        }

        // Advance byte cursor: move forward by window size (minus overlap for next window)
        // We don't try to track exact byte position — just advance by the
        // non-overlap portion of the window and let the next window's overlap
        // re-decode any straddled frames.
        let advance = FLAC_WINDOW_BYTES.min(bytes_len);
        let new_byte_cursor = read_start + advance;
        let new_frame_cursor = frame_cursor + total_new_frames as u64;

        *self.decode_byte_cursor.borrow_mut() = new_byte_cursor;
        *self.decode_frame_cursor.borrow_mut() = new_frame_cursor;

        // Flush any remaining pending samples as a partial chunk
        if !pending_interleaved.is_empty() {
            let (mono, raw) = if channels == 1 {
                (pending_interleaved, None)
            } else {
                let mono = mix_to_mono(&pending_interleaved, channels);
                (mono, Some(pending_interleaved))
            };
            let chunk_idx = pending_start_frame / CHUNK_FRAMES as u64;
            self.cache
                .borrow_mut()
                .insert(chunk_idx, CachedChunk { mono, raw });
        }

        if total_new_frames == 0 {
            Err("No new frames decoded".into())
        } else {
            Ok(total_new_frames)
        }
    }

    fn read_head_mono(&self, start: u64, buf: &mut [f32]) -> usize {
        let start = start as usize;
        let avail = self.head_frames.saturating_sub(start);
        let n = buf.len().min(avail);
        buf[..n].copy_from_slice(&self.head_mono[start..start + n]);
        n
    }

    fn read_cached_mono(&self, start: u64, buf: &mut [f32]) -> usize {
        let end = (start + buf.len() as u64).min(self.total_frames);
        let total_to_read = (end - start) as usize;
        let mut written = 0usize;
        let mut cache = self.cache.borrow_mut();

        while written < total_to_read {
            let frame = start + written as u64;
            let chunk_idx = frame / CHUNK_FRAMES as u64;
            let offset_in_chunk = (frame % CHUNK_FRAMES as u64) as usize;
            let chunk_end_frame = ((chunk_idx + 1) * CHUNK_FRAMES as u64).min(self.total_frames);
            let avail_in_chunk = (chunk_end_frame - frame) as usize;
            let to_read = avail_in_chunk.min(total_to_read - written);

            if let Some(chunk) = cache.get(chunk_idx) {
                if offset_in_chunk + to_read <= chunk.mono.len() {
                    buf[written..written + to_read]
                        .copy_from_slice(&chunk.mono[offset_in_chunk..offset_in_chunk + to_read]);
                } else {
                    // Partial chunk — read what's available, silence the rest
                    let avail = chunk.mono.len().saturating_sub(offset_in_chunk);
                    let n = avail.min(to_read);
                    if n > 0 {
                        buf[written..written + n]
                            .copy_from_slice(&chunk.mono[offset_in_chunk..offset_in_chunk + n]);
                    }
                    for s in &mut buf[written + n..written + to_read] {
                        *s = 0.0;
                    }
                }
            } else {
                for s in &mut buf[written..written + to_read] {
                    *s = 0.0;
                }
            }
            written += to_read;
        }
        total_to_read
    }

    fn read_cached_channel(&self, ch: u32, start: u64, buf: &mut [f32]) -> usize {
        let end = (start + buf.len() as u64).min(self.total_frames);
        let total_to_read = (end - start) as usize;
        let channels = self.channels as usize;
        let mut written = 0usize;
        let mut cache = self.cache.borrow_mut();

        while written < total_to_read {
            let frame = start + written as u64;
            let chunk_idx = frame / CHUNK_FRAMES as u64;
            let offset_in_chunk = (frame % CHUNK_FRAMES as u64) as usize;
            let chunk_end_frame = ((chunk_idx + 1) * CHUNK_FRAMES as u64).min(self.total_frames);
            let avail_in_chunk = (chunk_end_frame - frame) as usize;
            let to_read = avail_in_chunk.min(total_to_read - written);

            if let Some(chunk) = cache.get(chunk_idx) {
                if let Some(raw) = &chunk.raw {
                    for i in 0..to_read {
                        let idx = (offset_in_chunk + i) * channels + ch as usize;
                        buf[written + i] = if idx < raw.len() { raw[idx] } else { 0.0 };
                    }
                } else {
                    // Mono — use mono data
                    let end_idx = (offset_in_chunk + to_read).min(chunk.mono.len());
                    let start_idx = offset_in_chunk.min(end_idx);
                    let n = end_idx - start_idx;
                    buf[written..written + n].copy_from_slice(&chunk.mono[start_idx..end_idx]);
                    for s in &mut buf[written + n..written + to_read] {
                        *s = 0.0;
                    }
                }
            } else {
                for s in &mut buf[written..written + to_read] {
                    *s = 0.0;
                }
            }
            written += to_read;
        }
        total_to_read
    }

    fn read_head_channel(&self, ch: u32, start: u64, buf: &mut [f32]) -> usize {
        let start = start as usize;
        let avail = self.head_frames.saturating_sub(start);
        let n = buf.len().min(avail);
        if let Some(raw) = &self.head_raw {
            let channels = self.channels as usize;
            for i in 0..n {
                buf[i] = raw[(start + i) * channels + ch as usize];
            }
        } else {
            buf[..n].copy_from_slice(&self.head_mono[start..start + n]);
        }
        n
    }

    /// True when `[start_frame, start_frame + len)` can be read without
    /// returning silence for missing chunks.
    pub fn region_ready(&self, start_frame: u64, len: usize) -> bool {
        let end = (start_frame + len as u64).min(self.total_frames);
        self.cache
            .borrow()
            .covers(self.head_frames as u64, start_frame, end)
    }

    /// Check if background decode has finished the entire file.
    pub fn is_fully_decoded(&self) -> bool {
        *self.decode_frame_cursor.borrow() >= self.total_frames
    }

    /// Get the current decode frame cursor value (for background decode progress).
    pub fn decode_frame_cursor_value(&self) -> u64 {
        *self.decode_frame_cursor.borrow()
    }
}

impl AudioSource for StreamingFlacSource {
    fn total_samples(&self) -> u64 {
        self.total_frames
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channel_count(&self) -> u32 {
        self.channels
    }

    fn is_fully_loaded(&self) -> bool {
        false
    }

    fn read_samples(&self, channel: ChannelView, start: u64, buf: &mut [f32]) -> usize {
        let end = (start + buf.len() as u64).min(self.total_frames);
        if end <= start {
            return 0;
        }
        let total_len = (end - start) as usize;
        let buf = &mut buf[..total_len];
        let head_end = self.head_frames as u64;

        match channel {
            ChannelView::Stereo | ChannelView::MonoMix => {
                if end <= head_end {
                    self.read_head_mono(start, buf)
                } else if start >= head_end {
                    self.read_cached_mono(start, buf)
                } else {
                    let head_part = (head_end - start) as usize;
                    self.read_head_mono(start, &mut buf[..head_part]);
                    self.read_cached_mono(head_end, &mut buf[head_part..]);
                    total_len
                }
            }
            ChannelView::Channel(ch) => {
                if ch >= self.channels {
                    for s in buf.iter_mut() {
                        *s = 0.0;
                    }
                    return total_len;
                }
                if end <= head_end {
                    self.read_head_channel(ch, start, buf)
                } else if start >= head_end {
                    self.read_cached_channel(ch, start, buf)
                } else {
                    let head_part = (head_end - start) as usize;
                    self.read_head_channel(ch, start, &mut buf[..head_part]);
                    self.read_cached_channel(ch, head_end, &mut buf[head_part..]);
                    total_len
                }
            }
            ChannelView::Difference => {
                if self.channels < 2 {
                    for s in buf.iter_mut() {
                        *s = 0.0;
                    }
                    return total_len;
                }
                let mut left = vec![0.0f32; total_len];
                let mut right = vec![0.0f32; total_len];
                self.read_samples(ChannelView::Channel(0), start, &mut left);
                self.read_samples(ChannelView::Channel(1), start, &mut right);
                for i in 0..total_len {
                    buf[i] = left[i] - right[i];
                }
                total_len
            }
        }
    }

    fn as_contiguous(&self) -> Option<&[f32]> {
        None
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ─── Streaming helpers (format-agnostic) ────────────────────────────────────

/// Load `[start, start + len)` into a streaming source's cache. No-op for
/// in-memory sources. The MP3 seek flag is left for the caller to take.
async fn load_region(source: &dyn AudioSource, start: u64, len: usize) {
    let any = source.as_any();
    if let Some(s) = any.downcast_ref::<StreamingWavSource>() {
        s.prefetch_region(start, len).await;
    } else if let Some(s) = any.downcast_ref::<StreamingFlacSource>() {
        s.prefetch_region(start, len).await;
    } else if let Some(s) = any.downcast_ref::<StreamingMp3Source>() {
        s.prefetch_region(start, len).await;
    } else if let Some(s) = any.downcast_ref::<StreamingOggSource>() {
        s.prefetch_region(start, len).await;
    } else if let Some(s) = any.downcast_ref::<StreamingM4aSource>() {
        s.prefetch_region(start, len).await;
    }
}

/// MP3 only: `Some(is_vbr)` when the last load jumped to an estimated byte
/// position instead of decoding up to it. Clears the flag.
fn take_mp3_seek(source: &dyn AudioSource) -> Option<bool> {
    let s = source.as_any().downcast_ref::<StreamingMp3Source>()?;
    if s.did_seek_skip.get() {
        s.did_seek_skip.set(false);
        Some(s.is_vbr.get())
    } else {
        None
    }
}

/// Prefetch a sample region from a streaming source (WAV, FLAC, MP3, or OGG).
/// No-op for in-memory sources.
/// Returns `(did_seek_skip, is_vbr)` for MP3 sources; `(false, false)` otherwise.
///
/// This only warms the cache: chunks can be evicted again before a later
/// `read_region`, which then returns silence for them. To read samples, use
/// [`read_region_ready`] or [`read_views_ready`] instead.
pub async fn prefetch_streaming(source: &dyn AudioSource, start: u64, len: usize) -> (bool, bool) {
    load_region(source, start, len).await;
    match take_mp3_seek(source) {
        Some(is_vbr) => (true, is_vbr),
        None => (false, false),
    }
}

/// True when a `read_region` of `[start, start + len)` made now returns real
/// samples. Streaming sources read uncached chunks as silence, and a prefetch
/// can come up short (a failed read, or a chunk evicted that the source can't
/// decode again). Always true for in-memory sources.
pub fn region_ready(source: &dyn AudioSource, start: u64, len: usize) -> bool {
    let any = source.as_any();
    if let Some(s) = any.downcast_ref::<StreamingWavSource>() {
        s.region_ready(start, len)
    } else if let Some(s) = any.downcast_ref::<StreamingFlacSource>() {
        s.region_ready(start, len)
    } else if let Some(s) = any.downcast_ref::<StreamingMp3Source>() {
        s.region_ready(start, len)
    } else if let Some(s) = any.downcast_ref::<StreamingOggSource>() {
        s.region_ready(start, len)
    } else if let Some(s) = any.downcast_ref::<StreamingM4aSource>() {
        s.region_ready(start, len)
    } else {
        true
    }
}

/// Result of [`read_views_ready`].
pub struct RegionRead {
    /// One vector per requested view, in request order. `None` when some
    /// frames inside the file could not be loaded. A region running past the
    /// end of the file still reads, with shorter vectors.
    pub views: Option<Vec<Vec<f32>>>,
    /// MP3 only: `Some(is_vbr)` when the load jumped to an estimated position
    /// instead of decoding up to it, so the samples may be slightly off in time.
    pub mp3_seek: Option<bool>,
}

impl RegionRead {
    /// The first view's samples, for a single-view read.
    pub fn into_first(self) -> Option<Vec<f32>> {
        self.views.and_then(|v| v.into_iter().next())
    }
}

/// Load `[start, start + len)` and read it for each of `views`.
///
/// The check and the reads run straight after the load with no await between
/// them, so other tasks can't evict chunks in the gap. Use this (or
/// [`read_region_ready`]) for any read of a streaming source: a plain
/// `read_region` returns silence for chunks that aren't cached.
pub async fn read_views_ready(
    source: &dyn AudioSource,
    views: &[ChannelView],
    start: u64,
    len: usize,
) -> RegionRead {
    load_region(source, start, len).await;
    let mp3_seek = take_mp3_seek(source);
    let views = region_ready(source, start, len).then(|| {
        views
            .iter()
            .map(|&cv| source.read_region(cv, start, len))
            .collect()
    });
    RegionRead { views, mp3_seek }
}

/// Load and read `[start, start + len)` for one channel view. `None` when
/// some frames inside the file could not be loaded. See [`read_views_ready`].
pub async fn read_region_ready(
    source: &dyn AudioSource,
    view: ChannelView,
    start: u64,
    len: usize,
) -> Option<Vec<f32>> {
    read_views_ready(source, &[view], start, len)
        .await
        .into_first()
}

/// Check if a source is a streaming (non-in-memory) source.
pub fn is_streaming(source: &dyn AudioSource) -> bool {
    source
        .as_any()
        .downcast_ref::<StreamingWavSource>()
        .is_some()
        || source
            .as_any()
            .downcast_ref::<StreamingFlacSource>()
            .is_some()
        || source
            .as_any()
            .downcast_ref::<StreamingMp3Source>()
            .is_some()
        || source
            .as_any()
            .downcast_ref::<StreamingOggSource>()
            .is_some()
        || source
            .as_any()
            .downcast_ref::<StreamingM4aSource>()
            .is_some()
}

/// Read a byte range from a `web_sys::File` using `File.slice()` + `FileReader`.
///
/// `start` and `end` are byte offsets (like `File.slice(start, end)`).
pub async fn read_blob_range(
    file: &web_sys::File,
    start: f64,
    end: f64,
) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let blob = file
        .slice_with_f64_and_f64(start, end)
        .map_err(|e| format!("File.slice failed: {:?}", e))?;

    let reader =
        web_sys::FileReader::new().map_err(|e| format!("FileReader::new failed: {:?}", e))?;

    let reader_clone = reader.clone();
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let resolve_clone = resolve.clone();
        let reject_clone = reject.clone();

        let onload = wasm_bindgen::closure::Closure::once(move |_: web_sys::Event| {
            resolve_clone.call0(&wasm_bindgen::JsValue::NULL).unwrap();
        });
        let onerror = wasm_bindgen::closure::Closure::once(move |_: web_sys::Event| {
            reject_clone.call0(&wasm_bindgen::JsValue::NULL).unwrap();
        });

        reader_clone.set_onloadend(Some(onload.as_ref().unchecked_ref()));
        reader_clone.set_onerror(Some(onerror.as_ref().unchecked_ref()));

        onload.forget();
        onerror.forget();
    });

    reader
        .read_as_array_buffer(&blob)
        .map_err(|e| format!("read_as_array_buffer failed: {:?}", e))?;

    JsFuture::from(promise)
        .await
        .map_err(|e| format!("FileReader await failed: {:?}", e))?;

    let result = reader
        .result()
        .map_err(|e| format!("FileReader.result() failed: {:?}", e))?;
    let array_buffer = result
        .dyn_into::<js_sys::ArrayBuffer>()
        .map_err(|_| "Expected ArrayBuffer from FileReader".to_string())?;
    let uint8 = js_sys::Uint8Array::new(&array_buffer);
    Ok(uint8.to_vec())
}

#[cfg(test)]
mod chunk_cache_tests {
    use super::*;

    fn chunk() -> CachedChunk {
        CachedChunk {
            mono: vec![0.5; CHUNK_FRAMES],
            raw: None,
        }
    }

    #[test]
    fn covers_counts_head_and_cached_chunks() {
        let mut cache = ChunkCache::new();
        let cf = CHUNK_FRAMES as u64;
        cache.insert(1, chunk());
        // Head covers chunk 0; chunk 1 cached; chunk 2 missing.
        assert!(cache.covers(cf, 0, 2 * cf));
        assert!(!cache.covers(cf, 0, 2 * cf + 1));
        assert!(!cache.covers(0, 0, cf));
        assert!(cache.covers(0, 5, 5));
    }

    #[test]
    fn held_chunk_survives_eviction_and_can_be_put_back() {
        let mut cache = ChunkCache::new();
        let held = Rc::new(chunk());
        cache.insert_rc(0, held.clone());
        // Fill past the budget so chunk 0 is evicted.
        let n = CACHE_MAX_BYTES / held.byte_size() + 1;
        for i in 1..=n as u64 {
            cache.insert(i, chunk());
        }
        assert!(!cache.contains(0));
        cache.insert_rc(0, held);
        assert!(cache.contains(0));
        assert_eq!(cache.get(0).unwrap().mono[0], 0.5);
    }
}

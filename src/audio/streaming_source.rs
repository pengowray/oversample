//! StreamingWavSource — streams PCM samples from a `web_sys::File` on demand.
//!
//! For WAV files whose decoded size exceeds a threshold (~512 MB), only the
//! header is parsed and the first 30 seconds are decoded upfront. Remaining
//! samples are decoded on demand via `File.slice()` into an LRU chunk cache.
//!
//! Consumers call `prefetch_region()` (async) before `read_samples()` (sync).
//! If a region is not cached, `read_samples()` returns silence for those frames.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::audio::loader::WavHeader;
use crate::audio::source::{AudioSource, ChannelView};

/// Number of per-channel frames per cache chunk (~256K frames).
/// At 384 kHz stereo 32-bit float, one chunk ≈ 2 MB raw + 1 MB mono = 3 MB.
const CHUNK_FRAMES: usize = 262_144;

/// Maximum cache size in bytes (128 MB).
const CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;

/// Format info extracted from the WAV header, needed for PCM decoding.
#[derive(Clone, Debug)]
struct WavFormatInfo {
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u16,
    is_float: bool,
    data_offset: u64,
    bytes_per_frame: u32, // channels * (bits_per_sample / 8)
}

/// A single cached chunk of decoded audio.
struct CachedChunk {
    /// Mono-mixed samples for this chunk.
    mono: Vec<f32>,
    /// Interleaved raw samples (if multi-channel). `None` for mono files.
    raw: Option<Vec<f32>>,
}

impl CachedChunk {
    fn byte_size(&self) -> usize {
        self.mono.len() * 4
            + self.raw.as_ref().map_or(0, |r| r.len() * 4)
    }
}

/// LRU chunk cache for decoded audio regions.
struct ChunkCache {
    /// Decoded chunks keyed by chunk index (frame / CHUNK_FRAMES).
    chunks: HashMap<u64, CachedChunk>,
    /// LRU order: most recently used at the back.
    lru: Vec<u64>,
    /// Current total memory usage in bytes.
    total_bytes: usize,
}

impl ChunkCache {
    fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            lru: Vec::new(),
            total_bytes: 0,
        }
    }

    /// Get a cached chunk, updating LRU order.
    fn get(&mut self, chunk_idx: u64) -> Option<&CachedChunk> {
        if self.chunks.contains_key(&chunk_idx) {
            // Move to back of LRU
            self.lru.retain(|&k| k != chunk_idx);
            self.lru.push(chunk_idx);
            self.chunks.get(&chunk_idx)
        } else {
            None
        }
    }

    /// Insert a chunk, evicting oldest entries if over budget.
    fn insert(&mut self, chunk_idx: u64, chunk: CachedChunk) {
        let size = chunk.byte_size();

        // Remove existing entry first to fix memory accounting on duplicate inserts
        if let Some(existing) = self.chunks.remove(&chunk_idx) {
            self.total_bytes -= existing.byte_size();
            self.lru.retain(|&k| k != chunk_idx);
        }

        // Evict until we have room
        while self.total_bytes + size > CACHE_MAX_BYTES && !self.lru.is_empty() {
            let oldest = self.lru.remove(0);
            if let Some(removed) = self.chunks.remove(&oldest) {
                self.total_bytes -= removed.byte_size();
            }
        }

        self.total_bytes += size;
        self.chunks.insert(chunk_idx, chunk);
        self.lru.push(chunk_idx);
    }

    /// Update LRU position for a cached chunk (mark as recently used).
    fn touch(&mut self, chunk_idx: u64) {
        if self.chunks.contains_key(&chunk_idx) {
            self.lru.retain(|&k| k != chunk_idx);
            self.lru.push(chunk_idx);
        }
    }

    /// Check if a chunk is cached (without updating LRU).
    fn contains(&self, chunk_idx: u64) -> bool {
        self.chunks.contains_key(&chunk_idx)
    }
}

/// Handle to the underlying file — either a browser `web_sys::File` or a native
/// path string (used via Tauri IPC `read_file_range` command).
pub enum FileHandle {
    /// Browser / webview: uses `File.slice()` + `FileReader`.
    WebFile(web_sys::File),
    /// Tauri desktop/mobile: uses native `read_file_range` IPC command.
    TauriPath(String),
}

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
    /// Chunks currently being fetched (prevents duplicate concurrent reads).
    fetching: RefCell<HashSet<u64>>,
}

// SAFETY: WASM is single-threaded; these are required by AudioSource: Send + Sync.
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
                bits_per_sample: header.bits_per_sample,
                is_float: header.is_float,
                data_offset: header.data_offset,
                bytes_per_frame: header.channels as u32 * (header.bits_per_sample as u32 / 8),
            },
            total_frames: header.total_frames,
            head_mono: Arc::new(head_mono),
            head_raw: head_raw.map(Arc::new),
            head_frames,
            cache: RefCell::new(ChunkCache::new()),
            fetching: RefCell::new(HashSet::new()),
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

        // Collect which chunks actually need fetching
        let mut to_fetch: Vec<u64> = Vec::new();
        {
            let mut cache = self.cache.borrow_mut();
            let fetching = self.fetching.borrow();
            for chunk_idx in first_chunk..=last_chunk {
                if cache.contains(chunk_idx) {
                    // Already cached — touch LRU so it doesn't get evicted
                    // while we await fetching other chunks
                    cache.touch(chunk_idx);
                } else if !fetching.contains(&chunk_idx) {
                    to_fetch.push(chunk_idx);
                }
                // else: another task is already fetching this chunk — skip
            }
        }

        // Mark chunks as in-flight
        {
            let mut fetching = self.fetching.borrow_mut();
            for &idx in &to_fetch {
                fetching.insert(idx);
            }
        }

        for chunk_idx in to_fetch {
            // Compute byte range for this chunk
            let chunk_start_frame = chunk_idx * CHUNK_FRAMES as u64;
            let chunk_end_frame = (chunk_start_frame + CHUNK_FRAMES as u64).min(self.total_frames);
            let frames_in_chunk = (chunk_end_frame - chunk_start_frame) as usize;

            let byte_start = self.info.data_offset
                + chunk_start_frame * self.info.bytes_per_frame as u64;
            let byte_len = frames_in_chunk as u64 * self.info.bytes_per_frame as u64;

            // Read raw bytes from file via the appropriate method
            let bytes = match &self.handle {
                FileHandle::WebFile(file) => {
                    read_blob_range(file, byte_start as f64, (byte_start + byte_len) as f64).await
                }
                FileHandle::TauriPath(path) => {
                    crate::tauri_bridge::read_file_range(path, byte_start, byte_len).await
                }
            };
            let bytes = match bytes {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("StreamingWavSource: prefetch chunk {} failed: {}", chunk_idx, e);
                    self.fetching.borrow_mut().remove(&chunk_idx);
                    continue;
                }
            };

            // Decode PCM bytes to f32 samples
            let interleaved = decode_pcm_bytes(&bytes, &self.info);
            let channels = self.info.channels as usize;

            let (mono, raw) = if channels == 1 {
                (interleaved, None)
            } else {
                let mono = mix_to_mono(&interleaved, channels);
                (mono, Some(interleaved))
            };

            self.cache.borrow_mut().insert(chunk_idx, CachedChunk { mono, raw });
            self.fetching.borrow_mut().remove(&chunk_idx);
        }
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

    fn read_samples(
        &self,
        channel: ChannelView,
        start: u64,
        buf: &mut [f32],
    ) -> usize {
        let end = (start + buf.len() as u64).min(self.total_frames);
        if end <= start {
            return 0;
        }
        let total_len = (end - start) as usize;
        let buf = &mut buf[..total_len];

        let head_end = self.head_frames as u64;

        match channel {
            ChannelView::MonoMix => {
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
                    for s in buf.iter_mut() { *s = 0.0; }
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
                    for s in buf.iter_mut() { *s = 0.0; }
                    return total_len;
                }
                // Read L and R separately, compute difference
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

/// Decode raw PCM bytes into interleaved f32 samples.
fn decode_pcm_bytes(bytes: &[u8], info: &WavFormatInfo) -> Vec<f32> {
    match (info.is_float, info.bits_per_sample) {
        (true, 32) => {
            bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect()
        }
        (false, 16) => {
            let max = 32768.0f32;
            bytes
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / max)
                .collect()
        }
        (false, 24) => {
            let max = 8388608.0f32; // 2^23
            bytes
                .chunks_exact(3)
                .map(|b| {
                    // Sign-extend 24-bit to 32-bit
                    let val = (b[0] as i32) | ((b[1] as i32) << 8) | ((b[2] as i32) << 16);
                    let val = if val & 0x800000 != 0 { val | !0xFFFFFF } else { val };
                    val as f32 / max
                })
                .collect()
        }
        (false, 32) => {
            let max = 2147483648.0f32; // 2^31
            bytes
                .chunks_exact(4)
                .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / max)
                .collect()
        }
        _ => {
            log::warn!(
                "Unsupported PCM format: {}bit {}",
                info.bits_per_sample,
                if info.is_float { "float" } else { "int" }
            );
            vec![0.0; bytes.len() / (info.bits_per_sample as usize / 8)]
        }
    }
}

/// Mix interleaved multi-channel samples to mono.
fn mix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    let inv = 1.0 / channels as f32;
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() * inv)
        .collect()
}

// ─── File.slice() async helper ──────────────────────────────────────────────

/// Read a byte range from a `web_sys::File` using `File.slice()` + `FileReader`.
///
/// `start` and `end` are byte offsets (like `File.slice(start, end)`).
pub async fn read_blob_range(file: &web_sys::File, start: f64, end: f64) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let blob = file
        .slice_with_f64_and_f64(start, end)
        .map_err(|e| format!("File.slice failed: {:?}", e))?;

    let reader = web_sys::FileReader::new()
        .map_err(|e| format!("FileReader::new failed: {:?}", e))?;

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

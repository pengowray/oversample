//! AudioSource trait and implementations for sample access abstraction.
//!
//! This module provides the foundation for streaming large audio files.
//! Instead of requiring all samples in memory as `Arc<Vec<f32>>`, consumers
//! can use the `AudioSource` trait to read samples on demand.
//!
//! Currently only `InMemorySource` is implemented (wrapping the existing
//! `Arc<Vec<f32>>`). Future phases will add `StreamingWavSource` for files
//! that are too large to fit in WASM memory.

use std::sync::Arc;

/// Default analysis window in seconds.
/// Whole-file analysis operations (auto-gain, wSNR, bit analysis) should
/// default to scanning only this many seconds from the start of the file,
/// unless the user explicitly requests full-file analysis.
pub const DEFAULT_ANALYSIS_WINDOW_SECS: f64 = 30.0;

/// 64-bit sample position for large file support.
/// On wasm32, `usize` is 32 bits and cannot address more than ~4 billion
/// samples. Use `SamplePos` for all global sample position arithmetic.
pub type SamplePos = u64;

/// Channel selection for multi-channel files.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ChannelView {
    /// Average all channels (current default behavior).
    #[default]
    MonoMix,
    /// A specific channel by index (0 = first/left, 1 = second/right, etc.).
    Channel(u32),
    /// Left minus Right difference (stereo only).
    Difference,
}

/// Trait abstracting sample access for audio data.
///
/// All consumers should eventually target this instead of `Arc<Vec<f32>>`.
/// During the migration period, `AudioData` carries both `samples` (legacy)
/// and `source` (new) fields.
pub trait AudioSource: Send + Sync {
    /// Total number of per-channel sample frames.
    fn total_samples(&self) -> u64;

    /// Sample rate in Hz.
    fn sample_rate(&self) -> u32;

    /// Number of channels in the original file.
    fn channel_count(&self) -> u32;

    /// Duration in seconds.
    fn duration_secs(&self) -> f64 {
        self.total_samples() as f64 / self.sample_rate() as f64
    }

    /// Read decoded f32 samples for the given channel view into `buf`.
    ///
    /// `start` is a sample-frame index (not a byte offset).
    /// Returns the number of samples actually written to `buf` (may be less
    /// than `buf.len()` if the region extends past the end of the file).
    fn read_samples(
        &self,
        channel: ChannelView,
        start: u64,
        buf: &mut [f32],
    ) -> usize;

    /// Convenience: read a region and return a Vec.
    fn read_region(&self, channel: ChannelView, start: u64, len: usize) -> Vec<f32> {
        let mut buf = vec![0.0f32; len];
        let n = self.read_samples(channel, start, &mut buf);
        buf.truncate(n);
        buf
    }

    /// Whether all samples are available in memory (small file / legacy mode).
    fn is_fully_loaded(&self) -> bool;

    /// For backward compatibility: get a direct reference to in-memory mono samples.
    /// Returns `None` for streaming sources.
    fn as_contiguous(&self) -> Option<&[f32]>;
}

/// In-memory audio source wrapping `Arc<Vec<f32>>`.
///
/// This is the zero-cost migration path: the existing mono-mixed sample
/// buffer is wrapped and exposed through the `AudioSource` trait. All
/// existing code can continue using `audio.samples` directly during the
/// transition period.
pub struct InMemorySource {
    /// Mono-mixed samples (current format, always populated).
    pub samples: Arc<Vec<f32>>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Original channel count before mono mixing.
    pub channels: u32,
}

impl std::fmt::Debug for InMemorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemorySource")
            .field("len", &self.samples.len())
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .finish()
    }
}

impl AudioSource for InMemorySource {
    fn total_samples(&self) -> u64 {
        self.samples.len() as u64
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channel_count(&self) -> u32 {
        self.channels
    }

    fn is_fully_loaded(&self) -> bool {
        true
    }

    fn read_samples(
        &self,
        channel: ChannelView,
        start: u64,
        buf: &mut [f32],
    ) -> usize {
        match channel {
            ChannelView::MonoMix => {
                let start = start as usize;
                let avail = self.samples.len().saturating_sub(start);
                let n = buf.len().min(avail);
                buf[..n].copy_from_slice(&self.samples[start..start + n]);
                n
            }
            // Future: channel extraction from raw_samples (Phase 3)
            ChannelView::Channel(_) | ChannelView::Difference => {
                // Fall back to mono mix for now
                self.read_samples(ChannelView::MonoMix, start, buf)
            }
        }
    }

    fn as_contiguous(&self) -> Option<&[f32]> {
        Some(&self.samples)
    }
}

//! Shared DSP-pipeline chunking constants.
//!
//! Used by BOTH offline export ([`super::export`]) and live streaming playback
//! ([`super::streaming_playback`]) so the two stay consistent — these were
//! previously duplicated `const`s in each module with no shared source, so a
//! change in one could silently diverge from the other.
//!
//! Note: the live-mic path ([`super::mic_backend`]) deliberately uses its own,
//! smaller warmup (tighter monitoring latency) and is NOT sourced from here.

/// Samples processed per DSP chunk (extract → mode transform → filter → gain).
pub const CHUNK_SAMPLES: usize = 96_000;

/// IIR filter warmup samples processed before each chunk's audible region, to
/// let the filter state settle and avoid startup transients.
pub const FILTER_WARMUP: usize = 4096;

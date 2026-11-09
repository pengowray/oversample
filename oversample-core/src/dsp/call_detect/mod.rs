//! Bat call detection: finds calls, sorts bat calls from other sounds, traces
//! each call's shape and groups bat calls into trains.
//!
//! Ported from the X8 bat app's `bat-dsp` crate (a `no_std` real-time
//! detector for the TASCAM Portacapture X8 at 192 kHz), where it was tuned on
//! X8 recordings and checked on 247 Xeno-canto echolocation recordings; see
//! [`classify`] for how. The steps:
//!
//! 1. [`spectrum::SpectrumAnalyzer`] makes Hann-windowed FFT columns in dB.
//! 2. [`detector::CallDetector`] finds calls against a per-bin noise floor.
//! 3. [`classify::CallClassifier`] sorts each detection: bat call, single,
//!    steady tone, broadband click, too low, or audible sound.
//! 4. [`scan::CallScanner`] runs the three over a recording, records each
//!    call's contour (its loudest frequency, column by column), and at the end
//!    groups bat calls into [`trains::CallTrain`]s.
//!
//! The original fixed the FFT at 512 points and a column every 192 samples,
//! for 192 kHz. Here [`FrameConfig::for_rate`] keeps the bin width near
//! 375 Hz and the column spacing near 1 ms at any sample rate, so the
//! detector's settings (3-bin averaging, 1.5 kHz tone width, 1 ms minimum
//! call) mean the same thing at every rate.

pub mod classify;
pub mod detector;
pub mod scan;
pub mod spectrum;
pub mod trains;

#[cfg(test)]
mod test_util;

pub use classify::{CallClassifier, CallKind, Verdict};
pub use detector::{CallDetector, CallEvent};
pub use scan::{
    scan_calls, CallScanParams, CallScanResult, CallScanner, ContourPoint, DetectedCall,
};
pub use spectrum::{Column, SpectrumAnalyzer};
pub use trains::{group_trains, CallTrain, TrainParams};

/// Names this detector and its settings in annotations it makes
/// (`auto=calls-1`). Change it when a change to the detector or its defaults
/// changes which calls it finds, so annotations from each version can be
/// told apart.
pub const DETECTOR_ID: &str = "calls-1";

/// Bin width the FFT size is chosen for, in Hz: 512 points at 192 kHz.
pub const TARGET_BIN_HZ: f64 = 375.0;
/// Column spacing the hop is chosen for, in seconds: 192 samples at 192 kHz.
pub const TARGET_HOP_S: f64 = 0.001;

/// FFT size and hop for a sample rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameConfig {
    /// FFT size: the even number nearest to sample rate / [`TARGET_BIN_HZ`].
    pub fft_size: usize,
    /// Samples between columns: sample rate × [`TARGET_HOP_S`], rounded.
    pub hop: usize,
}

impl FrameConfig {
    /// 192 kHz gives 512 and 192, as in the original; 384 kHz gives 1024 and
    /// 384; 96 kHz gives 256 and 96; 250 kHz gives 666 and 250.
    pub fn for_rate(sample_rate: u32) -> Self {
        let half = (sample_rate as f64 / TARGET_BIN_HZ / 2.0).round() as usize;
        let fft_size = (half * 2).max(spectrum::MIN_FFT_SIZE);
        let hop = ((sample_rate as f64 * TARGET_HOP_S).round() as usize).clamp(1, fft_size);
        Self { fft_size, hop }
    }

    /// Bin width in Hz at this sample rate.
    pub fn bin_hz(&self, sample_rate: u32) -> f32 {
        sample_rate as f32 / self.fft_size as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_keep_375_hz_bins_and_1_ms_columns_at_each_rate() {
        let cases = [
            (192_000, 512, 192),
            (384_000, 1024, 384),
            (96_000, 256, 96),
            (48_000, 128, 48),
            (250_000, 666, 250),
            (500_000, 1334, 500),
            (44_100, 118, 44),
        ];
        for (rate, n, hop) in cases {
            let f = FrameConfig::for_rate(rate);
            assert_eq!((f.fft_size, f.hop), (n, hop), "{rate} Hz");
            let bin = f.bin_hz(rate);
            assert!((370.0..=380.0).contains(&bin), "{rate} Hz: {bin} Hz bins");
        }
    }
}

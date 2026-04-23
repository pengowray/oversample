// SPDX-License-Identifier: GPL-3.0-only OR MIT OR Apache-2.0
//! Thin adapter over the [`resonators`] crate — Alexandre François's Resonate
//! algorithm.
//!
//! The upstream crate implements the paper faithfully; this module only
//! reshapes its output into the project's [`SpectrogramColumn`] layout and
//! scales magnitudes to match STFT brightness so existing gain / floor_db
//! controls behave identically in Spectrogram and Resonators views.
//!
//! # Layout
//!
//! For compatibility with the existing spectrogram pipeline, we build a
//! linear-frequency bank of `num_bins = fft_size / 2 + 1` resonators covering
//! 0..Nyquist with `f_k = k · (sr/2) / (num_bins - 1)`. Downstream code
//! (row→freq mapping, tile blit, freq markers) needs no special cases.
//!
//! # References
//!
//! - Algorithm: <https://alexandrefrancois.org/Resonate/>
//! - C++ reference: <https://github.com/alexandrefrancois/noFFT>
//! - Rust reference (this crate): <https://github.com/jhartquist/resonators>

use crate::types::SpectrogramColumn;
use resonators::{ResonatorBank, ResonatorConfig, alpha_from_tau};

/// Recommended warm-up samples for a given bandwidth.
///
/// Returns ≈5τ samples, where τ = 1/(2π·bandwidth) is the EMA time constant.
/// At 5τ the EMA has converged to within ~1% of steady state.
pub fn warmup_samples(sample_rate: u32, bandwidth_hz: f32) -> usize {
    let bw = bandwidth_hz.max(1.0);
    let tau_secs = 1.0 / (std::f32::consts::TAU * bw);
    (5.0 * tau_secs * sample_rate as f32).ceil().max(256.0) as usize
}

/// Compute resonator columns over a slice of audio samples.
///
/// Parameters mirror `dsp::fft::compute_stft_columns`:
/// - `fft_size` determines `num_bins = fft_size/2 + 1` (frequency resolution).
/// - `hop_size` is the output column interval in samples.
/// - `col_start`/`col_count` select which columns to emit (0-based, counted
///   from sample 0 of the input slice). A fresh bank is built per call, so
///   the caller should pre-pad with warm-up samples and pass `col_start` =
///   the warm-up column count.
///
/// `bandwidth_hz` sets per-bin EMA bandwidth (uniform across all bins).
/// Smaller ⇒ sharper bins, slower tracking.
///
/// Output magnitudes are scaled by `fft_size * 0.5` to match the one-sided
/// STFT magnitude with Hann coherent gain, so existing brightness controls
/// work the same way in both views.
pub fn compute_resonator_columns(
    samples: &[f32],
    sample_rate: u32,
    fft_size: usize,
    hop_size: usize,
    col_start: usize,
    col_count: usize,
    bandwidth_hz: f32,
) -> Vec<SpectrogramColumn> {
    let num_bins = fft_size / 2 + 1;
    if samples.is_empty() || num_bins == 0 || col_count == 0 || hop_size == 0 {
        return vec![];
    }

    let sr_f = sample_rate as f32;
    let nyq = sr_f * 0.5;
    let denom = (num_bins - 1).max(1) as f32;

    // Clamp bandwidth to a stable range and convert to the library's alpha
    // convention via tau. `alpha_from_tau(tau, sr) = 1 - exp(-dt/tau)` — the
    // library's "alpha large = fast response" is the mirror image of our
    // prior scalar implementation's "alpha large = slow", so this conversion
    // hides that difference from the caller.
    let bw = bandwidth_hz.clamp(0.1, nyq * 0.99);
    let tau = 1.0 / (std::f32::consts::TAU * bw);
    let alpha = alpha_from_tau(tau, sr_f);

    // Build one ResonatorConfig per bin. Bin 0 is nominally DC (freq=0) but
    // the library rejects freq <= 0; use a tiny positive freq so it behaves
    // as a very-low bandpass (contribution negligible for bat audio).
    //
    // beta=1.0 disables the library's second-stage output EWMA so we get a
    // single-EWMA response matching the prior hand-rolled implementation,
    // which is what the user has tuned their bandwidth slider against.
    let configs: Vec<ResonatorConfig> = (0..num_bins)
        .map(|k| {
            let f_k = (k as f32 * nyq / denom).max(0.01);
            ResonatorConfig::new(f_k, alpha, 1.0)
        })
        .collect();
    let mut bank = ResonatorBank::new(&configs, sr_f);

    // Process exactly the samples needed for col_end frames. `resonate`
    // drops any trailing samples smaller than one hop, so passing more is
    // harmless but we trim for tidiness.
    let col_end = col_start + col_count;
    let need_samples = col_end.saturating_mul(hop_size).min(samples.len());
    if need_samples < hop_size {
        return vec![];
    }
    let slice = &samples[..need_samples];

    let complex_out = bank.resonate(slice, hop_size);
    let n_frames = complex_out.len() / num_bins;
    if n_frames <= col_start {
        return vec![];
    }

    let first = col_start;
    let last = (col_start + col_count).min(n_frames);
    let mag_scale = (fft_size as f32) * 0.5;

    let mut out: Vec<SpectrogramColumn> = Vec::with_capacity(last - first);
    for frame in first..last {
        let offset = frame * num_bins;
        let mags: Vec<f32> = complex_out[offset..offset + num_bins]
            .iter()
            .map(|c| (c.re * c.re + c.im * c.im).sqrt() * mag_scale)
            .collect();
        // Library emits at the end of each hop; frame 0 = after sample hop-1.
        let time_offset = ((frame + 1) * hop_size) as f64 / sample_rate as f64;
        out.push(SpectrogramColumn { magnitudes: mags, time_offset });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pure tone should produce a peak at the matching bin.
    #[test]
    fn peak_at_tone_frequency() {
        let sr = 48_000u32;
        let fft_size = 256;
        let hop = 64;
        let num_bins = fft_size / 2 + 1;

        // 6 kHz sine, 1 s long.
        let f = 6_000.0f32;
        let samples: Vec<f32> = (0..sr as usize)
            .map(|i| (std::f32::consts::TAU * f * i as f32 / sr as f32).sin())
            .collect();

        let cols = compute_resonator_columns(&samples, sr, fft_size, hop, 0, 100, 200.0);
        assert!(!cols.is_empty());

        // Look at a column well past warm-up.
        let mid = &cols[cols.len() - 1];
        let (peak_bin, _peak_val) = mid
            .magnitudes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();

        let nyq = (sr as f32) / 2.0;
        let expected = (f / (nyq / (num_bins - 1) as f32)).round() as usize;
        assert!(
            (peak_bin as isize - expected as isize).abs() <= 1,
            "peak at bin {peak_bin}, expected {expected}"
        );
    }
}

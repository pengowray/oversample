//! Spectral subtraction noise reduction: learn a broadband noise floor from
//! a segment of audio, then attenuate frequency bins near or below that floor
//! during playback. Complements the notch filter (which targets discrete tonal
//! noise) by handling broadband hiss, wind, and ambient noise.

use crate::dsp::fft::{hann_window, plan_fft_forward, plan_fft_inverse};
use realfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

/// A learned noise floor spectrum for spectral subtraction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoiseFloor {
    /// Per-bin noise magnitude (linear scale). Length = fft_size / 2 + 1.
    /// The median across frames, scaled so that steady noise gives the same
    /// value as its mean magnitude.
    pub bin_magnitudes: Vec<f64>,
    /// FFT size used to compute this noise floor.
    pub fft_size: usize,
    /// Sample rate of the audio this was learned from.
    pub sample_rate: u32,
    /// Duration of audio analyzed (seconds).
    pub analysis_duration_secs: f64,
    /// Number of STFT frames the floor was taken from.
    pub frame_count: u64,
}

// ── Noise floor learning ────────────────────────────────────────────────────

/// Pick FFT size based on sample rate (same logic as notch detection).
fn fft_size_for_rate(sample_rate: u32) -> usize {
    if sample_rate >= 192_000 {
        8192
    } else {
        4096
    }
}

/// How much audio every caller learns a noise floor from. Long enough that
/// calls rarely fill over half of it, which is what the median needs.
pub const LEARN_SECS: f64 = 10.0;

/// Most frames the floor is taken from.
const MAX_FLOOR_FRAMES: usize = 1024;

/// Mean magnitude over median magnitude for one FFT bin of steady noise
/// (a Rayleigh distribution): sqrt(pi / 2) / sqrt(2 ln 2).
const RAYLEIGH_MEAN_OVER_MEDIAN: f64 = 1.064_467;

/// Learns the noise floor from the first `analysis_duration_secs` of `samples`.
/// The stretch may contain calls. Yields periodically via a caller-supplied future.
pub async fn learn_noise_floor_async<F, Fut>(
    samples: &[f32],
    sample_rate: u32,
    analysis_duration_secs: f64,
    yield_now: F,
) -> Option<NoiseFloor>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let fft_size = fft_size_for_rate(sample_rate);
    let hop_size = fft_size / 2;
    let num_bins = fft_size / 2 + 1;

    let max_samples = (analysis_duration_secs * sample_rate as f64) as usize;
    let analysis = &samples[..samples.len().min(max_samples)];

    if analysis.len() < fft_size {
        return None;
    }

    let window = hann_window(fft_size);

    let fft = plan_fft_forward(fft_size);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    // Long stretches are thinned evenly to MAX_FLOOR_FRAMES frames.
    let total_frames = (analysis.len() - fft_size) / hop_size + 1;
    let stride = total_frames.div_ceil(MAX_FLOOR_FRAMES);
    let kept = total_frames.div_ceil(stride);
    if kept < 2 {
        return None;
    }

    // Magnitudes of every kept frame, one run of `kept` values per bin.
    let mut mags = vec![0.0f32; kept * num_bins];
    let yield_interval = 50;

    for k in 0..kept {
        let pos = k * stride * hop_size;
        for (inp, (&s, &w)) in input
            .iter_mut()
            .zip(analysis[pos..pos + fft_size].iter().zip(window.iter()))
        {
            *inp = s * w;
        }

        fft.process(&mut input, &mut spectrum).expect("FFT failed");

        for (bin, c) in spectrum.iter().enumerate() {
            mags[bin * kept + k] = c.norm();
        }

        if (k + 1) % yield_interval == 0 {
            yield_now().await;
        }
    }

    // The median ignores calls and clicks as long as they fill fewer than half
    // the frames of a bin. A mean is pulled up by every one of them.
    let mid = kept / 2;
    let floor: Vec<f64> = mags
        .chunks_exact_mut(kept)
        .map(|bin_mags| {
            let (_, median, _) = bin_mags.select_nth_unstable_by(mid, f32::total_cmp);
            *median as f64 * RAYLEIGH_MEAN_OVER_MEDIAN
        })
        .collect();

    let count = kept as u64;
    let pos = (total_frames - 1) * hop_size + fft_size;

    let actual_duration = pos as f64 / sample_rate as f64;

    Some(NoiseFloor {
        bin_magnitudes: floor,
        fft_size,
        sample_rate,
        analysis_duration_secs: actual_duration,
        frame_count: count,
    })
}

// ── Spectral subtraction application ────────────────────────────────────────

/// Apply spectral subtraction noise reduction via overlap-add STFT.
///
/// - `noise_floor`: the learned noise floor spectrum
/// - `strength`: 0.0 = no reduction, 1.0 = full subtraction, >1.0 = over-subtraction
/// - `floor_factor`: minimum residual as a fraction of original magnitude (prevents musical noise)
/// - `harmonic_suppression`: 0.0–1.0, propagates noise floor to 2x and 3x harmonic bins
pub fn apply_spectral_subtraction(
    samples: &[f32],
    sample_rate: u32,
    noise_floor: &NoiseFloor,
    strength: f64,
    floor_factor: f64,
    harmonic_suppression: f64,
) -> Vec<f32> {
    if samples.is_empty() || strength <= 0.0 {
        return samples.to_vec();
    }

    // Skip if sample rate doesn't match the learned floor — the per-bin
    // frequencies wouldn't line up, so subtraction would be wrong. Fail SAFE
    // (return the input untouched) but warn so the no-op is diagnosable rather
    // than silent.
    if noise_floor.sample_rate != sample_rate {
        log::warn!(
            "spectral subtraction skipped: noise floor learned at {} Hz but audio is {} Hz",
            noise_floor.sample_rate,
            sample_rate
        );
        return samples.to_vec();
    }

    let fft_size = noise_floor.fft_size;
    let hop_size = fft_size / 2;
    let len = samples.len();

    let window = hann_window(fft_size);
    let num_bins = fft_size / 2 + 1;

    // Build enhanced noise floor with harmonic propagation
    let effective_floor = if harmonic_suppression > 0.0 {
        let mut enhanced = noise_floor.bin_magnitudes.clone();
        for b in 0..num_bins {
            let mag = noise_floor.bin_magnitudes[b];
            if mag < 1e-20 {
                continue;
            }
            for multiplier in [2usize, 3usize] {
                let harmonic_bin = b * multiplier;
                if harmonic_bin < num_bins {
                    enhanced[harmonic_bin] += harmonic_suppression * mag;
                }
            }
        }
        enhanced
    } else {
        noise_floor.bin_magnitudes.clone()
    };

    let (fft_fwd, fft_inv) = (plan_fft_forward(fft_size), plan_fft_inverse(fft_size));

    let mut output = vec![0.0f32; len];
    let mut window_sum = vec![0.0f32; len];

    let mut frame = fft_fwd.make_input_vec();
    let mut spectrum = fft_fwd.make_output_vec();
    let mut time_out = fft_inv.make_output_vec();

    // Start one hop early so the first samples are covered by two frames, like
    // the rest. Covered by one frame only, they are divided by a window value
    // near zero, which turns the start of every block into a loud burst.
    let in_range = |pos: isize, i: usize| {
        let idx = pos + i as isize;
        (idx >= 0 && (idx as usize) < len).then_some(idx as usize)
    };
    let mut pos = -(hop_size as isize);
    while pos < len as isize {
        // Fill windowed frame
        frame.fill(0.0);
        for (i, &w) in window.iter().enumerate() {
            if let Some(idx) = in_range(pos, i) {
                frame[i] = samples[idx] * w;
            }
        }

        // Forward FFT
        fft_fwd
            .process(&mut frame, &mut spectrum)
            .expect("FFT forward failed");

        // Spectral subtraction per bin
        for (bin, c) in spectrum.iter_mut().enumerate() {
            let mag = c.norm() as f64;
            let noise_mag = if bin < num_bins {
                effective_floor[bin] * strength
            } else {
                0.0
            };

            let clean_mag = (mag - noise_mag).max(floor_factor * mag);

            // Preserve original phase, apply cleaned magnitude
            if mag > 1e-20 {
                let scale = (clean_mag / mag) as f32;
                *c = Complex::new(c.re * scale, c.im * scale);
            }
        }

        // Inverse FFT
        fft_inv
            .process(&mut spectrum, &mut time_out)
            .expect("FFT inverse failed");

        // Normalize + overlap-add
        let norm = 1.0 / fft_size as f32;
        for i in 0..fft_size {
            if let Some(idx) = in_range(pos, i) {
                output[idx] += time_out[i] * norm * window[i];
                window_sum[idx] += window[i] * window[i];
            }
        }

        pos += hop_size as isize;
    }

    // Normalize by window sum
    for i in 0..len {
        if window_sum[i] > 1e-6 {
            output[i] /= window_sum[i];
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    fn synthetic_floor(fft_size: usize, sample_rate: u32, magnitude: f64) -> NoiseFloor {
        NoiseFloor {
            bin_magnitudes: vec![magnitude; fft_size / 2 + 1],
            fft_size,
            sample_rate,
            analysis_duration_secs: 0.5,
            frame_count: 1,
        }
    }

    /// Gaussian noise from a fixed seed (LCG + Box-Muller).
    fn noise(n: usize, seed: u64) -> Vec<f32> {
        let mut state = seed;
        let mut uniform = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 11) as f64 + 1.0) / (1u64 << 53) as f64
        };
        (0..n)
            .map(|_| {
                let (u1, u2) = (uniform(), uniform());
                ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos() * 0.01) as f32
            })
            .collect()
    }

    fn learn(samples: &[f32], sample_rate: u32) -> NoiseFloor {
        let fut = learn_noise_floor_async(samples, sample_rate, 1e9, || std::future::ready(()));
        let mut fut = std::pin::pin!(fut);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match fut.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(floor) => floor.expect("enough audio"),
            std::task::Poll::Pending => panic!("learn did not finish"),
        }
    }

    /// Mean magnitude per bin, the floor this module used before the median.
    fn mean_floor(samples: &[f32], fft_size: usize) -> Vec<f64> {
        let window = hann_window(fft_size);
        let fft = plan_fft_forward(fft_size);
        let (mut input, mut spectrum) = (fft.make_input_vec(), fft.make_output_vec());
        let mut sum = vec![0.0f64; fft_size / 2 + 1];
        let mut frames = 0.0;
        for start in (0..=samples.len() - fft_size).step_by(fft_size / 2) {
            for (i, x) in input.iter_mut().enumerate() {
                *x = samples[start + i] * window[i];
            }
            fft.process(&mut input, &mut spectrum).unwrap();
            for (acc, c) in sum.iter_mut().zip(&spectrum) {
                *acc += c.norm() as f64;
            }
            frames += 1.0;
        }
        sum.iter().map(|s| s / frames).collect()
    }

    fn avg_db_ratio(a: &[f64], b: &[f64]) -> f64 {
        // Skip DC and Nyquist, whose magnitudes are not Rayleigh distributed.
        let n = a.len() - 2;
        (1..=n).map(|i| 20.0 * (a[i] / b[i]).log10()).sum::<f64>() / n as f64
    }

    #[test]
    fn steady_noise_floor_matches_the_mean_magnitude() {
        // The strength setting was tuned against a mean floor, so steady noise
        // must still give the same level.
        let sr = 48_000;
        let samples = noise(sr as usize * 4, 1);
        let floor = learn(&samples, sr);
        let mean = mean_floor(&samples, floor.fft_size);
        let db = avg_db_ratio(&floor.bin_magnitudes, &mean);
        assert!(
            db.abs() < 0.15,
            "median floor is {db:.2} dB from the mean floor"
        );
    }

    #[test]
    fn calls_in_the_learning_stretch_do_not_raise_the_floor() {
        // A loud 12 kHz tone burst fills about a quarter of the frames. That
        // moves the median to the 67th percentile of the noise, 2 dB up.
        let sr = 48_000u32;
        let quiet = noise(sr as usize * 4, 2);
        let mut with_calls = quiet.clone();
        for (i, s) in with_calls.iter_mut().enumerate() {
            if i % 48_000 < 12_000 {
                *s += 0.5 * (std::f32::consts::TAU * 12_000.0 * i as f32 / sr as f32).sin();
            }
        }
        let clean = learn(&quiet, sr);
        let floor = learn(&with_calls, sr);
        let bin = 12_000 * floor.fft_size / sr as usize;
        let rise = 20.0 * (floor.bin_magnitudes[bin] / clean.bin_magnitudes[bin]).log10();
        assert!(rise.abs() < 4.0, "tone bin rose {rise:.1} dB");
        let mean_rise = 20.0
            * (mean_floor(&with_calls, floor.fft_size)[bin] / clean.bin_magnitudes[bin]).log10();
        assert!(
            mean_rise > 30.0,
            "the test burst should swamp a mean floor, got {mean_rise:.1} dB"
        );
    }

    #[test]
    fn long_stretches_are_thinned_to_the_frame_cap() {
        let sr = 48_000;
        let samples = noise(2048 * (MAX_FLOOR_FRAMES * 3 + 5), 3);
        let floor = learn(&samples, sr);
        assert!(floor.frame_count as usize <= MAX_FLOOR_FRAMES);
        assert!(floor.frame_count as usize > MAX_FLOOR_FRAMES / 2);
    }

    #[test]
    fn block_start_is_no_louder_than_the_middle() {
        // Playback runs this on separate blocks, so a block's first samples
        // must come out at the same level as the rest.
        let sr = 384_000u32;
        let x = noise(96_000, 4);
        let floor = learn(&x, sr);
        let out = apply_spectral_subtraction(&x, sr, &floor, 0.6, 0.05, 0.0);
        let rms = |v: &[f32]| {
            (v.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
        };
        let (start, middle) = (rms(&out[..512]), rms(&out[40_000..60_000]));
        assert!(
            start < middle * 1.5,
            "start rms {start:.5}, middle rms {middle:.5}"
        );
    }

    #[test]
    fn fft_size_for_rate_picks_8192_above_192k() {
        assert_eq!(fft_size_for_rate(44_100), 4096);
        assert_eq!(fft_size_for_rate(96_000), 4096);
        assert_eq!(fft_size_for_rate(192_000), 8192);
        assert_eq!(fft_size_for_rate(384_000), 8192);
    }

    #[test]
    fn strength_zero_is_passthrough() {
        let floor = synthetic_floor(4096, 44_100, 1.0);
        let samples = vec![0.5f32; 4096];
        let out = apply_spectral_subtraction(&samples, 44_100, &floor, 0.0, 0.1, 0.0);
        assert_eq!(out, samples);
    }

    #[test]
    fn mismatched_sample_rate_is_passthrough() {
        // Floor learned at 44.1k, audio at 48k → must not be applied.
        let floor = synthetic_floor(4096, 44_100, 1.0);
        let samples = vec![0.5f32; 4096];
        let out = apply_spectral_subtraction(&samples, 48_000, &floor, 1.0, 0.1, 0.0);
        assert_eq!(out, samples);
    }

    #[test]
    fn empty_input_is_empty_output() {
        let floor = synthetic_floor(4096, 44_100, 1.0);
        let out = apply_spectral_subtraction(&[], 44_100, &floor, 1.0, 0.1, 0.0);
        assert!(out.is_empty());
    }

    #[test]
    fn aggressive_floor_attenuates_signal() {
        // Build a floor whose magnitude exceeds the test signal magnitude in every bin —
        // the floor_factor (here 0.1) should clamp output to 10% of original.
        let sr = 44_100u32;
        let fft_size = 4096;
        let floor = synthetic_floor(fft_size, sr, 1e6); // unrealistically loud "noise" floor
        let n = fft_size * 4;
        let samples: Vec<f32> = (0..n)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr as f32).sin())
            .collect();
        let out = apply_spectral_subtraction(&samples, sr, &floor, 1.0, 0.1, 0.0);
        // Steady-state region (skip windowing transients at the head/tail).
        let head = fft_size;
        let tail = out.len() - fft_size;
        let in_rms: f64 = (samples[head..tail]
            .iter()
            .map(|&s| (s as f64).powi(2))
            .sum::<f64>()
            / (tail - head) as f64)
            .sqrt();
        let out_rms: f64 = (out[head..tail]
            .iter()
            .map(|&s| (s as f64).powi(2))
            .sum::<f64>()
            / (tail - head) as f64)
            .sqrt();
        // floor_factor=0.1 means output magnitude is capped at 10% per bin, so total
        // RMS should drop by roughly the same factor (within a few percent slack).
        assert!(
            out_rms < in_rms * 0.2,
            "spectral subtraction should attenuate; in_rms={in_rms}, out_rms={out_rms}",
        );
    }

    #[test]
    fn zero_floor_with_unity_strength_is_near_passthrough() {
        // Floor of zero everywhere means nothing to subtract — output should track input
        // closely. (Window overlap-add introduces tiny boundary effects; check the middle.)
        let sr = 44_100u32;
        let fft_size = 4096;
        let floor = synthetic_floor(fft_size, sr, 0.0);
        let n = fft_size * 4;
        let samples: Vec<f32> = (0..n)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 1500.0 * i as f32 / sr as f32).sin())
            .collect();
        let out = apply_spectral_subtraction(&samples, sr, &floor, 1.0, 0.1, 0.0);
        let head = fft_size;
        let tail = out.len() - fft_size;
        let diff_rms: f64 = (samples[head..tail]
            .iter()
            .zip(out[head..tail].iter())
            .map(|(a, b)| ((a - b) as f64).powi(2))
            .sum::<f64>()
            / (tail - head) as f64)
            .sqrt();
        assert!(
            diff_rms < 0.05,
            "expected near-passthrough, diff_rms={diff_rms}"
        );
    }
}

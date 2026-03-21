//! Spectral subtraction noise reduction: learn a broadband noise floor from
//! a segment of audio, then attenuate frequency bins near or below that floor
//! during playback. Complements the notch filter (which targets discrete tonal
//! noise) by handling broadband hiss, wind, and ambient noise.

use serde::{Serialize, Deserialize};
use realfft::num_complex::Complex;
use realfft::RealFftPlanner;
use std::cell::RefCell;
use std::collections::HashMap;

/// A learned noise floor spectrum for spectral subtraction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoiseFloor {
    /// Per-bin mean magnitude (linear scale). Length = fft_size / 2 + 1.
    pub bin_magnitudes: Vec<f64>,
    /// FFT size used to compute this noise floor.
    pub fft_size: usize,
    /// Sample rate of the audio this was learned from.
    pub sample_rate: u32,
    /// Duration of audio analyzed (seconds).
    pub analysis_duration_secs: f64,
    /// Number of STFT frames averaged.
    pub frame_count: u64,
}

// ── Thread-local caches ─────────────────────────────────────────────────────

thread_local! {
    static SS_FFT_PLANNER: RefCell<RealFftPlanner<f32>> = RefCell::new(RealFftPlanner::new());
    static SS_HANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
}

fn hann_window(size: usize) -> Vec<f32> {
    SS_HANN_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(size)
            .or_insert_with(|| {
                (0..size)
                    .map(|i| {
                        0.5 * (1.0
                            - (2.0 * std::f32::consts::PI * i as f32 / (size - 1) as f32).cos())
                    })
                    .collect()
            })
            .clone()
    })
}

// ── Noise floor learning ────────────────────────────────────────────────────

/// Pick FFT size based on sample rate (same logic as notch detection).
fn fft_size_for_rate(sample_rate: u32) -> usize {
    if sample_rate >= 192_000 { 8192 } else { 4096 }
}

/// Async version that yields to the browser to keep the UI responsive.
pub async fn learn_noise_floor_async(
    samples: &[f32],
    sample_rate: u32,
    analysis_duration_secs: f64,
) -> Option<NoiseFloor> {
    let fft_size = fft_size_for_rate(sample_rate);
    let hop_size = fft_size / 2;
    let num_bins = fft_size / 2 + 1;

    let max_samples = (analysis_duration_secs * sample_rate as f64) as usize;
    let analysis = &samples[..samples.len().min(max_samples)];

    if analysis.len() < fft_size {
        return None;
    }

    let window = hann_window(fft_size);

    let fft = SS_FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(fft_size));
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    // Welford's online mean per bin
    let mut mean = vec![0.0f64; num_bins];
    let mut count = 0u64;

    let mut pos = 0;
    let mut frame_count = 0u32;
    let yield_interval = 50;

    while pos + fft_size <= analysis.len() {
        for (inp, (&s, &w)) in input
            .iter_mut()
            .zip(analysis[pos..pos + fft_size].iter().zip(window.iter()))
        {
            *inp = s * w;
        }

        fft.process(&mut input, &mut spectrum).expect("FFT failed");

        count += 1;
        for (i, c) in spectrum.iter().enumerate() {
            let mag = c.norm() as f64;
            mean[i] += (mag - mean[i]) / count as f64;
        }

        pos += hop_size;
        frame_count += 1;
        if frame_count.is_multiple_of(yield_interval) {
            yield_to_browser().await;
        }
    }

    if count < 2 {
        return None;
    }

    let actual_duration = pos as f64 / sample_rate as f64;

    Some(NoiseFloor {
        bin_magnitudes: mean,
        fft_size,
        sample_rate,
        analysis_duration_secs: actual_duration,
        frame_count: count,
    })
}

async fn yield_to_browser() {
    use wasm_bindgen_futures::JsFuture;
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback(&resolve);
        }
    });
    let _ = JsFuture::from(promise).await;
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

    // Skip if sample rate doesn't match the learned floor
    if noise_floor.sample_rate != sample_rate {
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

    let (fft_fwd, fft_inv) = SS_FFT_PLANNER.with(|p| {
        let mut p = p.borrow_mut();
        (p.plan_fft_forward(fft_size), p.plan_fft_inverse(fft_size))
    });

    let mut output = vec![0.0f32; len];
    let mut window_sum = vec![0.0f32; len];

    let mut frame = fft_fwd.make_input_vec();
    let mut spectrum = fft_fwd.make_output_vec();
    let mut time_out = fft_inv.make_output_vec();

    let mut pos = 0;
    while pos < len {
        // Fill windowed frame
        frame.fill(0.0);
        for (i, &w) in window.iter().enumerate() {
            if pos + i < len {
                frame[i] = samples[pos + i] * w;
            }
        }

        // Forward FFT
        fft_fwd.process(&mut frame, &mut spectrum).expect("FFT forward failed");

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
        fft_inv.process(&mut spectrum, &mut time_out).expect("FFT inverse failed");

        // Normalize + overlap-add
        let norm = 1.0 / fft_size as f32;
        for i in 0..fft_size {
            if pos + i < len {
                output[pos + i] += time_out[i] * norm * window[i];
                window_sum[pos + i] += window[i] * window[i];
            }
        }

        pos += hop_size;
    }

    // Normalize by window sum
    for i in 0..len {
        if window_sum[i] > 1e-6 {
            output[i] /= window_sum[i];
        }
    }

    output
}

//! Notch noise filtering: automatic detection of persistent electronic noise
//! bands and IIR biquad band-reject filters to suppress them during playback.

use serde::{Serialize, Deserialize};

/// A single detected or manually defined noise band.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoiseBand {
    pub center_hz: f64,
    pub bandwidth_hz: f64,
    pub q: f64,
    pub enabled: bool,
    /// Estimated strength in dB above local spectral floor (informational).
    pub strength_db: f64,
}

/// A complete noise profile (importable/exportable as .batm YAML).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoiseProfile {
    pub name: String,
    pub bands: Vec<NoiseBand>,
    pub source_sample_rate: u32,
    pub created: String,
    /// Learned spectral noise floor for spectral subtraction (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_floor: Option<crate::dsp::spectral_sub::NoiseFloor>,
    /// Harmonic suppression strength (0.0–1.0). Defaults to 0 for backward compat.
    #[serde(default)]
    pub harmonic_suppression: f64,
}

// ── Biquad notch filter ─────────────────────────────────────────────────────

/// Second-order IIR biquad section state.
#[derive(Clone, Debug)]
struct BiquadState {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BiquadState {
    /// Create a band-reject (notch) filter.
    /// Coefficients from the Audio EQ Cookbook (Robert Bristow-Johnson).
    fn notch(center_hz: f64, q: f64, sample_rate: u32) -> Self {
        let w0 = 2.0 * std::f64::consts::PI * center_hz / sample_rate as f64;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();
        let a0 = 1.0 + alpha;
        BiquadState {
            b0: 1.0 / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: 1.0 / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Create a peaking EQ filter with specified gain at center frequency.
    /// Negative gain_db cuts (attenuates). Used for harmonic suppression.
    /// Coefficients from the Audio EQ Cookbook (Robert Bristow-Johnson).
    fn peaking_eq(center_hz: f64, q: f64, gain_db: f64, sample_rate: u32) -> Self {
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * center_hz / sample_rate as f64;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();
        let a0 = 1.0 + alpha / a;
        BiquadState {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let x = x as f64;
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y as f32
    }
}

/// Apply cascaded notch filters for all enabled bands.
/// When `harmonic_suppression` > 0, also attenuate 2x and 3x harmonics via peaking EQ.
pub fn apply_notch_filters(
    samples: &[f32],
    sample_rate: u32,
    bands: &[NoiseBand],
    harmonic_suppression: f64,
) -> Vec<f32> {
    let nyquist = sample_rate as f64 / 2.0;

    let mut filters: Vec<BiquadState> = bands
        .iter()
        .filter(|b| b.enabled && b.center_hz > 0.0 && b.q > 0.0
            && b.center_hz < nyquist)
        .map(|b| BiquadState::notch(b.center_hz, b.q, sample_rate))
        .collect();

    // Add harmonic suppression filters (2x and 3x center frequency)
    if harmonic_suppression > 0.0 {
        let gain_db = -48.0 * harmonic_suppression;
        for band in bands.iter().filter(|b| b.enabled && b.center_hz > 0.0 && b.q > 0.0) {
            let q = (band.q * 0.7).max(3.0);
            for multiplier in [2.0, 3.0] {
                let harmonic_hz = band.center_hz * multiplier;
                if harmonic_hz > 0.0 && harmonic_hz < nyquist {
                    filters.push(BiquadState::peaking_eq(harmonic_hz, q, gain_db, sample_rate));
                }
            }
        }
    }

    if filters.is_empty() {
        return samples.to_vec();
    }

    let mut output = samples.to_vec();
    for sample in output.iter_mut() {
        let mut s = *sample;
        for f in filters.iter_mut() {
            s = f.process(s);
        }
        *sample = s;
    }
    output
}

// ── Noise detection ─────────────────────────────────────────────────────────

use realfft::RealFftPlanner;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static NOTCH_FFT_PLANNER: RefCell<RealFftPlanner<f32>> = RefCell::new(RealFftPlanner::new());
    static NOTCH_HANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
}

fn hann_window(size: usize) -> Vec<f32> {
    NOTCH_HANN_CACHE.with(|cache| {
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

/// Configuration for noise detection.
pub struct DetectionConfig {
    /// Seconds of audio to analyze from the start.
    pub analysis_duration_secs: f64,
    /// Minimum prominence ratio (peak / local floor) to flag as noise.
    pub prominence_threshold: f64,
    /// Half-width of the median window (in bins) for spectral floor estimation.
    pub floor_half_window: usize,
    /// Minimum Q factor for detected bands.
    pub min_q: f64,
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            analysis_duration_secs: 10.0,
            prominence_threshold: 6.0, // ~15.6 dB above neighbors
            floor_half_window: 15,     // ±15 bins → 31-bin window
            min_q: 5.0,
        }
    }
}

/// Detect persistent noise bands that are significantly louder than
/// surrounding frequencies. Returns bands sorted by center frequency.
pub fn detect_noise_bands(
    samples: &[f32],
    sample_rate: u32,
    config: &DetectionConfig,
) -> Vec<NoiseBand> {
    // Pick FFT size based on sample rate for ~23–47 Hz/bin resolution
    let fft_size = if sample_rate >= 192_000 { 8192 } else { 4096 };
    let hop_size = fft_size / 2;
    let num_bins = fft_size / 2 + 1;

    // Limit analysis to first N seconds
    let max_samples = (config.analysis_duration_secs * sample_rate as f64) as usize;
    let analysis = &samples[..samples.len().min(max_samples)];

    if analysis.len() < fft_size {
        return Vec::new();
    }

    let window = hann_window(fft_size);

    let fft = NOTCH_FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(fft_size));
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    // Accumulate mean magnitude per bin (Welford's online algorithm)
    let mut mean = vec![0.0f64; num_bins];
    let mut count = 0u64;

    let mut pos = 0;
    while pos + fft_size <= analysis.len() {
        // Fill windowed frame
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
            // Online mean update
            mean[i] += (mag - mean[i]) / count as f64;
        }

        pos += hop_size;
    }

    if count < 2 {
        return Vec::new();
    }

    // Estimate spectral floor via running median of mean magnitudes
    let floor = running_median(&mean, config.floor_half_window);

    // Compute prominence: how much louder each bin is vs its local floor
    let prominence: Vec<f64> = mean
        .iter()
        .zip(floor.iter())
        .map(|(m, f)| if *f > 1e-20 { m / f } else { 0.0 })
        .collect();

    // Find peaks above threshold that are local maxima in prominence
    let freq_per_bin = sample_rate as f64 / fft_size as f64;
    let mut bands = Vec::new();

    for i in 1..prominence.len() - 1 {
        if prominence[i] >= config.prominence_threshold
            && prominence[i] > prominence[i - 1]
            && prominence[i] >= prominence[i + 1]
        {
            // Expand to -3dB points on the prominence curve
            let half_val = prominence[i] / 2.0_f64.sqrt();

            let mut lo = i;
            while lo > 0 && prominence[lo - 1] > half_val {
                lo -= 1;
            }
            let mut hi = i;
            while hi + 1 < prominence.len() && prominence[hi + 1] > half_val {
                hi += 1;
            }

            let center_hz = i as f64 * freq_per_bin;
            let bandwidth_hz = ((hi - lo + 1) as f64 * freq_per_bin).max(freq_per_bin);
            let q = (center_hz / bandwidth_hz).max(config.min_q);
            let strength_db = if floor[i] > 1e-20 {
                20.0 * (mean[i] / floor[i]).log10()
            } else {
                0.0
            };

            bands.push(NoiseBand {
                center_hz,
                bandwidth_hz,
                q,
                enabled: true,
                strength_db,
            });
        }
    }

    // Merge overlapping bands (adjacent peaks whose -3dB regions overlap)
    merge_overlapping(&mut bands);

    bands
}

fn merge_overlapping(bands: &mut Vec<NoiseBand>) {
    if bands.len() < 2 {
        return;
    }
    bands.sort_by(|a, b| a.center_hz.partial_cmp(&b.center_hz).unwrap());

    let mut merged: Vec<NoiseBand> = Vec::new();
    for band in bands.drain(..) {
        if let Some(last) = merged.last_mut() {
            let last_hi = last.center_hz + last.bandwidth_hz / 2.0;
            let band_lo = band.center_hz - band.bandwidth_hz / 2.0;
            if band_lo <= last_hi {
                // Merge: keep the stronger peak's center, expand bandwidth
                let new_lo = (last.center_hz - last.bandwidth_hz / 2.0).min(band_lo);
                let new_hi = last_hi.max(band.center_hz + band.bandwidth_hz / 2.0);
                if band.strength_db > last.strength_db {
                    last.center_hz = band.center_hz;
                    last.strength_db = band.strength_db;
                }
                last.bandwidth_hz = new_hi - new_lo;
                last.q = (last.center_hz / last.bandwidth_hz).max(last.q.min(band.q));
                continue;
            }
        }
        merged.push(band);
    }
    *bands = merged;
}

/// Running median over a window of ±half_w elements.
fn running_median(data: &[f64], half_w: usize) -> Vec<f64> {
    let n = data.len();
    let mut result = vec![0.0; n];
    for (i, res) in result.iter_mut().enumerate().take(n) {
        let start = i.saturating_sub(half_w);
        let end = (i + half_w + 1).min(n);
        let mut window: Vec<f64> = data[start..end].to_vec();
        window.sort_by(|a, b| a.partial_cmp(b).unwrap());
        *res = window[window.len() / 2];
    }
    result
}

// ── Async detection wrapper ─────────────────────────────────────────────────

use wasm_bindgen_futures::JsFuture;

async fn yield_to_browser() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback(&resolve);
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// Async version of detect_noise_bands that yields to the browser
/// between STFT frames to keep the UI responsive.
pub async fn detect_noise_bands_async(
    samples: &[f32],
    sample_rate: u32,
    config: &DetectionConfig,
) -> Vec<NoiseBand> {
    let fft_size = if sample_rate >= 192_000 { 8192 } else { 4096 };
    let hop_size = fft_size / 2;
    let num_bins = fft_size / 2 + 1;

    let max_samples = (config.analysis_duration_secs * sample_rate as f64) as usize;
    let analysis = &samples[..samples.len().min(max_samples)];

    if analysis.len() < fft_size {
        return Vec::new();
    }

    let window = hann_window(fft_size);

    let fft = NOTCH_FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(fft_size));
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

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
        return Vec::new();
    }

    let floor = running_median(&mean, config.floor_half_window);

    let prominence: Vec<f64> = mean
        .iter()
        .zip(floor.iter())
        .map(|(m, f)| if *f > 1e-20 { m / f } else { 0.0 })
        .collect();

    let freq_per_bin = sample_rate as f64 / fft_size as f64;
    let mut bands = Vec::new();

    for i in 1..prominence.len() - 1 {
        if prominence[i] >= config.prominence_threshold
            && prominence[i] > prominence[i - 1]
            && prominence[i] >= prominence[i + 1]
        {
            let half_val = prominence[i] / 2.0_f64.sqrt();

            let mut lo = i;
            while lo > 0 && prominence[lo - 1] > half_val {
                lo -= 1;
            }
            let mut hi = i;
            while hi + 1 < prominence.len() && prominence[hi + 1] > half_val {
                hi += 1;
            }

            let center_hz = i as f64 * freq_per_bin;
            let bandwidth_hz = ((hi - lo + 1) as f64 * freq_per_bin).max(freq_per_bin);
            let q = (center_hz / bandwidth_hz).max(config.min_q);
            let strength_db = if floor[i] > 1e-20 {
                20.0 * (mean[i] / floor[i]).log10()
            } else {
                0.0
            };

            bands.push(NoiseBand {
                center_hz,
                bandwidth_hz,
                q,
                enabled: true,
                strength_db,
            });
        }
    }

    merge_overlapping(&mut bands);
    bands
}

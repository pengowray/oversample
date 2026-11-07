//! Measures the time difference of a sound between two channels, which shows
//! which microphone the sound reached first.
//!
//! Uses GCC-PHAT (generalised cross-correlation with phase transform): the cross
//! spectrum is divided by its own magnitude, so every frequency bin counts
//! equally and a loud low-frequency background can't outweigh a quiet call. The
//! optional band limits the bins used to the frequency range of the call.
//!
//! The peak must be positive, so a channel wired with inverted polarity gives no
//! result.

use realfft::num_complex::Complex;
use realfft::RealFftPlanner;

/// The largest time difference searched, in seconds. 10 ms is 3.4 m of path
/// difference. Searching further adds chances of a false peak.
pub const MAX_DELAY_SECS: f64 = 0.01;

/// The longest stretch of audio measured, in seconds. This keeps the FFT short
/// enough to run on the UI thread in WASM. Callers tell the user when a selection
/// is longer than this.
pub const MAX_ANALYSIS_SECS: f64 = 2.0;

/// Speed of sound in air at 20 °C, in metres per second.
pub const SPEED_OF_SOUND_M_S: f64 = 343.0;

const MIN_SAMPLES: usize = 64;

/// The fraction of the measured audio faded at each end. Without a fade, cutting
/// both channels at the same sample spreads any strong tone or hum across the whole
/// spectrum with matching phases, and the phase transform turns that into a false
/// peak near zero delay. A fade this short keeps most of a call that fills the
/// selection.
const FADE_FRACTION: f32 = 0.1;

/// How far the peak must stand above the median correlation. In uncorrelated
/// noise the largest of about 4000 values is near 6x the median.
const MIN_PEAK_TO_MEDIAN: f32 = 8.0;

/// A peak above this fraction of the highest peak, and further from it than
/// [`RIVAL_PEAK_MIN_SEPARATION_SECS`], makes the result ambiguous. A narrow band or
/// a steady tone gives a row of near-equal peaks, one per cycle of the carrier.
const MAX_RIVAL_PEAK_RATIO: f32 = 0.85;

/// Peaks closer than this to the highest peak don't count as rivals. A call with a
/// few kHz of bandwidth always has strong peaks one carrier cycle either side of
/// the true delay (about 70 us at 14 kHz). Choosing one of those by mistake is an
/// error of a few centimetres of path difference, which doesn't change the result
/// in practice. 0.1 ms is 3.4 cm.
const RIVAL_PEAK_MIN_SEPARATION_SECS: f64 = 0.0001;

/// Returns how many seconds the sound in `right` is ahead of the sound in `left`.
/// The value is negative when `left` is ahead. Returns `None` when no single delay
/// stands out, including when a channel is silent or under 64 samples long.
///
/// `band` is the frequency range to use, in Hz. Only the first
/// [`MAX_ANALYSIS_SECS`] of the input is measured.
pub fn right_leads_by_secs(
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
    band: Option<(f64, f64)>,
) -> Option<f64> {
    let sr = sample_rate as f64;
    let len = left
        .len()
        .min(right.len())
        .min((MAX_ANALYSIS_SECS * sr) as usize);
    if len < MIN_SAMPLES {
        return None;
    }
    let left = &left[..len];
    let right = &right[..len];
    if left.iter().all(|&s| s == 0.0) || right.iter().all(|&s| s == 0.0) {
        return None;
    }

    // Padding to at least twice the length keeps the correlation from wrapping
    // around.
    let fft_len = (len * 2).next_power_of_two();
    let mut planner = RealFftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(fft_len);
    let spectrum_of = |samples: &[f32]| -> Vec<Complex<f32>> {
        let mut input = forward.make_input_vec();
        for (i, (out, &sample)) in input.iter_mut().zip(samples).enumerate() {
            *out = sample * fade_gain(i, len);
        }
        let mut output = forward.make_output_vec();
        forward
            .process(&mut input, &mut output)
            .expect("FFT buffers come from the same plan");
        output
    };
    let left_spectrum = spectrum_of(left);
    let right_spectrum = spectrum_of(right);

    let (band_low, band_high) = band.unwrap_or((0.0, sr / 2.0));
    let hz_per_bin = sr / fft_len as f64;
    let last_bin = left_spectrum.len() - 1;
    let mut cross: Vec<Complex<f32>> = left_spectrum
        .iter()
        .zip(&right_spectrum)
        .enumerate()
        .map(|(bin, (l, r))| {
            let hz = bin as f64 * hz_per_bin;
            let product = l * r.conj();
            let magnitude = product.norm();
            // DC and Nyquist are left out because the inverse real FFT needs them
            // to be purely real.
            if bin == 0
                || bin == last_bin
                || hz < band_low
                || hz > band_high
                || magnitude <= f32::EPSILON
            {
                Complex::new(0.0, 0.0)
            } else {
                product / magnitude
            }
        })
        .collect();

    let inverse = planner.plan_fft_inverse(fft_len);
    let mut correlation = inverse.make_output_vec();
    inverse
        .process(&mut cross, &mut correlation)
        .expect("FFT buffers come from the same plan");

    // Index 0 of `window` is the most negative lag. A positive lag means `left`
    // is later than `right`, so `right` leads.
    let max_lag = ((MAX_DELAY_SECS * sr) as usize).min(len - 1);
    let window: Vec<f32> = (fft_len - max_lag..fft_len)
        .chain(0..=max_lag)
        .map(|i| correlation[i])
        .collect();

    let (peak_index, &peak) = window
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))?;
    if peak <= 0.0 {
        return None;
    }

    let mut magnitudes: Vec<f32> = window.iter().map(|c| c.abs()).collect();
    let middle = magnitudes.len() / 2;
    let (_, &mut median, _) = magnitudes.select_nth_unstable_by(middle, f32::total_cmp);
    if peak < median * MIN_PEAK_TO_MEDIAN {
        return None;
    }

    let min_separation = (RIVAL_PEAK_MIN_SEPARATION_SECS * sr) as usize;
    let has_rival_peak = window.windows(3).enumerate().any(|(i, values)| {
        (i + 1).abs_diff(peak_index) > min_separation
            && values[1] > values[0]
            && values[1] >= values[2]
            && values[1] > peak * MAX_RIVAL_PEAK_RATIO
    });
    if has_rival_peak {
        return None;
    }

    let lag = peak_index as i64 - max_lag as i64;
    Some(lag as f64 / sr)
}

/// Gain at sample `i` of `len`: a raised-cosine fade at each end, 1 in the middle.
fn fade_gain(i: usize, len: usize) -> f32 {
    let fade_len = (len as f32 * FADE_FRACTION).max(1.0);
    let from_edge = i.min(len - 1 - i) as f32;
    if from_edge >= fade_len {
        1.0
    } else {
        0.5 - 0.5 * (std::f32::consts::PI * from_edge / fade_len).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 192_000;

    /// Deterministic white noise in -1..1 (xorshift32).
    fn noise(len: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as f32 / u32::MAX as f32 * 2.0 - 1.0
            })
            .collect()
    }

    /// `delay` zeros, then `signal`, cut to the original length.
    fn delayed(signal: &[f32], delay: usize) -> Vec<f32> {
        let mut out = vec![0.0; delay];
        out.extend_from_slice(&signal[..signal.len() - delay]);
        out
    }

    /// A linear down-sweep lasting the whole slice, in a silent stretch of audio.
    fn sweep(len: usize, start_hz: f64, end_hz: f64) -> Vec<f32> {
        let duration = len as f64 / SR as f64;
        let rate = (end_hz - start_hz) / duration;
        (0..len)
            .map(|i| {
                let t = i as f64 / SR as f64;
                (2.0 * std::f64::consts::PI * (start_hz * t + 0.5 * rate * t * t)).sin() as f32
            })
            .collect()
    }

    fn secs(samples: i64) -> f64 {
        samples as f64 / SR as f64
    }

    #[test]
    fn right_ahead_of_left_gives_a_positive_delay() {
        let signal = noise(20_000, 1);
        let late = delayed(&signal, 186);
        assert_eq!(
            right_leads_by_secs(&late, &signal, SR, None),
            Some(secs(186))
        );
    }

    #[test]
    fn left_ahead_of_right_gives_a_negative_delay() {
        let signal = noise(20_000, 2);
        let late = delayed(&signal, 186);
        assert_eq!(
            right_leads_by_secs(&signal, &late, SR, None),
            Some(secs(-186))
        );
    }

    #[test]
    fn identical_channels_give_zero_delay() {
        let signal = noise(20_000, 3);
        assert_eq!(right_leads_by_secs(&signal, &signal, SR, None), Some(0.0));
    }

    #[test]
    fn a_sweep_buried_in_separate_noise_is_found_within_its_band() {
        // A 16-12 kHz sweep at the same level as the noise on each channel, as a
        // faint call in a real recording would be.
        let call = sweep(9_600, 16_000.0, 12_000.0);
        let mut left = noise(9_600 + 186, 4);
        let mut right = noise(9_600 + 186, 5);
        for (i, &s) in call.iter().enumerate() {
            right[i] += s;
            left[i + 186] += s;
        }
        assert_eq!(
            right_leads_by_secs(&left, &right, SR, Some((11_000.0, 17_000.0))),
            Some(secs(186))
        );
    }

    #[test]
    fn uncorrelated_noise_gives_no_result() {
        for seed in 10..20 {
            let left = noise(20_000, seed);
            let right = noise(20_000, seed + 100);
            assert_eq!(right_leads_by_secs(&left, &right, SR, None), None);
        }
    }

    #[test]
    fn a_steady_tone_gives_no_result() {
        // Every cycle of the tone lines up equally well, so the delay is ambiguous.
        // Both channels are cut from the middle of the tone, so neither has an onset
        // that would give the delay away, and each has its own quiet background noise,
        // as any real recording does.
        let tone = sweep(20_186, 15_000.0, 15_000.0);
        let with_noise = |tone: &[f32], seed: u32| -> Vec<f32> {
            let background = noise(tone.len(), seed);
            tone.iter()
                .zip(&background)
                .map(|(t, n)| t + n * 0.01)
                .collect()
        };
        let left = with_noise(&tone[..20_000], 21);
        let right = with_noise(&tone[186..], 22);
        assert_eq!(right_leads_by_secs(&left, &right, SR, None), None);
        assert_eq!(
            right_leads_by_secs(&left, &right, SR, Some((14_000.0, 16_000.0))),
            None
        );
    }

    #[test]
    fn strong_hum_under_a_quiet_delayed_signal_does_not_pull_the_result_to_zero() {
        let hum = sweep(20_186, 1_000.0, 1_000.0);
        let quiet = noise(20_186, 11);
        let mixed: Vec<f32> = hum.iter().zip(&quiet).map(|(h, q)| h + q * 0.01).collect();
        let right = &mixed[..20_000];
        // The hum stays in step on both channels, as mains hum does, and only the
        // quiet noise is delayed.
        let left: Vec<f32> = (0..20_000)
            .map(|i| hum[i] + if i >= 186 { quiet[i - 186] * 0.01 } else { 0.0 })
            .collect();
        assert_eq!(
            right_leads_by_secs(&left, right, SR, Some((5_000.0, 90_000.0))),
            Some(secs(186))
        );
    }

    #[test]
    fn a_silent_channel_gives_no_result() {
        let signal = noise(20_000, 6);
        let silence = vec![0.0; 20_000];
        assert_eq!(right_leads_by_secs(&signal, &silence, SR, None), None);
    }

    #[test]
    fn too_few_samples_give_no_result() {
        let signal = noise(MIN_SAMPLES - 1, 7);
        assert_eq!(right_leads_by_secs(&signal, &signal, SR, None), None);
    }

    #[test]
    fn a_delay_beyond_the_search_limit_is_not_reported_as_a_match() {
        let signal = noise(40_000, 8);
        let beyond_limit = (MAX_DELAY_SECS * SR as f64) as usize + 500;
        let late = delayed(&signal, beyond_limit);
        assert_eq!(right_leads_by_secs(&late, &signal, SR, None), None);
    }

    #[test]
    fn only_the_first_two_seconds_are_measured() {
        // The first 2 s have R ahead by 100 samples; the rest has L far ahead.
        let two_secs = (MAX_ANALYSIS_SECS * SR as f64) as usize;
        let signal = noise(two_secs, 9);
        let left = delayed(&signal, 100);
        let mut left_full = left;
        let mut right_full = signal;
        let tail = noise(two_secs, 10);
        right_full.extend_from_slice(&delayed(&tail, 300));
        left_full.extend_from_slice(&tail);
        assert_eq!(
            right_leads_by_secs(&left_full, &right_full, SR, None),
            Some(secs(100))
        );
    }
}

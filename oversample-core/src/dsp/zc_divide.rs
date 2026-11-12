use crate::dsp::filters::lowpass_filter;
use crate::dsp::ultrasonic::bat_band_lo_hz;
use crate::dsp::zero_crossing::{crossings_to_hz, CROSSINGS_PER_CYCLE};

/// Bandpass-filter samples to the bat range (`band_lo_hz` to Nyquist, 15 kHz
/// for the default lowest bat frequency). Shared by both `zc_divide` and
/// `zc_rate_per_bin`.
fn bandpass_ultrasonic(samples: &[f32], sample_rate: u32, band_lo_hz: f64) -> Vec<f32> {
    // High-pass via subtracting lowpass from original
    let lp = cascaded_lp(samples, band_lo_hz, sample_rate, 4);
    let filtered: Vec<f32> = samples.iter().zip(lp.iter()).map(|(s, l)| s - l).collect();

    // Low-pass at 150 kHz (only matters if sample rate > 300 kHz)
    let nyquist = sample_rate as f64 / 2.0;
    if nyquist > 150_000.0 {
        cascaded_lp(&filtered, 150_000.0, sample_rate, 4)
    } else {
        filtered
    }
}

/// Compute an adaptive Schmitt trigger threshold from the filtered signal.
/// Uses the peak amplitude: threshold = peak * fraction, with a tiny minimum
/// so that pure silence doesn't trigger.
fn adaptive_threshold(filtered: &[f32]) -> (f32, f32) {
    let peak = filtered.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    // Trigger at 5% of peak — catches quiet calls while rejecting noise floor.
    // Minimum of 1e-5 so pure digital silence never arms.
    let threshold_high = (peak * 0.05).max(1e-5);
    let threshold_low = threshold_high * 0.4;
    (threshold_high, threshold_low)
}

/// A crossing must pass this fraction of the envelope either side of zero.
/// Low enough that a sine sampled at 192 kHz passes it every half-cycle up to
/// about 87 kHz.
const CROSSING_FRACTION: f32 = 0.15;

/// What [`CrossingDetector::step`] found at one sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Crossing {
    None,
    /// The first crossing since arming, or since a gap of more than one
    /// 15 kHz cycle. Counting starts again from here.
    Start,
    /// A crossing within one 15 kHz cycle of the one before.
    Next,
}

/// Finds the zero crossings of a call, as the comparator of an FD detector
/// does. Shared by `zc_divide` and `zc_rate_per_bin` so that the chart shows
/// the crossings that make the clicks.
///
/// - Crossings count only while armed: the envelope arms the detector when it
///   rises above `threshold_high` and disarms it when it falls below
///   `threshold_low`.
/// - A crossing counts only once the signal passes a threshold either side of
///   zero: 15% of the envelope, and at least the disarm level. This stops
///   noise riding on a call from adding crossings.
/// - A gap of more than one 15 kHz cycle between crossings restarts the
///   count. With the threshold, this stops noise in the few milliseconds
///   after a call, while the envelope decays, from making clicks.
struct CrossingDetector {
    threshold_high: f32,
    threshold_low: f32,
    /// Longest gap between crossings, in samples, that still continues a count.
    max_gap: usize,
    armed: bool,
    /// True after the signal last went above the upper threshold.
    high: bool,
    /// Sample index of the last crossing while armed. Cleared on disarm.
    last_crossing: Option<usize>,
}

impl CrossingDetector {
    /// Crossings further apart than one cycle at `band_lo_hz` (the
    /// high-pass cutoff) belong to different signals.
    fn new(filtered: &[f32], sample_rate: u32, band_lo_hz: f64) -> Self {
        let (threshold_high, threshold_low) = adaptive_threshold(filtered);
        Self {
            threshold_high,
            threshold_low,
            max_gap: (sample_rate as f64 / band_lo_hz.max(1.0)).ceil() as usize,
            armed: false,
            high: false,
            last_crossing: None,
        }
    }

    /// Process sample `i`, with value `x` and envelope `env`.
    fn step(&mut self, i: usize, x: f32, env: f32) -> Crossing {
        if env > self.threshold_high {
            self.armed = true;
        } else if env < self.threshold_low {
            self.armed = false;
            self.last_crossing = None;
        }

        let threshold = (env * CROSSING_FRACTION).max(self.threshold_low);
        let crossed = if self.high {
            x < -threshold
        } else {
            x > threshold
        };
        if !crossed {
            return Crossing::None;
        }
        self.high = !self.high;
        if !self.armed {
            return Crossing::None;
        }

        let continues = self
            .last_crossing
            .is_some_and(|last| i - last <= self.max_gap);
        self.last_crossing = Some(i);
        if continues {
            Crossing::Next
        } else {
            Crossing::Start
        }
    }
}

/// Simulate a zero-crossing frequency division bat detector.
///
/// Real FD detectors work by:
/// 1. Bandpass filtering the input to the bat range (from 1 kHz below
///    `lowest_bat_hz`, 15 kHz by default, up to 150 kHz)
/// 2. Using a Schmitt trigger (hysteresis comparator) to reject noise crossings
///    (see [`CrossingDetector`])
/// 3. Counting whole cycles of the signal (two zero crossings each)
/// 4. Outputting a short pulse every `division_factor` cycles
///
/// So a call at `f` is heard at `f / division_factor`: a 40 kHz call with a
/// division factor of 10 makes clicks at 4 kHz.
///
/// The output amplitude tracks the input envelope so that louder bat calls
/// produce louder clicks, matching the behavior of analog FD detectors.
pub fn zc_divide(
    samples: &[f32],
    sample_rate: u32,
    division_factor: u32,
    skip_bandpass: bool,
    lowest_bat_hz: f32,
) -> Vec<f32> {
    let band_lo_hz = bat_band_lo_hz(lowest_bat_hz) as f64;
    if samples.len() < 2 || division_factor == 0 {
        return vec![0.0; samples.len()];
    }

    let filtered = if skip_bandpass {
        samples.to_vec()
    } else {
        bandpass_ultrasonic(samples, sample_rate, band_lo_hz)
    };

    // Envelope follower (~1ms window)
    let env_samples = ((sample_rate as f64 * 0.001) as usize).max(1);
    let envelope = smooth_envelope(&filtered, env_samples);

    let mut detector = CrossingDetector::new(&filtered, sample_rate, band_lo_hz);

    // Each cycle has two zero crossings, so a click needs twice
    // `division_factor` of them.
    let crossings_per_click = division_factor.saturating_mul(CROSSINGS_PER_CYCLE);
    let mut output = vec![0.0f32; samples.len()];
    let mut crossing_count: u32 = 0;

    // Click duration: ~0.15ms
    let click_len = ((sample_rate as f64 * 0.00015) as usize).max(2);
    let output_gain: f32 = 0.01;

    for (i, (&x, &env)) in filtered.iter().zip(&envelope).enumerate() {
        match detector.step(i, x, env) {
            Crossing::None => {}
            Crossing::Start => crossing_count = 0,
            Crossing::Next => {
                crossing_count += 1;
                if crossing_count >= crossings_per_click {
                    crossing_count = 0;
                    let amp = (env / detector.threshold_high).min(1.0) * output_gain;
                    let end = (i + click_len).min(samples.len());
                    for (k, out_sample) in output[i..end].iter_mut().enumerate() {
                        let phase = k as f64 / click_len as f64 * std::f64::consts::PI;
                        *out_sample = phase.sin() as f32 * amp;
                    }
                }
            }
        }
    }

    cascaded_lp(&output, 12_000.0, sample_rate, 2)
}

/// Compute the zero-crossing frequency per time bin for visualization.
///
/// Returns a Vec of (frequency_hz, is_armed) per bin. The frequency is the
/// number of zero crossings per second divided by two, so a 40 kHz tone reads
/// 40 kHz.
/// `bin_duration` is in seconds (e.g. 0.001 for 1ms bins).
/// Counts the same crossings as `zc_divide`: only while armed (signal
/// present), and only those that continue a count (see [`CrossingDetector`]).
pub fn zc_rate_per_bin(
    samples: &[f32],
    sample_rate: u32,
    bin_duration: f64,
    skip_bandpass: bool,
    lowest_bat_hz: f32,
) -> Vec<(f64, bool)> {
    let band_lo_hz = bat_band_lo_hz(lowest_bat_hz) as f64;
    if samples.len() < 2 {
        return Vec::new();
    }

    let filtered = if skip_bandpass {
        samples.to_vec()
    } else {
        bandpass_ultrasonic(samples, sample_rate, band_lo_hz)
    };
    let env_samples = ((sample_rate as f64 * 0.001) as usize).max(1);
    let envelope = smooth_envelope(&filtered, env_samples);
    let mut detector = CrossingDetector::new(&filtered, sample_rate, band_lo_hz);

    let bin_samples = ((sample_rate as f64 * bin_duration) as usize).max(1);
    let num_bins = filtered.len().div_ceil(bin_samples);
    let mut bins = Vec::with_capacity(num_bins);

    let mut bin_crossings: usize = 0;
    let mut bin_armed = false;

    for i in 1..filtered.len() {
        match detector.step(i, filtered[i], envelope[i]) {
            Crossing::None => {}
            Crossing::Start => bin_armed = true,
            Crossing::Next => {
                bin_crossings += 1;
                bin_armed = true;
            }
        }

        // End of bin?
        if (i % bin_samples) == 0 || i == filtered.len() - 1 {
            let actual_bin_dur = if i == filtered.len() - 1 {
                (i % bin_samples + 1) as f64 / sample_rate as f64
            } else {
                bin_duration
            };
            bins.push((crossings_to_hz(bin_crossings, actual_bin_dur), bin_armed));
            bin_crossings = 0;
            bin_armed = false;
        }
    }

    bins
}

pub(crate) fn cascaded_lp(
    samples: &[f32],
    cutoff: f64,
    sample_rate: u32,
    passes: usize,
) -> Vec<f32> {
    let mut result = samples.to_vec();
    for _ in 0..passes {
        result = lowpass_filter(&result, cutoff, sample_rate);
    }
    result
}

pub(crate) fn smooth_envelope(samples: &[f32], window: usize) -> Vec<f32> {
    let mut env = vec![0.0f32; samples.len()];
    let attack = 1.0 / window as f32;
    let release = 1.0 / (window as f32 * 4.0);

    let mut current = 0.0f32;
    for (i, &s) in samples.iter().enumerate() {
        let abs = s.abs();
        if abs > current {
            current += attack * (abs - current);
        } else {
            current += release * (abs - current);
        }
        env[i] = current;
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::ultrasonic::DEFAULT_LOWEST_BAT_HZ;
    use std::f64::consts::PI;

    fn make_sine(freq: f64, sample_rate: u32, duration: f64) -> Vec<f32> {
        let n = (sample_rate as f64 * duration) as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * PI * freq * t).sin() as f32
            })
            .collect()
    }

    #[test]
    fn test_ultrasonic_sine_produces_clicks() {
        let sr = 192_000;
        let input: Vec<f32> = make_sine(45_000.0, sr, 0.02)
            .iter()
            .map(|s| s * 0.8)
            .collect();
        let output = zc_divide(&input, sr, 10, false, DEFAULT_LOWEST_BAT_HZ);
        assert_eq!(output.len(), input.len());

        let peak = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(peak > 0.001, "Should produce audible clicks, peak={peak}");
        assert!(peak < 0.1, "Should not be too loud, peak={peak}");
    }

    #[test]
    fn test_quiet_signal_still_detected() {
        let sr = 192_000;
        // Quiet signal at 0.005 amplitude — typical of real bat recordings
        let input: Vec<f32> = make_sine(45_000.0, sr, 0.02)
            .iter()
            .map(|s| s * 0.005)
            .collect();
        let output = zc_divide(&input, sr, 8, false, DEFAULT_LOWEST_BAT_HZ);
        let peak = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(
            peak > 0.001,
            "Quiet bat calls should still produce clicks, peak={peak}"
        );
    }

    #[test]
    fn test_silence_produces_no_output() {
        let input = vec![0.0f32; 19200];
        let output = zc_divide(&input, 192_000, 10, false, DEFAULT_LOWEST_BAT_HZ);
        let peak = output.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(peak < 0.001, "Silence should produce no clicks");
    }

    #[test]
    fn test_empty_input() {
        let output = zc_divide(&[], 192_000, 10, false, DEFAULT_LOWEST_BAT_HZ);
        assert!(output.is_empty());
    }

    #[test]
    fn test_dc_signal_no_clicks() {
        let input = vec![1.0f32; 1000];
        let output = zc_divide(&input, 44100, 10, false, DEFAULT_LOWEST_BAT_HZ);
        assert!(output.iter().all(|&s| s.abs() < 0.001));
    }

    /// Count the clicks in `output`: upward crossings of half its peak.
    fn count_clicks(output: &[f32]) -> usize {
        let half = output.iter().fold(0.0f32, |m, &s| m.max(s)) * 0.5;
        output
            .windows(2)
            .filter(|w| w[0] < half && w[1] >= half)
            .count()
    }

    #[test]
    fn test_output_is_at_tone_frequency_over_division() {
        let sr = 192_000;
        // (tone Hz, division, expected click rate Hz)
        for (freq, division, expected) in [(40_000.0, 10, 4_000.0), (30_000.0, 6, 5_000.0)] {
            let input: Vec<f32> = make_sine(freq, sr, 0.1).iter().map(|s| s * 0.5).collect();
            let output = zc_divide(&input, sr, division, false, DEFAULT_LOWEST_BAT_HZ);
            // Skip the first 2 ms while the filters settle and the envelope rises.
            let skip = (sr as f64 * 0.002) as usize;
            let clicks = count_clicks(&output[skip..]);
            let rate = clicks as f64 / ((output.len() - skip) as f64 / sr as f64);
            let error = (rate - expected).abs() / expected;
            assert!(
                error < 0.02,
                "{freq} Hz with division {division}: expected clicks at {expected} Hz, got {rate:.0} Hz"
            );
        }
    }

    #[test]
    fn test_zc_rate_bins_read_tone_frequency() {
        let sr = 192_000;
        let input: Vec<f32> = make_sine(40_000.0, sr, 0.05)
            .iter()
            .map(|s| s * 0.5)
            .collect();
        let bins = zc_rate_per_bin(&input, sr, 0.001, false, DEFAULT_LOWEST_BAT_HZ);
        // Leave out the first and last bins, which the tone only partly fills.
        let interior = &bins[1..bins.len() - 1];
        assert!(interior.iter().all(|&(_, armed)| armed));
        for &(freq, _) in interior {
            let error = (freq - 40_000.0).abs() / 40_000.0;
            assert!(
                error < 0.03,
                "a 40 kHz tone should read 40 kHz, got {freq:.0} Hz"
            );
        }
    }

    /// Uniform noise in -amp..amp from a fixed seed.
    fn make_noise(amp: f32, n: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * amp
            })
            .collect()
    }

    #[test]
    fn test_noise_after_a_call_makes_no_clicks() {
        let sr = 192_000;
        let call_len = (sr as f64 * 0.02) as usize;
        let mut input = make_noise(0.002, (sr as f64 * 0.05) as usize, 7);
        let tone = make_sine(40_000.0, sr, 0.02);
        for (x, t) in input.iter_mut().zip(&tone) {
            *x += t * 0.5;
        }
        let output = zc_divide(&input, sr, 10, false, DEFAULT_LOWEST_BAT_HZ);
        let peak = output.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.001, "the call should make clicks, peak={peak}");
        // Allow 1 ms for the last click and the output filter to die away.
        let tail_start = call_len + (sr as f64 * 0.001) as usize;
        let tail_peak = output[tail_start..]
            .iter()
            .fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(
            tail_peak < peak * 0.01,
            "noise after the call made clicks: {tail_peak} against a peak of {peak}"
        );
    }

    #[test]
    fn test_crossings_more_than_a_15khz_cycle_apart_do_not_count() {
        let sr = 192_000;
        // Skip the bandpass so the 5 kHz tone reaches the detector. Its
        // half-cycles are 19.2 samples, longer than one 15 kHz cycle (12.8).
        let input = make_sine(5_000.0, sr, 0.05);
        let output = zc_divide(&input, sr, 2, true, DEFAULT_LOWEST_BAT_HZ);
        assert!(output.iter().all(|&s| s.abs() < 1e-6));
        let bins = zc_rate_per_bin(&input, sr, 0.001, true, DEFAULT_LOWEST_BAT_HZ);
        assert!(bins.iter().all(|&(freq, _)| freq == 0.0));
        // A 10 kHz tone (9.6 samples per half-cycle) still counts.
        let input = make_sine(10_000.0, sr, 0.05);
        let output = zc_divide(&input, sr, 2, true, DEFAULT_LOWEST_BAT_HZ);
        assert!(output.iter().any(|&s| s.abs() > 0.001));
    }

    #[test]
    fn test_zc_rate_bins() {
        let sr = 192_000;
        let input: Vec<f32> = make_sine(45_000.0, sr, 0.02)
            .iter()
            .map(|s| s * 0.5)
            .collect();
        let bins = zc_rate_per_bin(&input, sr, 0.001, false, DEFAULT_LOWEST_BAT_HZ);
        assert!(!bins.is_empty());
        // Most bins should show ~45 kHz and be armed
        let armed_bins: Vec<_> = bins.iter().filter(|(_, armed)| *armed).collect();
        assert!(!armed_bins.is_empty(), "Should have armed bins");
        for &(rate, _) in &armed_bins {
            let error = (rate - 45_000.0).abs() / 45_000.0;
            assert!(error < 0.15, "Rate should be ~45kHz, got {rate:.0}");
        }
    }
}

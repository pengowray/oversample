use crate::types::ZeroCrossingResult;

/// Zero crossings in one cycle of a periodic signal: one upward, one downward.
pub const CROSSINGS_PER_CYCLE: u32 = 2;

/// Frequency in Hz of a signal that crosses zero `crossings` times in
/// `duration_secs`. Returns 0 for a zero or negative duration.
pub fn crossings_to_hz(crossings: usize, duration_secs: f64) -> f64 {
    if duration_secs > 0.0 {
        crossings as f64 / (CROSSINGS_PER_CYCLE as f64 * duration_secs)
    } else {
        0.0
    }
}

/// Estimate the dominant frequency of a signal window by counting zero crossings.
///
/// Works well for narrowband signals like bat echolocation calls where energy
/// is concentrated around a single frequency.
///
/// Each full cycle of a sine wave produces two zero crossings (one up, one down).
/// Therefore: frequency = crossings / (2 * duration).
pub fn zero_crossing_frequency(samples: &[f32], sample_rate: u32) -> ZeroCrossingResult {
    if samples.len() < 2 {
        return ZeroCrossingResult {
            estimated_frequency_hz: 0.0,
            crossing_count: 0,
            duration_secs: 0.0,
        };
    }

    let mut crossings: usize = 0;

    for window in samples.windows(2) {
        // A crossing occurs when the sign changes between consecutive samples.
        // Treat exact zero as positive to avoid double-counting.
        let prev_positive = window[0] >= 0.0;
        let curr_positive = window[1] >= 0.0;

        if prev_positive != curr_positive {
            crossings += 1;
        }
    }

    let duration_secs = (samples.len() - 1) as f64 / sample_rate as f64;
    let estimated_frequency_hz = crossings_to_hz(crossings, duration_secs);

    ZeroCrossingResult {
        estimated_frequency_hz,
        crossing_count: crossings,
        duration_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn test_known_sine_wave() {
        let sample_rate = 192_000u32;
        let freq = 45_000.0f64; // 45 kHz bat call
        let duration = 0.01; // 10ms window
        let num_samples = (sample_rate as f64 * duration) as usize;

        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * PI * freq * t).sin() as f32
            })
            .collect();

        let result = zero_crossing_frequency(&samples, sample_rate);

        let error = (result.estimated_frequency_hz - freq).abs() / freq;
        assert!(
            error < 0.01,
            "Expected ~{freq} Hz, got {} Hz",
            result.estimated_frequency_hz
        );
    }

    #[test]
    fn test_dc_signal_returns_zero() {
        let samples = vec![1.0f32; 1000];
        let result = zero_crossing_frequency(&samples, 44100);
        assert_eq!(result.crossing_count, 0);
        assert_eq!(result.estimated_frequency_hz, 0.0);
    }

    #[test]
    fn test_empty_input() {
        let result = zero_crossing_frequency(&[], 44100);
        assert_eq!(result.estimated_frequency_hz, 0.0);
    }

    #[test]
    fn test_single_sample() {
        let result = zero_crossing_frequency(&[0.5], 44100);
        assert_eq!(result.estimated_frequency_hz, 0.0);
    }
}

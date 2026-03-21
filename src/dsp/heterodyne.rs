use crate::dsp::filters::lowpass_filter;
use std::f64::consts::PI;

/// Simulate a heterodyne bat detector by mixing (multiplying) the input signal
/// with a local oscillator, then low-pass filtering to extract the difference
/// frequency. This shifts ultrasonic frequencies down into the audible range.
///
/// The heterodyne principle: if the input signal has frequency f_in and the
/// local oscillator has frequency f_lo, multiplication produces two components:
///   - f_in + f_lo  (sum, removed by low-pass filter)
///   - |f_in - f_lo| (difference, the audible output)
pub fn heterodyne_mix(samples: &[f32], sample_rate: u32, lo_freq: f64, cutoff_hz: f64) -> Vec<f32> {
    let sr = sample_rate as f64;
    let angular_freq = 2.0 * PI * lo_freq;

    // Step 1: Generate local oscillator and multiply with input (ring modulation)
    let mixed: Vec<f32> = samples
        .iter()
        .enumerate()
        .map(|(i, &sample)| {
            let t = i as f64 / sr;
            let lo_sample = (angular_freq * t).cos() as f32;
            sample * lo_sample
        })
        .collect();

    // Step 2: Cascaded low-pass filter to remove the sum frequency component.
    // 4 passes of a single-pole IIR gives -24 dB/octave rolloff
    // (equivalent to a 4th-order Butterworth).
    let mut filtered = mixed;
    for _ in 0..4 {
        filtered = lowpass_filter(&filtered, cutoff_hz, sample_rate);
    }
    filtered
}

/// Stateful real-time heterodyne processor for live mic monitoring.
/// Maintains oscillator phase and cascaded LP filter states between
/// consecutive audio buffers to avoid clicks and transients.
pub struct RealtimeHet {
    phase: f64,
    lp_state: [f32; 4],
}

impl Default for RealtimeHet {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeHet {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            lp_state: [0.0; 4],
        }
    }

    /// Process `input` through heterodyne (ring modulation + 4-pass LP) and
    /// write result into `output`. Both slices must have the same length.
    pub fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        sample_rate: u32,
        lo_freq: f64,
        cutoff_hz: f64,
    ) {
        let sr = sample_rate as f64;
        let phase_inc = 2.0 * PI * lo_freq / sr;
        let dt = 1.0 / sr;
        let rc = 1.0 / (2.0 * PI * cutoff_hz);
        let alpha = (dt / (rc + dt)) as f32;

        for (i, &sample) in input.iter().enumerate() {
            // Ring modulation with continuous phase
            let lo = (self.phase + phase_inc * i as f64).cos() as f32;
            let mut val = sample * lo;

            // 4-pass cascaded single-pole LP filter
            for s in self.lp_state.iter_mut() {
                val = alpha * val + (1.0 - alpha) * *s;
                *s = val;
            }

            output[i] = val;
        }

        // Advance phase, keep in [0, 2π) to avoid precision loss
        self.phase = (self.phase + phase_inc * input.len() as f64) % (2.0 * PI);
    }

    /// Reset state (call when HET params change significantly or mic restarts)
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.lp_state = [0.0; 4];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heterodyne_shifts_frequency_down() {
        let sample_rate = 192_000u32;
        let input_freq = 45_000.0; // 45 kHz bat call
        let lo_freq = 44_000.0; // Tuned to 44 kHz
        // Expected difference: 1 kHz (audible)

        let duration = 0.05; // 50ms
        let num_samples = (sample_rate as f64 * duration) as usize;

        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * std::f64::consts::PI * input_freq * t).sin() as f32
            })
            .collect();

        let output = heterodyne_mix(&input, sample_rate, lo_freq, 15_000.0);
        assert_eq!(output.len(), input.len());

        // Verify the output has energy (is not all zeros)
        let rms: f64 = (output.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
            / output.len() as f64)
            .sqrt();
        assert!(
            rms > 0.01,
            "Output should have significant energy, got RMS={rms}"
        );

        // Use zero-crossing on the output to verify it's near 1 kHz
        let zc = crate::dsp::zero_crossing::zero_crossing_frequency(&output, sample_rate);
        let error = (zc.estimated_frequency_hz - 1000.0).abs();
        assert!(
            error < 200.0,
            "Expected ~1000 Hz difference tone, got {} Hz",
            zc.estimated_frequency_hz
        );
    }

    #[test]
    fn test_heterodyne_empty_input() {
        let output = heterodyne_mix(&[], 192_000, 45_000.0, 15_000.0);
        assert!(output.is_empty());
    }
}

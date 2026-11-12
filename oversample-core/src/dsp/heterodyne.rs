use crate::dsp::filters::Butterworth;
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

    // Step 2: low-pass to remove the sum frequency, keeping the difference
    // tone flat up to near `cutoff_hz` (-3 dB there, 48 dB per octave above).
    let mut filtered = mixed;
    Butterworth::lowpass(HET_LP_ORDER, sample_rate as f32, cutoff_hz as f32)
        .process_in_place(&mut filtered);
    filtered
}

/// Order of the Butterworth low-pass after the mixer. 8th order, so with a
/// comb of carriers spaced 2 × cutoff a call close to one carrier leaks
/// through its neighbour at about -16 dB (4th order: -9 dB), while a call
/// midway between two still comes through both at -3 dB.
const HET_LP_ORDER: usize = 8;

/// Stateful real-time heterodyne processor for live mic monitoring.
/// Maintains oscillator phase and cascaded LP filter states between
/// consecutive audio buffers to avoid clicks and transients.
pub struct RealtimeHet {
    phase: f64,
    /// The low-pass and the (sample rate, cutoff) it is tuned for. Kept
    /// across buffers; a new cutoff retunes it without clearing its memory.
    lp: Option<(Butterworth, u32, f64)>,
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
            lp: None,
        }
    }

    /// Process `input` through heterodyne (ring modulation + Butterworth
    /// low-pass) and write the result into `output`. Both slices must have the
    /// same length.
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
        let lp = match &mut self.lp {
            Some((lp, rate, cutoff)) if *rate == sample_rate => {
                if *cutoff != cutoff_hz {
                    lp.retune_lowpass(HET_LP_ORDER, sample_rate as f32, cutoff_hz as f32);
                    *cutoff = cutoff_hz;
                }
                lp
            }
            _ => {
                let lp = Butterworth::lowpass(HET_LP_ORDER, sample_rate as f32, cutoff_hz as f32);
                &mut self.lp.insert((lp, sample_rate, cutoff_hz)).0
            }
        };

        for (i, &sample) in input.iter().enumerate() {
            // Ring modulation with continuous phase
            let lo = (self.phase + phase_inc * i as f64).cos() as f32;
            output[i] = lp.process(sample * lo);
        }

        // Advance phase, keep in [0, 2π) to avoid precision loss
        self.phase = (self.phase + phase_inc * input.len() as f64) % (2.0 * PI);
    }

    /// Reset state (call when HET params change significantly or mic restarts)
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.lp = None;
    }
}

/// Comb heterodyne — mix the input against N evenly-spaced carriers and sum
/// the (lowpass-filtered) difference tones. Lets the user cover a wide
/// ultrasonic range without retuning, at the cost of overlapping difference
/// images when the carrier spacing is tighter than 2× cutoff. Spacing equal
/// to roughly 2× cutoff gives near-seamless coverage with minimal overlap.
///
/// The bands are summed without scaling: a call heard through one carrier
/// plays as loud as with a single carrier. (Dividing by N made it N times
/// quieter, -14 dB with 5 carriers.) Noise rises with the bandwidth covered,
/// as it would on a detector tuned to more of the spectrum.
pub fn heterodyne_comb_mix(
    samples: &[f32],
    sample_rate: u32,
    carrier_freqs: &[f64],
    cutoff_hz: f64,
) -> Vec<f32> {
    if carrier_freqs.is_empty() || samples.is_empty() {
        return samples.to_vec();
    }
    let mut acc = vec![0.0f32; samples.len()];
    for &lo in carrier_freqs {
        let band = heterodyne_mix(samples, sample_rate, lo, cutoff_hz);
        for (a, b) in acc.iter_mut().zip(band.iter()) {
            *a += *b;
        }
    }
    acc
}

/// Stateful real-time comb heterodyne processor. Tracks a phase and low-pass
/// state per carrier and sums the resulting difference tones, unscaled like
/// [`heterodyne_comb_mix`].
pub struct RealtimeCombHet {
    /// Per-carrier oscillator phase and cascaded LP state.
    voices: Vec<RealtimeHet>,
}

impl Default for RealtimeCombHet {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeCombHet {
    pub fn new() -> Self {
        Self { voices: Vec::new() }
    }

    /// Resize the voice bank to match the requested carrier count. New voices
    /// start with zero phase; existing voices keep their state to avoid
    /// clicks when the count changes during monitoring.
    fn ensure_n(&mut self, n: usize) {
        if self.voices.len() < n {
            self.voices.resize_with(n, RealtimeHet::new);
        } else if self.voices.len() > n {
            self.voices.truncate(n);
        }
    }

    pub fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        sample_rate: u32,
        carrier_freqs: &[f64],
        cutoff_hz: f64,
    ) {
        let n = carrier_freqs.len();
        if n == 0 || input.is_empty() {
            for o in output.iter_mut() {
                *o = 0.0;
            }
            return;
        }
        self.ensure_n(n);

        // Process the first carrier into the output buffer, then accumulate
        // the rest. Avoids a separate zero-fill pass.
        let mut scratch = vec![0.0f32; input.len()];
        self.voices[0].process(input, output, sample_rate, carrier_freqs[0], cutoff_hz);
        for (i, &lo) in carrier_freqs.iter().enumerate().skip(1) {
            self.voices[i].process(input, &mut scratch, sample_rate, lo, cutoff_hz);
            for (o, s) in output.iter_mut().zip(scratch.iter()) {
                *o += *s;
            }
        }
    }

    pub fn reset(&mut self) {
        for v in &mut self.voices {
            v.reset();
        }
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
        let rms: f64 =
            (output.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / output.len() as f64).sqrt();
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

    fn tone(freq: f64, sr: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / sr as f64).sin() as f32)
            .collect()
    }

    fn peak_db(y: &[f32]) -> f64 {
        20.0 * (y.iter().fold(0.0f32, |p, s| p.max(s.abs())) as f64).log10()
    }

    #[test]
    fn het_passband_is_flat_up_to_near_the_cutoff() {
        // The mixer halves the amplitude (-6 dB); the low-pass adds almost
        // nothing up to 2/3 of the cutoff, and -3 dB at it.
        let sr = 384_000;
        let n = sr as usize / 4;
        for (beat, lo_db, hi_db) in [
            (2_000.0, -6.3, -5.8),
            (10_000.0, -6.5, -5.8),
            (15_000.0, -9.5, -8.5),
        ] {
            let y = heterodyne_mix(&tone(45_000.0 + beat, sr, n), sr, 45_000.0, 15_000.0);
            let db = peak_db(&y[n / 4..]);
            assert!((lo_db..hi_db).contains(&db), "{beat} Hz beat: {db:.2} dB");
        }
    }

    #[test]
    fn realtime_het_matches_one_shot_across_buffers() {
        let sr = 384_000;
        let x = tone(47_000.0, sr, 8_192);
        let one_shot = heterodyne_mix(&x, sr, 45_000.0, 15_000.0);
        let mut rt = RealtimeHet::new();
        let mut out = vec![0.0; x.len()];
        for (input, output) in x.chunks(1_000).zip(out.chunks_mut(1_000)) {
            rt.process(input, output, sr, 45_000.0, 15_000.0);
        }
        let worst = one_shot
            .iter()
            .zip(&out)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "largest difference {worst}");
    }

    #[test]
    fn test_heterodyne_empty_input() {
        let output = heterodyne_mix(&[], 192_000, 45_000.0, 15_000.0);
        assert!(output.is_empty());
    }

    #[test]
    fn test_comb_heterodyne_picks_up_tone_near_any_carrier() {
        // 3 carriers spread across 30/50/70 kHz with a 6 kHz lowpass cutoff.
        // A 53 kHz tone is 3 kHz from the 50 kHz carrier (inside the band)
        // and 17/23 kHz from the others (well outside).
        let sample_rate = 192_000u32;
        let input_freq = 53_000.0;
        let carriers = [30_000.0, 50_000.0, 70_000.0];
        let cutoff = 6_000.0;

        let duration = 0.05;
        let num_samples = (sample_rate as f64 * duration) as usize;
        let input: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * std::f64::consts::PI * input_freq * t).sin() as f32
            })
            .collect();

        let output = heterodyne_comb_mix(&input, sample_rate, &carriers, cutoff);
        assert_eq!(output.len(), input.len());

        // Skip the LP startup transient before measuring RMS / ZC.
        let warmup = (sample_rate as f64 * 0.005) as usize;
        let tail = &output[warmup..];

        let rms: f64 =
            (tail.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        assert!(
            rms > 0.005,
            "Comb output should pick up the tone, got RMS={rms}"
        );

        let zc = crate::dsp::zero_crossing::zero_crossing_frequency(tail, sample_rate);
        let error = (zc.estimated_frequency_hz - 3000.0).abs();
        assert!(
            error < 400.0,
            "Expected ~3000 Hz difference tone from the 50 kHz carrier, got {} Hz",
            zc.estimated_frequency_hz
        );
    }

    #[test]
    fn comb_keeps_a_tone_at_its_single_carrier_level() {
        // A 53 kHz tone with carriers at 30/50/70 kHz: only the 50 kHz band
        // hears it, and it plays as loud as with that carrier alone.
        let sr = 192_000;
        let n = sr as usize / 4;
        let x = tone(53_000.0, sr, n);
        let single = peak_db(&heterodyne_mix(&x, sr, 50_000.0, 6_000.0)[n / 4..]);
        let comb_out = heterodyne_comb_mix(&x, sr, &[30_000.0, 50_000.0, 70_000.0], 6_000.0);
        let comb = peak_db(&comb_out[n / 4..]);
        assert!(
            (comb - single).abs() < 0.5,
            "comb {comb:.2} dB vs single {single:.2} dB"
        );
    }

    #[test]
    fn neighbouring_carrier_leaks_little() {
        // Carriers 30 kHz apart with a 15 kHz cutoff (Auto's spacing). A
        // 50 kHz call is 11 kHz from the 61 kHz carrier and 19 kHz from the
        // 31 kHz one; the copy through the farther carrier is >= 15 dB down.
        let sr = 384_000;
        let n = sr as usize / 4;
        let x = tone(50_000.0, sr, n);
        let near = peak_db(&heterodyne_mix(&x, sr, 61_000.0, 15_000.0)[n / 4..]);
        let far = peak_db(&heterodyne_mix(&x, sr, 31_000.0, 15_000.0)[n / 4..]);
        assert!(near - far >= 15.0, "near {near:.1} dB, far {far:.1} dB");
    }

    #[test]
    fn test_comb_heterodyne_empty_carriers_is_passthrough() {
        let input = vec![0.5f32, -0.5, 0.25, -0.25];
        let output = heterodyne_comb_mix(&input, 48_000, &[], 5_000.0);
        assert_eq!(output, input);
    }
}

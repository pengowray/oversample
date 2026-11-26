//! True peak: the highest level of the waveform between the samples, not
//! only at them.
//!
//! A recording clipped at 0 dBFS has samples at full scale, but the waveform
//! through them goes higher (up to about +4 dB in the bat demo recordings).
//! Playback modes that read between the samples, such as pitch shift, play
//! those higher values, and the browser's resampler does the same in 1:1.
//!
//! The `ebur128` crate (a port of libebur128) measures true peak by
//! oversampling, but stops oversampling at sample rates of 192 kHz and above,
//! where bat recordings are. So this module interpolates with the pitch
//! shifter's windowed-sinc kernel, at 4 points between each pair of samples.

use super::pitch_shift::{interpolate, KERNEL_HALF};

/// Points measured between each pair of samples, counting the sample itself
/// (4, as in ITU-R BS.1770). 8 read the three clipped demo recordings at most
/// 0.04 dB higher, at twice the cost.
const STEPS: usize = 4;

/// A gap between two samples is measured only when one of them is at least
/// this fraction (-3 dB) of the sample peak. On full-band and clipped noise
/// this gives the same result as measuring every gap
/// (`skipping_quiet_gaps_changes_nothing`), and it makes full-scale noise
/// about 6 times faster to measure: 8.5 ms for 96,000 samples, native.
const GATE: f32 = 0.7;

/// The true peak of `samples`, as a linear amplitude: at least the sample
/// peak. Gaps within `KERNEL_HALF` samples of either end are not
/// interpolated, so a buffer cut from a longer signal gets no false peaks
/// at its edges.
pub fn true_peak(samples: &[f32]) -> f32 {
    true_peak_gated(samples, GATE)
}

fn true_peak_gated(samples: &[f32], gate: f32) -> f32 {
    let sample_peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if sample_peak == 0.0 || samples.len() < 2 * KERNEL_HALF + 1 {
        return sample_peak;
    }
    let floor = sample_peak * gate;
    let mut peak = sample_peak;
    // `interpolate` reads KERNEL_HALF samples on each side of the gap.
    for i in (KERNEL_HALF - 1)..(samples.len() - KERNEL_HALF) {
        if samples[i].abs() < floor && samples[i + 1].abs() < floor {
            continue;
        }
        for k in 1..STEPS {
            let v = interpolate(samples, i as f64 + k as f64 / STEPS as f64).abs();
            peak = peak.max(v);
        }
    }
    peak
}

/// [`true_peak`] in dBFS; `None` for digital silence.
pub fn true_peak_db(samples: &[f32]) -> Option<f64> {
    let p = true_peak(samples);
    (p > 1e-10).then(|| 20.0 * (p as f64).log10())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    fn db(x: f32) -> f32 {
        20.0 * x.log10()
    }

    /// A tone at a quarter of the sample rate, scaled to a sample peak of 1.
    fn quarter_rate_tone(phase: f32) -> Vec<f32> {
        let raw: Vec<f32> = (0..4000).map(|i| (FRAC_PI_2 * i as f32 + phase).sin()).collect();
        let p = raw.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        raw.iter().map(|v| v / p).collect()
    }

    /// Sampled at its crests, the tone's true peak is its sample peak.
    /// Sampled 45 degrees later, it is 3 dB higher.
    #[test]
    fn a_tone_sampled_between_its_crests_reads_3_db_over_its_samples() {
        let at_crests = db(true_peak(&quarter_rate_tone(0.0)));
        let between = db(true_peak(&quarter_rate_tone(FRAC_PI_4)));
        assert!(at_crests.abs() < 0.05, "at crests: {at_crests:+.3} dB");
        assert!((between - 3.01).abs() < 0.1, "between crests: {between:+.3} dB");
    }

    #[test]
    fn silence_reads_zero() {
        assert_eq!(true_peak(&[0.0; 1000]), 0.0);
        assert_eq!(true_peak_db(&[0.0; 1000]), None);
        assert_eq!(true_peak(&[]), 0.0);
    }

    /// Too short to interpolate: the sample peak.
    #[test]
    fn a_buffer_shorter_than_the_kernel_reads_its_sample_peak() {
        let x = [0.1, -0.7, 0.3, 0.5];
        assert_eq!(true_peak(&x), 0.7);
    }

    /// Simple LCG noise, so the test needs no random-number crate.
    fn noise(len: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
            })
            .collect()
    }

    /// Skipping quiet gaps gives the same answer as measuring every gap, on
    /// full-band noise and on noise clipped at full scale.
    #[test]
    fn skipping_quiet_gaps_changes_nothing() {
        for seed in 1..6 {
            let n = noise(20_000, seed);
            let clipped: Vec<f32> = n.iter().map(|v| (v * 3.0).clamp(-1.0, 1.0)).collect();
            for x in [&n, &clipped] {
                let gated = true_peak(x);
                let full = true_peak_gated(x, 0.0);
                assert_eq!(gated, full, "seed {seed}");
                assert!(gated >= x.iter().fold(0.0f32, |m, v| m.max(v.abs())));
            }
        }
    }

    /// A loud spike at the very edge of a buffer is read at its sample value:
    /// there is not enough signal around it to interpolate.
    #[test]
    fn edges_read_their_sample_values() {
        let mut x = vec![0.0f32; 200];
        x[0] = 1.0;
        x[199] = -1.0;
        assert_eq!(true_peak(&x), 1.0);
    }
}

//! Detect the *effective* Nyquist frequency of a recording — the
//! highest frequency at which the file actually carries signal energy.
//!
//! Catches:
//! - **Upsampled** files: the spectrum is essentially silent above the
//!   original Nyquist (interpolation noise floor only).
//! - **Aggressive anti-aliasing** in the recorder: the spectrum rolls
//!   off well before the file's claimed Nyquist.
//!
//! Doesn't catch oversampled-then-decimated recordings (the output
//! spectrum is fully populated up to the output Nyquist).

use crate::dsp::psd::{self, PsdResult};

#[derive(Clone, Debug, PartialEq)]
pub enum EffectiveNyquistVerdict {
    /// Spectrum is populated all the way to the file's Nyquist.
    /// `claimed_nyquist_hz` is `sample_rate / 2`.
    FullBandwidth { claimed_nyquist_hz: f64 },
    /// Spectrum drops off well below the claimed Nyquist. `effective_hz`
    /// is the highest frequency with meaningful energy.
    BandLimited {
        claimed_nyquist_hz: f64,
        effective_hz: f64,
        /// Drop-off ratio: effective / claimed. < 0.95 triggers this verdict.
        ratio: f64,
    },
    /// Recording too short / format not handled.
    NotApplicable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveNyquistResult {
    pub verdict: EffectiveNyquistVerdict,
    pub explanation: String,
    pub claimed_nyquist_hz: f64,
    pub effective_hz: f64,
    /// In-band (low-frequency) noise floor in dB.
    pub low_band_floor_db: f64,
    /// Above-effective-Nyquist noise floor in dB.
    pub upper_band_floor_db: f64,
    /// Drop-off in dB between low-band and upper-band floors.
    pub drop_db: f64,
}

/// Number of equal-width bands the spectrum is divided into for the
/// per-band peak scan.
const N_BANDS: usize = 32;
/// A band is considered "populated" if its peak power is within this
/// many dB of the loudest band's peak.
const POPULATED_GAP_DB: f64 = 50.0;
/// Search NFFT for the PSD.
const NFFT: usize = 4096;
/// Treat anything below this ratio as band-limited.
const BAND_LIMITED_RATIO: f64 = 0.95;
/// How far above the samples' rounding noise a band's level must be to count
/// as real (analog) noise rather than rounding noise alone.
const ROUNDING_NOISE_MARGIN_DB: f64 = 6.0;

/// PSD level (dB/Hz, as [`psd::compute_psd`] gives it) of the rounding noise
/// of `bits`-bit samples at `sample_rate`: white noise of RMS step/sqrt(12),
/// spread over 0 to Nyquist. A band at this level holds nothing but rounding.
pub fn rounding_noise_db(bits: u16, sample_rate: u32) -> f64 {
    let step = 2f64.powi(1 - bits as i32);
    20.0 * (step / 12f64.sqrt()).log10() + 10.0 * (2.0 / sample_rate as f64).log10()
}

/// The integer resolution that sets a file's rounding noise: its PCM depth,
/// at most 22 bits, 22 for float files, and 16 for lossy files, whose
/// decoders give no PCM depth. 22, not 24: float arithmetic and window
/// leakage around loud tones leave about -191 dB/Hz at 384 kHz (the
/// upsampled-signal test), just above 24-bit rounding noise, while even a
/// quiet 24-bit converter's noise (-125 dBFS, about -178 dB/Hz) is well above
/// 22-bit rounding noise.
fn precision_bits(bits_per_sample: u16, is_float: bool) -> u16 {
    if is_float {
        22
    } else if bits_per_sample == 0 {
        16
    } else {
        bits_per_sample.clamp(8, 22)
    }
}

pub fn detect(
    samples: &[f32],
    sample_rate: u32,
    bits_per_sample: u16,
    is_float: bool,
) -> EffectiveNyquistResult {
    let claimed_nyquist = sample_rate as f64 / 2.0;
    let default = EffectiveNyquistResult {
        verdict: EffectiveNyquistVerdict::NotApplicable,
        explanation: String::new(),
        claimed_nyquist_hz: claimed_nyquist,
        effective_hz: claimed_nyquist,
        low_band_floor_db: f64::NAN,
        upper_band_floor_db: f64::NAN,
        drop_db: 0.0,
    };

    if samples.len() < NFFT * 4 {
        return EffectiveNyquistResult {
            explanation: "Recording too short for effective-Nyquist analysis".into(),
            ..default
        };
    }

    let psd: PsdResult = psd::compute_psd(samples, sample_rate, NFFT, None);
    let n_bins = psd.power_db.len(); // = NFFT/2 + 1
    if n_bins < N_BANDS * 2 {
        return EffectiveNyquistResult {
            explanation: "PSD bin count too low".into(),
            ..default
        };
    }

    // Divide the spectrum (excluding DC) into N_BANDS equal-width bands
    // and find the MAX power in each. We look at peaks, not noise floor,
    // because pure-tone signals can have arbitrarily low floors.
    let band_size = n_bins / N_BANDS;
    let mut band_peaks: Vec<f64> = Vec::with_capacity(N_BANDS);
    for b in 0..N_BANDS {
        let lo = (b * band_size).max(1);
        let hi = ((b + 1) * band_size).min(n_bins);
        let peak = psd.power_db[lo..hi]
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        band_peaks.push(peak);
    }

    // Global peak across all bands.
    let global_peak = band_peaks.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let populated_threshold = global_peak - POPULATED_GAP_DB;

    // Walk down from the top band, find the highest "populated" one.
    let effective_band = (0..N_BANDS)
        .rev()
        .find(|&b| band_peaks[b] > populated_threshold)
        .unwrap_or(0);
    let effective_hz = ((effective_band + 1) * band_size) as f64 * psd.freq_resolution;
    let ratio = (effective_hz / claimed_nyquist).min(1.0);

    // For diagnostics: median-of-band-peaks in lower vs upper halves.
    let split = effective_band + 1;
    let low_band_floor = if split > 0 {
        let mut lower = band_peaks[..split].to_vec();
        lower.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        lower[lower.len() / 2]
    } else {
        f64::NAN
    };
    let upper_band_floor = if split < N_BANDS {
        let mut upper = band_peaks[split..].to_vec();
        upper.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        upper[upper.len() / 2]
    } else {
        low_band_floor
    };
    let drop_db = low_band_floor - upper_band_floor;

    // Only flag as "band-limited" (suggesting upsampling / hardware bandwidth
    // limit) when the upper-band peaks are near the precision floor — that's
    // what distinguishes a digital cutoff from natural content bandlimiting
    // (e.g. bat calls don't extend up to Nyquist, but the noise still does).
    let silence_floor = rounding_noise_db(precision_bits(bits_per_sample, is_float), sample_rate)
        + ROUNDING_NOISE_MARGIN_DB;
    let upper_band_at_precision_floor = upper_band_floor < silence_floor;
    let (verdict, explanation) = if ratio < BAND_LIMITED_RATIO && upper_band_at_precision_floor {
        (
            EffectiveNyquistVerdict::BandLimited {
                claimed_nyquist_hz: claimed_nyquist,
                effective_hz,
                ratio,
            },
            format!(
                "Spectrum cuts off at {:.1} kHz ({:.0}% of the claimed Nyquist of \
                 {:.1} kHz). Above that there is no more than rounding noise \
                 (top-band median {:.0} dB/Hz). The recording was probably \
                 upsampled from a lower sample rate, or the recorder filters \
                 out the higher frequencies.",
                effective_hz / 1000.0,
                ratio * 100.0,
                claimed_nyquist / 1000.0,
                upper_band_floor,
            ),
        )
    } else {
        (
            EffectiveNyquistVerdict::FullBandwidth {
                claimed_nyquist_hz: claimed_nyquist,
            },
            format!(
                "Spectrum has content / noise floor up to within {:.0}% of the \
                 claimed Nyquist of {:.1} kHz (top-band median {:.0} dB/Hz).",
                ratio * 100.0,
                claimed_nyquist / 1000.0,
                upper_band_floor,
            ),
        )
    };
    let _ = drop_db; // kept in struct for diagnostic display

    EffectiveNyquistResult {
        verdict,
        explanation,
        claimed_nyquist_hz: claimed_nyquist,
        effective_hz,
        low_band_floor_db: low_band_floor,
        upper_band_floor_db: upper_band_floor,
        drop_db,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_upsampled_signal() {
        // A 96 kHz signal upsampled to 384 kHz: only frequencies up to
        // ~48 kHz have content; the rest is interpolation silence.
        let fs = 384_000u32;
        let n = 16_384;
        // Computed in f64: f32 time values alone add noise at about -80 dBFS
        // across the whole band, which no upsampler leaves.
        let signal: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / fs as f64;
                let tau = 2.0 * std::f64::consts::PI;
                // Several tones, all below 48 kHz
                ((tau * 10_000.0 * t).sin() * 0.3
                    + (tau * 25_000.0 * t).sin() * 0.2
                    + (tau * 42_000.0 * t).sin() * 0.1) as f32
            })
            .collect();
        let r = detect(&signal, fs, 32, true);
        assert!(
            matches!(r.verdict, EffectiveNyquistVerdict::BandLimited { .. }),
            "expected BandLimited, got {:?}",
            r,
        );
        // Effective Nyquist should be near 42 kHz (the highest tone)
        assert!(
            r.effective_hz < 60_000.0,
            "effective_hz = {}",
            r.effective_hz
        );
    }

    #[test]
    fn accepts_full_bandwidth_white_noise() {
        let fs = 384_000u32;
        let n = 16_384;
        let mut state: u32 = 0x42;
        let signal: Vec<f32> = (0..n)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                ((state >> 16) as i32 - 32768) as f32 / 32768.0 * 0.3
            })
            .collect();
        let r = detect(&signal, fs, 16, false);
        assert!(
            matches!(r.verdict, EffectiveNyquistVerdict::FullBandwidth { .. }),
            "expected FullBandwidth on white noise, got {:?}",
            r,
        );
    }

    /// Tones below 84 kHz at 384 kHz, plus white noise `noise_dbfs` RMS.
    fn tones_with_noise_floor(noise_dbfs: f64) -> Vec<f32> {
        let fs = 384_000.0;
        let amp = 10f64.powf(noise_dbfs / 20.0) * 3f64.sqrt();
        let mut state: u64 = 7;
        (0..384_000)
            .map(|i| {
                let t = i as f64 / fs;
                let tones: f64 = (1..40)
                    .map(|k| 0.02 * (2.0 * std::f64::consts::PI * 2_000.0 * k as f64 * t).sin())
                    .sum();
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let u = (state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
                (tones + u * amp) as f32
            })
            .collect()
    }

    #[test]
    fn rounding_noise_level_matches_the_measured_psd() {
        // 16-bit rounding noise is -101.1 dBFS RMS.
        let r = detect(&tones_with_noise_floor(-101.1), 384_000, 16, false);
        let expected = rounding_noise_db(16, 384_000);
        assert!(
            (r.upper_band_floor_db - expected).abs() < 2.0,
            "{} vs {expected}",
            r.upper_band_floor_db
        );
    }

    #[test]
    fn a_quiet_analog_noise_floor_is_not_called_silence() {
        // Analog noise at -90 dBFS in a 16-bit file, and at -125 dBFS in a
        // 24-bit file, reaches Nyquist: the recording is not band-limited.
        for (noise, bits) in [(-90.0, 16), (-125.0, 24)] {
            let r = detect(&tones_with_noise_floor(noise), 384_000, bits, false);
            assert!(
                matches!(r.verdict, EffectiveNyquistVerdict::FullBandwidth { .. }),
                "{bits}-bit, {noise} dBFS: {r:?}"
            );
        }
        // Rounding noise alone above the tones still reads as band-limited.
        let r = detect(&tones_with_noise_floor(-101.1), 384_000, 16, false);
        assert!(
            matches!(r.verdict, EffectiveNyquistVerdict::BandLimited { .. }),
            "{r:?}"
        );
    }
}

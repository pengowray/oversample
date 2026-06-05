use crate::dsp::fft::{plan_fft_forward, plan_fft_inverse};
use std::f32::consts::PI;

const FFT_SIZE: usize = 4096;
const HOP: usize = 1024; // 75% overlap

/// Phase-vocoder pitch shift via spectral bin remapping with phase propagation.
///
/// For each output bin it reads the (interpolated) magnitude and instantaneous
/// frequency of source bin `j / pitch_factor`, then reconstructs phase by
/// PROPAGATING a synthesis accumulator from that frequency — the canonical
/// phase-vocoder formulation — instead of snapping to a source bin's raw phase.
/// This is phase-coherent across frames, removing the warble/phasiness of the
/// old nearest-bin approach, while magnitude handling (hence output level) is
/// unchanged.
///
/// The analysis/synthesis phase state is carried across the frames within ONE
/// call but reset per call, so streaming chunks stay independent (robust at chunk
/// boundaries; the brief boundary transient is masked by the head fade + 75%
/// overlap).
///
/// - `factor > 1.0`: shift DOWN (divide frequencies). E.g. factor=10 shifts 50 kHz → 5 kHz.
/// - `factor < -1.0`: shift UP (multiply frequencies). E.g. factor=-10 shifts 5 Hz → 50 Hz.
/// - `|factor| <= 1.0`: bypass (returns input unchanged).
pub fn phase_vocoder_pitch_shift(samples: &[f32], factor: f64) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    let abs_factor = factor.abs() as f32;
    if abs_factor <= 1.0 {
        return samples.to_vec();
    }

    // pitch_factor: multiply frequencies by this amount
    // shift_down factor=10 → pitch_factor = 0.1 (divide freq by 10)
    // shift_up factor=-10 → pitch_factor = 10 (multiply freq by 10)
    let pitch_factor = if factor < 0.0 { abs_factor } else { 1.0 / abs_factor };

    if samples.len() < FFT_SIZE {
        return samples.to_vec();
    }

    let n_bins = FFT_SIZE / 2 + 1;
    let fft_f = FFT_SIZE as f32;
    let original_len = samples.len();

    // Pad input so the STFT frames fully cover every sample.
    // Without padding, the last (samples.len() % HOP) samples get no STFT frame,
    // producing a brief silence/click at streaming chunk boundaries.
    let padded_len = if !(samples.len() - FFT_SIZE).is_multiple_of(HOP) {
        let n = (samples.len() - FFT_SIZE) / HOP + 2; // one extra frame
        (n - 1) * HOP + FFT_SIZE
    } else {
        samples.len()
    };
    let samples = if padded_len > samples.len() {
        let mut padded = samples.to_vec();
        padded.resize(padded_len, 0.0);
        padded
    } else {
        samples.to_vec()
    };
    let samples = &samples[..];

    let n_frames = (samples.len().saturating_sub(FFT_SIZE)) / HOP + 1;
    if n_frames == 0 {
        return vec![0.0; original_len];
    }

    let out_len = (n_frames - 1) * HOP + FFT_SIZE;

    // Hann window
    let hann: Vec<f32> = (0..FFT_SIZE)
        .map(|i| {
            let x = PI * i as f32 / fft_f;
            x.sin().powi(2)
        })
        .collect();

    // FFT plans from the shared thread-local planner (no per-call allocation).
    let fft_forward = plan_fft_forward(FFT_SIZE);
    let fft_inverse = plan_fft_inverse(FFT_SIZE);

    let mut output = vec![0.0f32; out_len];
    let mut window_sum = vec![0.0f32; out_len];

    let mut fft_in = vec![0.0f32; FFT_SIZE];
    let mut spectrum = fft_forward.make_output_vec();
    let mut ifft_out = vec![0.0f32; FFT_SIZE];

    // Phase-vocoder state carried across the frames WITHIN this call, then reset
    // per call → stateless ACROSS streaming chunks (chunk-boundary robustness is
    // preserved; the brief boundary transient is masked by the head fade + 75%
    // overlap). `prev_phase` drives instantaneous-frequency analysis; `sum_phase`
    // is the synthesis phase accumulator for coherent reconstruction.
    let mut mag = vec![0.0f32; n_bins];
    let mut ana_freq = vec![0.0f32; n_bins]; // true (instantaneous) freq per source bin, in bins
    let mut prev_phase = vec![0.0f32; n_bins];
    let mut sum_phase = vec![0.0f32; n_bins];

    // expct = expected phase advance per hop for one bin-index step;
    // osamp = analysis oversampling (FFT_SIZE / HOP) = the canonical PV constants.
    let expct = 2.0 * PI * HOP as f32 / fft_f;
    let osamp = fft_f / HOP as f32;
    let two_pi = 2.0 * PI;

    for frame in 0..n_frames {
        let offset = frame * HOP;
        if offset + FFT_SIZE > samples.len() {
            break;
        }

        // Window the input frame
        for i in 0..FFT_SIZE {
            fft_in[i] = samples[offset + i] * hann[i];
        }

        // Forward FFT (skip the frame on the impossible size-mismatch error
        // rather than panicking in the audio thread).
        if fft_forward.process(&mut fft_in, &mut spectrum).is_err() {
            continue;
        }

        // ── Analysis: magnitude + instantaneous (true) frequency per bin ──
        // The true frequency is the bin centre plus the phase drift relative to
        // the previous frame (the heterodyned, wrapped phase difference) — what a
        // real phase vocoder uses instead of the raw bin index.
        for k in 0..n_bins {
            let re = spectrum[k].re;
            let im = spectrum[k].im;
            mag[k] = (re * re + im * im).sqrt();
            let p = im.atan2(re);
            if frame == 0 {
                ana_freq[k] = k as f32; // no previous frame — assume bin centre
            } else {
                let mut d = p - prev_phase[k] - k as f32 * expct;
                d -= two_pi * (d / two_pi).round(); // wrap deviation to [-π, π]
                ana_freq[k] = k as f32 + osamp * d / two_pi;
            }
            prev_phase[k] = p;
        }

        // ── Synthesis: inverse-map each output bin j from source j/pitch_factor,
        // interpolating magnitude AND true frequency, then PROPAGATE the synthesis
        // phase (accumulate the per-hop advance) so the reconstruction is phase-
        // coherent across frames instead of snapping to a single bin's phase.
        // Magnitude handling is unchanged from before, so output level is too. ──
        for (j, spec_bin) in spectrum.iter_mut().enumerate().take(n_bins) {
            let source = j as f32 / pitch_factor;
            let s_idx = source as usize;
            let s_frac = source - s_idx as f32;

            let (m, src_freq) = if s_idx + 1 < n_bins {
                (
                    mag[s_idx] * (1.0 - s_frac) + mag[s_idx + 1] * s_frac,
                    ana_freq[s_idx] * (1.0 - s_frac) + ana_freq[s_idx + 1] * s_frac,
                )
            } else if s_idx < n_bins {
                (mag[s_idx] * (1.0 - s_frac), ana_freq[s_idx])
            } else {
                (0.0, j as f32)
            };

            // Output instantaneous frequency = source freq × pitch_factor (bins).
            // Turn its deviation from the bin centre into a per-hop phase advance
            // and accumulate; wrap to keep the f32 accumulator precise.
            let out_freq = src_freq * pitch_factor;
            let advance = (out_freq - j as f32) / osamp * two_pi + j as f32 * expct;
            let mut ph = sum_phase[j] + advance;
            ph -= two_pi * (ph / two_pi).floor();
            sum_phase[j] = ph;

            spec_bin.re = m * ph.cos();
            spec_bin.im = m * ph.sin();
        }

        // DC and Nyquist bins must be real for realfft inverse
        spectrum[0].im = 0.0;
        spectrum[n_bins - 1].im = 0.0;

        // Inverse FFT (skip this frame's overlap-add on the impossible error).
        if fft_inverse.process(&mut spectrum, &mut ifft_out).is_err() {
            continue;
        }

        // Normalize + overlap-add
        let norm = 1.0 / fft_f;
        let out_offset = frame * HOP;
        for i in 0..FFT_SIZE {
            let j = out_offset + i;
            if j < out_len {
                output[j] += ifft_out[i] * norm * hann[i];
                window_sum[j] += hann[i] * hann[i];
            }
        }
    }

    // Normalize by window overlap sum
    for i in 0..out_len {
        if window_sum[i] > 1e-6 {
            output[i] /= window_sum[i];
        }
    }

    // Fade in over the first HOP samples to avoid onset click from incomplete
    // window overlap at the start of each chunk
    let fade_len = HOP.min(out_len);
    for (i, sample) in output.iter_mut().enumerate().take(fade_len) {
        *sample *= i as f32 / fade_len as f32;
    }

    // Truncate to original input length
    output.truncate(original_len);
    while output.len() < original_len {
        output.push(0.0);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_small_factor() {
        let input: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.1).sin()).collect();
        assert_eq!(phase_vocoder_pitch_shift(&input, 1.0), input);
        assert_eq!(phase_vocoder_pitch_shift(&input, -1.0), input);
        assert_eq!(phase_vocoder_pitch_shift(&input, 0.5), input);
    }

    #[test]
    fn test_empty_input() {
        assert!(phase_vocoder_pitch_shift(&[], 10.0).is_empty());
    }

    #[test]
    fn test_preserves_length_down() {
        let input: Vec<f32> = (0..8192).map(|i| (i as f32 * 0.01).sin()).collect();
        let output = phase_vocoder_pitch_shift(&input, 10.0);
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn test_preserves_length_up() {
        let input: Vec<f32> = (0..8192).map(|i| (i as f32 * 0.01).sin()).collect();
        let output = phase_vocoder_pitch_shift(&input, -10.0);
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn test_nonzero_output_down() {
        // 40 kHz tone at 192 kHz sample rate, shifted down by 10
        let sr = 192000.0f32;
        let freq = 40000.0f32;
        let n = 16384;
        let input: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sr).sin())
            .collect();
        let output = phase_vocoder_pitch_shift(&input, 10.0);
        let peak = output.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.01, "Output should not be silent, peak={}", peak);
    }

    #[test]
    fn test_nonzero_output_up() {
        let input: Vec<f32> = (0..16384).map(|i| (i as f32 * 0.01).sin()).collect();
        let output = phase_vocoder_pitch_shift(&input, -5.0);
        let peak = output.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.01, "Output should not be silent, peak={}", peak);
    }

    /// Dominant frequency (Hz) via the peak FFT bin of a Hann-windowed slice
    /// taken from the middle of the signal (avoids the head fade + edges).
    fn dominant_freq(samples: &[f32], sr: f32) -> f32 {
        use realfft::RealFftPlanner;
        let n = FFT_SIZE.min(samples.len());
        let start = (samples.len() - n) / 2;
        let mut buf: Vec<f32> = samples[start..start + n].to_vec();
        for (i, s) in buf.iter_mut().enumerate() {
            *s *= (PI * i as f32 / n as f32).sin().powi(2);
        }
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let mut spec = fft.make_output_vec();
        fft.process(&mut buf, &mut spec).unwrap();
        let (mut peak_bin, mut peak_mag) = (0usize, 0.0f32);
        for (k, c) in spec.iter().enumerate() {
            let m = c.re * c.re + c.im * c.im;
            if m > peak_mag {
                peak_mag = m;
                peak_bin = k;
            }
        }
        peak_bin as f32 * sr / n as f32
    }

    #[test]
    fn test_shifts_to_correct_frequency_down() {
        // 24 kHz tone @ 192 kHz, shift DOWN 8x -> expect ~3 kHz. (24 kHz lands on
        // an exact bin so the remap target is clean.)
        let sr = 192000.0f32;
        let input: Vec<f32> = (0..16384)
            .map(|i| (2.0 * PI * 24000.0 * i as f32 / sr).sin())
            .collect();
        let out = phase_vocoder_pitch_shift(&input, 8.0);
        let measured = dominant_freq(&out, sr);
        let tol = sr / FFT_SIZE as f32 * 2.0; // +/- 2 bins
        assert!(
            (measured - 3000.0).abs() < tol,
            "expected ~3000 Hz, got {measured} Hz"
        );
    }

    #[test]
    fn test_shifts_to_correct_frequency_up() {
        // 3 kHz tone @ 192 kHz, shift UP 8x (factor -8) -> expect ~24 kHz.
        let sr = 192000.0f32;
        let input: Vec<f32> = (0..16384)
            .map(|i| (2.0 * PI * 3000.0 * i as f32 / sr).sin())
            .collect();
        let out = phase_vocoder_pitch_shift(&input, -8.0);
        let measured = dominant_freq(&out, sr);
        let tol = sr / FFT_SIZE as f32 * 2.0;
        assert!(
            (measured - 24000.0).abs() < tol,
            "expected ~24000 Hz, got {measured} Hz"
        );
    }
}

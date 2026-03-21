use realfft::RealFftPlanner;
use std::f32::consts::PI;

const FFT_SIZE: usize = 4096;
const HOP: usize = 1024; // 75% overlap

/// Phase-vocoder pitch shift via direct spectral bin shifting.
///
/// Shifts frequency content by remapping STFT bins each frame. Uses the source
/// bin's analysis phase (scaled by pitch_factor) directly rather than accumulating
/// — this makes each frame stateless, eliminating phase discontinuities at
/// streaming chunk boundaries.
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

    // Set up FFT
    let mut planner = RealFftPlanner::<f32>::new();
    let fft_forward = planner.plan_fft_forward(FFT_SIZE);
    let fft_inverse = planner.plan_fft_inverse(FFT_SIZE);

    let mut output = vec![0.0f32; out_len];
    let mut window_sum = vec![0.0f32; out_len];

    let mut fft_in = vec![0.0f32; FFT_SIZE];
    let mut spectrum = fft_forward.make_output_vec();
    let mut ifft_out = vec![0.0f32; FFT_SIZE];

    // Per-frame scratch for analysis
    let mut mag = vec![0.0f32; n_bins];
    let mut phase = vec![0.0f32; n_bins];

    for frame in 0..n_frames {
        let offset = frame * HOP;
        if offset + FFT_SIZE > samples.len() {
            break;
        }

        // Window the input frame
        for i in 0..FFT_SIZE {
            fft_in[i] = samples[offset + i] * hann[i];
        }

        // Forward FFT
        fft_forward.process(&mut fft_in, &mut spectrum).unwrap();

        // Extract magnitude and phase per bin
        for k in 0..n_bins {
            let re = spectrum[k].re;
            let im = spectrum[k].im;
            mag[k] = (re * re + im * im).sqrt();
            phase[k] = im.atan2(re);
        }

        // Shift bins: for each output bin j, read from source bin j / pitch_factor.
        // Use source phase scaled by pitch_factor — this preserves the phase
        // relationship proportional to the frequency shift and is stateless
        // (no accumulator that drifts across streaming chunks).
        for (j, spec_bin) in spectrum.iter_mut().enumerate().take(n_bins) {
            let source = j as f32 / pitch_factor;
            let s_idx = source as usize;
            let s_frac = source - s_idx as f32;

            let (m, src_phase) = if s_idx + 1 < n_bins {
                // Interpolate magnitude; take nearest bin phase (avoids wrapping issues)
                (
                    mag[s_idx] * (1.0 - s_frac) + mag[s_idx + 1] * s_frac,
                    if s_frac < 0.5 { phase[s_idx] } else { phase[s_idx + 1] },
                )
            } else if s_idx < n_bins {
                (mag[s_idx] * (1.0 - s_frac), phase[s_idx])
            } else {
                (0.0, 0.0)
            };

            // Scale source phase by pitch_factor: a tone at bin s with phase φ
            // maps to bin j = s * pitch_factor with phase φ * pitch_factor,
            // preserving the phase-frequency relationship.
            let out_phase = src_phase * pitch_factor;

            spec_bin.re = m * out_phase.cos();
            spec_bin.im = m * out_phase.sin();
        }

        // DC and Nyquist bins must be real for realfft inverse
        spectrum[0].im = 0.0;
        spectrum[n_bins - 1].im = 0.0;

        // Inverse FFT
        fft_inverse.process(&mut spectrum, &mut ifft_out).unwrap();

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
}

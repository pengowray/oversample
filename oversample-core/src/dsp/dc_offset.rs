//! A recording's DC offset, measured once and removed from its spectrogram
//! displays.
//!
//! Many recorders add a constant bias to every sample. A spectrogram shows it
//! in its lowest row as a bright line along the whole recording, and it can be
//! louder than any call, which darkens the picture when the display's
//! brightness is set by the loudest point. In 127 of 293 demo recordings the
//! lowest bin was over 10 dB above the bins next to it; subtracting the plain
//! mean fixed all but 13, and a filter that followed drift did no better, so
//! one constant per recording is enough.
//!
//! Only displays use this. Playback, exports and analysis read the samples as
//! they are.

/// Seconds from the start of a recording that its offset is measured over.
///
/// The offset is constant, so this only needs enough samples for a steady
/// mean. At bat-detector rates three seconds is over half a million samples,
/// which puts the mean's own error far below the noise in the lowest bin, and
/// it is long enough that low-frequency sound (wind, handling) and a
/// recorder's start-up settling add little to it. It also stays inside the
/// 30 s that streaming files keep in memory, so measuring never waits on a
/// read from disk.
pub const OFFSET_WINDOW_SECS: f64 = 3.0;

/// Samples in [`OFFSET_WINDOW_SECS`] at `sample_rate`.
pub fn offset_window_len(sample_rate: u32) -> usize {
    (OFFSET_WINDOW_SECS * sample_rate as f64).ceil() as usize
}

/// Fewest exact zeros in a row that count as digital silence (a recorder's
/// padding, a muted stretch), which has no offset.
///
/// Shorter runs are ordinary samples that happen to be zero: a 16-bit
/// recording with an offset crosses zero now and then (XC1064993 has 119 such
/// samples). Those are shifted like any other sample; leaving one at zero
/// would add a click as loud as the offset, which shows as a bright column.
pub const MIN_SILENT_RUN: usize = 64;

/// Whether the run of exact zeros at `run` in a slice of `len` samples is
/// digital silence. A run at either end of the slice counts too, since it may
/// carry on past the slice.
fn is_silence(run: std::ops::Range<usize>, len: usize) -> bool {
    run.len() >= MIN_SILENT_RUN || run.start == 0 || run.end == len
}

/// End of the run of exact zeros that starts at `start`.
fn zero_run_end(samples: &[f32], start: usize) -> usize {
    samples[start..]
        .iter()
        .position(|&s| s != 0.0)
        .map_or(samples.len(), |n| start + n)
}

/// Sum and count of `samples`, leaving out digital silence
/// ([`MIN_SILENT_RUN`]) and samples that are not finite numbers (a corrupt
/// float file), either of which would spoil the mean.
fn sum_outside_silence(samples: &[f32]) -> (f64, usize) {
    let len = samples.len();
    let (mut sum, mut count) = (0.0f64, 0usize);
    let mut i = 0;
    while i < len {
        if samples[i] != 0.0 {
            if samples[i].is_finite() {
                sum += samples[i] as f64;
                count += 1;
            }
            i += 1;
            continue;
        }
        let end = zero_run_end(samples, i);
        if !is_silence(i..end, len) {
            count += end - i;
        }
        i = end;
    }
    (sum, count)
}

/// The offset of `samples`: their mean, leaving out digital silence
/// ([`MIN_SILENT_RUN`]), which has no offset and would pull the mean towards
/// zero. 0 when it is all silence.
pub fn mean_outside_silence(samples: &[f32]) -> f32 {
    match sum_outside_silence(samples) {
        (_, 0) => 0.0,
        (sum, count) => (sum / count as f64) as f32,
    }
}

/// Seconds of sound (samples outside digital silence) that a measurement
/// needs. With less, a short sound at the end of a silent stretch would set
/// the offset by itself, and subtracting its mean would add a line instead of
/// removing one.
pub const MIN_SOUND_SECS: f64 = 0.25;

/// The offset to remove from a recording, measured from its first
/// [`OFFSET_WINDOW_SECS`] (`samples`): [`mean_outside_silence`], or 0 when
/// they hold less than [`MIN_SOUND_SECS`] of sound.
pub fn recording_offset(samples: &[f32], sample_rate: u32) -> f32 {
    let (sum, count) = sum_outside_silence(samples);
    if (count as f64) < MIN_SOUND_SECS * sample_rate as f64 {
        return 0.0;
    }
    let dc = (sum / count as f64) as f32;
    if dc.is_finite() {
        dc
    } else {
        0.0
    }
}

/// Subtract `dc` from every sample except digital silence
/// ([`MIN_SILENT_RUN`]), which stays at zero so it stays black in a
/// spectrogram.
pub fn subtract_dc(samples: &mut [f32], dc: f32) {
    if dc == 0.0 || !dc.is_finite() {
        return;
    }
    let len = samples.len();
    let mut i = 0;
    while i < len {
        if samples[i] != 0.0 {
            samples[i] -= dc;
            i += 1;
            continue;
        }
        let end = zero_run_end(samples, i);
        if !is_silence(i..end, len) {
            for s in &mut samples[i..end] {
                *s -= dc;
            }
        }
        i = end;
    }
}

/// Bins of a Hann-windowed FFT that an offset reaches: bin 0 at its full
/// level, bin 1 at -6 dB and bin 2 at about -80 dB. Bin 3 gets about -90 dB.
pub const OFFSET_BINS: usize = 3;

/// A noise floor (one mean magnitude per bin of a Hann-windowed FFT of
/// `fft_size` points) learned from samples that still held an offset of `dc`,
/// made fit for a display that has removed the offset.
///
/// When the offset is louder than the floor at bin [`OFFSET_BINS`], each bin
/// it reaches is capped at that bin's level. Otherwise the display would treat
/// its now-quiet lowest rows as loud noise and darken them to black. `None`
/// when the offset is quieter than the noise there, or the floor is too short:
/// the floor is then fine as it is.
pub fn floor_without_offset(floor: &[f64], fft_size: usize, dc: f32) -> Option<Vec<f64>> {
    let reference = *floor.get(OFFSET_BINS)?;
    // The symmetric Hann window (`fft::hann_window`) sums to (N - 1) / 2,
    // which is the magnitude an offset of 1 gets in bin 0.
    let offset_mag = dc.abs() as f64 * fft_size.saturating_sub(1) as f64 / 2.0;
    if offset_mag <= reference {
        return None;
    }
    let mut out = floor.to_vec();
    for m in &mut out[..OFFSET_BINS] {
        *m = m.min(reference);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::fft::compute_stft_columns;

    #[test]
    fn mean_skips_digital_silence() {
        // Half digital silence: the plain mean would be 0.05.
        let mut samples = vec![0.0f32; 1000];
        samples.extend((0..1000).map(|i| 0.1 + if i % 2 == 0 { 0.02 } else { -0.02 }));
        assert!((mean_outside_silence(&samples) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn mean_counts_lone_zeros() {
        // Ordinary samples that happen to be zero are part of the mean.
        let samples = [0.2, 0.0, 0.2, 0.0, 0.2, 0.2];
        assert!((mean_outside_silence(&samples) - 0.8 / 6.0).abs() < 1e-6);
    }

    #[test]
    fn mean_of_silence_is_zero() {
        assert_eq!(mean_outside_silence(&[]), 0.0);
        assert_eq!(mean_outside_silence(&[0.0; 64]), 0.0);
        assert_eq!(mean_outside_silence(&[0.0; 3]), 0.0);
    }

    #[test]
    fn subtract_keeps_digital_silence_at_zero() {
        let mut samples = vec![0.3; 10];
        samples.extend([0.0; MIN_SILENT_RUN]);
        samples.extend([0.3; 10]);
        subtract_dc(&mut samples, 0.1);
        assert!(samples[10..10 + MIN_SILENT_RUN].iter().all(|&s| s == 0.0));
        assert!((samples[0] - 0.2).abs() < 1e-6);
        assert!((samples[samples.len() - 1] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn subtract_keeps_zeros_at_the_ends_at_zero() {
        // They may be the edge of silence that carries on past the slice.
        let mut samples = vec![0.0, 0.0, 0.3, -0.1, 0.0];
        subtract_dc(&mut samples, 0.1);
        assert_eq!(&samples[..2], &[0.0, 0.0]);
        assert_eq!(samples[4], 0.0);
        assert!((samples[2] - 0.2).abs() < 1e-6);
        assert!((samples[3] + 0.2).abs() < 1e-6);
    }

    #[test]
    fn subtract_shifts_lone_zeros() {
        // A zero inside the signal is an ordinary sample: leaving it at zero
        // would add a click as loud as the offset.
        let mut samples = vec![0.3, 0.0, 0.0, 0.3];
        subtract_dc(&mut samples, 0.1);
        for (s, want) in samples.iter().zip([0.2, -0.1, -0.1, 0.2]) {
            assert!((s - want).abs() < 1e-6, "{samples:?}");
        }
    }

    #[test]
    fn subtracting_the_mean_removes_the_offset_bin() {
        // A tone at bin 64 of a 1024-point FFT, on top of an offset.
        let n = 8192;
        let fft = 1024;
        let tone = |i: usize| 0.01 * (std::f32::consts::TAU * 64.0 * i as f32 / fft as f32).sin();
        let mut samples: Vec<f32> = (0..n).map(|i| 0.05 + tone(i)).collect();
        let lowest = |s: &[f32]| compute_stft_columns(s, 48_000, fft, fft, 0, 1)[0].magnitudes[0];
        let before = lowest(&samples);
        let dc = mean_outside_silence(&samples);
        subtract_dc(&mut samples, dc);
        let after = lowest(&samples);
        let tone_peak = compute_stft_columns(&samples, 48_000, fft, fft, 0, 1)[0].magnitudes[64];
        assert!(before > tone_peak, "offset louder than the tone before");
        assert!(
            after < tone_peak * 1e-3,
            "lowest bin {after} vs tone {tone_peak}"
        );
    }

    #[test]
    fn floor_lowest_bins_capped_when_offset_is_loud() {
        let floor = vec![50.0, 25.0, 0.2, 0.1, 0.1, 0.1];
        let capped = floor_without_offset(&floor, 1024, 0.1).unwrap();
        assert_eq!(capped, vec![0.1, 0.1, 0.1, 0.1, 0.1, 0.1]);
        // Bins already below the reference stay where they are.
        let low = vec![50.0, 0.05, 0.2, 0.1];
        assert_eq!(floor_without_offset(&low, 1024, 0.1).unwrap()[1], 0.05);
    }

    #[test]
    fn floor_unchanged_when_offset_is_quiet() {
        // Offset magnitude 1e-6 * 511.5 is far below the floor at bin 3.
        let floor = vec![3.0, 2.0, 1.0, 1.0, 1.0];
        assert!(floor_without_offset(&floor, 1024, 1e-6).is_none());
        assert!(floor_without_offset(&floor, 1024, 0.0).is_none());
        assert!(floor_without_offset(&floor[..3], 1024, 1.0).is_none());
    }

    #[test]
    fn offset_ignores_samples_that_are_not_numbers() {
        let mut samples = vec![0.1f32; 48_000];
        samples[1000] = f32::NAN;
        samples[2000] = f32::INFINITY;
        assert!((recording_offset(&samples, 48_000) - 0.1).abs() < 1e-6);
        let mut tile = vec![0.3f32; 4];
        subtract_dc(&mut tile, f32::NAN);
        assert_eq!(tile, vec![0.3; 4]);
    }

    #[test]
    fn offset_is_zero_with_too_little_sound() {
        // 5 ms of sound at the end of an otherwise silent window.
        let sr = 48_000;
        let mut samples = vec![0.0f32; 3 * sr as usize];
        let n = samples.len();
        for s in &mut samples[n - 240..] {
            *s = 0.5;
        }
        assert_eq!(recording_offset(&samples, sr), 0.0);
        // A quarter of a second is enough.
        for s in &mut samples[n - sr as usize / 4..] {
            *s = 0.5;
        }
        assert!((recording_offset(&samples, sr) - 0.5).abs() < 1e-6);
    }
}

//! Spectrogram levels in dBFS, the same at every FFT size.
//!
//! A Hann-windowed FFT of a full-scale sine centred on a bin gives that bin a
//! magnitude of (N−1)/4: the symmetric Hann window in `fft::hann_window` sums
//! to (N−1)/2, and a real sine puts half its amplitude in the
//! positive-frequency bin. Tiles store `20·log10(magnitude)`,
//! so subtracting [`full_scale_db`] for the tile's FFT size gives dBFS: a
//! full-scale tone reads 0 dB at every zoom level.
//!
//! Noise spreads its power across the bins, so per-bin noise drops by about
//! 3 dB each time the FFT size doubles, while tones stay put.

use crate::types::SpectrogramColumn;
use std::borrow::Borrow;

/// dB of a full-scale sine's peak bin with a Hann window of `fft_size`.
pub fn full_scale_db(fft_size: usize) -> f32 {
    20.0 * ((fft_size.max(5) - 1) as f32 / 4.0).log10()
}

/// How a tile's magnitudes were made. The kinds read a full-scale tone at
/// different levels, so each has its own reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileKind {
    /// Hann-windowed STFT magnitudes: the plain spectrogram and Flow.
    Stft,
    /// Reassigned power. Bins that aren't moved keep their plain STFT level,
    /// so these tiles use the STFT reference and their background matches the
    /// plain tiles drawn beside them while they compute. A bin-centred tone's
    /// main lobe (three bins at amplitude 1, ½, ½) is gathered into one bin,
    /// so its peak reads 10·log10(1.5) = 1.76 dB above the STFT peak.
    Reassigned,
    /// Resonator bank, scaled in `resonators` to read like the STFT.
    Resonator,
}

/// dB of a full-scale sine's peak in a tile of `kind` made with `fft_size`
/// points. Subtract it from a tile's dB to get dBFS.
pub fn tile_full_scale_db(kind: TileKind, fft_size: usize) -> f32 {
    match kind {
        // All three share the STFT reference; see each variant for why.
        TileKind::Stft | TileKind::Reassigned | TileKind::Resonator => full_scale_db(fft_size),
    }
}

/// How far above the reference a bin-centred tone's peak reads in a
/// reassigned tile: 10·log10(1.5), see [`TileKind::Reassigned`].
pub const REASSIGNED_TONE_GAIN_DB: f32 = 1.760_913;

/// FFT size that produced a one-sided spectrum of `bins` bins.
pub fn fft_size_for_bins(bins: usize) -> usize {
    bins.saturating_sub(1) * 2
}

/// A per-bin noise level measured at `from_fft` points, moved to `to_fft`.
pub fn noise_at_fft_size(noise_dbfs: f32, from_fft: usize, to_fft: usize) -> f32 {
    if from_fft == 0 || to_fft == 0 {
        return noise_dbfs;
    }
    noise_dbfs - 10.0 * (to_fft as f32 / from_fft as f32).log10()
}

/// Each bin's noise level is the magnitude that this fraction of its frames
/// fall below. Low enough to skip calls, high enough to skip silent gaps.
const NOISE_PERCENTILE: f32 = 0.2;

/// Fewest columns that give a usable noise estimate.
const MIN_NOISE_COLUMNS: usize = 8;

/// Most columns sampled for a noise estimate; longer inputs are thinned.
const MAX_NOISE_COLUMNS: usize = 1000;

/// Noise floor of a spectrogram in dBFS per bin, at the columns' FFT size.
///
/// Takes each bin's 20th percentile over time, then the median across bins,
/// so calls, steady sounds in a few bins (insects, mains hum) and the empty
/// band above a recorder's filter don't move it. Frames of digital silence
/// are skipped. `None` when there are too few columns or no sound at all.
pub fn noise_floor_dbfs<C: Borrow<SpectrogramColumn>>(columns: &[C]) -> Option<f32> {
    let bins = columns.first()?.borrow().magnitudes.len();
    noise_floor_dbfs_by(columns.len(), bins, |col, bin| {
        columns[col]
            .borrow()
            .magnitudes
            .get(bin)
            .copied()
            .unwrap_or(0.0)
    })
}

/// [`noise_floor_dbfs`] for magnitudes stored some other way: `mag(col, bin)`
/// for `cols` columns of `bins` one-sided FFT bins each.
pub fn noise_floor_dbfs_by(
    cols: usize,
    bins: usize,
    mag: impl Fn(usize, usize) -> f32,
) -> Option<f32> {
    if cols < MIN_NOISE_COLUMNS || bins < 3 {
        return None;
    }
    let step = cols.div_ceil(MAX_NOISE_COLUMNS);
    let mut per_bin = Vec::with_capacity(bins);
    let mut values = Vec::with_capacity(cols / step + 1);
    // Skip DC and Nyquist: DC offset and the edge bin aren't noise.
    for bin in 1..bins - 1 {
        values.clear();
        values.extend(
            (0..cols)
                .step_by(step)
                .map(|col| mag(col, bin))
                .filter(|m| *m > 0.0 && m.is_finite()),
        );
        if values.len() < MIN_NOISE_COLUMNS {
            continue;
        }
        let k = ((values.len() - 1) as f32 * NOISE_PERCENTILE) as usize;
        let (_, v, _) = values.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
        per_bin.push(20.0 * v.log10());
    }
    if per_bin.len() < (bins - 2) / 2 {
        return None;
    }
    let mid = per_bin.len() / 2;
    let (_, median, _) = per_bin.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    Some(*median - full_scale_db(fft_size_for_bins(bins)))
}

/// Noise floor of `samples` in dBFS per bin at `fft_size`, from up to
/// `max_frames` frames spread evenly across them.
pub fn estimate_noise_floor_dbfs(
    samples: &[f32],
    fft_size: usize,
    max_frames: usize,
) -> Option<f32> {
    if fft_size < 4 || samples.len() < fft_size || max_frames == 0 {
        return None;
    }
    let hop = ((samples.len() - fft_size) / max_frames).max(1);
    let columns = super::fft::compute_stft_columns(samples, 1, fft_size, hop, 0, max_frames);
    noise_floor_dbfs(&columns)
}

/// dB the Auto range reaches below the noise floor, so noise shows dark but
/// not black.
pub const AUTO_RANGE_MARGIN_DB: f32 = 10.0;

/// Smallest Auto range, for very noisy recordings.
pub const MIN_AUTO_RANGE_DB: f32 = 40.0;

/// Display range for a noise floor: from 0 dBFS down to
/// [`AUTO_RANGE_MARGIN_DB`] below the noise, rounded up to 10 dB, between
/// [`MIN_AUTO_RANGE_DB`] and `max_range_db` (the file's bit-depth limit).
/// Without a noise floor, the 16-bit range (120 dB) capped by the limit.
pub fn auto_range_db(noise_floor_dbfs: Option<f32>, max_range_db: f32) -> f32 {
    match noise_floor_dbfs.filter(|nf| nf.is_finite()) {
        None => super::depth_range::RANGE_16_BIT_DB.min(max_range_db),
        Some(nf) => {
            let range = ((AUTO_RANGE_MARGIN_DB - nf) / 10.0).ceil() * 10.0;
            range.clamp(MIN_AUTO_RANGE_DB.min(max_range_db), max_range_db)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::colors::magnitude_to_db;
    use crate::dsp::fft::compute_stft_columns;

    fn sine(n: usize, cycles_per_sample: f64, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f64::consts::PI * cycles_per_sample * i as f64).sin() as f32)
            .collect()
    }

    #[test]
    fn full_scale_tone_reads_zero_dbfs_at_every_fft_size() {
        for n in [16, 64, 128, 256, 512, 1024, 2048, 8192] {
            let bin = n / 8;
            let samples = sine(n * 4, bin as f64 / n as f64, 1.0);
            let cols = compute_stft_columns(&samples, 48_000, n, n, 0, 1);
            let peak = cols[0].magnitudes[bin];
            let dbfs = magnitude_to_db(peak) - full_scale_db(n);
            assert!(dbfs.abs() < 0.05, "N={n}: {dbfs} dBFS");
        }
    }

    #[test]
    fn full_scale_tone_level_in_every_tile_kind() {
        use crate::dsp::resonators::{
            compute_resonator_columns, warmup_samples, ResonatorAlphaMode, ResonatorLayout,
        };
        let (n, sr, bin) = (1024, 384_000u32, 100);
        let samples = sine(sr as usize, bin as f64 / n as f64, 1.0);

        let tile = crate::dsp::reassign::compute_reassigned_tile(
            &samples[..n * 40],
            0,
            30,
            n,
            256,
            -400.0,
            &[],
        );
        let peak = tile
            .db_data
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        let dbfs = peak - tile_full_scale_db(TileKind::Reassigned, n);
        assert!(
            (dbfs - REASSIGNED_TONE_GAIN_DB).abs() < 0.1,
            "reassigned: {dbfs} dBFS"
        );

        for bw in [5.0, 20.0, 100.0] {
            let warm_cols = warmup_samples(sr, bw) / 256 + 1;
            let cols = compute_resonator_columns(
                &samples,
                sr,
                n,
                256,
                warm_cols,
                10,
                bw,
                ResonatorAlphaMode::ConstBandwidth,
                8.0,
                ResonatorLayout::Linear,
                None,
            );
            let peak = cols
                .iter()
                .flat_map(|c| c.magnitudes.iter().copied())
                .fold(0.0, f32::max);
            let dbfs = magnitude_to_db(peak) - tile_full_scale_db(TileKind::Resonator, n);
            assert!(dbfs.abs() < 0.1, "resonator {bw} Hz: {dbfs} dBFS");
        }
    }

    #[test]
    fn white_noise_floor_drops_3_db_per_fft_doubling() {
        // Deterministic white noise, uniform in ±0.01.
        let mut x: u32 = 12345;
        let samples: Vec<f32> = (0..400_000)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x as f32 / u32::MAX as f32 - 0.5) * 0.02
            })
            .collect();
        let at_256 = estimate_noise_floor_dbfs(&samples, 256, 400).unwrap();
        let at_1024 = estimate_noise_floor_dbfs(&samples, 1024, 400).unwrap();
        let moved = noise_at_fft_size(at_256, 256, 1024);
        assert!(
            (moved - at_1024).abs() < 1.0,
            "{at_256} → {moved} vs {at_1024}"
        );
        // σ² = 0.02²/12, so per-bin noise power is σ²·6/N relative to full
        // scale: about −67 dB at N=1024. The 20th percentile sits ~6.5 dB
        // below the mean power.
        assert!((-75.0..-72.0).contains(&at_1024), "{at_1024}");
    }

    #[test]
    fn calls_and_one_loud_band_dont_move_the_noise_floor() {
        let mut x: u32 = 99;
        let n = 200_000;
        let mut samples: Vec<f32> = (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x as f32 / u32::MAX as f32 - 0.5) * 0.002
            })
            .collect();
        let quiet = estimate_noise_floor_dbfs(&samples, 512, 300).unwrap();
        // A steady tone (an insect) and loud bursts (calls) in 10% of frames.
        let tone = sine(n, 0.1, 0.3);
        for (i, s) in samples.iter_mut().enumerate() {
            *s += tone[i];
            if (i / 5000) % 10 == 0 {
                *s += 0.5 * ((i as f32) * 0.7).sin();
            }
        }
        let busy = estimate_noise_floor_dbfs(&samples, 512, 300).unwrap();
        assert!((busy - quiet).abs() < 2.0, "{quiet} vs {busy}");
    }

    #[test]
    fn digital_silence_has_no_noise_floor() {
        assert_eq!(
            estimate_noise_floor_dbfs(&vec![0.0; 50_000], 512, 100),
            None
        );
    }

    #[test]
    fn auto_range_sits_below_the_noise_and_inside_the_depth_limit() {
        assert_eq!(auto_range_db(Some(-93.0), 170.0), 110.0);
        assert_eq!(auto_range_db(Some(-140.0), 120.0), 120.0);
        assert_eq!(auto_range_db(Some(-15.0), 120.0), 40.0);
        assert_eq!(auto_range_db(None, 170.0), 120.0);
        assert_eq!(auto_range_db(Some(f32::NEG_INFINITY), 170.0), 120.0);
    }
}

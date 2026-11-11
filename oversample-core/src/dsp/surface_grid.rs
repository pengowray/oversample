//! Spectrogram grid for the 3D surface view of a short selection.
//!
//! The STFT runs at a normal hop (FFT/4), then cells are merged with max
//! pooling down to a grid small enough to draw and rotate smoothly. Max rather
//! than mean, so a call a few milliseconds long keeps its peak height when many
//! columns are merged into one.

use super::fft::compute_stft_columns;

/// Level grid in dB relative to the loudest cell (which is 0 dB).
#[derive(Clone, Debug)]
pub struct SurfaceGrid {
    /// Row-major, `rows * cols`; row 0 is the lowest frequency.
    pub level_db: Vec<f32>,
    pub cols: usize,
    pub rows: usize,
    /// Centre time of each column, in seconds from the start of `samples`.
    pub times_s: Vec<f64>,
    /// Centre frequency of each row, in Hz.
    pub freqs_hz: Vec<f64>,
}

/// Levels below this are clamped, so silence (log of 0) stays finite.
const MIN_DB: f32 = -200.0;

/// `None` when `samples` is shorter than one FFT window or the band holds no bins.
pub fn surface_grid(
    samples: &[f32],
    sample_rate: u32,
    fft_size: usize,
    band_hz: (f64, f64),
    max_cols: usize,
    max_rows: usize,
) -> Option<SurfaceGrid> {
    let hop = (fft_size / 4).max(1);
    let columns = compute_stft_columns(samples, sample_rate, fft_size, hop, 0, usize::MAX);
    if columns.is_empty() || max_cols == 0 || max_rows == 0 {
        return None;
    }
    let bin_hz = sample_rate as f64 / fft_size as f64;
    let last_bin = fft_size / 2;
    let lo_bin = ((band_hz.0.max(0.0) / bin_hz).floor() as usize).min(last_bin);
    let hi_bin = ((band_hz.1 / bin_hz).ceil() as usize).min(last_bin);
    if hi_bin <= lo_bin {
        return None;
    }
    let n_bins = hi_bin - lo_bin + 1;
    let n_cols = columns.len();
    let cols = n_cols.min(max_cols);
    let rows = n_bins.min(max_rows);

    // Group boundaries: group g covers [g*n/k, (g+1)*n/k).
    let span = |g: usize, n: usize, k: usize| (g * n / k, ((g + 1) * n / k).max(g * n / k + 1));

    let mut level_db = vec![MIN_DB; rows * cols];
    for c in 0..cols {
        let (c0, c1) = span(c, n_cols, cols);
        for r in 0..rows {
            let (b0, b1) = span(r, n_bins, rows);
            let mut peak = 0.0f32;
            for col in &columns[c0..c1] {
                for &m in &col.magnitudes[lo_bin + b0..lo_bin + b1] {
                    peak = peak.max(m);
                }
            }
            level_db[r * cols + c] = (20.0 * peak.log10()).max(MIN_DB);
        }
    }
    let top = level_db.iter().copied().fold(MIN_DB, f32::max);
    for v in &mut level_db {
        *v = (*v - top).max(MIN_DB);
    }

    let half_window = fft_size as f64 / 2.0 / sample_rate as f64;
    let times_s = (0..cols)
        .map(|c| {
            let (c0, c1) = span(c, n_cols, cols);
            (columns[c0].time_offset + columns[c1 - 1].time_offset) / 2.0 + half_window
        })
        .collect();
    let freqs_hz = (0..rows)
        .map(|r| {
            let (b0, b1) = span(r, n_bins, rows);
            (lo_bin as f64 + (b0 + b1 - 1) as f64 / 2.0) * bin_hz
        })
        .collect();

    Some(SurfaceGrid {
        level_db,
        cols,
        rows,
        times_s,
        freqs_hz,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, sr: u32, secs: f64) -> Vec<f32> {
        (0..(secs * sr as f64) as usize)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / sr as f64).sin() as f32)
            .collect()
    }

    #[test]
    fn peak_is_zero_db_at_the_tone() {
        let sr = 192_000;
        let g = surface_grid(
            &tone(40_000.0, sr, 0.2),
            sr,
            512,
            (20_000.0, 80_000.0),
            50,
            40,
        )
        .unwrap();
        assert_eq!(g.level_db.len(), g.rows * g.cols);
        assert!(g.cols <= 50 && g.rows <= 40);
        let (i, &max) = g
            .level_db
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        assert_eq!(max, 0.0);
        let f = g.freqs_hz[i / g.cols];
        assert!((f - 40_000.0).abs() < 2_000.0, "peak at {f} Hz");
        assert!(g.freqs_hz[0] >= 19_000.0 && *g.freqs_hz.last().unwrap() <= 81_000.0);
    }

    #[test]
    fn short_click_survives_pooling() {
        let sr = 192_000;
        let mut s = vec![0.0f32; sr as usize]; // 1 s of silence
        let burst = tone(50_000.0, sr, 0.003);
        s[96_000..96_000 + burst.len()].copy_from_slice(&burst);
        let g = surface_grid(&s, sr, 512, (0.0, 96_000.0), 40, 40).unwrap();
        // Many STFT columns merge into each grid column; the burst still reaches 0 dB.
        let col_max = |c: usize| {
            (0..g.rows)
                .map(|r| g.level_db[r * g.cols + c])
                .fold(MIN_DB, f32::max)
        };
        assert!((0..g.cols).any(|c| col_max(c) == 0.0));
        assert!((0..g.cols).filter(|&c| col_max(c) < -100.0).count() > g.cols / 2);
    }

    #[test]
    fn too_short_is_none() {
        assert!(surface_grid(&[0.0; 100], 48_000, 512, (0.0, 24_000.0), 10, 10).is_none());
    }
}

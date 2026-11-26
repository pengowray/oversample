//! Joining neighbouring spectrogram columns into one, for zoomed-out tiles.
//!
//! A zoomed-out tile shows several baseline columns per pixel column. Taking
//! the plain maximum keeps a call that fills only one of them, but the
//! maximum of several noise values is also higher than a typical one (about
//! 4 dB for 4 columns, 6.6 dB for 16), so the background drew lighter as
//! soon as the view zoomed out past a level of detail. Averaging keeps the
//! background but dims a short call by up to 10 log10(n) dB.
//!
//! [`pool_columns`] works in power and takes, per frequency bin, the larger
//! of the group's mean and its maximum divided by the expected maximum of `n`
//! noise values relative to their mean (the harmonic number H_n, for the
//! exponential distribution of noise power). Noise keeps about its level
//! (its median rises by under 2 dB, against 4.2 dB with the maximum), a
//! call that fills every column keeps its level, and a call in one column out
//! of `n` dims by 10 log10(H_n) dB: 3.2 dB for 4 columns, 5.3 dB for 16.

use crate::types::SpectrogramColumn;

/// H_n = 1 + 1/2 + ... + 1/n: the expected maximum of `n` independent
/// exponential values divided by their mean.
pub fn harmonic_number(n: usize) -> f32 {
    (1..=n).map(|k| 1.0 / k as f32).sum()
}

/// Joins each group of `factor` columns into one (see the module docs). A
/// shorter last group is dropped, as before, so a tile keeps its width.
pub fn pool_columns(cols: &[SpectrogramColumn], factor: usize) -> Vec<SpectrogramColumn> {
    if factor <= 1 || cols.is_empty() {
        return cols.to_vec();
    }
    let bias = harmonic_number(factor);
    cols.chunks_exact(factor)
        .map(|group| {
            let n_bins = group[0].magnitudes.len();
            let mut sum = vec![0.0f32; n_bins];
            let mut max = vec![0.0f32; n_bins];
            for col in group {
                for (bin, &mag) in col.magnitudes.iter().enumerate().take(n_bins) {
                    let p = mag * mag;
                    sum[bin] += p;
                    if p > max[bin] {
                        max[bin] = p;
                    }
                }
            }
            let n = group.len() as f32;
            let magnitudes = sum
                .iter()
                .zip(&max)
                .map(|(&s, &m)| (s / n).max(m / bias).sqrt())
                .collect();
            SpectrogramColumn {
                magnitudes,
                time_offset: group[0].time_offset,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noise magnitudes: the length of a complex Gaussian (Rayleigh), from a
    /// fixed-seed generator.
    fn noise_columns(n_cols: usize, n_bins: usize) -> Vec<SpectrogramColumn> {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut uniform = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ((seed >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        (0..n_cols)
            .map(|i| SpectrogramColumn {
                magnitudes: (0..n_bins)
                    .map(|_| {
                        // Box-Muller for two Gaussians, then the length.
                        let (u1, u2) = (uniform(), uniform());
                        let r = (-2.0 * u1.ln()).sqrt();
                        let re = r * (std::f64::consts::TAU * u2).cos();
                        let im = r * (std::f64::consts::TAU * u2).sin();
                        (re.hypot(im) * 1e-3) as f32
                    })
                    .collect(),
                time_offset: i as f64,
            })
            .collect()
    }

    fn median_db(cols: &[SpectrogramColumn]) -> f32 {
        let mut v: Vec<f32> = cols
            .iter()
            .flat_map(|c| c.magnitudes.iter().map(|&m| 20.0 * m.max(1e-12).log10()))
            .collect();
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    }

    fn max_pool(cols: &[SpectrogramColumn], factor: usize) -> Vec<SpectrogramColumn> {
        cols.chunks_exact(factor)
            .map(|g| SpectrogramColumn {
                magnitudes: (0..g[0].magnitudes.len())
                    .map(|b| g.iter().map(|c| c.magnitudes[b]).fold(0.0, f32::max))
                    .collect(),
                time_offset: g[0].time_offset,
            })
            .collect()
    }

    #[test]
    fn noise_keeps_about_its_level() {
        let cols = noise_columns(1024, 256);
        let base = median_db(&cols);
        for factor in [4, 16] {
            let pooled = median_db(&pool_columns(&cols, factor));
            let maxed = median_db(&max_pool(&cols, factor));
            // Joining columns evens out the speckle, which moves the median
            // toward the mean: 1.6 dB above the median for noise power.
            assert!(
                (pooled - base).abs() < 2.0,
                "factor {factor}: pooled noise median {pooled:.2} dB vs {base:.2} dB"
            );
            // The plain maximum is what made zoomed-out views lighter.
            assert!(
                maxed - base > 3.5,
                "factor {factor}: max lifts {:.2} dB",
                maxed - base
            );
        }
    }

    #[test]
    fn a_call_in_one_column_dims_by_the_harmonic_number() {
        let mut cols = vec![
            SpectrogramColumn {
                magnitudes: vec![0.0; 4],
                time_offset: 0.0,
            };
            16
        ];
        cols[5].magnitudes[2] = 1.0;
        for factor in [4, 16] {
            let pooled = pool_columns(&cols, factor);
            let got = pooled[5 / factor].magnitudes[2];
            let want = (1.0 / harmonic_number(factor)).sqrt();
            assert!(
                (got - want).abs() < 1e-6,
                "factor {factor}: {got} vs {want}"
            );
        }
    }

    #[test]
    fn a_steady_tone_keeps_its_level() {
        let cols = vec![
            SpectrogramColumn {
                magnitudes: vec![0.5, 0.25],
                time_offset: 0.0,
            };
            8
        ];
        for col in pool_columns(&cols, 4) {
            assert!((col.magnitudes[0] - 0.5).abs() < 1e-6);
            assert!((col.magnitudes[1] - 0.25).abs() < 1e-6);
        }
    }

    #[test]
    fn factor_one_and_short_groups() {
        let cols = noise_columns(10, 3);
        assert_eq!(pool_columns(&cols, 1).len(), 10);
        // 10 columns in groups of 4: the last 2 are dropped.
        let pooled = pool_columns(&cols, 4);
        assert_eq!(pooled.len(), 2);
        assert_eq!(pooled[1].time_offset, cols[4].time_offset);
    }
}

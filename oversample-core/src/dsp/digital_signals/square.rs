//! Square waves and other strongly non-sinusoidal steady tones: a steady
//! fundamental with a harmonic series that falls off about as 1/n.

use super::spectra::{BlockSpectra, Line, Run};
use super::{
    HARMONIC_MARGIN_DB, MAX_HARMONIC, MAX_JITTER_BINS, MAX_JITTER_FRACTION, MAX_SLOPE,
    MAX_SLOPE_ODD, MIN_FUNDAMENTAL_BINS, MIN_FUNDAMENTAL_HZ, ODD_ONLY_DB,
};

/// A harmonic series found in one block.
#[derive(Clone, Debug)]
pub(super) struct Series {
    pub f0: f32,
    pub lo_hz: f32,
    pub hi_hz: f32,
    pub run: Run,
    /// Harmonics matched above the fundamental.
    pub harmonics: usize,
    /// Median falloff exponent of the harmonics (1 for a square wave).
    pub slope: f32,
}

/// Debug detail on each fundamental tried, for tuning.
#[derive(Clone, Debug)]
pub struct SeriesTrial {
    pub f0: f32,
    pub matched: Vec<(usize, f32)>,
    pub slope: f32,
    pub odd_only: bool,
    pub jitter_hz: f32,
    pub unexplained: usize,
    pub accepted: bool,
    pub reason: &'static str,
}

/// Harmonic series among the block's steady lines, lowest fundamental first.
/// `steady` says which lines are steady enough to count.
pub(super) fn find_series(
    sp: &BlockSpectra,
    lines: &[Line],
    steady: &[bool],
    nyquist: f32,
    mut trials: Option<&mut Vec<SeriesTrial>>,
) -> Vec<Series> {
    let bin = sp.bin_hz;
    let f0_min = MIN_FUNDAMENTAL_HZ.max(MIN_FUNDAMENTAL_BINS * bin);
    let top = 0.97 * nyquist;
    let mut claimed = vec![false; lines.len()];
    let mut out = Vec::new();
    for i in 0..lines.len() {
        if claimed[i] || !steady[i] {
            continue;
        }
        let fund = &lines[i];
        // Needs at least the third harmonic below Nyquist.
        if fund.freq < f0_min || 3.0 * fund.freq > top {
            continue;
        }
        let nmax = ((top / fund.freq) as usize).min(MAX_HARMONIC);
        if nmax < 2 {
            continue;
        }
        // Match harmonics, refining f0 by least squares as they are found.
        let mut matched: Vec<(usize, usize)> = vec![(1, i)];
        let (mut sum_nf, mut sum_nn) = (fund.freq as f64, 1.0f64);
        for n in 2..=nmax {
            let f0 = (sum_nf / sum_nn) as f32;
            let target = n as f32 * f0;
            let tol = 0.6 * bin + 0.0005 * target;
            let best = lines
                .iter()
                .enumerate()
                .filter(|(j, l)| {
                    *j != i
                        && steady[*j]
                        && (l.freq - target).abs() <= tol
                        && l.run.overlap(&fund.run) * 2 >= fund.run.len().min(l.run.len())
                })
                .min_by(|a, b| {
                    (a.1.freq - target)
                        .abs()
                        .total_cmp(&(b.1.freq - target).abs())
                });
            if let Some((j, l)) = best {
                matched.push((n, j));
                sum_nf += n as f64 * l.freq as f64;
                sum_nn += (n * n) as f64;
            }
        }
        let f0 = (sum_nf / sum_nn) as f32;
        let above: Vec<(usize, f32)> = matched[1..]
            .iter()
            .map(|&(n, j)| (n, lines[j].level_db - fund.level_db))
            .collect();
        let mut trial = SeriesTrial {
            f0,
            matched: above.clone(),
            slope: f32::NAN,
            odd_only: false,
            jitter_hz: 0.0,
            unexplained: 0,
            accepted: false,
            reason: "",
        };
        let verdict = judge(sp, lines, &claimed, &matched, f0, nmax, &mut trial);
        trial.accepted = verdict.is_ok();
        if let Err(r) = verdict {
            trial.reason = r;
        }
        if let Some(t) = trials.as_deref_mut() {
            t.push(trial.clone());
        }
        if verdict.is_err() {
            continue;
        }
        for &(_, j) in &matched {
            claimed[j] = true;
        }
        let hi = matched
            .iter()
            .map(|&(_, j)| lines[j].freq)
            .fold(f0, f32::max);
        out.push(Series {
            f0,
            lo_hz: (f0 - 2.0 * bin).max(0.0),
            hi_hz: (hi + 2.0 * bin).min(nyquist),
            run: fund.run,
            harmonics: matched.len() - 1,
            slope: trial.slope,
        });
    }
    out
}

/// Whether a matched series is a square wave or similar; `Err` gives why
/// not.
fn judge(
    sp: &BlockSpectra,
    lines: &[Line],
    claimed: &[bool],
    matched: &[(usize, usize)],
    f0: f32,
    nmax: usize,
    trial: &mut SeriesTrial,
) -> Result<(), &'static str> {
    let fund = &lines[matched[0].1];
    let above = &trial.matched;
    if above.is_empty() {
        return Err("no harmonics");
    }
    // Per harmonic: the falloff exponent beta in level = -20 beta log10(n).
    let mut betas: Vec<f32> = above
        .iter()
        .map(|&(n, h)| -h / (20.0 * (n as f32).log10()))
        .collect();
    betas.sort_by(f32::total_cmp);
    let slope = betas[betas.len() / 2];
    trial.slope = slope;
    // Odd only: every even harmonic is missing or far under the 1/n law.
    let odd_only = above
        .iter()
        .filter(|(n, _)| n % 2 == 0)
        .all(|&(n, h)| h < -20.0 * (n as f32).log10() - ODD_ONLY_DB);
    trial.odd_only = odd_only;
    let odd: Vec<&(usize, f32)> = above.iter().filter(|(n, _)| n % 2 == 1).collect();
    // Harmonics within the margin of the 1/n law.
    let rich = above
        .iter()
        .filter(|&&(n, h)| h >= -20.0 * (n as f32).log10() - HARMONIC_MARGIN_DB)
        .count();
    let accepted_shape = (rich >= 3 && slope <= MAX_SLOPE)
        || (odd_only && odd.len() >= 2 && slope <= MAX_SLOPE_ODD)
        // A high fundamental with only its third harmonic below Nyquist.
        || (nmax <= 4
            && odd_only
            && odd.len() == 1
            && odd[0].0 == 3
            && odd[0].1 >= -20.0 * 3f32.log10() - HARMONIC_MARGIN_DB);
    if !accepted_shape {
        return Err("shape");
    }
    // Lines halfway between harmonics, or many other steady lines in the
    // series' span: the fundamental is wrong, or this is a forest of lines.
    let hi_f = matched
        .iter()
        .map(|&(_, j)| lines[j].freq)
        .fold(f0, f32::max);
    let in_series: Vec<usize> = matched.iter().map(|&(_, j)| j).collect();
    let unexplained = lines
        .iter()
        .enumerate()
        .filter(|(j, l)| {
            !claimed[*j]
                && !in_series.contains(j)
                && l.freq > f0
                && l.freq < hi_f
                && l.run.overlap(&fund.run) * 2 >= fund.run.len()
        })
        .count();
    trial.unexplained = unexplained;
    if unexplained >= matched.len() {
        return Err("unexplained lines");
    }
    // Steady pitch: the highest strong harmonics stay on their frequency.
    let mut strong: Vec<usize> = matched
        .iter()
        .map(|&(_, j)| j)
        .filter(|&j| lines[j].snr_db >= 15.0)
        .collect();
    strong.sort_by(|&a, &b| lines[b].freq.total_cmp(&lines[a].freq));
    let mut worst = 0.0f32;
    for &j in strong.iter().take(2) {
        let l = &lines[j];
        let jitter = sp.line_jitter_hz(l.bin, &fund.run.intersect(&l.run));
        let allowed = MAX_JITTER_BINS * sp.bin_hz + MAX_JITTER_FRACTION * l.freq;
        worst = worst.max(jitter / allowed);
        trial.jitter_hz = trial.jitter_hz.max(jitter);
    }
    if worst > 1.0 {
        return Err("pitch wanders");
    }
    Ok(())
}

impl Run {
    pub(super) fn intersect(&self, other: &Run) -> Run {
        let first = self.first.max(other.first);
        let last = self.last.min(other.last).max(first);
        Run {
            first,
            last,
            present: last + 1 - first,
        }
    }
}

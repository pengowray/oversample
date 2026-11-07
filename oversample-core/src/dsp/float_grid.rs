//! Detects 32-bit float audio whose samples are whole-number multiples of a
//! fixed step, which means the values came from integers and not from a
//! continuous float signal.
//!
//! A recorder that converts ADC integers to float by multiplying by a gain
//! leaves every sample on a grid: `x = n * step`. The mantissa bits look
//! random, so counting unused bits (`bit_analysis`) doesn't reveal it. Testing
//! the grid directly does, using the phase coherence
//! `|mean(exp(2*pi*i * |x| / step))|`, which we call the fit. It is 1 when every
//! sample is on the grid and about 1/sqrt(N) when none are.
//!
//! Dual-ADC recorders show two grids. The Portacapture X8 (firmware 2.03) merges
//! a high-gain word A and a low-gain word B as
//! `out = A * (1 - w) + B * 10^(D/20) * w`, with w moving between 0 and 1 over
//! 10 ms when B's envelope passes A's -3 dBFS (up) or -9 dBFS (down). So quiet
//! samples are on A's grid, loud samples are on B's grid, and samples in the fade
//! are on neither. Measured on the user's X8 recordings: both words have steps of
//! 2^-21 (22 significant bits of the 32-bit word), the step ratio equals D, and B
//! clips exactly 2^21 coarse steps from zero. Those numbers aren't assumed here:
//! the detector measures the steps and leaves device guesses to `device_hint`.
//!
//! Run this on one channel. A mono mix of two channels moves samples off both
//! channels' grids whenever one channel has switched and the other hasn't.
//!
//! A plain gain change keeps a grid and moves its step. Mixing several sources
//! or filtering removes it.

/// One measured step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridStep {
    /// Step size, where 1.0 is float full scale.
    pub step: f64,
    /// Phase coherence with this step, 0 to 1, in the band it was measured on.
    /// Below 1 when some of those samples belong to the other grid.
    pub fit: f64,
}

/// A run of samples at the file's peak, which means the signal clipped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlatCeiling {
    pub value: f64,
    /// Number of samples within 0.001% of the peak.
    pub samples: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FloatGridVerdict {
    /// Not a 32-bit float file.
    NotApplicable,
    /// Too few nonzero samples to test.
    TooFewSamples,
    /// Every sample is an exact multiple of 2^-(bits-1).
    IntegerSource { bits: u16 },
    /// Samples fit one fixed step.
    OneStep,
    /// Samples fit two fixed steps.
    TwoSteps,
    /// Samples fit no fixed step.
    NoFixedStep,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FloatGridResult {
    pub verdict: FloatGridVerdict,
    /// The smaller step. With `OneStep`, the only step.
    pub fine: Option<GridStep>,
    /// The larger step, for `TwoSteps`.
    pub coarse: Option<GridStep>,
    pub flat_ceiling: Option<FlatCeiling>,
    /// Signed bit width of a path, when a flat ceiling lands on a power of 2
    /// of the step it was reached on. Only the path that clipped is measured.
    pub path_bits: Option<u16>,
    /// Samples equal to -0.0.
    pub negative_zeros: usize,
    /// Nonzero samples the step search used.
    pub samples_checked: usize,
}

impl FloatGridResult {
    /// Ratio of coarse step to fine step in dB, for `TwoSteps`.
    pub fn step_ratio_db(&self) -> Option<f64> {
        match (self.fine, self.coarse) {
            (Some(f), Some(c)) => Some(20.0 * (c.step / f.step).log10()),
            _ => None,
        }
    }

    fn with_verdict(verdict: FloatGridVerdict) -> Self {
        Self {
            verdict,
            fine: None,
            coarse: None,
            flat_ceiling: None,
            path_bits: None,
            negative_zeros: 0,
            samples_checked: 0,
        }
    }
}

/// Fewest samples a band needs before its fit means anything. Unrelated data
/// fits at about 1/sqrt(N), so 0.07 here.
const MIN_BAND_SAMPLES: usize = 200;
/// Lowest fit accepted as a real grid. Mixed-grid files reach about 0.5 on
/// their dominant step, and unrelated data stays under 0.1.
const MIN_FIT: f64 = 0.3;
/// A second grid has to fit its own band at least this well.
const MIN_SECOND_FIT: f64 = 0.5;
/// An octave below this fit on the first step may hold a second grid.
const LOUD_OCTAVE_FIT: f64 = 0.5;
/// Steps closer than this ratio count as the same step.
const SAME_STEP_RATIO: f64 = 1.2;
/// Most samples sorted for the step search. 12M is about 30 s at 384 kHz.
const MAX_SEARCH_SAMPLES: usize = 12_000_000;
const SEARCH_BLOCK: usize = 1 << 16;
/// Most samples each fit evaluation reads.
const MAX_FIT_SAMPLES: usize = 4_000;
/// Candidate steps tried on each side of the current estimate.
const SEARCH_POINTS: i32 = 16;
/// Samples within this fraction of the peak count as being at the peak.
const CEILING_TOLERANCE: f64 = 1e-5;
const MIN_CEILING_SAMPLES: usize = 5;
/// Smallest step reported, finer than the 2^-31 of a 32-bit integer at full
/// scale. Below this, an exact fit only reflects float rounding of mixed audio.
const MIN_STEP: f64 = 1.0 / (1u64 << 33) as f64;
/// A grid needs this many distinct values in its lowest 256 grid points.
/// A short periodic waveform repeats so few values that many steps fit it.
const MIN_DISTINCT_VALUES: usize = 16;
/// Fractional part of `|x| / step` under which a sample counts as on the grid.
const ON_GRID_TOLERANCE: f64 = 0.1;
/// Largest integer width reported as an exact source depth. Above 24 bits the
/// mantissa can't hold every value, so loud samples wouldn't be exact.
const MAX_EXACT_INTEGER_BITS: i32 = 24;

/// Analyze one channel.
pub fn detect(samples: &[f32], bits_per_sample: u16, is_float: bool) -> FloatGridResult {
    if !is_float || bits_per_sample != 32 {
        return FloatGridResult::with_verdict(FloatGridVerdict::NotApplicable);
    }

    let scan = scan_samples(samples);
    if scan.nonzero < MIN_BAND_SAMPLES {
        return FloatGridResult {
            negative_zeros: scan.negative_zeros,
            ..FloatGridResult::with_verdict(FloatGridVerdict::TooFewSamples)
        };
    }

    let flat_ceiling = find_flat_ceiling(samples, scan.peak);
    let integer_bits = 1 - scan.min_quantum_exp;
    if (2..=MAX_EXACT_INTEGER_BITS).contains(&integer_bits) && scan.peak <= 1.0 {
        return FloatGridResult {
            flat_ceiling,
            negative_zeros: scan.negative_zeros,
            samples_checked: scan.nonzero,
            ..FloatGridResult::with_verdict(FloatGridVerdict::IntegerSource {
                bits: integer_bits as u16,
            })
        };
    }

    let mags = sorted_magnitudes(samples);
    let (fine, coarse) = find_steps(&mags, scan.peak);
    let verdict = match (fine, coarse) {
        (Some(_), Some(_)) => FloatGridVerdict::TwoSteps,
        (Some(_), None) => FloatGridVerdict::OneStep,
        _ => FloatGridVerdict::NoFixedStep,
    };
    let path_bits = flat_ceiling.and_then(|c| {
        path_bits_from_ceiling(c.value, coarse.map(|s| s.step).or(fine.map(|s| s.step))?)
    });

    FloatGridResult {
        verdict,
        fine,
        coarse,
        flat_ceiling,
        path_bits,
        negative_zeros: scan.negative_zeros,
        samples_checked: mags.len(),
    }
}

struct SampleScan {
    nonzero: usize,
    negative_zeros: usize,
    peak: f64,
    /// All samples are multiples of 2^min_quantum_exp.
    min_quantum_exp: i32,
}

fn scan_samples(samples: &[f32]) -> SampleScan {
    let mut scan = SampleScan {
        nonzero: 0,
        negative_zeros: 0,
        peak: 0.0,
        min_quantum_exp: i32::MAX,
    };
    for &s in samples {
        if s == 0.0 {
            if s.is_sign_negative() {
                scan.negative_zeros += 1;
            }
            continue;
        }
        if !s.is_finite() {
            continue;
        }
        scan.nonzero += 1;
        let m = s.abs() as f64;
        if m > scan.peak {
            scan.peak = m;
        }
        scan.min_quantum_exp = scan.min_quantum_exp.min(quantum_exp(s));
    }
    scan
}

/// Exponent of the largest power of 2 that divides `s` exactly.
fn quantum_exp(s: f32) -> i32 {
    let bits = s.to_bits();
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x7F_FFFF;
    let (significand, base) = if exp == 0 {
        (mantissa, -149)
    } else {
        (mantissa | (1 << 23), exp - 150)
    };
    base + significand.trailing_zeros() as i32
}

fn find_flat_ceiling(samples: &[f32], peak: f64) -> Option<FlatCeiling> {
    let floor = peak * (1.0 - CEILING_TOLERANCE);
    let count = samples
        .iter()
        .filter(|s| s.is_finite() && s.abs() as f64 >= floor)
        .count();
    (count >= MIN_CEILING_SAMPLES).then_some(FlatCeiling {
        value: peak,
        samples: count,
    })
}

fn path_bits_from_ceiling(ceiling: f64, step: f64) -> Option<u16> {
    let steps = ceiling / step;
    let exponent = steps.log2().round();
    let on_power_of_two = (steps - 2f64.powf(exponent)).abs() < 2.0;
    (on_power_of_two && (8.0..=32.0).contains(&exponent)).then_some(exponent as u16 + 1)
}

/// Magnitudes as raw f32 bits, sorted. For positive floats, sorting the bits
/// sorts the values, and this needs less memory than f64. Long inputs are
/// sampled in evenly spaced blocks so that loud passages late in the file are
/// still included.
fn sorted_magnitudes(samples: &[f32]) -> Vec<u32> {
    let keep_every = samples.len() / MAX_SEARCH_SAMPLES + 1;
    let mut mags: Vec<u32> = samples
        .chunks(SEARCH_BLOCK)
        .step_by(keep_every)
        .flatten()
        .filter(|s| **s != 0.0 && s.is_finite())
        .map(|s| s.abs().to_bits())
        .collect();
    mags.sort_unstable();
    mags
}

fn magnitude(bits: u32) -> f64 {
    f32::from_bits(bits) as f64
}

/// The part of sorted `mags` from `lo` up to but excluding `hi`.
fn band(mags: &[u32], lo: f64, hi: f64) -> &[u32] {
    let start = mags.partition_point(|&b| magnitude(b) < lo);
    let end = mags.partition_point(|&b| magnitude(b) < hi);
    &mags[start..end.max(start)]
}

/// Phase coherence of a band with a step. See the module comment.
fn fit(band: &[u32], step: f64) -> f64 {
    if band.is_empty() || step <= 0.0 {
        return 0.0;
    }
    let stride = (band.len() / MAX_FIT_SAMPLES).max(1);
    let (mut re, mut im, mut n) = (0.0f64, 0.0f64, 0usize);
    for &bits in band.iter().step_by(stride) {
        let phase = std::f64::consts::TAU * (magnitude(bits) / step);
        re += phase.cos();
        im += phase.sin();
        n += 1;
    }
    (re * re + im * im).sqrt() / n as f64
}

/// Best step near `step`, trying relative offsets up to `relative_span`.
fn search_near(band: &[u32], step: f64, relative_span: f64) -> (f64, f64) {
    let mut best = (step, fit(band, step));
    for i in -SEARCH_POINTS..=SEARCH_POINTS {
        if i == 0 {
            continue;
        }
        let candidate = step * (1.0 + relative_span * i as f64 / SEARCH_POINTS as f64);
        let f = fit(band, candidate);
        if f > best.1 {
            best = (candidate, f);
        }
    }
    best
}

fn ulp(magnitude: f64) -> f64 {
    2f64.powi(magnitude.log2().floor() as i32 - 23)
}

fn find_steps(mags: &[u32], peak: f64) -> (Option<GridStep>, Option<GridStep>) {
    let Some(first) = find_step_from_smallest(mags) else {
        return (None, None);
    };

    if let Some(larger) = find_step_in_loud_octaves(mags, first.step, peak) {
        return (Some(first), Some(larger));
    }

    // The first step may be the coarse one, in a file that is mostly loud with
    // some quiet stretches. Look at the samples that are off its grid.
    let off_grid: Vec<u32> = band(mags, 0.0, first.step * 4096.0)
        .iter()
        .copied()
        .filter(|&b| {
            let n = magnitude(b) / first.step;
            (n - n.round()).abs() > ON_GRID_TOLERANCE
        })
        .collect();
    if let Some(other) = find_step_from_smallest(&off_grid) {
        let ratio = other.step / first.step;
        if other.fit >= MIN_SECOND_FIT && !(1.0 / SAME_STEP_RATIO..SAME_STEP_RATIO).contains(&ratio)
        {
            return if other.step < first.step {
                (Some(other), Some(first))
            } else {
                (Some(first), Some(other))
            };
        }
    }

    (Some(first), None)
}

/// Finds a step from the smallest magnitudes, where float rounding is far
/// below any plausible step.
fn find_step_from_smallest(mags: &[u32]) -> Option<GridStep> {
    if mags.len() < MIN_BAND_SAMPLES {
        return None;
    }
    let mut distinct: Vec<f64> = Vec::with_capacity(400);
    for &b in mags {
        let m = magnitude(b);
        if distinct.last() != Some(&m) {
            distinct.push(m);
            if distinct.len() == 400 {
                break;
            }
        }
    }
    if distinct.len() < 32 {
        return None;
    }
    let mut gaps: Vec<f64> = distinct.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(|a, b| a.total_cmp(b));

    let mut seeds = vec![distinct[0]];
    for pct in [0, 5, 20, 50] {
        seeds.push(gaps[gaps.len() * pct / 100]);
    }

    best_step(
        seeds
            .into_iter()
            .filter_map(|seed| lock_from_zero(mags, seed)),
    )
}

/// Keeps the best fit. A step and its submultiples fit equally well, so among
/// close fits the largest step is the real one.
fn best_step(candidates: impl Iterator<Item = GridStep>) -> Option<GridStep> {
    let all: Vec<GridStep> = candidates.collect();
    let top_fit = all.iter().map(|c| c.fit).fold(0.0, f64::max);
    all.into_iter()
        .filter(|c| c.fit >= top_fit * 0.95)
        .max_by(|a, b| a.step.total_cmp(&b.step))
}

/// Refines a seed over bands that grow from zero. The allowed correction
/// shrinks as the band grows, because a relative error of 1/n in the step moves
/// the n-th multiple by a whole step.
fn lock_from_zero(mags: &[u32], seed: f64) -> Option<GridStep> {
    if seed <= 0.0 {
        return None;
    }
    let mut multiples = 64.0;
    while band(mags, 0.0, seed * multiples).len() < MIN_BAND_SAMPLES {
        multiples *= 4.0;
        if multiples > 65536.0 {
            return None;
        }
    }
    let first_band = band(mags, 0.0, seed * multiples);
    let (mut step, first_fit) = search_near(first_band, seed, 1.0 / multiples);
    if first_fit < MIN_FIT {
        return None;
    }
    step = undo_submultiple(mags, step);
    if step < MIN_STEP {
        return None;
    }

    let peak = magnitude(*mags.last()?);
    loop {
        let lo = step * multiples;
        multiples *= 4.0;
        let hi = step * multiples;
        if lo > peak || ulp(hi) > step / 16.0 {
            break;
        }
        let part = band(mags, lo, hi);
        if part.len() < MIN_BAND_SAMPLES {
            break;
        }
        let (refined, f) = search_near(part, step, 1.0 / multiples);
        if f < MIN_FIT {
            break;
        }
        step = refined;
    }
    let low_band = band(mags, 0.0, step * 256.0);
    let mut distinct = low_band.to_vec();
    distinct.dedup();
    if distinct.len() < MIN_DISTINCT_VALUES {
        return None;
    }
    let low_fit = fit(low_band, step);
    (low_fit >= MIN_FIT).then_some(GridStep { step, fit: low_fit })
}

/// Any submultiple of the real step also fits, so a seed from a tiny gap can
/// lock onto one. Its on-grid values then sit many grid points apart, so the
/// typical gap between them, in grid points, is the factor to multiply by.
fn undo_submultiple(mags: &[u32], step: f64) -> f64 {
    let wide = band(mags, 0.0, step * 65536.0);
    let mut on_grid: Vec<f64> = wide
        .iter()
        .step_by((wide.len() / MAX_FIT_SAMPLES).max(1))
        .map(|&b| magnitude(b) / step)
        .filter(|n| (n - n.round()).abs() < ON_GRID_TOLERANCE)
        .map(f64::round)
        .collect();
    on_grid.dedup();
    let mut gaps: Vec<f64> = on_grid.windows(2).map(|w| w[1] - w[0]).collect();
    if gaps.is_empty() {
        return step;
    }
    gaps.sort_by(|a, b| a.total_cmp(b));
    let factor = gaps[gaps.len() / 2];
    if factor < 2.0 {
        return step;
    }
    let larger = step * factor;
    let test_band = band(mags, 0.0, larger * 256.0);
    if fit(test_band, larger) >= fit(test_band, step) * 0.8 {
        larger
    } else {
        step
    }
}

/// Looks for a coarser step in octaves that fit `known_step` poorly.
fn find_step_in_loud_octaves(mags: &[u32], known_step: f64, peak: f64) -> Option<GridStep> {
    let mut octaves: Vec<(f64, &[u32])> = Vec::new();
    let mut lo = 2f64.powf((known_step * 64.0).log2().ceil());
    while lo <= peak {
        let part = band(mags, lo, lo * 2.0);
        if part.len() >= MIN_BAND_SAMPLES {
            let testable = ulp(lo * 2.0) <= known_step / 16.0;
            if !testable || fit(part, known_step) < LOUD_OCTAVE_FIT {
                octaves.push((lo, part));
            }
        }
        lo *= 2.0;
    }

    // The loudest octaves are the least likely to contain any quiet-path samples.
    octaves.reverse();
    let mut with_enough: Vec<_> = octaves.iter().filter(|(_, p)| p.len() >= 2000).collect();
    if with_enough.is_empty() {
        with_enough = octaves.iter().collect();
    }
    with_enough
        .into_iter()
        .take(3)
        .find_map(|&(lo, part)| step_within_octave(part, lo, known_step))
}

fn step_within_octave(part: &[u32], lo: f64, known_step: f64) -> Option<GridStep> {
    let hi = lo * 2.0;
    let rounding = ulp(hi);
    let mut distinct: Vec<f64> = part.iter().map(|&b| magnitude(b)).collect();
    distinct.dedup();
    let mut gaps: Vec<f64> = distinct
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|&g| g > 4.0 * rounding)
        .collect();
    if gaps.len() < MIN_BAND_SAMPLES / 2 {
        return None;
    }
    gaps.sort_by(|a, b| a.total_cmp(b));

    let seeds = [1, 5, 10, 25, 50].map(|pct| {
        // Averaging the gaps near this one removes most of the rounding noise.
        let seed = gaps[gaps.len() * pct / 100];
        let near: Vec<f64> = gaps
            .iter()
            .copied()
            .filter(|g| (g - seed).abs() <= (3.0 * rounding).max(seed * 0.1))
            .collect();
        near.iter().sum::<f64>() / near.len().max(1) as f64
    });

    best_step(
        seeds
            .into_iter()
            .filter_map(|seed| lock_within_octave(part, lo, hi, seed))
            .filter(|c| c.step > known_step * SAME_STEP_RATIO),
    )
}

/// Refines a seed over bands that grow upward from `lo`. Coherence magnitude
/// ignores a constant phase offset, so only the width of the band sets how
/// exact the step has to be, not its distance from zero.
fn lock_within_octave(part: &[u32], lo: f64, hi: f64, seed: f64) -> Option<GridStep> {
    if seed <= 0.0 {
        return None;
    }
    let octave_width = hi - lo;
    let density_width = octave_width * MIN_BAND_SAMPLES as f64 / part.len() as f64;
    let mut width = (32.0 * seed).max(density_width).min(octave_width);
    let mut step = seed;
    loop {
        let (refined, f) = search_near(band(part, lo, lo + width), step, step / width);
        if f < MIN_FIT {
            return None;
        }
        step = refined;
        if width >= octave_width {
            break;
        }
        width = (width * 4.0).min(octave_width);
    }
    let whole = fit(part, step);
    (whole >= MIN_SECOND_FIT).then_some(GridStep { step, fit: whole })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise with a Gaussian-like shape: xorshift, sum of 4 uniforms.
    fn noise(n: usize, rms: f64) -> Vec<f64> {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        let mut uniform = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        (0..n)
            .map(|_| (uniform() + uniform() + uniform() + uniform()) * rms * 1.732)
            .collect()
    }

    fn on_step(signal: &[f64], step: f64) -> Vec<f32> {
        signal
            .iter()
            .map(|x| ((x / step).round() * step) as f32)
            .collect()
    }

    fn assert_close(actual: f64, expected: f64, relative: f64) {
        assert!(
            ((actual - expected) / expected).abs() < relative,
            "got {actual}, expected {expected}"
        );
    }

    #[test]
    fn other_formats_are_not_applicable() {
        let r = detect(&[0.1, 0.2], 16, false);
        assert_eq!(r.verdict, FloatGridVerdict::NotApplicable);
        let r = detect(&[0.1, 0.2], 64, true);
        assert_eq!(r.verdict, FloatGridVerdict::NotApplicable);
    }

    #[test]
    fn silence_is_too_few_samples() {
        let r = detect(&vec![0.0; 10_000], 32, true);
        assert_eq!(r.verdict, FloatGridVerdict::TooFewSamples);
    }

    #[test]
    fn counts_negative_zeros() {
        let mut samples = on_step(&noise(10_000, 0.02), 0.7 * 2f64.powi(-21));
        samples[0] = -0.0;
        samples[1] = -0.0;
        samples[2] = 0.0;
        assert_eq!(detect(&samples, 32, true).negative_zeros, 2);
    }

    #[test]
    fn upconverted_integers_report_their_depth() {
        let signal = noise(200_000, 0.02);
        for (bits, step) in [(16u16, 2f64.powi(-15)), (24, 2f64.powi(-23))] {
            let r = detect(&on_step(&signal, step), 32, true);
            assert_eq!(r.verdict, FloatGridVerdict::IntegerSource { bits });
        }
    }

    #[test]
    fn integers_times_a_gain_show_one_step() {
        let step = 0.7404 * 2f64.powi(-21);
        let r = detect(&on_step(&noise(500_000, 0.02), step), 32, true);
        assert_eq!(r.verdict, FloatGridVerdict::OneStep);
        assert_close(r.fine.unwrap().step, step, 1e-5);
        assert!(r.fine.unwrap().fit > 0.95);
        assert_eq!(r.coarse, None);
    }

    #[test]
    fn scale_of_one_over_32767_shows_one_step() {
        let step = 1.0 / 32767.0;
        let r = detect(&on_step(&noise(500_000, 0.05), step), 32, true);
        assert_eq!(r.verdict, FloatGridVerdict::OneStep);
        assert_close(r.fine.unwrap().step, step, 1e-5);
    }

    #[test]
    fn filtered_float_has_no_fixed_step() {
        let signal = noise(500_000, 0.02);
        let filtered: Vec<f32> = signal
            .windows(3)
            .map(|w| (0.3 * w[0] + 0.4 * w[1] + 0.3 * w[2]) as f32)
            .collect();
        assert_eq!(
            detect(&filtered, 32, true).verdict,
            FloatGridVerdict::NoFixedStep
        );
    }

    #[test]
    fn mixing_two_grids_removes_the_step() {
        let a = on_step(&noise(500_000, 0.02), 0.7404 * 2f64.powi(-21));
        let b = on_step(&noise(500_000, 0.03)[7..], 0.9173 * 2f64.powi(-19));
        let mixed: Vec<f32> = a.iter().zip(&b).map(|(x, y)| 0.61 * x + 0.77 * y).collect();
        assert_eq!(
            detect(&mixed, 32, true).verdict,
            FloatGridVerdict::NoFixedStep
        );
    }

    /// Quiet noise on a fine grid, then a loud passage on a coarse grid that
    /// clips at `ceiling`, as on a dual-ADC recorder.
    fn two_grid_recording(ratio: f64, ceiling: f64) -> (Vec<f32>, f64, f64) {
        let fine = 0.99997 * 2f64.powi(-21);
        let coarse = fine * ratio;
        let mut out = on_step(&noise(2_000_000, 0.01), fine);
        let loud: Vec<f64> = noise(1_000_000, ceiling * 0.7)
            .iter()
            .map(|x| x.clamp(-ceiling, ceiling))
            .collect();
        out.extend(on_step(&loud, coarse));
        (out, fine, coarse)
    }

    #[test]
    fn dual_adc_recording_shows_both_steps_and_the_path_width() {
        let ratio = 10f64.powf(21.0 / 20.0);
        let ceiling = 2f64.powi(21) * 0.99997 * 2f64.powi(-21) * ratio;
        let (samples, fine, coarse) = two_grid_recording(ratio, ceiling);
        let r = detect(&samples, 32, true);
        assert_eq!(r.verdict, FloatGridVerdict::TwoSteps);
        assert_close(r.fine.unwrap().step, fine, 1e-5);
        assert_close(r.coarse.unwrap().step, coarse, 1e-5);
        assert_close(r.step_ratio_db().unwrap(), 21.0, 1e-3);
        assert!(r.flat_ceiling.is_some());
        assert_eq!(r.path_bits, Some(22));
    }

    #[test]
    fn whole_number_step_ratio_is_still_two_steps() {
        // A coarse step of exactly 10 fine steps puts coarse samples on the fine
        // grid too, so only the loud-octave search can tell them apart.
        let (samples, _, coarse) = two_grid_recording(10.0, 10.0);
        let r = detect(&samples, 32, true);
        assert_eq!(r.verdict, FloatGridVerdict::TwoSteps);
        assert_close(r.coarse.unwrap().step, coarse, 1e-4);
    }

    #[test]
    fn gain_change_moves_the_step_and_keeps_the_grid() {
        let step = 0.99997 * 2f64.powi(-21);
        let gain = 10f64.powf(-14.904 / 20.0);
        let lowered: Vec<f32> = on_step(&noise(500_000, 0.01), step)
            .iter()
            .map(|&x| (x as f64 * gain) as f32)
            .collect();
        let r = detect(&lowered, 32, true);
        assert_eq!(r.verdict, FloatGridVerdict::OneStep);
        assert_close(r.fine.unwrap().step, step * gain, 1e-5);
    }

    #[test]
    fn a_much_smaller_step_is_not_reported_when_some_samples_are_off_grid() {
        // 5% of samples off the grid lowers the real step's fit, and a tiny
        // submultiple must not win by fitting the few smallest values.
        let step = 0.99997 * 2f64.powi(-21);
        let signal = noise(500_000, 0.003);
        let mut samples = on_step(&signal, step);
        for (i, s) in samples.iter_mut().enumerate() {
            if i % 20 == 0 {
                *s = (signal[i] * 1.000_37) as f32;
            }
        }
        let r = detect(&samples, 32, true);
        assert_close(r.fine.unwrap().step, step, 1e-4);
    }

    #[test]
    fn clipping_without_a_power_of_two_gives_no_path_width() {
        let step = 0.7404 * 2f64.powi(-21);
        let mut samples = on_step(&noise(300_000, 0.02), step);
        for s in samples.iter_mut().take(20) {
            *s = 0.7;
        }
        let r = detect(&samples, 32, true);
        assert!(r.flat_ceiling.is_some());
        assert_eq!(r.path_bits, None);
    }
}

//! Digitally modulated carriers: candidate bands from the block spectra
//! (flat-topped blocks of energy, and clusters of tones that take turns),
//! then tests on the band's complex baseband signal.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::RealToComplex;

use super::spectra::{
    from_db, longest_run, opening, parabolic, percentile, to_db, BlockSpectra, Line, Run,
};
use super::{
    CARRIER_ERROR, CONSTANT_ENV_CV, FSK_FIT, FSK_MAX_ENV_CV, FSK_MAX_GAP_HZ, FSK_MAX_SPAN_HZ,
    FSK_MAX_TONES, FSK_SMOOTH_S, KAPPA_PSK, KAPPA_QAM, KAPPA_QAM_LABEL, MIN_BAND_HZ,
    MIN_STEADY_BAND_HZ, MOD_MAX_ENV_CV, PLATEAU_FLAT_DB, PLATEAU_FRAME_DB, PLATEAU_ON_DB,
    PLATEAU_SKIRT_DB, PLATEAU_TOP_DB, POWER_ALONE_DB, POWER_LINE_DB, PULSE_FAMILY_DB,
    STEADY_TONE_DB, SYMBOL_LINE_DB, SYMBOL_MAX_WIDTHS, TONE_DB,
};
use crate::dsp::fft::{plan_fft_forward, plan_fft_inverse};

/// A flat-topped block of steady energy in the mean spectrum.
#[derive(Clone, Debug)]
pub(super) struct Plateau {
    /// Where the spectrum is [`PLATEAU_ON_DB`] over the floor, in Hz.
    pub lo_hz: f32,
    pub hi_hz: f32,
    /// Its -3 dB edges (the symbol rate of a root-raised-cosine signal is
    /// the -3 dB width).
    pub lo3_hz: f32,
    pub hi3_hz: f32,
    /// Width at 10 dB under the top, Hz.
    pub b10_hz: f32,
    /// Top over the floor, dB.
    pub top_db: f32,
    pub run: Run,
}

impl Plateau {
    pub fn b3(&self) -> f32 {
        self.hi3_hz - self.lo3_hz
    }
}

/// Steady blocks of energy, at most `max_bins` wide. `run_ok` says whether
/// a run of frames is long enough.
pub(super) fn find_plateaus(
    sp: &BlockSpectra,
    max_bins: usize,
    min_bin: usize,
    run_ok: &dyn Fn(&Run) -> bool,
    mut rejects: Option<&mut Vec<String>>,
) -> Vec<Plateau> {
    let n = sp.bins;
    let bh = sp.bin_hz;
    let mut reject = |a: usize, b: usize, why: String| {
        if let Some(r) = rejects.as_deref_mut() {
            r.push(format!(
                "plateau {:.0}-{:.0} set aside: {why}",
                a as f32 * bh,
                (b + 1) as f32 * bh
            ));
        }
    };
    if n < 32 {
        return Vec::new();
    }
    let s = &sp.mean_db;
    let s3: Vec<f32> = (0..n)
        .map(|k| {
            let a = k.saturating_sub(1);
            let b = (k + 1).min(n - 1);
            s[a..=b].iter().sum::<f32>() / (b + 1 - a) as f32
        })
        .collect();
    let base = opening(&s3, max_bins / 2 + 1);
    let d: Vec<f32> = s3.iter().zip(&base).map(|(a, b)| a - b).collect();
    let edge_hi = n - 1 - (n / 50).max(3);
    let min_width = 8usize.max((400.0 / sp.bin_hz).ceil() as usize);
    let mut out = Vec::new();
    let mut k = min_bin.max(3);
    while k < edge_hi {
        if d[k] < PLATEAU_ON_DB {
            k += 1;
            continue;
        }
        let a = k;
        let mut b = k;
        let mut j = k + 1;
        while j < n {
            if d[j] >= PLATEAU_ON_DB {
                b = j;
                j += 1;
            } else if j + 2 < n && (d[j + 1] >= PLATEAU_ON_DB || d[j + 2] >= PLATEAU_ON_DB) {
                j += 1;
            } else {
                break;
            }
        }
        k = b + 1;
        let w = b + 1 - a;
        if w < min_width || b >= edge_hi {
            continue;
        }
        if w > max_bins {
            reject(a, b, "too wide".into());
            continue;
        }
        let c0 = a + w / 5;
        let c1 = b - w / 5;
        let mut mid: Vec<f32> = d[c0..=c1].to_vec();
        let top = percentile(&mut mid, 50.0);
        let hi = percentile(&mut mid, 90.0);
        let lo = percentile(&mut mid, 10.0);
        if top < PLATEAU_TOP_DB || hi - top > PLATEAU_FLAT_DB || top - lo > PLATEAU_FLAT_DB {
            reject(
                a,
                b,
                format!("top {top:.1}, {:.1} over, {:.1} under", hi - top, top - lo),
            );
            continue;
        }
        // Skirts both sides: within half the plateau's width either side the
        // spectrum falls well under its top (this does not lean on the
        // opening, which is unreliable near the ends of the spectrum, and
        // lets a neighbouring band sit close by).
        let side = (w / 2).max(3);
        let skirt_db = PLATEAU_SKIRT_DB.min(top - 3.0);
        let top_level = {
            let mut v: Vec<f32> = s3[c0..=c1].to_vec();
            percentile(&mut v, 50.0)
        };
        let lowest = |r: std::ops::Range<usize>| {
            if r.len() < 2 {
                f32::INFINITY
            } else {
                s3[r].iter().copied().fold(f32::INFINITY, f32::min)
            }
        };
        let left = lowest(a.saturating_sub(side)..a);
        let right = lowest((b + 1).min(n)..(b + 1 + side).min(n));
        if left > top_level - skirt_db || right > top_level - skirt_db {
            reject(
                a,
                b,
                format!(
                    "skirts {:.1} {:.1} under",
                    top_level - left,
                    top_level - right
                ),
            );
            continue;
        }
        let edge = |level: f32| {
            let l = (a..=b).find(|&i| d[i] >= level).unwrap_or(a);
            let h = (a..=b).rev().find(|&i| d[i] >= level).unwrap_or(b);
            (l, h)
        };
        let (l3, h3) = edge(top - 3.0);
        // The -10 dB width can reach outside the run when the top is low.
        let (l10, h10) = if top >= 13.0 {
            let lev = top - 10.0;
            let mut l = a;
            while l > 0 && d[l - 1] >= lev {
                l -= 1;
            }
            let mut h = b;
            while h + 1 < n && d[h + 1] >= lev {
                h += 1;
            }
            (l, h)
        } else {
            (a, b)
        };
        // Steady: the band's power per frame stays well over the floor's.
        let floor_sum: f32 = (l3..=h3).map(|i| from_db(base[i])).sum();
        let mean_sum: f32 = (l3..=h3).map(|i| from_db(sp.mean_db[i])).sum();
        let on = (floor_sum * from_db(PLATEAU_FRAME_DB))
            .max(mean_sum * from_db(-super::FRAME_UNDER_MEAN_DB));
        let run = longest_run((0..sp.frames).map(|f| {
            let p = sp.frame(f);
            p[l3..=h3].iter().sum::<f32>() >= on
        }));
        let Some(run) = run else { continue };
        if !run_ok(&run) || run.present * 100 < 85 * run.len() {
            reject(
                a,
                b,
                format!("run {}-{} ({})", run.first, run.last, run.present),
            );
            continue;
        }
        out.push(Plateau {
            lo_hz: a as f32 * bh,
            hi_hz: (b + 1) as f32 * bh,
            lo3_hz: (l3 as f32 - 0.5) * bh,
            hi3_hz: (h3 as f32 + 0.5) * bh,
            b10_hz: (h10 + 1 - l10) as f32 * bh,
            top_db: top,
            run,
        });
    }
    out
}

/// Narrow tones that take turns: together they are steady, each alone is
/// not.
#[derive(Clone, Debug)]
pub(super) struct Cluster {
    pub freqs: Vec<f32>,
    pub run: Run,
}

pub(super) fn find_clusters(
    sp: &BlockSpectra,
    lines: &[Line],
    skip: &[bool],
    run_ok: &dyn Fn(&Run) -> bool,
) -> Vec<Cluster> {
    let cands: Vec<&Line> = lines
        .iter()
        .zip(skip)
        .filter(|(l, &s)| !s && l.presence >= 0.05)
        .map(|(l, _)| l)
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cands.len() {
        let mut j = i;
        while j + 1 < cands.len()
            && cands[j + 1].freq - cands[j].freq <= FSK_MAX_GAP_HZ
            && cands[j + 1].freq - cands[i].freq <= FSK_MAX_SPAN_HZ
        {
            j += 1;
        }
        let group = &cands[i..=j];
        i = j + 1;
        let loudest = group
            .iter()
            .map(|l| l.level_db)
            .fold(f32::NEG_INFINITY, f32::max);
        let group: Vec<&Line> = group
            .iter()
            .copied()
            .filter(|l| l.level_db >= loudest - 15.0)
            .collect();
        if group.len() < 2 || group.len() > FSK_MAX_TONES {
            continue;
        }
        let ons: Vec<f32> = group.iter().map(|l| sp.line_threshold(l.bin)).collect();
        let run = longest_run((0..sp.frames).map(|f| {
            group
                .iter()
                .zip(&ons)
                .any(|(l, &on)| sp.line_on(f, l.bin, on))
        }));
        let Some(run) = run else { continue };
        if !run_ok(&run) || run.present * 100 < 90 * run.len() {
            continue;
        }
        out.push(Cluster {
            freqs: group.iter().map(|l| l.freq).collect(),
            run,
        });
    }
    out
}

/// The whole block's spectrum, for cutting out bands at fine resolution.
pub(super) struct BlockFft {
    pub len: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    pub output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
}

/// Share of the block tapered at each end before its FFT.
pub(super) const TAPER: f64 = 0.05;

impl BlockFft {
    pub fn new(len: usize) -> Self {
        let fft = plan_fft_forward(len);
        Self {
            len,
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
        }
    }

    /// Transform `samples` (at most `len`), tapered at both ends and
    /// zero-padded.
    pub fn run(&mut self, samples: &[f32]) {
        let n = samples.len().min(self.len);
        let ramp = ((n as f64 * TAPER) as usize).max(1);
        for (i, slot) in self.input.iter_mut().enumerate() {
            *slot = if i < n {
                let edge = i.min(n - 1 - i);
                let w = if edge < ramp {
                    let t = edge as f64 / ramp as f64;
                    (0.5 - 0.5 * (std::f64::consts::PI * t).cos()) as f32
                } else {
                    1.0
                };
                samples[i] * w
            } else {
                0.0
            };
        }
        if self
            .fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .is_err()
        {
            self.output.fill(Complex::new(0.0, 0.0));
        }
    }
}

/// A band moved down to 0 Hz as a complex signal.
pub(super) struct Baseband {
    pub z: Vec<Complex<f32>>,
    /// Samples per second.
    pub rate: f64,
    /// Input samples per baseband sample.
    pub step: f64,
    /// The frequency moved to 0 Hz.
    pub center_hz: f64,
}

/// Cut `lo_hz - pad_hz .. hi_hz + pad_hz` out of the block spectrum, weight
/// each bin by `weight(offset from centre)`, and move `center_hz` to 0 Hz.
pub(super) fn baseband(
    big: &BlockFft,
    fs: f64,
    lo_hz: f64,
    hi_hz: f64,
    pad_hz: f64,
    center_hz: f64,
    weight: &dyn Fn(f64) -> f32,
) -> Baseband {
    let l = big.len;
    let df = fs / l as f64;
    let top = l / 2 - 1;
    let k0 = (((lo_hz - pad_hz) / df).floor().max(1.0) as usize).min(top);
    let k1 = (((hi_hz + pad_hz) / df).ceil() as usize).clamp(k0, top);
    let kc = ((center_hz / df).round() as usize).clamp(k0, k1);
    let m = (3 * (k1 + 1 - k0)).next_power_of_two().max(64);
    let mut zs = vec![Complex::new(0.0f32, 0.0); m];
    for k in k0..=k1 {
        let off = k as f64 * df - kc as f64 * df;
        let w = weight(off);
        if w != 0.0 {
            let idx = (k as isize - kc as isize).rem_euclid(m as isize) as usize;
            zs[idx] = big.output[k] * w;
        }
    }
    Baseband {
        z: complex_ifft(&zs),
        rate: fs * m as f64 / l as f64,
        step: l as f64 / m as f64,
        center_hz: kc as f64 * df,
    }
}

/// 1 over `lo..hi` (offsets from `center`), falling to 0 over `pad` with a
/// raised cosine either side.
pub(super) fn band_taper(lo: f64, hi: f64, pad: f64) -> impl Fn(f64) -> f32 {
    move |f| {
        let d = if f < lo {
            lo - f
        } else if f > hi {
            f - hi
        } else {
            0.0
        };
        if d <= 0.0 {
            1.0
        } else if d >= pad {
            0.0
        } else {
            (0.5 + 0.5 * (std::f64::consts::PI * d / pad).cos()) as f32
        }
    }
}

/// Root-raised-cosine magnitude at `f` for symbol rate `rs` and roll-off
/// `alpha`.
pub(super) fn rrc(f: f64, rs: f64, alpha: f64) -> f32 {
    let f = f.abs();
    let f1 = (1.0 - alpha) * rs / 2.0;
    let f2 = (1.0 + alpha) * rs / 2.0;
    if f <= f1 {
        1.0
    } else if f >= f2 {
        0.0
    } else {
        let rc = 0.5 * (1.0 + (std::f64::consts::PI * (f - f1) / (alpha * rs)).cos());
        rc.sqrt() as f32
    }
}

/// Inverse complex FFT (unnormalised) by two real inverse FFTs.
fn complex_ifft(zs: &[Complex<f32>]) -> Vec<Complex<f32>> {
    let m = zs.len();
    let inv = plan_fft_inverse(m);
    let mut h = inv.make_input_vec();
    let mut g = inv.make_input_vec();
    for k in 0..=m / 2 {
        let a = zs[k];
        let b = zs[(m - k) % m].conj();
        h[k] = (a + b) * 0.5;
        let d = (a - b) * 0.5;
        // G = d / i.
        g[k] = Complex::new(d.im, -d.re);
    }
    for v in [&mut h, &mut g] {
        v[0].im = 0.0;
        v[m / 2].im = 0.0;
    }
    let mut hr = inv.make_output_vec();
    let mut gr = inv.make_output_vec();
    let mut scratch = inv.make_scratch_vec();
    let ok = inv
        .process_with_scratch(&mut h, &mut hr, &mut scratch)
        .is_ok()
        && inv
            .process_with_scratch(&mut g, &mut gr, &mut scratch)
            .is_ok();
    if !ok {
        return vec![Complex::new(0.0, 0.0); m];
    }
    hr.iter()
        .zip(&gr)
        .map(|(&a, &b)| Complex::new(a, b))
        .collect()
}

/// Power spectrum of a complex sequence zero-padded to `q` (a power of two),
/// by two real FFTs. Bin `k` is frequency `k` (or `k - q` above `q / 2`)
/// times `rate / q`.
fn complex_power(w: &[Complex<f32>], q: usize) -> Vec<f32> {
    let fft = plan_fft_forward(q);
    let mut re = fft.make_input_vec();
    let mut im = fft.make_input_vec();
    for (i, c) in w.iter().take(q).enumerate() {
        re[i] = c.re;
        im[i] = c.im;
    }
    let mut rs = fft.make_output_vec();
    let mut is = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    if fft
        .process_with_scratch(&mut re, &mut rs, &mut scratch)
        .is_err()
        || fft
            .process_with_scratch(&mut im, &mut is, &mut scratch)
            .is_err()
    {
        return vec![0.0; q];
    }
    let i = Complex::new(0.0f32, 1.0);
    (0..q)
        .map(|k| {
            let (r, s) = if k <= q / 2 {
                (rs[k], is[k])
            } else {
                (rs[q - k].conj(), is[q - k].conj())
            };
            (r + i * s).norm_sqr()
        })
        .collect()
}

/// Power spectrum of a real sequence zero-padded to `q`: bins 0 to `q / 2`.
fn real_power(x: &[f32], q: usize) -> Vec<f32> {
    let fft = plan_fft_forward(q);
    let mut input = fft.make_input_vec();
    for (slot, &v) in input.iter_mut().zip(x) {
        *slot = v;
    }
    let mut out = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    if fft
        .process_with_scratch(&mut input, &mut out, &mut scratch)
        .is_err()
    {
        return vec![0.0; q / 2 + 1];
    }
    out.iter().map(|c| c.norm_sqr()).collect()
}

/// How far the most power within one bin of `k` stands over the median of
/// the `half` bins either side, in dB.
fn local_line_db(spec: &[f32], k: usize, half: usize) -> f32 {
    let n = spec.len();
    if n < 3 {
        return 0.0;
    }
    let k = k.clamp(1, n - 2);
    let peak = spec[k - 1].max(spec[k]).max(spec[k + 1]);
    let a = k.saturating_sub(half);
    let b = (k + half).min(n - 1);
    let mut v: Vec<f32> = (a..=b)
        .filter(|&i| i + 2 < k || i > k + 2)
        .map(|i| spec[i])
        .collect();
    if v.is_empty() {
        return 0.0;
    }
    let med = percentile(&mut v, 50.0);
    to_db(peak as f64) - to_db(med as f64)
}

/// [`local_line_db`] on a spectrum whose ends wrap round (a complex
/// signal's, with negative frequencies at the top).
fn local_line_db_wrapped(spec: &[f32], k: usize, half: usize) -> f32 {
    let q = spec.len() as isize;
    let at = |o: isize| spec[(k as isize + o).rem_euclid(q) as usize];
    let peak = at(-1).max(at(0)).max(at(1));
    let half = (half as isize).min(q / 2 - 1);
    let mut v: Vec<f32> = (-half..=half).filter(|o| o.abs() > 2).map(at).collect();
    if v.is_empty() {
        return 0.0;
    }
    let med = percentile(&mut v, 50.0);
    to_db(peak as f64) - to_db(med as f64)
}

/// What the baseband tests found, for tuning and for the verdict.
#[derive(Clone, Debug, Default)]
pub struct ModFeatures {
    /// Symbol-rate line in the spectrum of |z|^2: its frequency and height
    /// over the local median, dB.
    pub symbol_hz: Option<f32>,
    pub symbol_db: f32,
    /// Why a symbol-rate line was set aside, if it was.
    pub symbol_reject: &'static str,
    /// Lines near 0 Hz in the spectra of z^2 and z^4, dB over the median.
    pub pow2_db: f32,
    pub pow4_db: f32,
    /// The carrier found from the power lines, Hz.
    pub carrier_hz: f32,
    /// A tone at the carrier (not suppressed), dB over the band median.
    pub tone_db: f32,
    /// Spread of |z| over its mean.
    pub env_cv: f32,
    /// E|s|^4 / (E|s|^2)^2 at the symbol instants.
    pub kappa: Option<f32>,
    /// FSK: share of time on one of the tones, and the tones used.
    pub fsk_fit: f32,
    pub fsk_tones: usize,
    pub fsk_switches: usize,
}

/// A modulated carrier found in one block.
#[derive(Clone, Debug)]
pub(super) struct ModHit {
    pub carrier_hz: f32,
    pub lo_hz: f32,
    pub hi_hz: f32,
    pub symbol_rate_hz: Option<f32>,
    pub modulation: Option<&'static str>,
}

/// The input samples a run of frames covers, kept clear of the block's
/// tapered ends, as baseband sample indices.
fn region(bb: &Baseband, run: &Run, hop: usize, n: usize, used: usize) -> (usize, usize) {
    let l = bb.step * bb.z.len() as f64;
    let ramp = used as f64 * TAPER;
    let s0 = ((run.first * hop) as f64).max(ramp);
    let s1 = ((run.last * hop + n) as f64).min(used as f64 - ramp).min(l);
    // One step in from each end: a run's end frames are partly outside it.
    let (s0, s1) = if s1 - s0 > 4.0 * hop as f64 {
        (s0 + hop as f64, s1 - hop as f64)
    } else {
        (s0, s1)
    };
    let n0 = (s0 / bb.step).ceil() as usize;
    let n1 = ((s1 / bb.step).floor() as usize).min(bb.z.len());
    (n0, n1.max(n0))
}

/// Spread of |z| over its mean, leaving out the lowest and highest 5% (a
/// run's partial frames at its ends).
fn env_cv(z: &[Complex<f32>]) -> f32 {
    let mut a: Vec<f32> = z.iter().map(|c| c.norm()).collect();
    a.sort_unstable_by(f32::total_cmp);
    let cut = a.len() / 20;
    let a = &a[cut..a.len() - cut];
    let n = a.len().max(1) as f64;
    let mut s = 0.0f64;
    let mut s2 = 0.0f64;
    for &v in a {
        s += v as f64;
        s2 += (v * v) as f64;
    }
    let mean = s / n;
    if mean <= 0.0 {
        return f32::INFINITY;
    }
    ((s2 / n - mean * mean).max(0.0).sqrt() / mean) as f32
}

/// Test a plateau for symbol structure.
#[allow(clippy::too_many_arguments)]
pub(super) fn analyze_plateau(
    big: &BlockFft,
    fs: f64,
    pl: &Plateau,
    hop: usize,
    n: usize,
    used: usize,
    feats: &mut ModFeatures,
) -> Option<ModHit> {
    if pl.b3() < MIN_BAND_HZ {
        return None;
    }
    let b3 = pl.b3() as f64;
    let center0 = 0.5 * (pl.lo3_hz + pl.hi3_hz) as f64;
    let lo = pl.lo_hz as f64;
    let hi = pl.hi_hz as f64;
    let pad = 0.3 * (hi - lo);
    let bb = baseband(
        big,
        fs,
        lo,
        hi,
        pad,
        center0,
        &band_taper(lo - center0, hi - center0, pad),
    );
    let (n0, n1) = region(&bb, &pl.run, hop, n, used);
    if n1 < n0 + 256 {
        return None;
    }
    let z = &bb.z[n0..n1];
    let p = z.iter().map(|c| c.norm_sqr() as f64).sum::<f64>() / z.len() as f64;
    if p <= 0.0 {
        return None;
    }
    feats.env_cv = env_cv(z);
    // Noise's |z| spreads by 0.52 of its mean; pulsed sound by more.
    if feats.env_cv > MOD_MAX_ENV_CV {
        return None;
    }
    let q = z.len().next_power_of_two();
    let hz = bb.rate / q as f64;
    let inv_p = (1.0 / p) as f32;

    // Symbol rate: a line in the spectrum of |z|^2.
    let e: Vec<f32> = z.iter().map(|c| c.norm_sqr() * inv_p - 1.0).collect();
    let es = real_power(&e, q);
    let kmax = ((0.45 * bb.rate / hz) as usize).min(es.len() - 2);
    let k_lo = ((0.4 * b3 / hz) as usize).max(3);
    let k_hi = ((SYMBOL_MAX_WIDTHS * b3 / hz) as usize).min(kmax);
    let mut symbol: Option<f64> = None;
    if k_hi > k_lo + 4 {
        let kp = (k_lo..=k_hi)
            .max_by(|&a, &b| es[a].total_cmp(&es[b]))
            .unwrap();
        let db = local_line_db(&es, kp, 40);
        feats.symbol_db = db;
        let near: f32 = es[kp.saturating_sub(10)..=(kp + 10).min(es.len() - 1)]
            .iter()
            .sum();
        let core: f32 = es[kp - 1..=kp + 1].iter().sum();
        if db >= SYMBOL_LINE_DB && core >= 0.5 * near {
            let d = parabolic(
                to_db(es[kp - 1] as f64),
                to_db(es[kp] as f64),
                to_db(es[kp + 1] as f64),
            );
            let rs = (kp as f64 + d as f64) * hz;
            // A pulse train has lines at fractions and multiples of its rate;
            // a symbol stream has only the one.
            let family = [rs / 2.0, rs / 3.0, 2.0 * rs]
                .iter()
                .filter(|&&f| f / hz >= 3.0 && f / hz < kmax as f64)
                .any(|&f| local_line_db(&es, (f / hz).round() as usize, 40) >= PULSE_FAMILY_DB);
            // And no stronger line below the search range.
            let lower = (((0.03 * b3) / hz) as usize).max(3)..k_lo;
            let below = lower
                .clone()
                .filter(|&k| es[k] >= es[kp] * 0.25)
                .any(|k| local_line_db(&es, k, 40) >= db - 6.0);
            if family {
                feats.symbol_reject = "pulse family";
            } else if below {
                feats.symbol_reject = "line below";
            } else {
                symbol = Some(rs);
                feats.symbol_hz = Some(rs as f32);
            }
        }
    }

    // Carrier: lines near 0 Hz in the spectra of z^2 (BPSK) and z^4 (QPSK,
    // square QAM).
    let w1: Vec<Complex<f32>> = z.iter().map(|c| c * inv_p.sqrt()).collect();
    let s1 = complex_power(&w1, q);
    let band_median = {
        let half = (0.5 * b3 / hz) as usize;
        let mut v: Vec<f32> = (0..=half)
            .flat_map(|k| [k, (q - k) % q])
            .map(|k| s1[k])
            .collect();
        percentile(&mut v, 50.0)
    };
    let mut carrier = bb.center_hz;
    let mut pow_ok = [false; 2];
    for (slot, power) in [(0usize, 2i32), (1, 4)] {
        let w: Vec<Complex<f32>> = w1.iter().map(|c| c.powi(power)).collect();
        let sp = complex_power(&w, q);
        let reach = ((power as f64 * CARRIER_ERROR * b3) / hz) as isize + 3;
        let mut best = (0isize, 0.0f32);
        for off in -reach..=reach {
            let k = off.rem_euclid(q as isize) as usize;
            if sp[k] > best.1 {
                best = (off, sp[k]);
            }
        }
        let kb = best.0.rem_euclid(q as isize) as usize;
        let half = ((0.1 * b3 / hz) as usize).clamp(20, 2000);
        let db = local_line_db_wrapped(&sp, kb, half);
        if slot == 0 {
            feats.pow2_db = db;
        } else {
            feats.pow4_db = db;
        }
        if db < POWER_LINE_DB {
            continue;
        }
        // A tone at the carrier makes a power line too; a modulated carrier
        // is suppressed.
        let f_line = best.0 as f64 * hz / power as f64;
        let kt = (f_line / hz).round() as isize;
        let tone = (kt - 2..=kt + 2)
            .map(|o| s1[o.rem_euclid(q as isize) as usize])
            .fold(0.0f32, f32::max);
        let tone_db = to_db(tone as f64) - to_db(band_median as f64);
        feats.tone_db = feats.tone_db.max(tone_db);
        if tone_db >= TONE_DB {
            continue;
        }
        pow_ok[slot] = true;
        if !pow_ok[0] || slot == 0 {
            carrier = bb.center_hz + f_line;
        }
    }
    feats.carrier_hz = carrier as f32;
    // Without a symbol rate, only a strong power line or a steady envelope
    // will do.
    let pow_db = feats.pow2_db.max(feats.pow4_db);
    // A steady envelope with no tone in the band (a tone in noise has a
    // steady envelope too).
    let in_band_peak = {
        let half = (0.5 * b3 / hz) as usize;
        (0..=half)
            .flat_map(|k| [k, (q - k) % q])
            .map(|k| s1[k])
            .fold(0.0f32, f32::max)
    };
    feats.tone_db = feats
        .tone_db
        .max(to_db(in_band_peak as f64) - to_db(band_median as f64));
    let steady_envelope = feats.env_cv <= CONSTANT_ENV_CV
        && pl.b3() >= MIN_STEADY_BAND_HZ
        && to_db(in_band_peak as f64) - to_db(band_median as f64) < STEADY_TONE_DB;
    if symbol.is_none()
        && !(pow_ok.iter().any(|&o| o) && pow_db >= POWER_ALONE_DB)
        && !steady_envelope
    {
        return None;
    }

    // Constellation shape at the symbol instants.
    if let Some(rs) = symbol {
        let alpha = if pl.top_db >= 13.0 {
            ((pl.b10_hz as f64 / rs - 1.0) / 0.59).clamp(0.1, 1.0)
        } else {
            0.35
        };
        let mf = baseband(big, fs, lo, hi, pad, carrier, &|f| rrc(f, rs, alpha));
        let (m0, m1) = region(&mf, &pl.run, hop, n, used);
        if m1 > m0 + 256 {
            feats.kappa = kappa_at_symbols(&mf.z[m0..m1], mf.rate, rs);
        }
    }
    let modulation = if pow_ok[0] {
        Some("PSK")
    } else {
        match feats.kappa {
            Some(k) if k < KAPPA_PSK => Some("PSK"),
            Some(k) if k < KAPPA_QAM_LABEL => Some("QAM"),
            _ => None,
        }
    };
    let symbols_shaped = feats.kappa.is_some_and(|k| k < KAPPA_QAM);
    if symbol.is_some() && !pow_ok[0] && !pow_ok[1] && !symbols_shaped && !steady_envelope {
        // A symbol-rate line on something noise-like at the instants.
        return None;
    }
    Some(ModHit {
        carrier_hz: carrier as f32,
        lo_hz: pl.lo_hz,
        hi_hz: pl.hi_hz,
        symbol_rate_hz: symbol.map(|v| v as f32),
        modulation,
    })
}

/// Fourth moment over squared second moment of `z` sampled once per symbol,
/// at the phase where |z|^2 peaks (found from the symbol-rate component of
/// |z|^2, with the rate refined to the strongest).
fn kappa_at_symbols(z: &[Complex<f32>], rate: f64, rs: f64) -> Option<f32> {
    let n = z.len();
    let dur = n as f64 / rate;
    let e: Vec<f64> = z.iter().map(|c| c.norm_sqr() as f64).collect();
    let mut best = (rs, 0.0f64, 0.0f64);
    for j in -6..=6 {
        let r = rs + j as f64 * 0.1 / dur;
        let (mut cr, mut ci) = (0.0f64, 0.0f64);
        let w = std::f64::consts::TAU * r / rate;
        for (i, &v) in e.iter().enumerate() {
            let a = w * i as f64;
            cr += v * a.cos();
            ci -= v * a.sin();
        }
        let mag = cr.hypot(ci);
        if mag > best.1 {
            best = (r, mag, ci.atan2(cr));
        }
    }
    let (r, _, psi) = best;
    // |z|^2 ~ m + A cos(2 pi r t + psi): peaks at t = (k - psi / 2 pi) / r.
    let per = rate / r;
    let first = -psi / std::f64::consts::TAU * per;
    let mut s2 = 0.0f64;
    let mut s4 = 0.0f64;
    let mut count = 0usize;
    let mut k = ((4.0 * per - first) / per).ceil();
    loop {
        let t = first + k * per;
        if t >= n as f64 - 4.0 * per - 2.0 {
            break;
        }
        if t >= 2.0 {
            let v = cubic(z, t).norm_sqr() as f64;
            s2 += v;
            s4 += v * v;
            count += 1;
        }
        k += 1.0;
    }
    if count < 32 || s2 <= 0.0 {
        return None;
    }
    let m2 = s2 / count as f64;
    Some((s4 / count as f64 / (m2 * m2)) as f32)
}

/// Catmull-Rom interpolation of `z` at fractional index `t`.
fn cubic(z: &[Complex<f32>], t: f64) -> Complex<f32> {
    let i = t.floor() as usize;
    let u = (t - i as f64) as f32;
    let at = |j: isize| z[(i as isize + j).clamp(0, z.len() as isize - 1) as usize];
    let (p0, p1, p2, p3) = (at(-1), at(0), at(1), at(2));
    let u2 = u * u;
    let u3 = u2 * u;
    (p1 * 2.0
        + (p2 - p0) * u
        + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * u2
        + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * u3)
        * 0.5
}

/// Test a cluster of tones for frequency-shift keying: one tone at a time,
/// steady loudness.
#[allow(clippy::too_many_arguments)]
pub(super) fn analyze_cluster(
    big: &BlockFft,
    fs: f64,
    cl: &Cluster,
    bin_hz: f32,
    hop: usize,
    n: usize,
    used: usize,
    feats: &mut ModFeatures,
) -> Option<ModHit> {
    let fmin = cl.freqs.iter().copied().fold(f32::INFINITY, f32::min) as f64;
    let fmax = cl.freqs.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let spacing = cl
        .freqs
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64)
        .fold(f64::INFINITY, f64::min);
    let margin = (3.0 * bin_hz as f64).max(0.5 * spacing);
    let lo = fmin - margin;
    let hi = fmax + margin;
    let center = 0.5 * (fmin + fmax);
    let pad = 2.0 * bin_hz as f64;
    let bb = baseband(
        big,
        fs,
        lo,
        hi,
        pad,
        center,
        &band_taper(lo - center, hi - center, pad),
    );
    let (n0, n1) = region(&bb, &cl.run, hop, n, used);
    if n1 < n0 + 256 {
        return None;
    }
    let z = &bb.z[n0..n1];
    feats.env_cv = env_cv(z);
    if feats.env_cv > FSK_MAX_ENV_CV {
        return None;
    }
    // Instantaneous frequency, averaged over FSK_SMOOTH_S.
    let k = ((bb.rate * FSK_SMOOTH_S).round() as usize).max(1);
    let d: Vec<Complex<f32>> = z.windows(2).map(|w| w[1] * w[0].conj()).collect();
    if d.len() <= k {
        return None;
    }
    let tol = (0.75 * bin_hz as f64)
        .max(0.3 * spacing)
        .min(0.45 * spacing);
    let mut acc: Complex<f32> = d[..k].iter().sum();
    let mut total = 0.0f64;
    let mut fit = 0.0f64;
    let mut share = vec![0.0f64; cl.freqs.len()];
    let mut labels: Vec<u8> = Vec::with_capacity(d.len() - k);
    let to_hz = bb.rate / std::f64::consts::TAU;
    for i in 0..d.len() - k {
        if i > 0 {
            acc += d[i + k - 1] - d[i - 1];
        }
        let w = acc.norm() as f64;
        let f = acc.im.atan2(acc.re) as f64 * to_hz + bb.center_hz;
        total += w;
        let (j, dist) = cl
            .freqs
            .iter()
            .enumerate()
            .map(|(j, &t)| (j, (f - t as f64).abs()))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        if dist <= tol {
            fit += w;
            share[j] += w;
            labels.push(j as u8);
        } else {
            labels.push(u8::MAX);
        }
    }
    if total <= 0.0 {
        return None;
    }
    feats.fsk_fit = (fit / total) as f32;
    feats.fsk_tones = share.iter().filter(|&&s| s >= 0.1 * total).count();
    // Runs on each tone (samples off every tone count towards the run
    // they fall in); runs shorter than the averaging are folded into the
    // run before, then neighbours on the same tone joined.
    let mut runs: Vec<(u8, usize)> = Vec::new();
    for &lab in &labels {
        match runs.last_mut() {
            Some(r) if lab == u8::MAX || lab == r.0 => r.1 += 1,
            _ if lab == u8::MAX => {}
            _ => runs.push((lab, 1)),
        }
    }
    let mut joined: Vec<(u8, usize)> = Vec::with_capacity(runs.len());
    for (lab, len) in runs {
        match joined.last_mut() {
            Some(r) if len < k || r.0 == lab => r.1 += len,
            _ => joined.push((lab, len)),
        }
    }
    let switches = joined.len().saturating_sub(1);
    // The first and last runs are cut by the ends of the region.
    let dwells: Vec<usize> = if joined.len() > 2 {
        joined[1..joined.len() - 1].iter().map(|r| r.1).collect()
    } else {
        Vec::new()
    };
    feats.fsk_switches = switches;
    if feats.fsk_fit < FSK_FIT || feats.fsk_tones < 2 || switches < 3 {
        return None;
    }
    let symbol_rate_hz = symbol_from_dwells(&dwells).map(|t| (bb.rate / t) as f32);
    Some(ModHit {
        carrier_hz: center as f32,
        lo_hz: (fmin - 2.0 * bin_hz as f64) as f32,
        hi_hz: (fmax + 2.0 * bin_hz as f64) as f32,
        symbol_rate_hz,
        modulation: Some("FSK"),
    })
}

/// The symbol length that the dwell times are whole multiples of, in
/// samples, when most of them are.
fn symbol_from_dwells(dwells: &[usize]) -> Option<f64> {
    if dwells.len() < 4 {
        return None;
    }
    let mut v: Vec<f32> = dwells.iter().map(|&d| d as f32).collect();
    let t0 = percentile(&mut v, 10.0) as f64;
    if t0 <= 0.0 {
        return None;
    }
    let mut num = 0.0f64;
    let mut den = 0.0f64;
    let mut good = 0usize;
    for &d in dwells {
        let r = d as f64 / t0;
        let m = r.round().max(1.0);
        if (r - m).abs() <= 0.25 {
            good += 1;
            num += d as f64;
            den += m;
        }
    }
    (good * 10 >= dwells.len() * 7 && den > 0.0).then(|| num / den)
}

//! One block's frame spectra, its mean spectrum, floors, narrow lines and
//! runs of frames.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::RealToComplex;

use super::{FRAME_UNDER_MEAN_DB, LINE_DB, LINE_FRAME_DB, LINE_SHOULDER_DB, MAX_DROPOUT_FRAMES};
use crate::dsp::fft::{hann_window, plan_fft_forward};

/// Bins per segment for the line floor.
const FLOOR_SEGMENT: usize = 32;
/// The line floor is this percentile of each segment's levels.
const FLOOR_PERCENTILE: f32 = 25.0;

/// A Hann-windowed frame FFT, its buffers, and the power scale.
pub(super) struct FrameFft {
    pub n: usize,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Scales bin power so a full-scale sine's peak bin is 0 dB.
    power_scale: f32,
}

impl FrameFft {
    pub fn new(n: usize) -> Self {
        let fft = plan_fft_forward(n);
        let window = hann_window(n);
        let wsum: f32 = window.iter().sum();
        Self {
            n,
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window,
            power_scale: 4.0 / (wsum * wsum),
        }
    }

    /// Power spectrum of `frame` (length `n`) into `out` (`n / 2 + 1` bins).
    pub fn power(&mut self, frame: &[f32], out: &mut [f32]) {
        for ((slot, &x), &w) in self.input.iter_mut().zip(frame).zip(&self.window) {
            *slot = x * w;
        }
        if self
            .fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .is_err()
        {
            out.fill(0.0);
            return;
        }
        for (o, c) in out.iter_mut().zip(&self.output) {
            *o = c.norm_sqr() * self.power_scale;
        }
    }
}

/// A narrow peak in a block's mean spectrum.
#[derive(Clone, Debug)]
pub(super) struct Line {
    pub bin: usize,
    /// Interpolated frequency, in Hz.
    pub freq: f32,
    /// Level in the mean spectrum, dB.
    pub level_db: f32,
    /// Level over the line floor, dB.
    pub snr_db: f32,
    /// The longest run of frames the line is present in, and how many of
    /// its frames it is present in.
    pub run: Run,
    /// The share of all the block's frames the line is present in.
    pub presence: f32,
}

/// A run of frames, first to last inclusive, with short dropouts bridged.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Run {
    pub first: usize,
    pub last: usize,
    pub present: usize,
}

impl Run {
    pub fn len(&self) -> usize {
        self.last + 1 - self.first
    }

    pub fn overlap(&self, other: &Run) -> usize {
        let a = self.first.max(other.first);
        let b = self.last.min(other.last);
        if b >= a {
            b + 1 - a
        } else {
            0
        }
    }
}

/// The longest run of `true` in `present`, bridging up to
/// [`MAX_DROPOUT_FRAMES`] missing frames in a row.
pub(super) fn longest_run(present: impl Iterator<Item = bool>) -> Option<Run> {
    let mut best: Option<Run> = None;
    let mut cur: Option<Run> = None;
    let mut misses = 0usize;
    for (f, on) in present.enumerate() {
        if on {
            match &mut cur {
                Some(r) => {
                    r.last = f;
                    r.present += 1;
                }
                None => {
                    cur = Some(Run {
                        first: f,
                        last: f,
                        present: 1,
                    })
                }
            }
            misses = 0;
        } else if let Some(r) = cur {
            misses += 1;
            if misses > MAX_DROPOUT_FRAMES {
                if best.is_none_or(|b| r.len() > b.len()) {
                    best = Some(r);
                }
                cur = None;
                misses = 0;
            }
        }
    }
    if let Some(r) = cur {
        if best.is_none_or(|b| r.len() > b.len()) {
            best = Some(r);
        }
    }
    best
}

/// One block's spectra: every frame's power, and summaries of them.
pub(super) struct BlockSpectra<'a> {
    /// `frames` rows of `bins` powers.
    pub power: &'a [f32],
    pub frames: usize,
    pub bins: usize,
    pub bin_hz: f32,
    /// Mean power over the frames, in dB.
    pub mean_db: Vec<f32>,
    /// The floor narrow lines are measured against, in dB.
    pub floor_db: Vec<f32>,
}

impl<'a> BlockSpectra<'a> {
    pub fn new(power: &'a [f32], frames: usize, bins: usize, bin_hz: f32) -> Self {
        let mut mean = vec![0.0f64; bins];
        for f in 0..frames {
            for (m, &p) in mean.iter_mut().zip(&power[f * bins..(f + 1) * bins]) {
                *m += p as f64;
            }
        }
        let inv = 1.0 / frames.max(1) as f64;
        let mean_db: Vec<f32> = mean.iter().map(|&m| to_db(m * inv)).collect();
        let floor_db = segment_floor(&mean_db);
        Self {
            power,
            frames,
            bins,
            bin_hz,
            mean_db,
            floor_db,
        }
    }

    #[inline]
    pub fn frame(&self, f: usize) -> &[f32] {
        &self.power[f * self.bins..(f + 1) * self.bins]
    }

    /// Whether a line at `bin` is present in frame `f`: the most power within
    /// one bin of it is [`LINE_FRAME_DB`] over the line floor.
    pub fn line_on(&self, f: usize, bin: usize, floor_lin: f32) -> bool {
        let p = self.frame(f);
        let lo = bin.saturating_sub(1);
        let hi = (bin + 1).min(self.bins - 1);
        p[lo..=hi].iter().any(|&v| v >= floor_lin)
    }

    /// Power within one bin of a line at `bin` that counts as present in a
    /// frame: [`LINE_FRAME_DB`] over the floor, and no more than
    /// [`FRAME_UNDER_MEAN_DB`] under the line's mean level (so stray noise
    /// next to a strong line's run is not counted as the line).
    pub fn line_threshold(&self, bin: usize) -> f32 {
        from_db(
            (self.floor_db[bin] + LINE_FRAME_DB).max(self.mean_db[bin] - FRAME_UNDER_MEAN_DB),
        )
    }

    /// Narrow peaks in the mean spectrum at least [`LINE_DB`] over the line
    /// floor, from `min_bin` up, with their runs of frames.
    pub fn lines(&self, min_bin: usize, max_bin: usize) -> Vec<Line> {
        let s = &self.mean_db;
        let mut out = Vec::new();
        let lo = min_bin.max(3);
        let hi = max_bin.min(self.bins.saturating_sub(4));
        let mut k = lo;
        while k <= hi {
            let v = s[k];
            let snr = v - self.floor_db[k];
            let is_peak = snr >= LINE_DB
                && v > s[k - 1]
                && v >= s[k + 1]
                && v >= s[k - 2]
                && v >= s[k + 2];
            if !is_peak {
                k += 1;
                continue;
            }
            // Narrow: three bins away the spectrum is near the floor, or well
            // under the peak.
            let shoulder = s[k - 3].max(s[k + 3]) - self.floor_db[k];
            if shoulder > (snr - LINE_SHOULDER_DB).max(3.0) {
                k += 1;
                continue;
            }
            let freq = (k as f32 + parabolic(s[k - 1], v, s[k + 1])) * self.bin_hz;
            let on = self.line_threshold(k);
            let mut count = 0usize;
            let run = longest_run((0..self.frames).map(|f| {
                let p = self.line_on(f, k, on);
                count += p as usize;
                p
            }));
            out.push(Line {
                bin: k,
                freq,
                level_db: v,
                snr_db: snr,
                run: run.unwrap_or_default(),
                presence: count as f32 / self.frames.max(1) as f32,
            });
            k += 3;
        }
        out
    }

    /// Per-frame interpolated frequency of a line near `bin` over `run`, and
    /// its standard deviation in Hz.
    pub fn line_jitter_hz(&self, bin: usize, run: &Run) -> f32 {
        if bin < 2 || bin + 2 >= self.bins {
            return 0.0;
        }
        let mut sum = 0.0f64;
        let mut sum2 = 0.0f64;
        let mut n = 0usize;
        for f in run.first..=run.last {
            let p = self.frame(f);
            let mut k = bin;
            for j in bin - 1..=bin + 1 {
                if p[j] > p[k] {
                    k = j;
                }
            }
            if p[k] <= 0.0 {
                continue;
            }
            let d = parabolic(
                to_db(p[k - 1] as f64),
                to_db(p[k] as f64),
                to_db(p[k + 1] as f64),
            );
            let x = (k as f64 + d as f64) * self.bin_hz as f64;
            sum += x;
            sum2 += x * x;
            n += 1;
        }
        if n < 2 {
            return 0.0;
        }
        let mean = sum / n as f64;
        ((sum2 / n as f64 - mean * mean).max(0.0)).sqrt() as f32
    }
}

/// Offset of a peak from the middle of three levels, in bins (-0.5 to 0.5).
pub(super) fn parabolic(a: f32, b: f32, c: f32) -> f32 {
    let d = a - 2.0 * b + c;
    if d.abs() < 1e-12 {
        return 0.0;
    }
    (0.5 * (a - c) / d).clamp(-0.5, 0.5)
}

pub(super) fn to_db(p: f64) -> f32 {
    (10.0 * (p + 1e-30).log10()) as f32
}

pub(super) fn from_db(db: f32) -> f32 {
    10f32.powf(db / 10.0)
}

/// The value at percentile `q` (0 to 100) of `v`, which is reordered.
pub(super) fn percentile(v: &mut [f32], q: f32) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    let i = (((q / 100.0) * (v.len() - 1) as f32).round() as usize).min(v.len() - 1);
    let (_, x, _) = v.select_nth_unstable_by(i, f32::total_cmp);
    *x
}

/// A floor under narrow peaks: the [`FLOOR_PERCENTILE`]th percentile of each
/// [`FLOOR_SEGMENT`] bins, joined by straight lines between segment middles.
fn segment_floor(s: &[f32]) -> Vec<f32> {
    let n = s.len();
    let segs = n.div_ceil(FLOOR_SEGMENT).max(1);
    let mut mids = Vec::with_capacity(segs);
    let mut vals = Vec::with_capacity(segs);
    let mut tmp = Vec::with_capacity(FLOOR_SEGMENT);
    for g in 0..segs {
        let a = g * FLOOR_SEGMENT;
        let b = ((g + 1) * FLOOR_SEGMENT).min(n);
        if b <= a {
            break;
        }
        tmp.clear();
        tmp.extend_from_slice(&s[a..b]);
        vals.push(percentile(&mut tmp, FLOOR_PERCENTILE));
        mids.push((a + b - 1) as f32 / 2.0);
    }
    let mut out = vec![0.0f32; n];
    let mut g = 0usize;
    for (k, o) in out.iter_mut().enumerate() {
        let x = k as f32;
        while g + 1 < mids.len() && mids[g + 1] < x {
            g += 1;
        }
        *o = if mids.len() == 1 || x <= mids[0] {
            vals[0]
        } else if g + 1 >= mids.len() {
            vals[mids.len() - 1]
        } else {
            let t = (x - mids[g]) / (mids[g + 1] - mids[g]);
            vals[g] + t * (vals[g + 1] - vals[g])
        };
    }
    out
}

/// Running minimum (`max == false`) or maximum over a centred window of
/// `2 * half + 1` values, clipped at the ends, in linear time (van Herk /
/// Gil-Werman).
pub(super) fn running_extreme(x: &[f32], half: usize, max: bool) -> Vec<f32> {
    let n = x.len();
    if n == 0 {
        return Vec::new();
    }
    let w = 2 * half + 1;
    let pick = |a: f32, b: f32| if max { a.max(b) } else { a.min(b) };
    let pad = if max { f32::NEG_INFINITY } else { f32::INFINITY };
    // Padded input: `half` pad values each side.
    let len = n + 2 * half;
    let at = |i: usize| {
        if i < half || i >= half + n {
            pad
        } else {
            x[i - half]
        }
    };
    let mut fwd = vec![pad; len];
    let mut bwd = vec![pad; len];
    for i in 0..len {
        fwd[i] = if i % w == 0 { at(i) } else { pick(fwd[i - 1], at(i)) };
    }
    for i in (0..len).rev() {
        bwd[i] = if i % w == w - 1 || i == len - 1 {
            at(i)
        } else {
            pick(bwd[i + 1], at(i))
        };
    }
    // Window for output k covers padded i = k .. k + w - 1.
    (0..n)
        .map(|k| {
            let j = k + w - 1;
            if j >= len {
                bwd[k]
            } else {
                pick(bwd[k], fwd[j])
            }
        })
        .collect()
}

/// Morphological opening of `x` with a flat window of `2 * half + 1`: the
/// floor under every feature narrower than the window.
pub(super) fn opening(x: &[f32], half: usize) -> Vec<f32> {
    let eroded = running_extreme(x, half, false);
    running_extreme(&eroded, half, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(x: &[f32], half: usize, max: bool) -> Vec<f32> {
        (0..x.len())
            .map(|k| {
                let a = k.saturating_sub(half);
                let b = (k + half).min(x.len() - 1);
                let it = x[a..=b].iter().copied();
                if max {
                    it.fold(f32::NEG_INFINITY, f32::max)
                } else {
                    it.fold(f32::INFINITY, f32::min)
                }
            })
            .collect()
    }

    #[test]
    fn running_extreme_matches_naive() {
        let x: Vec<f32> = (0..97)
            .map(|i| ((i * 37 % 23) as f32 - 11.0) * 0.7)
            .collect();
        for half in [0, 1, 2, 5, 13, 60] {
            for max in [false, true] {
                assert_eq!(running_extreme(&x, half, max), naive(&x, half, max));
            }
        }
    }

    #[test]
    fn longest_run_bridges_short_dropouts() {
        let on = [
            true, false, true, true, false, false, true, true, false, false, false, false,
            true,
        ];
        let r = longest_run(on.iter().copied()).unwrap();
        assert_eq!((r.first, r.last, r.present), (0, 7, 5));
    }
}

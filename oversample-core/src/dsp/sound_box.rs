//! Estimates a box round the most prominent sound in a stretch of audio: when
//! it starts and ends, and its lowest and highest frequency.
//!
//! BirdNET-style classifiers score fixed windows (3 s) and name a class, but
//! say nothing about where in the window or at what frequencies the sound is.
//! This finds the sound in the spectrum of the stretch, measured against the
//! stretch's own background, so the app can draw a box round it.
//!
//! Method:
//! 1. Short-time spectrum with bins of 30 to 47 Hz and a hop of half the
//!    window. High rates are first halved (half-band FIR) down to about 2.5
//!    times the top of `band`. Only the bins inside `band` are kept. Power is
//!    smoothed over 3 bins and 3 frames, so noise reads as a steady level.
//!    Frames of digital silence are left out.
//! 2. For each bin: a loud level (3% of frames are louder), a middle level
//!    and a quiet level (20th percentile).
//! 3. The background is the quiet levels with every peak narrower than an
//!    octave removed (a morphological opening across frequency). A sound that
//!    comes and goes stands above its own bin's quiet level; a narrow steady
//!    sound (a cricket singing the whole stretch) stands above the
//!    neighbouring bins.
//! 4. Excess = loud level minus background. Noise alone gives some excess
//!    too (its "spread", measured on the stretch itself), and a peak needs
//!    6 dB more than that to count.
//! 5. Regions grow from the highest peaks, down and up while the excess stays
//!    within 20 dB of the peak. Regions that pulse together are grouped as
//!    one sound (harmonics, broadband calls in pieces). The group with the most
//!    excess, summed over the mel scale, is the sound; its box spans its
//!    regions.
//! 6. Time: the energy in those regions, frame by frame, against their
//!    background. The box runs from the first to the last run of frames within
//!    12 dB of the sound's loud frames.
//!
//! A sound that is steady over the whole stretch and wider than an octave
//! (constant rain, an engine at one speed) cannot be told from the
//! background this way: it gives `None`, or a box round something else.

use super::fft::{hann_window, plan_fft_forward};

/// A box round a sound: seconds from the start of the stretch, and Hz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundBox {
    pub start_s: f64,
    pub end_s: f64,
    pub lo_hz: f32,
    pub hi_hz: f32,
}

/// Widest bin to aim for; the FFT size is the power of two that gives this or
/// narrower.
const TARGET_BIN_HZ: f32 = 50.0;
/// The rate is halved while the halved rate stays at least this many times
/// the top of the band.
const DECIMATE_HEADROOM: f32 = 2.5;
/// Power below this (relative to a full-scale sine) counts as digital silence.
const SILENT_POWER: f32 = 1e-18;
/// Floor for dB values, so a near-silent bin cannot give a huge excess.
const FLOOR_DB: f32 = -160.0;
/// Percentile over time taken as a bin's quiet level.
const QUIET_PERCENTILE: f32 = 0.20;
/// Share of frames above a bin's loud level (at least `LOUD_MIN_FRAMES`, at
/// most `LOUD_MAX_FRAMES`). Small, so short calls still count.
const LOUD_SHARE: f32 = 0.03;
const LOUD_MIN_FRAMES: usize = 2;
const LOUD_MAX_FRAMES: usize = 40;
/// Width of the background's peak removal, in octaves: steady sounds
/// narrower than this stand out from it.
const BACKGROUND_OCTAVES: f32 = 1.0;
/// How far noise alone rises above a bin's quiet level is measured as this
/// percentile, over the bins, of loud minus quiet level. Most bins hold no
/// sound, and the measure follows the background's own unsteadiness.
const SPREAD_PERCENTILE: f32 = 0.25;
/// In noise, the rise from quiet to loud level is about this many times the
/// rise from quiet to middle level.
const NOISE_RISE_RATIO: f32 = 3.0;
/// Excess a peak needs to count as a sound, in dB above the noise's spread.
const DETECT_DB: f32 = 6.0;
/// Excess a bin needs to be part of the box, in dB above the noise's spread.
const KEEP_DB: f32 = 3.0;
/// A region extends while the excess is within this of its peak's.
const EXTEND_DB: f32 = 20.0;
/// Most regions looked at.
const MAX_REGIONS: usize = 12;
/// Regions whose levels over time correlate at least this well are one sound.
const SAME_SOUND_CORRELATION: f32 = 0.6;
/// A region comes and goes (rather than being steady) when its loud frames
/// are at least this far above its quiet ones, in dB.
const PULSED_DB: f32 = 6.0;
/// Bins below the extension threshold the box may cross.
const MAX_GAP_BINS: usize = 2;
/// A frame is part of the sound when its box energy is within this of the
/// sound's loud frames, and above the noise's spread.
const TIME_RANGE_DB: f32 = 12.0;
/// Fewest frames in a row above the threshold that count as sound.
const MIN_RUN_FRAMES: usize = 2;
/// Seconds added before the start and after the end.
const PAD_S: f64 = 0.02;

/// `samples`: mono audio of one stretch (up to about 30 s), starting at time 0.
/// `band`: the frequencies the model listens to, such as (250.0, 15_000.0) Hz.
/// Returns the box of the most prominent sound in the stretch, or `None` when
/// nothing stands out from the stretch's own background.
pub fn estimate(samples: &[f32], sample_rate: u32, band: (f32, f32)) -> Option<SoundBox> {
    let spec = Spectrum::new(samples, sample_rate, band)?;
    let levels = spec.levels();
    let background = opening(&levels.quiet, &spec.background_windows());
    let excess: Vec<f32> = levels
        .loud
        .iter()
        .zip(&background)
        .map(|(l, b)| l - b)
        .collect();
    let spread = levels.noise_spread();

    let parts = pick_sound(&spec, &excess, &background, spread)?;
    let k_lo = parts.iter().map(|r| r.0).min()?;
    let k_hi = parts.iter().map(|r| r.1).max()?;
    let (t_first, t_last) = spec.time_extent(&parts, &background, spread)?;

    let bin_hz = spec.bin_hz;
    let hop_s = spec.hop as f64 / spec.rate as f64;
    let half_fft_s = spec.fft_size as f64 / 2.0 / spec.rate as f64;
    let duration = samples.len() as f64 / sample_rate as f64;
    let band_lo = band.0.min(band.1);
    let band_hi = band.0.max(band.1).min(sample_rate as f32 / 2.0);
    // Frame t's window covers [t*hop, t*hop + fft); the 3-frame smoothing adds
    // one hop each side.
    let mut start_s = t_first as f64 * hop_s + half_fft_s - hop_s - PAD_S;
    let mut end_s = t_last as f64 * hop_s + half_fft_s + hop_s + PAD_S;
    // A sound that reaches the first or last frame runs to the stretch's edge.
    if t_first == 0 {
        start_s = 0.0;
    }
    if t_last + 1 == spec.n_frames {
        end_s = duration;
    }
    Some(SoundBox {
        start_s: start_s.clamp(0.0, duration),
        end_s: end_s.clamp(0.0, duration),
        lo_hz: (((spec.k0 + k_lo) as f32 - 0.5) * bin_hz).max(band_lo),
        hi_hz: (((spec.k0 + k_hi) as f32 + 0.5) * bin_hz).min(band_hi),
    })
}

/// Each bin's levels over time, in dB.
struct Levels {
    /// `QUIET_PERCENTILE`.
    quiet: Vec<f32>,
    /// The median.
    middle: Vec<f32>,
    /// See `LOUD_SHARE`.
    loud: Vec<f32>,
}

impl Levels {
    /// How far noise alone rises from a bin's quiet level to its loud level,
    /// in dB. Measured on the bins and frames that are mostly noise: the
    /// smaller of `SPREAD_PERCENTILE` of the bins' rises, and the median
    /// rise from quiet to middle level scaled to what noise shows.
    fn noise_spread(&self) -> f32 {
        let mut rise: Vec<f32> = self
            .loud
            .iter()
            .zip(&self.quiet)
            .map(|(l, q)| l - q)
            .collect();
        let rank = (rise.len() as f32 * SPREAD_PERCENTILE) as usize;
        let by_bins = *rise.select_nth_unstable_by(rank, f32::total_cmp).1;
        let mut lower: Vec<f32> = self
            .middle
            .iter()
            .zip(&self.quiet)
            .map(|(m, q)| m - q)
            .collect();
        let half = lower.len() / 2;
        let by_frames = *lower.select_nth_unstable_by(half, f32::total_cmp).1 * NOISE_RISE_RATIO;
        by_bins.min(by_frames).max(0.0)
    }
}

/// Smoothed power of the in-band bins, frame by frame.
struct Spectrum {
    /// Rate after halving (see [`halve`]).
    rate: u32,
    fft_size: usize,
    hop: usize,
    bin_hz: f32,
    /// FFT bin of the first kept bin.
    k0: usize,
    n_bins: usize,
    n_frames: usize,
    /// `n_frames * n_bins`, frame-major, power (a full-scale sine is 1).
    power: Vec<f32>,
    /// Frames that are not digital silence.
    live: Vec<bool>,
}

impl Spectrum {
    fn new(samples: &[f32], sample_rate: u32, band: (f32, f32)) -> Option<Self> {
        if sample_rate == 0 {
            return None;
        }
        let lo = band.0.min(band.1).max(0.0);
        let hi = band.0.max(band.1).min(sample_rate as f32 / 2.0);
        // Halve the rate while the band still fits with room to spare: only
        // the band matters, and a smaller FFT is cheaper.
        let mut rate = sample_rate;
        let mut halved: Option<Vec<f32>> = None;
        while rate as f32 / 2.0 >= DECIMATE_HEADROOM * hi && rate.is_multiple_of(2) {
            let src = halved.as_deref().unwrap_or(samples);
            halved = Some(halve(src, rate as f32, hi));
            rate /= 2;
        }
        let samples = halved.as_deref().unwrap_or(samples);

        let sr = rate as f32;
        let fft_size = ((sr / TARGET_BIN_HZ).ceil() as usize)
            .next_power_of_two()
            .max(256);
        let hop = fft_size / 2;
        let bin_hz = sr / fft_size as f32;
        let k0 = ((lo / bin_hz).ceil() as usize).max(1);
        let k1 = ((hi / bin_hz).floor() as usize).min(fft_size / 2 - 1);
        if k1 < k0 + 4 || samples.len() < fft_size {
            return None;
        }
        let n_bins = k1 - k0 + 1;
        let n_frames = (samples.len() - fft_size) / hop + 1;
        if n_frames < 4 {
            return None;
        }

        let window = hann_window(fft_size);
        let wsum: f32 = window.iter().sum();
        // A full-scale sine reads 1 (0 dB).
        let norm = 4.0 / (wsum * wsum);
        let fft = plan_fft_forward(fft_size);
        let mut input = fft.make_input_vec();
        let mut output = fft.make_output_vec();
        let mut scratch = fft.make_scratch_vec();

        // Power, smoothed over 3 bins.
        let mut fsmooth = vec![0f32; n_frames * n_bins];
        let mut live = vec![true; n_frames];
        let mut row = vec![0f32; n_bins];
        for t in 0..n_frames {
            let frame = &samples[t * hop..t * hop + fft_size];
            for ((x, s), w) in input.iter_mut().zip(frame).zip(&window) {
                *x = s * w;
            }
            fft.process_with_scratch(&mut input, &mut output, &mut scratch)
                .ok()?;
            let mut peak = 0f32;
            for (p, c) in row.iter_mut().zip(&output[k0..=k1]) {
                *p = c.norm_sqr() * norm;
                peak = peak.max(*p);
            }
            live[t] = peak > SILENT_POWER;
            let out = &mut fsmooth[t * n_bins..(t + 1) * n_bins];
            out[0] = (row[0] + row[1]) / 2.0;
            out[n_bins - 1] = (row[n_bins - 2] + row[n_bins - 1]) / 2.0;
            for k in 1..n_bins - 1 {
                out[k] = (row[k - 1] + row[k] + row[k + 1]) / 3.0;
            }
        }
        if live.iter().filter(|&&l| l).count() < 4 {
            return None;
        }

        // Then over 3 frames, live frames only.
        let mut power = vec![0f32; fsmooth.len()];
        for t in 0..n_frames {
            if !live[t] {
                continue;
            }
            let prev = (t > 0 && live[t - 1]).then(|| t - 1);
            let next = (t + 1 < n_frames && live[t + 1]).then_some(t + 1);
            let count = 1.0 + prev.is_some() as u8 as f32 + next.is_some() as u8 as f32;
            let out = &mut power[t * n_bins..(t + 1) * n_bins];
            out.copy_from_slice(&fsmooth[t * n_bins..(t + 1) * n_bins]);
            for u in [prev, next].into_iter().flatten() {
                for (o, p) in out.iter_mut().zip(&fsmooth[u * n_bins..(u + 1) * n_bins]) {
                    *o += p;
                }
            }
            out.iter_mut().for_each(|o| *o /= count);
        }

        Some(Spectrum {
            rate,
            fft_size,
            hop,
            bin_hz,
            k0,
            n_bins,
            n_frames,
            power,
            live,
        })
    }

    /// Each bin's quiet, middle and loud level over the live frames, in dB.
    fn levels(&self) -> Levels {
        let live: Vec<usize> = (0..self.n_frames).filter(|&t| self.live[t]).collect();
        let n = live.len();
        let quiet_rank = ((n as f32 * QUIET_PERCENTILE) as usize).min(n - 1);
        let loud_rank = loud_rank(n);
        let middle_rank = n / 2;
        let mut col = vec![0f32; n];
        let mut quiet = vec![0f32; self.n_bins];
        let mut middle = vec![0f32; self.n_bins];
        let mut loud = vec![0f32; self.n_bins];
        for k in 0..self.n_bins {
            for (c, &t) in col.iter_mut().zip(&live) {
                *c = self.power[t * self.n_bins + k];
            }
            let [q, m, l] = select_ranks(&mut col, [quiet_rank, middle_rank, loud_rank]);
            quiet[k] = db(q);
            middle[k] = db(m);
            loud[k] = db(l);
        }
        Levels {
            quiet,
            middle,
            loud,
        }
    }

    /// For each bin `j`, the windows of the background's peak removal that
    /// end and start at it: `BACKGROUND_OCTAVES` wide, at least 4 bins.
    fn background_windows(&self) -> Vec<(usize, usize)> {
        let r = 2f32.powf(BACKGROUND_OCTAVES);
        (0..self.n_bins)
            .map(|j| {
                let f = (self.k0 + j) as f32;
                let below = ((f - f / r).round() as usize).max(4);
                let above = ((f * r - f).round() as usize).max(4);
                (below, above)
            })
            .collect()
    }

    /// For each bin, its width on the mel scale, which spaces frequencies
    /// roughly as hearing (and BirdNET's input) does.
    fn mel_widths(&self) -> Vec<f32> {
        (0..self.n_bins)
            .map(|k| {
                let f = (self.k0 + k) as f32 * self.bin_hz;
                2595.0 / std::f32::consts::LN_10 / (700.0 + f) * self.bin_hz
            })
            .collect()
    }

    /// Frame by frame, the power in the bins of `parts` against the
    /// background's, in dB. Silent frames are minus infinity.
    fn envelope(&self, parts: &[(usize, usize)], background: &[f32]) -> Vec<f32> {
        let base: f32 = parts
            .iter()
            .flat_map(|&(lo, hi)| &background[lo..=hi])
            .map(|b| 10f32.powf(b / 10.0))
            .sum();
        (0..self.n_frames)
            .map(|t| {
                if !self.live[t] {
                    return f32::NEG_INFINITY;
                }
                let row = &self.power[t * self.n_bins..(t + 1) * self.n_bins];
                let p: f32 = parts
                    .iter()
                    .map(|&(lo, hi)| row[lo..=hi].iter().sum::<f32>())
                    .sum();
                db(p / base)
            })
            .collect()
    }

    /// First and last frame of the sound in the bins of `parts`.
    fn time_extent(
        &self,
        parts: &[(usize, usize)],
        background: &[f32],
        spread: f32,
    ) -> Option<(usize, usize)> {
        let x = self.envelope(parts, background);
        let mut sorted: Vec<f32> = x.iter().copied().filter(|v| v.is_finite()).collect();
        let rank = loud_rank(sorted.len());
        let (_, x_loud, _) = sorted.select_nth_unstable_by(rank, f32::total_cmp);
        let threshold = (*x_loud - TIME_RANGE_DB).max(spread);

        let mut first = None;
        let mut last = None;
        let mut run = 0usize;
        for (t, &v) in x.iter().enumerate() {
            if v >= threshold {
                run += 1;
                if run >= MIN_RUN_FRAMES {
                    first.get_or_insert(t + 1 - run);
                    last = Some(t);
                }
            } else {
                run = 0;
            }
        }
        Some((first?, last?))
    }
}

/// Power to dB, floored at `FLOOR_DB`.
fn db(p: f32) -> f32 {
    (10.0 * p.max(1e-30).log10()).max(FLOOR_DB)
}

/// The values that would be at `ranks` if `v` were sorted. Reorders `v`.
fn select_ranks<const N: usize>(v: &mut [f32], ranks: [usize; N]) -> [f32; N] {
    let mut order: [usize; N] = std::array::from_fn(|i| i);
    order.sort_by_key(|&i| ranks[i]);
    let mut out = [0f32; N];
    // `rest` holds the values of rank `base` and up.
    let mut rest = v;
    let mut base = 0;
    let mut last: Option<(usize, f32)> = None;
    for i in order {
        let r = ranks[i];
        if let Some((_, value)) = last.filter(|&(lr, _)| lr == r) {
            out[i] = value;
            continue;
        }
        let (_, x, after) = rest.select_nth_unstable_by(r - base, f32::total_cmp);
        out[i] = *x;
        last = Some((r, *x));
        rest = after;
        base = r + 1;
    }
    out
}

/// Rank of the loud level among `n` sorted values.
fn loud_rank(n: usize) -> usize {
    let top = ((n as f32 * LOUD_SHARE).round() as usize).clamp(LOUD_MIN_FRAMES, LOUD_MAX_FRAMES);
    n.saturating_sub(1 + top)
}

/// Halve the sample rate, keeping 0 to `hi` Hz. A half-band FIR (Blackman
/// window, about 74 dB down) only has to stop what would fold back below
/// `hi`, which starts at `rate / 2 - hi`, so it needs few taps. Centred, so
/// no delay.
fn halve(x: &[f32], rate: f32, hi: f32) -> Vec<f32> {
    // Blackman: transition width about 5.5 / taps of the rate.
    let transition = (rate / 2.0 - 2.0 * hi).max(rate / 64.0);
    let mut half_len = (2.75 * rate / transition).ceil() as usize;
    if half_len.is_multiple_of(2) {
        half_len += 1;
    }
    // Taps at odd offsets 1, 3, .. half_len (the even ones are zero).
    let span = (half_len + 1) as f32;
    let mut taps: Vec<(usize, f32)> = (1..=half_len)
        .step_by(2)
        .map(|j| {
            let sinc = (std::f32::consts::FRAC_PI_2 * j as f32).sin()
                / (std::f32::consts::FRAC_PI_2 * j as f32);
            let w = 0.42
                + 0.5 * (std::f32::consts::PI * j as f32 / span).cos()
                + 0.08 * (std::f32::consts::TAU * j as f32 / span).cos();
            (j, 0.5 * sinc * w)
        })
        .collect();
    let gain = 0.5 + 2.0 * taps.iter().map(|t| t.1).sum::<f32>();
    taps.iter_mut().for_each(|t| t.1 /= gain);
    let centre = 0.5 / gain;

    // Polyphase: y[m] = centre * x[2m] + sum over taps of
    // h * (x[2m - j] + x[2m + j]), and the odd samples x[2m -/+ j] are
    // odd[m - (j + 1) / 2] and odd[m + (j - 1) / 2]. A block of outputs at a
    // time, the odd samples it needs are copied to a buffer padded with zeros,
    // so every tap is one pass over contiguous memory that stays in cache.
    const BLOCK: usize = 4096;
    let pad = half_len.div_ceil(2) + 1;
    let n_odd = x.len() / 2;
    let n_out = x.len().div_ceil(2);
    let mut y = Vec::with_capacity(n_out);
    let mut odd = vec![0f32; BLOCK + 2 * pad];
    for m0 in (0..n_out).step_by(BLOCK) {
        let m1 = (m0 + BLOCK).min(n_out);
        // odd[i] holds odd sample m0 - pad + i.
        for (i, o) in odd[..m1 - m0 + 2 * pad].iter_mut().enumerate() {
            let k = (m0 + i).wrapping_sub(pad);
            *o = if k < n_odd { x[2 * k + 1] } else { 0.0 };
        }
        let start = y.len();
        y.extend(
            x[2 * m0..]
                .iter()
                .step_by(2)
                .take(m1 - m0)
                .map(|v| centre * v),
        );
        let block = &mut y[start..];
        for &(j, h) in &taps {
            let before = &odd[pad - j.div_ceil(2)..];
            let after = &odd[pad + (j - 1) / 2..];
            for ((y, a), b) in block.iter_mut().zip(before).zip(after) {
                *y += h * (a + b);
            }
        }
    }
    y
}

/// Grey-scale opening across frequency: each bin takes the highest of the
/// minimums of all windows that contain it. Bin `j` has two windows,
/// `win[j].0` bins ending at `j` and `win[j].1` bins starting at it, so every
/// bin is the edge of some window. Removes peaks narrower than the windows and
/// keeps wider plateaus whole, sloping edges included.
fn opening(v: &[f32], win: &[(usize, usize)]) -> Vec<f32> {
    let n = v.len();
    let mut out = vec![f32::NEG_INFINITY; n];
    let mut apply = |a: usize, b: usize| {
        let m = v[a..=b].iter().copied().fold(f32::INFINITY, f32::min);
        for o in &mut out[a..=b] {
            *o = o.max(m);
        }
    };
    for (j, &(below, above)) in win.iter().enumerate() {
        apply(j.saturating_sub(below), j);
        apply(j, (j + above).min(n - 1));
    }
    out
}

/// The frequency regions (bin ranges) of the most prominent sound.
///
/// Regions that rise and fall together are one sound: harmonics, or a
/// broadband sound in pieces. Only regions that come and go are grouped this
/// way, as steady ones rise and fall together whenever the whole recording's
/// level does. A region next to a louder one that belongs with it is that
/// sound's flank. Members whose peak is more than `EXTEND_DB` below the
/// group's first (loudest) peak, and flanks, belong to the sound but are left
/// out of its box, as for the bins of one region. The group with the highest
/// score wins: excess above the noise, summed over the mel scale.
fn pick_sound(
    spec: &Spectrum,
    excess: &[f32],
    background: &[f32],
    spread: f32,
) -> Option<Vec<(usize, usize)>> {
    let regions = find_regions(excess, spread);
    let n = regions.len();
    let weights = spec.mel_widths();
    let score = |i: usize| -> f32 {
        let (lo, hi) = regions[i];
        (lo..=hi)
            .map(|k| (excess[k] - spread).max(0.0) * weights[k])
            .sum()
    };
    let peak = |i: usize| {
        let (lo, hi) = regions[i];
        excess[lo..=hi].iter().copied().fold(f32::MIN, f32::max)
    };
    let envelopes: Vec<Vec<f32>> = regions
        .iter()
        .map(|r| spec.envelope(&[*r], background))
        .collect();
    let pulsed: Vec<bool> = envelopes.iter().map(|e| swing(e) >= PULSED_DB).collect();
    let together = |a: usize, b: usize| {
        if pulsed[a] && pulsed[b] {
            correlation(&envelopes[a], &envelopes[b]) >= SAME_SOUND_CORRELATION
        } else {
            !pulsed[a] && !pulsed[b]
        }
    };
    let touching = |a: usize, b: usize| {
        let ((a_lo, a_hi), (b_lo, b_hi)) = (regions[a], regions[b]);
        a_hi + 1 + MAX_GAP_BINS >= b_lo && b_hi + 1 + MAX_GAP_BINS >= a_lo
    };

    // Regions come loudest peak first.
    let mut group: Vec<Option<usize>> = vec![None; n];
    let mut hidden = vec![false; n];
    for seed in 0..n {
        if group[seed].is_some() {
            continue;
        }
        if let Some(j) = (0..seed).find(|&j| touching(j, seed) && together(j, seed)) {
            group[seed] = group[j];
            hidden[seed] = true;
            continue;
        }
        group[seed] = Some(seed);
        if !pulsed[seed] {
            continue;
        }
        for other in seed + 1..n {
            if group[other].is_none() && pulsed[other] && together(seed, other) {
                group[other] = Some(seed);
                hidden[other] = peak(other) < peak(seed) - EXTEND_DB;
            }
        }
    }
    let members = |g: usize| -> Vec<usize> {
        (0..n)
            .filter(|&i| group[i] == Some(g) && !hidden[i])
            .collect()
    };
    // Hidden regions are left out of the box but count towards the score.
    let group_score = |g: usize| {
        (0..n)
            .filter(|&i| group[i] == Some(g))
            .map(score)
            .sum::<f32>()
    };
    let best = (0..n)
        .filter(|&i| group[i] == Some(i))
        .max_by(|&a, &b| group_score(a).total_cmp(&group_score(b)))?;
    Some(members(best).into_iter().map(|i| regions[i]).collect())
}

/// How far an envelope's loud frames are above its quiet ones, in dB.
fn swing(env: &[f32]) -> f32 {
    let mut v: Vec<f32> = env.iter().copied().filter(|x| x.is_finite()).collect();
    if v.len() < 4 {
        return 0.0;
    }
    let quiet_rank = (v.len() as f32 * QUIET_PERCENTILE) as usize;
    let rank = loud_rank(v.len());
    let loud = *v.select_nth_unstable_by(rank, f32::total_cmp).1;
    let quiet = *v.select_nth_unstable_by(quiet_rank, f32::total_cmp).1;
    loud - quiet
}

/// Pearson correlation of two envelopes over the frames where both are
/// finite.
fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let pairs: Vec<(f32, f32)> = a
        .iter()
        .zip(b)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|(x, y)| (*x, *y))
        .collect();
    let n = pairs.len() as f32;
    if n < 3.0 {
        return 0.0;
    }
    let (ma, mb) = pairs
        .iter()
        .fold((0.0, 0.0), |(sa, sb), (x, y)| (sa + x / n, sb + y / n));
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in &pairs {
        sab += (x - ma) * (y - mb);
        saa += (x - ma) * (x - ma);
        sbb += (y - mb) * (y - mb);
    }
    if saa <= 0.0 || sbb <= 0.0 {
        return 0.0;
    }
    sab / (saa * sbb).sqrt()
}

/// Frequency regions that stand out, as bin ranges, loudest peak first. Each
/// grows from the highest excess not yet in a region, down and up while the
/// excess stays within `EXTEND_DB` of its peak and `KEEP_DB` above the noise.
fn find_regions(excess: &[f32], spread: f32) -> Vec<(usize, usize)> {
    let n = excess.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| excess[b].total_cmp(&excess[a]));
    let mut taken = vec![false; n];
    let mut regions = Vec::new();
    for peak in order {
        if excess[peak] < spread + DETECT_DB || regions.len() == MAX_REGIONS {
            break;
        }
        if taken[peak] {
            continue;
        }
        let keep = (excess[peak] - EXTEND_DB).max(spread + KEEP_DB);
        let extend = |step: &dyn Fn(usize) -> Option<usize>| {
            let mut edge = peak;
            let mut k = peak;
            let mut gap = 0;
            while let Some(next) = step(k) {
                if taken[next] {
                    break;
                }
                k = next;
                if excess[k] >= keep {
                    edge = k;
                    gap = 0;
                } else {
                    gap += 1;
                    if gap > MAX_GAP_BINS {
                        break;
                    }
                }
            }
            edge
        };
        let lo = extend(&|k| k.checked_sub(1));
        let hi = extend(&|k| (k + 1 < n).then_some(k + 1));
        taken[lo..=hi].iter_mut().for_each(|t| *t = true);
        regions.push((lo, hi));
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const BAND: (f32, f32) = (250.0, 15_000.0);

    struct Rng(u32);
    impl Rng {
        /// Roughly Gaussian, unit variance (sum of 4 uniforms).
        fn gauss(&mut self) -> f32 {
            let mut s = 0.0;
            for _ in 0..4 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 17;
                self.0 ^= self.0 << 5;
                s += self.0 as f32 / u32::MAX as f32 - 0.5;
            }
            s * 3f32.sqrt()
        }
    }

    fn hiss(n: usize, rms: f32, seed: u32) -> Vec<f32> {
        let mut r = Rng(seed);
        (0..n).map(|_| r.gauss() * rms).collect()
    }

    fn tone(n: usize, sr: u32, hz: f32, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (std::f64::consts::TAU * hz as f64 * i as f64 / sr as f64).sin() as f32)
            .collect()
    }

    /// Noise band-passed to `lo..hi` Hz, with unit-ish level, by summing
    /// sines at random phases every 10 Hz.
    fn band_noise(n: usize, sr: u32, lo: f32, hi: f32, amp: f32, seed: u32) -> Vec<f32> {
        let mut r = Rng(seed);
        let mut out = vec![0f32; n];
        let mut f = lo;
        let mut count = 0;
        while f <= hi {
            let ph = r.gauss() * 3.0;
            let w = TAU * f / sr as f32;
            for (i, o) in out.iter_mut().enumerate() {
                *o += (w * i as f32 + ph).sin();
            }
            f += 10.0;
            count += 1;
        }
        let g = amp / (count as f32 / 2.0).sqrt();
        out.iter_mut().for_each(|o| *o *= g);
        out
    }

    fn add(a: &mut [f32], b: &[f32], at: usize) {
        for (x, y) in a[at..].iter_mut().zip(b) {
            *x += y;
        }
    }

    /// Raised-cosine gate: 1 inside `start..end` samples, 5 ms ramps.
    fn gate(x: &mut [f32], sr: u32, start: usize, end: usize) {
        let ramp = (sr as usize / 200).max(1);
        for (i, v) in x.iter_mut().enumerate() {
            let g = if i < start || i >= end {
                0.0
            } else {
                let d = (i - start).min(end - 1 - i);
                if d >= ramp {
                    1.0
                } else {
                    0.5 - 0.5 * (std::f32::consts::PI * d as f32 / ramp as f32).cos()
                }
            };
            *v *= g;
        }
    }

    #[test]
    fn steady_tone_gives_a_narrow_box_over_the_whole_stretch() {
        let sr = 48_000;
        let n = 3 * sr as usize;
        let mut x = hiss(n, 0.01, 1);
        add(&mut x, &tone(n, sr, 4_500.0, 0.1), 0);
        let b = estimate(&x, sr, BAND).expect("a box");
        assert!(b.lo_hz >= 4_300.0 && b.hi_hz <= 4_700.0, "{b:?}");
        assert!(b.lo_hz <= 4_500.0 && b.hi_hz >= 4_500.0, "{b:?}");
        assert!(b.start_s < 0.05 && b.end_s > 2.95, "{b:?}");
    }

    #[test]
    fn noise_burst_gives_its_band_and_times() {
        let sr = 48_000;
        let n = 3 * sr as usize;
        let mut x = hiss(n, 0.005, 2);
        let mut burst = band_noise(sr as usize, sr, 2_000.0, 3_000.0, 0.2, 3);
        gate(&mut burst, sr, 0, sr as usize);
        add(&mut x, &burst, sr as usize);
        let b = estimate(&x, sr, BAND).expect("a box");
        assert!(
            (b.start_s - 1.0).abs() < 0.1 && (b.end_s - 2.0).abs() < 0.1,
            "{b:?}"
        );
        assert!(
            (b.lo_hz - 2_000.0).abs() < 200.0 && (b.hi_hz - 3_000.0).abs() < 200.0,
            "{b:?}"
        );
    }

    #[test]
    fn pulsed_tone_box_spans_the_pulses() {
        let sr = 44_100;
        let n = 3 * sr as usize;
        let mut x = hiss(n, 0.005, 4);
        // 40 ms pulses of 3.2 kHz every 200 ms from 0.6 s to 2.2 s.
        let len = (0.04 * sr as f32) as usize;
        for p in 0..9 {
            let at = ((0.6 + 0.2 * p as f32) * sr as f32) as usize;
            let mut pulse = tone(len, sr, 3_200.0, 0.1);
            gate(&mut pulse, sr, 0, len);
            add(&mut x, &pulse, at);
        }
        let b = estimate(&x, sr, BAND).expect("a box");
        assert!(
            (b.start_s - 0.6).abs() < 0.1 && (b.end_s - 2.24).abs() < 0.1,
            "{b:?}"
        );
        assert!(b.lo_hz >= 2_900.0 && b.hi_hz <= 3_500.0, "{b:?}");
    }

    #[test]
    fn white_noise_gives_none() {
        for (sr, secs) in [(48_000u32, 3usize), (48_000, 30), (384_000, 3), (22_050, 3)] {
            for seed in [5, 6, 7] {
                let x = hiss(sr as usize * secs, 0.05, seed);
                assert_eq!(
                    estimate(&x, sr, BAND),
                    None,
                    "{sr} Hz, {secs} s, seed {seed}"
                );
            }
        }
    }

    #[test]
    fn sloped_noise_gives_none() {
        // Brown-ish noise: leaky integral of white noise, steep slope.
        let sr = 48_000;
        let w = hiss(3 * sr as usize, 0.05, 8);
        let mut acc = 0.0;
        let x: Vec<f32> = w
            .iter()
            .map(|v| {
                acc = 0.98 * acc + v;
                acc
            })
            .collect();
        assert_eq!(estimate(&x, sr, BAND), None);
    }

    #[test]
    fn digital_silence_gives_none() {
        assert_eq!(estimate(&vec![0.0; 3 * 48_000], 48_000, BAND), None);
        assert_eq!(estimate(&[], 48_000, BAND), None);
        assert_eq!(estimate(&[0.1; 100], 48_000, BAND), None);
    }

    #[test]
    fn noise_after_digital_silence_gives_none() {
        let sr = 48_000;
        let mut x = vec![0.0; 3 * sr as usize];
        add(&mut x, &hiss(2 * sr as usize, 0.05, 9), sr as usize);
        assert_eq!(estimate(&x, sr, BAND), None);
    }

    #[test]
    fn high_sample_rate() {
        let sr = 384_000;
        let n = 3 * sr as usize;
        let mut x = hiss(n, 0.005, 10);
        // A loud bat-like tone far above the band must not matter.
        add(&mut x, &tone(n, sr, 45_000.0, 0.5), 0);
        let mut burst = tone(sr as usize / 2, sr, 6_000.0, 0.1);
        gate(&mut burst, sr, 0, sr as usize / 2);
        add(&mut x, &burst, sr as usize);
        let b = estimate(&x, sr, BAND).expect("a box");
        assert!(
            (b.start_s - 1.0).abs() < 0.1 && (b.end_s - 1.5).abs() < 0.1,
            "{b:?}"
        );
        assert!(b.lo_hz >= 5_700.0 && b.hi_hz <= 6_300.0, "{b:?}");
    }

    #[test]
    fn broadband_burst_gives_a_wide_box() {
        let sr = 48_000;
        let n = 3 * sr as usize;
        let mut x = hiss(n, 0.002, 11);
        let mut burst = band_noise(sr as usize, sr, 1_000.0, 10_000.0, 0.2, 12);
        gate(&mut burst, sr, 0, sr as usize);
        add(&mut x, &burst, sr as usize / 2);
        let b = estimate(&x, sr, BAND).expect("a box");
        assert!(b.lo_hz < 1_500.0 && b.hi_hz > 9_000.0, "{b:?}");
        assert!(
            (b.start_s - 0.5).abs() < 0.1 && (b.end_s - 1.5).abs() < 0.1,
            "{b:?}"
        );
    }

    #[test]
    fn steady_broadband_sound_gives_none_or_the_whole_sound() {
        let sr = 48_000;
        let n = 3 * sr as usize;
        let mut x = hiss(n, 0.002, 13);
        add(&mut x, &band_noise(n, sr, 2_000.0, 8_000.0, 0.2, 14), 0);
        if let Some(b) = estimate(&x, sr, BAND) {
            assert!(b.lo_hz < 2_500.0 && b.hi_hz > 7_500.0, "a sliver: {b:?}");
        }
    }

    #[test]
    fn halving_keeps_the_band_and_stops_what_would_fold_into_it() {
        let sr = 96_000;
        let n = sr as usize;
        let rms = |v: &[f32]| {
            (v[1000..v.len() - 1000].iter().map(|s| s * s).sum::<f32>() / (v.len() - 2000) as f32)
                .sqrt()
        };
        let kept = halve(&tone(n, sr, 10_000.0, 0.5), sr as f32, 15_000.0);
        assert_eq!(kept.len(), n / 2);
        let db_kept = 20.0 * (rms(&kept) / (0.5 / 2f32.sqrt())).log10();
        assert!(db_kept.abs() < 0.1, "{db_kept} dB");
        // 40 kHz would fold to 8 kHz at 48 kHz.
        let folded = halve(&tone(n, sr, 40_000.0, 0.5), sr as f32, 15_000.0);
        let db_folded = 20.0 * (rms(&folded) / (0.5 / 2f32.sqrt())).log10();
        assert!(db_folded < -60.0, "{db_folded} dB");
    }

    #[test]
    fn band_above_nyquist_is_clamped() {
        let sr = 16_000;
        let n = 3 * sr as usize;
        let mut x = hiss(n, 0.01, 15);
        add(&mut x, &tone(n, sr, 4_500.0, 0.1), 0);
        let b = estimate(&x, sr, BAND).expect("a box");
        assert!(b.lo_hz >= 4_300.0 && b.hi_hz <= 4_700.0, "{b:?}");
        assert_eq!(estimate(&x, sr, (9_000.0, 15_000.0)), None);
    }

    #[test]
    fn harmonics_that_pulse_together_share_one_box() {
        let sr = 48_000;
        let n = 3 * sr as usize;
        let mut x = hiss(n, 0.002, 17);
        // 100 ms calls every 300 ms, fundamental 1.5 kHz and two harmonics.
        let len = sr as usize / 10;
        for p in 0..8 {
            let mut call = tone(len, sr, 1_500.0, 0.1);
            add(&mut call, &tone(len, sr, 3_000.0, 0.05), 0);
            add(&mut call, &tone(len, sr, 4_500.0, 0.03), 0);
            gate(&mut call, sr, 0, len);
            add(&mut x, &call, sr as usize * 3 / 10 * (p + 1));
        }
        let b = estimate(&x, sr, BAND).expect("a box");
        assert!(b.lo_hz > 1_300.0 && b.lo_hz < 1_500.0, "{b:?}");
        assert!(b.hi_hz > 4_500.0 && b.hi_hz < 4_700.0, "{b:?}");
        assert!(
            (b.start_s - 0.3).abs() < 0.1 && (b.end_s - 2.5).abs() < 0.1,
            "{b:?}"
        );
    }

    #[test]
    fn a_steady_tone_does_not_join_a_pulsed_sound() {
        let sr = 48_000;
        let n = 3 * sr as usize;
        let mut x = hiss(n, 0.002, 18);
        add(&mut x, &tone(n, sr, 9_000.0, 0.02), 0);
        let mut burst = band_noise(sr as usize, sr, 2_000.0, 5_000.0, 0.2, 19);
        gate(&mut burst, sr, 0, sr as usize);
        add(&mut x, &burst, sr as usize);
        let b = estimate(&x, sr, BAND).expect("a box");
        assert!(b.hi_hz < 5_500.0 && b.lo_hz > 1_500.0, "{b:?}");
    }

    /// Timing on a 30 s stretch at 384 kHz and at 48 kHz. Run in release:
    /// `cargo test -p oversample-core --release --lib sound_box_timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sound_box_timing() {
        for sr in [384_000u32, 48_000] {
            let n = 30 * sr as usize;
            let mut x = hiss(n, 0.01, 16);
            add(&mut x, &tone(n, sr, 4_500.0, 0.1), 0);
            // Wall time, and this thread's CPU time on Linux (busy machines).
            let cpu_ms = || {
                std::fs::read_to_string("/proc/thread-self/stat")
                    .ok()
                    .and_then(|s| {
                        let f: Vec<u64> = s
                            .rsplit(')')
                            .next()?
                            .split_whitespace()
                            .skip(11)
                            .take(2)
                            .filter_map(|v| v.parse().ok())
                            .collect();
                        Some(f.iter().sum::<u64>() * 10)
                    })
                    .unwrap_or(0)
            };
            for _ in 0..5 {
                let (t, c) = (std::time::Instant::now(), cpu_ms());
                let b = estimate(&x, sr, BAND);
                println!(
                    "{sr} Hz, 30 s: {:?}, CPU {} ms, {b:?}",
                    t.elapsed(),
                    cpu_ms() - c
                );
            }
        }
    }
}

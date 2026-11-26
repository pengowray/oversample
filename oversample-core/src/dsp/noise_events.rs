//! Wind and handling noise: where in a recording the microphone was buffeted
//! by wind, or the recorder was knocked or rubbed.
//!
//! - **Wind** is strong, fluctuating noise with most of its energy below
//!   about 1 kHz, often reaching a few kHz, lasting from a few hundred ms to
//!   many seconds.
//! - **Handling** is a thump, knock or scrape: short (about 5 to 300 ms),
//!   sudden, and reaching down to the lowest frequencies while also reaching
//!   above 1 kHz. Bird and bat calls are tonal or sweep and start above a few
//!   hundred Hz; rain drops are clicks with little energy below 1 kHz.
//!
//! [`NoiseScanner`] takes a recording's mono samples in pieces of any size,
//! like [`CallScanner`](super::call_detect::CallScanner), so a long file can
//! be read a piece at a time. [`scan_noise`] does the same for samples
//! already in memory.
//!
//! ## Method
//!
//! - Only 50 Hz to 4 kHz is looked at. At 32 kHz and above the samples are
//!   first brought down to between 16 and 32 kHz by a 6th-order CIC
//!   decimator (six moving sums as long as the decimation factor, in exact
//!   integer arithmetic). Its stop bands sit on the multiples of the new
//!   rate: sound at 12 kHz folds into 0 to 4 kHz at least 63 dB down, and a
//!   bat call (20 kHz and above) at least 89 dB down.
//!   A one-pole filter at 20 Hz then removes any DC offset.
//! - Every [`HOP_S`], a [`FRAME_S`] Hann frame gives each of 8 bands'
//!   ([`BAND_EDGES_HZ`]) mean power per FFT bin, in dB, and a
//!   [`TONAL_FRAME_S`] frame around it gives the **tonal share**: the part
//!   of 60 Hz to 1 kHz's power in narrow peaks at least 10 times the power
//!   50 Hz either side (a hoot, a hum, a voice's harmonics). Only these are
//!   kept while samples arrive (17 bytes per frame, about 6 MB per hour of
//!   recording), since the floors below need the whole file.
//! - Each band's **floor** is the [`FLOOR_PERCENTILE`]th percentile of its
//!   levels over the file and its **typical level** the median, so steady
//!   hiss and hum are not events. Frames of digital silence are left out,
//!   and neither is put under [`MIN_FLOOR_DB`]. The same is done for the
//!   "low part" (50 to 500 Hz, the three lowest bands' power together) and
//!   the "high part" (1.2 to 4 kHz).
//! - **Wind**: a frame is windy when, over 50 ms, the low part is at least
//!   [`WIND_DB`] over its floor, at least two of the three lowest bands are
//!   [`BAND_ON_DB`] over theirs, the low part's power per Hz is at least
//!   [`WIND_TILT_DB`] above the high part's (wind is loudest at the bottom),
//!   and the tonal share is at most [`WIND_MAX_TONAL`]. Windy frames less
//!   than [`WIND_MERGE_S`] apart are joined. A stretch is reported when it
//!   lasts at least [`MIN_WIND_S`] and its median 10 ms frame is
//!   [`WIND_SUSTAIN_DB`] over the floor, so a run of clicks is not wind.
//! - **Handling**: the low part rises at least [`ONSET_DB`] within 20 ms to
//!   at least [`HANDLING_DB`] over its typical level, and falls back under
//!   [`RELEASE_DB`] within [`MAX_HANDLING_S`]. At its loudest, the low part's
//!   power per Hz is at least [`HANDLING_TILT_DB`] above the high part's (a
//!   thump, not a click), the tonal share is at most [`HANDLING_MAX_TONAL`],
//!   and the bands over their typical levels reach from 250 Hz or lower up
//!   to 1.2 kHz or higher. A thump with more than [`TEXTURE_MAX_ONSETS`]
//!   sudden rises around it within [`TEXTURE_WINDOW_S`] that are up to
//!   [`TEXTURE_RELAX_DB`] fainter than handling is the loudest of many
//!   drops (rain, drips), not handling. A run of knocks that are all loud
//!   stays handling. Handling within or next to wind is left to the
//!   wind, as buffeting makes thumps of its own.
//! - An event's band runs from the lowest of the three lowest bands that is
//!   [`EXTENT_DB`] over its floor (0 Hz when that is the lowest band) up
//!   through the bands above it that are too.
//! - A band's level counts towards an event's band only within [`LEAK_DB`]
//!   of the frame's loudest band, so window leakage from a loud sound into
//!   an almost empty band is not read as sound there.
//!
//! ## Tuning
//!
//! Tuned on synthetic signals (the tests below) and by looking at
//! spectrograms of the corpus recordings (bat-demo-sounds, no-bat-sounds and
//! sound-event-sounds; `tests/noise_events_corpus.rs` lists the results).
//! Set from the corpus: handling measured against the typical level with a
//! power-mean "before" (a p10 floor and a minimum gave false handling in
//! loud, uneven noise), [`WIND_SUSTAIN_DB`], [`MIN_FLOOR_DB`] and the
//! texture rule. First choices, not swept: [`WIND_DB`], [`WIND_TILT_DB`],
//! the two tonal limits and [`HANDLING_TILT_DB`].
//!
//! Results, 2026-11: no events in steady rain, hail, streams, pebbles,
//! rustling reeds or a desktop computer's interference, and none from
//! insect songs themselves; a few insect recordings have wind marked over
//! low gusty rumble under the song. Handling is found in 24% of
//! hand-release bat recordings and 14% of the others. Known misses and
//! confusions: wind that lasts the whole recording is the floor and is not
//! found; thunder, passing traffic or engines, low animal calls (bullfrog,
//! flying foxes, a jaguar recording), music and creaking or bubbling sound
//! effects can read as wind; the whistle of "howling" wind is tonal and is
//! not wind on the microphone.
//!

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::RealToComplex;

use crate::dsp::fft::{hann_window, plan_fft_forward};
use crate::erased_sort::SortErased;

/// Frame length, in seconds.
pub const FRAME_S: f64 = 0.020;
/// Time from one frame to the next, in seconds.
pub const HOP_S: f64 = 0.010;
/// Band edges, in Hz: 8 bands from 50 Hz to 4 kHz.
pub const BAND_EDGES_HZ: [f32; BANDS + 1] = [
    50.0, 125.0, 250.0, 500.0, 800.0, 1200.0, 1800.0, 2700.0, 4000.0,
];
const BANDS: usize = 8;
/// The low part: 50 to 500 Hz.
const LOW: std::ops::Range<usize> = 0..3;
/// The high part: 1.2 to 4 kHz.
const HIGH: std::ops::Range<usize> = 5..8;

/// The floor's percentile among a band's frame levels.
pub const FLOOR_PERCENTILE: f64 = 10.0;
/// Floors and typical levels are never put lower than this, in dB (bin
/// power, a full-scale sine's peak bin at 0 dB). It is about the noise of
/// 16-bit samples, so in a recording high-passed to near digital silence
/// below 4 kHz, sounds too faint to hear there are not events.
pub const MIN_FLOOR_DB: f32 = -120.0;
/// A band's level counts only within this many dB of the frame's loudest
/// band.
pub const LEAK_DB: f32 = 60.0;
/// A band at least this far over its floor has sound in it.
pub const BAND_ON_DB: f32 = 6.0;
/// The low part at least this far over its floor (over 50 ms): wind.
pub const WIND_DB: f32 = 12.0;
/// The low part's power per Hz at least this far above the high part's:
/// wind.
pub const WIND_TILT_DB: f32 = 10.0;
/// The median 10 ms frame of a stretch of wind has the low part at least
/// this far over its floor, so a run of clicks is not wind.
pub const WIND_SUSTAIN_DB: f32 = 8.0;
/// Wind frames closer than this are one stretch of wind.
pub const WIND_MERGE_S: f64 = 0.5;
/// A stretch of wind frames shorter than this is not reported.
pub const MIN_WIND_S: f64 = 0.3;
/// Handling starts with the low part rising this far within 20 ms.
pub const ONSET_DB: f32 = 12.0;
/// At its loudest, handling has the low part at least this far over its
/// floor.
pub const HANDLING_DB: f32 = 15.0;
/// Handling ends when the low part falls below this over its floor.
pub const RELEASE_DB: f32 = 6.0;
/// A sound that does not fall back within this long is not handling.
pub const MAX_HANDLING_S: f64 = 0.3;
/// Sudden rises this much fainter than handling's ([`HANDLING_DB`] and
/// [`ONSET_DB`] less this) are counted for the texture test.
pub const TEXTURE_RELAX_DB: f32 = 6.0;
/// More than [`TEXTURE_MAX_ONSETS`] fainter sudden rises within this window
/// around a thump make it part of a texture (rain, drips), not handling.
pub const TEXTURE_WINDOW_S: f64 = 10.0;
pub const TEXTURE_MAX_ONSETS: usize = 8;
/// An event's band takes in the bands at least this far over their floors.
pub const EXTENT_DB: f32 = 6.0;

/// Long frame for the tonality measure, in seconds.
pub const TONAL_FRAME_S: f64 = 0.080;
/// The tonality measure looks from here to [`TONAL_HI_HZ`].
const TONAL_LO_HZ: f32 = 60.0;
const TONAL_HI_HZ: f32 = 1000.0;
/// A tonal peak is at least [`TONAL_CONTRAST`] times the power this far
/// either side, in Hz.
const TONAL_AWAY_HZ: f32 = 50.0;
const TONAL_CONTRAST: f32 = 10.0;
/// Wind frames have at most this share of the low part's power in tonal
/// peaks (over 50 ms).
pub const WIND_MAX_TONAL: f32 = 0.3;
/// Handling has at most this share of the low part's power in tonal peaks.
pub const HANDLING_MAX_TONAL: f32 = 0.5;
/// Handling's low part, power per Hz, at least this far above the high
/// part's at its loudest: a thump, not a click.
pub const HANDLING_TILT_DB: f32 = 6.0;
/// The DC blocker's cutoff, in Hz.
const DC_CUTOFF_HZ: f64 = 20.0;
/// Analysis rate the samples are brought down to, at least.
const MIN_ANALYSIS_RATE: u32 = 16_000;
const CIC_ORDER: usize = 6;
/// Frames of the 50 ms smoothing for wind: this many each side.
const SMOOTH_EACH_SIDE: usize = 2;
/// Levels are stored as `(dB - LEVEL_BASE_DB) * 100`; 0 is digital silence.
const LEVEL_BASE_DB: f32 = -300.0;
/// No band level is put lower than this, in dB.
const MIN_LEVEL_DB: f32 = -200.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoiseKind {
    Wind,
    Handling,
}

/// A stretch of wind or handling noise.
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseEvent {
    pub kind: NoiseKind,
    pub start_s: f64,
    pub end_s: f64,
    /// Bottom of the band the noise covers, in Hz (0 when it reaches the
    /// lowest band looked at).
    pub lo_hz: f32,
    /// Top of the band the noise covers, in Hz (at most 4 kHz, the top of the
    /// bands looked at).
    pub hi_hz: f32,
    /// How far the low part (50 to 500 Hz) rose over its floor at the
    /// event's loudest, in dB.
    pub level_db: f32,
}

/// Brings samples down by `r` with six moving sums of length `r`, in
/// wrapping integer arithmetic so long files lose no precision.
#[derive(Clone)]
struct Cic {
    r: u32,
    phase: u32,
    integ: [i64; CIC_ORDER],
    comb: [i64; CIC_ORDER],
    scale: f64,
}

/// Input samples are scaled by 2^24 to integers. With |x| <= 8 and r <= 31
/// the output stays under 2^58.
const CIC_INPUT_SCALE: f64 = 16_777_216.0;

impl Cic {
    fn new(r: u32) -> Self {
        let gain = (r as f64).powi(CIC_ORDER as i32);
        Self {
            r,
            phase: 0,
            integ: [0; CIC_ORDER],
            comb: [0; CIC_ORDER],
            scale: 1.0 / (gain * CIC_INPUT_SCALE),
        }
    }

    fn reset(&mut self) {
        *self = Self::new(self.r);
    }

    #[inline]
    fn push(&mut self, x: f32) -> Option<f32> {
        let x = if x.is_finite() {
            x.clamp(-8.0, 8.0)
        } else {
            0.0
        };
        let mut acc = (x as f64 * CIC_INPUT_SCALE) as i64;
        for s in &mut self.integ {
            *s = s.wrapping_add(acc);
            acc = *s;
        }
        self.phase += 1;
        if self.phase < self.r {
            return None;
        }
        self.phase = 0;
        let mut y = acc;
        for c in &mut self.comb {
            let t = y;
            y = y.wrapping_sub(*c);
            *c = t;
        }
        Some((y as f64 * self.scale) as f32)
    }
}

/// A run of frames with no missing samples between them.
#[derive(Clone, Copy, Debug)]
struct Segment {
    first_frame: usize,
    /// Input sample the segment's first frame starts at.
    start_sample: u64,
}

/// Finds wind and handling noise in a recording read in pieces.
pub struct NoiseScanner {
    sample_rate: u32,
    decim: u32,
    cic: Option<Cic>,
    /// One-pole DC blocker on the analysis-rate samples: input and output
    /// before, and the pole.
    dc: (f64, f64, f64),
    /// Short (band levels) and long (tonality) frame lengths, and the hop,
    /// in analysis-rate samples. The short frame is the middle of the long
    /// one.
    win: usize,
    long_win: usize,
    hop: usize,
    short: Spectrum,
    long: Spectrum,
    /// Bin range of each band in the short frame; empty when the band is
    /// above Nyquist.
    band_bins: [(usize, usize); BANDS],
    tonal_bins: TonalBins,
    /// Analysis-rate samples not yet used up by frames.
    buf: Vec<f32>,
    /// Input samples taken in so far, including missing ones.
    pos: u64,
    /// Input sample the current segment started at.
    seg_start: u64,
    seg_open: bool,
    segments: Vec<Segment>,
    /// `BANDS` stored levels per frame.
    levels: Vec<u16>,
    /// Per frame: the low part's tonal share, 0 to 255.
    tonal: Vec<u8>,
}

/// A Hann-windowed FFT of a fixed length, zero-padded to a power of two.
struct Spectrum {
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Scales bin power so a full-scale sine's peak bin is 0 dB.
    power_scale: f32,
    bin_hz: f64,
}

impl Spectrum {
    fn new(len: usize, rate: f64) -> Self {
        let size = len.next_power_of_two();
        let fft = plan_fft_forward(size);
        let window = hann_window(len);
        let wsum: f32 = window.iter().sum();
        Self {
            window,
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            power_scale: 4.0 / (wsum * wsum),
            bin_hz: rate / size as f64,
        }
    }

    /// Transform `frame`; false if the FFT failed.
    fn run(&mut self, frame: &[f32]) -> bool {
        let n = self.window.len();
        for (i, slot) in self.input.iter_mut().enumerate() {
            *slot = if i < n {
                frame[i] * self.window[i]
            } else {
                0.0
            };
        }
        self.fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .is_ok()
    }

    fn power(&self, k: usize) -> f32 {
        self.output[k].norm_sqr() * self.power_scale
    }
}

/// Where the tonality measure looks, in long-frame bins.
#[derive(Clone, Copy)]
struct TonalBins {
    lo: usize,
    hi: usize,
    /// A peak is compared with the bins this far each side.
    away: usize,
    /// Bins each side of a peak counted as the peak's power.
    half: usize,
}

impl NoiseScanner {
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate.max(1);
        let decim = if sample_rate >= 2 * MIN_ANALYSIS_RATE {
            sample_rate / MIN_ANALYSIS_RATE
        } else {
            1
        };
        let fa = sample_rate as f64 / decim as f64;
        let win = ((FRAME_S * fa).round() as usize).max(8);
        let long_win = ((TONAL_FRAME_S * fa).round() as usize).max(win);
        let hop = ((HOP_S * fa).round() as usize).max(1);
        let short = Spectrum::new(win, fa);
        let long = Spectrum::new(long_win, fa);
        let nyquist = fa / 2.0;
        let top_bin = |s: &Spectrum| (nyquist * 0.95 / s.bin_hz) as usize;
        let mut band_bins = [(0, 0); BANDS];
        for (b, bins) in band_bins.iter_mut().enumerate() {
            let k0 = ((BAND_EDGES_HZ[b] as f64 / short.bin_hz).ceil() as usize).max(1);
            let k1 =
                ((BAND_EDGES_HZ[b + 1] as f64 / short.bin_hz).ceil() as usize).min(top_bin(&short));
            if k1 > k0 {
                *bins = (k0, k1);
            }
        }
        let away = ((TONAL_AWAY_HZ as f64 / long.bin_hz).round() as usize).max(2);
        let tonal_bins = TonalBins {
            lo: ((TONAL_LO_HZ as f64 / long.bin_hz).ceil() as usize).max(away),
            hi: ((TONAL_HI_HZ as f64 / long.bin_hz) as usize).min(top_bin(&long) - away),
            away,
            half: (away / 2).max(1),
        };
        Self {
            sample_rate,
            decim,
            cic: (decim > 1).then(|| Cic::new(decim)),
            dc: (0.0, 0.0, 1.0 - std::f64::consts::TAU * DC_CUTOFF_HZ / fa),
            win,
            long_win,
            hop,
            short,
            long,
            band_bins,
            tonal_bins,
            buf: Vec::with_capacity(long_win + hop),
            pos: 0,
            seg_start: 0,
            seg_open: false,
            segments: Vec::new(),
            levels: Vec::new(),
            tonal: Vec::new(),
        }
    }

    /// Add the next samples (mono), in a piece of any size.
    pub fn push(&mut self, samples: &[f32]) {
        for &x in samples {
            let y = match &mut self.cic {
                Some(cic) => cic.push(x),
                None => Some(if x.is_finite() { x } else { 0.0 }),
            };
            self.pos += 1;
            let Some(y) = y else { continue };
            let (x1, y1, a) = self.dc;
            let out = y as f64 - x1 + a * y1;
            self.dc = (y as f64, out, a);
            self.buf.push(out as f32);
            if self.buf.len() >= self.long_win {
                self.frame();
                self.buf.drain(..self.hop);
            }
        }
    }

    /// `n` samples are missing from here on (a piece that could not be
    /// read). Later events keep their true times, and no event spans the gap.
    pub fn gap(&mut self, n: u64) {
        self.pos += n;
        self.buf.clear();
        if let Some(cic) = &mut self.cic {
            cic.reset();
        }
        self.dc = (0.0, 0.0, self.dc.2);
        self.seg_start = self.pos;
        self.seg_open = false;
    }

    /// End the scan: the events found, in time order.
    pub fn finish(self) -> Vec<NoiseEvent> {
        Analysis::new(&self).events()
    }

    fn frame(&mut self) {
        let first_of_segment = !self.seg_open;
        if first_of_segment {
            self.segments.push(Segment {
                first_frame: self.tonal.len(),
                start_sample: self.seg_start,
            });
            self.seg_open = true;
        }
        let off = (self.long_win - self.win) / 2;
        let short = &self.buf[off..off + self.win];
        if short.iter().all(|&v| v == 0.0) || !self.short.run(short) {
            self.levels.extend([0u16; BANDS]);
            self.tonal.push(0);
            return;
        }
        for &(k0, k1) in &self.band_bins {
            if k1 <= k0 {
                self.levels.push(0);
                continue;
            }
            let p: f32 = (k0..k1).map(|k| self.short.power(k)).sum();
            let db = (10.0 * (p as f64 / (k1 - k0) as f64 + 1e-30).log10()) as f32;
            self.levels.push(encode_db(db));
        }
        // Tonality changes slowly: measured every other frame.
        let tonal = if first_of_segment || self.tonal.len().is_multiple_of(2) {
            self.tonal_share()
        } else {
            *self.tonal.last().unwrap_or(&0)
        };
        self.tonal.push(tonal);
    }

    /// Share of the low part's power in narrow peaks (tones and harmonics),
    /// from the long frame, 0 to 255.
    fn tonal_share(&mut self) -> u8 {
        let TonalBins { lo, hi, away, half } = self.tonal_bins;
        if hi <= lo + 2 * away || !self.long.run(&self.buf[..self.long_win]) {
            return 0;
        }
        let base = lo - away;
        let p: Vec<f32> = (base..=hi + away).map(|k| self.long.power(k)).collect();
        let at = |k: usize| p[k - base];
        let total: f32 = (lo..=hi).map(at).sum();
        if total <= 0.0 {
            return 0;
        }
        let mut peak_power = 0.0f32;
        let mut k = lo;
        while k <= hi {
            let v = at(k);
            let is_peak = k >= lo + half
                && (k - half..=k + half).all(|j| at(j) <= v)
                && v >= TONAL_CONTRAST * at(k - away)
                && v >= TONAL_CONTRAST * at(k + away);
            if is_peak {
                peak_power += (k - half..=(k + half).min(hi)).map(at).sum::<f32>();
                k += half + 1;
            } else {
                k += 1;
            }
        }
        ((peak_power / total).clamp(0.0, 1.0) * 255.0).round() as u8
    }
}

/// Scan mono samples already in memory.
pub fn scan_noise(samples: &[f32], sample_rate: u32) -> Vec<NoiseEvent> {
    let mut scanner = NoiseScanner::new(sample_rate);
    scanner.push(samples);
    scanner.finish()
}

fn encode_db(db: f32) -> u16 {
    let db = db.max(MIN_LEVEL_DB);
    ((db - LEVEL_BASE_DB) * 100.0)
        .round()
        .clamp(1.0, u16::MAX as f32) as u16
}

fn decode_db(v: u16) -> Option<f32> {
    (v != 0).then(|| v as f32 / 100.0 + LEVEL_BASE_DB)
}

/// Power sum in dB of the given levels.
fn power_db(levels: impl Iterator<Item = (f32, f32)>) -> Option<f32> {
    let mut sum = 0.0f64;
    let mut weight = 0.0f64;
    for (db, w) in levels {
        sum += w as f64 * 10f64.powf(db as f64 / 10.0);
        weight += w as f64;
    }
    (weight > 0.0).then(|| (10.0 * (sum / weight).max(1e-30).log10()) as f32)
}

/// The value at percentile `p` of `values` in dB, to 0.1 dB, by histogram
/// (no sort).
fn percentile_db(values: impl Iterator<Item = f32>, p: f64) -> Option<f32> {
    const STEP: f32 = 0.1;
    let lo = MIN_LEVEL_DB;
    let n_bins = ((60.0 - lo) / STEP) as usize + 1;
    let mut hist = vec![0u32; n_bins];
    let mut count = 0u64;
    for v in values {
        let i = (((v - lo) / STEP).round().max(0.0) as usize).min(n_bins - 1);
        hist[i] += 1;
        count += 1;
    }
    if count == 0 {
        return None;
    }
    let rank = ((p / 100.0) * (count - 1) as f64).round() as u64;
    let mut seen = 0u64;
    for (i, &c) in hist.iter().enumerate() {
        seen += c as u64;
        if seen > rank {
            return Some(lo + i as f32 * STEP);
        }
    }
    None
}

/// A floor or typical level: percentile `p` of `values`, never under
/// [`MIN_FLOOR_DB`].
fn reference_db(values: impl Iterator<Item = f32>, p: f64) -> Option<f32> {
    percentile_db(values, p).map(|v| v.max(MIN_FLOOR_DB))
}

/// The stored levels turned into floors and per-frame measures.
struct Analysis<'a> {
    s: &'a NoiseScanner,
    frames: usize,
    /// Band bandwidths, 0 for unavailable bands.
    widths: [f32; BANDS],
    /// Each band's floor and typical (median) level.
    floors: [Option<f32>; BANDS],
    medians: [Option<f32>; BANDS],
    /// Per frame: the low part's level (power per Hz, dB); NaN when silent.
    low: Vec<f32>,
    high: Vec<f32>,
    /// The low part's typical level, per frame.
    low_median: Option<f32>,
    /// The low part over 50 ms, and its floor.
    low_s: Vec<f32>,
    low_s_floor: Option<f32>,
    high_s: Vec<f32>,
}

impl<'a> Analysis<'a> {
    fn new(s: &'a NoiseScanner) -> Self {
        let frames = s.levels.len() / BANDS;
        let mut widths = [0.0f32; BANDS];
        for b in 0..BANDS {
            let (k0, k1) = s.band_bins[b];
            if k1 > k0 {
                widths[b] = BAND_EDGES_HZ[b + 1] - BAND_EDGES_HZ[b];
            }
        }
        let mut floors = [None; BANDS];
        let mut medians = [None; BANDS];
        for b in 0..BANDS {
            if widths[b] > 0.0 {
                let band = || (0..frames).filter_map(move |t| decode_db(s.levels[t * BANDS + b]));
                floors[b] = reference_db(band(), FLOOR_PERCENTILE);
                medians[b] = reference_db(band(), 50.0);
            }
        }
        let mut a = Self {
            s,
            frames,
            widths,
            floors,
            medians,
            low: Vec::new(),
            high: Vec::new(),
            low_median: None,
            low_s: Vec::new(),
            low_s_floor: None,
            high_s: Vec::new(),
        };
        a.low = (0..frames).map(|t| a.part_db(t, LOW)).collect();
        a.high = (0..frames).map(|t| a.part_db(t, HIGH)).collect();
        a.low_median = reference_db(a.low.iter().copied().filter(|v| v.is_finite()), 50.0);
        a.low_s = a.smooth(&a.low);
        a.high_s = a.smooth(&a.high);
        a.low_s_floor = reference_db(
            a.low_s.iter().copied().filter(|v| v.is_finite()),
            FLOOR_PERCENTILE,
        );
        a
    }

    fn level(&self, t: usize, b: usize) -> Option<f32> {
        if self.widths[b] == 0.0 {
            return None;
        }
        decode_db(self.s.levels[t * BANDS + b])
    }

    /// The frame's loudest band level.
    fn frame_max(&self, t: usize) -> Option<f32> {
        (0..BANDS)
            .filter_map(|b| self.level(t, b))
            .max_by(f32::total_cmp)
    }

    /// Band `b`'s level over its typical level, `None` when the band is
    /// silent or more than [`LEAK_DB`] under the frame's loudest band.
    fn band_over_median(&self, t: usize, b: usize) -> Option<f32> {
        let l = self.level(t, b)?;
        if l < self.frame_max(t)? - LEAK_DB {
            return None;
        }
        Some(l - self.medians[b]?)
    }

    /// A part's power per Hz in dB; NaN when silent or unavailable.
    fn part_db(&self, t: usize, bands: std::ops::Range<usize>) -> f32 {
        power_db(bands.filter_map(|b| self.level(t, b).map(|l| (l, self.widths[b]))))
            .unwrap_or(f32::NAN)
    }

    fn segment_of(&self, t: usize) -> usize {
        self.s
            .segments
            .partition_point(|seg| seg.first_frame <= t)
            .saturating_sub(1)
    }

    /// Frames `[lo, hi)` of the segment holding `t`.
    fn segment_range(&self, t: usize) -> (usize, usize) {
        let i = self.segment_of(t);
        let lo = self.s.segments[i].first_frame;
        let hi = self
            .s
            .segments
            .get(i + 1)
            .map_or(self.frames, |s| s.first_frame);
        (lo, hi)
    }

    /// Power mean over [`SMOOTH_EACH_SIDE`] frames each side, within the
    /// segment.
    fn smooth(&self, v: &[f32]) -> Vec<f32> {
        let mut out = vec![f32::NAN; v.len()];
        for seg in 0..self.s.segments.len() {
            let lo = self.s.segments[seg].first_frame;
            let hi = self
                .s
                .segments
                .get(seg + 1)
                .map_or(self.frames, |s| s.first_frame);
            for t in lo..hi {
                if !v[t].is_finite() {
                    continue;
                }
                let a = t.saturating_sub(SMOOTH_EACH_SIDE).max(lo);
                let b = (t + SMOOTH_EACH_SIDE + 1).min(hi);
                out[t] = power_db(v[a..b].iter().filter(|x| x.is_finite()).map(|&x| (x, 1.0)))
                    .unwrap_or(f32::NAN);
            }
        }
        out
    }

    /// Band `b` over its floor, power mean over the 50 ms around `t`.
    fn band_ex_smoothed(&self, t: usize, b: usize) -> Option<f32> {
        let (lo, hi) = self.segment_range(t);
        let a = t.saturating_sub(SMOOTH_EACH_SIDE).max(lo);
        let e = (t + SMOOTH_EACH_SIDE + 1).min(hi);
        let max = self.frame_max(t)?;
        let l = power_db((a..e).filter_map(|u| self.level(u, b).map(|l| (l, 1.0))))?;
        if l < max - LEAK_DB {
            return None;
        }
        Some(l - self.floors[b]?)
    }

    /// Start of frame `t` in input samples.
    fn frame_start(&self, t: usize) -> u64 {
        let seg = self.s.segments[self.segment_of(t)];
        seg.start_sample + ((t - seg.first_frame) * self.s.hop) as u64 * self.s.decim as u64
    }

    /// Time of the middle of frame `t`, in seconds.
    fn frame_mid_s(&self, t: usize) -> f64 {
        let mid = self.frame_start(t) as f64 + (self.s.long_win as f64 * self.s.decim as f64) / 2.0;
        mid / self.s.sample_rate as f64
    }

    fn hop_s(&self) -> f64 {
        self.s.hop as f64 * self.s.decim as f64 / self.s.sample_rate as f64
    }

    /// Frames `[a, b]` as times: the hop around each frame's middle.
    fn span_s(&self, a: usize, b: usize) -> (f64, f64) {
        let half = self.hop_s() / 2.0;
        (
            (self.frame_mid_s(a) - half).max(0.0),
            self.frame_mid_s(b) + half,
        )
    }

    /// Tonal share of frame `t`, 0 to 1.
    fn tonal(&self, t: usize) -> f32 {
        self.s.tonal[t] as f32 / 255.0
    }

    /// Mean tonal share over the 50 ms around `t`.
    fn tonal_around(&self, t: usize) -> f32 {
        let (lo, hi) = self.segment_range(t);
        let a = t.saturating_sub(SMOOTH_EACH_SIDE).max(lo);
        let e = (t + SMOOTH_EACH_SIDE + 1).min(hi);
        (a..e).map(|u| self.tonal(u)).sum::<f32>() / (e - a) as f32
    }

    fn is_wind_frame(&self, t: usize) -> bool {
        let (Some(floor), l, h) = (self.low_s_floor, self.low_s[t], self.high_s[t]) else {
            return false;
        };
        if !l.is_finite() || l - floor < WIND_DB || self.tonal_around(t) > WIND_MAX_TONAL {
            return false;
        }
        if h.is_finite() && l - h < WIND_TILT_DB {
            return false;
        }
        LOW.filter(|&b| self.band_ex_smoothed(t, b).is_some_and(|e| e >= BAND_ON_DB))
            .count()
            >= 2
    }

    /// Band of an event from each band's level over its floor.
    fn extent(&self, ex: &[Option<f32>; BANDS]) -> (f32, f32) {
        let on = |b: usize| ex[b].is_some_and(|e| e >= EXTENT_DB);
        let Some(first) = (0..3).find(|&b| on(b)) else {
            return (0.0, BAND_EDGES_HZ[LOW.end]);
        };
        let mut last = first;
        while last + 1 < BANDS && on(last + 1) {
            last += 1;
        }
        let lo = if first == 0 {
            0.0
        } else {
            BAND_EDGES_HZ[first]
        };
        (lo, BAND_EDGES_HZ[last + 1])
    }

    fn events(&self) -> Vec<NoiseEvent> {
        let mut wind = self.wind_events();
        let handling = self.handling_events();
        // Handling within or next to wind is left to the wind: buffeting
        // makes thumps of its own.
        let margin = WIND_MERGE_S / 2.0;
        let mut out: Vec<NoiseEvent> = handling
            .into_iter()
            .filter(|h| {
                !wind
                    .iter()
                    .any(|w| h.start_s < w.end_s + margin && h.end_s > w.start_s - margin)
            })
            .collect();
        out.append(&mut wind);
        out.sort_erased_by(|a, b| a.start_s.total_cmp(&b.start_s));
        out
    }

    fn wind_events(&self) -> Vec<NoiseEvent> {
        let merge = (WIND_MERGE_S / self.hop_s()).round() as usize;
        let mut events = Vec::new();
        let mut t = 0;
        while t < self.frames {
            if !self.is_wind_frame(t) {
                t += 1;
                continue;
            }
            let (_, seg_end) = self.segment_range(t);
            let start = t;
            let mut last = t;
            let mut u = t + 1;
            while u < seg_end && u <= last + merge {
                if self.is_wind_frame(u) {
                    last = u;
                }
                u += 1;
            }
            t = last + 1;
            let (start_s, end_s) = self.span_s(start, last);
            if end_s - start_s < MIN_WIND_S {
                continue;
            }
            // Sustained, not a run of clicks: the median frame is loud too.
            let mut lows: Vec<f32> = (start..=last)
                .map(|u| self.low[u])
                .filter(|v| v.is_finite())
                .collect();
            let floor = self.low_s_floor.unwrap_or(f32::INFINITY);
            if percentile_db(lows.drain(..), 50.0).is_none_or(|m| m - floor < WIND_SUSTAIN_DB) {
                continue;
            }
            events.push(self.event(NoiseKind::Wind, start, last, &self.low_s, self.low_s_floor));
        }
        events
    }

    /// An event over frames `[a, b]`; its level from `part` over `floor`.
    fn event(
        &self,
        kind: NoiseKind,
        a: usize,
        b: usize,
        part: &[f32],
        floor: Option<f32>,
    ) -> NoiseEvent {
        let mut ex = [None; BANDS];
        for (band, slot) in ex.iter_mut().enumerate() {
            *slot = match kind {
                // Wind: each band's power mean over the event.
                NoiseKind::Wind => {
                    power_db((a..=b).filter_map(|t| self.level(t, band).map(|l| (l, 1.0))))
                        .and_then(|l| Some(l - self.floors[band]?))
                }
                // Handling: each band at its loudest.
                NoiseKind::Handling => (a..=b)
                    .filter_map(|t| self.band_over_median(t, band))
                    .max_by(f32::total_cmp),
            };
        }
        let (lo_hz, hi_hz) = self.extent(&ex);
        let level_db = (a..=b)
            .filter(|&t| part[t].is_finite())
            .map(|t| part[t] - floor.unwrap_or(part[t]))
            .fold(0.0f32, f32::max);
        let (start_s, end_s) = self.span_s(a, b);
        NoiseEvent {
            kind,
            start_s,
            end_s,
            lo_hz,
            hi_hz,
            level_db,
        }
    }

    /// The low part over its typical level at frame `t`.
    fn low_over_median(&self, t: usize) -> Option<f32> {
        let l = self.low[t];
        let typical = self.low_median?;
        l.is_finite().then_some(l - typical)
    }

    /// When frame `t` starts a sudden rise of the low part (at least
    /// `onset_db` within 20 ms, to at least `level_db` over its typical
    /// level): the level before it, over the typical level.
    fn onset(&self, t: usize, level_db: f32, onset_db: f32) -> Option<f32> {
        let now = self.low_over_median(t)?;
        let (seg_lo, _) = self.segment_range(t);
        if t < seg_lo + 4 || now < level_db {
            return None;
        }
        // Before: the frames 20 to 40 ms earlier (the frame just before
        // overlaps this one), power mean.
        let before = power_db(
            (t - 4..=t - 2)
                .filter_map(|u| self.low_over_median(u))
                .map(|e| (e, 1.0)),
        )?;
        (now - before >= onset_db).then_some(before)
    }

    /// Frames where sudden rises start that are too faint for handling
    /// (rain drops, drips).
    fn faint_onsets(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut t = 0;
        while t < self.frames {
            if self
                .onset(
                    t,
                    HANDLING_DB - TEXTURE_RELAX_DB,
                    ONSET_DB - TEXTURE_RELAX_DB,
                )
                .is_some()
            {
                // Only rises that stay under handling's level: rain runs on
                // down to faint drops, while knocks stand out on their own.
                let peak = (t..(t + 4).min(self.frames))
                    .filter_map(|u| self.low_over_median(u))
                    .fold(f32::MIN, f32::max);
                if peak < HANDLING_DB {
                    out.push(t);
                }
                t += 3;
            } else {
                t += 1;
            }
        }
        out
    }

    fn handling_events(&self) -> Vec<NoiseEvent> {
        let ex = |t: usize| self.low_over_median(t);
        let max_frames = (MAX_HANDLING_S / self.hop_s()).round() as usize;
        let texture_frames = (TEXTURE_WINDOW_S / 2.0 / self.hop_s()).round() as usize;
        let faint = self.faint_onsets();
        let mut events = Vec::new();
        let mut t = 0;
        while t < self.frames {
            let (_, seg_hi) = self.segment_range(t);
            let Some(before) = self.onset(t, HANDLING_DB, ONSET_DB) else {
                t += 1;
                continue;
            };
            // Among many sudden sounds (rain, drips, a typewriter) a thump
            // is one more drop, not handling.
            let near = faint.partition_point(|&u| u + texture_frames < t)
                ..faint.partition_point(|&u| u <= t + texture_frames);
            if near.len() > TEXTURE_MAX_ONSETS {
                t += 1;
                continue;
            }
            // Start: the first frame that rose clearly above `before`.
            let start = (t - 1..=t)
                .find(|&u| ex(u).is_some_and(|e| e >= before + RELEASE_DB))
                .unwrap_or(t);
            // End: the last frame before the low part falls under RELEASE_DB.
            let mut end = t;
            let mut u = t + 1;
            while u < seg_hi && ex(u).is_some_and(|e| e >= RELEASE_DB) {
                end = u;
                u += 1;
                if end - start > max_frames {
                    break;
                }
            }
            t = end + 1;
            if end - start > max_frames {
                // Too long: skip the rest of this sound.
                while t < seg_hi && ex(t).is_some_and(|e| e >= RELEASE_DB) {
                    t += 1;
                }
                continue;
            }
            let peak = (start..=end)
                .max_by(|&a, &b| ex(a).unwrap_or(-1e9).total_cmp(&ex(b).unwrap_or(-1e9)))
                .unwrap_or(t);
            let tilt = self.low[peak] - self.high[peak];
            if tilt.is_finite() && tilt < HANDLING_TILT_DB {
                continue;
            }
            if self.tonal(peak) > HANDLING_MAX_TONAL {
                continue;
            }
            let ev = self.event(NoiseKind::Handling, start, end, &self.low, self.low_median);
            if ev.lo_hz <= BAND_EDGES_HZ[1] && ev.hi_hz >= BAND_EDGES_HZ[5] {
                events.push(ev);
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::call_detect::test_util::{add, faded_chirp, noise, sine, Butterworth};

    const HISS: f32 = 0.002;

    fn secs(rate: u32, s: f64) -> usize {
        (s * rate as f64) as usize
    }

    fn hiss(rate: u32, len_s: f64) -> Vec<f32> {
        noise(HISS, secs(rate, len_s), 7)
    }

    /// Low-passed noise with a gusty envelope, `len_s` long.
    fn wind(rate: u32, len_s: f64, seed: u32) -> Vec<f32> {
        let n = secs(rate, len_s);
        let mut x = noise(1.0, n, seed);
        let mut lp = Butterworth::lowpass(4, rate as f32, 400.0);
        lp.process_in_place(&mut x);
        for (i, v) in x.iter_mut().enumerate() {
            let t = i as f64 / rate as f64;
            // Gusts: a few swells per second, fading in and out at the ends.
            let gust = 0.6 + 0.4 * (std::f64::consts::TAU * 1.7 * t).sin();
            let edge = (t / 0.05).min((len_s - t) / 0.05).clamp(0.0, 1.0);
            *v *= (0.5 * gust * edge) as f32;
        }
        x
    }

    /// A knock: a broadband click with a low thud under it, about 30 ms.
    fn thump(rate: u32, seed: u32) -> Vec<f32> {
        let n = secs(rate, 0.03);
        let click = noise(0.3, n, seed);
        (0..n)
            .map(|i| {
                let t = i as f64 / rate as f64;
                let thud = 0.5 * (std::f64::consts::TAU * 90.0 * t).sin() * (-t / 0.012).exp();
                click[i] * (-t / 0.006).exp() as f32 + thud as f32
            })
            .collect()
    }

    fn kinds(events: &[NoiseEvent]) -> Vec<NoiseKind> {
        events.iter().map(|e| e.kind).collect()
    }

    fn scene(rate: u32) -> Vec<f32> {
        let mut x = hiss(rate, 12.0);
        add(&mut x, secs(rate, 2.0), &wind(rate, 3.0, 11));
        add(&mut x, secs(rate, 8.0), &thump(rate, 5));
        add(&mut x, secs(rate, 10.0), &thump(rate, 9));
        x
    }

    fn check_scene(rate: u32, events: &[NoiseEvent]) {
        assert_eq!(
            kinds(events),
            vec![NoiseKind::Wind, NoiseKind::Handling, NoiseKind::Handling],
            "at {rate} Hz: {events:?}"
        );
        let w = &events[0];
        assert!((w.start_s - 2.0).abs() < 0.15, "{rate}: wind start {w:?}");
        assert!((w.end_s - 5.0).abs() < 0.15, "{rate}: wind end {w:?}");
        assert_eq!(w.lo_hz, 0.0);
        assert!(w.hi_hz >= 500.0 && w.hi_hz <= 2700.0, "{rate}: {w:?}");
        for (h, at) in events[1..].iter().zip([8.0, 10.0]) {
            assert!((h.start_s - at).abs() < 0.03, "{rate}: {h:?}");
            assert!(h.end_s - h.start_s < 0.2, "{rate}: {h:?}");
            assert!(h.hi_hz >= 1200.0, "{rate}: {h:?}");
        }
    }

    #[test]
    fn finds_wind_and_handling_at_many_rates() {
        for rate in [
            8_000, 22_050, 44_100, 48_000, 96_000, 250_000, 384_000, 500_000,
        ] {
            let events = scan_noise(&scene(rate), rate);
            check_scene(rate, &events);
        }
    }

    #[test]
    fn pieces_give_the_same_events() {
        let rate = 96_000;
        let x = scene(rate);
        let whole = scan_noise(&x, rate);
        let mut s = NoiseScanner::new(rate);
        let mut i = 0;
        let mut size = 1;
        while i < x.len() {
            let j = (i + size).min(x.len());
            s.push(&x[i..j]);
            i = j;
            size = size * 7 % 4099 + 1;
        }
        assert_eq!(s.finish(), whole);
    }

    #[test]
    fn gap_keeps_times() {
        let rate = 48_000;
        let x = scene(rate);
        let cut = secs(rate, 6.0);
        let missing = secs(rate, 1.0) as u64;
        let mut s = NoiseScanner::new(rate);
        s.push(&x[..cut]);
        s.gap(missing);
        s.push(&x[cut + missing as usize..]);
        check_scene(rate, &s.finish());
    }

    #[test]
    fn hiss_tone_and_chirps_are_quiet() {
        let rate = 48_000;
        let n = secs(rate, 10.0);
        // White hiss alone.
        assert_eq!(scan_noise(&hiss(rate, 10.0), rate), vec![]);
        // A steady low hum over hiss.
        let mut x = hiss(rate, 10.0);
        add(&mut x, 0, &sine(rate as f32, 120.0, 0.2, n));
        assert_eq!(scan_noise(&x, rate), vec![]);
        // A tone that starts and stops: tonal, so neither kind.
        let mut x = hiss(rate, 10.0);
        add(
            &mut x,
            secs(rate, 3.0),
            &faded_chirp(rate as f32, 300.0, 300.0, 0.3, secs(rate, 2.0)),
        );
        add(
            &mut x,
            secs(rate, 6.0),
            &faded_chirp(rate as f32, 400.0, 400.0, 0.3, secs(rate, 0.1)),
        );
        assert_eq!(scan_noise(&x, rate), vec![]);
        // Bird-like chirps from 2 to 4 kHz.
        let mut x = hiss(rate, 10.0);
        for k in 0..20 {
            let c = faded_chirp(rate as f32, 4000.0, 2000.0, 0.3, secs(rate, 0.08));
            add(&mut x, secs(rate, 0.5 + k as f64 * 0.45), &c);
        }
        assert_eq!(scan_noise(&x, rate), vec![]);
    }

    #[test]
    fn loud_bat_calls_do_not_fold_down() {
        for rate in [250_000, 384_000] {
            let mut x = noise(0.0005, secs(rate, 6.0), 3);
            for k in 0..50 {
                let at = secs(rate, 0.3 + k as f64 * 0.1);
                for (f0, f1) in [(80_000.0, 40_000.0), (50_000.0, 45_000.0)] {
                    let c = faded_chirp(rate as f32, f0, f1, 0.9, secs(rate, 0.005));
                    add(&mut x, at, &c);
                }
            }
            assert_eq!(scan_noise(&x, rate), vec![], "at {rate} Hz");
        }
    }

    #[test]
    fn rain_like_clicks_are_not_handling() {
        // Many tiny high-passed clicks: energy mostly above 1 kHz.
        let rate = 44_100;
        let mut x = hiss(rate, 10.0);
        let mut hp = Butterworth::highpass(4, rate as f32, 1500.0);
        for k in 0..200 {
            let mut c = noise(0.2, secs(rate, 0.002), 100 + k);
            hp.process_in_place(&mut c);
            add(&mut x, secs(rate, 0.1 + k as f64 * 0.049), &c);
        }
        assert_eq!(scan_noise(&x, rate), vec![]);
    }

    #[test]
    fn a_run_of_knocks_is_handling_not_wind() {
        // Ten loud knocks in 3 s: each is handling, and the run is not wind
        // (the frames between knocks are quiet) nor rain (no faint drops).
        let rate = 48_000;
        let mut x = hiss(rate, 10.0);
        for k in 0..10 {
            add(
                &mut x,
                secs(rate, 4.0 + k as f64 * 0.3),
                &thump(rate, 20 + k),
            );
        }
        let events = scan_noise(&x, rate);
        assert_eq!(kinds(&events), vec![NoiseKind::Handling; 10], "{events:?}");
    }

    #[test]
    fn faint_sounds_near_digital_silence_are_not_events() {
        // A recording high-passed far above 4 kHz: 0 to 4 kHz holds only
        // tiny residues, far below the noise of 16-bit samples.
        let rate = 384_000;
        let mut x = noise(1e-7, secs(rate, 6.0), 4);
        add(
            &mut x,
            secs(rate, 1.0),
            &wind(rate, 1.5, 3)
                .iter()
                .map(|v| v * 1e-5)
                .collect::<Vec<_>>(),
        );
        for k in 0..5 {
            let t: Vec<f32> = thump(rate, 40 + k).iter().map(|v| v * 1e-5).collect();
            add(&mut x, secs(rate, 3.5 + k as f64 * 0.4), &t);
        }
        assert_eq!(scan_noise(&x, rate), vec![]);
    }
}

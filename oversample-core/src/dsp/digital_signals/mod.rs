//! Electronic and digital signals in a recording: interference from power
//! supplies, LED drivers, motor controllers and data links.
//!
//! Two kinds are found:
//!
//! - **Square waves**, and other strongly non-sinusoidal steady tones such
//!   as PWM: a fundamental with a harmonic series whose levels fall off about
//!   as 1/n (a square wave has odd harmonics at 1/n and no even ones; PWM
//!   with duty d has harmonics shaped by sin(n pi d) / n). The pitch holds
//!   still, often for seconds or the whole file.
//! - **Modulated carriers** (QAM, PSK, FSK): a steady band-limited block of
//!   energy round a carrier with symbol structure in it, or a few tones that
//!   take turns at a steady loudness.
//!
//! [`DigitalScanner`] takes a recording's mono samples in pieces of any size,
//! like [`NoiseScanner`](super::noise_events::NoiseScanner).
//! [`scan_digital`] does the same for samples already in memory.
//!
//! ## Method
//!
//! The samples are read in **blocks** of 32 frames' length (0.68 to 1.05 s).
//! A frame is `next_pow2(rate / 50)` samples (bins 25 to 47 Hz wide), Hann
//! windowed, with half-frame steps, so a block has 63 frames. Only the
//! current block's samples and frame spectra are held; each block leaves a
//! few small findings, which are joined into events as blocks arrive.
//!
//! - **Lines**: narrow peaks in the block's mean spectrum at least
//!   [`LINE_DB`] over a floor (the 25th percentile of each 32 bins). A line
//!   is *steady* when, through a run of at least [`MIN_RUN_S`] (half that at
//!   a block's start or end), the most power within one bin of it is
//!   [`LINE_FRAME_DB`] over the floor and within [`FRAME_UNDER_MEAN_DB`] of
//!   its mean level in 85% of the frames (dropouts of up to
//!   [`MAX_DROPOUT_FRAMES`] frames are bridged).
//! - **Square waves**: from the lowest steady line up (at least
//!   [`MIN_FUNDAMENTAL_HZ`], with its third harmonic below Nyquist),
//!   harmonics are matched among the steady lines at n times the
//!   fundamental (within 0.6 bins, refining the fundamental by least squares
//!   as they are found). Each harmonic's level gives a falloff exponent beta
//!   (level = -20 beta log10 n): 1 for a square wave or sawtooth, 2 or more
//!   for the natural harmonics of a voice or call. A series is accepted when
//!   the median beta is at most [`MAX_SLOPE`] and either three harmonics are
//!   within [`HARMONIC_MARGIN_DB`] of the 1/n law, or two harmonics are found
//!   with an odd one at or over the law; or when only odd harmonics are there
//!   (even ones at least [`ODD_ONLY_DB`] under the law), two or more of them,
//!   with beta at most [`MAX_SLOPE_ODD`]; or when the fundamental is so high
//!   that only its third harmonic is below Nyquist and that harmonic is
//!   within the margin with no even one. The series is set aside when more
//!   unrelated lines lie among its harmonics than it has harmonics (a forest
//!   of lines, or the wrong fundamental), or when its two highest strong
//!   harmonics wander in frequency from frame to frame by more than
//!   [`MAX_JITTER_BINS`] bins plus [`MAX_JITTER_FRACTION`] of their
//!   frequency (insect wing tones and motors wander; oscillators do not). A
//!   square wave found in the block before carries on while its fundamental
//!   is still a steady line, even when other sound hides the harmonics.
//! - **Modulated, candidate bands**: a *plateau* is where the mean spectrum,
//!   smoothed over 3 bins, stands [`PLATEAU_ON_DB`] over its morphological
//!   opening (the floor under every feature narrower than [`MAX_BAND_HZ`]),
//!   at least 400 Hz wide, with a top [`PLATEAU_TOP_DB`] over the floor, its
//!   middle 60% within [`PLATEAU_FLAT_DB`] of the top (10th to 90th
//!   percentile), the spectrum falling [`PLATEAU_SKIRT_DB`] under the top
//!   within half its width on both sides, and its power in each frame
//!   staying [`PLATEAU_FRAME_DB`] over the floor's through a run. A
//!   *cluster* is two to [`FSK_MAX_TONES`] lines not in a square wave, at
//!   most [`FSK_MAX_GAP_HZ`] apart, that together are present in 90% of a
//!   run's frames.
//! - **Modulated, tests**: the candidate band (with 30% more either side, so
//!   the roll-off skirts that carry the symbol-rate feature are kept) is cut
//!   out of an FFT of the whole block and moved to 0 Hz as a complex signal
//!   z. A plateau needs a -3 dB width of [`MIN_BAND_HZ`] (narrower ones are
//!   mostly lossy-codec artifacts) and |z| that spreads by at most
//!   [`MOD_MAX_ENV_CV`] of its mean (noise spreads by 0.52, pulsed sound by
//!   more). Then it needs one of:
//!   - Symbol rate: a narrow line in the spectrum of |z|^2 between 0.4 and
//!     [`SYMBOL_MAX_WIDTHS`] times the -3 dB width (a root-raised-cosine
//!     signal's -3 dB width is its symbol rate), [`SYMBOL_LINE_DB`] over the
//!     local median, with no line [`PULSE_FAMILY_DB`] high at half, a third
//!     or twice its frequency and none nearly as strong below the range
//!     (pulsed insect song has a family of lines; a symbol stream has one).
//!     z is then passed through a root-raised-cosine matched filter and
//!     sampled once per symbol at the phase where |z|^2 peaks.
//!     kappa = E|s|^4 / (E|s|^2)^2 is 1 for PSK, 1.32 for 16-QAM and 2 for
//!     noise: the samples must have kappa under [`KAPPA_QAM`]; under
//!     [`KAPPA_PSK`] is labelled PSK, under [`KAPPA_QAM_LABEL`] QAM.
//!   - Carrier: a line near 0 Hz in the spectrum of z^2 (BPSK) or z^4 (QPSK
//!     and square QAM), [`POWER_ALONE_DB`] over the local median (with a
//!     symbol rate, [`POWER_LINE_DB`] is enough to refine the carrier), and
//!     no tone at the carrier in z itself ([`TONE_DB`]): a pure tone makes
//!     power lines too, a modulated carrier is suppressed. A z^2 line means
//!     BPSK.
//!   - Steady envelope: |z| spreads by at most [`CONSTANT_ENV_CV`] of its
//!     mean over a band at least [`MIN_STEADY_BAND_HZ`] wide with no bin of
//!     its fine spectrum [`STEADY_TONE_DB`] over the band's median (a
//!     frequency-swept clock or pest repeller; a tone in noise has a steady
//!     envelope too, but shows as a line). Its kind is left open.
//!
//!   A cluster is FSK when |z| spreads by at most [`FSK_MAX_ENV_CV`] of its
//!   mean, the instantaneous frequency (averaged over [`FSK_SMOOTH_S`]) is
//!   on one of the tones [`FSK_FIT`] of the time, at least two tones are
//!   used and it switches at least three times. The symbol rate comes from
//!   the dwell times when most are whole multiples of the shortest.
//!
//!   A band that continues an event is taken as more of it without the
//!   tests once the event has a symbol rate (or four tested blocks); the
//!   tests are run again every [`RECHECK_BLOCKS`] blocks to gather symbol
//!   rates and kinds.
//! - **Events**: findings that agree (fundamental within 0.3%, or bands
//!   overlapping by half with middles close) are one event, which carries on
//!   through up to [`MISSED_BLOCKS`] blocks in a row without a finding. Its
//!   start and end come from the runs of frames, so they are good to about
//!   20 ms. Events shorter than [`MIN_EVENT_S`] are dropped, events of one
//!   kind that overlap in time and band are joined, and a gap ends every
//!   event. An event's symbol rate is the median of its blocks' and its kind
//!   the most common.
//!
//! ## Cost
//!
//! Single-threaded release build, on the corpus run (2026-11): about 180x
//! real time at 384 kHz, 125x to 165x at 500 kHz, 440x at 192 kHz and 1100x
//! at 48 kHz. The frame FFTs are most of it. A block with a candidate band
//! adds one FFT of the whole block and a few of the band, so recordings with
//! a band in every block that fails the tests run at 40x to 80x (a band that
//! has failed twice in a row is tested again only every
//! [`RETEST_FAILED_BLOCKS`] blocks). Memory: at 500 kHz the scanner keeps
//! about 2 MB of samples, 2 MB of frame spectra and 4 to 6 MB of block-FFT
//! buffers; testing a 20 kHz-wide band peaked at 16 MB in all (11 MB at 384
//! kHz), and the widest bands with every test run could reach about twice
//! that for a moment.

mod modulated;
mod spectra;
mod square;

pub use modulated::ModFeatures;
pub use square::SeriesTrial;

use modulated::{
    analyze_cluster, analyze_plateau, find_clusters, find_plateaus, BlockFft, Cluster, ModHit,
    Plateau,
};
use spectra::{BlockSpectra, FrameFft, Run};

use crate::erased_sort::SortErased;

/// A line stands this far over the floor in the block's mean spectrum, dB.
pub const LINE_DB: f32 = 10.0;
/// In a frame, a line is present when within one bin of it the power is
/// this far over the floor, dB.
pub const LINE_FRAME_DB: f32 = 6.0;
/// In a frame, a line or band is present only within this of its mean level
/// over the block, dB.
pub const FRAME_UNDER_MEAN_DB: f32 = 8.0;
/// Three bins from a line's peak the spectrum is this far under the peak
/// (or within 3 dB of the floor), dB.
pub const LINE_SHOULDER_DB: f32 = 12.0;
/// Missing frames in a row that a run bridges.
pub const MAX_DROPOUT_FRAMES: usize = 2;
/// A steady line or band is present through at least this long, seconds
/// (half of it at a block's start or end).
pub const MIN_RUN_S: f64 = 0.3;
/// Events shorter than this are dropped, seconds.
pub const MIN_EVENT_S: f64 = 0.5;
/// Lowest fundamental looked for, Hz (and at least 5 bins).
pub const MIN_FUNDAMENTAL_HZ: f32 = 150.0;
const MIN_FUNDAMENTAL_BINS: f32 = 5.0;
/// Highest harmonic matched.
const MAX_HARMONIC: usize = 40;
/// A harmonic within this of the 1/n law counts as rich, dB.
pub const HARMONIC_MARGIN_DB: f32 = 8.0;
/// Highest median falloff exponent for a series with even harmonics.
pub const MAX_SLOPE: f32 = 1.3;
/// Highest median falloff exponent for an odd-only series.
pub const MAX_SLOPE_ODD: f32 = 1.8;
/// Even harmonics this far under the 1/n law count as missing, dB.
pub const ODD_ONLY_DB: f32 = 15.0;
/// A steady harmonic's frame-to-frame frequency spread at most this many
/// bins plus [`MAX_JITTER_FRACTION`] of its frequency.
pub const MAX_JITTER_BINS: f32 = 0.35;
pub const MAX_JITTER_FRACTION: f32 = 0.001;
/// A plateau stands this far over the floor, dB.
pub const PLATEAU_ON_DB: f32 = 8.0;
/// A plateau's top (median of its middle 60%) is this far over the floor, dB.
pub const PLATEAU_TOP_DB: f32 = 10.0;
/// A plateau's middle 60% is within this of its top (from the 10th to the
/// 90th percentile), dB.
pub const PLATEAU_FLAT_DB: f32 = 6.0;
/// Within half a plateau's width either side, the spectrum falls this far
/// under its top (or 3 dB less than the top's height over the floor), dB.
pub const PLATEAU_SKIRT_DB: f32 = 8.0;
/// In a frame, a plateau's band power is this far over the floor's, dB.
pub const PLATEAU_FRAME_DB: f32 = 6.0;
/// Widest plateau looked for, Hz (and at most 60% of the spectrum).
pub const MAX_BAND_HZ: f32 = 30_000.0;
/// The symbol rate is looked for from 0.4 to this many times the -3 dB
/// width (a root-raised-cosine signal's -3 dB width is its symbol rate;
/// other pulse shapes are narrower).
pub const SYMBOL_MAX_WIDTHS: f64 = 2.0;
/// Symbol-rate line height over the local median, dB.
pub const SYMBOL_LINE_DB: f32 = 18.0;
/// A line this high at half, a third or twice the symbol rate makes it a
/// pulse train, dB.
pub const PULSE_FAMILY_DB: f32 = 12.0;
/// z^2 or z^4 line height over the median, dB.
pub const POWER_LINE_DB: f32 = 20.0;
/// Without a symbol-rate line, a z^2 or z^4 line must be this high, dB.
pub const POWER_ALONE_DB: f32 = 28.0;
/// A modulated band's |z| spreads by at most this share of its mean (noise
/// spreads by 0.52, pulsed sound by more; root-raised-cosine QPSK by about
/// 0.2 and 16-QAM by 0.33).
pub const MOD_MAX_ENV_CV: f32 = 0.45;
/// A band whose |z| spreads by at most this share of its mean is a steady
/// carrier (PSK, or a frequency-modulated clock) even with no lines.
pub const CONSTANT_ENV_CV: f32 = 0.2;
/// A steady-envelope band has no bin of its fine spectrum this far over the
/// band's median, dB (noise alone reaches about 12 dB).
pub const STEADY_TONE_DB: f32 = 20.0;
/// A tone this far over the band's median at the carrier is not a
/// suppressed carrier, dB.
pub const TONE_DB: f32 = 15.0;
/// How far the carrier may be from the band's middle, as a share of the
/// -3 dB width.
const CARRIER_ERROR: f64 = 0.12;
/// kappa under this is PSK.
pub const KAPPA_PSK: f32 = 1.2;
/// kappa under this (and over [`KAPPA_PSK`]) is QAM.
pub const KAPPA_QAM: f32 = 1.8;
/// Only kappa under this is labelled QAM (16-QAM is 1.32, and noise raises
/// it); from here to [`KAPPA_QAM`] the kind is left open.
pub const KAPPA_QAM_LABEL: f32 = 1.55;
/// Most tones in an FSK cluster, and the widest gap between neighbours and
/// span, Hz.
pub const FSK_MAX_TONES: usize = 16;
pub const FSK_MAX_GAP_HZ: f32 = 3000.0;
pub const FSK_MAX_SPAN_HZ: f32 = 12_000.0;
/// FSK's |z| spread at most this share of its mean.
pub const FSK_MAX_ENV_CV: f32 = 0.3;
/// FSK's instantaneous frequency is averaged over this, seconds.
pub const FSK_SMOOTH_S: f64 = 0.0005;
/// FSK's instantaneous frequency is on a tone at least this share of the
/// time.
pub const FSK_FIT: f32 = 0.7;
/// Bands that continue an event are tested again every this many blocks.
pub const RECHECK_BLOCKS: u64 = 8;
/// An event carries on through this many blocks in a row without a
/// finding.
pub const MISSED_BLOCKS: u64 = 2;
/// A modulated band is at least this wide at -3 dB, Hz (narrower bands are
/// mostly lossy-codec artifacts and smeared tones).
pub const MIN_BAND_HZ: f32 = 500.0;
/// A band with a steady envelope and no lines is at least this wide at
/// -3 dB, Hz.
pub const MIN_STEADY_BAND_HZ: f32 = 1000.0;
/// A band that failed the tests in the two blocks before is tested again
/// only in every this many blocks.
pub const RETEST_FAILED_BLOCKS: u64 = 3;
/// Frames per block are `2 * BLOCK_FRAMES - 1`.
const BLOCK_FRAMES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DigitalKind {
    /// A square wave, PWM or similar steady tone with a harmonic series.
    SquareWave,
    /// A digitally modulated carrier (QAM, PSK, FSK).
    Modulated,
}

/// An electronic or digital signal found in a recording.
#[derive(Clone, Debug, PartialEq)]
pub struct DigitalEvent {
    pub kind: DigitalKind,
    pub start_s: f64,
    pub end_s: f64,
    /// Band covered: the fundamental up to the highest harmonic found, or the
    /// occupied band, Hz.
    pub lo_hz: f32,
    pub hi_hz: f32,
    /// Square wave or PWM: the fundamental, Hz.
    pub fundamental_hz: Option<f32>,
    /// Modulated: the carrier, Hz.
    pub carrier_hz: Option<f32>,
    /// Modulated: the symbol rate, when found, symbols per second.
    pub symbol_rate_hz: Option<f32>,
    /// Modulated: "QAM", "PSK" or "FSK", when known.
    pub modulation: Option<&'static str>,
}

/// One block's finding.
#[derive(Clone, Debug)]
struct Hit {
    kind: DigitalKind,
    start_s: f64,
    end_s: f64,
    lo_hz: f32,
    hi_hz: f32,
    /// Fundamental or carrier.
    freq: f32,
    symbol_rate_hz: Option<f32>,
    modulation: Option<&'static str>,
    /// Passed the tests in this block (not only continuing an event).
    tested: bool,
}

/// An event still being added to.
struct OpenEvent {
    kind: DigitalKind,
    start_s: f64,
    end_s: f64,
    lo_hz: f32,
    hi_hz: f32,
    freq_sum: f64,
    freq_n: u32,
    symbol_rates: Vec<f32>,
    votes: [u32; 3],
    last_block: u64,
    tested_block: u64,
}

const MODULATIONS: [&str; 3] = ["QAM", "PSK", "FSK"];

impl OpenEvent {
    fn freq(&self) -> f32 {
        (self.freq_sum / self.freq_n.max(1) as f64) as f32
    }

    fn matches(&self, h: &Hit, bin_hz: f32) -> bool {
        if self.kind != h.kind {
            return false;
        }
        let f = self.freq();
        match h.kind {
            DigitalKind::SquareWave => (h.freq - f).abs() <= (0.6 * bin_hz).max(0.003 * f),
            DigitalKind::Modulated => bands_match(
                (self.lo_hz, self.hi_hz, f),
                (h.lo_hz, h.hi_hz, h.freq),
                bin_hz,
            ),
        }
    }

    fn add(&mut self, h: &Hit, block: u64) {
        self.end_s = self.end_s.max(h.end_s);
        self.lo_hz = self.lo_hz.min(h.lo_hz);
        self.hi_hz = self.hi_hz.max(h.hi_hz);
        if h.tested {
            self.freq_sum += h.freq as f64;
            self.freq_n += 1;
            self.tested_block = block;
            if let Some(r) = h.symbol_rate_hz {
                if self.symbol_rates.len() < 64 {
                    self.symbol_rates.push(r);
                }
            }
            if let Some(m) = h.modulation {
                if let Some(i) = MODULATIONS.iter().position(|&x| x == m) {
                    self.votes[i] += 1;
                }
            }
        }
        self.last_block = block;
    }

    fn close(mut self) -> Option<DigitalEvent> {
        if self.end_s - self.start_s < MIN_EVENT_S {
            return None;
        }
        let f = self.freq();
        let symbol_rate_hz = (!self.symbol_rates.is_empty()).then(|| {
            self.symbol_rates.sort_by(f32::total_cmp);
            self.symbol_rates[self.symbol_rates.len() / 2]
        });
        let best = (0..3).max_by_key(|&i| self.votes[i]).unwrap();
        let modulation = (self.votes[best] > 0).then(|| MODULATIONS[best]);
        let square = self.kind == DigitalKind::SquareWave;
        Some(DigitalEvent {
            kind: self.kind,
            start_s: self.start_s,
            end_s: self.end_s,
            lo_hz: self.lo_hz,
            hi_hz: self.hi_hz,
            fundamental_hz: square.then_some(f),
            carrier_hz: (!square).then_some(f),
            symbol_rate_hz: if square { None } else { symbol_rate_hz },
            modulation: if square { None } else { modulation },
        })
    }
}

/// Two bands are the same signal: they overlap by half the narrower one's
/// width and their middles are close.
fn bands_match(a: (f32, f32, f32), b: (f32, f32, f32), bin_hz: f32) -> bool {
    let overlap = a.1.min(b.1) - a.0.max(b.0);
    let narrow = (a.1 - a.0).min(b.1 - b.0).max(bin_hz);
    let width = (a.1 - a.0).max(b.1 - b.0);
    overlap >= 0.5 * narrow && (a.2 - b.2).abs() <= (3.0 * bin_hz).max(0.1 * width)
}

/// Finds square waves and modulated carriers in a recording read in pieces.
pub struct DigitalScanner {
    rate: u32,
    /// Frame length and step, samples.
    n: usize,
    hop: usize,
    bins: usize,
    bin_hz: f32,
    block_len: usize,
    /// Frames a steady run needs, and at a block's start or end.
    run_min: usize,
    run_min_edge: usize,
    frame_fft: FrameFft,
    big: Option<BlockFft>,
    /// The current block's samples.
    buf: Vec<f32>,
    /// Input sample `buf[0]` is.
    block_start: u64,
    /// Blocks analysed (for continuity between them).
    block: u64,
    /// The current block's frame power spectra.
    power: Vec<f32>,
    open: Vec<OpenEvent>,
    done: Vec<DigitalEvent>,
    /// Candidates of the last block that touched its end but were not
    /// confirmed, so an event found in the next block can start earlier.
    pending: Vec<Hit>,
    /// Bands that failed the tests in blocks in a row: (low, high, middle,
    /// last block, failures in a row).
    failed: Vec<(f32, f32, f32, u64, u32)>,
    trace: Option<Vec<String>>,
}

impl DigitalScanner {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1000);
        let n = (rate as usize).div_ceil(50).next_power_of_two().max(64);
        let hop = n / 2;
        let bins = n / 2 + 1;
        let bin_hz = rate as f32 / n as f32;
        let block_len = BLOCK_FRAMES * n;
        let run_min = ((MIN_RUN_S * rate as f64 / hop as f64).ceil() as usize).max(4);
        Self {
            rate,
            n,
            hop,
            bins,
            bin_hz,
            block_len,
            run_min,
            run_min_edge: run_min.div_ceil(2),
            frame_fft: FrameFft::new(n),
            big: None,
            buf: Vec::with_capacity(block_len),
            block_start: 0,
            block: 0,
            power: Vec::new(),
            open: Vec::new(),
            done: Vec::new(),
            pending: Vec::new(),
            failed: Vec::new(),
            trace: None,
        }
    }

    /// Keep a text trace of every candidate and its measures, for tuning.
    #[doc(hidden)]
    pub fn set_trace(&mut self, on: bool) {
        self.trace = on.then(Vec::new);
    }

    #[doc(hidden)]
    pub fn take_trace(&mut self) -> Vec<String> {
        self.trace.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Add the next samples (mono), in a piece of any size.
    pub fn push(&mut self, samples: &[f32]) {
        let mut rest = samples;
        while !rest.is_empty() {
            let take = (self.block_len - self.buf.len()).min(rest.len());
            self.buf.extend(
                rest[..take]
                    .iter()
                    .map(|&x| if x.is_finite() { x } else { 0.0 }),
            );
            rest = &rest[take..];
            if self.buf.len() == self.block_len {
                self.analyze_block();
                self.block_start += self.block_len as u64;
                self.buf.clear();
            }
        }
    }

    /// `n` samples are missing from here on (a piece that could not be
    /// read). Later events keep their true times, and no event spans the gap.
    pub fn gap(&mut self, n: u64) {
        self.analyze_block();
        self.block_start += self.buf.len() as u64 + n;
        self.buf.clear();
        self.close_all();
        // No continuity across the gap.
        self.block += 1;
    }

    /// End the scan: the events found, in time order.
    pub fn finish(mut self) -> Vec<DigitalEvent> {
        self.analyze_block();
        self.close_all();
        let mut out = std::mem::take(&mut self.done);
        out.sort_erased_by(|a, b| a.start_s.total_cmp(&b.start_s));
        join_overlapping(out, self.bin_hz)
    }

    fn close_all(&mut self) {
        for e in self.open.drain(..) {
            if let Some(ev) = e.close() {
                self.done.push(ev);
            }
        }
        self.pending.clear();
    }

    fn time(&self, frame_offset: usize) -> f64 {
        (self.block_start + frame_offset as u64) as f64 / self.rate as f64
    }

    fn run_ok(&self, run: &Run, frames: usize) -> bool {
        let edge = run.first <= MAX_DROPOUT_FRAMES || run.last + 1 + MAX_DROPOUT_FRAMES >= frames;
        run.len() >= self.run_min.min(frames) || (edge && run.len() >= self.run_min_edge)
    }

    fn trace(&mut self, s: impl FnOnce() -> String) {
        if let Some(t) = &mut self.trace {
            t.push(s());
        }
    }

    fn analyze_block(&mut self) {
        let used = self.buf.len();
        if used < self.n {
            return;
        }
        let frames = (used - self.n) / self.hop + 1;
        if frames < self.run_min_edge {
            return;
        }
        let bins = self.bins;
        self.power.resize(frames * bins, 0.0);
        for f in 0..frames {
            let s = f * self.hop;
            self.frame_fft.power(
                &self.buf[s..s + self.n],
                &mut self.power[f * bins..(f + 1) * bins],
            );
        }
        let power = std::mem::take(&mut self.power);
        let hits = self.block_hits(&power, frames, used);
        self.power = power;
        self.merge(hits, frames);
        self.block += 1;
    }

    fn block_hits(&mut self, power: &[f32], frames: usize, used: usize) -> Vec<Hit> {
        let sp = BlockSpectra::new(power, frames, self.bins, self.bin_hz);
        let nyquist = self.rate as f32 / 2.0;
        let min_bin = ((100.0 / self.bin_hz) as usize).max(3);
        let lines = sp.lines(min_bin, self.bins - 1);
        let steady: Vec<bool> = lines
            .iter()
            .map(|l| self.run_ok(&l.run, frames) && l.run.present * 100 >= 85 * l.run.len())
            .collect();
        let frame_time =
            |me: &Self, r: &Run| (me.time(r.first * me.hop), me.time(r.last * me.hop + me.n));
        let mut hits = Vec::new();
        if self.trace.is_some() && std::env::var_os("DIGITAL_TRACE_LINES").is_some() {
            let t0 = self.time(0);
            for (l, st) in lines.iter().zip(&steady) {
                self.trace(|| {
                    format!(
                        "{t0:.2}s line {:.1} Hz {:.1} dB snr {:.1} run {}-{} ({}) steady {st}",
                        l.freq, l.level_db, l.snr_db, l.run.first, l.run.last, l.run.present
                    )
                });
            }
        }

        let mut trials = self.trace.is_some().then(Vec::new);
        let series = square::find_series(&sp, &lines, &steady, nyquist, trials.as_mut());
        if let Some(trials) = trials {
            let t0 = self.time(0);
            for t in trials {
                self.trace(|| format!("{t0:.2}s series {t:?}"));
            }
        }
        // Lines in an accepted series are not FSK tones.
        let mut skip = vec![false; lines.len()];
        for s in &series {
            for (j, l) in lines.iter().enumerate() {
                let n = (l.freq / s.f0).round();
                if n >= 1.0 && l.freq <= s.hi_hz && (l.freq - n * s.f0).abs() <= self.bin_hz {
                    skip[j] = true;
                }
            }
        }
        for s in &series {
            let (start_s, end_s) = frame_time(self, &s.run);
            hits.push(Hit {
                kind: DigitalKind::SquareWave,
                start_s,
                end_s,
                lo_hz: s.lo_hz,
                hi_hz: s.hi_hz,
                freq: s.f0,
                symbol_rate_hz: None,
                modulation: None,
                tested: true,
            });
        }
        // A square wave from the block before whose fundamental is still a
        // steady line carries on, even if other sound hides its harmonics
        // here.
        for e in &self.open {
            if e.kind != DigitalKind::SquareWave
                || e.last_block + 1 != self.block
                || hits.iter().any(|h| e.matches(h, self.bin_hz))
            {
                continue;
            }
            let f = e.freq();
            let tol = (0.6 * self.bin_hz).max(0.003 * f);
            if let Some((l, _)) = lines
                .iter()
                .zip(&steady)
                .find(|(l, &st)| st && (l.freq - f).abs() <= tol)
            {
                let (start_s, end_s) = frame_time(self, &l.run);
                hits.push(Hit {
                    kind: DigitalKind::SquareWave,
                    start_s,
                    end_s,
                    lo_hz: e.lo_hz,
                    hi_hz: e.hi_hz,
                    freq: l.freq,
                    symbol_rate_hz: None,
                    modulation: None,
                    tested: false,
                });
            }
        }

        let max_bins = ((MAX_BAND_HZ / self.bin_hz) as usize).min(self.bins * 3 / 5);
        let run_ok = |r: &Run| self.run_ok(r, frames);
        let mut rejects = self.trace.is_some().then(Vec::new);
        let plateaus = find_plateaus(&sp, max_bins, min_bin, &run_ok, rejects.as_mut());
        let clusters = find_clusters(&sp, &lines, &skip, &run_ok);
        if let Some(r) = rejects {
            let t0 = self.time(0);
            for line in r {
                self.trace(|| format!("{t0:.2}s {line}"));
            }
        }
        enum Cand {
            Plateau(Plateau),
            Cluster(Cluster),
        }
        let cands: Vec<Cand> = plateaus
            .into_iter()
            .map(Cand::Plateau)
            .chain(clusters.into_iter().map(Cand::Cluster))
            .collect();
        drop(sp);
        let mut big_done = false;
        for cand in cands {
            let (lo, hi, run) = match &cand {
                Cand::Plateau(p) => (p.lo_hz, p.hi_hz, p.run),
                Cand::Cluster(c) => (
                    c.freqs[0] - 2.0 * self.bin_hz,
                    c.freqs[c.freqs.len() - 1] + 2.0 * self.bin_hz,
                    c.run,
                ),
            };
            let (start_s, end_s) = frame_time(self, &run);
            let mid = 0.5 * (lo + hi);
            let mut hit = Hit {
                kind: DigitalKind::Modulated,
                start_s,
                end_s,
                lo_hz: lo,
                hi_hz: hi,
                freq: mid,
                symbol_rate_hz: None,
                modulation: None,
                tested: false,
            };
            // A band that carries on an event from the block before.
            let cont = self.open.iter().find(|e| {
                e.last_block + 1 + MISSED_BLOCKS >= self.block && e.matches(&hit, self.bin_hz)
            });
            if let Some(e) = cont {
                // Until a symbol rate is known, a few more blocks are tested.
                let settled = !e.symbol_rates.is_empty() || e.freq_n >= 4;
                if settled && self.block < e.tested_block + RECHECK_BLOCKS {
                    hit.freq = e.freq();
                    hits.push(hit);
                    continue;
                }
            }
            let continuing = cont.is_some();
            // A band that has failed the tests in the blocks before is
            // tested again only every RETEST_FAILED_BLOCKS blocks (a steady
            // hump of noise would otherwise be tested in every block).
            let block = self.block;
            let bin_hz = self.bin_hz;
            let failed_at = self.failed.iter().position(|f| {
                f.3 + 1 == block && bands_match((f.0, f.1, f.2), (lo, hi, mid), bin_hz)
            });
            let skip = !continuing
                && failed_at.is_some_and(|i| {
                    self.failed[i].4 >= 2 && !block.is_multiple_of(RETEST_FAILED_BLOCKS)
                });
            if skip {
                let i = failed_at.unwrap();
                self.failed[i].3 = block;
                if run.last + 1 + MAX_DROPOUT_FRAMES >= frames {
                    hits.push(hit);
                }
                continue;
            }
            if !big_done {
                let len = self.block_len;
                let big = self.big.get_or_insert_with(|| BlockFft::new(len));
                big.run(&self.buf);
                big_done = true;
            }
            let big = self.big.as_ref().unwrap();
            let fs = self.rate as f64;
            let mut feats = ModFeatures::default();
            let found: Option<ModHit> = match &cand {
                Cand::Plateau(p) => analyze_plateau(big, fs, p, self.hop, self.n, used, &mut feats),
                Cand::Cluster(c) => {
                    analyze_cluster(big, fs, c, self.bin_hz, self.hop, self.n, used, &mut feats)
                }
            };
            let t0 = self.time(0);
            if self.trace.is_some() {
                let what = match &cand {
                    Cand::Plateau(p) => format!(
                        "plateau {:.0}-{:.0} b3 {:.0} top {:.1} run {}-{}",
                        p.lo_hz,
                        p.hi_hz,
                        p.b3(),
                        p.top_db,
                        p.run.first,
                        p.run.last
                    ),
                    Cand::Cluster(c) => {
                        format!("cluster {:?} run {}-{}", c.freqs, c.run.first, c.run.last)
                    }
                };
                self.trace(|| format!("{t0:.2}s {what} -> {:?} {feats:?}", found.is_some()));
            }
            if found.is_some() {
                if let Some(i) = failed_at {
                    self.failed.swap_remove(i);
                }
            }
            match found {
                Some(m) => {
                    hit.lo_hz = m.lo_hz;
                    hit.hi_hz = m.hi_hz;
                    hit.freq = m.carrier_hz;
                    hit.symbol_rate_hz = m.symbol_rate_hz;
                    hit.modulation = m.modulation;
                    hit.tested = true;
                    hits.push(hit);
                }
                None if continuing => hits.push(hit),
                None => {
                    match failed_at {
                        Some(i) => {
                            self.failed[i].3 = block;
                            self.failed[i].4 += 1;
                        }
                        None => self.failed.push((lo, hi, mid, block, 1)),
                    }
                    if run.last + 1 + MAX_DROPOUT_FRAMES >= frames {
                        // Kept for one block: an event found next can start
                        // here.
                        hit.tested = false;
                        hits.push(hit);
                    }
                }
            }
        }
        let block = self.block;
        self.failed.retain(|f| f.3 == block);
        hits
    }

    fn merge(&mut self, hits: Vec<Hit>, frames: usize) {
        let block = self.block;
        let mut pending = Vec::new();
        let _ = frames;
        for h in hits {
            let idx = self.open.iter().position(|e| {
                e.last_block + 1 + MISSED_BLOCKS >= block && e.matches(&h, self.bin_hz)
            });
            match idx {
                Some(i) => self.open[i].add(&h, block),
                None if h.tested => {
                    let mut start_s = h.start_s;
                    // An untested band in the block before that ran up to
                    // this one.
                    if let Some(p) = self.pending.iter().find(|p| {
                        p.kind == h.kind
                            && bands_match(
                                (p.lo_hz, p.hi_hz, p.freq),
                                (h.lo_hz, h.hi_hz, h.freq),
                                self.bin_hz,
                            )
                    }) {
                        start_s = start_s.min(p.start_s);
                    }
                    let mut e = OpenEvent {
                        kind: h.kind,
                        start_s,
                        end_s: h.end_s,
                        lo_hz: h.lo_hz,
                        hi_hz: h.hi_hz,
                        freq_sum: 0.0,
                        freq_n: 0,
                        symbol_rates: Vec::new(),
                        votes: [0; 3],
                        last_block: block,
                        tested_block: block,
                    };
                    e.add(&h, block);
                    self.open.push(e);
                }
                None => pending.push(h),
            }
        }
        self.pending = pending;
        let mut i = 0;
        while i < self.open.len() {
            if self.open[i].last_block + MISSED_BLOCKS < block {
                let e = self.open.swap_remove(i);
                if let Some(ev) = e.close() {
                    self.done.push(ev);
                }
            } else {
                i += 1;
            }
        }
    }
}

/// Joins events of one kind that overlap in time and share most of a band
/// (two findings of one signal whose middles drifted apart). `events` is in
/// time order, and so is the result.
fn join_overlapping(events: Vec<DigitalEvent>, bin_hz: f32) -> Vec<DigitalEvent> {
    let mut out: Vec<DigitalEvent> = Vec::with_capacity(events.len());
    for e in events {
        let same = out.iter_mut().find(|o| {
            o.kind == e.kind
                && e.start_s <= o.end_s
                && match e.kind {
                    DigitalKind::SquareWave => {
                        let (a, b) = (
                            o.fundamental_hz.unwrap_or(0.0),
                            e.fundamental_hz.unwrap_or(0.0),
                        );
                        (a - b).abs() <= (0.6 * bin_hz).max(0.003 * a)
                    }
                    DigitalKind::Modulated => {
                        let overlap = o.hi_hz.min(e.hi_hz) - o.lo_hz.max(e.lo_hz);
                        let narrow = (o.hi_hz - o.lo_hz).min(e.hi_hz - e.lo_hz).max(bin_hz);
                        overlap >= 0.5 * narrow
                    }
                }
        });
        match same {
            Some(o) => {
                // The longer one's details stand.
                if e.end_s - e.start_s > o.end_s - o.start_s {
                    o.fundamental_hz = e.fundamental_hz;
                    o.carrier_hz = e.carrier_hz;
                    o.symbol_rate_hz = e.symbol_rate_hz.or(o.symbol_rate_hz);
                    o.modulation = e.modulation.or(o.modulation);
                }
                o.end_s = o.end_s.max(e.end_s);
                o.lo_hz = o.lo_hz.min(e.lo_hz);
                o.hi_hz = o.hi_hz.max(e.hi_hz);
            }
            None => out.push(e),
        }
    }
    out
}

/// Scan mono samples already in memory.
pub fn scan_digital(samples: &[f32], sample_rate: u32) -> Vec<DigitalEvent> {
    let mut scanner = DigitalScanner::new(sample_rate);
    scanner.push(samples);
    scanner.finish()
}

#[cfg(test)]
mod tests;

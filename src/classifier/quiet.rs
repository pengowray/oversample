//! Quiet windows: the windows of a classifier run with no sound above the
//! recording's background noise. Sound groups (`super::sound_groups`) leave
//! them out, so 2 groups are two kinds of sound, not sound and silence.
//!
//! Method, on the band the model looks at ([`model_band`]; the caller passes
//! it) and in frames of 1/150 of a window (20 ms for BirdNET's 3 s):
//!
//! - the band is split into up to [`MAX_BANDS`] parts, evenly on a log
//!   frequency scale, none narrower than [`MIN_BAND_CELLS`] / frame length
//!   (1.2 kHz for BirdNET, 6.4 kHz for BattyBirdNET). A narrowband call (a
//!   bird's whistle, a bat's 22 kHz call) stands about 10 dB higher over the
//!   noise in its own part than over the noise of the whole band;
//! - each frame's level in each part is its mean square in dB;
//! - each part's floor is the 10th percentile of its levels over the file;
//! - a window has sound when, in any part, its loudest frame is at least
//!   [`CLICK_DB`] over that part's floor (a click or a call of one or two
//!   frames), its 3rd loudest frame is at least [`SHORT_CALL_DB`] over (a
//!   short call), or its frame at the 95th percentile is at least
//!   [`LONG_CALL_DB`] over (a faint call that lasts); otherwise it is quiet;
//! - when no window's 3rd loudest frame is [`CONTRAST_DB`] over the floor in
//!   any part, no window is quiet: a recording of steady noise and nothing
//!   else has no background to tell apart;
//! - digital silence is always quiet.
//!
//! So noise with texture (rain drops, insects, a shopping mall) has sound in
//! every window, while steady hiss is quiet: the same hiss stretch is quiet
//! in a recording that also has calls. One loud event does not turn rain
//! into quiet, because each rain window has its own drops. Noise that only
//! swells and fades slowly does lose the windows at the bottom of the swell
//! when something louder happens: a lull in a recording of howling wind is
//! quiet.
//!
//! Tuned on real recordings (Gentoo Penguin XC540767, bat soundscapes
//! XC809053 and XC825336, Cardioderma XC843108, a jaguar, Baird's Sandpiper,
//! rain, wind, crickets, a shopping mall, people talking).
//!
//! Levels come from one FFT per frame. The samples are differenced twice
//! first, which keeps rumble under the band from leaking into it (the FFT
//! bins are scaled back). Each frame's FFT also takes in a short stretch of
//! the frames each side, under tapers whose squares add up to 1 with the
//! neighbours' tapers, so a click on a frame boundary is not lost and a loud
//! sound outside the band leaks little into it.

use std::ops::Range;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::RealToComplex;

use crate::classifier::Method;

/// Frames per model window.
pub const FRAMES_PER_WINDOW: usize = 150;
/// The loudest frame at least this far over the floor, in any part of the
/// band: a call shorter than 3 frames.
pub const CLICK_DB: f32 = 9.0;
/// The 3rd loudest frame at least this far over the floor, in any part of
/// the band: a short call.
pub const SHORT_CALL_DB: f32 = 7.0;
/// The 95th percentile frame at least this far over the floor, in any part
/// of the band: a faint call that lasts at least 5% of the window (150 ms of
/// 3 s).
pub const LONG_CALL_DB: f32 = 5.0;
/// When no window's 3rd loudest frame is this far over the floor in any
/// part of the band, no window is quiet apart from digital silence.
pub const CONTRAST_DB: f32 = 15.0;
/// Most parts the band is split into.
pub const MAX_BANDS: usize = 8;
/// Each part is at least this many times 1 / frame length wide, so in a
/// frame of plain noise its level varies by about 1 dB.
pub const MIN_BAND_CELLS: f32 = 24.0;
/// The floor's percentile among all frame levels.
const FLOOR_PERCENTILE: f64 = 10.0;
/// A frame whose mean square in the band is below this (-120 dBFS) counts
/// as digital silence.
const SILENT: f64 = 1e-12;
/// A frame whose samples are all under this (-120 dBFS) is digital silence.
const SILENT_AMPLITUDE: f32 = 1e-6;
/// A part's level is never put lower than this, in dB.
const LOWEST_DB: f32 = -140.0;
/// A part's level in hundredths of a dB, for a digitally silent frame.
const SILENT_CDB: i16 = i16::MIN;

/// The band the quiet check measures for `method`, in Hz: what the model's
/// own spectrograms cover, less the rumble under 250 Hz (a choice made
/// here, not the models'), scaled for BattyBirdNET.
///
/// Read from the mel filter banks inside the model files (the 2-D constant
/// matrices of the ONNX graphs):
/// - BirdNET V2.4 (`birdnet-v2.4-embeddings-fp32.onnx`, also the backbone
///   under BattyBirdNET's heads) makes two mel spectrograms of its 48 kHz
///   input: 96 bands over 23 to 2,977 Hz (2048-point FFT) and 96 bands over
///   516 to 14,953 Hz (1024-point FFT). BirdNET-Analyzer 1.5.1's `config.py`
///   agrees: `SIG_FMIN = 0`, `SIG_FMAX = 15000`, "model specific".
/// - Perch 2.0 (`perch_v2_embed.tract.onnx`) makes one mel spectrogram of
///   its 32 kHz input: 128 bands over 62 to 15,969 Hz (1024-point FFT), as
///   `research/perch.md` says (60 Hz to 16 kHz).
/// - BattyBirdNET gives BirdNET audio at 256 kHz as if it were 48 kHz, so
///   its band is BirdNET's times 256 / 48: up to 80 kHz. Its head files have
///   no spectrogram of their own.
pub fn model_band(method: Method) -> (f32, f32) {
    match method {
        Method::BirdNet => (250.0, 15_000.0),
        Method::Perch => (250.0, 16_000.0),
        Method::Bats => {
            let scale = method.model_rate() as f32 / 48_000.0;
            (250.0 * scale, 15_000.0 * scale)
        }
    }
}

/// Frame levels of a recording, for [`quiet_windows`].
#[derive(Clone, Debug, PartialEq)]
pub struct FrameLevels {
    /// Length of a frame in seconds; frame `i` starts at `i * frame_secs`.
    pub frame_secs: f64,
    /// Per frame, its level over the whole band in dB (full scale 0 dB);
    /// `f32::NEG_INFINITY` for digital silence.
    pub db: Vec<f32>,
    /// The parts of the band, in Hz.
    bands: Vec<(f32, f32)>,
    /// Per frame, then per part, the level in hundredths of a dB;
    /// [`SILENT_CDB`] for digital silence.
    band_cdb: Vec<i16>,
}

impl FrameLevels {
    /// The parts of the band the levels are measured in, in Hz.
    pub fn bands(&self) -> &[(f32, f32)] {
        &self.bands
    }

    /// Frame `frame`'s level in part `band`, in dB; `f32::NEG_INFINITY` for
    /// digital silence.
    fn band_db(&self, frame: usize, band: usize) -> f32 {
        match self.band_cdb[frame * self.bands.len() + band] {
            SILENT_CDB => f32::NEG_INFINITY,
            c => f32::from(c) / 100.0,
        }
    }
}

/// Measures [`FrameLevels`] from samples fed in pieces, so a long recording
/// need not be in memory at once.
pub struct LevelMeter {
    /// Samples fed.
    fed: usize,
    /// Samples in a frame.
    frame_len: usize,
    frame_secs: f64,
    /// Samples still needed: from the next frame's span (less 2 before
    /// it, for the differences), and always the last span's worth.
    pending: Vec<f32>,
    /// Index in the recording of `pending[0]`.
    start: usize,
    /// Weights over a frame's span: tapers of `taper` samples overlap the
    /// frames each side, flat between.
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Per FFT bin: the scale that undoes the differencing and turns power
    /// into the band's mean square.
    weight: Vec<f32>,
    /// Bins of each part of the band.
    bins: Vec<Range<usize>>,
    bands: Vec<(f32, f32)>,
    db: Vec<f32>,
    band_cdb: Vec<i16>,
}

/// Sum of `w * |c|^2`, in 8 lanes so it can use SIMD.
fn weighted_power(c: &[Complex<f32>], w: &[f32]) -> f64 {
    let mut lanes = [0.0f32; 8];
    let (cs, c_rest) = c.as_chunks::<8>();
    let (ws, w_rest) = w.as_chunks::<8>();
    for (c8, w8) in cs.iter().zip(ws) {
        for ((l, c), w) in lanes.iter_mut().zip(c8).zip(w8) {
            *l += w * (c.re * c.re + c.im * c.im);
        }
    }
    let tail: f32 = c_rest
        .iter()
        .zip(w_rest)
        .map(|(c, w)| w * c.norm_sqr())
        .sum();
    f64::from(lanes.iter().sum::<f32>() + tail)
}

/// The smallest FFT length of the form 2^k or 3 * 2^k that is at least `n`.
fn fft_len(n: usize) -> usize {
    let p = n.next_power_of_two();
    if p / 4 * 3 >= n {
        p / 4 * 3
    } else {
        p
    }
}

/// The parts of `lo..hi`: up to [`MAX_BANDS`], evenly on a log scale, each
/// at least `min_width` Hz (a part that would be narrower joins the next;
/// one left over at the top joins the one under it).
fn split_band(lo: f32, hi: f32, min_width: f32) -> Vec<(f32, f32)> {
    let ratio = (hi / lo).powf(1.0 / MAX_BANDS as f32);
    let mut out: Vec<(f32, f32)> = Vec::new();
    let mut start = lo;
    for i in 1..=MAX_BANDS {
        let end = if i == MAX_BANDS {
            hi
        } else {
            lo * ratio.powi(i as i32)
        };
        if end - start >= min_width {
            out.push((start, end));
            start = end;
        }
    }
    if start < hi {
        match out.last_mut() {
            Some(last) => last.1 = hi,
            None => out.push((lo, hi)),
        }
    }
    out
}

impl LevelMeter {
    /// Frames of `frame_secs` (at least 1 sample) at `sample_rate`, measured
    /// in `band` (Hz). The band's top is held under 0.45 of the rate.
    pub fn new(sample_rate: u32, frame_secs: f64, band: (f32, f32)) -> Self {
        let rate = f64::from(sample_rate.max(1));
        let top = f64::from(band.1).min(rate * 0.45);
        let lo = f64::from(band.0).clamp(0.0, top * 0.5).max(1.0);
        let frame_len = ((frame_secs * rate).round() as usize).max(1);
        // The tapers fill the FFT's spare length, at least 1/16 of a frame
        // and at most half.
        let n = fft_len(frame_len + frame_len / 16);
        let taper = ((n - frame_len).min(frame_len / 2)) / 2 * 2;
        let span = frame_len + taper;
        let mut window = vec![1.0f32; span];
        for t in 0..taper {
            let s = (std::f64::consts::PI * (t as f64 + 0.5) / (2.0 * taper as f64))
                .sin()
                .powi(2);
            let a = std::f64::consts::FRAC_PI_2 * s;
            window[t] = a.sin() as f32;
            window[frame_len + t] = a.cos() as f32;
        }
        let fft = oversample_core::dsp::fft::plan_fft_forward(n);
        let frame_secs = frame_len as f64 / rate;
        let bands = split_band(lo as f32, top as f32, MIN_BAND_CELLS / frame_secs as f32);
        let bin_hz = rate / n as f64;
        let bin_of = |f: f32| ((f64::from(f) / bin_hz).ceil() as usize).max(1);
        let bins: Vec<Range<usize>> = bands
            .iter()
            .map(|&(a, b)| bin_of(a)..bin_of(b).min(n / 2 + 1))
            .collect();
        // Mean square of the band = sum over its bins of 2 |X|^2 / (n * L),
        // since the window's squares add up to L over a span; the twice
        // differenced signal has (2 sin(pi k / n))^4 times the power.
        let weight: Vec<f32> = (0..=n / 2)
            .map(|k| {
                let d = 2.0 * (std::f64::consts::PI * k as f64 / n as f64).sin();
                if k == 0 {
                    0.0
                } else {
                    (2.0 / (n as f64 * frame_len as f64) / d.powi(4)) as f32
                }
            })
            .collect();
        Self {
            fed: 0,
            frame_len,
            frame_secs,
            pending: Vec::new(),
            start: 0,
            window,
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            weight,
            bins,
            bands,
            db: Vec::new(),
            band_cdb: Vec::new(),
        }
    }

    /// The next samples of the recording.
    pub fn push(&mut self, samples: &[f32]) {
        self.fed += samples.len();
        self.pending.extend_from_slice(samples);
        self.measure_pending(None);
    }

    /// Samples a frame is measured from: its span and 2 before it.
    fn reach(&self) -> usize {
        self.window.len() + 2
    }

    /// Where in the recording frame `i`'s samples start. Each span starts
    /// half a taper before its frame; the first starts at the recording's
    /// start, and with `end`, none runs past the end.
    fn span_start(&self, i: usize, end: Option<usize>) -> usize {
        let taper = self.window.len() - self.frame_len;
        let s = (i * self.frame_len).saturating_sub(taper / 2 + 2);
        end.map_or(s, |e| s.min(e.saturating_sub(self.reach())))
    }

    /// Measures every frame whose samples are all in `pending`, then drops
    /// the samples no later frame needs. With `end` (the recording's
    /// length), the last frames are measured from the last samples.
    fn measure_pending(&mut self, end: Option<usize>) {
        let reach = self.reach();
        let pending = std::mem::take(&mut self.pending);
        loop {
            let at = self.span_start(self.db.len(), end) - self.start;
            if at + reach > pending.len() || end.is_some_and(|_| self.db.len() >= self.target()) {
                break;
            }
            // Digital silence: every sample of the frame itself (not its
            // tapers) under -120 dBFS. Stops at the first sound.
            let i = self.db.len() * self.frame_len - self.start;
            let core = &pending[i..(i + self.frame_len).min(pending.len())];
            let silent = core.iter().all(|v| v.abs() < SILENT_AMPLITUDE);
            self.measure_frame(&pending[at..at + reach], silent);
        }
        self.pending = pending;
        let keep = self
            .span_start(self.db.len(), None)
            .min((self.start + self.pending.len()).saturating_sub(reach))
            .max(self.start);
        self.pending.drain(..keep - self.start);
        self.start = keep;
    }

    /// Frames in the recording: a last part-frame of at least half a frame
    /// counts.
    fn target(&self) -> usize {
        self.fed / self.frame_len + usize::from(self.fed % self.frame_len * 2 >= self.frame_len)
    }

    /// Measures one frame from its span and the 2 samples before it.
    fn measure_frame(&mut self, x: &[f32], silent: bool) {
        if silent {
            self.db.push(f32::NEG_INFINITY);
            self.band_cdb
                .extend(std::iter::repeat_n(SILENT_CDB, self.bands.len()));
            return;
        }
        // Differenced twice, then weighted.
        let span = self.window.len();
        for (((i, &w), &a), (&b, &c)) in self
            .input
            .iter_mut()
            .zip(&self.window)
            .zip(&x[2..])
            .zip(x[1..].iter().zip(x))
        {
            *i = (a - 2.0 * b + c) * w;
        }
        self.input[span..].fill(0.0);
        self.fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .expect("buffer lengths match the plan");
        let mut parts = [0.0f64; MAX_BANDS];
        for (p, r) in parts.iter_mut().zip(&self.bins) {
            *p = weighted_power(&self.spectrum[r.clone()], &self.weight[r.clone()]);
        }
        let parts = &parts[..self.bins.len()];
        let total: f64 = parts.iter().sum();
        if total < SILENT {
            self.db.push(f32::NEG_INFINITY);
            self.band_cdb
                .extend(std::iter::repeat_n(SILENT_CDB, parts.len()));
        } else {
            self.db.push((10.0 * total.log10()) as f32);
            self.band_cdb.extend(parts.iter().map(|&p| {
                let db = (10.0 * p.max(1e-30).log10()) as f32;
                (db.max(LOWEST_DB) * 100.0).round() as i16
            }));
        }
    }

    /// The levels; a last part-frame of at least half a frame counts.
    pub fn finish(mut self) -> FrameLevels {
        let frames = self.target();
        // A recording shorter than a span is padded with silence.
        let short = self.reach().saturating_sub(self.fed);
        self.pending.extend(std::iter::repeat_n(0.0, short));
        let end = self.fed + short;
        self.measure_pending(Some(end));
        self.db.truncate(frames);
        self.band_cdb.truncate(frames * self.bands.len());
        FrameLevels {
            frame_secs: self.frame_secs,
            db: self.db,
            bands: self.bands,
            band_cdb: self.band_cdb,
        }
    }
}

/// The value at percentile `p` (0 to 100) of sorted `v`, nearest rank.
fn percentile(sorted: &[f32], p: f64) -> f32 {
    let i = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// A window's loudest, 3rd loudest and 95th percentile frame levels in one
/// part of the band.
#[derive(Clone, Copy, Debug)]
struct Marks {
    loudest: f32,
    third: f32,
    p95: f32,
}

impl Marks {
    /// The marks of `f`, which is not empty; sorts it.
    fn of(f: &mut [f32]) -> Self {
        f.sort_by(|a, b| b.total_cmp(a));
        Self {
            loudest: f[0],
            third: f[2.min(f.len() - 1)],
            p95: f[((f.len() as f64 * 0.05).ceil() as usize).clamp(1, f.len()) - 1],
        }
    }

    /// The marks less `floor`.
    fn over(self, floor: f32) -> Self {
        Self {
            loudest: self.loudest - floor,
            third: self.third - floor,
            p95: self.p95 - floor,
        }
    }

    fn has_sound(self) -> bool {
        self.loudest >= CLICK_DB || self.third >= SHORT_CALL_DB || self.p95 >= LONG_CALL_DB
    }
}

/// The frames in window `(s, e)` (seconds): those that start in it.
fn window_frames(levels: &FrameLevels, (s, e): (f64, f64)) -> Range<usize> {
    // Frame starts are multiples of `frame_secs`; the small offset keeps a
    // start that is a whole number of frames from rounding up past itself.
    let index =
        |t: f64| ((t / levels.frame_secs - 1e-6).ceil().max(0.0) as usize).min(levels.db.len());
    let (a, b) = (index(s), index(e));
    a..b.max(a)
}

/// The floor of each part of the band, `None` when it has no sound at all.
fn floors(levels: &FrameLevels) -> Vec<Option<f32>> {
    (0..levels.bands.len())
        .map(|b| {
            let mut v: Vec<f32> = (0..levels.db.len())
                .map(|i| levels.band_db(i, b))
                .filter(|v| v.is_finite())
                .collect();
            if v.is_empty() {
                return None;
            }
            v.sort_by(f32::total_cmp);
            Some(percentile(&v, FLOOR_PERCENTILE))
        })
        .collect()
}

/// Per part of the band, the window's marks over that part's floor; empty
/// when the window has no frames or is all digital silence.
fn window_marks(levels: &FrameLevels, floors: &[Option<f32>], w: (f64, f64)) -> Vec<Marks> {
    let r = window_frames(levels, w);
    if levels.db[r.clone()].iter().all(|v| *v == f32::NEG_INFINITY) {
        return Vec::new();
    }
    let mut f = Vec::with_capacity(r.len());
    floors
        .iter()
        .enumerate()
        .filter_map(|(b, floor)| {
            let floor = (*floor)?;
            f.clear();
            f.extend(r.clone().map(|i| levels.band_db(i, b)));
            Some(Marks::of(&mut f).over(floor))
        })
        .collect()
}

/// Which of `windows` (start and end, in seconds) are quiet, by the rule in
/// the module notes. A window with no frames (past the end of the levels) is
/// not quiet.
pub fn quiet_windows(levels: &FrameLevels, windows: &[(f64, f64)]) -> Vec<bool> {
    let floors = floors(levels);
    let marks: Vec<Vec<Marks>> = windows
        .iter()
        .map(|&w| window_marks(levels, &floors, w))
        .collect();
    let contrast = marks.iter().flatten().any(|m| m.third >= CONTRAST_DB);
    windows
        .iter()
        .zip(&marks)
        .map(|(&w, m)| {
            if window_frames(levels, w).is_empty() {
                false
            } else if m.is_empty() {
                // All digital silence.
                true
            } else {
                contrast && !m.iter().any(|m| m.has_sound())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pca::SplitMix64;

    const RATE: u32 = 48_000;
    const BAND: (f32, f32) = (250.0, 15_000.0);

    /// `secs` of white noise at `rms`.
    fn noise(secs: f64, rms: f32, rng: &mut SplitMix64) -> Vec<f32> {
        let n = (secs * f64::from(RATE)) as usize;
        // Uniform in [-a, a] has RMS a / sqrt(3).
        let a = rms * 3f32.sqrt();
        (0..n)
            .map(|_| (rng.next_f64() as f32 * 2.0 - 1.0) * a)
            .collect()
    }

    /// Add a tone of `hz` and `rms` from `at` for `secs`, at `rate`.
    fn add_tone_at(x: &mut [f32], rate: u32, hz: f32, at: f64, secs: f64, rms: f32) {
        let a = rms * 2f32.sqrt();
        let start = (at * f64::from(rate)) as usize;
        let len = (secs * f64::from(rate)) as usize;
        // The phase in f64: in f32 a long tone's phase loses whole radians.
        let step = std::f64::consts::TAU * f64::from(hz) / f64::from(rate);
        for (i, v) in x[start..start + len].iter_mut().enumerate() {
            *v += a * ((step * i as f64) % std::f64::consts::TAU).sin() as f32;
        }
    }

    /// Add a 3 kHz tone of `rms` from `at` for `secs`.
    fn add_tone(x: &mut [f32], at: f64, secs: f64, rms: f32) {
        add_tone_at(x, RATE, 3000.0, at, secs, rms);
    }

    fn db_to_amp(db: f32) -> f32 {
        10f32.powf(db / 20.0)
    }

    /// 3 s windows over `x`, and which are quiet.
    fn run(x: &[f32]) -> Vec<bool> {
        let mut m = LevelMeter::new(RATE, 3.0 / FRAMES_PER_WINDOW as f64, BAND);
        // In uneven pieces, as a long file is read.
        for piece in x.chunks(7_001) {
            m.push(piece);
        }
        let levels = m.finish();
        let n = x.len().div_ceil(3 * RATE as usize);
        let windows: Vec<(f64, f64)> = (0..n)
            .map(|i| (i as f64 * 3.0, (i + 1) as f64 * 3.0))
            .collect();
        quiet_windows(&levels, &windows)
    }

    /// 10 windows of noise at -60 dBFS; loud calls (40 dB over the noise) in
    /// windows 0 and 5, so the recording has a clear quiet background.
    fn base(rng: &mut SplitMix64) -> Vec<f32> {
        let noise_rms = db_to_amp(-60.0);
        let mut x = noise(30.0, noise_rms, rng);
        add_tone(&mut x, 0.5, 1.0, noise_rms * db_to_amp(40.0));
        add_tone(&mut x, 15.5, 1.0, noise_rms * db_to_amp(40.0));
        x
    }

    /// The share of white noise's power at `rate` that falls in `lo..hi` Hz,
    /// as an amplitude factor.
    fn share(lo: f32, hi: f32, rate: u32) -> f32 {
        ((hi - lo) / (rate as f32 / 2.0)).sqrt()
    }

    #[test]
    fn bands_split_the_band() {
        let m = LevelMeter::new(RATE, 0.02, BAND);
        let b = &m.bands;
        assert_eq!(b.len(), 5);
        assert_eq!(b[0].0, 250.0);
        assert_eq!(b[b.len() - 1].1, 15_000.0);
        for w in b.windows(2) {
            assert_eq!(w[0].1, w[1].0);
        }
        for &(lo, hi) in b {
            assert!(hi - lo >= MIN_BAND_CELLS / 0.02 - 1.0);
        }
        // A narrow band is one part.
        assert_eq!(split_band(250.0, 900.0, 1_200.0), vec![(250.0, 900.0)]);
    }

    #[test]
    fn levels_match_the_signal() {
        // A tone at -20 dBFS measures about -20 dB, at 48 kHz and at
        // 384 kHz, and frame times line up with the samples.
        let mut x = vec![0.0f32; RATE as usize];
        add_tone(&mut x, 0.0, 1.0, db_to_amp(-20.0));
        let mut m = LevelMeter::new(RATE, 0.02, BAND);
        m.push(&x);
        let l = m.finish();
        assert_eq!(l.db.len(), 50);
        for &v in &l.db[2..48] {
            assert!((v + 20.0).abs() < 0.3, "{v}");
        }
        let rate = 384_000;
        let mut x = vec![0.0f32; rate as usize];
        add_tone_at(&mut x, rate, 40_000.0, 0.5, 0.25, db_to_amp(-20.0));
        let mut m = LevelMeter::new(rate, 0.0125, (1_333.0, 80_000.0));
        m.push(&x);
        let l = m.finish();
        assert_eq!(l.db.len(), 80);
        for (i, &v) in l.db.iter().enumerate() {
            let t = i as f64 * l.frame_secs;
            if t > 0.51 && t < 0.73 {
                assert!((v + 20.0).abs() < 0.3, "{t} {v}");
            }
            if t < 0.48 || t > 0.77 {
                assert!(v < -60.0, "{t} {v}");
            }
        }
    }

    #[test]
    fn tone_bursts_over_noise() {
        let mut rng = SplitMix64::new(1);
        let q = run(&base(&mut rng));
        let mut want = vec![true; 10];
        want[0] = false;
        want[5] = false;
        assert_eq!(q, want);
    }

    #[test]
    fn faint_long_call_is_not_quiet() {
        let noise_rms = db_to_amp(-60.0);
        let in_band = share(BAND.0, BAND.1, RATE);
        for over in [6.0, 8.0, 10.0] {
            let mut rng = SplitMix64::new(2);
            let mut x = base(&mut rng);
            // Tone power `over` dB above the noise's in the band, for 0.5 s.
            add_tone(&mut x, 9.8, 0.5, noise_rms * in_band * db_to_amp(over));
            let q = run(&x);
            assert!(!q[3], "a 0.5 s call {over} dB over the noise is quiet");
            assert!(q[2] && q[4]);
        }
    }

    #[test]
    fn short_call_is_not_quiet() {
        // 100 ms, 15 dB over the noise, in an otherwise silent 3 s window.
        let noise_rms = db_to_amp(-60.0);
        let mut rng = SplitMix64::new(3);
        let mut x = base(&mut rng);
        add_tone(&mut x, 22.0, 0.1, noise_rms * db_to_amp(15.0));
        let q = run(&x);
        assert!(!q[7]);
        // One 20 ms call (a single frame), 25 dB over the noise.
        let mut rng = SplitMix64::new(3);
        let mut x = base(&mut rng);
        add_tone(&mut x, 22.0, 0.02, noise_rms * db_to_amp(25.0));
        assert!(!run(&x)[7]);
        // A 40 ms tick 3 dB over the noise in its own part of the band
        // (1.9 to 3.2 kHz) is too short and too faint.
        let mut rng = SplitMix64::new(3);
        let mut x = base(&mut rng);
        let part = share(1_932.0, 3_220.0, RATE);
        add_tone(&mut x, 22.0, 0.04, noise_rms * part * db_to_amp(3.0));
        assert!(run(&x)[7]);
    }

    #[test]
    fn faint_narrow_calls_are_not_quiet() {
        // As in XC809053: BattyBirdNET windows at 256 kHz, three 5 ms calls
        // at 22 kHz only about 4 dB over the noise of the whole band (1.3 to
        // 80 kHz), but 11 dB over the noise of their own part.
        let rate = 256_000u32;
        let win = 0.5625;
        let band = (1_333.0, 80_000.0);
        let mut rng = SplitMix64::new(10);
        let noise_rms = db_to_amp(-70.0);
        let a = noise_rms * 3f32.sqrt();
        let n = (10.0 * win * f64::from(rate)) as usize;
        let mut x: Vec<f32> = (0..n)
            .map(|_| (rng.next_f64() as f32 * 2.0 - 1.0) * a)
            .collect();
        // Loud passes in windows 0 and 5.
        add_tone_at(&mut x, rate, 40_000.0, 0.1, 0.2, db_to_amp(-30.0));
        add_tone_at(
            &mut x,
            rate,
            40_000.0,
            5.0 * win + 0.1,
            0.2,
            db_to_amp(-30.0),
        );
        for k in 0..3 {
            let at = 3.0 * win + 0.1 + 0.12 * f64::from(k);
            add_tone_at(&mut x, rate, 22_000.0, at, 0.005, noise_rms);
        }
        let mut m = LevelMeter::new(rate, win / FRAMES_PER_WINDOW as f64, band);
        m.push(&x);
        let windows: Vec<(f64, f64)> = (0..10)
            .map(|i| (i as f64 * win, (i + 1) as f64 * win))
            .collect();
        let levels = m.finish();
        // Over the whole band the calls are faint.
        let r = window_frames(&levels, windows[3]);
        let mut whole: Vec<f32> = levels.db[r].to_vec();
        let mut all: Vec<f32> = levels.db.clone();
        all.sort_by(f32::total_cmp);
        let floor = percentile(&all, FLOOR_PERCENTILE);
        assert!(Marks::of(&mut whole).over(floor).loudest < 6.0);
        let q = quiet_windows(&levels, &windows);
        let mut want = vec![true; 10];
        for i in [0, 3, 5] {
            want[i] = false;
        }
        assert_eq!(q, want);
    }

    #[test]
    fn one_bat_call_is_not_quiet() {
        // BattyBirdNET: 0.5625 s windows at 256 kHz, its band 1.3 to 80 kHz.
        let rate = 256_000u32;
        let win = 0.5625;
        let band = (1_333.0, 80_000.0);
        let mut rng = SplitMix64::new(9);
        let noise_rms = db_to_amp(-70.0) * 3f32.sqrt();
        let n = (10.0 * win * f64::from(rate)) as usize;
        let mut x: Vec<f32> = (0..n)
            .map(|_| (rng.next_f64() as f32 * 2.0 - 1.0) * noise_rms)
            .collect();
        // Loud passes in windows 0 and 5; one 5 ms call 25 dB over the
        // noise in window 3.
        add_tone_at(&mut x, rate, 40_000.0, 0.1, 0.2, db_to_amp(-30.0));
        add_tone_at(
            &mut x,
            rate,
            40_000.0,
            5.0 * win + 0.1,
            0.2,
            db_to_amp(-30.0),
        );
        add_tone_at(
            &mut x,
            rate,
            40_000.0,
            3.0 * win + 0.2,
            0.005,
            db_to_amp(-45.0),
        );
        let mut m = LevelMeter::new(rate, win / FRAMES_PER_WINDOW as f64, band);
        m.push(&x);
        let windows: Vec<(f64, f64)> = (0..10)
            .map(|i| (i as f64 * win, (i + 1) as f64 * win))
            .collect();
        let q = quiet_windows(&m.finish(), &windows);
        let mut want = vec![true; 10];
        for i in [0, 3, 5] {
            want[i] = false;
        }
        assert_eq!(q, want);
    }

    #[test]
    fn constant_sound_removes_nothing() {
        let mut rng = SplitMix64::new(4);
        let x = noise(30.0, 0.1, &mut rng);
        assert_eq!(run(&x), vec![false; 10]);
        // Noise that rises and falls by 10 dB: still no clear background.
        let mut rng = SplitMix64::new(5);
        let mut x = noise(30.0, 0.01, &mut rng);
        for (i, v) in x.iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            *v *= db_to_amp(5.0 * (t * 0.7).sin());
        }
        assert_eq!(run(&x), vec![false; 10]);
    }

    /// Steady noise with drops like rain: 20 a second, 5 ms each, at a
    /// random pitch from 9 to 14 kHz. A frame with a drop is only about 4 dB
    /// over the noise of the whole band, so over the whole band no window
    /// has sound by the thresholds; in the drops' part it is 7 dB.
    fn rain(rng: &mut SplitMix64) -> Vec<f32> {
        let noise_rms = db_to_amp(-50.0);
        let mut x = noise(30.0, noise_rms, rng);
        let drops = (30.0 * 20.0) as usize;
        for _ in 0..drops {
            let at = rng.next_f64() * 29.9;
            let hz = 9_000.0 + 5_000.0 * rng.next_f64() as f32;
            add_tone_at(&mut x, RATE, hz, at, 0.005, 2.0 * noise_rms);
        }
        x
    }

    #[test]
    fn rain_with_a_loud_event_keeps_its_windows() {
        let mut rng = SplitMix64::new(12);
        let x = rain(&mut rng);
        assert_eq!(run(&x), vec![false; 10]);
        // One loud call, 40 dB over the rain, makes no rain window quiet.
        let mut x = x;
        add_tone(&mut x, 12.5, 1.0, db_to_amp(-10.0));
        assert_eq!(run(&x), vec![false; 10]);
    }

    #[test]
    fn short_file_can_lose_its_quiet_window() {
        // Two windows: calls 13 dB over the noise of the whole band in the
        // first, nothing in the second. Over the whole band no window
        // reaches CONTRAST_DB, but in the calls' own part they do.
        let noise_rms = db_to_amp(-60.0);
        let in_band = share(BAND.0, BAND.1, RATE);
        let mut rng = SplitMix64::new(13);
        let mut x = noise(6.0, noise_rms, &mut rng);
        for k in 0..5 {
            let at = 0.3 + 0.5 * f64::from(k);
            add_tone(&mut x, at, 0.1, noise_rms * in_band * db_to_amp(13.0));
        }
        assert_eq!(run(&x), vec![false, true]);
    }

    #[test]
    fn digital_silence_is_quiet() {
        let mut rng = SplitMix64::new(6);
        let mut x = noise(30.0, 0.1, &mut rng);
        x[6 * RATE as usize..9 * RATE as usize].fill(0.0);
        let mut want = vec![false; 10];
        want[2] = true;
        assert_eq!(run(&x), want);
        // All silence: every window is quiet.
        assert_eq!(run(&vec![0.0; 9 * RATE as usize]), vec![true; 3]);
    }

    #[test]
    fn rumble_below_the_band_does_not_count() {
        // A 40 Hz hum 30 dB over the noise all through: under the band's
        // 250 Hz bottom, so the quiet windows stay quiet.
        let mut rng = SplitMix64::new(7);
        let mut x = base(&mut rng);
        let a = db_to_amp(-30.0) * 2f32.sqrt();
        for (i, v) in x.iter_mut().enumerate() {
            *v += a * (std::f32::consts::TAU * 40.0 * i as f32 / RATE as f32).sin();
        }
        let q = run(&x);
        assert!(q[1] && q[9] && !q[0] && !q[5], "{q:?}");
    }

    #[test]
    fn tone_above_the_band_does_not_count() {
        // A steady 20 kHz tone 30 dB over the noise, over BirdNET's 15 kHz
        // top: the quiet windows stay quiet.
        let mut rng = SplitMix64::new(14);
        let mut x = base(&mut rng);
        add_tone_at(&mut x, RATE, 20_000.0, 0.0, 30.0, db_to_amp(-30.0));
        let q = run(&x);
        assert!(q[1] && q[9] && !q[0] && !q[5], "{q:?}");
        // Bursts of it, 30 dB over the noise, are not heard either.
        let mut rng = SplitMix64::new(14);
        let mut x = base(&mut rng);
        add_tone_at(&mut x, RATE, 20_000.0, 22.0, 0.3, db_to_amp(-30.0));
        assert!(run(&x)[7]);
    }

    #[test]
    fn windows_past_the_end_are_not_quiet() {
        let mut rng = SplitMix64::new(8);
        let mut m = LevelMeter::new(RATE, 0.02, BAND);
        m.push(&base(&mut rng));
        let levels = m.finish();
        assert_eq!(levels.db.len(), 1500);
        assert_eq!(quiet_windows(&levels, &[(30.0, 33.0)]), vec![false]);
        let none = LevelMeter::new(RATE, 0.02, BAND).finish();
        assert!(none.db.is_empty());
        assert_eq!(quiet_windows(&none, &[(0.0, 3.0)]), vec![false]);
    }

    #[test]
    fn model_bands() {
        assert_eq!(model_band(Method::BirdNet), (250.0, 15_000.0));
        assert_eq!(model_band(Method::Perch), (250.0, 16_000.0));
        let (lo, hi) = model_band(Method::Bats);
        assert!((lo - 1_333.3).abs() < 0.1 && (hi - 80_000.0).abs() < 0.1);
    }

    /// Times the level pass on an hour of noise at 48 kHz (BirdNET's band)
    /// and at 384 kHz (BattyBirdNET's band), and prints samples per second:
    /// `cargo test --release -p oversample --lib level_speed -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement"]
    fn level_speed() {
        for (rate, win, band) in [
            (48_000u32, 3.0, (250.0, 15_000.0)),
            (256_000, 0.5625, (1_333.3, 80_000.0)),
            (384_000, 0.5625, (1_333.3, 80_000.0)),
        ] {
            let mut rng = SplitMix64::new(11);
            let piece: Vec<f32> = (0..1 << 20)
                .map(|_| (rng.next_f64() as f32 * 2.0 - 1.0) * 0.01)
                .collect();
            let total = 3600 * rate as usize;
            let start = std::time::Instant::now();
            let mut m = LevelMeter::new(rate, win / FRAMES_PER_WINDOW as f64, band);
            let mut done = 0;
            while done < total {
                let n = piece.len().min(total - done);
                m.push(&piece[..n]);
                done += n;
            }
            let levels = m.finish();
            let secs = start.elapsed().as_secs_f64();
            println!(
                "{rate} Hz: an hour in {secs:.2} s, {:.0} M samples/s, {} frames",
                total as f64 / secs / 1e6,
                levels.db.len()
            );
        }
    }

    /// Prints per-window marks and flags for raw mono f32 little-endian
    /// audio: `QUIET_RAW=path QUIET_RATE=48000 QUIET_WIN=3 QUIET_BAND=250,15000`.
    /// A last window under a third of a window is left out, as a run does.
    /// The marks are the highest of the parts, and the part with the highest
    /// 3rd loudest frame.
    #[test]
    #[ignore = "measurement; needs QUIET_RAW"]
    fn measure_raw_file() {
        let path = std::env::var("QUIET_RAW").unwrap();
        let rate: u32 = std::env::var("QUIET_RATE").unwrap().parse().unwrap();
        let win: f64 = std::env::var("QUIET_WIN").map_or(3.0, |v| v.parse().unwrap());
        let band: Vec<f32> = std::env::var("QUIET_BAND")
            .unwrap_or_else(|_| "250,15000".into())
            .split(',')
            .map(|v| v.parse().unwrap())
            .collect();
        let bytes = std::fs::read(path).unwrap();
        let x: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&b| f32::from_le_bytes(b))
            .collect();
        let mut m = LevelMeter::new(rate, win / FRAMES_PER_WINDOW as f64, (band[0], band[1]));
        for piece in x.chunks(1 << 20) {
            m.push(piece);
        }
        let levels = m.finish();
        let secs = x.len() as f64 / f64::from(rate);
        let mut n = (secs / win).floor() as usize;
        if secs - n as f64 * win >= win / 3.0 || n == 0 {
            n += 1;
        }
        let windows: Vec<(f64, f64)> = (0..n)
            .map(|i| (i as f64 * win, (i + 1) as f64 * win))
            .collect();
        let q = quiet_windows(&levels, &windows);
        let floors = floors(&levels);
        let parts: Vec<String> = levels
            .bands()
            .iter()
            .map(|(a, b)| format!("{:.1}-{:.1}k", a / 1000.0, b / 1000.0))
            .collect();
        println!(
            "parts {}; {} of {n} quiet",
            parts.join(" "),
            q.iter().filter(|&&b| b).count()
        );
        for (i, &w) in windows.iter().enumerate() {
            let m = window_marks(&levels, &floors, w);
            let top = |f: fn(&Marks) -> f32| m.iter().map(f).fold(f32::NEG_INFINITY, f32::max);
            let at = (0..m.len())
                .max_by(|&a, &b| m[a].third.total_cmp(&m[b].third))
                .map_or(String::new(), |b| parts[b].clone());
            println!(
                "{i:3} {:7.2}s  max {:5.1}  3rd {:5.1}  p95 {:5.1}  {at:>11}  {}",
                w.0,
                top(|m| m.loudest),
                top(|m| m.third),
                top(|m| m.p95),
                if q[i] { "QUIET" } else { "" }
            );
        }
    }
}

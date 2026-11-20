//! Quiet windows: the windows of a classifier run with no sound above the
//! recording's background noise. Sound groups (`super::sound_groups`) leave
//! them out, so 2 groups are two kinds of sound, not sound and silence.
//!
//! Method, on the audio filtered to the band the model looks at (the caller
//! passes it: `crate::components::sound_groups` has it per method) and in
//! frames of 1/150 of a window (20 ms for BirdNET's 3 s):
//!
//! - each frame's level is its RMS in dB;
//! - the recording's floor is the 10th percentile of all its frames' levels;
//! - a window has sound when its loudest frame is at least [`CLICK_DB`]
//!   over the floor (a call of one or two frames, such as a bat call), its
//!   3rd loudest frame is at least [`SHORT_CALL_DB`] over the floor (a short
//!   call), or its frame at the 95th percentile is at least [`LONG_CALL_DB`]
//!   over the floor (a faint call that lasts); otherwise it is quiet. So a
//!   call that is both short (under 5% of the window) and faint (under
//!   12 dB) counts as quiet;
//! - when no window's 3rd loudest frame is [`CONTRAST_DB`] over the floor
//!   (steady rain, insects, a shopping mall), no window is quiet: the
//!   windows differ too little to say which are background;
//! - digital silence is always quiet.
//!
//! Tuned on real recordings (Gentoo Penguin XC540767, bat soundscapes
//! XC809053 and XC825336, a jaguar, rain, wind, crickets, a shopping mall,
//! people talking). On the penguin recording only 36 to 39 s is quiet: the
//! other gaps between the loud calls hold faint calls or clicks.

use oversample_core::dsp::filters::Butterworth;

/// Frames per model window.
pub const FRAMES_PER_WINDOW: usize = 150;
/// The loudest frame at least this far over the floor: a call shorter than
/// 3 frames.
pub const CLICK_DB: f32 = 15.0;
/// The 3rd loudest frame at least this far over the floor: a short call.
pub const SHORT_CALL_DB: f32 = 12.0;
/// The 95th percentile frame at least this far over the floor: a faint call
/// that lasts at least 5% of the window (150 ms of 3 s).
pub const LONG_CALL_DB: f32 = 6.0;
/// When no window's 3rd loudest frame is this far over the floor, no window
/// is quiet apart from digital silence.
pub const CONTRAST_DB: f32 = 15.0;
/// The floor's percentile among all frame levels.
const FLOOR_PERCENTILE: f64 = 10.0;
/// Mean square below this (-120 dBFS) is digital silence.
const SILENT: f64 = 1e-12;

/// Frame levels of a recording, for [`quiet_windows`].
#[derive(Clone, Debug, PartialEq)]
pub struct FrameLevels {
    /// Length of a frame in seconds; frame `i` starts at `i * frame_secs`.
    pub frame_secs: f64,
    /// Per frame, its RMS in dB (full scale 0 dB);
    /// `f32::NEG_INFINITY` for digital silence.
    pub db: Vec<f32>,
}

/// Measures [`FrameLevels`] from samples fed in pieces, so a long recording
/// need not be in memory at once.
pub struct LevelMeter {
    highpass: Butterworth,
    lowpass: Option<Butterworth>,
    frame_len: usize,
    frame_secs: f64,
    sum: f64,
    /// Sum of squares of the unfiltered samples, for digital silence (the
    /// filters ring on for a moment after the sound stops).
    raw_sum: f64,
    count: usize,
    db: Vec<f32>,
}

impl LevelMeter {
    /// Frames of `frame_secs` (at least 1 sample) at `sample_rate`, filtered
    /// to `band` (Hz). The low-pass is left out when the band's top is near
    /// or above the recording's Nyquist frequency.
    pub fn new(sample_rate: u32, frame_secs: f64, band: (f32, f32)) -> Self {
        let rate = sample_rate.max(1) as f32;
        let frame_len = ((frame_secs * f64::from(rate)).round() as usize).max(1);
        Self {
            highpass: Butterworth::highpass(2, rate, band.0.min(rate * 0.45)),
            lowpass: (band.1 < rate * 0.45).then(|| Butterworth::lowpass(8, rate, band.1)),
            frame_len,
            frame_secs: frame_len as f64 / f64::from(rate),
            sum: 0.0,
            raw_sum: 0.0,
            count: 0,
            db: Vec::new(),
        }
    }

    /// The next samples of the recording.
    pub fn push(&mut self, samples: &[f32]) {
        for &x in samples {
            let mut y = self.highpass.process(x);
            if let Some(lp) = &mut self.lowpass {
                y = lp.process(y);
            }
            self.sum += f64::from(y) * f64::from(y);
            self.raw_sum += f64::from(x) * f64::from(x);
            self.count += 1;
            if self.count == self.frame_len {
                self.close_frame();
            }
        }
    }

    fn close_frame(&mut self) {
        let ms = self.sum / self.count as f64;
        let silent = self.raw_sum / (self.count as f64) < SILENT;
        self.db.push(if silent || ms < SILENT {
            f32::NEG_INFINITY
        } else {
            (10.0 * ms.log10()) as f32
        });
        self.sum = 0.0;
        self.raw_sum = 0.0;
        self.count = 0;
    }

    /// The levels; a last part-frame of at least half a frame counts.
    pub fn finish(mut self) -> FrameLevels {
        if self.count * 2 >= self.frame_len {
            self.close_frame();
        }
        FrameLevels {
            frame_secs: self.frame_secs,
            db: self.db,
        }
    }
}

/// The value at percentile `p` (0 to 100) of sorted `v`, nearest rank.
fn percentile(sorted: &[f32], p: f64) -> f32 {
    let i = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// A window's loudest, 3rd loudest and 95th percentile frame levels.
#[derive(Clone, Copy, Debug)]
struct Marks {
    loudest: f32,
    third: f32,
    p95: f32,
}

/// The window's [`Marks`], or `None` when it has no frames. A frame is in
/// the window when it starts in it.
fn window_marks(levels: &FrameLevels, (s, e): (f64, f64)) -> Option<Marks> {
    // Frame starts are multiples of `frame_secs`; the small offset keeps a
    // start that is a whole number of frames from rounding up past itself.
    let index =
        |t: f64| ((t / levels.frame_secs - 1e-6).ceil().max(0.0) as usize).min(levels.db.len());
    let mut f: Vec<f32> = levels.db.get(index(s)..index(e))?.to_vec();
    if f.is_empty() {
        return None;
    }
    f.sort_by(|a, b| b.total_cmp(a));
    Some(Marks {
        loudest: f[0],
        third: f[2.min(f.len() - 1)],
        p95: f[((f.len() as f64 * 0.05).ceil() as usize).clamp(1, f.len()) - 1],
    })
}

/// Which of `windows` (start and end, in seconds) are quiet, by the rule in
/// the module notes. A window with no frames (past the end of the levels) is
/// not quiet.
pub fn quiet_windows(levels: &FrameLevels, windows: &[(f64, f64)]) -> Vec<bool> {
    let mut heard: Vec<f32> = levels
        .db
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    heard.sort_by(f32::total_cmp);
    // All digital silence: every window with frames is quiet.
    let floor = if heard.is_empty() {
        f32::NEG_INFINITY
    } else {
        percentile(&heard, FLOOR_PERCENTILE)
    };
    let marks: Vec<Option<Marks>> = windows.iter().map(|&w| window_marks(levels, w)).collect();
    let loudest = marks
        .iter()
        .flatten()
        .map(|m| m.third)
        .fold(f32::NEG_INFINITY, f32::max);
    let contrast = loudest - floor >= CONTRAST_DB;
    marks
        .iter()
        .map(|m| match *m {
            None => false,
            Some(m) if m.loudest == f32::NEG_INFINITY => true,
            Some(m) => {
                contrast
                    && m.loudest - floor < CLICK_DB
                    && m.third - floor < SHORT_CALL_DB
                    && m.p95 - floor < LONG_CALL_DB
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

    /// Add a 3 kHz tone of `rms` from `at` for `secs`.
    fn add_tone(x: &mut [f32], at: f64, secs: f64, rms: f32) {
        let a = rms * 2f32.sqrt();
        let start = (at * f64::from(RATE)) as usize;
        let len = (secs * f64::from(RATE)) as usize;
        for (i, v) in x[start..start + len].iter_mut().enumerate() {
            *v += a * (std::f32::consts::TAU * 3000.0 * i as f32 / RATE as f32).sin();
        }
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
        // The share of the white noise's power inside the 250 Hz to 15 kHz
        // band, so `over` is against the noise the check measures.
        let in_band = ((BAND.1 - BAND.0) / (RATE as f32 / 2.0)).sqrt();
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
        // A 40 ms tick 8 dB over the noise is too short and too faint.
        let mut rng = SplitMix64::new(3);
        let mut x = base(&mut rng);
        add_tone(&mut x, 22.0, 0.04, noise_rms * db_to_amp(8.0));
        assert!(run(&x)[7]);
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
        // A 40 kHz call of `secs` at `db` (dBFS) from `at`.
        let mut call = |at: f64, secs: f64, db: f32| {
            let a = db_to_amp(db) * 2f32.sqrt();
            let start = (at * f64::from(rate)) as usize;
            for i in 0..(secs * f64::from(rate)) as usize {
                let t = i as f32 / rate as f32;
                x[start + i] += a * (std::f32::consts::TAU * 40_000.0 * t).sin();
            }
        };
        // Loud passes in windows 0 and 5; one 5 ms call 25 dB over the
        // noise in window 3.
        call(0.1, 0.2, -30.0);
        call(5.0 * win + 0.1, 0.2, -30.0);
        call(3.0 * win + 0.2, 0.005, -45.0);
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
        // 250 Hz high-pass, so the quiet windows stay quiet.
        let mut rng = SplitMix64::new(7);
        let mut x = base(&mut rng);
        let a = db_to_amp(-30.0) * 2f32.sqrt();
        for (i, v) in x.iter_mut().enumerate() {
            *v += a * (std::f32::consts::TAU * 40.0 * i as f32 / RATE as f32).sin();
        }
        let q = run(&x);
        assert!(q[1] && q[9] && !q[0] && !q[5]);
    }

    #[test]
    fn windows_past_the_end_are_not_quiet() {
        let mut rng = SplitMix64::new(8);
        let mut m = LevelMeter::new(RATE, 0.02, BAND);
        m.push(&base(&mut rng));
        let levels = m.finish();
        assert_eq!(quiet_windows(&levels, &[(30.0, 33.0)]), vec![false]);
        let none = FrameLevels {
            frame_secs: 0.02,
            db: Vec::new(),
        };
        assert_eq!(quiet_windows(&none, &[(0.0, 3.0)]), vec![false]);
    }

    /// Prints per-window marks and flags for raw mono f32 little-endian
    /// audio: `QUIET_RAW=path QUIET_RATE=48000 QUIET_WIN=3 QUIET_BAND=250,15000`.
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
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        let mut m = LevelMeter::new(rate, win / FRAMES_PER_WINDOW as f64, (band[0], band[1]));
        m.push(&x);
        let levels = m.finish();
        let secs = x.len() as f64 / f64::from(rate);
        let n = (secs / win).ceil() as usize;
        let windows: Vec<(f64, f64)> = (0..n)
            .map(|i| (i as f64 * win, (i + 1) as f64 * win))
            .collect();
        let q = quiet_windows(&levels, &windows);
        let mut heard: Vec<f32> = levels
            .db
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect();
        heard.sort_by(f32::total_cmp);
        let floor = percentile(&heard, FLOOR_PERCENTILE);
        println!(
            "floor {floor:.1} dB, {} of {n} quiet",
            q.iter().filter(|&&b| b).count()
        );
        for (i, &w) in windows.iter().enumerate() {
            let m = window_marks(&levels, w).unwrap();
            println!(
                "{i:3} {:6.1}s  max {:5.1}  3rd {:5.1}  p95 {:5.1}  {}",
                w.0,
                m.loudest - floor,
                m.third - floor,
                m.p95 - floor,
                if q[i] { "QUIET" } else { "" }
            );
        }
    }
}

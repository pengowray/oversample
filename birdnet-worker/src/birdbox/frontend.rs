//! The image BirdBox gives the network: a log-floor mel spectrogram of 3 s of
//! 32 kHz audio, coloured with "inferno", 256 x 256 pixels. Made from a stream
//! of audio, one window at a time, so a run can show progress, be cancelled,
//! and never hold the whole recording.
//!
//! Ported from BirdBox v2.1.0 (MIT, BirdNET-Team,
//! <https://github.com/birdnet-team/BirdBox>): `src/inference/utils/pcen_inference.py`
//! (frame grid, rolling floor), `src/inference/onnx_export.py` (flattop window,
//! HTK mel filterbank, colour lookup and column mapping that reproduce the
//! matplotlib PNG) and `docs/how-it-works/spectrogram.md`, by way of
//! `research/birdbox-bench/src/frontend.rs`, which matches BirdBox's CLI. The
//! mel filterbank follows librosa's `filters.mel` (ISC), as BirdBox's own copy
//! does.
//!
//! 1. Silence is added at the end so the last 3 s window on the 1.5 s grid is
//!    complete.
//! 2. Frames on one grid for the whole recording: frame m covers samples
//!    375 m - 250 to 375 m + 1798. The 250 samples before the start are samples
//!    15,750 to 15,999 (a leftover of BirdBox's former padding, which the model
//!    was trained with). Audio is scaled by 2^31 (it cancels out later).
//! 3. Each frame: periodic flattop window, 2048-point FFT, power, 256 HTK mel
//!    bands from 50 Hz to 15 kHz, dB (power floored at 1e-10). A frame whose mel
//!    power adds up to zero is digital silence.
//! 4. Window c (start 1.5 c s) is frames 128 c to 128 c + 251. Its floor per
//!    band is the 20th percentile (linear interpolation, as numpy.percentile) of
//!    that band over the frames in the 60 s before the window's end, leaving out
//!    digital silence (all frames if every frame is silent). Nothing after the
//!    window changes it, so a window is final once its audio has arrived.
//! 5. Pixel: (dB - floor + 3) * 100 / 48, divided by 100, times 256, rounded
//!    down, clamped to 0..255: the index into inferno. Band 255 at the top.
//!    Column j shows frame floor((j + 0.5) * 252 / 256).

use super::inferno::INFERNO;
use realfft::RealFftPlanner;

pub const SR: usize = 32_000;
pub const N_FFT: usize = 2048;
pub const HOP: usize = 375;
pub const N_MELS: usize = 256;
const FMIN: f64 = oversample_ipc::classifier::boxes::FMIN;
const FMAX: f64 = oversample_ipc::classifier::boxes::FMAX;
pub const CLIP_SAMPLES: usize = 96_000;
pub const CLIP_HOP_SAMPLES: usize = 48_000;
pub const CLIP_FRAMES: usize = 252;
pub const CLIP_HOP_FRAMES: usize = 128;
/// Frame m starts at sample m * HOP + FRAME_ORIGIN.
const FRAME_ORIGIN: i64 = 42 * 375 - 16_000; // -250
/// Samples before the start come from here (BirdBox's former 0.5 s pad).
const LEFT_PAD: i64 = 16_000;
const FLOOR_WINDOW_SAMPLES: i64 = 60 * SR as i64;
const FLOOR_PERCENTILE: f64 = 20.0;
const FLOOR_LO_DB: f32 = -3.0;
const FLOOR_HI_DB: f32 = 45.0;
pub const IMAGE: usize = oversample_ipc::classifier::boxes::IMAGE;
const AUDIO_SCALE: f32 = 2_147_483_648.0; // 2^31

fn hz_to_mel(f: f64) -> f64 {
    2595.0 * (1.0 + f / 700.0).log10()
}
fn mel_to_hz(m: f64) -> f64 {
    700.0 * (10f64.powf(m / 2595.0) - 1.0)
}

/// librosa.filters.mel(sr=32000, n_fft=2048, n_mels=256, fmin=50, fmax=15000,
/// htk=True) with Slaney area normalisation, as rows of weights, and the
/// number of FFT bins any row uses.
fn mel_filterbank() -> (Vec<Vec<f64>>, usize) {
    let n_bins = 1 + N_FFT / 2;
    let fft_freqs: Vec<f64> = (0..n_bins)
        .map(|k| k as f64 * (SR as f64 / 2.0) / (n_bins - 1) as f64)
        .collect();
    let (m0, m1) = (hz_to_mel(FMIN), hz_to_mel(FMAX));
    let edges: Vec<f64> = (0..N_MELS + 2)
        .map(|i| mel_to_hz(m0 + (m1 - m0) * i as f64 / (N_MELS + 1) as f64))
        .collect();
    let mut w = vec![vec![0.0; n_bins]; N_MELS];
    for b in 0..N_MELS {
        let enorm = 2.0 / (edges[b + 2] - edges[b]);
        for (k, &f) in fft_freqs.iter().enumerate() {
            let lower = (f - edges[b]) / (edges[b + 1] - edges[b]);
            let upper = (edges[b + 2] - f) / (edges[b + 2] - edges[b + 1]);
            w[b][k] = lower.min(upper).max(0.0) * enorm;
        }
    }
    let used = (0..n_bins)
        .rev()
        .find(|&k| w.iter().any(|r| r[k] != 0.0))
        .unwrap_or(n_bins - 1)
        + 1;
    (w, used)
}

/// scipy.signal.get_window("flattop", 2048, fftbins=True)
fn flattop() -> Vec<f64> {
    let a = [
        0.21557895,
        0.41663158,
        0.277263158,
        0.083578947,
        0.006947368,
    ];
    (0..N_FFT)
        .map(|i| {
            let x = -std::f64::consts::PI + 2.0 * std::f64::consts::PI * i as f64 / N_FFT as f64;
            a.iter()
                .enumerate()
                .map(|(k, c)| c * (k as f64 * x).cos())
                .sum()
        })
        .collect()
}

/// The fixed parts: the window, the mel bands and the FFT.
struct Frontend {
    window: Vec<f64>,
    /// Per band: first bin with a non-zero weight and the weights from there on.
    bands: Vec<(usize, Vec<f64>)>,
    fft: std::sync::Arc<dyn realfft::RealToComplex<f64>>,
}

impl Frontend {
    fn new() -> Self {
        let (w, used) = mel_filterbank();
        let bands = w
            .into_iter()
            .map(|row| {
                let first = row[..used].iter().position(|&v| v != 0.0).unwrap_or(0);
                let last = row[..used].iter().rposition(|&v| v != 0.0).unwrap_or(0);
                (first, row[first..=last].to_vec())
            })
            .collect();
        let fft = RealFftPlanner::<f64>::new().plan_fft_forward(N_FFT);
        Frontend {
            window: flattop(),
            bands,
            fft,
        }
    }
}

/// Samples after end padding: the end of the first window on the 1.5 s grid
/// that still holds the last sample (at least one window).
pub fn padded_len(n: usize) -> usize {
    let extra = n.saturating_sub(CLIP_SAMPLES);
    CLIP_SAMPLES + extra.div_ceil(CLIP_HOP_SAMPLES) * CLIP_HOP_SAMPLES
}

/// Windows for `n` samples.
pub fn clip_count(n: usize) -> usize {
    (padded_len(n) - CLIP_SAMPLES) / CLIP_HOP_SAMPLES + 1
}

/// First sample of frame `m`.
fn frame_start(m: usize) -> i64 {
    m as i64 * HOP as i64 + FRAME_ORIGIN
}

/// Window c's network input, 3 x 256 x 256 (R, G, B planes, 0 to 1).
pub struct Clip {
    pub index: usize,
    pub image: Vec<f32>,
}

/// Turns a stream of 32 kHz mono audio into BirdBox's window images. Keeps
/// the audio of the frames not made yet and the dB frames of the last 60 s.
pub struct BoxStream {
    fe: Frontend,
    /// Samples from `audio_at` on (not scaled).
    audio: Vec<f32>,
    audio_at: u64,
    received: u64,
    /// The length of the audio, once it has all arrived.
    content: Option<u64>,
    /// dB frames from `frames_at` on, N_MELS values each, and whether each is
    /// sound (not digital silence).
    db: Vec<f32>,
    sound: Vec<bool>,
    frames_at: usize,
    next_frame: usize,
    next_clip: usize,
    buf: Vec<f64>,
    spec: Vec<realfft::num_complex::Complex<f64>>,
    scratch: Vec<realfft::num_complex::Complex<f64>>,
    power: Vec<f64>,
}

impl Default for BoxStream {
    fn default() -> Self {
        Self::new()
    }
}

impl BoxStream {
    pub fn new() -> Self {
        let fe = Frontend::new();
        let buf = fe.fft.make_input_vec();
        let spec = fe.fft.make_output_vec();
        let scratch = fe.fft.make_scratch_vec();
        BoxStream {
            fe,
            audio: Vec::new(),
            audio_at: 0,
            received: 0,
            content: None,
            db: Vec::new(),
            sound: Vec::new(),
            frames_at: 0,
            next_frame: 0,
            next_clip: 0,
            buf,
            spec,
            scratch,
            power: vec![0.0; N_FFT / 2 + 1],
        }
    }

    /// Samples received so far.
    pub fn samples(&self) -> u64 {
        self.received
    }

    /// Windows handed out so far.
    pub fn clips(&self) -> usize {
        self.next_clip
    }

    /// Feed 32 kHz mono samples (-1 to 1); returns the windows they complete.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Clip> {
        self.audio.extend_from_slice(samples);
        self.received += samples.len() as u64;
        self.make_frames();
        self.take_clips()
    }

    /// End of the audio: returns the remaining windows, the last ones padded
    /// with silence.
    pub fn finish(&mut self) -> Vec<Clip> {
        self.content = Some(self.received);
        self.make_frames();
        self.take_clips()
    }

    /// Frames in the whole recording, once its length is known.
    fn total_frames(&self) -> Option<usize> {
        self.content
            .map(|n| CLIP_HOP_FRAMES * (clip_count(n as usize) - 1) + CLIP_FRAMES)
    }

    /// Sample `s` of the recording, scaled; samples before the start come
    /// from BirdBox's former padding and those past the end are silence.
    fn sample(&self, s: i64) -> f32 {
        let s = if s < 0 { LEFT_PAD + s } else { s };
        let s = s as u64;
        if s < self.audio_at {
            return 0.0;
        }
        match self.audio.get((s - self.audio_at) as usize) {
            Some(&v) => v * AUDIO_SCALE,
            None => 0.0,
        }
    }

    /// True when frame `m` can be made: all its samples (and for frame 0,
    /// the samples it takes from 15,750 on) have arrived, or the audio has
    /// ended.
    fn frame_ready(&self, m: usize) -> bool {
        if let Some(total) = self.total_frames() {
            return m < total;
        }
        let end = frame_start(m) + N_FFT as i64;
        let need = if m == 0 { end.max(LEFT_PAD) } else { end };
        self.received as i64 >= need
    }

    fn make_frames(&mut self) {
        while self.frame_ready(self.next_frame) {
            let m = self.next_frame;
            let start = frame_start(m);
            for i in 0..N_FFT {
                self.buf[i] = self.sample(start + i as i64) as f64 * self.fe.window[i];
            }
            self.fe
                .fft
                .process_with_scratch(&mut self.buf, &mut self.spec, &mut self.scratch)
                .expect("FFT buffers are sized by the planner");
            for (p, c) in self.power.iter_mut().zip(self.spec.iter()) {
                *p = c.re * c.re + c.im * c.im;
            }
            let mut total = 0.0;
            for (first, w) in &self.fe.bands {
                let mel: f64 = w
                    .iter()
                    .zip(&self.power[*first..])
                    .map(|(a, p)| a * p)
                    .sum();
                total += mel;
                self.db.push((10.0 * mel.max(1e-10).log10()) as f32);
            }
            self.sound.push(total > 0.0);
            self.next_frame += 1;
            // Keep the audio from the next frame's start on; frame 0 is made
            // first, so the samples it takes from 15,750 on are still here.
            let keep = frame_start(self.next_frame).max(0) as u64;
            if keep > self.audio_at {
                let drop = ((keep - self.audio_at) as usize).min(self.audio.len());
                self.audio.drain(..drop);
                self.audio_at += drop as u64;
            }
        }
    }

    /// The window's frames: `lo..hi`, the frames inside the 60 s before the
    /// window's end, and the window's own last frame + 1.
    fn floor_frames(c: usize) -> (usize, usize) {
        let end = (c * CLIP_HOP_SAMPLES + CLIP_SAMPLES) as i64;
        let hop = HOP as i64;
        let hi = c * CLIP_HOP_FRAMES + CLIP_FRAMES;
        let lo = (-(-(end - FLOOR_WINDOW_SAMPLES - FRAME_ORIGIN)).div_euclid(hop)).max(0) as usize;
        (lo, hi)
    }

    fn take_clips(&mut self) -> Vec<Clip> {
        let mut out = Vec::new();
        loop {
            let c = self.next_clip;
            if let Some(n) = self.content {
                if c >= clip_count(n as usize) {
                    break;
                }
            }
            let (lo, hi) = Self::floor_frames(c);
            if self.next_frame < hi {
                break;
            }
            let floor = self.floor(lo, hi);
            out.push(Clip {
                index: c,
                image: self.image(c, &floor),
            });
            self.next_clip += 1;
            // Frames before the next window's floor are no longer needed.
            let (next_lo, _) = Self::floor_frames(self.next_clip);
            let next_lo = next_lo.min(c * CLIP_HOP_FRAMES + CLIP_HOP_FRAMES);
            if next_lo > self.frames_at {
                let drop = next_lo - self.frames_at;
                self.db.drain(..drop * N_MELS);
                self.sound.drain(..drop);
                self.frames_at = next_lo;
            }
        }
        out
    }

    fn db_at(&self, m: usize, b: usize) -> f32 {
        self.db[(m - self.frames_at) * N_MELS + b]
    }

    /// Step 4: the floor per band, in dB, over frames `lo..hi`.
    fn floor(&self, lo: usize, hi: usize) -> Vec<f32> {
        let mut frames: Vec<usize> = (lo..hi)
            .filter(|&m| self.sound[m - self.frames_at])
            .collect();
        if frames.is_empty() {
            frames = (lo..hi).collect();
        }
        let count = frames.len();
        let position = (count - 1) as f64 * FLOOR_PERCENTILE / 100.0;
        let below = position.floor() as usize;
        let fraction = position - below as f64;
        let mut vals = vec![0f32; count];
        (0..N_MELS)
            .map(|b| {
                for (v, &m) in vals.iter_mut().zip(&frames) {
                    *v = self.db_at(m, b);
                }
                let (_, low, rest) = vals.select_nth_unstable_by(below, |a, b| a.total_cmp(b));
                let low = *low as f64;
                let high = if below + 1 < count {
                    rest.iter().copied().fold(f32::INFINITY, f32::min) as f64
                } else {
                    low
                };
                // numpy's linear interpolation (_lerp)
                let v = if fraction >= 0.5 {
                    high - (high - low) * (1.0 - fraction)
                } else {
                    low + (high - low) * fraction
                };
                v as f32
            })
            .collect()
    }

    /// Step 5: window c's image.
    fn image(&self, c: usize, floor: &[f32]) -> Vec<f32> {
        let scale: f32 = 100.0 / (FLOOR_HI_DB - FLOOR_LO_DB);
        let first = c * CLIP_HOP_FRAMES;
        let plane = IMAGE * IMAGE;
        let mut img = vec![0f32; 3 * plane];
        for j in 0..IMAGE {
            let f = first + ((j as f64 + 0.5) * CLIP_FRAMES as f64 / IMAGE as f64).floor() as usize;
            for (b, &fl) in floor.iter().enumerate() {
                // The order of operations of BirdBox's numpy and matplotlib
                // code, in float32.
                let x = (self.db_at(f, b) - FLOOR_LO_DB) * scale - fl * scale;
                let level = ((x / 100.0) * 256.0).floor().clamp(0.0, 255.0) as usize;
                let row = IMAGE - 1 - b;
                let rgb = INFERNO[level];
                for (ch, &v) in rgb.iter().enumerate() {
                    img[ch * plane + row * IMAGE + j] = v as f32 / 255.0;
                }
            }
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bench's whole-recording version (`research/birdbox-bench`), which
    /// matches BirdBox's CLI image for image: every frame, then each window's
    /// floor and image.
    fn whole_file_images(audio: &[f32]) -> Vec<Vec<f32>> {
        let fe = Frontend::new();
        let content = audio.len();
        let n_clips = clip_count(content);
        let n_frames = CLIP_HOP_FRAMES * (n_clips - 1) + CLIP_FRAMES;
        let sample = |s: i64| -> f32 {
            let s = if s < 0 { LEFT_PAD + s } else { s } as usize;
            if s < content {
                audio[s] * AUDIO_SCALE
            } else {
                0.0
            }
        };
        let mut db = vec![0f32; n_frames * N_MELS];
        let mut sound = vec![false; n_frames];
        let mut buf = fe.fft.make_input_vec();
        let mut spec = fe.fft.make_output_vec();
        let mut power = vec![0f64; N_FFT / 2 + 1];
        for m in 0..n_frames {
            let start = frame_start(m);
            for (i, b) in buf.iter_mut().enumerate() {
                *b = sample(start + i as i64) as f64 * fe.window[i];
            }
            fe.fft.process(&mut buf, &mut spec).unwrap();
            for (p, c) in power.iter_mut().zip(spec.iter()) {
                *p = c.re * c.re + c.im * c.im;
            }
            let mut total = 0.0;
            for (b, (first, w)) in fe.bands.iter().enumerate() {
                let mel: f64 = w.iter().zip(&power[*first..]).map(|(a, p)| a * p).sum();
                total += mel;
                db[m * N_MELS + b] = (10.0 * mel.max(1e-10).log10()) as f32;
            }
            sound[m] = total > 0.0;
        }
        // A stream that holds everything, for its floor and image code.
        let s = BoxStream {
            db,
            sound,
            ..BoxStream::new()
        };
        (0..n_clips)
            .map(|c| {
                let (lo, hi) = BoxStream::floor_frames(c);
                let hi = hi.min(n_frames);
                s.image(c, &s.floor(lo, hi))
            })
            .collect()
    }

    fn test_audio(n: usize) -> Vec<f32> {
        let mut seed = 12345u32;
        (0..n)
            .map(|i| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed as f32 / u32::MAX as f32 - 0.5) * 0.02;
                let t = i as f32 / SR as f32;
                // A tone that comes and goes, and a stretch of digital silence.
                let tone = if (t % 7.0) < 2.0 {
                    0.3 * (2.0 * std::f32::consts::PI * 3000.0 * t).sin()
                } else {
                    0.0
                };
                if (20.0..24.0).contains(&t) {
                    0.0
                } else {
                    noise + tone
                }
            })
            .collect()
    }

    #[test]
    fn the_stream_gives_the_whole_file_images() {
        // 70 s: the floor's 60 s history fills and slides. Not a whole number
        // of windows, so the last ones are padded.
        let audio = test_audio(70 * SR + 12_345);
        let want = whole_file_images(&audio);
        for piece in [7_777usize, 48_000, 200_003] {
            let mut s = BoxStream::new();
            let mut got = Vec::new();
            for p in audio.chunks(piece) {
                got.extend(s.push(p));
            }
            got.extend(s.finish());
            assert_eq!(got.len(), want.len(), "pieces of {piece}");
            for (k, (g, w)) in got.iter().zip(&want).enumerate() {
                assert_eq!(g.index, k);
                assert!(g.image == *w, "window {k}, pieces of {piece}");
            }
            // It keeps about 60 s of frames, not the whole recording.
            assert!(s.sound.len() <= 5_200, "{} frames kept", s.sound.len());
        }
    }

    #[test]
    fn short_and_silent_recordings_give_one_window() {
        for n in [0usize, 100, 15_000, 95_999] {
            let audio = vec![0.0; n];
            let want = whole_file_images(&audio);
            let mut s = BoxStream::new();
            let mut got = s.push(&audio);
            got.extend(s.finish());
            assert_eq!(got.len(), 1, "{n} samples");
            assert!(got[0].image == want[0], "{n} samples");
        }
        assert_eq!(clip_count(96_001), 2);
    }
}

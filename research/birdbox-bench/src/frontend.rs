//! The image BirdBox gives the network: a log-floor mel spectrogram of 3 s of 32 kHz
//! audio, coloured with "inferno", 256 x 256 pixels.
//!
//! Ported from BirdBox v2.1.0 (MIT, BirdNET-Team, https://github.com/birdnet-team/BirdBox):
//! `src/inference/utils/pcen_inference.py` (frame grid, rolling floor),
//! `src/inference/onnx_export.py` (flattop window, HTK mel filterbank, colour lookup and
//! column mapping that reproduce the matplotlib PNG) and
//! `docs/how-it-works/spectrogram.md`. The mel filterbank follows librosa's
//! `filters.mel` (ISC), as BirdBox's own copy does.
//!
//! Steps, per file:
//! 1. Pad the end with silence so the last 3 s window on the 1.5 s grid is complete.
//! 2. Frames on one grid for the whole file: frame m covers samples 375 m - 250 to
//!    375 m - 250 + 2048. The 250 samples before the file start are samples 15,750 to
//!    15,999 of the file (a leftover of BirdBox's former PCEN padding, kept because the
//!    model was trained on it). Audio is scaled by 2^31 first (cancels out later).
//! 3. Each frame: periodic flattop window, 2048-point FFT, power, 256 HTK mel bands from
//!    50 Hz to 15 kHz, dB (power floored at 1e-10). A frame whose mel power sums to zero
//!    is "digital silence".
//! 4. Window c (start 1.5 c s) is frames 128 c to 128 c + 251. Its floor per band is the
//!    20th percentile (linear interpolation, as numpy.percentile) of that band's dB over
//!    the frames that lie inside the 60 s before the window's end, leaving out digital
//!    silence (all frames if every frame is silent).
//! 5. Pixel value: (dB - floor + 3) * 100 / 48, divided by 100, times 256, rounded down,
//!    clamped to 0..255: the inferno colour index. Rows: mel band 255 at the top.
//!    Columns: pixel j shows frame floor((j + 0.5) * 252 / 256).

use crate::inferno::INFERNO;
use realfft::RealFftPlanner;

pub const SR: usize = 32_000;
pub const N_FFT: usize = 2048;
pub const HOP: usize = 375;
pub const N_MELS: usize = 256;
pub const FMIN: f64 = 50.0;
pub const FMAX: f64 = 15_000.0;
pub const CLIP_SAMPLES: usize = 96_000;
pub const CLIP_HOP_SAMPLES: usize = 48_000;
pub const CLIP_FRAMES: usize = 252;
pub const CLIP_HOP_FRAMES: usize = 128;
/// Frame m starts at sample m * HOP + FRAME_ORIGIN.
pub const FRAME_ORIGIN: i64 = 42 * 375 - 16_000; // -250
/// Samples before the file start come from here (BirdBox's 0.5 s left pad).
pub const LEFT_PAD: usize = 16_000;
pub const FLOOR_WINDOW_SAMPLES: i64 = 60 * SR as i64;
pub const FLOOR_PERCENTILE: f64 = 20.0;
pub const FLOOR_LO_DB: f32 = -3.0;
pub const FLOOR_HI_DB: f32 = 45.0;
pub const IMAGE: usize = 256;
const AUDIO_SCALE: f32 = 2_147_483_648.0; // 2^31

fn hz_to_mel(f: f64) -> f64 {
    2595.0 * (1.0 + f / 700.0).log10()
}
fn mel_to_hz(m: f64) -> f64 {
    700.0 * (10f64.powf(m / 2595.0) - 1.0)
}

/// librosa.filters.mel(sr=32000, n_fft=2048, n_mels=256, fmin=50, fmax=15000, htk=True)
/// with Slaney area normalisation, as rows of (first bin, weights).
fn mel_filterbank() -> (Vec<Vec<f64>>, usize) {
    let n_bins = 1 + N_FFT / 2;
    let fft_freqs: Vec<f64> = (0..n_bins).map(|k| k as f64 * (SR as f64 / 2.0) / (n_bins - 1) as f64).collect();
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
    let used = (0..n_bins).rev().find(|&k| w.iter().any(|r| r[k] != 0.0)).unwrap() + 1;
    (w, used)
}

/// scipy.signal.get_window("flattop", 2048, fftbins=True)
fn flattop() -> Vec<f64> {
    let a = [0.21557895, 0.41663158, 0.277263158, 0.083578947, 0.006947368];
    (0..N_FFT)
        .map(|i| {
            let x = -std::f64::consts::PI + 2.0 * std::f64::consts::PI * i as f64 / N_FFT as f64;
            a.iter().enumerate().map(|(k, c)| c * (k as f64 * x).cos()).sum()
        })
        .collect()
}

pub struct Frontend {
    window: Vec<f64>,
    /// Per band: first bin with a non-zero weight and the weights from there on.
    bands: Vec<(usize, Vec<f64>)>,
    fft: std::sync::Arc<dyn realfft::RealToComplex<f64>>,
}

impl Default for Frontend {
    fn default() -> Self {
        Self::new()
    }
}

impl Frontend {
    pub fn new() -> Self {
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
        Frontend { window: flattop(), bands, fft }
    }
}

/// A file's padded audio and its dB mel frames.
pub struct FileFrames {
    /// Samples before padding.
    pub content_samples: usize,
    /// Samples after padding (the end of the last window).
    pub padded_samples: usize,
    pub n_clips: usize,
    /// frames x N_MELS, dB.
    pub db: Vec<f32>,
    pub sound: Vec<bool>,
}

impl FileFrames {
    pub fn n_frames(&self) -> usize {
        self.sound.len()
    }
    pub fn content_end_s(&self) -> f64 {
        self.content_samples as f64 / SR as f64
    }
    pub fn file_end_s(&self) -> f64 {
        self.padded_samples as f64 / SR as f64
    }
    pub fn clip_start_s(c: usize) -> f64 {
        (c * CLIP_HOP_SAMPLES) as f64 / SR as f64
    }
}

/// Samples after end padding: the end of the first window on the 1.5 s grid that still
/// holds the last sample (at least one window).
pub fn padded_len(n: usize) -> usize {
    let extra = n.saturating_sub(CLIP_SAMPLES);
    CLIP_SAMPLES + extra.div_ceil(CLIP_HOP_SAMPLES) * CLIP_HOP_SAMPLES
}

/// Steps 1 to 3 for a whole file (32 kHz mono, -1..1).
pub fn file_frames(fe: &Frontend, audio: &[f32]) -> FileFrames {
    let content = audio.len();
    let padded = padded_len(content);
    let n_clips = (padded - CLIP_SAMPLES) / CLIP_HOP_SAMPLES + 1;
    let n_frames = CLIP_HOP_FRAMES * (n_clips - 1) + CLIP_FRAMES;
    let sample = |s: i64| -> f32 {
        let s = if s < 0 { LEFT_PAD as i64 + s } else { s };
        let s = s as usize;
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
    let mut scratch = fe.fft.make_scratch_vec();
    let mut power = vec![0f64; N_FFT / 2 + 1];
    for m in 0..n_frames {
        let start = m as i64 * HOP as i64 + FRAME_ORIGIN;
        for (i, b) in buf.iter_mut().enumerate() {
            *b = sample(start + i as i64) as f64 * fe.window[i];
        }
        fe.fft.process_with_scratch(&mut buf, &mut spec, &mut scratch).unwrap();
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
    FileFrames { content_samples: content, padded_samples: padded, n_clips, db, sound }
}

/// Step 4: the floor of window c per band, in dB.
pub fn rolling_floor(ff: &FileFrames, c: usize) -> Vec<f32> {
    let end = (c * CLIP_HOP_SAMPLES + CLIP_SAMPLES) as i64;
    let hop = HOP as i64;
    let hi = ((end - N_FFT as i64 - FRAME_ORIGIN).div_euclid(hop) + 1).min(ff.n_frames() as i64) as usize;
    let lo = (-(-(end - FLOOR_WINDOW_SAMPLES - FRAME_ORIGIN)).div_euclid(hop)).max(0) as usize;
    let mut frames: Vec<usize> = (lo..hi).filter(|&m| ff.sound[m]).collect();
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
                *v = ff.db[m * N_MELS + b];
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

/// Step 5: window c's network input, 3 x 256 x 256 (R, G, B planes, 0..1).
pub fn clip_image(ff: &FileFrames, c: usize, floor: &[f32]) -> Vec<f32> {
    let scale: f32 = 100.0 / (FLOOR_HI_DB - FLOOR_LO_DB);
    let first = c * CLIP_HOP_FRAMES;
    let mut img = vec![0f32; 3 * IMAGE * IMAGE];
    let plane = IMAGE * IMAGE;
    for j in 0..IMAGE {
        let f = first + ((j as f64 + 0.5) * CLIP_FRAMES as f64 / IMAGE as f64).floor() as usize;
        for b in 0..N_MELS {
            // The order of operations of BirdBox's numpy and matplotlib code, in float32
            let x = (ff.db[f * N_MELS + b] - FLOOR_LO_DB) * scale - floor[b] * scale;
            let level = ((x / 100.0) * 256.0).floor().clamp(0.0, 255.0) as usize;
            let row = IMAGE - 1 - b;
            let rgb = INFERNO[level];
            for ch in 0..3 {
                img[ch * plane + row * IMAGE + j] = rgb[ch] as f32 / 255.0;
            }
        }
    }
    img
}

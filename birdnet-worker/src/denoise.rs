//! The learned bat-call denoiser: a small convolution network that gives a
//! gain for every bin of a 512-point STFT.
//!
//! The network and its training are in the `batchi-noisier2noise` project
//! (`n2n.py`). This file repeats that project's `denoise` step for step, so a
//! recording comes out the same here as there: `torch.stft` framing (the audio
//! reflected by half a window at each end, a periodic Hann window), the same
//! noise scale, features, blocks and gain floor.
//!
//! The audio is used sample for sample at whatever rate it has. The network
//! was trained that way on recordings from 250 to 500 kHz.

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::collections::HashMap;
use std::sync::Arc;
use tract_onnx::prelude::*;

/// STFT window length in samples.
pub const WINDOW: usize = 512;
/// Samples between STFT frames.
pub const HOP: usize = 128;
/// Bins per frame.
pub const BINS: usize = WINDOW / 2 + 1;
/// Frames the network gives gains for in one step.
pub const BLOCK: usize = 1024;
/// Extra frames it sees on each side of a block.
pub const CONTEXT: usize = 64;
/// Share of the input kept in every bin. Without it the network draws
/// call-like streaks out of noise.
pub const GAIN_FLOOR: f32 = 0.1;
/// Shortest recording the denoiser takes, in samples.
pub const MIN_SAMPLES: usize = WINDOW * 4;

/// Bins below this (15 kHz at 384 kHz) are left out of the noise scale.
const SCALE_LOW_BIN: usize = 20;
/// Most values the noise scale's median is taken over.
const SCALE_MAX_VALUES: usize = 2_000_000;
/// Median |STFT| of unit-variance white noise under this window: a Rayleigh
/// distribution with sigma^2 = sum(w^2) / 2 = 96.
const UNIT_NOISE_MEDIAN: f32 = 11.536_31;
/// Added to magnitudes before the logarithm.
const LOG_OFFSET: f32 = 0.1;

/// Gives the network's mask logits for a block of features.
pub trait MaskModel {
    /// `feat` holds `BINS` rows of `frames` values. The answer has the same
    /// shape.
    fn logits(&mut self, feat: &[f32], frames: usize) -> Result<Vec<f32>, String>;
}

/// The network in tract. tract wants a fixed input shape, so there is one
/// plan for each block length met (a recording has at most three).
pub struct TractMask {
    bytes: Vec<u8>,
    plans: HashMap<usize, Arc<TypedRunnableModel>>,
}

impl TractMask {
    pub fn load(bytes: Vec<u8>) -> TractResult<TractMask> {
        let mut model = TractMask {
            bytes,
            plans: HashMap::new(),
        };
        // Build the commonest shape now, so a bad file fails at load.
        model.plan(BLOCK + 2 * CONTEXT)?;
        Ok(model)
    }

    fn plan(&mut self, frames: usize) -> TractResult<Arc<TypedRunnableModel>> {
        if let Some(plan) = self.plans.get(&frames) {
            return Ok(plan.clone());
        }
        let plan = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .with_ignore_output_shapes(true)
            .model_for_read(&mut std::io::Cursor::new(&self.bytes))?
            .with_input_fact(0, f32::fact([1, 1, BINS, frames]).into())?
            .into_optimized()?
            .into_runnable()?;
        self.plans.insert(frames, plan.clone());
        Ok(plan)
    }
}

impl MaskModel for TractMask {
    fn logits(&mut self, feat: &[f32], frames: usize) -> Result<Vec<f32>, String> {
        let run = |model: &mut TractMask| -> TractResult<Vec<f32>> {
            let plan = model.plan(frames)?;
            let input = Tensor::from_shape(&[1, 1, BINS, frames], feat)?;
            let out = plan.run(tvec!(input.into()))?;
            Ok(out[0]
                .to_plain_array_view::<f32>()?
                .iter()
                .copied()
                .collect())
        };
        let logits = run(self).map_err(|e| e.to_string())?;
        if logits.len() != feat.len() {
            return Err(format!(
                "the model gave {} values for {} inputs",
                logits.len(),
                feat.len()
            ));
        }
        Ok(logits)
    }
}

/// `torch.hann_window(WINDOW)`: periodic, not the symmetric window the rest of
/// Oversample uses.
fn periodic_hann() -> Vec<f32> {
    (0..WINDOW)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WINDOW as f32).cos())
        .collect()
}

/// The lower median, as `torch.median` gives it. Reorders `values`.
fn lower_median(values: &mut [f32]) -> f32 {
    let mid = (values.len() - 1) / 2;
    *values.select_nth_unstable_by(mid, f32::total_cmp).1
}

/// One recording being denoised, a block per [`Denoiser::step`].
pub struct Denoiser {
    /// The audio reflected by half a window at each end, divided by `scale`.
    padded: Vec<f32>,
    /// Samples in the recording.
    len: usize,
    scale: f32,
    frames: usize,
    /// First frame of the next block.
    next: usize,
    out: Vec<f32>,
    window_sum: Vec<f32>,
    window: Vec<f32>,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
}

impl Denoiser {
    /// Takes mono samples at the recording's own rate.
    pub fn new(samples: &[f32]) -> Result<Denoiser, String> {
        if samples.len() < MIN_SAMPLES {
            return Err(format!(
                "the recording has {} samples; the denoiser needs {MIN_SAMPLES}",
                samples.len()
            ));
        }
        let len = samples.len();
        let half = WINDOW / 2;
        let mut padded = Vec::with_capacity(len + WINDOW);
        padded.extend((1..=half).rev().map(|i| samples[i]));
        padded.extend_from_slice(samples);
        padded.extend((1..=half).map(|i| samples[len - 1 - i]));

        let mut planner = RealFftPlanner::<f32>::new();
        let mut denoiser = Denoiser {
            out: vec![0.0; padded.len()],
            window_sum: vec![0.0; padded.len()],
            padded,
            len,
            scale: 1.0,
            frames: 1 + len / HOP,
            next: 0,
            window: periodic_hann(),
            forward: planner.plan_fft_forward(WINDOW),
            inverse: planner.plan_fft_inverse(WINDOW),
        };
        let scale = denoiser.noise_scale();
        if !(scale.is_finite() && scale > 0.0) {
            return Err("the recording is silent".to_string());
        }
        denoiser.scale = scale;
        for s in &mut denoiser.padded {
            *s /= scale;
        }
        Ok(denoiser)
    }

    /// Steps the recording takes.
    pub fn blocks(&self) -> usize {
        self.frames.div_ceil(BLOCK)
    }

    /// Steps done so far.
    pub fn blocks_done(&self) -> usize {
        self.next.div_ceil(BLOCK)
    }

    /// STFT of frames `lo..hi`, frame after frame, `BINS` values each.
    fn stft(&self, lo: usize, hi: usize) -> Vec<Complex<f32>> {
        let mut input = self.forward.make_input_vec();
        let mut spectrum = self.forward.make_output_vec();
        let mut out = Vec::with_capacity((hi - lo) * BINS);
        for t in lo..hi {
            let start = t * HOP;
            for (i, x) in input.iter_mut().enumerate() {
                *x = self.padded[start + i] * self.window[i];
            }
            self.forward
                .process(&mut input, &mut spectrum)
                .expect("FFT sizes are fixed");
            out.extend_from_slice(&spectrum);
        }
        out
    }

    /// The factor that brings the recording's noise to about unit variance:
    /// the median magnitude over every frame and every bin from
    /// `SCALE_LOW_BIN` up, against that of unit white noise. It needs no
    /// call-free stretch. Long recordings use every k-th frame.
    fn noise_scale(&self) -> f32 {
        let per_frame = BINS - SCALE_LOW_BIN;
        let stride = (self.frames * per_frame).div_ceil(SCALE_MAX_VALUES).max(1);
        let mut power = Vec::with_capacity(self.frames / stride * per_frame + per_frame);
        for t in (0..self.frames).step_by(stride) {
            let frame = self.stft(t, t + 1);
            power.extend(frame[SCALE_LOW_BIN..].iter().map(|c| c.norm_sqr()));
        }
        lower_median(&mut power).sqrt() / UNIT_NOISE_MEDIAN
    }

    /// Denoises the next block. False once the recording is finished.
    pub fn step(&mut self, model: &mut dyn MaskModel) -> Result<bool, String> {
        if self.next >= self.frames {
            return Ok(false);
        }
        let start = self.next;
        let end = (start + BLOCK).min(self.frames);
        let lo = start.saturating_sub(CONTEXT);
        let hi = (end + CONTEXT).min(self.frames);
        let frames = hi - lo;
        let spectrum = self.stft(lo, hi);

        // Features: log magnitude, less each bin's median over these frames.
        let mut feat = vec![0.0f32; BINS * frames];
        let mut column = vec![0.0f32; frames];
        for bin in 0..BINS {
            let row = &mut feat[bin * frames..(bin + 1) * frames];
            for (t, value) in row.iter_mut().enumerate() {
                *value = (spectrum[t * BINS + bin].norm() + LOG_OFFSET).ln();
            }
            column.copy_from_slice(row);
            let median = lower_median(&mut column);
            for value in row.iter_mut() {
                *value -= median;
            }
        }
        let logits = model.logits(&feat, frames)?;

        // Gain 2 * sigmoid(logit) - 1, floored, on this block's own frames.
        let mut bins = self.inverse.make_input_vec();
        let mut time = self.inverse.make_output_vec();
        for t in start..end {
            let local = t - lo;
            for (bin, value) in bins.iter_mut().enumerate() {
                let logit = logits[bin * frames + local];
                let gain = (2.0 / (1.0 + (-logit).exp()) - 1.0).max(GAIN_FLOOR);
                *value = spectrum[local * BINS + bin] * gain;
            }
            // realfft wants the imaginary parts at DC and Nyquist to be zero.
            bins[0].im = 0.0;
            bins[BINS - 1].im = 0.0;
            self.inverse
                .process(&mut bins, &mut time)
                .expect("FFT sizes are fixed");
            let at = t * HOP;
            for i in 0..WINDOW {
                self.out[at + i] += time[i] / WINDOW as f32 * self.window[i];
                self.window_sum[at + i] += self.window[i] * self.window[i];
            }
        }
        self.next = end;
        Ok(self.next < self.frames)
    }

    /// The denoised recording, the same length as what went in.
    pub fn finish(self) -> Vec<f32> {
        let half = WINDOW / 2;
        (half..half + self.len)
            .map(|i| {
                let sum = self.window_sum[i];
                if sum > 1e-8 {
                    self.out[i] / sum * self.scale
                } else {
                    0.0
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model that keeps everything: a large logit gives a gain of 1.
    struct KeepAll;
    impl MaskModel for KeepAll {
        fn logits(&mut self, feat: &[f32], _frames: usize) -> Result<Vec<f32>, String> {
            Ok(vec![30.0; feat.len()])
        }
    }

    /// A model that rejects everything: the gain floor is what is left.
    struct KeepNone;
    impl MaskModel for KeepNone {
        fn logits(&mut self, feat: &[f32], _frames: usize) -> Result<Vec<f32>, String> {
            Ok(vec![-30.0; feat.len()])
        }
    }

    fn noise(n: usize) -> Vec<f32> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.02
            })
            .collect()
    }

    fn run(samples: &[f32], model: &mut dyn MaskModel) -> Vec<f32> {
        let mut d = Denoiser::new(samples).unwrap();
        let blocks = d.blocks();
        let mut steps = 0;
        loop {
            steps += 1;
            if !d.step(model).unwrap() {
                break;
            }
        }
        assert_eq!(steps, blocks);
        d.finish()
    }

    #[test]
    fn a_gain_of_one_gives_the_recording_back() {
        // Long enough for three blocks, and not a whole number of hops.
        let x = noise(HOP * (2 * BLOCK + 300) + 77);
        let y = run(&x, &mut KeepAll);
        assert_eq!(y.len(), x.len());
        let worst = x
            .iter()
            .zip(&y)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-5, "largest difference {worst}");
    }

    #[test]
    fn rejected_sound_is_left_at_the_gain_floor() {
        let x = noise(HOP * 600);
        let y = run(&x, &mut KeepNone);
        let rms = |v: &[f32]| (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt();
        let ratio = rms(&y) / rms(&x);
        assert!((ratio - GAIN_FLOOR).abs() < 0.005, "ratio {ratio}");
    }

    #[test]
    fn the_noise_scale_brings_white_noise_to_unit_variance() {
        let x = noise(HOP * 4000);
        let d = Denoiser::new(&x).unwrap();
        let rms = (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
        // The generator is uniform, not Gaussian, but its STFT bins are close
        // to Gaussian, so the scale lands near the RMS.
        assert!((d.scale / rms - 1.0).abs() < 0.05, "{} vs {rms}", d.scale);
    }

    #[test]
    fn short_and_silent_recordings_are_refused() {
        assert!(Denoiser::new(&[0.1; 100]).is_err());
        assert!(Denoiser::new(&vec![0.0; MIN_SAMPLES * 2]).is_err());
    }
}

//! The learned denoiser: a small convolution network that gives a gain for
//! every bin of a 512-point STFT.
//!
//! The network and its training are in the `batchi-noisier2noise` project
//! (`n2n.py`). This file repeats that project's `denoise` step for step, so a
//! recording comes out the same here as there: `torch.stft` framing (the audio
//! reflected by half a window at each end, a periodic Hann window), the same
//! noise scale, features, blocks and gain chain (see [`chain`], and
//! `reference.py --chain` there). The one difference is the noise scale of a
//! recording that is mostly empty upper bins or digital silence: see
//! `Denoiser::noise_scale`.
//!
//! The audio is used sample for sample at whatever rate it has. The network
//! (v2, `oversample-denoise-v2.onnx`) was trained that way on bat recordings
//! at 250 to 500 kHz and bird recordings at 22 to 96 kHz. It sees bins and
//! frames, not hertz.

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::collections::HashMap;
use std::sync::Arc;
use tract_onnx::prelude::*;

pub mod chain;
pub mod stream;

use chain::GainChain;
pub use chain::{check_smooth_frames, DenoiseGain};
pub use oversample_ipc::classifier::DEFAULT_SMOOTH_FRAMES;

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
/// The bins (20 to 99) whose median power says how loud a bin with sound in
/// it is, as `analyse` in `bird_n2n.py` has them.
const SCALE_REFERENCE_BINS: usize = 80;
/// A bin is empty when its median power is no more than this share of the
/// reference bins' median power (40 dB down). An MP3 encoder's low-pass and a
/// conversion up from a lower sample rate leave bins like that.
const EMPTY_BIN_RATIO: f32 = 1e-4;
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

/// The samples with half a window of their own reflection at each end, as
/// `torch.stft` pads them.
fn reflect_padded(samples: &[f32]) -> Vec<f32> {
    let len = samples.len();
    let half = WINDOW / 2;
    let mut padded = Vec::with_capacity(len + WINDOW);
    padded.extend((1..=half).rev().map(|i| samples[i]));
    padded.extend_from_slice(samples);
    padded.extend((1..=half).map(|i| samples[len - 1 - i]));
    padded
}

/// The window and the FFTs, and the steps that [`Denoiser`] (a recording held
/// whole) and [`stream::StreamDenoiser`] (a recording that arrives in pieces)
/// both take, so that the two give the same samples.
struct Tools {
    window: Vec<f32>,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
}

impl Tools {
    fn new() -> Tools {
        let mut planner = RealFftPlanner::<f32>::new();
        Tools {
            window: periodic_hann(),
            forward: planner.plan_fft_forward(WINDOW),
            inverse: planner.plan_fft_inverse(WINDOW),
        }
    }

    /// STFT of frames `lo..hi` of `padded`, frame after frame, `BINS` values
    /// each. Frame `t` starts at `padded[t * HOP]`.
    fn stft(&self, padded: &[f32], lo: usize, hi: usize) -> Vec<Complex<f32>> {
        let mut input = self.forward.make_input_vec();
        let mut spectrum = self.forward.make_output_vec();
        let mut out = Vec::with_capacity((hi - lo) * BINS);
        for t in lo..hi {
            let start = t * HOP;
            for (i, x) in input.iter_mut().enumerate() {
                *x = padded[start + i] * self.window[i];
            }
            self.forward
                .process(&mut input, &mut spectrum)
                .expect("FFT sizes are fixed");
            out.extend_from_slice(&spectrum);
        }
        out
    }

    /// Adds to `power` the power of one frame's bins from `SCALE_LOW_BIN` up.
    /// The frame starts at `padded[0]`.
    fn scale_frame(&self, padded: &[f32], power: &mut Vec<f32>) {
        let frame = self.stft(padded, 0, 1);
        power.extend(frame[SCALE_LOW_BIN..].iter().map(|c| c.norm_sqr()));
    }

    /// The network's gains for a block of `frames` frames, 2 * sigmoid(logit)
    /// - 1 for each cell, `frames` values for each bin, bin after bin, for
    /// each channel. `spectra` has each channel's STFT of those frames. A
    /// channel whose scale is zero is left out. [`GainChain`] makes the
    /// gains that go on the recording from these.
    fn block_gains(
        &self,
        spectra: &[Vec<Complex<f32>>],
        scales: &[f32],
        frames: usize,
        model: &mut dyn MaskModel,
    ) -> Result<Vec<Vec<f32>>, String> {
        let mut gains = Vec::new();
        let mut feat = vec![0.0f32; BINS * frames];
        let mut column = vec![0.0f32; frames];
        for (spectrum, scale) in spectra.iter().zip(scales) {
            if *scale <= 0.0 {
                continue;
            }
            // Features: log magnitude, less each bin's median over these
            // frames.
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
            gains.push(
                logits
                    .iter()
                    .map(|logit| 2.0 / (1.0 + (-logit).exp()) - 1.0)
                    .collect(),
            );
        }
        Ok(gains)
    }

    /// Adds `count` frames of `spectrum`, from its frame `first` on, to `out`
    /// with `gains` on them. `gains` has `count` values for each bin, bin
    /// after bin, and the first frame added starts at `out[0]`.
    fn add_frames(
        &self,
        spectrum: &[Complex<f32>],
        first: usize,
        gains: &[f32],
        count: usize,
        out: &mut [f32],
    ) {
        let mut bins = self.inverse.make_input_vec();
        let mut time = self.inverse.make_output_vec();
        for n in 0..count {
            let local = first + n;
            for (bin, value) in bins.iter_mut().enumerate() {
                *value = spectrum[local * BINS + bin] * gains[bin * count + n];
            }
            // realfft wants the imaginary parts at DC and Nyquist to be zero.
            bins[0].im = 0.0;
            bins[BINS - 1].im = 0.0;
            self.inverse
                .process(&mut bins, &mut time)
                .expect("FFT sizes are fixed");
            let at = n * HOP;
            for i in 0..WINDOW {
                out[at + i] += time[i] / WINDOW as f32 * self.window[i];
            }
        }
    }

    /// Adds the squared window of `count` frames to `window_sum`, the first
    /// frame starting at `window_sum[0]`.
    fn add_window(&self, count: usize, window_sum: &mut [f32]) {
        for n in 0..count {
            let at = n * HOP;
            for i in 0..WINDOW {
                window_sum[at + i] += self.window[i] * self.window[i];
            }
        }
    }
}

/// One output sample: what the frames added up to, over the window sum, at
/// the channel's own level again.
fn output_sample(added: f32, window_sum: f32, scale: f32) -> f32 {
    if window_sum > 1e-8 {
        added / window_sum * scale
    } else {
        0.0
    }
}

/// How many frames apart the noise scale samples a recording of `frames`
/// frames: every frame, until that is more than `SCALE_MAX_VALUES` values.
fn scale_stride(frames: u64) -> u64 {
    let per_frame = (BINS - SCALE_LOW_BIN) as u64;
    (frames * per_frame)
        .div_ceil(SCALE_MAX_VALUES as u64)
        .max(1)
}

/// The noise scale from the power of the sampled frames: `BINS -
/// SCALE_LOW_BIN` values for each frame, frame after frame. What it works
/// out is described at [`Denoiser::noise_scale`].
fn scale_from_power(power: Vec<f32>) -> f32 {
    let per_frame = BINS - SCALE_LOW_BIN;
    // Each bin's median power over the frames that are not digital
    // silence. The reference bins' median of those is the level of a bin
    // with sound in it, and a bin far under it is empty.
    let sounding: Vec<&[f32]> = power
        .chunks_exact(per_frame)
        .filter(|frame| frame.iter().any(|p| *p > 0.0))
        .collect();
    if sounding.is_empty() {
        return 0.0;
    }
    let mut column = Vec::with_capacity(sounding.len());
    let medians: Vec<f32> = (0..per_frame)
        .map(|bin| {
            column.clear();
            column.extend(sounding.iter().map(|frame| frame[bin]));
            lower_median(&mut column)
        })
        .collect();
    let empty_under = lower_median(&mut medians[..SCALE_REFERENCE_BINS].to_vec()) * EMPTY_BIN_RATIO;

    // The plain median, over a copy: `sounding` needs `power` in order.
    let plain = lower_median(&mut power.clone());
    let median = if plain > empty_under {
        plain
    } else {
        let Some(top) = medians.iter().rposition(|m| *m > empty_under) else {
            return 0.0;
        };
        let mut values: Vec<f32> = sounding
            .iter()
            .flat_map(|frame| &frame[..=top])
            .copied()
            .collect();
        lower_median(&mut values)
    };
    median.sqrt() / UNIT_NOISE_MEDIAN
}

/// A measured scale as the denoiser uses it: zero for a channel with no
/// sound in it.
fn usable_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        0.0
    }
}

/// Why a recording of `len` samples is refused.
fn too_short(len: u64) -> String {
    format!("the recording has {len} samples; the denoiser needs {MIN_SAMPLES}")
}

/// Why a recording with no sound in any channel is refused.
const SILENT: &str = "the recording is silent";

/// One channel of the recording being denoised.
struct Channel {
    /// The channel reflected by half a window at each end, divided by `scale`.
    padded: Vec<f32>,
    /// The channel's noise scale. Zero for a channel with no sound in it,
    /// which is left out of the gains and comes out silent.
    scale: f32,
    out: Vec<f32>,
}

/// One recording being denoised, a block per [`Denoiser::step`]. It holds the
/// whole recording. [`stream::StreamDenoiser`] gives the same samples from a
/// recording that arrives in pieces.
///
/// A recording with more than one channel gets one gain for all of its
/// channels: each channel has its own noise scale, features and run of the
/// network, and every time-frequency cell then takes the largest of the
/// channels' gains. A sound that one channel's gains keep is kept in all of
/// them, and the level difference between the channels stays as it was.
/// Where that comes in each [`DenoiseGain`] chain is in [`chain`].
pub struct Denoiser {
    channels: Vec<Channel>,
    /// Samples in each channel.
    len: usize,
    frames: usize,
    /// First frame of the next block.
    next: usize,
    window_sum: Vec<f32>,
    tools: Tools,
    chain: GainChain,
}

impl Denoiser {
    /// Takes mono samples at the recording's own rate.
    pub fn new(samples: &[f32]) -> Result<Denoiser, String> {
        Denoiser::interleaved(samples, 1)
    }

    /// Takes a recording of `channels` channels, its samples interleaved, at
    /// the recording's own rate.
    pub fn interleaved(samples: &[f32], channels: usize) -> Result<Denoiser, String> {
        if channels == 0 || !samples.len().is_multiple_of(channels) {
            return Err(format!(
                "{} samples do not divide into {channels} channels",
                samples.len()
            ));
        }
        let len = samples.len() / channels;
        if len < MIN_SAMPLES {
            return Err(too_short(len as u64));
        }
        let padded_len = len + WINDOW;
        let mut denoiser = Denoiser {
            channels: Vec::with_capacity(channels),
            len,
            frames: 1 + len / HOP,
            next: 0,
            window_sum: vec![0.0; padded_len],
            tools: Tools::new(),
            chain: GainChain::new(DenoiseGain::Plain, DEFAULT_SMOOTH_FRAMES as usize),
        };
        for c in 0..channels {
            let mut padded = if channels == 1 {
                reflect_padded(samples)
            } else {
                let one: Vec<f32> = samples.iter().skip(c).step_by(channels).copied().collect();
                reflect_padded(&one)
            };
            let scale = usable_scale(denoiser.noise_scale(&padded));
            if scale > 0.0 {
                for s in &mut padded {
                    *s /= scale;
                }
            }
            denoiser.channels.push(Channel {
                padded,
                scale,
                out: vec![0.0; padded_len],
            });
        }
        if denoiser.channels.iter().all(|c| c.scale == 0.0) {
            return Err(SILENT.to_string());
        }
        Ok(denoiser)
    }

    /// The same recording with `gain` done to the network's gain. Without
    /// it the chain is [`DenoiseGain::Plain`]. Called before the first step.
    pub fn with_gain(mut self, gain: DenoiseGain) -> Denoiser {
        debug_assert_eq!(self.next, 0, "the chain is chosen before the first step");
        self.chain = GainChain::new(gain, self.chain.frames());
        self
    }

    /// The same recording with a median of `frames` frames in the smoothed
    /// and heavy chains, checked by [`check_smooth_frames`]. Without it the
    /// median takes [`DEFAULT_SMOOTH_FRAMES`]. Called before the first step.
    pub fn with_smooth_frames(mut self, frames: usize) -> Denoiser {
        debug_assert_eq!(self.next, 0, "the chain is chosen before the first step");
        self.chain = GainChain::new(self.chain.kind(), frames);
        self
    }

    /// The factor each channel was divided by to bring its noise to about
    /// unit variance. Zero for a channel with no sound in it.
    pub fn scales(&self) -> Vec<f32> {
        self.channels.iter().map(|c| c.scale).collect()
    }

    /// Steps the recording takes.
    pub fn blocks(&self) -> usize {
        self.frames.div_ceil(BLOCK)
    }

    /// Steps done so far.
    pub fn blocks_done(&self) -> usize {
        self.next.div_ceil(BLOCK)
    }

    /// The factor that brings the recording's noise to about unit variance:
    /// the median magnitude over every frame and every bin from
    /// `SCALE_LOW_BIN` up, against that of unit white noise. It needs no
    /// call-free stretch. Long recordings use every k-th frame.
    ///
    /// In two kinds of recording that median is zero or far too small,
    /// because over half of the values it is taken over are empty:
    ///
    /// - Empty upper bins (audio converted up from a lower sample rate, or a
    ///   low-pass well under the Nyquist frequency).
    /// - Digital silence in half of the frames or more.
    ///
    /// So when the median is no higher than an empty bin's level, it is taken
    /// again over less: the frames that are not all zeros, and the bins up to
    /// the highest one with sound in it. Every other recording keeps the
    /// plain median, bit for bit. Zero means the recording has no sound at
    /// all.
    fn noise_scale(&self, padded: &[f32]) -> f32 {
        let per_frame = BINS - SCALE_LOW_BIN;
        let stride = scale_stride(self.frames as u64) as usize;
        let mut power = Vec::with_capacity(self.frames / stride * per_frame + per_frame);
        for t in (0..self.frames).step_by(stride) {
            self.tools.scale_frame(&padded[t * HOP..], &mut power);
        }
        scale_from_power(power)
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

        let spectra: Vec<_> = self
            .channels
            .iter()
            .map(|channel| self.tools.stft(&channel.padded, lo, hi))
            .collect();
        let raw = self
            .tools
            .block_gains(&spectra, &self.scales(), frames, model)?;
        let ready = self
            .chain
            .push(&raw, frames, start - lo, end - start, end == self.frames);

        // The frames whose gains are ready: this block's own frames, or with
        // the smoothed chains the last few of the block before and all but
        // the last few of this one. They are inside this block's context, and
        // the gains are the same for every channel.
        let first = ready.first as usize;
        for (channel, spectrum) in self.channels.iter_mut().zip(&spectra) {
            if channel.scale == 0.0 {
                continue;
            }
            self.tools.add_frames(
                spectrum,
                first - lo,
                &ready.gains,
                ready.count,
                &mut channel.out[first * HOP..],
            );
        }
        self.tools
            .add_window(ready.count, &mut self.window_sum[first * HOP..]);
        self.next = end;
        Ok(self.next < self.frames)
    }

    /// The denoised recording, the same length as what went in, with its
    /// channels interleaved.
    pub fn finish(self) -> Vec<f32> {
        let half = WINDOW / 2;
        let channels = self.channels.len();
        let mut out = Vec::with_capacity(self.len * channels);
        for i in half..half + self.len {
            let sum = self.window_sum[i];
            for channel in &self.channels {
                out.push(output_sample(channel.out[i], sum, channel.scale));
            }
        }
        out
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
        run_with(samples, model, DenoiseGain::Plain)
    }

    fn run_with(samples: &[f32], model: &mut dyn MaskModel, gain: DenoiseGain) -> Vec<f32> {
        let mut d = Denoiser::new(samples).unwrap().with_gain(gain);
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
    fn a_gain_of_one_gives_the_recording_back_in_every_chain() {
        let x = noise(HOP * (BLOCK + 300) + 77);
        for gain in [DenoiseGain::Smoothed, DenoiseGain::Heavy] {
            let y = run_with(&x, &mut KeepAll, gain);
            let worst = x
                .iter()
                .zip(&y)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(worst < 1e-5, "{gain:?}: largest difference {worst}");
        }
    }

    #[test]
    fn rejected_sound_is_left_at_the_gain_floor() {
        let x = noise(HOP * 600);
        let rms = |v: &[f32]| (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt();
        for (gain, floor) in [
            (DenoiseGain::Plain, GAIN_FLOOR),
            (DenoiseGain::Smoothed, GAIN_FLOOR),
            (DenoiseGain::Heavy, chain::HEAVY_FLOOR),
        ] {
            let y = run_with(&x, &mut KeepNone, gain);
            let ratio = rms(&y) / rms(&x);
            assert!(
                (ratio / floor - 1.0).abs() < 0.05,
                "{gain:?}: ratio {ratio}"
            );
        }
    }

    #[test]
    fn the_noise_scale_brings_white_noise_to_unit_variance() {
        let x = noise(HOP * 4000);
        let d = Denoiser::new(&x).unwrap();
        let rms = (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
        // The generator is uniform, not Gaussian, but its STFT bins are close
        // to Gaussian, so the scale lands near the RMS.
        let scale = d.scales()[0];
        assert!((scale / rms - 1.0).abs() < 0.05, "{scale} vs {rms}");
    }

    /// `x` with everything above `cutoff` (a share of the sample rate)
    /// filtered out by a windowed-sinc low-pass, as an MP3 encoder or a
    /// sample rate conversion leaves a recording.
    fn low_passed(x: &[f32], cutoff: f32) -> Vec<f32> {
        let half = 127i32;
        let taps: Vec<f32> = (-half..=half)
            .map(|k| {
                let t = k as f32;
                let sinc = if k == 0 {
                    2.0 * cutoff
                } else {
                    (std::f32::consts::TAU * cutoff * t).sin() / (std::f32::consts::PI * t)
                };
                let u = (k + half) as f32 / (2 * half) as f32;
                let blackman = 0.42 - 0.5 * (std::f32::consts::TAU * u).cos()
                    + 0.08 * (2.0 * std::f32::consts::TAU * u).cos();
                sinc * blackman
            })
            .collect();
        (0..x.len())
            .map(|i| {
                taps.iter()
                    .enumerate()
                    .filter_map(|(j, tap)| {
                        let at = i as isize + j as isize - half as isize;
                        x.get(usize::try_from(at).ok()?).map(|s| s * tap)
                    })
                    .sum()
            })
            .collect()
    }

    #[test]
    fn a_recording_with_sound_in_every_bin_keeps_the_scale_it_had() {
        // The scale as it was first written: the median over every frame and
        // every bin from SCALE_LOW_BIN up. Recordings made before the change
        // to `noise_scale` must come out bit for bit the same.
        let x = noise(HOP * 3000 + 5);
        let padded = reflect_padded(&x);
        let window = periodic_hann();
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(WINDOW);
        let mut input = fft.make_input_vec();
        let mut spectrum = fft.make_output_vec();
        let mut power = Vec::new();
        for t in 0..1 + x.len() / HOP {
            for (i, v) in input.iter_mut().enumerate() {
                *v = padded[t * HOP + i] * window[i];
            }
            fft.process(&mut input, &mut spectrum).unwrap();
            power.extend(spectrum[SCALE_LOW_BIN..].iter().map(|c| c.norm_sqr()));
        }
        let first = lower_median(&mut power).sqrt() / UNIT_NOISE_MEDIAN;
        let d = Denoiser::new(&x).unwrap();
        assert_eq!(d.scales()[0].to_bits(), first.to_bits());
    }

    #[test]
    fn a_recording_that_is_mostly_digital_silence_is_denoised() {
        // 30% noise, then 70% zeros: over half of all values are zero, so the
        // median over everything is zero.
        let sound = noise(HOP * 1200);
        let mut x = sound.clone();
        x.resize(HOP * 4000, 0.0);
        let d = Denoiser::new(&x).expect("a recording with sound in it");
        let alone = Denoiser::new(&sound).unwrap().scales()[0];
        assert!(
            (d.scales()[0] / alone - 1.0).abs() < 0.05,
            "{} vs {alone}",
            d.scales()[0]
        );
    }

    #[test]
    fn empty_upper_bins_are_left_out_of_the_noise_scale() {
        // Noise with nothing above a third of the Nyquist frequency: two
        // thirds of the bins are empty, as in audio converted up from a lower
        // sample rate. The scale is that of the bins with sound in them.
        let white = noise(HOP * 1500);
        let x = low_passed(&white, 1.0 / 6.0);
        let d = Denoiser::new(&x).unwrap();
        let full = Denoiser::new(&white).unwrap().scales()[0];
        assert!(
            (d.scales()[0] / full - 1.0).abs() < 0.1,
            "{} vs {full} for white noise",
            d.scales()[0]
        );
    }

    #[test]
    fn digital_silence_in_a_recording_gives_no_bad_samples() {
        // Half noise, half zeros, with the silence in the middle.
        let mut x = noise(HOP * 2400 + 31);
        let (from, to) = (HOP * 500, HOP * 1700);
        x[from..to].fill(0.0);
        for model in [&mut KeepAll as &mut dyn MaskModel, &mut KeepNone] {
            let y = run(&x, model);
            assert!(y.iter().all(|s| s.is_finite()));
            // Away from its edges the silence stays silent.
            let inside = &y[from + WINDOW..to - WINDOW];
            assert!(inside.iter().all(|s| *s == 0.0));
        }
        let y = run(&x, &mut KeepAll);
        let worst = x
            .iter()
            .zip(&y)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-5, "largest difference {worst}");
    }

    /// A model that keeps a cell that stands well above its bin's median
    /// and rejects the rest.
    struct KeepLoud;
    impl MaskModel for KeepLoud {
        fn logits(&mut self, feat: &[f32], _frames: usize) -> Result<Vec<f32>, String> {
            Ok(feat
                .iter()
                .map(|f| if *f > 2.0 { 30.0 } else { -30.0 })
                .collect())
        }
    }

    fn interleave(left: &[f32], right: &[f32]) -> Vec<f32> {
        left.iter().zip(right).flat_map(|(l, r)| [*l, *r]).collect()
    }

    fn channel(interleaved: &[f32], c: usize) -> Vec<f32> {
        interleaved.iter().skip(c).step_by(2).copied().collect()
    }

    fn run_stereo(samples: &[f32], model: &mut dyn MaskModel) -> Vec<f32> {
        run_stereo_with(samples, model, DenoiseGain::Plain)
    }

    fn run_stereo_with(samples: &[f32], model: &mut dyn MaskModel, gain: DenoiseGain) -> Vec<f32> {
        let mut d = Denoiser::interleaved(samples, 2).unwrap().with_gain(gain);
        while d.step(model).unwrap() {}
        d.finish()
    }

    /// How much of `x` is at `bin`'s frequency, over the samples from `from`
    /// to `to` under a Hann window, which keeps other frequencies out.
    fn level_at(x: &[f32], bin: usize, from: usize, to: usize) -> f32 {
        let step = std::f32::consts::TAU * bin as f32 / WINDOW as f32;
        let n = (to - from) as f32;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, s) in x[from..to].iter().enumerate() {
            let s = *s * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n).cos());
            re += f64::from(s * (step * i as f32).cos());
            im += f64::from(s * (step * i as f32).sin());
        }
        (re * re + im * im).sqrt() as f32
    }

    #[test]
    fn both_channels_get_the_gain_of_the_channel_that_keeps_more() {
        // Left: noise with a tone that comes and goes. Right: other noise.
        // Only the left channel's gains keep the tone's cells, and the right
        // channel must get the same gain there.
        let n = HOP * 2000;
        let tone_bin = 60;
        let step = std::f32::consts::TAU * tone_bin as f32 / WINDOW as f32;
        let on = |i: usize| (i / (HOP * 100)) % 4 == 1;
        let mut left = noise(n);
        for (i, s) in left.iter_mut().enumerate() {
            if on(i) {
                *s += 0.2 * (step * i as f32).sin();
            }
        }
        let right: Vec<f32> = noise(2 * n)[n..].to_vec();
        let x = interleave(&left, &right);
        let y = run_stereo(&x, &mut KeepLoud);
        assert_eq!(y.len(), x.len());
        let (out_left, out_right) = (channel(&y, 0), channel(&y, 1));

        // Inside a stretch with the tone on, away from its edges.
        let (from, to) = (HOP * 110, HOP * 190);
        assert!(on(from) && on(to - 1));
        let kept_left =
            level_at(&out_left, tone_bin, from, to) / level_at(&left, tone_bin, from, to);
        let kept_right =
            level_at(&out_right, tone_bin, from, to) / level_at(&right, tone_bin, from, to);
        assert!((kept_left - 1.0).abs() < 0.02, "left keeps {kept_left}");
        assert!((kept_right - 1.0).abs() < 0.02, "right keeps {kept_right}");
        // A bin away from the tone is at the gain floor in both channels.
        let other = 150;
        for (before, after) in [(&left, &out_left), (&right, &out_right)] {
            let kept = level_at(after, other, from, to) / level_at(before, other, from, to);
            assert!((kept - GAIN_FLOOR).abs() < 0.02, "bin {other} keeps {kept}");
        }

        // On its own, the right channel loses the tone's bin too.
        let alone = run(&right, &mut KeepLoud);
        let kept = level_at(&alone, tone_bin, from, to) / level_at(&right, tone_bin, from, to);
        assert!((kept - GAIN_FLOOR).abs() < 0.02, "right alone keeps {kept}");
    }

    #[test]
    fn two_channels_that_are_the_same_come_out_as_the_mono_result() {
        let x = noise(HOP * (BLOCK + 300) + 77);
        for gain in [
            DenoiseGain::Plain,
            DenoiseGain::Smoothed,
            DenoiseGain::Heavy,
        ] {
            let mono = run_with(&x, &mut KeepLoud, gain);
            let y = run_stereo_with(&interleave(&x, &x), &mut KeepLoud, gain);
            assert!(channel(&y, 0) == mono, "{gain:?}");
            assert!(channel(&y, 1) == mono, "{gain:?}");
        }
    }

    #[test]
    fn a_silent_channel_adds_nothing_to_the_smoothed_gains() {
        let x = noise(HOP * 900);
        let silent = vec![0.0; x.len()];
        for gain in [DenoiseGain::Smoothed, DenoiseGain::Heavy] {
            let y = run_stereo_with(&interleave(&silent, &x), &mut KeepLoud, gain);
            assert!(
                channel(&y, 1) == run_with(&x, &mut KeepLoud, gain),
                "{gain:?}"
            );
            assert!(channel(&y, 0).iter().all(|s| *s == 0.0));
        }
    }

    #[test]
    fn a_silent_channel_stays_silent_and_adds_nothing_to_the_gains() {
        let x = noise(HOP * 900);
        let silent = vec![0.0; x.len()];
        let y = run_stereo(&interleave(&x, &silent), &mut KeepLoud);
        assert!(channel(&y, 0) == run(&x, &mut KeepLoud));
        assert!(channel(&y, 1).iter().all(|s| *s == 0.0));
        // Two silent channels are refused, as a silent mono recording is.
        assert!(Denoiser::interleaved(&interleave(&silent, &silent), 2).is_err());
        // Samples that don't divide into the channels are refused.
        assert!(Denoiser::interleaved(&x[..x.len() - 1], 2).is_err());
    }

    #[test]
    fn short_and_silent_recordings_are_refused() {
        assert!(Denoiser::new(&[0.1; 100]).is_err());
        assert!(Denoiser::new(&vec![0.0; MIN_SAMPLES * 2]).is_err());
    }
}

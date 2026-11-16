//! The denoiser for a recording that arrives in pieces.
//!
//! [`StreamDenoiser`] gives the same samples as [`super::Denoiser`], bit for
//! bit, and holds a few blocks of the recording at a time instead of all of
//! it. The recording goes through twice:
//!
//! 1. [`StreamDenoiser::measure`] takes every piece and works out each
//!    channel's noise scale, which is one number for the whole recording and
//!    is needed before the first block runs.
//! 2. [`StreamDenoiser::push`] takes every piece again and returns the
//!    denoised samples that are finished.
//!
//! A block's output is finished once the block after it has no frame left to
//! add to it, so the output runs about one block behind the input.

use super::{
    output_sample, scale_from_power, scale_stride, too_short, usable_scale, DenoiseGain, GainChain,
    MaskModel, Tools, BLOCK, CONTEXT, HOP, MIN_SAMPLES, SILENT, WINDOW,
};

const HALF: usize = WINDOW / 2;
/// Samples of one channel taken in at a time, so that a caller's large piece
/// costs no more memory than a small one.
const CHUNK: usize = BLOCK * HOP;

/// One channel with half a window of its own reflection at each end, as
/// `reflect_padded` gives it, built while the samples arrive. Only the part
/// from `base` on is held.
struct Padded {
    buf: Vec<f32>,
    /// Where `buf[0]` is in the padded channel.
    base: u64,
    /// The channel's first samples, held until there are enough to reflect.
    front: Option<Vec<f32>>,
    /// The channel's last `HALF + 1` samples so far, for the reflection at
    /// the end.
    tail: Vec<f32>,
    /// Samples taken so far.
    taken: u64,
    /// Samples the channel has in all.
    len: u64,
}

impl Padded {
    fn new(len: u64) -> Padded {
        Padded {
            buf: Vec::new(),
            base: 0,
            front: Some(Vec::new()),
            tail: Vec::new(),
            taken: 0,
            len,
        }
    }

    /// Takes the channel's next samples. The caller gives no more than `len`
    /// in all, and no empty slice.
    fn push(&mut self, samples: &[f32]) {
        self.taken += samples.len() as u64;
        let keep = HALF + 1;
        if samples.len() >= keep {
            self.tail.clear();
            self.tail
                .extend_from_slice(&samples[samples.len() - keep..]);
        } else {
            self.tail.extend_from_slice(samples);
            let extra = self.tail.len().saturating_sub(keep);
            self.tail.drain(..extra);
        }
        match &mut self.front {
            Some(front) => {
                front.extend_from_slice(samples);
                if front.len() > HALF {
                    self.buf.extend((1..=HALF).rev().map(|i| front[i]));
                    self.buf.extend_from_slice(front);
                    self.front = None;
                }
            }
            None => self.buf.extend_from_slice(samples),
        }
        if self.taken == self.len {
            // `len` is at least MIN_SAMPLES, so the tail is full.
            let last = self.tail.len() - 1;
            self.buf.extend((1..=HALF).map(|i| self.tail[last - i]));
        }
    }

    /// One past the last padded sample held.
    fn end(&self) -> u64 {
        self.base + self.buf.len() as u64
    }

    /// The padded samples from `index` on.
    fn from(&self, index: u64) -> &[f32] {
        &self.buf[(index - self.base) as usize..]
    }

    /// Lets go of the padded samples before `index`.
    fn drop_before(&mut self, index: u64) {
        let n = (index.saturating_sub(self.base)).min(self.buf.len() as u64) as usize;
        self.buf.drain(..n);
        self.base += n as u64;
    }
}

/// One channel while its noise scale is measured.
struct Meter {
    padded: Padded,
    /// The power of the frames sampled so far.
    power: Vec<f32>,
    /// The next frame to sample.
    next: u64,
}

/// The second time through: the blocks.
struct Pass {
    /// Each channel's noise scale. Zero for a channel with no sound in it.
    scales: Vec<f32>,
    /// Each channel divided by its scale.
    padded: Vec<Padded>,
    /// Each channel's frames added up, from the first frame not yet added
    /// on. Empty for a channel with no sound in it.
    out: Vec<Vec<f32>>,
    /// The squared windows of those frames, added up.
    window_sum: Vec<f32>,
    /// First frame of the next block.
    next: u64,
    /// The next padded sample to give out.
    emitted: u64,
    chain: GainChain,
}

enum Stage {
    Measuring(Vec<Meter>),
    Denoising(Pass),
}

/// One recording being denoised as it arrives. See the module's notes.
pub struct StreamDenoiser {
    tools: Tools,
    channels: usize,
    /// Samples in each channel.
    len: u64,
    frames: u64,
    gain: DenoiseGain,
    stage: Stage,
}

impl StreamDenoiser {
    /// For a recording of `len` samples in each of `channels` channels.
    pub fn new(len: u64, channels: usize) -> Result<StreamDenoiser, String> {
        if channels == 0 {
            return Err("the recording has no channels".to_string());
        }
        if len < MIN_SAMPLES as u64 {
            return Err(too_short(len));
        }
        let meters = (0..channels)
            .map(|_| Meter {
                padded: Padded::new(len),
                power: Vec::new(),
                next: 0,
            })
            .collect();
        Ok(StreamDenoiser {
            tools: Tools::new(),
            channels,
            len,
            frames: 1 + len / HOP as u64,
            gain: DenoiseGain::Plain,
            stage: Stage::Measuring(meters),
        })
    }

    /// The same recording with `gain` done to the network's gain, as
    /// [`super::Denoiser::with_gain`]. Called before the first piece.
    pub fn with_gain(mut self, gain: DenoiseGain) -> StreamDenoiser {
        self.gain = gain;
        self
    }

    /// Each channel's noise scale, as [`super::Denoiser::scales`]. Empty
    /// until the whole recording has been measured.
    pub fn scales(&self) -> Vec<f32> {
        match &self.stage {
            Stage::Measuring(_) => Vec::new(),
            Stage::Denoising(pass) => pass.scales.clone(),
        }
    }

    /// Blocks the recording takes.
    pub fn blocks(&self) -> u64 {
        self.frames.div_ceil(BLOCK as u64)
    }

    /// Blocks done so far.
    pub fn blocks_done(&self) -> u64 {
        match &self.stage {
            Stage::Measuring(_) => 0,
            Stage::Denoising(pass) => pass.next.div_ceil(BLOCK as u64),
        }
    }

    /// True once every denoised sample has been given out.
    pub fn finished(&self) -> bool {
        matches!(&self.stage, Stage::Denoising(pass) if pass.next >= self.frames)
    }

    /// The samples of `piece` (interleaved) that the stage in progress has
    /// already taken must leave room for it.
    fn check_piece(&self, piece: &[f32], taken: u64) -> Result<(), String> {
        if !piece.len().is_multiple_of(self.channels) {
            return Err(format!(
                "{} samples do not divide into {} channels",
                piece.len(),
                self.channels
            ));
        }
        let more = (piece.len() / self.channels) as u64;
        if taken + more > self.len {
            return Err(format!(
                "{} samples arrived for a recording of {}",
                taken + more,
                self.len
            ));
        }
        Ok(())
    }

    /// First time through: takes the recording's next samples, interleaved,
    /// for the noise scale. True once the whole recording has been measured;
    /// the same samples then go to [`StreamDenoiser::push`], from the start.
    pub fn measure(&mut self, piece: &[f32]) -> Result<bool, String> {
        let Stage::Measuring(meters) = &self.stage else {
            return Err("the recording has been measured already".to_string());
        };
        self.check_piece(piece, meters[0].padded.taken)?;
        let Stage::Measuring(meters) = &mut self.stage else {
            unreachable!("checked above");
        };
        let stride = scale_stride(self.frames);
        let mut one = Vec::new();
        for part in piece.chunks(CHUNK * self.channels) {
            for (c, meter) in meters.iter_mut().enumerate() {
                one.clear();
                one.extend(part.iter().skip(c).step_by(self.channels));
                meter.padded.push(&one);
                while meter.next < self.frames
                    && meter.padded.end() >= meter.next * HOP as u64 + WINDOW as u64
                {
                    self.tools
                        .scale_frame(meter.padded.from(meter.next * HOP as u64), &mut meter.power);
                    meter.next += stride;
                }
                meter.padded.drop_before(meter.next * HOP as u64);
            }
        }
        if meters[0].padded.taken < self.len {
            return Ok(false);
        }
        let scales: Vec<f32> = meters
            .iter_mut()
            .map(|m| usable_scale(scale_from_power(std::mem::take(&mut m.power))))
            .collect();
        if scales.iter().all(|s| *s == 0.0) {
            return Err(SILENT.to_string());
        }
        self.stage = Stage::Denoising(Pass {
            padded: (0..self.channels).map(|_| Padded::new(self.len)).collect(),
            out: vec![Vec::new(); self.channels],
            window_sum: Vec::new(),
            next: 0,
            emitted: HALF as u64,
            scales,
            chain: GainChain::new(self.gain),
        });
        Ok(true)
    }

    /// Second time through: takes the recording's next samples, interleaved,
    /// and returns the denoised samples that are finished, interleaved. Over
    /// the whole recording the answers add up to as many samples as went in.
    pub fn push(&mut self, piece: &[f32], model: &mut dyn MaskModel) -> Result<Vec<f32>, String> {
        let Stage::Denoising(pass) = &self.stage else {
            return Err("the recording has not been measured".to_string());
        };
        self.check_piece(piece, pass.padded[0].taken)?;
        let Stage::Denoising(pass) = &mut self.stage else {
            unreachable!("checked above");
        };
        let mut done = Vec::new();
        let mut one = Vec::new();
        for part in piece.chunks(CHUNK * self.channels) {
            for (c, padded) in pass.padded.iter_mut().enumerate() {
                let scale = pass.scales[c];
                one.clear();
                let samples = part.iter().skip(c).step_by(self.channels);
                if scale > 0.0 {
                    one.extend(samples.map(|s| s / scale));
                } else {
                    one.extend(samples);
                }
                padded.push(&one);
            }
            while pass.block(&self.tools, self.frames, self.len, model, &mut done)? {}
        }
        Ok(done)
    }
}

impl Pass {
    /// Denoises the next block if all the samples it needs have arrived, and
    /// adds the samples that are finished to `done`. False when it has to
    /// wait for more, or the recording is finished.
    fn block(
        &mut self,
        tools: &Tools,
        frames: u64,
        len: u64,
        model: &mut dyn MaskModel,
        done: &mut Vec<f32>,
    ) -> Result<bool, String> {
        if self.next >= frames {
            return Ok(false);
        }
        let hop = HOP as u64;
        let start = self.next;
        let end = (start + BLOCK as u64).min(frames);
        let lo = start.saturating_sub(CONTEXT as u64);
        let hi = (end + CONTEXT as u64).min(frames);
        if self.padded[0].end() < (hi - 1) * hop + WINDOW as u64 {
            return Ok(false);
        }
        let count = (hi - lo) as usize;
        let own = (end - start) as usize;

        let spectra: Vec<_> = self
            .padded
            .iter()
            .map(|p| tools.stft(p.from(lo * hop), 0, count))
            .collect();
        let raw = tools.block_gains(&spectra, &self.scales, count, model)?;
        let ready = self
            .chain
            .push(&raw, count, (start - lo) as usize, own, end == frames);

        // `out` and `window_sum` start at `ready.first`, the first frame not
        // yet added.
        let added = ready.count;
        let reach = (added * HOP + WINDOW - HOP).max(self.window_sum.len());
        for ((out, spectrum), scale) in self.out.iter_mut().zip(&spectra).zip(&self.scales) {
            if *scale == 0.0 {
                continue;
            }
            out.resize(reach, 0.0);
            let from = (ready.first - lo) as usize;
            tools.add_frames(spectrum, from, &ready.gains, added, out);
        }
        self.window_sum.resize(reach, 0.0);
        tools.add_window(added, &mut self.window_sum);

        // No later frame reaches the samples before the first frame not yet
        // added. After the last block that is all of them.
        let first = ready.first * hop;
        let settled = if end == frames {
            HALF as u64 + len
        } else {
            (ready.first + added as u64) * hop
        };
        for i in self.emitted..settled {
            let at = (i - first) as usize;
            let sum = self.window_sum[at];
            for (out, scale) in self.out.iter().zip(&self.scales) {
                let added = out.get(at).copied().unwrap_or(0.0);
                done.push(output_sample(added, sum, *scale));
            }
        }
        self.emitted = settled;

        let gone = added * HOP;
        for out in &mut self.out {
            out.drain(..gone.min(out.len()));
        }
        self.window_sum.drain(..gone);
        for padded in &mut self.padded {
            padded.drop_before(end.saturating_sub(CONTEXT as u64) * hop);
        }
        self.next = end;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::denoise::{Denoiser, BINS};

    /// A model whose answer for a cell depends on the cell, the frame before
    /// it and the bin under it, so a block with the wrong frames or the wrong
    /// medians gives other gains.
    struct Neighbours;
    impl MaskModel for Neighbours {
        fn logits(&mut self, feat: &[f32], frames: usize) -> Result<Vec<f32>, String> {
            Ok((0..feat.len())
                .map(|i| {
                    let (bin, t) = (i / frames, i % frames);
                    let before = feat[bin * frames + t.saturating_sub(1)];
                    let under = feat[bin.saturating_sub(1) * frames + t];
                    2.0 * feat[i] + before + 0.5 * under - 1.0 + (BINS - bin) as f32 * 0.002
                })
                .collect())
        }
    }

    /// Noise with louder bursts, so the gains differ from cell to cell.
    fn noise(n: usize, seed: u64) -> Vec<f32> {
        let mut state = seed;
        (0..n)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let burst = if (i / 3000) % 5 == 2 { 6.0 } else { 1.0 };
                ((state >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.02 * burst
            })
            .collect()
    }

    fn interleave(left: &[f32], right: &[f32]) -> Vec<f32> {
        left.iter().zip(right).flat_map(|(l, r)| [*l, *r]).collect()
    }

    /// The recording held whole.
    fn whole(samples: &[f32], channels: usize, gain: DenoiseGain) -> (Vec<f32>, Vec<f32>) {
        let mut d = Denoiser::interleaved(samples, channels)
            .unwrap()
            .with_gain(gain);
        let scales = d.scales();
        while d.step(&mut Neighbours).unwrap() {}
        (scales, d.finish())
    }

    /// The recording in pieces of `piece` samples of each channel.
    fn in_pieces(
        samples: &[f32],
        channels: usize,
        piece: usize,
        gain: DenoiseGain,
    ) -> (Vec<f32>, Vec<f32>) {
        let mut d = StreamDenoiser::new((samples.len() / channels) as u64, channels)
            .unwrap()
            .with_gain(gain);
        let pieces: Vec<&[f32]> = samples.chunks(piece * channels).collect();
        for (i, p) in pieces.iter().enumerate() {
            let measured = d.measure(p).unwrap();
            assert_eq!(measured, i + 1 == pieces.len());
        }
        let mut out = Vec::new();
        for p in &pieces {
            assert!(!d.finished());
            out.extend(d.push(p, &mut Neighbours).unwrap());
        }
        assert!(d.finished());
        assert_eq!(d.blocks_done(), d.blocks());
        (d.scales(), out)
    }

    fn bits(v: &[f32]) -> Vec<u32> {
        v.iter().map(|s| s.to_bits()).collect()
    }

    /// Every piece size gives the samples of the recording held whole, bit
    /// for bit, with each of the gain chains.
    fn same_in_pieces(samples: &[f32], channels: usize, pieces: &[usize]) {
        for gain in [
            DenoiseGain::Plain,
            DenoiseGain::Smoothed,
            DenoiseGain::Heavy,
        ] {
            let (scales, expected) = whole(samples, channels, gain);
            assert_eq!(expected.len(), samples.len());
            for &piece in pieces {
                let (piece_scales, out) = in_pieces(samples, channels, piece, gain);
                assert_eq!(
                    bits(&piece_scales),
                    bits(&scales),
                    "{gain:?} scales, pieces of {piece}"
                );
                assert_eq!(
                    out.len(),
                    expected.len(),
                    "{gain:?} length, pieces of {piece}"
                );
                assert!(
                    bits(&out) == bits(&expected),
                    "{gain:?} samples, pieces of {piece}"
                );
            }
        }
    }

    /// Piece sizes that line up with the blocks, that don't, that are smaller
    /// than the reflection at the ends, and one piece for everything.
    const PIECES: [usize; 7] = [
        BLOCK * HOP,
        2 * BLOCK * HOP,
        BLOCK * HOP - 1,
        50_000,
        4096,
        100,
        usize::MAX / 4,
    ];

    #[test]
    fn a_mono_recording_in_pieces_equals_the_recording_held_whole() {
        // Three blocks, and not a whole number of hops.
        let x = noise(HOP * (2 * BLOCK + 300) + 77, 0x2545_f491_4f6c_dd1d);
        same_in_pieces(&x, 1, &PIECES);
    }

    #[test]
    fn a_stereo_recording_in_pieces_equals_the_recording_held_whole() {
        let n = HOP * (2 * BLOCK + 300) + 77;
        let left = noise(n, 11);
        let right: Vec<f32> = noise(n, 23).iter().map(|s| s * 0.4).collect();
        same_in_pieces(&interleave(&left, &right), 2, &PIECES);
    }

    #[test]
    fn a_recording_of_whole_blocks_equals_the_recording_held_whole() {
        // The last frame is the last of a block, and the one after it starts
        // a block of one frame.
        for n in [HOP * 2 * BLOCK - 1, HOP * 2 * BLOCK] {
            same_in_pieces(&noise(n, 5), 1, &[BLOCK * HOP, 77_777]);
        }
    }

    #[test]
    fn a_recording_shorter_than_a_block_equals_the_recording_held_whole() {
        same_in_pieces(&noise(HOP * 300 + 5, 77), 1, &PIECES);
        same_in_pieces(&noise(MIN_SAMPLES, 3), 1, &[MIN_SAMPLES, 100, 1]);
        let (left, right) = (noise(HOP * 300 + 5, 1), noise(HOP * 300 + 5, 2));
        same_in_pieces(&interleave(&left, &right), 2, &PIECES);
    }

    #[test]
    fn the_scale_of_a_long_recording_uses_the_same_frames() {
        // Long enough for the scale to leave frames out.
        let x = noise(HOP * 9000 + 3, 9);
        assert!(scale_stride(1 + x.len() as u64 / HOP as u64) > 1);
        same_in_pieces(&x, 1, &[BLOCK * HOP, 50_000]);
    }

    #[test]
    fn digital_silence_and_a_silent_channel_come_out_the_same() {
        let mut x = noise(HOP * 2400 + 31, 4);
        x[HOP * 500..HOP * 1700].fill(0.0);
        same_in_pieces(&x, 1, &[BLOCK * HOP, 50_000]);
        let silent = vec![0.0; x.len()];
        same_in_pieces(&interleave(&x, &silent), 2, &[BLOCK * HOP, 50_000]);
        same_in_pieces(&interleave(&silent, &x), 2, &[50_000]);
    }

    #[test]
    fn what_the_denoiser_holds_does_not_grow_with_the_recording() {
        let x = noise(HOP * 6 * BLOCK, 8);
        for gain in [
            DenoiseGain::Plain,
            DenoiseGain::Smoothed,
            DenoiseGain::Heavy,
        ] {
            let mut d = StreamDenoiser::new(x.len() as u64, 1)
                .unwrap()
                .with_gain(gain);
            for p in x.chunks(CHUNK) {
                d.measure(p).unwrap();
            }
            let (mut most, mut most_gains) = (0, 0);
            for p in x.chunks(CHUNK) {
                d.push(p, &mut Neighbours).unwrap();
                if let Stage::Denoising(pass) = &d.stage {
                    most = most.max(pass.padded[0].buf.len() + pass.out[0].len());
                    most_gains = most_gains.max(pass.chain.held_frames());
                }
            }
            assert!(most < 3 * CHUNK, "{gain:?}: {most} samples held");
            assert!(
                most_gains <= 8,
                "{gain:?}: gains of {most_gains} frames held"
            );
        }
    }

    #[test]
    fn short_silent_and_wrong_recordings_are_refused() {
        assert!(StreamDenoiser::new(100, 1).is_err());
        assert!(StreamDenoiser::new(10_000, 0).is_err());
        let mut d = StreamDenoiser::new(4096, 1).unwrap();
        assert!(d.push(&[0.0; 4096], &mut Neighbours).is_err());
        assert!(d.measure(&vec![0.0; 4096]).is_err(), "silent");
        let mut d = StreamDenoiser::new(4096, 2).unwrap();
        assert!(d.measure(&[0.1; 3]).is_err(), "half a frame");
        assert!(d.measure(&vec![0.1; 2 * 4097]).is_err(), "too many samples");
    }
}

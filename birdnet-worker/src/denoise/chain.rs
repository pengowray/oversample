//! What is done to the network's gain before it goes on the STFT.
//!
//! The network gives a gain `g = 2 * sigmoid(logit) - 1` for every cell
//! (bin and frame) of each channel. [`DenoiseGain`] picks one of three
//! chains, the same as `reference.py --chain` in the `batchi-noisier2noise`
//! project:
//!
//! - `Plain`: `max(g, 0.1)`.
//! - `Smoothed`: `g` (at least 0) becomes the median of the 5 bins x 9 frames
//!   round the cell (or another odd number of frames, the start message's
//!   `smooth_frames`; the app asks for 3 above 96 kHz), with the recording's
//!   first and last frame and the lowest and highest bin repeated past the
//!   edges; that is capped at the largest `g` of the cell's 3 x 3
//!   neighbours that are inside the recording; then floored at 0.1.
//! - `Heavy`: the smoothed gain before its floor, squared; then each bin's
//!   gain falls by at most 0.16 dB from one frame to the next (it may rise
//!   at once); then floored at 0.01.
//!
//! The smoothing and the release run over the gains of the whole recording,
//! each frame's gain taken from the block whose own frame it is. With a
//! median of 9 frames, a block's last 4 frames need the next block's first 4
//! frames, so [`GainChain`] gives out a block's gains 4 frames late (1 frame
//! late with 3 frames), and the last block gives out everything left. The
//! release carries each bin's gain from one block into the next. A
//! recording held whole and one that arrives in pieces push the same blocks,
//! so they get the same gains bit for bit.
//!
//! With more than one channel, each channel is smoothed by itself, and every
//! cell then takes the largest of the channels' smoothed gains; the floor,
//! the square and the release come after that. `Plain` takes the largest of
//! the channels' gains, as it always has.

pub use oversample_ipc::classifier::DenoiseGain;

use super::{BINS, GAIN_FLOOR};

/// Bins each side of a cell that the median takes in.
const MEDIAN_BINS: usize = 2;
/// Fewest frames the median may take in. The cap reads the frame after the
/// cell, so the chain must wait for at least one frame.
pub const MIN_SMOOTH_FRAMES: u32 = 3;
/// Most frames the median may take in.
pub const MAX_SMOOTH_FRAMES: u32 = 31;
/// Most cells in the median: 5 bins x 31 frames.
const MAX_MEDIAN_CELLS: usize = (2 * MEDIAN_BINS + 1) * MAX_SMOOTH_FRAMES as usize;
/// The heavy chain's gain floor (-40 dB).
pub const HEAVY_FLOOR: f32 = 0.01;
/// The most the heavy chain's gain falls from one frame to the next, in dB.
pub const RELEASE_DB_PER_FRAME: f64 = 0.16;

/// Checks a start message's `smooth_frames`: an odd number from
/// [`MIN_SMOOTH_FRAMES`] to [`MAX_SMOOTH_FRAMES`].
pub fn check_smooth_frames(frames: u32) -> Result<usize, String> {
    if frames % 2 == 1 && (MIN_SMOOTH_FRAMES..=MAX_SMOOTH_FRAMES).contains(&frames) {
        Ok(frames as usize)
    } else {
        Err(format!(
            "smooth_frames out of range: {frames} (must be an odd number from {MIN_SMOOTH_FRAMES} to {MAX_SMOOTH_FRAMES})"
        ))
    }
}

/// The gains for frames `first..first + count`, `count` values for each bin,
/// bin after bin.
pub(super) struct Ready {
    pub first: u64,
    pub count: usize,
    pub gains: Vec<f32>,
}

/// Takes each block's gains and gives out the chain's gains. See the
/// module's notes.
pub(super) struct GainChain {
    kind: DenoiseGain,
    /// Frames each side of a cell that the median takes in: 4 for a median
    /// of 9 frames, 1 for 3. Also how many frames late a block's gains are
    /// given out.
    half: usize,
    /// The next frame to give out.
    next: u64,
    /// The smoothed chains' `max(g, 0)` for frames `held_from..held_to`, one
    /// list for each channel with sound in it, frame after frame, `BINS`
    /// values each.
    held: Vec<Vec<f32>>,
    held_from: u64,
    held_to: u64,
    /// The heavy chain's gain on the last frame given out, before its floor.
    carry: Vec<f32>,
    /// The heavy chain's factor per frame: 0.16 dB down.
    release: f32,
}

impl GainChain {
    /// `frames` is the number of frames in the median, checked by
    /// [`check_smooth_frames`]. The plain chain does not use it.
    pub(super) fn new(kind: DenoiseGain, frames: usize) -> GainChain {
        debug_assert!(check_smooth_frames(frames as u32).is_ok(), "{frames}");
        GainChain {
            kind,
            half: frames / 2,
            next: 0,
            held: Vec::new(),
            held_from: 0,
            held_to: 0,
            carry: vec![0.0; BINS],
            release: 10f64.powf(-RELEASE_DB_PER_FRAME / 20.0) as f32,
        }
    }

    /// Takes one block. `raw` has `2 * sigmoid(logit) - 1` for `frames`
    /// frames, `frames` values for each bin, bin after bin, for each channel
    /// with sound in it. The block's own frames are `own` frames from
    /// `own_from` on. `last` says that this is the recording's last block.
    pub(super) fn push(
        &mut self,
        raw: &[Vec<f32>],
        frames: usize,
        own_from: usize,
        own: usize,
        last: bool,
    ) -> Ready {
        if self.kind == DenoiseGain::Plain {
            return self.plain(raw, frames, own_from, own);
        }
        if self.held.is_empty() {
            self.held = vec![Vec::new(); raw.len()];
        }
        for (held, raw) in self.held.iter_mut().zip(raw) {
            held.reserve(own * BINS);
            for n in 0..own {
                let t = own_from + n;
                held.extend((0..BINS).map(|bin| raw[bin * frames + t].max(0.0)));
            }
        }
        self.held_to += own as u64;
        let limit = if last {
            self.held_to
        } else {
            self.held_to.saturating_sub(self.half as u64).max(self.next)
        };
        let ready = self.smoothed(limit);
        // Frames from `limit - half` on are still needed.
        let keep_from = limit.saturating_sub(self.half as u64).max(self.held_from);
        let gone = ((keep_from - self.held_from) as usize) * BINS;
        for held in &mut self.held {
            held.drain(..gone);
        }
        self.held_from = keep_from;
        ready
    }

    /// The chain's kind.
    pub(super) fn kind(&self) -> DenoiseGain {
        self.kind
    }

    /// Frames in the median.
    pub(super) fn frames(&self) -> usize {
        2 * self.half + 1
    }

    /// Frames of gains held for the next block.
    #[cfg(test)]
    pub(super) fn held_frames(&self) -> u64 {
        self.held_to - self.held_from
    }

    /// The plain chain: the largest of the channels' gains, at least
    /// `GAIN_FLOOR`.
    fn plain(&mut self, raw: &[Vec<f32>], frames: usize, own_from: usize, own: usize) -> Ready {
        let mut gains = vec![0.0f32; BINS * own];
        for (c, raw) in raw.iter().enumerate() {
            for bin in 0..BINS {
                let from = &raw[bin * frames + own_from..bin * frames + own_from + own];
                let to = &mut gains[bin * own..(bin + 1) * own];
                for (shared, g) in to.iter_mut().zip(from) {
                    let g = g.max(GAIN_FLOOR);
                    *shared = if c == 0 { g } else { shared.max(g) };
                }
            }
        }
        let first = self.next;
        self.next += own as u64;
        Ready {
            first,
            count: own,
            gains,
        }
    }

    /// The smoothed or heavy chain's gains for frames `self.next..limit`.
    fn smoothed(&mut self, limit: u64) -> Ready {
        let first = self.next;
        let count = (limit - first) as usize;
        let mut gains = vec![0.0f32; BINS * count];
        // The smoothed gain is never above the cap. When the cap is at most
        // 0.1 (smoothed chain), or its square at most 0.01 (heavy chain), the
        // cell's final gain is the floor whatever the median is, and 0 in
        // place of the smoothed gain gives the same final gains, bit for bit,
        // in this cell, in the other channel's and in the release after it.
        // So the median is skipped there, which is most cells of a quiet
        // recording.
        let unused = |cap: f32| match self.kind {
            DenoiseGain::Heavy => cap * cap <= HEAVY_FLOOR,
            _ => cap <= GAIN_FLOOR,
        };
        let last_frame = self.held_to - 1;
        let mut shared = [0.0f32; BINS];
        let mut one = [0.0f32; BINS];
        let half = self.half;
        let window = 2 * half + 1;
        let mut cells_store = [0.0f32; MAX_MEDIAN_CELLS];
        let cells = &mut cells_store[..(2 * MEDIAN_BINS + 1) * window];
        let middle = cells.len() / 2;
        for n in 0..count {
            let t = first + n as u64;
            for (c, held) in self.held.iter().enumerate() {
                let column = |frame: u64| {
                    let at = (frame - self.held_from) as usize * BINS;
                    &held[at..at + BINS]
                };
                // The median's frames, the first and last frame repeated
                // past the recording's ends. Only the first `window` are used.
                let median_columns: [&[f32]; MAX_SMOOTH_FRAMES as usize] =
                    std::array::from_fn(|i| {
                        if i >= window {
                            return &[][..];
                        }
                        let frame = (t + i as u64).saturating_sub(half as u64);
                        column(frame.min(last_frame))
                    });
                // The cap's frames, only those inside the recording.
                let before = (t > 0).then(|| column(t - 1));
                let after = (t < last_frame).then(|| column(t + 1));
                let cap_columns = [before, Some(column(t)), after];
                for (bin, out) in one.iter_mut().enumerate() {
                    let (lo, hi) = (bin.saturating_sub(1), (bin + 1).min(BINS - 1));
                    let cap = cap_columns
                        .iter()
                        .flatten()
                        .flat_map(|col| &col[lo..=hi])
                        .fold(0.0f32, |a, b| a.max(*b));
                    if unused(cap) {
                        *out = 0.0;
                        continue;
                    }
                    let mut k = 0;
                    for col in &median_columns[..window] {
                        for d in 0..=2 * MEDIAN_BINS {
                            let b = (bin + d).saturating_sub(MEDIAN_BINS).min(BINS - 1);
                            cells[k] = col[b];
                            k += 1;
                        }
                    }
                    let median = *cells.select_nth_unstable_by(middle, f32::total_cmp).1;
                    *out = median.min(cap);
                }
                if c == 0 {
                    shared = one;
                } else {
                    for (s, o) in shared.iter_mut().zip(&one) {
                        *s = s.max(*o);
                    }
                }
            }
            for (bin, s) in shared.iter().enumerate() {
                gains[bin * count + n] = match self.kind {
                    DenoiseGain::Heavy => {
                        let held = (s * s).max(self.carry[bin] * self.release);
                        self.carry[bin] = held;
                        held.max(HEAVY_FLOOR)
                    }
                    _ => s.max(GAIN_FLOOR),
                };
            }
        }
        self.next = limit;
        Ready {
            first,
            count,
            gains,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One channel's gains, `frames` frames of `BINS` bins, bin after bin,
    /// with `cell(bin, frame)` in each.
    fn map(frames: usize, mut cell: impl FnMut(usize, usize) -> f32) -> Vec<f32> {
        (0..BINS * frames)
            .map(|i| cell(i / frames, i % frames))
            .collect()
    }

    /// The chain's gains for `raws` pushed as blocks of the given lengths,
    /// with the median of 9 frames.
    fn run(kind: DenoiseGain, raws: &[Vec<f32>], frames: usize, blocks: &[usize]) -> Vec<f32> {
        run_with(kind, 9, raws, frames, blocks)
    }

    /// [`run`] with a median of `window` frames.
    fn run_with(
        kind: DenoiseGain,
        window: usize,
        raws: &[Vec<f32>],
        frames: usize,
        blocks: &[usize],
    ) -> Vec<f32> {
        assert_eq!(blocks.iter().sum::<usize>(), frames);
        let mut chain = GainChain::new(kind, window);
        let mut out = vec![f32::NAN; BINS * frames];
        let mut from = 0;
        for (i, own) in blocks.iter().enumerate() {
            let ready = chain.push(raws, frames, from, *own, i + 1 == blocks.len());
            for bin in 0..BINS {
                for n in 0..ready.count {
                    out[bin * frames + ready.first as usize + n] =
                        ready.gains[bin * ready.count + n];
                }
            }
            from += own;
        }
        assert_eq!(chain.next as usize, frames);
        out
    }

    /// The smoothed gain (no floor) of one channel over the whole map,
    /// worked out the slow way: sort all 45 values, take the 23rd.
    fn slow_smooth(g: &[f32], frames: usize) -> Vec<f32> {
        slow_smooth_with(g, frames, 9)
    }

    /// [`slow_smooth`] with a median of 5 bins x `window` frames.
    fn slow_smooth_with(g: &[f32], frames: usize, window: usize) -> Vec<f32> {
        let half = (window / 2) as isize;
        let at = |bin: isize, t: isize| {
            let bin = bin.clamp(0, BINS as isize - 1) as usize;
            let t = t.clamp(0, frames as isize - 1) as usize;
            g[bin * frames + t].max(0.0)
        };
        map(frames, |bin, t| {
            let (bin, t) = (bin as isize, t as isize);
            let mut cells = Vec::new();
            for db in -2..=2 {
                for dt in -half..=half {
                    cells.push(at(bin + db, t + dt));
                }
            }
            cells.sort_by(f32::total_cmp);
            let middle = cells.len() / 2;
            let mut cap = 0.0f32;
            for db in -1..=1 {
                for dt in -1..=1 {
                    let (b, f) = (bin + db, t + dt);
                    if b >= 0 && b < BINS as isize && f >= 0 && f < frames as isize {
                        cap = cap.max(at(b, f));
                    }
                }
            }
            cells[middle].min(cap)
        })
    }

    #[test]
    fn a_lone_loud_cell_is_smoothed_away() {
        // One cell at 0.9 in a field of 0.3: the median round it is 0.3, and
        // its neighbours' median is 0.3 too.
        let frames = 40;
        let g = map(
            frames,
            |bin, t| if bin == 100 && t == 20 { 0.9 } else { 0.3 },
        );
        let out = run(DenoiseGain::Smoothed, &[g], frames, &[frames]);
        assert!(out.iter().all(|v| *v == 0.3));
    }

    #[test]
    fn a_band_keeps_its_gain_and_does_not_spread() {
        // Bins 100 to 104 at 0.9 for frames 10 to 29, the rest at 0.05. A
        // cell keeps 0.9 when more than half of its 5 x 9 window is in the
        // band.
        let frames = 40;
        let inside = |bin: usize, t: usize| (100..105).contains(&bin) && (10..30).contains(&t);
        let g = map(frames, |bin, t| if inside(bin, t) { 0.9 } else { 0.05 });
        let out = run(DenoiseGain::Smoothed, &[g], frames, &[frames]);
        let at = |bin: usize, t: usize| out[bin * frames + t];
        assert_eq!(at(102, 20), 0.9);
        assert_eq!(at(100, 20), 0.9, "3 of the window's 5 bins are in the band");
        assert_eq!(at(99, 20), GAIN_FLOOR, "2 of 5 bins are");
        assert_eq!(at(102, 10), 0.9, "5 of the window's 9 frames are");
        assert_eq!(at(102, 9), GAIN_FLOOR, "4 of 9 frames are");
        assert_eq!(at(50, 20), GAIN_FLOOR);
    }

    #[test]
    fn the_cap_is_the_largest_raw_gain_round_the_cell() {
        // A 3 x 3 patch of low gains in a field of 0.5. The median round its
        // middle cell is 0.5 (9 of 45 cells are low), but no cell round it
        // is over 0.2, so it stays at 0.2.
        let frames = 20;
        let g = map(frames, |bin, t| {
            if (9..=11).contains(&bin) && (9..=11).contains(&t) {
                if bin == 10 && t == 10 {
                    0.2
                } else {
                    0.0
                }
            } else {
                0.5
            }
        });
        let out = run(DenoiseGain::Smoothed, &[g], frames, &[frames]);
        assert_eq!(out[10 * frames + 10], 0.2);
        // The patch's corner has a neighbour at 0.5.
        assert_eq!(out[9 * frames + 9], 0.5);
    }

    #[test]
    fn the_edges_repeat_for_the_median_and_are_left_out_of_the_cap() {
        let frames = 30;
        let g = map(frames, |bin, t| ((bin * 7 + t * 13) % 17) as f32 / 17.0);
        let expected: Vec<f32> = slow_smooth(&g, frames)
            .iter()
            .map(|v| v.max(GAIN_FLOOR))
            .collect();
        let out = run(DenoiseGain::Smoothed, &[g], frames, &[frames]);
        assert_eq!(out, expected);
    }

    #[test]
    fn blocks_give_the_gains_of_the_map_held_whole() {
        // Pseudo-random gains from -0.2 to 1, cut into blocks of different
        // sizes: each must give the same gains as one block of everything.
        let frames = 3000;
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let g = map(frames, |_, _| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 * 1.2 - 0.2
        });
        let expected_smooth: Vec<f32> = slow_smooth(&g, frames);
        for kind in [
            DenoiseGain::Plain,
            DenoiseGain::Smoothed,
            DenoiseGain::Heavy,
        ] {
            let whole = run(kind, &[g.clone()], frames, &[frames]);
            if kind == DenoiseGain::Smoothed {
                let expected: Vec<f32> =
                    expected_smooth.iter().map(|v| v.max(GAIN_FLOOR)).collect();
                assert!(whole == expected);
            }
            for blocks in [
                &[1024, 1024, 952][..],
                &[1024, 1024, 948, 4],
                &[1024, 1024, 951, 1],
                &[2000, 1000],
            ] {
                assert!(
                    run(kind, &[g.clone()], frames, blocks) == whole,
                    "{kind:?} {blocks:?}"
                );
            }
        }
    }

    /// The whole chain worked out the slow way, with every median taken:
    /// each channel smoothed, the largest of the channels, then the floor,
    /// or the square, the release and the floor.
    fn slow_chain(
        kind: DenoiseGain,
        window: usize,
        channels: &[Vec<f32>],
        frames: usize,
    ) -> Vec<f32> {
        let mut s = vec![0.0f32; BINS * frames];
        for g in channels {
            for (s, v) in s.iter_mut().zip(slow_smooth_with(g, frames, window)) {
                *s = s.max(v);
            }
        }
        if kind == DenoiseGain::Smoothed {
            return s.iter().map(|v| v.max(GAIN_FLOOR)).collect();
        }
        let k = 10f64.powf(-RELEASE_DB_PER_FRAME / 20.0) as f32;
        let mut out = vec![0.0f32; BINS * frames];
        for bin in 0..BINS {
            let mut held = 0.0f32;
            for t in 0..frames {
                let v = s[bin * frames + t];
                held = (v * v).max(held * k);
                out[bin * frames + t] = held.max(HEAVY_FLOOR);
            }
        }
        out
    }

    #[test]
    fn skipping_the_median_of_quiet_cells_changes_no_gain() {
        // Mostly quiet gains round the skip's limits (0.1, and the gains
        // whose square is 0.01 or one step over it), with a loud patch now
        // and then, so that the release runs down into the quiet cells.
        let frames = 2500;
        // 0.1 squared is one step over 0.01, and 0.099_999_99 squared is
        // under it. Gains over 0.1 are rarer, so that many cells have no
        // neighbour over 0.1 and their median is skipped.
        let low = [0.0, 0.05, 0.099_999_99, 0.1];
        let high = [0.100_000_01, 0.100_000_02, 0.12];
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut quiet = |loud_every: usize| {
            map(frames, |bin, t| {
                let r = next();
                if (t / 7) % loud_every == 0 && (bin / 5) % 6 == 0 {
                    0.3 + (r >> 40) as f32 / (1u64 << 24) as f32 * 0.7
                } else if r % 20 == 0 {
                    high[(r >> 32) as usize % high.len()]
                } else {
                    low[(r >> 32) as usize % low.len()]
                }
            })
        };
        let (left, right) = (quiet(9), quiet(13));
        for kind in [DenoiseGain::Smoothed, DenoiseGain::Heavy] {
            for window in [9, 3] {
                for channels in [vec![left.clone()], vec![left.clone(), right.clone()]] {
                    let expected = slow_chain(kind, window, &channels, frames);
                    let out = run_with(kind, window, &channels, frames, &[1024, 1024, 452]);
                    let differ = out.iter().zip(&expected).filter(|(a, b)| a != b).count();
                    assert_eq!(
                        differ,
                        0,
                        "{kind:?}, {window} frames, {} channels",
                        channels.len()
                    );
                }
            }
        }
    }

    #[test]
    fn the_release_lets_the_gain_fall_by_0_16_db_a_frame() {
        // Every bin at 1 for frames 0 to 9, then 0. The smoothing keeps a
        // step where it is, so the heavy gain is 1 to frame 9, then falls by
        // 0.16 dB a frame until it reaches the floor of 0.01 (-40 dB).
        let frames = 400;
        let g = map(frames, |_, t| if t < 10 { 1.0 } else { 0.0 });
        let out = run(DenoiseGain::Heavy, &[g], frames, &[256, 144]);
        let k = 10f64.powf(-0.16 / 20.0) as f32;
        let mut expected = 1.0f32;
        for t in 0..frames {
            let s = if t < 10 { 1.0f32 } else { 0.0 };
            expected = (s * s).max(expected * k);
            assert_eq!(out[t], expected.max(HEAVY_FLOOR), "frame {t}");
            assert_eq!(out[200 * frames + t], out[t], "every bin the same");
        }
        let db = |v: f32| 20.0 * v.log10();
        assert!(
            (db(out[60]) - (-0.16 * 51.0)).abs() < 0.01,
            "{}",
            db(out[60])
        );
        assert_eq!(out[frames - 1], HEAVY_FLOOR);
        // It rises at once.
        let g = map(frames, |_, t| if t < 200 { 0.0 } else { 1.0 });
        let out = run(DenoiseGain::Heavy, &[g], frames, &[frames]);
        assert_eq!(out[199], HEAVY_FLOOR);
        assert_eq!(out[200], 1.0);
    }

    #[test]
    fn two_channels_take_the_larger_smoothed_gain() {
        // Left keeps bins 50 to 59, right keeps 150 to 159: each channel's
        // smoothed band survives, and a channel the same as the other gives
        // the one-channel gains.
        let frames = 60;
        let left = map(
            frames,
            |bin, _| if (50..60).contains(&bin) { 0.8 } else { 0.0 },
        );
        let right = map(
            frames,
            |bin, _| if (150..160).contains(&bin) { 0.6 } else { 0.0 },
        );
        for kind in [DenoiseGain::Smoothed, DenoiseGain::Heavy] {
            let both = run(kind, &[left.clone(), right.clone()], frames, &[30, 30]);
            let l = run(kind, &[left.clone()], frames, &[30, 30]);
            let r = run(kind, &[right.clone()], frames, &[30, 30]);
            assert!(both
                .iter()
                .zip(l.iter().zip(&r))
                .all(|(b, (l, r))| *b == l.max(*r)));
            assert!(
                run(kind, &[left.clone(), left.clone()], frames, &[60])
                    == run(kind, &[left.clone()], frames, &[60])
            );
        }
        // Each channel is smoothed by itself. Left is loud on every third
        // frame and right on the frames after those: each alone has 3 loud
        // frames of 9, so its median is 0, though together they have 6.
        let a = map(frames, |_, t| if t % 3 == 0 { 0.9 } else { 0.0 });
        let b = map(frames, |_, t| if t % 3 == 1 { 0.9 } else { 0.0 });
        let out = run(DenoiseGain::Smoothed, &[a, b], frames, &[frames]);
        // Away from the first and last frames, which repeat.
        for t in 8..frames - 8 {
            assert_eq!(out[100 * frames + t], GAIN_FLOOR, "frame {t}");
        }
    }

    #[test]
    fn three_frames_keep_a_call_two_frames_long() {
        // Bins 100 to 104 at 0.9 for frames 10 and 11 (256 samples, 0.7 ms
        // at 384 kHz), the rest at 0.05. A median of 5 bins x 3 frames has 15
        // cells and keeps 0.9 where at least 8 of them are in the band.
        let frames = 30;
        let inside = |bin: usize, t: usize| (100..105).contains(&bin) && (10..12).contains(&t);
        let g = map(frames, |bin, t| if inside(bin, t) { 0.9 } else { 0.05 });
        let out = run_with(DenoiseGain::Smoothed, 3, &[g.clone()], frames, &[frames]);
        let at = |bin: usize, t: usize| out[bin * frames + t];
        assert_eq!(at(102, 10), 0.9, "5 bins x 2 frames: 10 of 15 cells");
        assert_eq!(at(102, 11), 0.9, "10 of 15 cells");
        assert_eq!(at(101, 10), 0.9, "4 bins x 2 frames: 8 of 15 cells");
        assert_eq!(at(100, 10), GAIN_FLOOR, "3 bins x 2 frames: 6 of 15 cells");
        assert_eq!(at(102, 9), GAIN_FLOOR, "5 bins x 1 frame: 5 of 15 cells");
        assert_eq!(at(102, 12), GAIN_FLOOR, "5 of 15 cells");
        assert_eq!(at(50, 10), GAIN_FLOOR);
        // A median of 9 frames has 45 cells, only 10 of them in the band, so
        // it takes the call away.
        let out = run_with(DenoiseGain::Smoothed, 9, &[g], frames, &[frames]);
        assert_eq!(out[102 * frames + 10], GAIN_FLOOR);
    }

    #[test]
    fn three_frames_in_blocks_and_pieces_give_the_gains_of_the_map_held_whole() {
        // As `blocks_give_the_gains_of_the_map_held_whole`, with the median of
        // 3 frames, and also with the map pushed in pieces of 1 to 7 frames.
        let frames = 3000;
        let mut state = 0x6a09_e667_f3bc_c908u64;
        let g = map(frames, |_, _| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 * 1.2 - 0.2
        });
        let expected: Vec<f32> = slow_smooth_with(&g, frames, 3)
            .iter()
            .map(|v| v.max(GAIN_FLOOR))
            .collect();
        let whole = run_with(DenoiseGain::Smoothed, 3, &[g.clone()], frames, &[frames]);
        assert!(whole == expected);
        let mut pieces = Vec::new();
        let mut left = frames;
        for size in [1, 2, 3, 7, 1, 1, 5].iter().cycle() {
            let size = (*size).min(left);
            pieces.push(size);
            left -= size;
            if left == 0 {
                break;
            }
        }
        for kind in [DenoiseGain::Smoothed, DenoiseGain::Heavy] {
            let whole = run_with(kind, 3, &[g.clone()], frames, &[frames]);
            for blocks in [
                &[1024, 1024, 952][..],
                &[1024, 1024, 950, 2],
                &[1024, 1024, 951, 1],
                &[1, 1023, 1976],
                &pieces,
            ] {
                assert!(
                    run_with(kind, 3, &[g.clone()], frames, blocks) == whole,
                    "{kind:?} {:?}",
                    &blocks[..blocks.len().min(5)]
                );
            }
        }
    }

    #[test]
    fn the_number_of_frames_is_odd_and_from_3_to_31() {
        assert_eq!(check_smooth_frames(3), Ok(3));
        assert_eq!(check_smooth_frames(9), Ok(9));
        assert_eq!(check_smooth_frames(31), Ok(31));
        for bad in [0, 1, 2, 4, 8, 33, u32::MAX - 1] {
            assert!(check_smooth_frames(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn plain_is_the_floored_gain_and_the_larger_of_two_channels() {
        let frames = 10;
        let left = map(frames, |bin, t| (bin + t) as f32 / 300.0 - 0.1);
        let right = map(frames, |bin, _| if bin % 3 == 0 { 0.7 } else { -0.5 });
        let out = run(
            DenoiseGain::Plain,
            &[left.clone(), right.clone()],
            frames,
            &[4, 6],
        );
        for i in 0..BINS * frames {
            assert_eq!(
                out[i],
                left[i].max(GAIN_FLOOR).max(right[i].max(GAIN_FLOOR))
            );
        }
    }
}

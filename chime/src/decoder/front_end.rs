//! The front end: from input samples to limited baseband samples.
//!
//! The input is mixed down from 18 kHz to complex baseband, low-pass filtered and
//! decimated to about 8 kHz. The newest baseband samples wait in a ring for the timing
//! search and the trellises.
//!
//! # Two stages
//!
//! The low-pass filter that shapes the symbols is a 4th-order Butterworth at 1 kHz. Run at
//! 192 kHz on I and on Q, it and the mixer cost about 47 operations per input sample, which
//! was nearly all of the decoder's cost while it listens. So a cheap first stage decimates
//! to about 24 kHz before it, and the Butterworth runs at that rate.
//!
//! The first stage is a 3rd-order CIC filter, three moving averages of one output period
//! each, with the mixer folded into its taps: the taps for input sample `n` of the output
//! that ends at sample `m` are `c[m - n] e^(j w (m - n))`, and the output is then turned by
//! `e^(-j w m)`. Each input sample adds into the three outputs whose taps cover it, which is 6
//! multiply-adds. Its response is within 0.1 dB of flat over the 2 kHz band, so the symbols
//! keep the Butterworth's shape. Sounds that it lets through land on the carrier only when
//! they are within 1 kHz of the carrier plus or minus a multiple of the first stage's output
//! rate: at 192 kHz, 6, 30, 42, 54, 66, 78 and 90 kHz. It takes those down by at least 80 dB.
//!
//! # Why samples are limited
//!
//! A click or a bat call can be 40 dB above the chime in this band for a millisecond.
//! That is short, but one such bucket would outweigh the chime in the trellis metric,
//! the phase references and the echo filter for tens of milliseconds. So no baseband
//! sample may have more than 8 times the running mean power. Limiting changes a sample's
//! size only and keeps its phase. Noise and a chime in a room stay below that limit.
//!
//! The same limit rescues an overloaded input. When a loud melody clips the input, the
//! carrier gets through only near the melody's zero crossings, except between notes and
//! at the start of the melody, where it gets through in full, 20 to 30 dB louder. Without
//! the limit those moments outweigh the rest of the chime.
//!
//! A sound that stays above the limit for 5 ms is taken as the new level at once. So the
//! start of a loud chime is limited for 5 ms only, and the detector's timing, which rooms
//! are sensitive to, stays close to that of an unlimited signal. The ring holds the
//! samples as they arrived, and the timing search and the trellises apply the limit when
//! they read them. By then the running power has caught up with the chime, so these
//! see the first milliseconds of a chime whole. In a large room those are the only part
//! free of echoes.

use core::ptr::addr_of_mut;

use crate::biquad::Butterworth4;

use super::cplx::Cplx;
use super::{POWER_FLOOR, fill};
use crate::format::CARRIER_HZ;

const TARGET_BASEBAND_HZ: f64 = 8_000.0;

/// The first stage's output rate is at least this. Its nulls fall at multiples of that
/// rate, so a lower rate brings the sounds it lets through nearer the carrier.
const MIN_STAGE1_RATE_HZ: f64 = 20_000.0;

/// A first stage that decimates by less than this rejects too little near its nulls and
/// saves little, so the Butterworth runs at the input rate instead.
const MIN_STAGE1_DECIMATION: u32 = 4;

/// Largest decimation of the first stage, the size of its table of taps. 16 suits input
/// rates up to 384 kHz.
const MAX_STAGE1_DECIMATION: usize = 16;

/// Number of moving averages in the first stage. Each one adds about 27 dB of rejection
/// near its nulls.
const STAGE1_ORDER: usize = 3;

/// Cutoff of the 4th-order Butterworth low-pass filter on I and on Q, in Hz.
const FILTER_CUTOFF_HZ: f32 = 1_000.0;

/// Delay of the symbol edges through that filter, in seconds. It is the sum of
/// 1 / (Q * 2 pi * cutoff) over the filter's two sections.
const FILTER_DELAY_S: f64 = 0.000_416;

/// Added to the Butterworth's input so that its states do not decay to subnormal
/// numbers during digital silence, which are slow on some processors.
const SUBNORMAL_GUARD: f32 = 1e-20;

/// Outputs of the first stage with more power than this are treated as faults and replaced
/// by 0, as are NaNs. That is a sample of about 1e15 or more in the input. One NaN would
/// otherwise stay in the Butterworth's states for ever.
const MAX_STAGE1_POWER: f32 = 1e30;

/// Largest baseband power as a multiple of the running mean power.
const LIMIT_RATIO: f32 = 8.0;

/// Weight of each baseband sample in the running mean power, about 10 ms.
const LIMIT_WEIGHT: f32 = 1.0 / 80.0;

/// A sound stays above the limit for this many baseband samples, 5 ms, before it is taken
/// as the new level. The longest burst this has to catch is a ring of 0.3 ms through the
/// filter. An overloaded chime needs more than 4 ms, because the first moments of the
/// melody leave the carrier unchopped.
const LOUD_RUN_SAMPLES: u32 = 40;

/// Baseband samples kept for the timing search and for the first buckets of a walk.
/// This is a power of two so that an index wraps with a mask. It holds 58 ms at the
/// highest possible baseband rate, 8.8 kHz.
const RING_LEN: usize = 512;
const RING_MASK: usize = RING_LEN - 1;

/// The newest baseband samples, as they arrived.
pub(super) struct Ring {
    samples: [Cplx; RING_LEN],
    /// Number of samples pushed so far. A sample's index is its position in this count.
    count: u64,
}

impl Ring {
    /// Builds an empty ring at `this`.
    ///
    /// # Safety
    ///
    /// `this` must be valid for writes and aligned.
    pub(super) unsafe fn init_in_place(this: *mut Ring) {
        // SAFETY: the caller makes `this` valid for writes, and these are all of its fields.
        unsafe {
            fill(addr_of_mut!((*this).samples), Cplx::ZERO);
            addr_of_mut!((*this).count).write(0);
        }
    }

    fn push(&mut self, z: Cplx) {
        self.samples[self.count as usize & RING_MASK] = z;
        self.count += 1;
    }

    pub(super) fn count(&self) -> u64 {
        self.count
    }

    /// The sample at `index`, which must be among the newest `RING_LEN` samples.
    pub(super) fn at(&self, index: u64) -> Cplx {
        self.samples[index as usize & RING_MASK]
    }
}

/// A baseband sample from the front end.
pub(super) struct Baseband {
    /// The sample, limited relative to the running power before it.
    pub(super) limited: Cplx,
    /// The power of the sample before limiting.
    pub(super) raw_power: f32,
}

/// The decimation of the first stage for an input rate and a total decimation: the largest
/// factor of `decimation` that keeps the first stage's output rate at least
/// `MIN_STAGE1_RATE_HZ`, or 1 if that is below `MIN_STAGE1_DECIMATION`.
fn stage1_decimation(rate: f64, decimation: u32) -> u32 {
    let most = (rate / MIN_STAGE1_RATE_HZ) as u32;
    let best = (1..=most.min(MAX_STAGE1_DECIMATION as u32)).rev().find(|d| decimation % d == 0).unwrap_or(1);
    if best >= MIN_STAGE1_DECIMATION { best } else { 1 }
}

/// The weight of the input `k` samples before an output's last sample, in a first stage of
/// three moving averages of `d1` samples each: the number of ways to write `k` as a sum of one
/// step from each average, over `d1` cubed. Counted rather than convolved, so that building
/// a decoder needs no scratch arrays on the stack.
fn stage1_weight(k: usize, d1: usize) -> f64 {
    const { assert!(STAGE1_ORDER == 3, "stage1_weight counts three moving averages") };
    let mut ways = 0;
    for a in 0..d1 {
        for b in 0..d1 {
            ways += (k >= a + b && k - a - b < d1) as u32;
        }
    }
    ways as f64 / (d1 * d1 * d1) as f64
}

/// The first stage's taps with the mixer folded in, for input sample `position` of its
/// block of `d1`: the taps for the output that ends with this block, the next one and the one
/// after. Zero for a position past the block.
fn stage1_taps(position: usize, d1: usize, step: f64) -> [Cplx; STAGE1_ORDER] {
    let mut taps = [Cplx::ZERO; STAGE1_ORDER];
    if position >= d1 {
        return taps;
    }
    for (output, tap) in taps.iter_mut().enumerate() {
        let k = output * d1 + d1 - 1 - position;
        let weight = stage1_weight(k, d1);
        let angle = step * k as f64;
        *tap = Cplx { re: (weight * libm::cos(angle)) as f32, im: (weight * libm::sin(angle)) as f32 };
    }
    taps
}

/// The two stages of filters and decimation, and the running power for the limit.
pub(super) struct FrontEnd {
    /// The first stage's taps for each position in its block. See [`stage1_taps`].
    taps: [[Cplx; STAGE1_ORDER]; MAX_STAGE1_DECIMATION],
    /// The first stage's outputs that are under way: the one that ends with this block,
    /// the next and the one after.
    sums: [Cplx; STAGE1_ORDER],
    /// The first stage's decimation, and the position of the next input sample in its block.
    d1: u32,
    position: u32,
    /// `e^(-j w m)` for the end `m` of the first stage's next output, and its step.
    turn: Cplx,
    turn_step: Cplx,
    filter_i: Butterworth4,
    filter_q: Butterworth4,
    /// The second stage's decimation, and the number of first stage outputs since the last
    /// baseband sample.
    d2: u32,
    d2_count: u32,
    sample_rate: f64,
    baseband_rate: f64,
    /// Delay of the symbol edges through both stages, in input samples.
    delay: f64,
    running_power: f32,
    /// Number of baseband samples in a row that have been above the limit.
    loud_run: u32,
    input_samples: u64,
}

impl FrontEnd {
    /// Builds a front end at `this`, field by field, so that its table of taps never passes
    /// through the stack.
    ///
    /// # Safety
    ///
    /// `this` must be valid for writes and aligned.
    pub(super) unsafe fn init_in_place(this: *mut FrontEnd, sample_rate_hz: u32) {
        let rate = sample_rate_hz as f64;
        let decimation = libm::round(rate / TARGET_BASEBAND_HZ) as u32;
        let d1 = stage1_decimation(rate, decimation);
        let step = 2.0 * core::f64::consts::PI * CARRIER_HZ as f64 / rate;
        let filter = Butterworth4::lowpass((rate / d1 as f64) as f32, FILTER_CUTOFF_HZ);
        let turn = |angle: f64| Cplx { re: libm::cos(angle) as f32, im: libm::sin(angle) as f32 };
        // SAFETY: the caller makes `this` valid for writes, and these are all of its fields.
        unsafe {
            let rows = addr_of_mut!((*this).taps).cast::<[Cplx; STAGE1_ORDER]>();
            for position in 0..MAX_STAGE1_DECIMATION {
                rows.add(position).write(stage1_taps(position, d1 as usize, step));
            }
            addr_of_mut!((*this).sums).write([Cplx::ZERO; STAGE1_ORDER]);
            addr_of_mut!((*this).d1).write(d1);
            addr_of_mut!((*this).position).write(0);
            addr_of_mut!((*this).turn).write(turn(-step * (d1 - 1) as f64));
            addr_of_mut!((*this).turn_step).write(turn(-step * d1 as f64));
            addr_of_mut!((*this).filter_i).write(filter);
            addr_of_mut!((*this).filter_q).write(filter);
            addr_of_mut!((*this).d2).write(decimation / d1);
            addr_of_mut!((*this).d2_count).write(0);
            addr_of_mut!((*this).sample_rate).write(rate);
            addr_of_mut!((*this).baseband_rate).write(rate / decimation as f64);
            addr_of_mut!((*this).delay).write(FILTER_DELAY_S * rate + (STAGE1_ORDER * (d1 as usize - 1)) as f64 / 2.0);
            addr_of_mut!((*this).running_power).write(POWER_FLOOR);
            addr_of_mut!((*this).loud_run).write(0);
            addr_of_mut!((*this).input_samples).write(0);
        }
    }

    pub(super) fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub(super) fn baseband_rate(&self) -> f64 {
        self.baseband_rate
    }

    pub(super) fn input_samples(&self) -> u64 {
        self.input_samples
    }

    /// Takes input samples from `input` until a baseband sample is due, adds it to `ring`
    /// and returns it, with the number of input samples taken. Returns `None` with all of
    /// `input` taken when no baseband sample was due.
    ///
    /// This runs for every input sample, so the X8 patch runs it from ITCM. The first
    /// stage's sums stay in registers for the whole loop.
    #[inline(never)]
    pub(super) fn push(&mut self, input: &[f32], ring: &mut Ring) -> (usize, Option<Baseband>) {
        let d1 = self.d1 as usize;
        let [mut s0, mut s1, mut s2] = self.sums;
        let mut taken = 0;
        while taken < input.len() {
            let position = self.position as usize;
            let n = (d1 - position).min(input.len() - taken);
            for (&x, row) in input[taken..taken + n].iter().zip(&self.taps[position..position + n]) {
                s0 += row[0] * x;
                s1 += row[1] * x;
                s2 += row[2] * x;
            }
            taken += n;
            if position + n < d1 {
                self.position = (position + n) as u32;
                break;
            }
            self.position = 0;
            let out = s0;
            (s0, s1, s2) = (s1, s2, Cplx::ZERO);
            if let Some(baseband) = self.stage2(out, ring) {
                self.sums = [s0, s1, s2];
                self.input_samples += taken as u64;
                return (taken, Some(baseband));
            }
        }
        self.sums = [s0, s1, s2];
        self.input_samples += taken as u64;
        (taken, None)
    }

    /// Takes one input sample, as [`FrontEnd::push`] does a block. This is the same sum, without
    /// the block's setup.
    pub(super) fn push_one(&mut self, sample: f32, ring: &mut Ring) -> Option<Baseband> {
        self.input_samples += 1;
        let row = self.taps[self.position as usize];
        let [s0, s1, s2] = &mut self.sums;
        *s0 += row[0] * sample;
        *s1 += row[1] * sample;
        *s2 += row[2] * sample;
        self.position += 1;
        if self.position < self.d1 {
            return None;
        }
        self.position = 0;
        let out = self.sums[0];
        self.sums = [self.sums[1], self.sums[2], Cplx::ZERO];
        self.stage2(out, ring)
    }

    /// Takes one output of the first stage, and returns a baseband sample when one is due.
    #[inline(always)]
    fn stage2(&mut self, out: Cplx, ring: &mut Ring) -> Option<Baseband> {
        // Checking here rather than at each input sample costs an eighth as much. A fault
        // in one input sample loses the three outputs that it adds into, 125 us at 192 kHz.
        let out = if out.power() < MAX_STAGE1_POWER { out } else { Cplx::ZERO };
        let z = out.mul(self.turn);
        self.turn = self.turn.mul(self.turn_step);
        let i = self.filter_i.process_sample(z.re + SUBNORMAL_GUARD);
        let q = self.filter_q.process_sample(z.im + SUBNORMAL_GUARD);

        self.d2_count += 1;
        if self.d2_count < self.d2 {
            return None;
        }
        self.d2_count = 0;

        // Without this the turn's amplitude would drift through rounding. Near 1 this is
        // the first-order form of dividing by the square root of the power.
        self.turn = self.turn * (0.5 * (3.0 - self.turn.power()));

        let z = Cplx { re: i, im: q };
        let raw_power = z.power();
        let limit = self.power_limit();
        let (limited, used_power) = if raw_power > limit {
            self.loud_run += 1;
            if self.loud_run >= LOUD_RUN_SAMPLES {
                self.running_power = raw_power;
                (z, raw_power)
            } else {
                (z * libm::sqrtf(limit / raw_power), limit)
            }
        } else {
            self.loud_run = 0;
            (z, raw_power)
        };
        // The floor keeps this out of the subnormal numbers during digital silence.
        self.running_power = (self.running_power + (used_power - self.running_power) * LIMIT_WEIGHT).max(POWER_FLOOR);
        ring.push(z);
        Some(Baseband { limited, raw_power })
    }

    /// The largest power a baseband sample may have at the moment.
    pub(super) fn power_limit(&self) -> f32 {
        LIMIT_RATIO * self.running_power
    }

    /// The input sample index of a position in baseband samples.
    pub(super) fn input_position(&self, baseband_position: f64) -> f64 {
        let decimation = (self.d1 * self.d2) as f64;
        baseband_position * decimation + (decimation - 1.0) - self.delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_stage_decimates_to_about_24_khz_at_the_usual_rates() {
        assert_eq!(stage1_decimation(192_000.0, 24), 8);
        assert_eq!(stage1_decimation(96_000.0, 12), 4);
        assert_eq!(stage1_decimation(384_000.0, 48), 16);
        assert_eq!(stage1_decimation(48_000.0, 6), 1);
        assert_eq!(stage1_decimation(44_100.0, 6), 1);
    }

    #[test]
    fn the_first_stage_weights_are_three_moving_averages() {
        for d1 in [1, 4, 8, 16] {
            let mut convolved = std::vec![1.0f64];
            for _ in 0..STAGE1_ORDER {
                let mut next = std::vec![0.0; convolved.len() + d1 - 1];
                for (k, &weight) in convolved.iter().enumerate() {
                    for out in &mut next[k..k + d1] {
                        *out += weight / d1 as f64;
                    }
                }
                convolved = next;
            }
            for k in 0..STAGE1_ORDER * d1 {
                let expected = convolved.get(k).copied().unwrap_or(0.0);
                assert!((stage1_weight(k, d1) - expected).abs() < 1e-12, "d1 {d1}, k {k}");
            }
        }
    }

    /// A tone at the carrier plus `offset_hz`, through the front end, as baseband power over
    /// that of the carrier itself.
    fn relative_power_db(rate: u32, offset_hz: f64) -> f64 {
        let power = |hz: f64| {
            let mut slot = core::mem::MaybeUninit::uninit();
            // SAFETY: the slot is valid for writes, and `init_in_place` writes every field.
            let front_end = unsafe {
                FrontEnd::init_in_place(slot.as_mut_ptr(), rate);
                slot.assume_init_mut()
            };
            let mut ring = Ring { samples: [Cplx::ZERO; RING_LEN], count: 0 };
            let input: std::vec::Vec<f32> = (0..rate as usize / 4)
                .map(|n| libm::sin(2.0 * core::f64::consts::PI * hz * n as f64 / rate as f64) as f32)
                .collect();
            let (mut at, mut sum, mut count) = (0, 0.0f64, 0);
            while at < input.len() {
                let (taken, baseband) = front_end.push(&input[at..], &mut ring);
                at += taken;
                if let Some(baseband) = baseband
                    && at > input.len() / 2
                {
                    sum += baseband.raw_power as f64;
                    count += 1;
                }
            }
            sum / count as f64
        };
        let carrier = CARRIER_HZ as f64;
        10.0 * libm::log10(power(carrier + offset_hz) / power(carrier))
    }

    #[test]
    fn the_band_keeps_the_butterworth_shape() {
        for rate in [48_000, 96_000, 192_000] {
            let at_cutoff = relative_power_db(rate, 1_000.0);
            assert!((at_cutoff + 3.0).abs() < 0.2, "{rate} Hz: {at_cutoff:.2} dB at 1 kHz");
        }
    }

    #[test]
    fn sounds_that_alias_onto_the_carrier_are_80_db_down() {
        // At 192 kHz the first stage's nulls are at multiples of 24 kHz from the carrier. These
        // are tones at 5, 7, 41, 67 and 89 kHz, which land 1 kHz from the carrier.
        for offset in [-23_000.0, -25_000.0, 23_000.0, 49_000.0, 71_000.0] {
            let db = relative_power_db(192_000, offset);
            assert!(db < -80.0, "{offset} Hz from the carrier: {db:.1} dB");
        }
    }
}

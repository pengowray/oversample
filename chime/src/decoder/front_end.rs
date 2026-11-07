//! The front end: from input samples to limited baseband samples.
//!
//! The input is mixed down from 18 kHz to complex baseband, low-pass filtered and
//! decimated to about 8 kHz. The newest baseband samples wait in a ring for the timing
//! search and the trellises.
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

/// Cutoff of the 4th-order Butterworth low-pass filter on I and on Q, in Hz.
const FILTER_CUTOFF_HZ: f32 = 1_000.0;

/// Delay of the symbol edges through that filter, in seconds. It is the sum of
/// 1 / (Q * 2 pi * cutoff) over the filter's two sections.
const FILTER_DELAY_S: f64 = 0.000_416;

/// Added to the mixer output so that the filter states do not decay to subnormal
/// numbers during digital silence, which are slow on some processors.
const SUBNORMAL_GUARD: f32 = 1e-20;

/// Samples larger than this are treated as faults and replaced by 0, as are NaNs.
/// One NaN would otherwise stay in the filter states for ever.
const MAX_SAMPLE: f32 = 1e15;

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

/// The mixer, filters and decimator, and the running power for the limit.
pub(super) struct FrontEnd {
    mixer: Cplx,
    mixer_step: Cplx,
    filter_i: Butterworth4,
    filter_q: Butterworth4,
    decimation: u32,
    decimation_count: u32,
    sample_rate: f64,
    baseband_rate: f64,
    running_power: f32,
    /// Number of baseband samples in a row that have been above the limit.
    loud_run: u32,
    input_samples: u64,
}

impl FrontEnd {
    pub(super) fn new(sample_rate_hz: u32) -> FrontEnd {
        let rate = sample_rate_hz as f64;
        let decimation = libm::round(rate / TARGET_BASEBAND_HZ) as u32;
        let filter = Butterworth4::lowpass(sample_rate_hz as f32, FILTER_CUTOFF_HZ);
        let step = 2.0 * core::f64::consts::PI * CARRIER_HZ as f64 / rate;
        FrontEnd {
            mixer: Cplx::ONE,
            mixer_step: Cplx { re: libm::cos(step) as f32, im: libm::sin(step) as f32 },
            filter_i: filter,
            filter_q: filter,
            decimation,
            decimation_count: 0,
            sample_rate: rate,
            baseband_rate: rate / decimation as f64,
            running_power: POWER_FLOOR,
            loud_run: 0,
            input_samples: 0,
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

    /// Takes one input sample. When a baseband sample is due, adds it to `ring` and
    /// returns it.
    #[inline(never)]
    pub(super) fn push(&mut self, sample: f32, ring: &mut Ring) -> Option<Baseband> {
        self.input_samples += 1;
        let sample = if sample.abs() < MAX_SAMPLE { sample } else { 0.0 };

        // Multiplying by exp(-j w t) moves the carrier to 0 Hz and halves its
        // amplitude. The term at -36 kHz does not pass the filters.
        let i = self.filter_i.process_sample(sample * self.mixer.re + SUBNORMAL_GUARD);
        let q = self.filter_q.process_sample(SUBNORMAL_GUARD - sample * self.mixer.im);
        self.mixer = self.mixer.mul(self.mixer_step);

        self.decimation_count += 1;
        if self.decimation_count < self.decimation {
            return None;
        }
        self.decimation_count = 0;

        // Without this the mixer's amplitude would drift through rounding. Near 1
        // this is the first-order form of dividing by the square root of the power.
        self.mixer = self.mixer * (0.5 * (3.0 - self.mixer.power()));

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
        baseband_position * self.decimation as f64 + (self.decimation - 1) as f64
            - FILTER_DELAY_S * self.sample_rate
    }
}

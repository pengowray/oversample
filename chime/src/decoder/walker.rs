//! One reading of a chime: a trellis, its trackers and the test for the end of the walk.

use super::cplx::Cplx;
use super::front_end::Ring;
use super::round_to_u64;
use super::trackers::{BucketGrid, CarrierTracker, EchoFilter, echo_of};
use super::trellis::{Path, T0, Trellis, in_start_symbol, in_stop_symbol, starts_a_symbol};
use crate::UnreadableReason;
use crate::format::{MAX_MESSAGE_BYTES, START_SYMBOLS, STOP_SYMBOLS};

/// Weight of the older buckets in a path's phase reference, for each new bucket. A
/// short memory follows a moving phone better, and when the signal is clean it costs
/// nothing. A long memory gives a better reference when noise or echoes disturb the
/// buckets. Both of those lower the coherence that the timing search finds, so the
/// memory depends on that. 0.85 is about 7 buckets, or 17 ms, and 0.6 is about 2.5
/// buckets, or 6 ms.
const LONG_REFERENCE_MEMORY: f32 = 0.85;
const SHORT_REFERENCE_MEMORY: f32 = 0.6;
const COHERENCE_FOR_LONG_MEMORY: f32 = 0.72;
const COHERENCE_FOR_SHORT_MEMORY: f32 = 0.84;

/// A walk must confirm this many start symbols.
const MIN_START_SYMBOLS: u8 = 4;

/// The walk checks that the start symbols are there at this bucket.
const START_CHECK_BUCKET: u32 = 15;

/// A path with more start symbols than this is not reading a chime. A steady tone can
/// look like this.
const MAX_START_SYMBOLS: u8 = START_SYMBOLS as u8 + 8;

/// No walk lasts longer than this. The longest possible chime is 16 start symbols, 23
/// bytes of data symbols of 10 ms and 8 stop symbols, or about 3.4 s.
const MAX_WALK_BUCKETS: u32 = 1400;

/// Weight of each new bucket in the average bucket power, about 80 ms.
const POWER_WEIGHT: f32 = 1.0 / 32.0;

/// A bucket can raise the average as if its power were this many times the average,
/// so that a click does not raise the average for long.
const POWER_OUTLIER_LIMIT: f32 = 4.0;

/// The average bucket power is taken as the chime's level at this bucket.
const LEVEL_REFERENCE_BUCKET: u32 = 64;

/// A path can finish its stop symbols only if the average bucket power is above this
/// fraction of the chime's level. The average falls by about half in 60 ms.
const STOP_POWER_FRACTION: f32 = 0.6;

/// The signal counts as lost when the average bucket power falls below this fraction
/// of the chime's level, measured above the noise floor.
const LOST_FRACTION: f32 = 0.05;

/// A bucket counts as part of the signal when its power is above the noise by at least
/// this fraction of the highest average the walk has reached. After a chime is cut off,
/// the average takes about 240 ms to fall far enough to end the walk, and the buckets of
/// that time must not count as received.
const RECEIVED_FRACTION: f32 = 0.25;

/// How a trellis follows the carrier and uses its echo filter.
#[derive(Clone, Copy)]
pub(super) struct Tracking {
    /// Change of the carrier frequency estimate, in radians per bucket, for a phase error
    /// of 1 radian in the newest bucket, as a fraction of (1 - reference memory). The
    /// phase error is scaled by (1 - reference memory). A loop with the path's reference
    /// is critically damped when its gain is (1 - reference memory)^2 / 4.
    frequency_gain_fraction: f32,
    /// The echo filter's output is used only when the power of its error is below this
    /// fraction of the error of the direct signal alone, so only when the filter finds
    /// something in the signal that it can predict. Without this test, the noise in the
    /// taps costs more than 2 dB of sensitivity when there are no echoes.
    echo_useful_fraction: f32,
}

/// For a still phone, in a room or at low SNR. A critically damped loop lets the least
/// noise into the frequency estimate, and the echo filter's output is used whenever it
/// helps a little.
pub(super) const STEADY: Tracking = Tracking { frequency_gain_fraction: 0.25, echo_useful_fraction: 0.8 };

/// For a moving phone, and for an overloaded input. A phone that moves 2 cm at 2 Hz
/// changes the carrier by up to 13 Hz in a quarter of a second, and four times the loop
/// gain follows that. The echo filter's output is used only when it helps a lot, because
/// clipping makes the carrier's amplitude follow the melody, and the filter finds patterns
/// in that which are not echoes. With these settings alone, a chime in the large reverb
/// profile at +20 dB SNR reads about half the time, against nearly always with the steady
/// settings, which is why the steady trellis runs beside this one.
pub(super) const MOVING: Tracking = Tracking { frequency_gain_fraction: 1.0, echo_useful_fraction: 0.5 };

/// How a walk ended. The states say which path holds the result.
#[derive(Clone, Copy)]
pub(super) enum WalkEnd {
    Message(usize),
    Failed(UnreadableReason, usize),
    Silent,
}

/// A trellis with its trackers.
pub(super) struct Walker {
    tracking: Tracking,
    trellis: Trellis,
    carrier: CarrierTracker,
    grid: BucketGrid,
    echoes: EchoFilter,
    walk_start: f64,
    bucket_index: u32,
    previous_half: Cplx,
    reference_memory: f32,
    power_average: f32,
    level_reference: f32,
    level_sum: f32,
    level_count: u32,
    /// The highest value of `power_average` so far.
    peak_average: f32,
    previous_bucket_received: bool,
    /// `level_sum`, `level_count` and the best path's byte count at the latest pair of
    /// buckets in a row that counted as part of the signal.
    received_sum: f32,
    received_count: u32,
    received_bytes: u8,
    end: Option<WalkEnd>,
}

impl Walker {
    /// Builds a walker at `this`, field by field, so that its trellis does not pass through
    /// the stack.
    ///
    /// # Safety
    ///
    /// `this` must be valid for writes and aligned.
    pub(super) unsafe fn init_in_place(this: *mut Walker, tracking: Tracking, bucket_len: f64) {
        use core::ptr::addr_of_mut as field;
        // SAFETY: the caller makes `this` valid for writes, and these are all of its fields.
        unsafe {
            field!((*this).tracking).write(tracking);
            Trellis::init_in_place(field!((*this).trellis));
            field!((*this).carrier).write(CarrierTracker::new());
            field!((*this).grid).write(BucketGrid::new(bucket_len));
            field!((*this).echoes).write(EchoFilter::new());
            field!((*this).walk_start).write(0.0);
            field!((*this).bucket_index).write(0);
            field!((*this).previous_half).write(Cplx::ZERO);
            field!((*this).reference_memory).write(LONG_REFERENCE_MEMORY);
            field!((*this).power_average).write(0.0);
            field!((*this).level_reference).write(0.0);
            field!((*this).level_sum).write(0.0);
            field!((*this).level_count).write(0);
            field!((*this).peak_average).write(0.0);
            field!((*this).previous_bucket_received).write(false);
            field!((*this).received_sum).write(0.0);
            field!((*this).received_count).write(0);
            field!((*this).received_bytes).write(0);
            field!((*this).end).write(None);
        }
    }

    /// Starts a walk at a position in baseband samples.
    pub(super) fn begin(&mut self, start: f64, carrier_frequency: f32, coherence: f32) {
        let clean = (coherence - COHERENCE_FOR_LONG_MEMORY) / (COHERENCE_FOR_SHORT_MEMORY - COHERENCE_FOR_LONG_MEMORY);
        self.reference_memory =
            LONG_REFERENCE_MEMORY + (SHORT_REFERENCE_MEMORY - LONG_REFERENCE_MEMORY) * clean.clamp(0.0, 1.0);
        self.grid.begin(start);
        self.walk_start = start;
        self.bucket_index = 0;
        self.previous_half = Cplx::ZERO;
        self.carrier.begin(carrier_frequency);
        self.echoes.begin();
        self.power_average = 0.0;
        self.level_reference = 0.0;
        self.level_sum = 0.0;
        self.level_count = 0;
        self.peak_average = 0.0;
        self.previous_bucket_received = false;
        self.received_sum = 0.0;
        self.received_count = 0;
        self.received_bytes = 0;
        self.end = None;
        self.trellis.begin();
    }

    pub(super) fn end(&self) -> Option<WalkEnd> {
        self.end
    }

    pub(super) fn walk_start(&self) -> f64 {
        self.walk_start
    }

    pub(super) fn path(&self, state: usize) -> &Path {
        &self.trellis.paths()[state]
    }

    /// The path that holds this walker's message, if it read one.
    pub(super) fn message_path(&self) -> Option<&Path> {
        match self.end {
            Some(WalkEnd::Message(state)) => Some(self.path(state)),
            _ => None,
        }
    }

    /// Makes this walker end with the given message, for a test of what the decoder does
    /// with a reading.
    #[cfg(test)]
    pub(super) fn end_with_message(&mut self, message: &[u8]) {
        self.trellis.set_message_at_t0(message);
        self.end = Some(WalkEnd::Message(T0));
    }

    /// Makes this walker end with nothing, for a test of what the decoder does with a
    /// reading.
    #[cfg(test)]
    pub(super) fn finish_silently(&mut self) {
        self.end = Some(WalkEnd::Silent);
    }

    /// The mean bucket power over the walk.
    pub(super) fn mean_power(&self) -> f32 {
        self.level_sum / self.level_count.max(1) as f32
    }

    /// The mean bucket power up to the point where the signal stopped.
    pub(super) fn received_power(&self) -> f32 {
        if self.received_count == 0 {
            return self.mean_power();
        }
        self.received_sum / self.received_count as f32
    }

    /// The best path's complete bytes at the point where the signal stopped.
    pub(super) fn received_bytes(&self) -> usize {
        self.received_bytes as usize
    }

    /// The chime's level, once the walk has gone far enough to measure it.
    pub(super) fn level_reference(&self) -> f32 {
        self.level_reference
    }

    /// Processes the next bucket if all of its samples have arrived. `power_limit` is the
    /// limit on the power of a sample, and `noise_bucket` is the noise floor of a bucket.
    pub(super) fn advance(&mut self, ring: &Ring, power_limit: f32, noise_bucket: f32) {
        if self.end.is_some() {
            return;
        }
        if let Some(halves) = self.next_bucket(ring, power_limit) {
            self.end = self.step(halves, noise_bucket);
        }
    }

    /// Returns the two halves of the next 2.5 ms bucket if all of its samples have arrived.
    fn next_bucket(&self, ring: &Ring, power_limit: f32) -> Option<(Cplx, Cplx)> {
        let bucket_len = self.grid.bucket_len();
        let edge = self.grid.edge();
        let start = round_to_u64(edge);
        let middle = round_to_u64(edge + 0.5 * bucket_len);
        let end = round_to_u64(edge + bucket_len);
        if end > ring.count() {
            return None;
        }
        let mut first_half = Cplx::ZERO;
        for index in start..middle {
            first_half += ring.at(index).limited(power_limit);
        }
        let mut second_half = Cplx::ZERO;
        for index in middle..end {
            second_half += ring.at(index).limited(power_limit);
        }
        Some((first_half, second_half))
    }

    /// Processes one bucket. Returns how the walk ended, if it did.
    fn step(&mut self, (first_half, second_half): (Cplx, Cplx), noise_bucket: f32) -> Option<WalkEnd> {
        let first_half = self.carrier.remove_from(first_half);
        let second_half = self.carrier.remove_from(second_half);
        let bucket = first_half + second_half;
        let distance = (self.grid.edge() - self.walk_start) as f32;
        let memory = self.reference_memory;

        let echo_taps = *self.echoes.taps();
        let best_state = self.trellis.step(bucket, &echo_taps, distance, memory);
        let best_path = self.trellis.paths()[best_state];
        let half_echo = echo_of(&echo_taps, best_path.history >> 1) * 0.5;

        let frequency_gain = self.tracking.frequency_gain_fraction * (1.0 - memory);
        self.carrier.update(bucket - half_echo * 2.0, best_path.reference, frequency_gain);
        self.echoes.update(bucket, best_path.history, self.bucket_index, self.tracking.echo_useful_fraction);

        if self.bucket_index > 0 {
            self.grid.advance(self.previous_half, first_half - half_echo, starts_a_symbol(best_state));
        } else {
            self.grid.advance(Cplx::ZERO, Cplx::ZERO, false);
        }
        self.previous_half = second_half - half_echo;

        self.bucket_index += 1;
        self.update_power(bucket, best_path.byte_count, noise_bucket);
        self.check_end(best_state, noise_bucket)
    }

    fn update_power(&mut self, bucket: Cplx, best_path_bytes: u8, noise_bucket: f32) {
        let samples = self.grid.bucket_len();
        let power = bucket.power() / (samples * samples) as f32;
        self.level_sum += power;
        self.level_count += 1;

        let limited = if self.bucket_index > 8 {
            power.min(POWER_OUTLIER_LIMIT * self.power_average)
        } else {
            power
        };
        let weight = (1.0 / self.bucket_index as f32).max(POWER_WEIGHT);
        self.power_average += (limited - self.power_average) * weight;
        if self.bucket_index == LEVEL_REFERENCE_BUCKET {
            self.level_reference = self.power_average;
        }

        // Noise alone can reach the limit in one bucket, but it rarely does so twice in a row.
        self.peak_average = self.peak_average.max(self.power_average);
        let received = power >= noise_bucket + RECEIVED_FRACTION * (self.peak_average - noise_bucket);
        if received && self.previous_bucket_received {
            self.received_sum = self.level_sum;
            self.received_count = self.level_count;
            self.received_bytes = best_path_bytes;
        }
        self.previous_bucket_received = received;
    }

    fn check_end(&self, best_state: usize, noise_bucket: f32) -> Option<WalkEnd> {
        if self.trellis.stop_is_complete() {
            return Some(WalkEnd::Message(T0));
        }

        let best = &self.trellis.paths()[best_state];
        if self.bucket_index == START_CHECK_BUCKET && best.start_symbols < MIN_START_SYMBOLS {
            return Some(WalkEnd::Silent);
        }
        if best.byte_count as usize > MAX_MESSAGE_BYTES || self.bucket_index >= MAX_WALK_BUCKETS {
            return Some(WalkEnd::Failed(UnreadableReason::TooManyBytes, best_state));
        }
        // A path in the stop symbols can be following noise after the signal has gone, so
        // the power must also still be there, averaged over the last stop symbols.
        let stopping = in_stop_symbol(best_state);
        if stopping
            && best.stop_symbols >= STOP_SYMBOLS as u8
            && (self.power_average > STOP_POWER_FRACTION * self.level_reference
                || best.stop_symbols >= 2 * STOP_SYMBOLS as u8)
        {
            let reason = if best.crc_matches() { UnreadableReason::MessageRefused } else { UnreadableReason::BadCrc };
            return Some(WalkEnd::Failed(reason, best_state));
        }
        if in_start_symbol(best_state) && best.start_symbols > MAX_START_SYMBOLS {
            return Some(WalkEnd::Silent);
        }
        if self.level_reference > 0.0 {
            let limit = noise_bucket + LOST_FRACTION * (self.level_reference - noise_bucket);
            if self.power_average < limit {
                // Stop symbols that end early still end a message. The longest message
                // can end without any.
                let could_be_complete = stopping || best.byte_count as usize == MAX_MESSAGE_BYTES;
                if could_be_complete && best.is_good_message() {
                    return Some(WalkEnd::Message(best_state));
                }
                // Stop symbols of an earlier chime can start a walk, and they end like
                // this with no bytes. There is nothing to report in that case.
                if best.byte_count == 0 {
                    return Some(WalkEnd::Silent);
                }
                return Some(WalkEnd::Failed(UnreadableReason::SignalLost, best_state));
            }
        }
        None
    }
}

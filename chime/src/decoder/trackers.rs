//! The trackers. Three things change during a chime, and the best path's decisions drive
//! a tracker for each:
//!
//! - The carrier frequency, because of clock errors and a moving phone.
//! - The symbol timing, because of clock errors and because the encoder rounds symbol
//!   lengths up to whole samples.
//! - The echoes. In a room, each reflection arrives with its own delay and phase. After
//!   a polarity flip the buckets therefore hold a mix of old and new polarity for up
//!   to tens of milliseconds, and a reference for the signal alone fails even at high
//!   SNR. An adaptive filter learns the echoes from the data, and each path removes
//!   the echoes that its own past polarities would cause.

use super::cplx::Cplx;

// -- Carrier ------------------------------------------------------------------

/// Largest carrier frequency estimate, in radians per bucket: about 50 Hz.
const MAX_FREQUENCY: f32 = 0.8;

/// The carrier frequency and phase that the walk removes from each bucket.
pub(super) struct CarrierTracker {
    phasor: Cplx,
    frequency: f32,
}

impl CarrierTracker {
    pub(super) const fn new() -> CarrierTracker {
        CarrierTracker {
            phasor: Cplx::ONE,
            frequency: 0.0,
        }
    }

    pub(super) fn begin(&mut self, frequency: f32) {
        self.phasor = Cplx::ONE;
        self.frequency = frequency;
    }

    /// Removes the tracked carrier from a sum of baseband samples.
    pub(super) fn remove_from(&self, sum: Cplx) -> Cplx {
        sum.mul_conj(self.phasor)
    }

    /// Updates the frequency and phase from the best path. `left_over` is the newest
    /// bucket without echoes, and `reference` is the path's reference, which now includes
    /// that bucket.
    ///
    /// The imaginary part below comes from the older buckets in the reference, because
    /// the bucket has no imaginary part relative to itself. With the reference's own
    /// tracking of the phase, the frequency makes a second-order loop. `gain` is the
    /// change of the frequency, in radians per bucket, for a phase error of 1 radian.
    pub(super) fn update(&mut self, left_over: Cplx, reference: Cplx, gain: f32) {
        let phase_error = left_over.mul_conj(reference).im / reference.power().max(1e-30);
        let frequency = self.frequency + gain * phase_error.clamp(-1.0, 1.0);
        let frequency = frequency.clamp(-MAX_FREQUENCY, MAX_FREQUENCY);
        self.frequency = frequency;
        // cos and sin of a small angle, followed by the same correction of the amplitude
        // as the mixer. A small error in the step only looks like a small frequency error.
        let step = Cplx {
            re: 1.0 - 0.5 * frequency * frequency,
            im: frequency,
        };
        let phasor = self.phasor.mul(step);
        self.phasor = phasor * (0.5 * (3.0 - phasor.power()));
    }
}

// -- Timing -------------------------------------------------------------------

/// Weight of the older buckets in the timing error, for each new bucket.
const TIMING_MEMORY: f32 = 0.9;

/// Fraction of the timing error corrected at each polarity flip. At -5 dB SNR one flip
/// measures the timing to about 5 samples. This gain brings that to about 1 sample (125
/// us), and it follows the 44.1 kHz encoder's drift of 0.09 samples per data 0 with a
/// lag of about 2 samples at most.
const TIMING_GAIN: f32 = 0.05;

/// Largest correction at one flip, in baseband samples, so that a click does not move
/// the grid far.
const TIMING_MAX_STEP: f32 = 0.25;

/// The grid of buckets.
pub(super) struct BucketGrid {
    bucket_len: f64,
    edge: f64,
    error: f32,
    power: f32,
}

impl BucketGrid {
    pub(super) const fn new(bucket_len: f64) -> BucketGrid {
        BucketGrid {
            bucket_len,
            edge: 0.0,
            error: 0.0,
            power: 0.0,
        }
    }

    pub(super) fn begin(&mut self, start: f64) {
        self.edge = start;
        self.error = 0.0;
        self.power = 0.0;
    }

    pub(super) fn bucket_len(&self) -> f64 {
        self.bucket_len
    }

    /// The position of the next bucket's first sample, in baseband samples.
    pub(super) fn edge(&self) -> f64 {
        self.edge
    }

    /// Moves the grid on by one bucket, with a correction from the timing error.
    ///
    /// The encoder rounds symbol lengths up to whole samples, so a chime rendered at
    /// 44.1 kHz grows by 11 us for each data 0, and a 21-byte chime is about 2 ms
    /// longer than its nominal length. Clock errors add to this.
    ///
    /// The timing error is that of Gardner's detector, and `flip` says whether the
    /// best path has a polarity flip on the edge. `before` and `after` are the sums
    /// over the half buckets on each side of the edge. When the true flip is late by
    /// d samples, before + after = 2 d a in the direction of before - after, whose
    /// size is 2 w a for half buckets of w samples and carrier amplitude a. An edge
    /// with no flip holds no timing information, only noise, so it does not count.
    pub(super) fn advance(&mut self, before: Cplx, after: Cplx, flip: bool) {
        let mut step = 0.0;
        if flip {
            let sum = before + after;
            let difference = before - after;
            self.error = self.error * TIMING_MEMORY + sum.dot(difference);
            self.power = self.power * TIMING_MEMORY + difference.power();
            let half_len = 0.5 * self.bucket_len as f32;
            step = TIMING_GAIN * half_len * self.error / self.power.max(1e-30);
            step = step.clamp(-TIMING_MAX_STEP, TIMING_MAX_STEP);
        }
        self.edge += self.bucket_len + step as f64;
    }
}

// -- Echoes -------------------------------------------------------------------

/// Number of earlier buckets whose echoes the trellis models: 30 ms.
pub(super) const ECHO_TAPS: usize = 12;

/// Step size of the adaptive filter that learns the echoes, for each new bucket.
const ECHO_STEP: f32 = 0.04;

/// Step size for the filter's first tap, the direct signal, which must follow the
/// phase of the carrier.
const DIRECT_TAP_STEP: f32 = 0.15;

/// Noise power in each tap of that filter, as a fraction of the power of its error.
const ECHO_TAP_NOISE: f32 = 0.6 * ECHO_STEP;

/// Taps with less power than this many times the noise in a tap are set to zero, and
/// the power of larger taps is reduced by that amount.
const ECHO_TAP_MARGIN: f32 = 2.0;

/// Taps below this power relative to the direct signal are set to zero whatever the
/// noise is. At high SNR the filter's error is not noise but small differences between
/// the real symbol shapes and the model, and these make the taps wander.
const ECHO_TAP_FLOOR: f32 = 0.004;

/// Weight of each new bucket in the power of the filter's error.
const ECHO_ERROR_WEIGHT: f32 = 0.02;

/// Number of recent buckets over which the filter compares polarity patterns: 50 ms.
const PATTERN_WINDOW: u32 = 20;

/// Two polarity patterns count as the same or opposite when the size of
/// (buckets that agree - buckets that disagree) / buckets in the window is above this.
const PATTERN_SAME: f32 = 0.75;

// The history of a path must hold the window at the largest lag.
const _: () = assert!(PATTERN_WINDOW as usize + ECHO_TAPS <= 32);

/// The echoes in a bucket from the earlier buckets of a path. Bit i of `history` is
/// the polarity of the bucket i + 1 buckets ago.
pub(super) fn echo_of(taps: &[Cplx; ECHO_TAPS], history: u32) -> Cplx {
    let mut echo = Cplx::ZERO;
    for (index, &tap) in taps.iter().enumerate() {
        if (history >> index) & 1 == 1 {
            echo += tap;
        } else {
            echo = echo - tap;
        }
    }
    echo
}

/// Number of taps that may learn from the recent polarities of the best path, counted
/// from the shortest lag up.
///
/// A tap can learn only when its polarity pattern differs from that of the direct signal
/// and of the taps with shorter lags. If two patterns are the same or exact opposites,
/// the filter cannot tell those two taps apart. A change that belongs to one goes into
/// both, and both are wrong when the patterns differ again.
///
/// This matters most for a moving phone. A run of equal symbols makes the polarities
/// periodic. The two zero bytes of a time zone of 0 are 56 buckets of ++--++--, in which
/// the taps at even lags have the pattern of the direct signal. The direct signal's phase
/// moves during the run, the taps then take on part of the direct signal, and the message
/// breaks when the run ends. Start symbols are the same case: in their pattern of +++---,
/// only the taps at lags 1 and 2 can learn.
///
/// Two taps whose lags differ by m buckets have the same or opposite patterns when the
/// history matches itself, or its opposite, m buckets earlier. So the first m at which
/// that happens stops the tap at lag m and all taps with longer lags.
fn taps_that_can_learn(history: u32) -> usize {
    let mask = (1u32 << PATTERN_WINDOW) - 1;
    for lag in 1..=ECHO_TAPS {
        let disagreeing = ((history ^ (history >> lag)) & mask).count_ones();
        let similarity = (PATTERN_WINDOW as f32 - 2.0 * disagreeing as f32) / PATTERN_WINDOW as f32;
        if similarity.abs() > PATTERN_SAME {
            return lag - 1;
        }
    }
    ECHO_TAPS
}

/// The adaptive filter that learns the echoes.
pub(super) struct EchoFilter {
    /// The filter's raw estimates of the echoes.
    estimates: [Cplx; ECHO_TAPS],
    /// The same estimates with noise reduced, as the trellis uses them.
    taps: [Cplx; ECHO_TAPS],
    error_power: f32,
    direct_error_power: f32,
    direct_tap: Cplx,
}

impl EchoFilter {
    pub(super) const fn new() -> EchoFilter {
        EchoFilter {
            estimates: [Cplx::ZERO; ECHO_TAPS],
            taps: [Cplx::ZERO; ECHO_TAPS],
            error_power: 0.0,
            direct_error_power: 0.0,
            direct_tap: Cplx::ZERO,
        }
    }

    pub(super) fn begin(&mut self) {
        *self = EchoFilter::new();
    }

    pub(super) fn taps(&self) -> &[Cplx; ECHO_TAPS] {
        &self.taps
    }

    /// Updates the filter from the best path's polarities.
    ///
    /// This is an ordinary adaptive filter from those polarities to the buckets. Its
    /// first tap is the direct signal, which is a separate estimate from the path's
    /// reference. If the filter used the reference, the two could share the direct
    /// signal in any proportion and the taps could drift until they broke the message.
    ///
    /// `bucket_index` counts the buckets of the walk, and `useful_fraction` is the share
    /// of the direct signal's error that the filter's error must stay below for the
    /// filter's output to be used.
    pub(super) fn update(
        &mut self,
        bucket: Cplx,
        history: u32,
        bucket_index: u32,
        useful_fraction: f32,
    ) {
        let polarity_of = |age: usize| {
            if (history >> age) & 1 == 1 {
                1.0f32
            } else {
                -1.0
            }
        };

        let mut predicted = self.direct_tap * polarity_of(0);
        for (tap, &estimate) in self.estimates.iter().enumerate() {
            predicted += estimate * polarity_of(tap + 1);
        }
        let error = bucket - predicted;
        let direct_error = bucket - self.direct_tap * polarity_of(0);
        let count = bucket_index.max(1) as f32;
        let error_weight = (1.0 / count).max(ECHO_ERROR_WEIGHT);
        self.error_power += (error.power() - self.error_power) * error_weight;
        self.direct_error_power += (direct_error.power() - self.direct_error_power) * error_weight;
        self.direct_tap += error * (DIRECT_TAP_STEP * polarity_of(0));

        let tap_noise = ECHO_TAP_NOISE * self.error_power;
        let tap_limit = (ECHO_TAP_MARGIN * tap_noise).max(ECHO_TAP_FLOOR * self.direct_tap.power());
        let learning = taps_that_can_learn(history);
        for tap in 0..ECHO_TAPS {
            let estimate = &mut self.estimates[tap];
            if tap < learning {
                *estimate += error * (ECHO_STEP * polarity_of(tap + 1));
            }
            let power = estimate.power();
            let gain = if power > tap_limit {
                1.0 - tap_limit / power
            } else {
                0.0
            };
            self.taps[tap] = *estimate * gain;
        }
        if self.error_power > useful_fraction * self.direct_error_power {
            self.taps = [Cplx::ZERO; ECHO_TAPS];
        }
    }
}

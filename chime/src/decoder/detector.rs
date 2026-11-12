//! The detector: a cheap test for start symbols, and the noise floor.
//!
//! The detector compares the signal with itself 7.5 ms earlier. During the start symbols
//! the two have opposite polarity whatever the symbol timing is. This comparison has
//! many false alarms in noise, which the timing search then rejects.
//!
//! The detector also waits after a chime, and it measures the band level and the noise
//! floor.
//!
//! # Waiting after a chime
//!
//! The last stop symbols and their echoes look like start symbols. A fixed wait is not
//! enough in a hard room: at an RT60 of 500 ms the echoes of a chime are still far above
//! a quiet noise floor 300 ms later. So after the fixed wait the detector goes on waiting
//! until one of these happens:
//!
//! - The level falls to twice the noise floor, or 90 dB below the chime.
//! - The level rises to 10 times its lowest value since the chime. That is a new sound,
//!   perhaps the next chime, which must not be missed. The recorder setup page leaves
//!   1 s between chimes.
//! - 0.7 s have passed.

use super::POWER_FLOOR;
use super::cplx::Cplx;

/// The detector works on sums over 2.5 ms, taken every 1.25 ms. Each sum has a
/// fifth of the noise power of a baseband sample in the 2 kHz band, which keeps rho
/// large at low SNR.
const HALF_BUCKET_S: f64 = 0.001_25;

/// Number of 1.25 ms steps in the 7.5 ms lag.
const LAG_STEPS: usize = 6;

/// Number of 1.25 ms steps in one detector block of 5 ms.
const STEPS_PER_BLOCK: u32 = 4;

/// Number of 5 ms blocks in the detector's window of 50 ms.
const WINDOW_BLOCKS: usize = 10;

/// rho above this starts a timing search. The value is low because the timing
/// search rejects false detections.
const RHO_TRIGGER: f32 = 0.33;

/// rho below this in size means that the window has no chime or tone in it, so its
/// power counts as the noise floor.
const RHO_QUIET: f32 = 0.25;

/// Number of 5 ms blocks between the trigger and the timing search, so that the
/// window holds several symbols of a loud chime that triggers early.
const SETTLE_BLOCKS: u32 = 6;

/// Weight of each new block in the noise floor estimates, about 100 ms.
const NOISE_WEIGHT: f32 = 0.05;

/// Power below this is digital silence, not noise. It is about -250 dBFS, far below what
/// any converter produces, and well above the power of the front end's subnormal guard.
const DIGITAL_SILENCE: f32 = 1e-25;

/// Lowest weight of each new block in the band level estimate, about 1 s.
const BAND_WEIGHT: f32 = 0.005;

/// Longest wait for the echoes to fade after the fixed wait, in 5 ms blocks: 0.7 s.
const TAIL_BLOCKS: u32 = 140;

/// Weight of each new block in the level of the echoes, about 20 ms.
const TAIL_WEIGHT: f32 = 0.25;

/// The echoes have faded when their power falls this far below the chime's, in dB.
const TAIL_END_DB: f32 = -90.0;

/// The echoes have also faded when their power falls below this many times the noise.
const TAIL_NOISE_RATIO: f32 = 2.0;

/// A rise to this many times the lowest level of the echoes is a new sound: 10 dB.
const TAIL_RISE_RATIO: f32 = 10.0;

pub(super) struct Detector {
    half_bucket_len: u32,
    half_bucket_count: u32,
    half_bucket_sum: Cplx,
    previous_half_bucket: Cplx,
    lag_sums: [Cplx; LAG_STEPS],
    lag_index: usize,
    steps_in_block: u32,
    block_p: f32,
    block_q: f32,
    block_e: f32,
    block_band: f32,
    window_p: [f32; WINDOW_BLOCKS],
    window_q: [f32; WINDOW_BLOCKS],
    window_e: [f32; WINDOW_BLOCKS],
    window_band: [f32; WINDOW_BLOCKS],
    window_index: usize,
    /// Blocks in the window since it was cleared, up to the size of the window.
    window_blocks: usize,

    /// A trigger fired, and the timing search runs when this many blocks have passed.
    settling_blocks: u32,

    /// Blocks of the fixed wait that are left.
    fixed_wait_blocks: u32,
    /// Blocks that are left of the wait for the echoes to fade.
    tail_blocks: u32,
    /// Bucket power of the walk that started the wait.
    tail_chime_power: f32,
    tail_power: f32,
    tail_lowest: f32,

    // Level estimates. Both noise floors and `band_power` are baseband power per
    // sample. `noise_bucket` is the same for a sum over 2.5 ms, divided by the
    // squared number of samples in the sum.
    band_power: f32,
    band_blocks: u32,
    noise_band: f32,
    noise_bucket: f32,
    noise_blocks: u32,
}

impl Detector {
    pub(super) fn new(baseband_rate: f64) -> Detector {
        Detector {
            half_bucket_len: libm::round(HALF_BUCKET_S * baseband_rate) as u32,
            half_bucket_count: 0,
            half_bucket_sum: Cplx::ZERO,
            previous_half_bucket: Cplx::ZERO,
            lag_sums: [Cplx::ZERO; LAG_STEPS],
            lag_index: 0,
            steps_in_block: 0,
            block_p: 0.0,
            block_q: 0.0,
            block_e: 0.0,
            block_band: 0.0,
            window_p: [0.0; WINDOW_BLOCKS],
            window_q: [0.0; WINDOW_BLOCKS],
            window_e: [0.0; WINDOW_BLOCKS],
            window_band: [0.0; WINDOW_BLOCKS],
            window_index: 0,
            window_blocks: 0,
            settling_blocks: 0,
            fixed_wait_blocks: 0,
            tail_blocks: 0,
            tail_chime_power: 0.0,
            tail_power: 0.0,
            tail_lowest: 0.0,
            band_power: 0.0,
            band_blocks: 0,
            noise_band: 0.0,
            noise_bucket: 0.0,
            noise_blocks: 0,
        }
    }

    pub(super) fn band_power(&self) -> f32 {
        self.band_power
    }

    pub(super) fn noise_band(&self) -> f32 {
        self.noise_band
    }

    pub(super) fn noise_bucket(&self) -> f32 {
        self.noise_bucket
    }

    /// False until the detector has seen some noise before a chime, and again after
    /// digital silence, which has no noise to measure.
    pub(super) fn noise_measured(&self) -> bool {
        self.noise_band > 0.0
    }

    /// The lag product over the window. Its phase is the phase change of the signal
    /// over one start symbol, apart from the polarity flip.
    pub(super) fn lag_product(&self) -> (f32, f32) {
        (self.window_p.iter().sum(), self.window_q.iter().sum())
    }

    /// Takes one baseband sample. `walking` means that a walk is under way, which pauses
    /// the trigger and the noise floor. Returns true when the timing search should run.
    pub(super) fn push(&mut self, z: Cplx, raw_power: f32, walking: bool) -> bool {
        self.half_bucket_sum += z;
        self.block_band += raw_power;
        self.half_bucket_count += 1;
        if self.half_bucket_count < self.half_bucket_len {
            return false;
        }
        self.half_bucket_count = 0;

        let sum = self.half_bucket_sum + self.previous_half_bucket;
        self.previous_half_bucket = self.half_bucket_sum;
        self.half_bucket_sum = Cplx::ZERO;
        let lagged = self.lag_sums[self.lag_index];
        self.lag_sums[self.lag_index] = sum;
        self.lag_index = (self.lag_index + 1) % LAG_STEPS;

        // The minus sign makes rho positive when the polarity has flipped.
        let product = sum.mul_conj(lagged);
        self.block_p -= product.re;
        self.block_q -= product.im;
        self.block_e += 0.5 * (sum.power() + lagged.power());

        self.steps_in_block += 1;
        if self.steps_in_block == STEPS_PER_BLOCK {
            self.steps_in_block = 0;
            return self.close_block(walking);
        }
        false
    }

    fn close_block(&mut self, walking: bool) -> bool {
        let samples = (self.half_bucket_len * STEPS_PER_BLOCK) as f32;
        let samples_in_sum = (self.half_bucket_len * 2) as f32;
        let block_band = self.block_band / samples;
        let newest_bucket =
            self.block_e / (STEPS_PER_BLOCK as f32 * samples_in_sum * samples_in_sum);

        // The oldest block in the window is the one about to be replaced. The noise
        // floor uses it because rho has had 50 ms to rise if a chime was starting.
        let oldest = self.window_index;
        let oldest_band = self.window_band[oldest] / samples;
        let oldest_bucket =
            self.window_e[oldest] / (STEPS_PER_BLOCK as f32 * samples_in_sum * samples_in_sum);
        let window_was_full = self.window_blocks == WINDOW_BLOCKS;
        self.window_blocks = (self.window_blocks + 1).min(WINDOW_BLOCKS);

        self.window_p[oldest] = self.block_p;
        self.window_q[oldest] = self.block_q;
        self.window_e[oldest] = self.block_e;
        self.window_band[oldest] = self.block_band;
        self.window_index = (oldest + 1) % WINDOW_BLOCKS;
        self.block_p = 0.0;
        self.block_q = 0.0;
        self.block_e = 0.0;
        self.block_band = 0.0;

        self.band_blocks = self.band_blocks.saturating_add(1);
        let band_weight = (1.0 / self.band_blocks as f32).max(BAND_WEIGHT);
        self.band_power += (block_band - self.band_power) * band_weight;

        // Summing the window from its blocks each time means rounding errors from a
        // loud moment cannot stay in the sums.
        let sum_p: f32 = self.window_p.iter().sum();
        let sum_e: f32 = self.window_e.iter().sum();
        let window_power = sum_e
            / (WINDOW_BLOCKS as f32 * STEPS_PER_BLOCK as f32 * samples_in_sum * samples_in_sum);
        let rho = if window_power > POWER_FLOOR {
            sum_p / sum_e
        } else {
            0.0
        };

        if walking {
            return false;
        }

        // The echoes of a chime are not the noise floor. The estimate has to follow noise
        // down to any level, however far below `POWER_FLOOR`: an estimate left high after
        // a loud sound would count a later, quieter chime as lost.
        let waiting = self.fixed_wait_blocks > 0 || self.tail_blocks > 0;
        if rho.abs() < RHO_QUIET && window_was_full && !waiting {
            if oldest_band > DIGITAL_SILENCE {
                self.noise_blocks = self.noise_blocks.saturating_add(1);
                let weight = (1.0 / self.noise_blocks as f32).max(NOISE_WEIGHT);
                self.noise_band += (oldest_band - self.noise_band) * weight;
                self.noise_bucket += (oldest_bucket - self.noise_bucket) * weight;
            } else {
                // Digital silence has no noise to measure. The next noise starts a new average.
                self.noise_blocks = 0;
                self.noise_band = 0.0;
                self.noise_bucket = 0.0;
            }
        }

        if self.settling_blocks > 0 {
            self.settling_blocks -= 1;
            return self.settling_blocks == 0;
        }
        if !self.still_waiting(newest_bucket) && rho >= RHO_TRIGGER {
            self.settling_blocks = SETTLE_BLOCKS;
        }
        false
    }

    /// Follows the wait after a chime, block by block. Returns true during the wait.
    fn still_waiting(&mut self, newest_bucket: f32) -> bool {
        self.tail_power += (newest_bucket - self.tail_power) * TAIL_WEIGHT;
        self.tail_lowest = self.tail_lowest.min(self.tail_power);
        if self.fixed_wait_blocks > 0 {
            self.fixed_wait_blocks -= 1;
            return true;
        }
        if self.tail_blocks == 0 {
            return false;
        }
        self.tail_blocks -= 1;
        if self.tail_power > TAIL_RISE_RATIO * self.tail_lowest {
            self.tail_blocks = 0;
            return false;
        }
        let faded = self.tail_power < TAIL_NOISE_RATIO * self.noise_bucket
            || self.tail_power < self.tail_chime_power * libm::powf(10.0, TAIL_END_DB / 10.0);
        if faded || self.tail_blocks == 0 {
            // The window still holds the end of the echoes, so it must refill first.
            self.tail_blocks = 0;
            self.fixed_wait_blocks = WINDOW_BLOCKS as u32;
        }
        true
    }

    /// Clears the window when a walk ends, because it may hold stop symbols.
    pub(super) fn clear_window(&mut self) {
        self.window_p = [0.0; WINDOW_BLOCKS];
        self.window_q = [0.0; WINDOW_BLOCKS];
        self.window_e = [0.0; WINDOW_BLOCKS];
        self.window_band = [0.0; WINDOW_BLOCKS];
        self.window_blocks = 0;
    }

    /// Starts the wait after a chime with the given bucket power.
    pub(super) fn wait_for_echoes(&mut self, fixed_wait_blocks: u32, chime_power: f32) {
        self.fixed_wait_blocks = fixed_wait_blocks;
        self.tail_blocks = TAIL_BLOCKS;
        self.tail_chime_power = chime_power;
        self.tail_power = chime_power;
        self.tail_lowest = chime_power;
    }

    /// Cancels any wait, after a walk that heard no chime.
    pub(super) fn no_wait(&mut self) {
        self.fixed_wait_blocks = 0;
        self.tail_blocks = 0;
    }
}

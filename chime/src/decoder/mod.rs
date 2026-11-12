//! Streaming chime decoder.
//!
//! The decoder has four stages, each in its own module:
//!
//! 1. [`front_end`] mixes the input down from 18 kHz to complex baseband, low-pass
//!    filters it, decimates it to about 8 kHz and limits loud samples.
//! 2. [`detector`] compares the signal with itself 7.5 ms earlier, which is a cheap test
//!    for start symbols with many false alarms in noise. It also measures the noise floor.
//! 3. [`timing_search`] adds up the last 50 ms over the 7.5 ms symbol period to find the
//!    symbol boundaries. Its score rejects most false alarms of the detector.
//! 4. Two [`walker`]s read the message on a grid of 2.5 ms buckets, because every symbol
//!    length is a multiple of 2.5 ms. Each has a [`trellis`] and its own [`trackers`].
//!
//! # Why a trellis
//!
//! Deciding each symbol as it arrives is not enough. A wrong decision on a symbol
//! length adds or removes a symbol, which shifts all later bits by one, and the
//! Hamming code cannot correct that. So the trellis keeps the best path to each
//! position in a symbol, for each symbol of a byte, and only the stop symbols and the
//! CRC choose among the paths. This gains about 5 dB over deciding each symbol.
//!
//! # Why two trellises
//!
//! A phone in the hand moves, and the carrier frequency moves with it. Following that
//! needs a fast carrier loop. In a room, or at low SNR, a fast loop lets echoes and noise
//! into the frequency estimate, so the same loop loses chimes there. No single setting
//! did well at both when measured, and nothing in the signal says which one applies. So
//! both run on the same samples, from the same start, and the CRC chooses. See
//! [`walker::STEADY`] and [`walker::MOVING`].
//!
//! Each trellis runs until it ends by itself. If only one reads a message, that is the
//! chime. If both read the same message, the steady trellis gives the event's fields. If
//! they read different messages, one of them is a chance match of the CRC, and there is
//! no safe way to tell which. A wrong clock is much worse than playing the chime again,
//! so the decoder reports [`UnreadableReason::ConflictingReads`].
//!
//! # When a failed walk counts as a chime
//!
//! Many sounds start a walk, and in a real recording with no chime about half of the
//! walks get as far as a byte: a tone or a burst near 18 kHz passes the timing search,
//! and a trellis finds some path through anything. So a walk that reads no message is
//! reported as [`Unreadable`] only with evidence that a chime was being received, which
//! is one of these:
//!
//! - The start symbols were clean and the signal was well above the noise. A chime that
//!   is cut off looks like this.
//! - The signal lasted for a whole message, with some sign of start symbols, and was well
//!   above the noise. A chime in a room looks like this, where the echoes spoil the start
//!   symbols but do not hide the signal.
//! - Enough codewords were valid. A chime too noisy to read looks like this, and only
//!   this evidence works when the chime is below the noise. See [`Path::codeword_score`].
//!
//! On 253 recordings with no chime this leaves no reports, and it keeps every failure of
//! a chime that was cut off or in a room, and about half of those from -7 to -9 dB SNR.
//!
//! # Cost
//!
//! The struct holds about 19 KB and nothing is allocated. On a Cortex-M7,
//! [`Decoder::init_in_place`] needs under 1 KB of stack, [`Decoder::new`] about 38 KB and
//! `push` about 1.5 KB. These figures come from reading the compiled code for
//! `thumbv7em-none-eabihf` at opt-level 3 with fat LTO. Nothing has run on the device.
//!
//! The rest of this section is an estimate, because nothing has been timed on a Cortex-M7.
//! At 192 kHz the front end costs about 19 floating-point operations per input sample: 12
//! in its first stage and 53 per output of the first stage, which comes every 8 samples.
//! With one stage it cost 47. The rest costs about 23 per baseband sample while listening
//! and about 375 per baseband sample during a message, which at 192 kHz is 1 and 16 per
//! input sample.
//!
//! Feed the decoder with [`Decoder::push_block`] where you can. It keeps the first stage's
//! sums in registers across the block, where [`Decoder::push`] loads and stores them for
//! each sample. On a PC without auto-vectorisation, at 192 kHz with no chime, `push_block`
//! costs 0.6 ms a second and `push` 0.8 ms. With one stage `push` cost 1.8 ms. The `cost`
//! example measures this. In one call of `push`, the
//! timing search is about 7600 operations and two trellis steps are about 6500, and one
//! call never does both.
//!
//! Those counts leave out the following work, so they are not a worst case:
//!
//! - Each sample over the limit adds a division, two multiplications and a square root in
//!   software, about 57 instructions. One search limited 168 of its 400 samples. Such a
//!   search could take about twice the cycles of a search with no limiting.
//! - The call that starts a walk also writes about 6.4 KB of paths, in `Trellis::begin`.
//! - Each bucket copies 124 paths of 52 bytes, about 6.4 KB.
//! - Each baseband sample during a walk has 6 conversions from `f64` to `u64` in software.
//!
//! On a PC at 192 kHz the longest call of `push` takes about 12 us, and the timing search
//! is the largest single cost. That order is not known to hold on a Cortex-M7.

use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

mod cplx;
mod detector;
mod front_end;
mod timing_search;
mod trackers;
mod trellis;
mod walker;

use self::cplx::Cplx;
use self::detector::Detector;
use self::front_end::{Baseband, FrontEnd, Ring};
use self::timing_search::MAX_FOLD_SLOTS;
use self::trellis::Path;
use self::walker::{MOVING, STEADY, WalkEnd, Walker};
use crate::format::{CRC_BYTES, MAX_PAYLOAD_BYTES, START_STOP_US, START_SYMBOLS, TIME_BYTES};
use crate::{Chime, Event, Unreadable, UnreadableReason};

/// Lowest sample rate the decoder accepts, in Hz. The 18 kHz carrier and its upper
/// sideband must fit below half the sample rate.
const MIN_SAMPLE_RATE_HZ: u32 = 40_000;

/// Length of a bucket in seconds.
const BUCKET_S: f64 = 0.0025;

/// Lowest baseband power per sample for a detection. It corresponds to a carrier of
/// about -127 dBFS RMS.
const POWER_FLOOR: f32 = 1e-13;

/// Number of 5 ms blocks during which no new detection starts after a walk ends.
/// The last stop symbols and their reverb look like start symbols.
const HOLDOFF_AFTER_CHIME_BLOCKS: u32 = 60;
const HOLDOFF_AFTER_BAD_CHIME_BLOCKS: u32 = 40;

/// Carrier power over the mean bucket power for a clean chime. The buckets at the
/// edges of a symbol hold its ramps, and the filter spreads the symbol edges.
const LEVEL_CORRECTION: f32 = 1.043;

/// The signal counts as well above the noise when its bucket power, 160 ms into a walk,
/// is at least this many times the noise in a bucket: 7 dB. A carrier at this level is
/// about -1 dB SNR in the 2 kHz band. In 253 recordings with no chime, no walk that failed
/// reached this level.
const WELL_ABOVE_NOISE: f32 = 5.0;

/// Coherence of the start symbols that shows clean start symbols. A clean chime has about
/// 0.77. A tone burst near 18 kHz can also reach this, but unless the burst lasts for
/// more than 160 ms, the signal is no longer well above the noise when the walk measures
/// it.
const CLEAN_START_COHERENCE: f32 = 0.5;

/// Coherence of the start symbols that shows some start symbols, for a signal that lasted
/// for a whole message. Failed chimes in rooms have at least 0.22.
const SOME_START_COHERENCE: f32 = 0.2;

/// Bytes of a whole message: the shortest payload and the CRC.
const WHOLE_MESSAGE_BYTES: usize = TIME_BYTES + CRC_BYTES;

/// [`Path::codeword_score`] that counts as evidence of a chime. In the shortest message,
/// which has 16 codewords, this takes 8 codewords with no errors. Random symbols reach that
/// about once in 3000 messages.
const CODEWORD_SCORE_OF_A_CHIME: i32 = 20;

fn round_to_u64(position: f64) -> u64 {
    (position + 0.5) as u64
}

/// Fills an array element by element, so that it does not pass through the stack.
///
/// # Safety
///
/// `array` must be valid for writes and aligned.
unsafe fn fill<T: Copy, const N: usize>(array: *mut [T; N], value: T) {
    let first = array.cast::<T>();
    for index in 0..N {
        // SAFETY: the caller makes the array valid for writes, and `index` is inside it.
        unsafe { first.add(index).write(value) };
    }
}

fn power_to_db(power: f32) -> f32 {
    10.0 * libm::log10f(power.max(1e-20))
}

/// What the two trellises read between them.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Reading {
    Nothing,
    /// One message, from this walker. If both read it, this is the first.
    Message(usize),
    /// Two different messages that both passed the CRC.
    Conflict,
}

fn compare_readings(first: Option<&[u8]>, second: Option<&[u8]>) -> Reading {
    match (first, second) {
        (Some(a), Some(b)) if a != b => Reading::Conflict,
        (Some(_), _) => Reading::Message(0),
        (None, Some(_)) => Reading::Message(1),
        (None, None) => Reading::Nothing,
    }
}

/// True when a walk that read no message shows that a chime was being received.
/// `well_above_noise` is about the signal over the walk, and `coherence` is from the timing
/// search that started it.
fn heard_a_chime(coherence: f32, well_above_noise: bool, path: &Path) -> bool {
    let whole_message = path.byte_count as usize >= WHOLE_MESSAGE_BYTES;
    let start_symbols_seen =
        coherence >= CLEAN_START_COHERENCE || (coherence >= SOME_START_COHERENCE && whole_message);
    (well_above_noise && start_symbols_seen) || path.codeword_score() >= CODEWORD_SCORE_OF_A_CHIME
}

/// The last stage that [`Decoder::push`] runs, for timing the stages one by one. Each
/// stage needs the ones before it. With a stage other than `Walk`, the decoder reads no
/// chime.
#[cfg(feature = "profile")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    FrontEnd,
    Detector,
    /// The timing search runs when the detector asks for it, but no walk starts.
    Search,
    Walk,
}

/// Streaming chime decoder for one channel of audio.
pub struct Decoder {
    front_end: FrontEnd,
    ring: Ring,
    detector: Detector,
    fold_slots: [Cplx; MAX_FOLD_SLOTS],
    /// True while a walk is under way.
    walking: bool,
    /// The coherence from the timing search that started the latest walk.
    coherence: f32,
    walkers: [Walker; 2],
    #[cfg(feature = "profile")]
    last_stage: Stage,
}

impl Decoder {
    /// Returns `None` below 40 kHz.
    ///
    /// The decoder passes through the stack here, so this suits a PC. On the X8, use
    /// [`Decoder::init_in_place`].
    pub fn new(sample_rate_hz: u32) -> Option<Decoder> {
        let mut slot = MaybeUninit::uninit();
        Decoder::init_in_place(&mut slot, sample_rate_hz)?;
        // SAFETY: `init_in_place` returned a decoder, so it has written every field.
        Some(unsafe { slot.assume_init() })
    }

    /// Builds a decoder in `slot`, which can be a `static`, field by field, so that the
    /// decoder never passes through the stack. Returns `None` below 40 kHz, and `slot` is
    /// then unchanged.
    ///
    /// To change the sample rate, call this again with the same slot. Whatever `slot` held
    /// is overwritten, and a decoder has nothing to drop.
    pub fn init_in_place(
        slot: &mut MaybeUninit<Decoder>,
        sample_rate_hz: u32,
    ) -> Option<&mut Decoder> {
        if sample_rate_hz < MIN_SAMPLE_RATE_HZ {
            return None;
        }
        let this = slot.as_mut_ptr();
        // SAFETY: `this` comes from a reference, so it is valid for writes and aligned. The
        // writes below cover every field, and the reference to the whole decoder is made
        // after them. The front end is complete before its baseband rate is read.
        unsafe {
            FrontEnd::init_in_place(addr_of_mut!((*this).front_end), sample_rate_hz);
            let baseband_rate = (*this).front_end.baseband_rate();
            let bucket_len = BUCKET_S * baseband_rate;
            Ring::init_in_place(addr_of_mut!((*this).ring));
            addr_of_mut!((*this).detector).write(Detector::new(baseband_rate));
            fill(addr_of_mut!((*this).fold_slots), Cplx::ZERO);
            addr_of_mut!((*this).walking).write(false);
            addr_of_mut!((*this).coherence).write(0.0);
            let walkers = addr_of_mut!((*this).walkers).cast::<Walker>();
            Walker::init_in_place(walkers, STEADY, bucket_len);
            Walker::init_in_place(walkers.add(1), MOVING, bucket_len);
            #[cfg(feature = "profile")]
            addr_of_mut!((*this).last_stage).write(Stage::Walk);
            Some(&mut *this)
        }
    }

    /// Feeds one sample of any scale, normally +-1.0. Returns an event when a chime
    /// ends.
    ///
    /// [`Decoder::push_block`] does the same for a block of samples, at a lower cost per
    /// sample.
    pub fn push(&mut self, sample: f32) -> Option<Event> {
        let baseband = self.front_end.push_one(sample, &mut self.ring)?;
        self.take_baseband(baseband)
    }

    /// Feeds a block of samples, as [`Decoder::push`] does one at a time, and calls
    /// `on_event` with each event and the index in `block` of the sample that ended it.
    pub fn push_block(&mut self, block: &[f32], mut on_event: impl FnMut(usize, Event)) {
        let mut at = 0;
        while at < block.len() {
            let (taken, baseband) = self.front_end.push(&block[at..], &mut self.ring);
            at += taken;
            if let Some(event) = baseband.and_then(|baseband| self.take_baseband(baseband)) {
                on_event(at - 1, event);
            }
        }
    }

    /// Everything after the front end, for one baseband sample.
    fn take_baseband(&mut self, baseband: Baseband) -> Option<Event> {
        #[cfg(feature = "profile")]
        if self.last_stage == Stage::FrontEnd {
            return None;
        }

        // At the start of a walk a few buckets are waiting. Taking one for each baseband
        // sample bounds the work in one call of push, and the walk soon catches up. A
        // walk that starts in this call waits for the next, because the timing search
        // is a large cost.
        let was_walking = self.walking;
        if self
            .detector
            .push(baseband.limited, baseband.raw_power, was_walking)
        {
            #[cfg(feature = "profile")]
            if self.last_stage == Stage::Detector {
                return None;
            }
            self.search_for_symbols();
        }
        if was_walking {
            let power_limit = self.front_end.power_limit();
            let noise_bucket = self.detector.noise_bucket();
            for walker in self.walkers.iter_mut() {
                walker.advance(&self.ring, power_limit, noise_bucket);
            }
            if self.walkers.iter().all(|walker| walker.end().is_some()) {
                return self.finish_walk();
            }
        }
        None
    }

    /// Makes [`Decoder::push`] stop after `stage`.
    #[cfg(feature = "profile")]
    pub fn set_last_stage(&mut self, stage: Stage) {
        self.last_stage = stage;
    }

    /// RMS level of the 17 to 19 kHz band, in dBFS. This is a running average with a time
    /// constant of 1 s, or the mean of what has arrived during the first second. Its
    /// lowest value is -120 dBFS.
    pub fn band_level_dbfs(&self) -> f32 {
        power_to_db(2.0 * self.detector.band_power()).max(-120.0)
    }

    /// Runs the timing search and starts a walk if it finds start symbols.
    fn search_for_symbols(&mut self) {
        let found = timing_search::search(
            &self.ring,
            &mut self.fold_slots,
            self.front_end.baseband_rate(),
            self.detector.lag_product(),
            self.front_end.power_limit(),
        );
        #[cfg(feature = "profile")]
        if self.last_stage == Stage::Search {
            return;
        }
        if let Some(found) = found {
            self.walking = true;
            self.coherence = found.coherence;
            for walker in self.walkers.iter_mut() {
                walker.begin(found.start, found.carrier_frequency, found.coherence);
            }
        }
    }

    /// Decides what the finished walk read and makes the event, if there is one.
    fn finish_walk(&mut self) -> Option<Event> {
        self.walking = false;
        self.detector.clear_window();

        let [steady, moving] = &self.walkers;
        let reading = compare_readings(
            steady.message_path().map(Path::message),
            moving.message_path().map(Path::message),
        );
        match reading {
            Reading::Message(index) => {
                let event = self.chime_event(index);
                self.detector
                    .wait_for_echoes(HOLDOFF_AFTER_CHIME_BLOCKS, self.walkers[index].mean_power());
                Some(Event::Chime(event))
            }
            Reading::Conflict => self.unreadable_event(UnreadableReason::ConflictingReads),
            Reading::Nothing => self.failed_walk_event(),
        }
    }

    /// The walker whose path looks most like a chime, of those that did not go silent.
    fn most_chime_like_walker(&self) -> Option<(usize, usize)> {
        let mut best = None;
        let mut best_score = i32::MIN;
        for (index, walker) in self.walkers.iter().enumerate() {
            let state = match walker.end() {
                Some(WalkEnd::Message(state) | WalkEnd::Failed(_, state)) => state,
                _ => continue,
            };
            let score = walker.path(state).codeword_score();
            if score > best_score {
                best_score = score;
                best = Some((index, state));
            }
        }
        best
    }

    /// The event for a walk that read no message, if it heard a chime.
    fn failed_walk_event(&mut self) -> Option<Event> {
        let chosen = self.most_chime_like_walker();
        let heard = chosen.is_some_and(|(index, state)| {
            let walker = &self.walkers[index];
            let well_above_noise =
                walker.level_reference() >= WELL_ABOVE_NOISE * self.detector.noise_bucket();
            heard_a_chime(self.coherence, well_above_noise, walker.path(state))
        });
        if !heard {
            self.detector.no_wait();
            return None;
        }
        let (index, _) = chosen?;
        let reason = match self.walkers[index].end() {
            Some(WalkEnd::Failed(reason, _)) => reason,
            _ => UnreadableReason::BadCrc,
        };
        self.unreadable_event(reason)
    }

    /// Makes an [`Unreadable`] event from the most chime-like walker.
    fn unreadable_event(&mut self, reason: UnreadableReason) -> Option<Event> {
        let (index, state) = self.most_chime_like_walker()?;
        let walker = &self.walkers[index];
        let path = walker.path(state);
        // After a signal is lost the walk has gone on through what followed it.
        let (mean_power, bytes_received) = if reason == UnreadableReason::SignalLost {
            (walker.received_power(), walker.received_bytes())
        } else {
            (walker.mean_power(), path.byte_count as usize)
        };
        let event = Unreadable {
            start_sample: self.start_sample(walker.walk_start(), path),
            end_sample: self.front_end.input_samples(),
            reason,
            bytes_received,
            level_dbfs: self.carrier_level(mean_power).0,
        };
        let chime_power = walker.mean_power();
        // These three reached the stop symbols, which look like start symbols.
        let fixed_wait = if matches!(
            reason,
            UnreadableReason::BadCrc
                | UnreadableReason::ConflictingReads
                | UnreadableReason::MessageRefused
        ) {
            HOLDOFF_AFTER_BAD_CHIME_BLOCKS
        } else {
            0
        };
        self.detector.wait_for_echoes(fixed_wait, chime_power);
        Some(Event::Unreadable(event))
    }

    fn chime_event(&self, index: usize) -> Chime {
        let walker = &self.walkers[index];
        let path = walker.message_path().expect("this walker read a message");
        let (level_dbfs, level_power) = self.carrier_level(walker.mean_power());
        let payload_len = path.byte_count as usize - CRC_BYTES;
        let mut payload = [0u8; MAX_PAYLOAD_BYTES];
        payload[..payload_len].copy_from_slice(&path.bytes[..payload_len]);
        let snr_db = if self.detector.noise_measured() {
            10.0 * libm::log10f(level_power / (2.0 * self.detector.noise_band()))
        } else {
            f32::NAN
        };
        Chime {
            payload,
            payload_len,
            start_sample: self.start_sample(walker.walk_start(), path),
            end_sample: self.front_end.input_samples(),
            corrected_bits: path.corrected_bits as u32,
            level_dbfs,
            snr_db,
        }
    }

    /// The input sample index where the first start symbol began, for a path. A path with
    /// data symbols gives this by counting back from the first of them, which comes out
    /// negative when the input started during the start symbols. A path without any shows
    /// only where the walk started.
    fn start_sample(&self, walk_start: f64, path: &Path) -> i64 {
        if path.data_start >= 0.0 {
            let data_start = self
                .front_end
                .input_position(walk_start + path.data_start as f64);
            let start = data_start
                - START_SYMBOLS as f64 * START_STOP_US as f64 * 1e-6 * self.front_end.sample_rate();
            libm::floor(start + 0.5) as i64
        } else {
            round_to_u64(self.front_end.input_position(walk_start).max(0.0)) as i64
        }
    }

    /// RMS level of the carrier in dBFS and in power, from the mean power of the walk's
    /// buckets. A carrier of amplitude A has baseband power A^2 / 4, while its RMS power
    /// is A^2 / 2.
    fn carrier_level(&self, mean: f32) -> (f32, f32) {
        let power = 2.0 * LEVEL_CORRECTION * (mean - self.detector.noise_bucket()).max(mean * 0.01);
        (power_to_db(power), power)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_that_agree_or_stand_alone_are_a_message() {
        let a: &[u8] = &[1, 2, 3];
        assert_eq!(compare_readings(Some(a), Some(a)), Reading::Message(0));
        assert_eq!(compare_readings(Some(a), None), Reading::Message(0));
        assert_eq!(compare_readings(None, Some(a)), Reading::Message(1));
        assert_eq!(compare_readings(None, None), Reading::Nothing);
    }

    /// A decoder at the end of a walk, in which each walker has read the given message.
    fn after_a_walk(steady: &[u8], moving: Option<&[u8]>) -> Decoder {
        let mut decoder = Decoder::new(48_000).unwrap();
        decoder.walking = true;
        decoder.walkers[0].end_with_message(steady);
        match moving {
            Some(message) => decoder.walkers[1].end_with_message(message),
            None => decoder.walkers[1].begin(0.0, 0.0, 0.5),
        }
        decoder
    }

    /// A message with a good CRC, whose first payload byte is `first_byte`.
    fn message(first_byte: u8) -> [u8; 8] {
        let mut bytes = [first_byte, 2, 3, 4, 5, 6, 0, 0];
        let crc = crate::format::crc16(&bytes[..6]);
        bytes[6] = (crc & 0xFF) as u8;
        bytes[7] = (crc >> 8) as u8;
        bytes
    }

    #[test]
    fn two_different_messages_with_good_crcs_are_unreadable() {
        let (first, second) = (message(1), message(9));
        let mut decoder = after_a_walk(&first, Some(&second));
        match decoder.finish_walk() {
            Some(Event::Unreadable(unreadable)) => {
                assert_eq!(unreadable.reason, UnreadableReason::ConflictingReads);
                assert_eq!(unreadable.bytes_received, 8);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_same_message_from_both_is_one_chime() {
        let bytes = message(1);
        let mut decoder = after_a_walk(&bytes, Some(&bytes));
        match decoder.finish_walk() {
            Some(Event::Chime(chime)) => assert_eq!(chime.payload[..chime.payload_len], bytes[..6]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_message_from_either_is_a_chime_when_the_other_read_nothing() {
        let bytes = message(1);
        let mut decoder = after_a_walk(&bytes, None);
        decoder.walkers[1].finish_silently();
        match decoder.finish_walk() {
            Some(Event::Chime(chime)) => assert_eq!(chime.payload[..chime.payload_len], bytes[..6]),
            other => panic!("{other:?}"),
        }

        let mut decoder = after_a_walk(&bytes, None);
        decoder.walkers[0].finish_silently();
        decoder.walkers[1].end_with_message(&bytes);
        match decoder.finish_walk() {
            Some(Event::Chime(chime)) => assert_eq!(chime.payload[..chime.payload_len], bytes[..6]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn readings_that_differ_are_a_conflict() {
        let a: &[u8] = &[1, 2, 3];
        let b: &[u8] = &[1, 2, 4];
        let shorter: &[u8] = &[1, 2];
        assert_eq!(compare_readings(Some(a), Some(b)), Reading::Conflict);
        assert_eq!(compare_readings(Some(a), Some(shorter)), Reading::Conflict);
    }
}

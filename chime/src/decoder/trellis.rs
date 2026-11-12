//! The trellis: the best path to each position in a symbol, for each symbol of a byte.
//!
//! Each path carries its own phase reference and its own decoded bytes, which keeps the
//! code small: there is no traceback, and there are no tentative decisions to undo.

use core::ptr::addr_of_mut;

use super::cplx::Cplx;
use super::fill;
use super::trackers::{ECHO_TAPS, echo_of};
use crate::format::{self, CRC_BYTES, MAX_MESSAGE_BYTES, SYMBOLS_PER_BYTE, TimeMessage};

/// Trellis states. Each state is a position in a symbol, and a path in a state has
/// just consumed the bucket at that position. S0 to S2 are the three buckets of a
/// start symbol and T0 to T2 those of a stop symbol. A data symbol has up to four
/// buckets, and the data states also record which symbol of its byte it is, because
/// a path can only stop at the end of a byte. A path that gains or loses a symbol
/// through noise is therefore in a different state from the correct path, and the
/// two never compete. Only the stop symbols and the CRC can choose between them.
pub(super) const S0: usize = 0;
pub(super) const S1: usize = 1;
pub(super) const S2: usize = 2;
pub(super) const T0: usize = 3;
pub(super) const T1: usize = 4;
pub(super) const T2: usize = 5;
pub(super) const FIRST_DATA_STATE: usize = 6;
pub(super) const STATES: usize = FIRST_DATA_STATE + SYMBOLS_PER_BYTE * 4;

/// The state for bucket `bucket` of symbol `symbol` of a byte.
const fn data_state(symbol: usize, bucket: usize) -> usize {
    FIRST_DATA_STATE + symbol * 4 + bucket
}

/// A message whose CRC is right is still refused if more than this percentage of its
/// codewords needed correcting. The CRC has only 16 bits and a failed walk tests many
/// possible messages, so noise sometimes passes it. Wrong symbols give a word that needs
/// correcting 7 times in 8, and 5 of the 8 wrong messages seen in tests were above this
/// percentage. The most seen on a true read is 10 of 16 words, at -9 dB SNR, where few
/// chimes can be read at all.
const MAX_CORRECTED_PERCENT: u32 = 65;

/// A path must hold this many stop symbols before its CRC decides the message.
const STOP_SYMBOLS_TO_ACCEPT: u8 = 4;

/// Polarity history of a path before the walk: the pattern of the start symbols, three
/// buckets of each polarity, with the newest bucket in bit 0.
const START_HISTORY: u32 = 0b1100_0111_0001_1100_0111_0001_1100_0111;

/// One path through the trellis, with everything it has decoded so far.
#[derive(Clone, Copy)]
pub(super) struct Path {
    /// Sum of the agreement of each bucket with the path's reference. Larger is
    /// better. Only differences between paths mean anything.
    metric: f32,
    /// Decaying sum of the path's buckets with the polarity of the path's current
    /// symbol. The phase of this sum is the phase the next bucket should have.
    pub(super) reference: Cplx,
    /// Polarity of the path's latest buckets, newest in bit 0.
    pub(super) history: u32,
    pub(super) start_symbols: u8,
    pub(super) stop_symbols: u8,
    /// Distance in baseband samples from the start of the walk to the first data
    /// symbol. Negative while the path has no data symbols.
    pub(super) data_start: f32,
    /// The two Hamming codewords of the current byte, low nibble first.
    codewords: [u8; 2],
    /// Complete bytes, CRC included. One more than `MAX_MESSAGE_BYTES` means that
    /// the message is too long, and that byte is not stored.
    pub(super) byte_count: u8,
    /// Codewords the Hamming code corrected. Each word has at most one correction.
    pub(super) corrected_bits: u16,
    pub(super) bytes: [u8; MAX_MESSAGE_BYTES],
}

impl Path {
    const DEAD: Path = Path {
        metric: f32::NEG_INFINITY,
        reference: Cplx::ZERO,
        history: 0,
        start_symbols: 0,
        stop_symbols: 0,
        data_start: -1.0,
        codewords: [0; 2],
        byte_count: 0,
        corrected_bits: 0,
        bytes: [0; MAX_MESSAGE_BYTES],
    };

    /// True when the data symbol that finishes this path's current byte can be the
    /// last data symbol of a message. The format has only four message lengths, and each
    /// other length allowed would be one more chance for noise to pass the CRC.
    fn can_stop_after_this_byte(&self) -> bool {
        format::is_payload_len((self.byte_count as usize + 1).saturating_sub(CRC_BYTES))
    }

    /// Records a finished data symbol, number `symbol` of its byte, as a bit.
    fn push_bit(&mut self, symbol: usize, bit: u8) {
        self.codewords[symbol % 2] |= bit << (symbol / 2);
        if symbol == SYMBOLS_PER_BYTE - 1 {
            let (low, low_corrected) = format::hamming_decode(self.codewords[0]);
            let (high, high_corrected) = format::hamming_decode(self.codewords[1]);
            if (self.byte_count as usize) < MAX_MESSAGE_BYTES {
                self.bytes[self.byte_count as usize] = (high << 4) | low;
            }
            self.byte_count = self.byte_count.saturating_add(1);
            self.corrected_bits += low_corrected as u16 + high_corrected as u16;
            self.codewords = [0; 2];
        }
    }

    /// Writes this path, moved on by one bucket, to `next`. `metric` is the metric of
    /// the move. `flipped` means that the bucket is the first of a new symbol.
    fn extend_into(&self, next: &mut Path, bucket: Cplx, metric: f32, flipped: bool, memory: f32) {
        if metric == f32::NEG_INFINITY {
            next.metric = metric;
            return;
        }
        *next = *self;
        next.metric = metric;
        let polarity = if flipped {
            -self.reference
        } else {
            self.reference
        };
        next.reference = polarity * memory + bucket;
        next.history = (self.history << 1) | ((self.history & 1) ^ flipped as u32);
    }

    /// The payload length of this path's bytes, if the format has that length.
    fn payload_len(&self) -> Option<usize> {
        let len = (self.byte_count as usize).checked_sub(CRC_BYTES)?;
        format::is_payload_len(len).then_some(len)
    }

    /// True when the bytes end with the correct CRC of the bytes before it.
    pub(super) fn crc_matches(&self) -> bool {
        let Some(payload_len) = self.payload_len() else {
            return false;
        };
        let crc = format::crc16(&self.bytes[..payload_len]);
        self.bytes[payload_len] == (crc & 0xFF) as u8
            && self.bytes[payload_len + 1] == (crc >> 8) as u8
    }

    /// True when the CRC matches and the message is one that a chime could carry.
    pub(super) fn is_good_message(&self) -> bool {
        let Some(payload_len) = self.payload_len() else {
            return false;
        };
        let count = payload_len + CRC_BYTES;
        if self.corrected_bits as u32 * 100 > MAX_CORRECTED_PERCENT * (2 * count) as u32 {
            return false;
        }
        // The CRC of zeros is zero, so a message of only zeros always passes. A path
        // that takes noise for short symbols produces just that.
        if self.bytes[..count].iter().all(|&byte| byte == 0) {
            return false;
        }
        TimeMessage::parse(&self.bytes[..payload_len]).is_some() && self.crc_matches()
    }

    /// The message bytes, CRC included, of a path whose message is good.
    pub(super) fn message(&self) -> &[u8] {
        &self.bytes[..self.byte_count as usize]
    }

    /// How much this path's codewords look like real data. A codeword that needed no
    /// correction counts for 5 and one that did counts against by 2.
    ///
    /// Wrong symbols give a word with no errors once in 8 words, and a real chime near
    /// the limit of reading gives one about once in 2. So a clean word is 4 times as
    /// likely in a real chime and a corrected word about half as likely, and the
    /// logarithms of those are in the ratio 5 to 2.
    pub(super) fn codeword_score(&self) -> i32 {
        let corrected = self.corrected_bits as i32;
        let clean = 2 * self.byte_count as i32 - corrected;
        5 * clean - 2 * corrected
    }
}

/// True for a state that is the first bucket of a symbol, which follows a polarity flip.
pub(super) fn starts_a_symbol(state: usize) -> bool {
    state == S0
        || state == T0
        || (state >= FIRST_DATA_STATE && (state - FIRST_DATA_STATE).is_multiple_of(4))
}

/// True for a state in a stop symbol.
pub(super) fn in_stop_symbol(state: usize) -> bool {
    state == T0 || state == T1 || state == T2
}

/// True for a state in a start symbol.
pub(super) fn in_start_symbol(state: usize) -> bool {
    state <= S2
}

/// The paths of a walk, for the newest bucket and the bucket before.
pub(super) struct Trellis {
    buffers: [[Path; STATES]; 2],
    /// Which of `buffers` holds the current paths.
    current: usize,
}

impl Trellis {
    /// Builds a trellis at `this`.
    ///
    /// # Safety
    ///
    /// `this` must be valid for writes and aligned.
    pub(super) unsafe fn init_in_place(this: *mut Trellis) {
        // SAFETY: the caller makes `this` valid for writes, and these are all of its fields.
        unsafe {
            let buffers = addr_of_mut!((*this).buffers).cast::<[Path; STATES]>();
            fill(buffers, Path::DEAD);
            fill(buffers.add(1), Path::DEAD);
            addr_of_mut!((*this).current).write(0);
        }
    }

    /// Prepares for a new walk.
    pub(super) fn begin(&mut self) {
        // A path "before" the walk, so that the first bucket is the first bucket of a
        // start symbol.
        self.buffers[0] = [Path::DEAD; STATES];
        self.buffers[0][S2].metric = 0.0;
        self.buffers[0][S2].history = START_HISTORY;
        self.current = 0;
    }

    pub(super) fn paths(&self) -> &[Path; STATES] {
        &self.buffers[self.current]
    }

    /// Puts a message into the current path in state T0.
    #[cfg(test)]
    pub(super) fn set_message_at_t0(&mut self, message: &[u8]) {
        let path = &mut self.buffers[self.current][T0];
        path.bytes[..message.len()].copy_from_slice(message);
        path.byte_count = message.len() as u8;
    }

    /// Moves all paths on by one bucket. Returns the state of the best path.
    ///
    /// `distance` is the distance in baseband samples from the start of the walk to this
    /// bucket, and `memory` is the weight of the older buckets in a path's reference.
    pub(super) fn step(
        &mut self,
        bucket: Cplx,
        echo_taps: &[Cplx; ECHO_TAPS],
        distance: f32,
        memory: f32,
    ) -> usize {
        let (buffer_a, buffer_b) = self.buffers.split_at_mut(1);
        let (old, new) = if self.current == 0 {
            (&buffer_a[0], &mut buffer_b[0])
        } else {
            (&buffer_b[0], &mut buffer_a[0])
        };
        self.current = 1 - self.current;

        // Each path removes the echoes that its own past polarities would cause. A path
        // whose past fits the signal has less left over, and the second term rewards
        // that. With no echoes this term is the same for all paths.
        let left_over_weight = 0.5 / (1.0 - memory);
        let mut cleaned = [Cplx::ZERO; STATES];
        let mut scores = [0.0f32; STATES];
        let mut penalties = [0.0f32; STATES];
        for state in 0..STATES {
            let path = &old[state];
            if path.metric == f32::NEG_INFINITY {
                continue;
            }
            let left_over = bucket - echo_of(echo_taps, path.history);
            cleaned[state] = left_over;
            scores[state] = path.reference.dot(left_over);
            penalties[state] = left_over.power() * left_over_weight;
        }
        let stay = |state: usize| old[state].metric + scores[state] - penalties[state];
        let flip = |state: usize| old[state].metric - scores[state] - penalties[state];

        // Moves inside a symbol.
        for (from, to) in [(S0, S1), (S1, S2), (T0, T1), (T1, T2)] {
            old[from].extend_into(&mut new[to], cleaned[from], stay(from), false, memory);
        }
        for symbol in 0..SYMBOLS_PER_BYTE {
            for position in 0..3 {
                let from = data_state(symbol, position);
                old[from].extend_into(&mut new[from + 1], cleaned[from], stay(from), false, memory);
            }
        }

        // Moves to the first bucket of a new symbol.
        old[S2].extend_into(&mut new[S0], cleaned[S2], flip(S2), true, memory);
        new[S0].start_symbols = new[S0].start_symbols.saturating_add(1);

        for symbol in 0..SYMBOLS_PER_BYTE {
            let previous = (symbol + SYMBOLS_PER_BYTE - 1) % SYMBOLS_PER_BYTE;
            let after_0 = data_state(previous, 1);
            let after_1 = data_state(previous, 3);
            let next = &mut new[data_state(symbol, 0)];
            let (source, bit) = if flip(after_1) > flip(after_0) {
                (after_1, 1)
            } else {
                (after_0, 0)
            };
            let can_start_data = symbol == 0 && old[S2].start_symbols > 0;
            if can_start_data && flip(S2) > flip(source) {
                old[S2].extend_into(next, cleaned[S2], flip(S2), true, memory);
                next.data_start = distance;
            } else {
                old[source].extend_into(next, cleaned[source], flip(source), true, memory);
                next.push_bit(previous, bit);
            }
        }

        // A stop symbol follows a stop symbol, or a data symbol that finishes a byte.
        let last = SYMBOLS_PER_BYTE - 1;
        let after_0 = data_state(last, 1);
        let after_1 = data_state(last, 3);
        let mut best = flip(T2);
        let mut source = T2;
        if old[after_0].can_stop_after_this_byte() && flip(after_0) > best {
            best = flip(after_0);
            source = after_0;
        }
        if old[after_1].can_stop_after_this_byte() && flip(after_1) > best {
            best = flip(after_1);
            source = after_1;
        }
        old[source].extend_into(&mut new[T0], cleaned[source], best, true, memory);
        if source == T2 {
            new[T0].stop_symbols = new[T0].stop_symbols.saturating_add(1);
        } else {
            new[T0].push_bit(last, if source == after_1 { 1 } else { 0 });
        }

        // Keep the metrics small, so that adding to them stays accurate.
        let mut best_state = 0;
        for state in 1..STATES {
            if new[state].metric > new[best_state].metric {
                best_state = state;
            }
        }
        let best_metric = new[best_state].metric;
        for path in new.iter_mut() {
            path.metric -= best_metric;
        }
        best_state
    }

    /// True when the path in state T0 holds enough stop symbols, leads the other stop
    /// positions and has a good message.
    ///
    /// The stop symbols of a path can start at three positions on the grid, and a path
    /// with a wrong last symbol or two can still pass the CRC after the Hamming
    /// correction. That is why the path must also lead the other stop positions.
    pub(super) fn stop_is_complete(&self) -> bool {
        let paths = self.paths();
        let stopping = &paths[T0];
        stopping.metric > f32::NEG_INFINITY
            && stopping.metric >= paths[T1].metric
            && stopping.metric >= paths[T2].metric
            && stopping.stop_symbols >= STOP_SYMBOLS_TO_ACCEPT
            && stopping.is_good_message()
    }
}

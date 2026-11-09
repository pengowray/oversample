//! Telling bat calls from the other sounds the call detector finds.
//!
//! [`CallDetector`](super::detector::CallDetector) reports anything that rises
//! above the background in the ultrasonic band: bat calls, and also clicks,
//! rustles, pecking, sweeping, the upper harmonics of bird calls, and insects.
//! [`CallClassifier`] sorts each detection with checks that hold for bats in
//! general, not for one species:
//!
//! 1. **Steady tone**: the detector reports a long steady tone, such as a
//!    rat's 22 kHz alarm call, as a run of short pieces wherever its level
//!    dips. A detection at most 1.5 kHz wide is a piece of a tone when it
//!    joins a chain of such pieces, each starting within 20 ms of the end of
//!    the one before and peaking within 1.5 kHz of it, that has lasted
//!    150 ms or more: longer than any bat call. The tone's frequency is then
//!    remembered for 2 s after its last piece, and narrow detections within
//!    1.5 kHz of it in that time are pieces of it too, so the tone does not
//!    start again as a train of bat calls after each pause. Pieces of the
//!    chain from before it reached 150 ms can no longer start a train.
//!    Regular pulses count as a steady tone too (see below).
//! 2. **Broadband**: the call's range reaches the bottom of the detection
//!    band. Clicks and rustles are loudest at low frequencies and reach down
//!    to it; bat calls stop well above it.
//! 3. **Too low**: the peak is below the lowest bat frequency (default
//!    16 kHz, which keeps most free-tailed bats; settable). With the
//!    detector's band starting at 15 kHz, most sounds this low are already
//!    caught as broadband.
//! 4. **Audible**: the audible band (2 to 12 kHz) was loud at the same moment
//!    (default: more than 4 dB above its floor). Bats are silent there; birds,
//!    voices and footsteps are not.
//! 5. **Train**: bats call in trains, several calls a second at similar
//!    frequencies. A call that passes the first four checks is a bat call
//!    once a similar call (peak within 5 kHz) passed them in the last 0.6 s,
//!    and at least one of the two is 18 dB above the floor. Until then it is
//!    a single.
//!
//! The first call of a train only becomes a bat call when the second arrives,
//! so [`Verdict::earlier`] hands it back then, for a display to mark too.
//!
//! **Regular pulses.** A detection that passes the first four checks is a
//! piece of a steady tone when it extends a run of 8 or more pulses at a similar
//! frequency (peak within 1.5 kHz) with the same spacing, within 1 ms, up to 3
//! pulses in a row being allowed to go missing. Pulses that regular are not a bat's:
//! bats vary the spacing of their calls. In 286 recordings (the library and the X8's),
//! runs of bat calls stopped at 5 pulses. 12 recordings had runs of 7 to 445, spaced
//! about 10, 17, 33 or 400 ms, at one frequency each, such as 44.6 or 83 kHz. Some of
//! those spacings fit the mains (10 ms is twice 50 Hz), so these may be electrical
//! interference; that has not been checked. 261105_0082 has 3,570 such pulses, which
//! the train check took for bat calls. With this check, it has none, and 276 of the
//! 286 recordings have the same bat calls as before. Pieces of the run from before it
//! reached 8 can no longer start a train.
//!
//! Tuned on Portacapture X8 recordings at 192 kHz (call detector with
//! 512-point columns every 128 samples): of 97 calls from a bat at 45 to
//! 50 kHz it kept 94, it kept 97 of 98 calls in two annotated passes, and in
//! two recordings with no bats (birds feeding beside the mics, traffic,
//! sweeping) it reduced detections from 96 and 39 a minute to 11.1 and 3.6
//! (7.4 and 2.0 with the lowest bat frequency at 20 kHz).
//!
//! The figures here were measured with columns every 128 samples. The app now
//! makes a column every 192 samples. Rechecked with that (`bat-app` example
//! `hop_compare`): bat calls in 238 recordings went from 5399 to 5486, and false
//! bat calls in 91 minutes of no-bat recordings from 474 to 489. Moving the
//! columns half a hop later, still every 128 samples, gives 5387 and 491.
//!
//! Checked on 247 Xeno-canto echolocation recordings of 76 genera from
//! around the world, resampled to 192 kHz (`bat-app` example `library`):
//! 240 gave bat calls, and 72% of all detections in them were classified as
//! bat calls (65% for free-tailed bats; 54% with the lowest bat frequency at
//! 20 kHz). The 7 that gave none were free-tails calling below 16 kHz (2),
//! horseshoe bats above 100 kHz, beyond the X8's range (2), clips of 0.3 s
//! (2), and a low-quality recording with loud background sound. A lower
//! audible band and laxer thresholds gained little recall for many more
//! false detections, so those defaults stay.
//!
//! The steady-tone check was added for a brown rat's 22 kHz alarm call
//! (Xeno-canto XC960575, 79 s at 96 kHz, resampled to 192 kHz), which the
//! other checks counted as 476 bat calls out of 560 detections. With it, 435
//! detections are pieces of a tone and 75 are still counted as bat calls,
//! mostly the first pieces of a call after a pause of more than 2 s. On the
//! recordings above it changed nothing except one Xeno-canto recording
//! (XC823559, "other unknown" sounds beside Myotis calls), where a steady
//! 16 to 17 kHz tone had been counted as 29 bat calls. Shorter chains (100 ms)
//! or wider pieces (2.5 kHz) caught a few more rat pieces but also took calls
//! from horseshoe, sheath-tailed and free-tailed bats, whose steady calls the
//! detector also splits into pieces.
//!
//! Ported from the X8 bat app's `bat-dsp` crate unchanged, apart from the
//! Rust edition. The figures above were measured there, at 192 kHz with
//! 512-point columns. Oversample keeps the same bin width and column spacing
//! at other sample rates (see [`FrameConfig`](super::FrameConfig)), and its
//! band reaches 1 kHz below Nyquist instead of 95 kHz, so horseshoe bats above
//! 100 kHz are found in recordings at 250 kHz or more.

use super::detector::CallEvent;

pub const DEFAULT_MIN_BAT_HZ: f32 = 16_000.0;
pub const DEFAULT_MAX_AUDIBLE_DB: f32 = 4.0;
pub const DEFAULT_TRAIN_WINDOW_S: f32 = 0.6;
pub const DEFAULT_TRAIN_FREQ_HZ: f32 = 5_000.0;
pub const DEFAULT_TRAIN_MIN_DB: f32 = 18.0;
/// Widest detection, from its lowest to its highest frequency, that can be a
/// piece of a steady tone.
pub const TONE_MAX_WIDTH_HZ: f32 = 1_500.0;
/// Most a tone's pieces may differ in peak frequency, one to the next.
pub const TONE_FREQ_HZ: f32 = 1_500.0;
/// Longest gap between the end of one piece of a tone and the start of the
/// next.
pub const TONE_GAP_MS: f32 = 20.0;
/// A chain of pieces lasting this long is a steady tone: longer than any bat
/// call.
pub const TONE_MIN_MS: f32 = 150.0;
/// How long a tone's frequency is remembered after its last piece.
pub const TONE_HOLD_S: f32 = 2.0;
/// Pulses at the same spacing that make a run of regular pulses.
pub const REGULAR_MIN_PULSES: u32 = 8;
/// How far a pulse may be from the spacing of its run. The detector's times are
/// whole columns, so this is at least a column at the app's 1 ms per column.
pub const REGULAR_TOLERANCE_MS: f32 = 1.0;

/// A range whose bottom is within this of the band's bottom reaches it.
const EDGE_MARGIN_HZ: f32 = 1_000.0;
/// Recent calls remembered for trains.
const RECENT: usize = 16;
/// Recent narrow detections remembered for chains of tone pieces.
const PIECES: usize = 8;
/// Steady tones remembered at once.
const TONES: usize = 4;
/// Runs of evenly spaced pulses followed at once.
const PULSE_RUNS: usize = 4;
/// Most pulses in a row that may go missing from a run that has not yet reached
/// REGULAR_MIN_PULSES, plus one. The detector misses some, and so does a gap in the
/// analysis.
const MOST_SPACINGS: u64 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CallKind {
    /// A piece of a steady tone lasting longer than a bat call, such as a
    /// rat's 22 kHz alarm call, or one of a run of regular pulses.
    Tone,
    /// Reaches the bottom of the detection band: a click or rustle.
    Broadband,
    /// Peak below the lowest bat frequency.
    TooLow,
    /// A loud audible sound came at the same moment.
    Audible,
    /// Passes the checks, but no similar call came just before it (yet).
    Single,
    /// Passes the checks and follows a similar call: a bat call.
    Bat,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Verdict {
    pub kind: CallKind,
    /// When this call is the second of a train, the first call, which is a
    /// bat call too but was reported as a single.
    pub earlier: Option<CallEvent>,
}

#[derive(Clone, Copy, Debug)]
struct Recent {
    event: CallEvent,
    /// Already reported as a bat call, directly or as `earlier`.
    bat: bool,
}

/// A narrow detection, kept to see whether it continues a chain.
#[derive(Clone, Copy, Debug)]
struct Piece {
    end: u64,
    peak_hz: f32,
    /// Start of the first piece of the chain this piece belongs to.
    chain_start: u64,
}

/// A steady tone heard recently.
#[derive(Clone, Copy, Debug)]
struct Tone {
    freq_hz: f32,
    /// Start of the chain of pieces that made it a tone.
    start: u64,
    /// End of its latest piece.
    end: u64,
}

/// Pulses at a similar frequency, followed to see whether they are evenly spaced.
#[derive(Clone, Copy, Debug)]
struct PulseRun {
    peak_hz: f32,
    /// Start of the first pulse and of the latest pulse.
    first: u64,
    last: u64,
    /// The spacing between pulses, 0 until the second pulse.
    spacing: u64,
    pulses: u32,
}

#[derive(Clone, Debug)]
pub struct CallClassifier {
    sample_rate: f32,
    band_lo_hz: f32,
    min_bat_hz: f32,
    max_audible_db: f32,
    train_window: u64,
    train_freq_hz: f32,
    train_min_db: f32,
    tone_gap: u64,
    tone_min: u64,
    tone_hold: u64,
    recent: [Option<Recent>; RECENT],
    next: usize,
    pieces: [Option<Piece>; PIECES],
    next_piece: usize,
    tones: [Option<Tone>; TONES],
    pulse_runs: [Option<PulseRun>; PULSE_RUNS],
    regular_tolerance: u64,
    /// Start of the tone the latest detection belongs to, if it is a piece
    /// of one.
    tone_since: u64,
    /// The latest detection was a piece of a steady tone.
    last_was_tone: bool,
}

impl CallClassifier {
    /// `band_lo_hz` is the bottom of the call detector's band.
    pub fn new(sample_rate: f32, band_lo_hz: f32) -> Self {
        let mut c = Self {
            sample_rate,
            band_lo_hz,
            min_bat_hz: DEFAULT_MIN_BAT_HZ,
            max_audible_db: DEFAULT_MAX_AUDIBLE_DB,
            train_window: 0,
            train_freq_hz: DEFAULT_TRAIN_FREQ_HZ,
            train_min_db: DEFAULT_TRAIN_MIN_DB,
            tone_gap: (TONE_GAP_MS * 0.001 * sample_rate) as u64,
            tone_min: (TONE_MIN_MS * 0.001 * sample_rate) as u64,
            tone_hold: (TONE_HOLD_S * sample_rate) as u64,
            recent: [None; RECENT],
            next: 0,
            pieces: [None; PIECES],
            next_piece: 0,
            tones: [None; TONES],
            pulse_runs: [None; PULSE_RUNS],
            regular_tolerance: (REGULAR_TOLERANCE_MS * 0.001 * sample_rate) as u64 + 1,
            tone_since: 0,
            last_was_tone: false,
        };
        c.set_train_window_s(DEFAULT_TRAIN_WINDOW_S);
        c
    }

    pub fn set_band_lo_hz(&mut self, hz: f32) {
        self.band_lo_hz = hz;
    }

    pub fn set_min_bat_hz(&mut self, hz: f32) {
        self.min_bat_hz = hz;
    }

    pub fn min_bat_hz(&self) -> f32 {
        self.min_bat_hz
    }

    pub fn set_max_audible_db(&mut self, db: f32) {
        self.max_audible_db = db;
    }

    pub fn max_audible_db(&self) -> f32 {
        self.max_audible_db
    }

    /// How long before a call a similar call may come and still make a train.
    pub fn set_train_window_s(&mut self, s: f32) {
        self.train_window = (s.max(0.0) * self.sample_rate) as u64;
    }

    /// How far apart two calls' peaks may be and still make a train.
    pub fn set_train_freq_hz(&mut self, hz: f32) {
        self.train_freq_hz = hz;
    }

    /// At least one call of a pair must rise this far above the floor.
    pub fn set_train_min_db(&mut self, db: f32) {
        self.train_min_db = db;
    }

    pub fn reset(&mut self) {
        self.recent = [None; RECENT];
        self.next = 0;
        self.pieces = [None; PIECES];
        self.next_piece = 0;
        self.tones = [None; TONES];
        self.pulse_runs = [None; PULSE_RUNS];
        self.last_was_tone = false;
    }

    /// When the latest detection was a piece of a steady tone, where the tone
    /// started: the first piece of the chain that made it a tone. Pauses of
    /// up to 2 s, after which pieces still count as the same tone, are
    /// included.
    pub fn tone_start(&self) -> Option<u64> {
        self.last_was_tone.then_some(self.tone_since)
    }

    /// Classify the next detection. Detections must come in time order.
    pub fn classify(&mut self, event: &CallEvent) -> Verdict {
        self.last_was_tone = self.is_tone(event);
        let kind = if self.last_was_tone {
            CallKind::Tone
        } else if event.min_freq_hz <= self.band_lo_hz + EDGE_MARGIN_HZ {
            CallKind::Broadband
        } else if event.peak_freq_hz < self.min_bat_hz {
            CallKind::TooLow
        } else if event.audible_db_above_floor > self.max_audible_db {
            CallKind::Audible
        } else {
            CallKind::Single
        };
        if kind != CallKind::Single {
            return Verdict {
                kind,
                earlier: None,
            };
        }
        if let Some(first) = self.regular_pulses_since(event) {
            self.last_was_tone = true;
            self.tone_since = first;
            return Verdict {
                kind: CallKind::Tone,
                earlier: None,
            };
        }

        // The latest earlier call that makes a train with this one.
        let mut partner: Option<usize> = None;
        for (i, slot) in self.recent.iter().enumerate() {
            let Some(r) = slot else { continue };
            let gap = event.start_sample.saturating_sub(r.event.start_sample);
            let similar = (r.event.peak_freq_hz - event.peak_freq_hz).abs() <= self.train_freq_hz;
            let strong =
                r.event.peak_db_above_floor.max(event.peak_db_above_floor) >= self.train_min_db;
            let later = partner.is_none_or(|p| {
                self.recent[p].is_some_and(|q| r.event.start_sample > q.event.start_sample)
            });
            if gap <= self.train_window && similar && strong && later {
                partner = Some(i);
            }
        }

        let mut verdict = Verdict {
            kind: CallKind::Single,
            earlier: None,
        };
        if let Some(p) = partner {
            verdict.kind = CallKind::Bat;
            if let Some(r) = &mut self.recent[p] {
                if !r.bat {
                    r.bat = true;
                    verdict.earlier = Some(r.event);
                }
            }
        }
        self.recent[self.next] = Some(Recent {
            event: *event,
            bat: verdict.kind == CallKind::Bat,
        });
        self.next = (self.next + 1) % RECENT;
        verdict
    }

    /// When the detection extends a run of regular pulses, the start of the run; see
    /// the module documentation. Follows the detection's run as it goes.
    ///
    /// A detection at a run's frequency that does not fit the run's spacing starts
    /// the run again. Otherwise a bat's calls, one of which fits by chance now and
    /// then, would build up a run over time. A run that has reached REGULAR_MIN_PULSES
    /// is not restarted, because such a detection may be a bat at the same frequency
    /// as a machine that is still running.
    fn regular_pulses_since(&mut self, event: &CallEvent) -> Option<u64> {
        let (start, peak) = (event.start_sample, event.peak_freq_hz);
        let tolerance = self.regular_tolerance;
        let tone_hold = self.tone_hold;
        let fits = |run: &PulseRun, most_spacings: u64| {
            let since = start.saturating_sub(run.last);
            // How many spacings have passed, to the nearest whole number.
            let spacings = (since + run.spacing / 2) / run.spacing.max(1);
            (1..=most_spacings).contains(&spacings)
                && since.abs_diff(spacings * run.spacing) <= tolerance
        };
        let similar = |run: &PulseRun| (run.peak_hz - peak).abs() <= TONE_FREQ_HZ;

        for run in self.pulse_runs.iter_mut().flatten() {
            if run.pulses >= REGULAR_MIN_PULSES
                && similar(run)
                && start.saturating_sub(run.last) <= tone_hold
                && fits(run, u64::MAX)
            {
                run.last = start;
                return Some(run.first);
            }
        }
        let new_run = PulseRun {
            peak_hz: peak,
            first: start,
            last: start,
            spacing: 0,
            pulses: 1,
        };
        let building = self
            .pulse_runs
            .iter_mut()
            .flatten()
            .find(|run| run.pulses < REGULAR_MIN_PULSES && similar(run));
        let Some(run) = building else {
            // Replace the run that has gone longest without a pulse.
            let slot = (0..PULSE_RUNS)
                .min_by_key(|&i| self.pulse_runs[i].map_or(0, |r| r.last + 1))
                .unwrap_or(0);
            self.pulse_runs[slot] = Some(new_run);
            return None;
        };
        if run.spacing == 0 {
            run.spacing = start.saturating_sub(run.last);
        } else if !fits(run, MOST_SPACINGS) {
            *run = new_run;
            return None;
        }
        run.last = start;
        run.pulses += 1;
        if run.pulses < REGULAR_MIN_PULSES {
            return None;
        }
        // The run's earlier pulses can no longer start a train.
        let first = run.first;
        for slot in &mut self.recent {
            if slot.is_some_and(|r| {
                r.event.start_sample >= first && (r.event.peak_freq_hz - peak).abs() <= TONE_FREQ_HZ
            }) {
                *slot = None;
            }
        }
        Some(first)
    }

    /// Whether the detection is a piece of a steady tone; see the module
    /// documentation. Remembers narrow detections and tones as it goes.
    fn is_tone(&mut self, event: &CallEvent) -> bool {
        if event.max_freq_hz - event.min_freq_hz > TONE_MAX_WIDTH_HZ {
            return false;
        }
        let (start, end, peak) = (event.start_sample, event.end_sample, event.peak_freq_hz);
        let mut tone = false;
        // Near a tone heard in the last TONE_HOLD_S: follow it as it drifts.
        for t in self.tones.iter_mut().flatten() {
            if start.saturating_sub(t.end) <= self.tone_hold
                && (t.freq_hz - peak).abs() <= TONE_FREQ_HZ
            {
                t.freq_hz = peak;
                t.end = t.end.max(end);
                self.tone_since = t.start;
                tone = true;
                break;
            }
        }

        // The chain this piece continues: the latest narrow detection that
        // ended just before it at a similar frequency.
        let mut chain_start = start;
        let mut latest_end = 0;
        for p in self.pieces.iter().flatten() {
            if start.saturating_sub(p.end) <= self.tone_gap
                && (p.peak_hz - peak).abs() <= TONE_FREQ_HZ
                && p.end >= latest_end
            {
                latest_end = p.end;
                chain_start = p.chain_start;
            }
        }
        self.pieces[self.next_piece] = Some(Piece {
            end,
            peak_hz: peak,
            chain_start,
        });
        self.next_piece = (self.next_piece + 1) % PIECES;

        if end.saturating_sub(chain_start) >= self.tone_min {
            if !tone {
                // A new tone: replace one that has ended, or the oldest.
                let slot = (0..TONES)
                    .min_by_key(|&i| self.tones[i].map_or(0, |t| t.end + 1))
                    .unwrap_or(0);
                self.tones[slot] = Some(Tone {
                    freq_hz: peak,
                    start: chain_start,
                    end,
                });
                self.tone_since = chain_start;
            }
            tone = true;
            // The chain's earlier pieces can no longer start a train.
            for slot in &mut self.recent {
                if slot.is_some_and(|r| {
                    r.event.start_sample >= chain_start
                        && r.event.max_freq_hz - r.event.min_freq_hz <= TONE_MAX_WIDTH_HZ
                        && (r.event.peak_freq_hz - peak).abs() <= TONE_FREQ_HZ
                }) {
                    *slot = None;
                }
            }
        }
        for t in &mut self.tones {
            if t.is_some_and(|t| start.saturating_sub(t.end) > self.tone_hold) {
                *t = None;
            }
        }
        tone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 192_000.0;

    /// `count` pulses from `at_s` on, with `spacing_ms` between their starts.
    fn pulses(
        c: &mut CallClassifier,
        at_s: f32,
        spacing_ms: f32,
        count: usize,
        khz: f32,
    ) -> Vec<CallKind> {
        (0..count)
            .map(|i| {
                c.classify(&call(at_s + i as f32 * spacing_ms * 0.001, khz, 40.0))
                    .kind
            })
            .collect()
    }

    #[test]
    fn evenly_spaced_pulses_stop_counting_as_bat_calls() {
        for spacing_ms in [10.0, 17.0, 50.0, 401.0] {
            let mut c = classifier();
            let kinds = pulses(&mut c, 1.0, spacing_ms, 20, 44.6);
            let first_tone = kinds.iter().position(|&k| k == CallKind::Tone).unwrap();
            assert_eq!(
                first_tone,
                REGULAR_MIN_PULSES as usize - 1,
                "{spacing_ms}: {kinds:?}"
            );
            assert!(
                kinds[first_tone..].iter().all(|&k| k == CallKind::Tone),
                "{spacing_ms}: {kinds:?}"
            );
        }
    }

    #[test]
    fn a_missing_pulse_does_not_restart_a_run_of_regular_pulses() {
        let mut c = classifier();
        for i in (0..30).filter(|i| ![5, 12, 13, 14].contains(i)) {
            let kind = c.classify(&call(1.0 + i as f32 * 0.010, 44.6, 40.0)).kind;
            if i > 10 {
                assert_eq!(kind, CallKind::Tone, "pulse {i}");
            }
        }
    }

    #[test]
    fn calls_with_varied_spacing_stay_bat_calls() {
        // Spacings that vary by more than the tolerance, as a bat's do.
        let mut c = classifier();
        let mut t = 1.0;
        let mut kinds = Vec::new();
        for i in 0..60 {
            kinds.push(c.classify(&call(t, 45.0, 40.0)).kind);
            t += 0.080 + 0.0031 * ((i * 7 % 5) as f32);
        }
        assert!(kinds[1..].iter().all(|&k| k == CallKind::Bat), "{kinds:?}");
    }

    #[test]
    fn a_bat_at_another_frequency_is_still_a_bat_beside_regular_pulses() {
        let mut c = classifier();
        pulses(&mut c, 1.0, 10.0, 30, 44.6);
        let first = c.classify(&call(1.31, 60.0, 40.0));
        assert_eq!(first.kind, CallKind::Single);
        let second = c.classify(&call(1.39, 60.0, 40.0));
        assert_eq!(second.kind, CallKind::Bat);
    }

    #[test]
    fn regular_pulses_cannot_start_a_train_afterwards() {
        let mut c = classifier();
        pulses(&mut c, 1.0, 10.0, 8, 44.6);
        // Just after the run, at an uneven time: nothing left to pair with.
        let later = c.classify(&call(1.1234, 45.0, 40.0));
        assert_eq!(later.kind, CallKind::Single);
    }

    fn call(at_s: f32, peak_khz: f32, db: f32) -> CallEvent {
        let start = (at_s * FS) as u64;
        CallEvent {
            start_sample: start,
            end_sample: start + 960,
            peak_freq_hz: peak_khz * 1000.0,
            peak_db_above_floor: db,
            audible_db_above_floor: -3.0,
            min_freq_hz: (peak_khz - 1.0) * 1000.0,
            max_freq_hz: (peak_khz + 8.0) * 1000.0,
        }
    }

    fn classifier() -> CallClassifier {
        CallClassifier::new(FS, 15_000.0)
    }

    /// A narrow detection, 0.75 kHz wide.
    fn piece(at_s: f32, len_ms: f32, khz: f32, db: f32) -> CallEvent {
        let start = (at_s * FS) as u64;
        CallEvent {
            start_sample: start,
            end_sample: start + (len_ms * 0.001 * FS) as u64,
            peak_freq_hz: khz * 1000.0,
            peak_db_above_floor: db,
            audible_db_above_floor: -3.0,
            min_freq_hz: (khz - 0.375) * 1000.0,
            max_freq_hz: (khz + 0.375) * 1000.0,
        }
    }

    /// Pieces of `len_ms` every `every_ms` from `from_s` to `to_s`.
    fn pieces(from_s: f32, to_s: f32, len_ms: f32, every_ms: f32, khz: f32) -> Vec<CallEvent> {
        let n = ((to_s - from_s) * 1000.0 / every_ms) as usize;
        (0..n)
            .map(|i| piece(from_s + i as f32 * every_ms * 0.001, len_ms, khz, 22.0))
            .collect()
    }

    #[test]
    fn a_steady_tone_in_pieces_is_a_tone_and_stays_one_after_a_pause() {
        let mut c = classifier();
        // 30 ms pieces with 5 ms gaps, drifting down slowly, as a rat's
        // 22 kHz call breaks up.
        let mut kinds = Vec::new();
        for (i, mut e) in pieces(1.0, 2.0, 30.0, 35.0, 22.0).into_iter().enumerate() {
            e.peak_freq_hz -= i as f32 * 20.0;
            kinds.push(c.classify(&e).kind);
        }
        // The first pieces come before the chain reaches 150 ms; the rest are
        // pieces of the tone.
        let bats = kinds.iter().filter(|&&k| k == CallKind::Bat).count();
        assert!(bats <= 4, "{kinds:?}");
        assert!(kinds[5..].iter().all(|&k| k == CallKind::Tone), "{kinds:?}");
        // The tone has lasted from its first piece.
        assert_eq!(c.tone_start(), Some((1.0 * FS) as u64));
        // After a pause of 1 s the tone carries on, slightly lower: still the
        // same tone from its first piece.
        for e in pieces(3.0, 3.5, 30.0, 35.0, 21.3) {
            assert_eq!(c.classify(&e).kind, CallKind::Tone);
        }
        assert_eq!(c.tone_start(), Some((1.0 * FS) as u64));
        // A bat sweeping through the same frequencies is still a bat.
        c.classify(&call(3.6, 25.0, 25.0));
        assert_eq!(c.classify(&call(3.7, 25.0, 25.0)).kind, CallKind::Bat);
    }

    #[test]
    fn a_tone_is_forgotten_after_the_hold_time() {
        let mut c = classifier();
        for e in pieces(1.0, 1.5, 30.0, 35.0, 22.0) {
            c.classify(&e);
        }
        let later = 1.5 + TONE_HOLD_S + 0.5;
        assert_eq!(
            c.classify(&piece(later, 30.0, 22.0, 22.0)).kind,
            CallKind::Single
        );
    }

    #[test]
    fn narrow_calls_with_gaps_between_them_are_bat_calls() {
        // Constant-frequency calls of 50 ms every 100 ms, as a horseshoe bat
        // makes: narrow, but with gaps longer than the tone check allows.
        let mut c = classifier();
        // Fewer than REGULAR_MIN_PULSES, because these are evenly spaced.
        let kinds: Vec<_> = pieces(1.0, 1.7, 50.0, 100.0, 70.0)
            .iter()
            .map(|e| c.classify(e).kind)
            .collect();
        assert!(kinds[1..].iter().all(|&k| k == CallKind::Bat), "{kinds:?}");
    }

    #[test]
    fn a_train_is_confirmed_on_its_second_call_and_hands_back_the_first() {
        let mut c = classifier();
        let first = call(1.00, 47.0, 25.0);
        assert_eq!(
            c.classify(&first),
            Verdict {
                kind: CallKind::Single,
                earlier: None
            }
        );
        let second = c.classify(&call(1.08, 47.5, 22.0));
        assert_eq!(second.kind, CallKind::Bat);
        assert_eq!(second.earlier, Some(first));
        // The third call joins the train; the first is not handed back twice.
        assert_eq!(
            c.classify(&call(1.16, 46.8, 20.0)),
            Verdict {
                kind: CallKind::Bat,
                earlier: None
            }
        );
    }

    #[test]
    fn calls_far_apart_in_time_or_frequency_stay_single() {
        let mut c = classifier();
        assert_eq!(c.classify(&call(1.0, 47.0, 25.0)).kind, CallKind::Single);
        assert_eq!(c.classify(&call(2.0, 47.0, 25.0)).kind, CallKind::Single);
        assert_eq!(c.classify(&call(2.1, 60.0, 25.0)).kind, CallKind::Single);
    }

    #[test]
    fn a_weak_pair_stays_single() {
        let mut c = classifier();
        c.classify(&call(1.0, 47.0, 14.0));
        assert_eq!(c.classify(&call(1.1, 47.0, 15.0)).kind, CallKind::Single);
    }

    #[test]
    fn clicks_low_sounds_and_audible_sounds_are_not_bats() {
        let mut c = classifier();
        let mut click = call(1.0, 22.0, 30.0);
        click.min_freq_hz = 15_000.0;
        assert_eq!(c.classify(&click).kind, CallKind::Broadband);
        // Below the lowest bat frequency, with the band started lower so the
        // call does not reach its bottom.
        let mut low = CallClassifier::new(FS, 8_000.0);
        assert_eq!(low.classify(&call(1.1, 14.0, 30.0)).kind, CallKind::TooLow);
        let mut bird = call(1.2, 26.0, 30.0);
        bird.audible_db_above_floor = 12.0;
        assert_eq!(c.classify(&bird).kind, CallKind::Audible);
        // None of them start a train.
        assert_eq!(c.classify(&call(1.3, 22.0, 30.0)).kind, CallKind::Single);
    }

    #[test]
    fn the_lowest_bat_frequency_is_settable() {
        let mut c = classifier();
        c.set_min_bat_hz(20_000.0);
        assert_eq!(c.classify(&call(1.0, 18.0, 30.0)).kind, CallKind::TooLow);
        c.set_band_lo_hz(8_000.0);
        c.set_min_bat_hz(12_000.0);
        assert_eq!(c.classify(&call(1.1, 14.0, 30.0)).kind, CallKind::Single);
    }
}

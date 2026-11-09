//! Grouping bat calls into trains.
//!
//! The classifier marks a call as a bat call when a similar call came shortly
//! before it, but it does not say where one train ends and the next begins.
//! [`group_trains`] does: it walks the bat calls in time order and adds each
//! one to the train whose latest call
//!
//! - ended at most [`TrainParams::max_gap_s`] (default 0.5 s) before this
//!   call started, and
//! - peaked within [`TrainParams::max_freq_step_hz`] (default 5 kHz, the
//!   classifier's train check) of this call's peak.
//!
//! When several trains qualify, the one whose latest peak is nearest in
//! frequency wins, so two bats calling at the same time at different
//! frequencies give two trains. A call that fits no train starts a new one.
//! Comparing each call with the train's latest call, not its first, lets a
//! train follow a bat whose calls drift in frequency as it approaches prey.
//!
//! Only bat calls are grouped, so a click or a bird between two calls does
//! not split a train. Trains with fewer than [`TrainParams::min_calls`]
//! calls (default 2) are dropped; their calls stay as calls.
//!
//! Why 0.5 s: bats searching for prey call every 50 to 400 ms, and the
//! detector misses some calls in a pass, which doubles a gap. The classifier
//! pairs calls up to 0.6 s apart, start to start. A longer silence usually
//! means the bat has gone and the next calls are a new pass.

use super::classify::{CallKind, DEFAULT_TRAIN_FREQ_HZ};
use super::scan::DetectedCall;

pub const DEFAULT_MAX_GAP_S: f64 = 0.5;
pub const DEFAULT_MIN_TRAIN_CALLS: usize = 2;

/// Settings for [`group_trains`].
#[derive(Clone, Debug, PartialEq)]
pub struct TrainParams {
    /// Longest silence, in seconds, from the end of a train's latest call to
    /// the start of the next call in the train.
    pub max_gap_s: f64,
    /// Most a call's peak frequency may differ from the peak of the train's
    /// latest call, in Hz.
    pub max_freq_step_hz: f32,
    /// Fewest calls in a train.
    pub min_calls: usize,
}

impl Default for TrainParams {
    fn default() -> Self {
        Self {
            max_gap_s: DEFAULT_MAX_GAP_S,
            max_freq_step_hz: DEFAULT_TRAIN_FREQ_HZ,
            min_calls: DEFAULT_MIN_TRAIN_CALLS,
        }
    }
}

/// A train of bat calls.
#[derive(Clone, Debug, PartialEq)]
pub struct CallTrain {
    /// Start of the first call, in seconds.
    pub start_s: f64,
    /// End of the last call, in seconds.
    pub end_s: f64,
    /// Lowest and highest frequency of any call's range, in Hz.
    pub min_freq_hz: f32,
    pub max_freq_hz: f32,
    /// The train's calls, as indices into the slice given to
    /// [`group_trains`], in time order.
    pub calls: Vec<usize>,
}

impl CallTrain {
    pub fn duration_s(&self) -> f64 {
        self.end_s - self.start_s
    }

    /// Median of the calls' peak frequencies, in Hz.
    pub fn median_peak_hz(&self, calls: &[DetectedCall]) -> f32 {
        let mut peaks: Vec<f32> = self
            .calls
            .iter()
            .filter_map(|&i| calls.get(i).map(|c| c.event.peak_freq_hz))
            .collect();
        if peaks.is_empty() {
            return 0.0;
        }
        peaks.sort_by(f32::total_cmp);
        peaks[peaks.len() / 2]
    }
}

/// Group the bat calls in `calls` (in time order, as a scan gives them) into
/// trains; see the module documentation. Calls of other kinds are skipped.
pub fn group_trains(calls: &[DetectedCall], params: &TrainParams) -> Vec<CallTrain> {
    let mut trains: Vec<CallTrain> = Vec::new();
    // Peak frequency of each train's latest call.
    let mut last_peak: Vec<f32> = Vec::new();
    // Trains that may still take a call, by index into `trains`.
    let mut open: Vec<usize> = Vec::new();

    for (i, call) in calls.iter().enumerate() {
        if call.kind != CallKind::Bat {
            continue;
        }
        let peak = call.event.peak_freq_hz;
        open.retain(|&t| call.start_s - trains[t].end_s <= params.max_gap_s);
        let best = open
            .iter()
            .copied()
            .map(|t| (t, (last_peak[t] - peak).abs()))
            .filter(|&(_, step)| step <= params.max_freq_step_hz)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(t, _)| t);
        match best {
            Some(t) => {
                let train = &mut trains[t];
                train.end_s = train.end_s.max(call.end_s);
                train.min_freq_hz = train.min_freq_hz.min(call.event.min_freq_hz);
                train.max_freq_hz = train.max_freq_hz.max(call.event.max_freq_hz);
                train.calls.push(i);
                last_peak[t] = peak;
            }
            None => {
                open.push(trains.len());
                last_peak.push(peak);
                trains.push(CallTrain {
                    start_s: call.start_s,
                    end_s: call.end_s,
                    min_freq_hz: call.event.min_freq_hz,
                    max_freq_hz: call.event.max_freq_hz,
                    calls: vec![i],
                });
            }
        }
    }
    trains.retain(|t| t.calls.len() >= params.min_calls.max(1));
    trains
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::call_detect::detector::CallEvent;

    const FS: f64 = 192_000.0;

    fn call(at_s: f64, len_ms: f64, peak_khz: f32, kind: CallKind) -> DetectedCall {
        let start = (at_s * FS) as u64;
        let end = start + (len_ms * 0.001 * FS) as u64;
        DetectedCall {
            event: CallEvent {
                start_sample: start,
                end_sample: end,
                peak_freq_hz: peak_khz * 1000.0,
                peak_db_above_floor: 30.0,
                audible_db_above_floor: -3.0,
                min_freq_hz: (peak_khz - 2.0) * 1000.0,
                max_freq_hz: (peak_khz + 10.0) * 1000.0,
            },
            start_s: start as f64 / FS,
            end_s: end as f64 / FS,
            kind,
            contour: Vec::new(),
        }
    }

    /// `count` bat calls every `every_s` from `from_s`, at `khz`.
    fn train(from_s: f64, every_s: f64, count: usize, khz: f32) -> Vec<DetectedCall> {
        (0..count)
            .map(|i| call(from_s + i as f64 * every_s, 5.0, khz, CallKind::Bat))
            .collect()
    }

    fn by_time(mut calls: Vec<DetectedCall>) -> Vec<DetectedCall> {
        calls.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
        calls
    }

    #[test]
    fn a_pause_longer_than_the_gap_splits_two_passes() {
        let mut calls = train(1.0, 0.1, 10, 45.0);
        calls.extend(train(3.0, 0.1, 6, 45.0));
        let trains = group_trains(&calls, &TrainParams::default());
        assert_eq!(trains.len(), 2, "{trains:?}");
        assert_eq!(trains[0].calls, (0..10).collect::<Vec<_>>());
        assert_eq!(trains[1].calls, (10..16).collect::<Vec<_>>());
        assert!((trains[0].start_s - 1.0).abs() < 1e-3);
        assert!((trains[0].end_s - 1.905).abs() < 1e-3);
        assert_eq!(trains[0].min_freq_hz, 43_000.0);
        assert_eq!(trains[0].max_freq_hz, 55_000.0);
    }

    #[test]
    fn the_gap_is_adjustable() {
        let mut calls = train(1.0, 0.1, 5, 45.0);
        calls.extend(train(1.8, 0.1, 5, 45.0));
        // 0.4 s from the end of the first group to the start of the second.
        assert_eq!(group_trains(&calls, &TrainParams::default()).len(), 1);
        let tight = TrainParams {
            max_gap_s: 0.3,
            ..TrainParams::default()
        };
        assert_eq!(group_trains(&calls, &tight).len(), 2);
    }

    #[test]
    fn two_bats_at_once_at_different_frequencies_give_two_trains() {
        let mut calls = train(1.0, 0.1, 10, 45.0);
        calls.extend(train(1.03, 0.13, 8, 25.0));
        let calls = by_time(calls);
        let trains = group_trains(&calls, &TrainParams::default());
        assert_eq!(trains.len(), 2, "{trains:?}");
        assert_eq!(trains[0].calls.len(), 10);
        assert_eq!(trains[1].calls.len(), 8);
        assert!(trains[0]
            .calls
            .iter()
            .all(|&i| calls[i].event.peak_freq_hz == 45_000.0));
    }

    #[test]
    fn a_drifting_train_is_followed_call_by_call() {
        // 60 kHz down to 42 kHz in 3 kHz steps: more than 5 kHz end to end.
        let calls: Vec<_> = (0..7)
            .map(|i| {
                call(
                    1.0 + 0.08 * i as f64,
                    4.0,
                    60.0 - 3.0 * i as f32,
                    CallKind::Bat,
                )
            })
            .collect();
        assert_eq!(group_trains(&calls, &TrainParams::default()).len(), 1);
    }

    #[test]
    fn other_kinds_are_skipped_and_do_not_split_a_train() {
        let mut calls = train(1.0, 0.1, 6, 45.0);
        calls.push(call(1.25, 2.0, 20.0, CallKind::Broadband));
        calls.push(call(1.32, 30.0, 45.0, CallKind::Tone));
        let calls = by_time(calls);
        let trains = group_trains(&calls, &TrainParams::default());
        assert_eq!(trains.len(), 1);
        assert_eq!(trains[0].calls.len(), 6);
        assert!(trains[0]
            .calls
            .iter()
            .all(|&i| calls[i].kind == CallKind::Bat));
    }

    #[test]
    fn a_lone_bat_call_is_not_a_train() {
        let mut calls = train(1.0, 0.1, 4, 45.0);
        calls.push(call(3.0, 5.0, 45.0, CallKind::Bat));
        let trains = group_trains(&calls, &TrainParams::default());
        assert_eq!(trains.len(), 1);
        assert_eq!(trains[0].calls.len(), 4);
        let mut params = TrainParams::default();
        params.min_calls = 1;
        assert_eq!(group_trains(&calls, &params).len(), 2);
    }

    #[test]
    fn median_peak() {
        let calls: Vec<_> = [40.0, 48.0, 44.0]
            .iter()
            .enumerate()
            .map(|(i, &k)| call(1.0 + 0.1 * i as f64, 4.0, k, CallKind::Bat))
            .collect();
        let trains = group_trains(&calls, &TrainParams::default());
        assert_eq!(trains[0].median_peak_hz(&calls), 44_000.0);
    }
}

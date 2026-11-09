//! Running the call detector over a recording.
//!
//! [`CallScanner`] takes a recording's mono samples in pieces of any size, so
//! a caller can read a long file a piece at a time and yield between pieces.
//! For each column it runs the detector, and for each detection the
//! classifier. [`scan_calls`] does the same for samples already in memory.
//!
//! Each call also gets a **contour**: for each column from the call's start to
//! its end, the frequency that rose highest above the noise floor, searched
//! within the call's frequency range plus a margin (default 2 kHz each side),
//! with the peak placed between bins by a parabola. Points more than 20 dB
//! (settable) below the contour's loudest point are dropped, which removes the
//! columns at either end where the call is fading in or out and the loudest
//! bin is noise. The detector reports a call only after it ends (2 ms later,
//! the merge gap), so the scanner keeps the last columns, enough for the
//! longest call, to trace it from.
//!
//! When the classifier confirms a train on its second call, it hands back the
//! first, reported earlier as a single; the scanner marks that call as a bat
//! call too. At the end, [`CallScanner::finish`] groups the bat calls into
//! trains with [`group_trains`].

use super::classify::{CallClassifier, CallKind, DEFAULT_MIN_BAT_HZ};
use super::detector::{
    default_band_hi_hz, CallDetector, CallEvent, DEFAULT_AUDIBLE_BAND_HZ, DEFAULT_BAND_LO_HZ,
    DEFAULT_MAX_CALL_MS, DEFAULT_MERGE_GAP_MS, DEFAULT_MIN_CALL_MS, DEFAULT_RANGE_DB,
    DEFAULT_RELEASE_DB, DEFAULT_THRESHOLD_DB,
};
use super::spectrum::{Column, SpectrumAnalyzer};
use super::trains::{group_trains, CallTrain, TrainParams};
use super::FrameConfig;

/// The bottom of the detection band sits this far below the lowest bat
/// frequency (16 and 15 kHz by default).
pub const BAND_BELOW_MIN_BAT_HZ: f32 = 1_000.0;
/// The top of the audible band sits this far below the bottom of the
/// detection band (15 and 12 kHz by default).
pub const AUDIBLE_BELOW_BAND_HZ: f32 = 3_000.0;
pub const DEFAULT_CONTOUR_MARGIN_HZ: f32 = 2_000.0;
pub const DEFAULT_CONTOUR_RANGE_DB: f32 = 20.0;
/// Columns kept beyond the longest call and the merge gap, for the contour.
const SPARE_COLUMNS: usize = 16;

/// Settings for [`CallScanner`].
#[derive(Clone, Debug, PartialEq)]
pub struct CallScanParams {
    /// Lowest peak frequency a bat call can have, in Hz (default 16 kHz,
    /// which keeps most free-tailed bats).
    ///
    /// Lowering it also lowers the bottom of the detection band (kept
    /// [`BAND_BELOW_MIN_BAT_HZ`] below it, at most 15 kHz) and the top of the
    /// audible band (kept [`AUDIBLE_BELOW_BAND_HZ`] below the band, at most
    /// 12 kHz). Without that, a call below 15 kHz would be outside the band,
    /// and its own sound in the audible band would mark it as an audible
    /// sound. Set it near 9 kHz to find free-tailed bats calling near 11 kHz;
    /// expect more false detections from birds and insects.
    pub min_bat_hz: f32,
    /// Top of the detection band, in Hz. `None`: 1 kHz below Nyquist.
    pub band_hi_hz: Option<f32>,
    /// dB above the floor that starts a call.
    pub threshold_db: f32,
    /// dB above the floor that keeps a call going.
    pub release_db: f32,
    /// Calls shorter than this are dropped.
    pub min_call_ms: f32,
    /// Quiet gaps shorter than this join the calls on either side.
    pub merge_gap_ms: f32,
    /// Sounds longer than this are background, not calls.
    pub max_call_ms: f32,
    /// How far below the peak a call's frequency range reaches, in dB.
    pub range_db: f32,
    /// How far beyond the call's range the contour is searched, each side,
    /// in Hz.
    pub contour_margin_hz: f32,
    /// Contour points more than this far below the contour's loudest point
    /// are dropped, in dB.
    pub contour_range_db: f32,
    pub trains: TrainParams,
}

impl Default for CallScanParams {
    fn default() -> Self {
        Self {
            min_bat_hz: DEFAULT_MIN_BAT_HZ,
            band_hi_hz: None,
            threshold_db: DEFAULT_THRESHOLD_DB,
            release_db: DEFAULT_RELEASE_DB,
            min_call_ms: DEFAULT_MIN_CALL_MS,
            merge_gap_ms: DEFAULT_MERGE_GAP_MS,
            max_call_ms: DEFAULT_MAX_CALL_MS,
            range_db: DEFAULT_RANGE_DB,
            contour_margin_hz: DEFAULT_CONTOUR_MARGIN_HZ,
            contour_range_db: DEFAULT_CONTOUR_RANGE_DB,
            trains: TrainParams::default(),
        }
    }
}

impl CallScanParams {
    /// Bottom of the detection band, in Hz.
    pub fn band_lo_hz(&self) -> f32 {
        DEFAULT_BAND_LO_HZ.min(self.min_bat_hz - BAND_BELOW_MIN_BAT_HZ)
    }

    /// The audible band checked during each call, in Hz.
    pub fn audible_band_hz(&self) -> (f32, f32) {
        let (lo, hi) = DEFAULT_AUDIBLE_BAND_HZ;
        (lo, hi.min(self.band_lo_hz() - AUDIBLE_BELOW_BAND_HZ))
    }

    /// The detection band at this sample rate, or `None` when the rate is too
    /// low to hold any of it.
    pub fn band_hz(&self, sample_rate: f32) -> Option<(f32, f32)> {
        let top = default_band_hi_hz(sample_rate);
        let hi = self.band_hi_hz.map_or(top, |hi| hi.min(top));
        let lo = self.band_lo_hz().max(0.0);
        (hi > lo).then_some((lo, hi))
    }
}

/// One point of a call's contour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContourPoint {
    /// Column centre, in seconds from the first sample scanned.
    pub time_s: f64,
    /// Loudest frequency in that column, in Hz, between bins.
    pub freq_hz: f32,
    /// Its level above the noise floor, in dB.
    pub db: f32,
}

/// A detection with its kind and contour.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectedCall {
    /// What the detector reported, in samples from the first sample scanned.
    pub event: CallEvent,
    /// `event.start_sample` in seconds.
    pub start_s: f64,
    /// `event.end_sample` in seconds.
    pub end_s: f64,
    pub kind: CallKind,
    /// Loudest frequency per column, in time order. Can be empty for a call
    /// whose columns had already left the scanner's memory, which happens
    /// only for a call that ran across a [`CallScanner::gap`].
    pub contour: Vec<ContourPoint>,
}

impl DetectedCall {
    pub fn duration_ms(&self) -> f64 {
        (self.end_s - self.start_s) * 1000.0
    }
}

/// Everything a scan found.
#[derive(Clone, Debug, PartialEq)]
pub struct CallScanResult {
    /// In time order.
    pub calls: Vec<DetectedCall>,
    /// In time order of their first call.
    pub trains: Vec<CallTrain>,
    pub frame: FrameConfig,
    /// The detection band, or `None` when the sample rate was too low.
    pub band_hz: Option<(f32, f32)>,
}

/// The last columns, oldest first, in one flat buffer.
#[derive(Clone, Debug)]
struct ColumnHistory {
    bins: usize,
    centres: Vec<u64>,
    db: Vec<f32>,
    /// Slot the next column goes in.
    next: usize,
    len: usize,
}

impl ColumnHistory {
    fn new(bins: usize, capacity: usize) -> Self {
        Self {
            bins,
            centres: vec![0; capacity],
            db: vec![0.0; capacity * bins],
            next: 0,
            len: 0,
        }
    }

    fn capacity(&self) -> usize {
        self.centres.len()
    }

    fn push(&mut self, column: &Column<'_>) {
        let slot = self.next;
        self.centres[slot] = column.centre_sample;
        self.db[slot * self.bins..(slot + 1) * self.bins].copy_from_slice(column.db);
        self.next = (slot + 1) % self.capacity();
        self.len = (self.len + 1).min(self.capacity());
    }

    /// Columns with centres from `start` to `end`, oldest first.
    fn between(&self, start: u64, end: u64) -> impl Iterator<Item = (u64, &[f32])> + '_ {
        let cap = self.capacity();
        let first = (self.next + cap - self.len) % cap;
        (0..self.len)
            .map(move |i| (first + i) % cap)
            .filter(move |&slot| (start..=end).contains(&self.centres[slot]))
            .map(move |slot| {
                (
                    self.centres[slot],
                    &self.db[slot * self.bins..(slot + 1) * self.bins],
                )
            })
    }
}

/// How contours are traced.
#[derive(Clone, Copy, Debug)]
struct ContourSettings {
    sample_rate: f64,
    bin_hz: f32,
    margin_hz: f32,
    range_db: f32,
}

/// Trace a call's contour from the columns in `history`, with `floor(bin)`
/// giving each bin's noise floor in dB.
fn trace_contour(
    history: &ColumnHistory,
    event: &CallEvent,
    floor: impl Fn(usize) -> f32,
    s: &ContourSettings,
) -> Vec<ContourPoint> {
    let last = history.bins - 1;
    let lo = (((event.min_freq_hz - s.margin_hz) / s.bin_hz)
        .floor()
        .max(0.0) as usize)
        .min(last);
    let hi = (((event.max_freq_hz + s.margin_hz) / s.bin_hz)
        .ceil()
        .max(0.0) as usize)
        .min(last);
    // The floor is the same for every column of the call: it does not adapt
    // while a call is in progress.
    let floors: Vec<f32> = (0..=last).map(&floor).collect();
    let mut points: Vec<ContourPoint> = history
        .between(event.start_sample, event.end_sample)
        .map(|(centre, db)| {
            let excess = |k: usize| db[k] - floors[k];
            let k = (lo..=hi)
                .max_by(|&a, &b| excess(a).total_cmp(&excess(b)))
                .unwrap_or(lo);
            let (offset, level) = if k > 0 && k < last {
                parabola_peak(excess(k - 1), excess(k), excess(k + 1))
            } else {
                (0.0, excess(k))
            };
            ContourPoint {
                time_s: centre as f64 / s.sample_rate,
                freq_hz: (k as f32 + offset) * s.bin_hz,
                db: level,
            }
        })
        .collect();
    let loudest = points.iter().map(|p| p.db).fold(f32::MIN, f32::max);
    points.retain(|p| p.db >= loudest - s.range_db);
    points
}

/// Offset (in bins, within ±0.5) and height of the peak of a parabola
/// through three equally spaced values, the middle one highest. Returns the
/// middle value unchanged when it is not a peak.
fn parabola_peak(a: f32, b: f32, c: f32) -> (f32, f32) {
    let curve = a - 2.0 * b + c;
    if b < a || b < c || curve >= 0.0 {
        return (0.0, b);
    }
    let p = (0.5 * (a - c) / curve).clamp(-0.5, 0.5);
    (p, b - 0.25 * (a - c) * p)
}

/// Runs the call detector and classifier over a recording given in pieces;
/// see the module documentation.
#[derive(Debug)]
pub struct CallScanner {
    frame: FrameConfig,
    band_hz: Option<(f32, f32)>,
    analyzer: SpectrumAnalyzer,
    detector: CallDetector,
    classifier: CallClassifier,
    history: ColumnHistory,
    contour: ContourSettings,
    trains: TrainParams,
    calls: Vec<DetectedCall>,
}

impl CallScanner {
    pub fn new(sample_rate: u32, params: &CallScanParams) -> Self {
        let fs = sample_rate as f32;
        let frame = FrameConfig::for_rate(sample_rate);
        let mut analyzer = SpectrumAnalyzer::new(fs, frame.fft_size);
        analyzer.set_hop(frame.hop);
        let mut detector = CallDetector::new(fs, frame.fft_size);
        let band_hz = params.band_hz(fs);
        if let Some((lo, hi)) = band_hz {
            detector.set_band(lo, hi);
        }
        let (audible_lo, audible_hi) = params.audible_band_hz();
        detector.set_audible_band(audible_lo, audible_hi);
        detector.set_threshold_db(params.threshold_db);
        detector.set_release_db(params.release_db);
        detector.set_min_call_ms(params.min_call_ms);
        detector.set_merge_gap_ms(params.merge_gap_ms);
        detector.set_max_call_ms(params.max_call_ms);
        detector.set_range_db(params.range_db);
        let mut classifier = CallClassifier::new(fs, params.band_lo_hz());
        classifier.set_min_bat_hz(params.min_bat_hz);

        // A call is reported at most its longest length plus the merge gap
        // after it started.
        let span_ms = params.max_call_ms.max(0.0) + params.merge_gap_ms.max(0.0);
        let span_samples = span_ms as f64 * 0.001 * sample_rate as f64;
        let capacity = (span_samples / frame.hop as f64).ceil() as usize + SPARE_COLUMNS;
        let bins = frame.fft_size / 2 + 1;
        Self {
            frame,
            band_hz,
            analyzer,
            contour: ContourSettings {
                sample_rate: sample_rate as f64,
                bin_hz: detector.bin_hz(),
                margin_hz: params.contour_margin_hz.max(0.0),
                range_db: params.contour_range_db.max(0.0),
            },
            detector,
            classifier,
            history: ColumnHistory::new(bins, capacity),
            trains: params.trains.clone(),
            calls: Vec::new(),
        }
    }

    pub fn frame(&self) -> FrameConfig {
        self.frame
    }

    /// The detection band, or `None` when the sample rate is too low; then
    /// nothing is found.
    pub fn band_hz(&self) -> Option<(f32, f32)> {
        self.band_hz
    }

    /// Calls found so far, in time order. The first call of a train shows as
    /// a single until the train's second call arrives.
    pub fn calls(&self) -> &[DetectedCall] {
        &self.calls
    }

    /// Add the next samples, in a piece of any size.
    pub fn push(&mut self, samples: &[f32]) {
        if self.band_hz.is_none() {
            return;
        }
        let Self {
            analyzer,
            detector,
            classifier,
            history,
            contour,
            calls,
            ..
        } = self;
        analyzer.push(samples, |column| {
            history.push(column);
            if let Some(event) = detector.process(column) {
                record(event, detector, classifier, history, contour, calls);
            }
        });
    }

    /// `samples` samples are missing from here on (a piece that could not be
    /// read). Later calls keep their true times; see [`CallDetector::gap`].
    pub fn gap(&mut self, samples: u64) {
        if let Some(event) = self.detector.gap() {
            record(
                event,
                &self.detector,
                &mut self.classifier,
                &self.history,
                &self.contour,
                &mut self.calls,
            );
        }
        self.analyzer.skip(samples);
    }

    /// End the scan: report a call still in progress, and group the bat
    /// calls into trains.
    pub fn finish(mut self) -> CallScanResult {
        if let Some(event) = self.detector.flush() {
            record(
                event,
                &self.detector,
                &mut self.classifier,
                &self.history,
                &self.contour,
                &mut self.calls,
            );
        }
        let trains = group_trains(&self.calls, &self.trains);
        CallScanResult {
            calls: self.calls,
            trains,
            frame: self.frame,
            band_hz: self.band_hz,
        }
    }
}

/// Classify a detection, trace its contour and add it to `calls`.
fn record(
    event: CallEvent,
    detector: &CallDetector,
    classifier: &mut CallClassifier,
    history: &ColumnHistory,
    contour: &ContourSettings,
    calls: &mut Vec<DetectedCall>,
) {
    let points = trace_contour(history, &event, |k| detector.floor_db(k), contour);
    let verdict = classifier.classify(&event);
    if let Some(first) = verdict.earlier {
        if let Some(c) = calls
            .iter_mut()
            .rev()
            .find(|c| c.event.start_sample == first.start_sample)
        {
            c.kind = CallKind::Bat;
        }
    }
    calls.push(DetectedCall {
        event,
        start_s: event.start_sample as f64 / contour.sample_rate,
        end_s: event.end_sample as f64 / contour.sample_rate,
        kind: verdict.kind,
        contour: points,
    });
}

/// Scan mono samples already in memory.
pub fn scan_calls(samples: &[f32], sample_rate: u32, params: &CallScanParams) -> CallScanResult {
    let mut scanner = CallScanner::new(sample_rate, params);
    scanner.push(samples);
    scanner.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::call_detect::test_util::{add, faded_chirp, noise, sine};

    fn secs(rate: u32, s: f64) -> usize {
        (s * rate as f64) as usize
    }

    /// `count` calls from `f0` to `f1` Hz, `len_ms` long, every `every_s`
    /// from `from_s`, in noise.
    fn calls_in_noise(
        rate: u32,
        total_s: f64,
        from_s: f64,
        every_s: f64,
        count: usize,
        (f0, f1): (f32, f32),
        len_ms: f64,
    ) -> (Vec<f32>, Vec<f64>) {
        let fs = rate as f32;
        let mut x = noise(0.003, secs(rate, total_s), 5);
        let call = faded_chirp(fs, f0, f1, 0.1, secs(rate, len_ms * 0.001));
        let mut starts = Vec::new();
        for i in 0..count {
            // Spacing varies by a few ms, as a bat's does.
            let t = from_s + i as f64 * every_s + 0.003 * (i * 7 % 5) as f64;
            add(&mut x, secs(rate, t), &call);
            starts.push(t);
        }
        (x, starts)
    }

    fn assert_found(result: &CallScanResult, starts: &[f64], what: &str) {
        let bats: Vec<&DetectedCall> = result
            .calls
            .iter()
            .filter(|c| c.kind == CallKind::Bat)
            .collect();
        assert_eq!(bats.len(), starts.len(), "{what}: {:#?}", result.calls);
        for (c, &t) in bats.iter().zip(starts) {
            assert!(
                (c.start_s - t).abs() < 0.002,
                "{what}: call at {} s, expected {t} s",
                c.start_s
            );
        }
    }

    #[test]
    fn the_first_call_of_a_train_is_marked_a_bat_call() {
        let rate = 192_000;
        let (x, starts) = calls_in_noise(rate, 1.5, 0.3, 0.1, 8, (80_000.0, 40_000.0), 5.0);
        let result = scan_calls(&x, rate, &CallScanParams::default());
        assert_found(&result, &starts, "192 kHz");
        assert_eq!(result.trains.len(), 1);
        assert_eq!(result.trains[0].calls.len(), 8);
    }

    #[test]
    fn the_same_calls_are_found_at_96_and_384_khz() {
        for rate in [96_000, 384_000] {
            let (x, starts) = calls_in_noise(rate, 1.5, 0.3, 0.1, 8, (45_000.0, 25_000.0), 5.0);
            let result = scan_calls(&x, rate, &CallScanParams::default());
            assert_eq!(result.frame, FrameConfig::for_rate(rate));
            assert_found(&result, &starts, &format!("{rate} Hz"));
            let c = &result.calls[3];
            assert!(
                (20_000.0..=30_000.0).contains(&c.event.min_freq_hz)
                    && (40_000.0..=50_000.0).contains(&c.event.max_freq_hz),
                "{rate} Hz: {c:?}"
            );
        }
    }

    #[test]
    fn horseshoe_calls_above_100_khz_are_found_at_384_khz() {
        // Constant-frequency calls at 110 kHz, 40 ms long, every 120 ms: above
        // the original's fixed 95 kHz band top.
        let rate = 384_000;
        let fs = rate as f32;
        let mut x = noise(0.003, secs(rate, 1.5), 9);
        let call = faded_chirp(fs, 110_000.0, 110_000.0, 0.1, secs(rate, 0.04));
        let mut starts = Vec::new();
        for i in 0..6 {
            let t = 0.3 + 0.12 * i as f64 + 0.004 * (i % 3) as f64;
            add(&mut x, secs(rate, t), &call);
            starts.push(t);
        }
        let result = scan_calls(&x, rate, &CallScanParams::default());
        assert_found(&result, &starts, "horseshoe");
        assert!((result.calls[0].event.peak_freq_hz - 110_000.0).abs() < 400.0);
    }

    #[test]
    fn a_lower_lowest_bat_frequency_finds_free_tails_near_11_khz() {
        let rate = 96_000;
        let (x, starts) = calls_in_noise(rate, 2.0, 0.3, 0.25, 6, (13_000.0, 10_500.0), 12.0);
        let default = scan_calls(&x, rate, &CallScanParams::default());
        assert!(
            default.calls.iter().all(|c| c.kind != CallKind::Bat),
            "{:#?}",
            default.calls
        );
        let params = CallScanParams {
            min_bat_hz: 9_000.0,
            ..CallScanParams::default()
        };
        assert_eq!(params.band_lo_hz(), 8_000.0);
        assert_eq!(params.audible_band_hz(), (2_000.0, 5_000.0));
        let low = scan_calls(&x, rate, &params);
        assert_found(&low, &starts, "free-tail");
    }

    #[test]
    fn the_contour_follows_a_downward_sweep() {
        let rate = 192_000;
        let (x, starts) = calls_in_noise(rate, 1.0, 0.3, 0.1, 4, (80_000.0, 40_000.0), 5.0);
        let result = scan_calls(&x, rate, &CallScanParams::default());
        assert_eq!(result.calls.len(), 4);
        let c = &result.calls[1];
        let pts = &c.contour;
        assert!(pts.len() >= 4, "{pts:?}");
        // In time order, within the call, falling in frequency.
        for w in pts.windows(2) {
            assert!(w[1].time_s > w[0].time_s);
            assert!(w[1].freq_hz < w[0].freq_hz, "{pts:?}");
        }
        assert!(pts[0].time_s >= c.start_s && pts.last().unwrap().time_s <= c.end_s);
        // Each point is near the chirp's frequency at that moment; the window
        // is 2.7 ms long, so allow for the sweep's spread over it.
        let t0 = starts[1];
        for p in pts {
            let at = ((p.time_s - t0) / 0.005).clamp(0.0, 1.0);
            let expect = 80_000.0 - 40_000.0 * at as f32;
            assert!(
                (p.freq_hz - expect).abs() < 6_000.0,
                "{} s: {} Hz, expected about {expect}",
                p.time_s,
                p.freq_hz
            );
        }
        let loudest = pts.iter().map(|p| p.db).fold(f32::MIN, f32::max);
        assert!(pts
            .iter()
            .all(|p| p.db >= loudest - DEFAULT_CONTOUR_RANGE_DB));
    }

    #[test]
    fn a_constant_frequency_contour_is_flat_between_bins() {
        // 50.2 kHz sits between bins 133 and 134 (375 Hz bins).
        let rate = 192_000;
        let fs = rate as f32;
        let mut x = noise(0.003, secs(rate, 1.0), 4);
        add(
            &mut x,
            secs(rate, 0.4),
            &sine(fs, 50_200.0, 0.1, secs(rate, 0.02)),
        );
        let result = scan_calls(&x, rate, &CallScanParams::default());
        assert_eq!(result.calls.len(), 1);
        let pts = &result.calls[0].contour;
        assert!(pts.len() >= 15, "{pts:?}");
        for p in &pts[3..pts.len() - 3] {
            assert!((p.freq_hz - 50_200.0).abs() < 60.0, "{p:?}");
        }
    }

    #[test]
    fn pieces_of_any_size_give_the_same_result() {
        let rate = 192_000;
        let (x, _) = calls_in_noise(rate, 1.5, 0.3, 0.1, 8, (80_000.0, 40_000.0), 5.0);
        let whole = scan_calls(&x, rate, &CallScanParams::default());
        let mut scanner = CallScanner::new(rate, &CallScanParams::default());
        for piece in x.chunks(9_973) {
            scanner.push(piece);
        }
        assert_eq!(scanner.finish(), whole);
    }

    #[test]
    fn a_rate_too_low_for_the_band_finds_nothing() {
        let params = CallScanParams::default();
        assert_eq!(params.band_hz(32_000.0), None);
        let result = scan_calls(&noise(0.1, 32_000, 3), 32_000, &params);
        assert!(result.calls.is_empty() && result.band_hz.is_none());
    }

    #[test]
    fn the_history_holds_the_longest_call() {
        let rate = 192_000;
        let fs = rate as f32;
        // A 95 ms tone burst: just under the longest call.
        let mut x = noise(0.003, secs(rate, 1.0), 6);
        add(
            &mut x,
            secs(rate, 0.4),
            &sine(fs, 40_000.0, 0.1, secs(rate, 0.095)),
        );
        let result = scan_calls(&x, rate, &CallScanParams::default());
        assert_eq!(result.calls.len(), 1, "{:#?}", result.calls);
        let c = &result.calls[0];
        let first = c.contour.first().unwrap().time_s;
        let last = c.contour.last().unwrap().time_s;
        assert!(
            first - c.start_s < 0.003 && c.end_s - last < 0.003,
            "{first} to {last} for {c:?}"
        );
    }
}

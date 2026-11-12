//! Call sketches: idealised drawings of a species' echolocation calls and call
//! train, drawn beside a recording on the same time and frequency scale.
//!
//! The library is authored in the `bat-call-sketches` repository (a sibling
//! checkout at `../batchi-extras/bat-call-sketches`, format in its FORMAT.md).
//! Its `tools/bundle.py` checks each species file and writes
//! `src/data/call_sketches.json`, which is compiled in here.

use serde::Deserialize;
use std::sync::OnceLock;

/// A `min` / `typical` / `max` range. Any part may be missing.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Range {
    pub min: Option<f64>,
    pub typical: Option<f64>,
    pub max: Option<f64>,
}

impl Range {
    /// The typical value, or the middle of min and max.
    pub fn value(&self) -> Option<f64> {
        self.typical.or(match (self.min, self.max) {
            (Some(a), Some(b)) => Some((a + b) / 2.0),
            _ => None,
        })
    }

    /// "42 to 51" when min and max are known, otherwise the typical value.
    pub fn label(&self) -> Option<String> {
        match (self.min, self.max) {
            (Some(a), Some(b)) if a < b => Some(format!("{} to {}", fmt_num(a), fmt_num(b))),
            _ => self.value().map(fmt_num),
        }
    }
}

fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Harmonic {
    pub multiple: f64,
    #[serde(default)]
    pub relative_db: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SketchCall {
    pub name: String,
    #[serde(default)]
    pub duration_ms: Option<Range>,
    #[serde(default)]
    pub start_khz: Option<Range>,
    #[serde(default)]
    pub end_khz: Option<Range>,
    #[serde(default)]
    pub peak_khz: Option<Range>,
    /// Characteristic frequency (Fc): the flattest part of the call, as
    /// AnaBat-based keys report it. Kept apart from peak frequency.
    #[serde(default)]
    pub characteristic_khz: Option<Range>,
    /// `[time 0..1, kHz]` points. Empty for a social call known only by its
    /// frequencies: it is listed but not drawn.
    #[serde(default)]
    pub shape: Vec<[f64; 2]>,
    #[serde(default)]
    pub harmonics: Vec<Harmonic>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct SketchTrain {
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub interval_ms: Option<Range>,
    #[serde(default)]
    pub sequence: Vec<String>,
    /// How long a whole approach or buzz lasts.
    #[serde(default)]
    pub length_ms: Option<Range>,
}

impl SketchTrain {
    fn is_ramp(&self) -> bool {
        self.pattern == "ramp"
    }
}

/// Which kind of call a form describes. Search calls are the sketch's
/// top-level calls; the others come from its `forms` list.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FormKind {
    #[default]
    Search,
    Approach,
    Buzz,
    Social,
}

impl FormKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Search => "Search calls",
            Self::Approach => "Approach",
            Self::Buzz => "Feeding buzz",
            Self::Social => "Social",
        }
    }

    /// The prefix this form's fields take in `sources[].fields`.
    fn field_prefix(self) -> Option<&'static str> {
        match self {
            Self::Search => None,
            Self::Approach => Some("approach"),
            Self::Buzz => Some("buzz"),
            Self::Social => Some("social"),
        }
    }
}

/// An approach, buzz or social form as stored in a sketch's `forms` list.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CallForm {
    pub kind: FormKind,
    #[serde(default)]
    pub label: Option<String>,
    pub calls: Vec<SketchCall>,
    #[serde(default)]
    pub train: Option<SketchTrain>,
    #[serde(default)]
    pub one_source: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Which form to show: its kind, and its label for a social call.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FormKey {
    pub kind: FormKind,
    pub label: Option<String>,
}

/// One kind of call of a species, borrowed from its sketch: search calls
/// (the top-level fields) or one of its `forms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Form<'a> {
    pub kind: FormKind,
    pub label: Option<&'a str>,
    pub calls: &'a [SketchCall],
    pub train: Option<&'a SketchTrain>,
    pub one_source: &'a [String],
    pub notes: Option<&'a str>,
}

impl<'a> Form<'a> {
    pub fn key(&self) -> FormKey {
        FormKey {
            kind: self.kind,
            label: self.label.map(str::to_string),
        }
    }

    /// "Search", "Feeding buzz", or a social call's own label ("Song").
    pub fn name(&self) -> &'a str {
        self.label.unwrap_or(self.kind.label())
    }

    /// The calls in train order (the first call alone when there is no train).
    pub fn sequence(&self) -> Vec<&'a SketchCall> {
        let names = self.train.map(|t| t.sequence.as_slice()).unwrap_or(&[]);
        let seq: Vec<&SketchCall> = names
            .iter()
            .filter_map(|n| self.calls.iter().find(|c| &c.name == n))
            .collect();
        if seq.is_empty() {
            self.calls.first().into_iter().collect()
        } else {
            seq
        }
    }

    /// True when there is enough to draw it to scale: every call in the train
    /// has a length and a shape.
    pub fn drawable(&self) -> bool {
        let seq = self.sequence();
        !seq.is_empty()
            && seq.iter().all(|c| {
                c.duration_ms.as_ref().and_then(Range::value).is_some() && c.shape.len() >= 2
            })
    }

    /// Lowest and highest kHz of the calls in the train (their shape points).
    pub fn khz_extent(&self) -> Option<(f64, f64)> {
        self.sequence()
            .iter()
            .flat_map(|c| c.shape.iter().map(|p| p[1]))
            .fold(None, |acc, k| match acc {
                None => Some((k, k)),
                Some((lo, hi)) => Some((f64::min(lo, k), f64::max(hi, k))),
            })
    }

    /// True when `field` (such as `"duration_ms"`) rests on one source.
    pub fn is_one_source(&self, field: &str) -> bool {
        self.one_source.iter().any(|f| f == field)
    }

    /// Start-to-start time between calls, seconds (the middle of the range
    /// for a ramp).
    pub fn interval_s(&self) -> Option<f64> {
        self.train?
            .interval_ms
            .as_ref()?
            .value()
            .map(|ms| ms / 1000.0)
    }

    /// How long the whole sequence lasts, seconds.
    pub fn length_s(&self) -> Option<f64> {
        self.train?
            .length_ms
            .as_ref()?
            .value()
            .map(|ms| ms / 1000.0)
    }

    pub fn is_ramp(&self) -> bool {
        self.train.is_some_and(SketchTrain::is_ramp)
    }

    /// The sources that support a number of this form.
    pub fn sources<'s>(&self, all: &'s [Source]) -> Vec<&'s Source> {
        let prefix = match (self.kind.field_prefix(), self.label) {
            (None, _) => None,
            (Some(p), Some(l)) if self.kind == FormKind::Social => Some(format!("{p}.{l}.")),
            (Some(p), _) => Some(format!("{p}.")),
        };
        all.iter()
            .filter(|s| match &prefix {
                None => s.fields.is_empty() || s.fields.iter().any(|f| !f.contains('.')),
                Some(p) => s.fields.iter().any(|f| f.starts_with(p.as_str())),
            })
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Source {
    pub citation: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CallSketch {
    /// Same as the bat book's species id.
    pub id: String,
    pub scientific_name: String,
    #[serde(default)]
    pub common_name: Option<String>,
    pub call_type: String,
    #[serde(default)]
    pub status: Option<String>,
    pub calls: Vec<SketchCall>,
    #[serde(default)]
    pub train: Option<SketchTrain>,
    #[serde(default)]
    pub notes: Option<String>,
    pub sources: Vec<Source>,
    /// Fields whose numbers come from one source only so far, such as
    /// `duration_ms`. They apply to every call in the sketch.
    #[serde(default)]
    pub one_source: Vec<String>,
    /// Approach, feeding buzz and social calls.
    #[serde(default)]
    pub forms: Vec<CallForm>,
}

impl CallSketch {
    /// Search calls: the top-level calls, train and one_source.
    pub fn search(&self) -> Form<'_> {
        Form {
            kind: FormKind::Search,
            label: None,
            calls: &self.calls,
            train: self.train.as_ref(),
            one_source: &self.one_source,
            notes: None,
        }
    }

    /// Search calls first, then the forms in file order.
    pub fn forms(&self) -> Vec<Form<'_>> {
        std::iter::once(self.search())
            .chain(self.forms.iter().map(|f| Form {
                kind: f.kind,
                label: f.label.as_deref(),
                calls: &f.calls,
                train: f.train.as_ref(),
                one_source: &f.one_source,
                notes: f.notes.as_deref(),
            }))
            .collect()
    }

    /// The form with this key, if the species has it.
    pub fn form(&self, key: &FormKey) -> Option<Form<'_>> {
        self.forms().into_iter().find(|f| &f.key() == key)
    }

    /// True when the search calls can be drawn.
    pub fn drawable(&self) -> bool {
        self.search().drawable()
    }
}

const BUNDLE: &str = include_str!("../data/call_sketches.json");

/// Every sketch in the bundled library.
pub fn all() -> &'static [CallSketch] {
    static SKETCHES: OnceLock<Vec<CallSketch>> = OnceLock::new();
    SKETCHES.get_or_init(|| match serde_json::from_str(BUNDLE) {
        Ok(v) => v,
        Err(e) => {
            log::error!("call_sketches.json: {e}");
            Vec::new()
        }
    })
}

/// The sketch for a bat book species id.
pub fn for_species(id: &str) -> Option<&'static CallSketch> {
    all().iter().find(|s| s.id == id)
}

/// A point on a smooth curve through the shape points (Catmull-Rom), at time
/// `t` from 0 to 1. Returns kHz.
pub fn shape_at(shape: &[[f64; 2]], t: f64) -> f64 {
    match shape.len() {
        0 => 0.0,
        1 => shape[0][1],
        n => {
            let t = t.clamp(0.0, 1.0);
            let i = shape.windows(2).position(|w| t <= w[1][0]).unwrap_or(n - 2);
            let p1 = shape[i];
            let p2 = shape[i + 1];
            let p0 = if i > 0 { shape[i - 1] } else { p1 };
            let p3 = if i + 2 < n { shape[i + 2] } else { p2 };
            let span = (p2[0] - p1[0]).max(1e-9);
            let u = ((t - p1[0]) / span).clamp(0.0, 1.0);
            // Tangents scaled to the segment, so uneven time steps don't overshoot.
            let m1 = if i > 0 {
                (p2[1] - p0[1]) / (p2[0] - p0[0]).max(1e-9) * span
            } else {
                p2[1] - p1[1]
            };
            let m2 = if i + 2 < n {
                (p3[1] - p1[1]) / (p3[0] - p1[0]).max(1e-9) * span
            } else {
                p2[1] - p1[1]
            };
            let u2 = u * u;
            let u3 = u2 * u;
            (2.0 * u3 - 3.0 * u2 + 1.0) * p1[1]
                + (u3 - 2.0 * u2 + u) * m1
                + (-2.0 * u3 + 3.0 * u2) * p2[1]
                + (u3 - u2) * m2
        }
    }
}

/// One call placed in the strip.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedCall<'a> {
    pub call: &'a SketchCall,
    /// In a ramp, the last call of the sequence and how far (0 to 1) this
    /// call has moved towards it.
    pub towards: Option<(&'a SketchCall, f64)>,
    /// Left edge, CSS px from the strip's left edge.
    pub x: f64,
    /// Width, CSS px.
    pub width: f64,
}

impl PlacedCall<'_> {
    /// kHz at time `t` (0 to 1) of this call, blending a ramp's two shapes.
    pub fn khz_at(&self, t: f64) -> f64 {
        let a = shape_at(&self.call.shape, t);
        match self.towards {
            Some((b, f)) => a + (shape_at(&b.shape, t) - a) * f,
            None => a,
        }
    }
}

/// Where the calls of a train go in a strip of at most `max_width` CSS px, at
/// `px_per_s` CSS px per second (the main view's scale).
#[derive(Clone, Debug, PartialEq)]
pub struct StripLayout<'a> {
    pub calls: Vec<PlacedCall<'a>>,
    /// Width the strip needs, CSS px.
    pub width: f64,
    /// The narrowest call is under 2 px wide at this zoom.
    pub too_small: bool,
    /// The sequence is longer than the strip, so its end isn't drawn.
    pub cut_off: bool,
}

/// Space left of the first call and right of the last, CSS px.
pub const STRIP_PAD: f64 = 12.0;
/// Narrowest strip, CSS px: room for its heading.
pub const STRIP_MIN_WIDTH: f64 = 150.0;
/// Search and social calls: a few calls show the spacing of the train.
const MAX_REPEATS: usize = 4;
/// Approach and buzz: every call of the sequence, up to this many.
const MAX_SEQUENCE_CALLS: usize = 400;

fn ms_s(r: Option<&Range>) -> Option<f64> {
    r.and_then(Range::value).map(|ms| ms / 1000.0)
}

pub fn layout<'a>(form: Form<'a>, px_per_s: f64, max_width: f64) -> StripLayout<'a> {
    let seq = form.sequence();
    let dur = |c: &SketchCall| ms_s(c.duration_ms.as_ref()).unwrap_or(0.0);
    let inner_max = (max_width - 2.0 * STRIP_PAD).max(0.0);
    let mut calls = Vec::new();
    let mut x = STRIP_PAD;
    let mut cut_off = false;

    if form.is_ramp() && seq.len() == 2 {
        // Calls change steadily from the first to the last, and the interval
        // shrinks from its max to its min, over the sequence's length.
        let (first, last) = (seq[0], seq[1]);
        let iv = form.train.and_then(|t| t.interval_ms.as_ref());
        let iv_max = iv.and_then(|r| r.max.or(r.typical)).unwrap_or(0.0) / 1000.0;
        let iv_min = iv
            .and_then(|r| r.min.or(r.typical))
            .unwrap_or(iv_max * 1000.0)
            / 1000.0;
        // Without a length, assume the time the intervals take to shrink
        // over 8 calls.
        let length = form.length_s().unwrap_or((iv_max + iv_min) / 2.0 * 8.0);
        let mut t = 0.0;
        while calls.len() < MAX_SEQUENCE_CALLS {
            let f = if length > 0.0 {
                (t / length).min(1.0)
            } else {
                1.0
            };
            let w = (dur(first) + (dur(last) - dur(first)) * f) * px_per_s;
            if !calls.is_empty() && x + w - STRIP_PAD > inner_max {
                cut_off = true;
                break;
            }
            calls.push(PlacedCall {
                call: first,
                towards: Some((last, f)),
                x,
                width: w,
            });
            let step = iv_max + (iv_min - iv_max) * f;
            if step <= 0.0 || t >= length {
                break;
            }
            t += step;
            x += step * px_per_s;
        }
    } else if !seq.is_empty() {
        let interval = form.interval_s();
        let limit = match (form.kind, form.length_s(), interval) {
            // A sequence of known length: all its calls (a social call's
            // elements, an approach, a buzz).
            (FormKind::Approach | FormKind::Buzz | FormKind::Social, Some(len), Some(iv))
                if iv > 0.0 =>
            {
                ((len / iv).floor() as usize + 1).clamp(1, MAX_SEQUENCE_CALLS)
            }
            (FormKind::Approach | FormKind::Buzz, None, Some(_)) => MAX_SEQUENCE_CALLS,
            _ => MAX_REPEATS,
        };
        for k in 0..limit {
            let call = seq[k % seq.len()];
            let w = dur(call) * px_per_s;
            if k > 0 && x + w - STRIP_PAD > inner_max {
                cut_off = limit > MAX_REPEATS;
                break;
            }
            calls.push(PlacedCall {
                call,
                towards: None,
                x,
                width: w,
            });
            match interval {
                Some(iv) => x += iv * px_per_s,
                None => break,
            }
        }
    }

    let narrowest = calls.iter().map(|c| c.width).fold(f64::INFINITY, f64::min);
    let too_small = !narrowest.is_finite() || narrowest < 2.0;
    let right = calls.last().map(|c| c.x + c.width).unwrap_or(STRIP_PAD) + STRIP_PAD;
    StripLayout {
        calls,
        width: right
            .max(STRIP_MIN_WIDTH)
            .min(max_width.max(STRIP_MIN_WIDTH)),
        too_small,
        cut_off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_parses_and_ids_are_unique() {
        let all = all();
        assert!(!all.is_empty());
        let mut ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        for s in all {
            assert!(s.sources.len() >= 2, "{} has fewer than 2 sources", s.id);
        }
    }

    /// The strip finds a sketch by bat book id, so a sketch whose id is in no
    /// book is never shown.
    #[test]
    fn every_sketch_id_is_in_a_bat_book() {
        use super::super::{data::get_manifest, types::BatBookRegion};
        let book_ids: std::collections::HashSet<&str> = BatBookRegion::ALL
            .iter()
            .flat_map(|&r| get_manifest(r).entries)
            .map(|e| e.id)
            .collect();
        for s in all() {
            assert!(
                book_ids.contains(s.id.as_str()),
                "sketch {} is in no bat book",
                s.id
            );
        }
    }

    #[test]
    fn shape_passes_through_its_points() {
        let shape = [[0.0, 70.0], [0.15, 55.0], [0.4, 48.0], [1.0, 45.0]];
        for p in shape {
            assert!((shape_at(&shape, p[0]) - p[1]).abs() < 1e-9);
        }
        // Monotonic data stays between its neighbours.
        let mid = shape_at(&shape, 0.7);
        assert!(mid < 48.0 && mid > 45.0, "{mid}");
    }

    fn sketch(interval: Option<f64>, dur: f64) -> CallSketch {
        CallSketch {
            id: "x".into(),
            scientific_name: "X y".into(),
            common_name: None,
            call_type: "FM".into(),
            status: None,
            calls: vec![SketchCall {
                name: "search".into(),
                duration_ms: Some(Range {
                    typical: Some(dur),
                    ..Default::default()
                }),
                start_khz: None,
                end_khz: None,
                peak_khz: None,
                characteristic_khz: None,
                shape: vec![[0.0, 60.0], [1.0, 40.0]],
                harmonics: vec![],
            }],
            train: interval.map(|iv| SketchTrain {
                pattern: "regular".into(),
                interval_ms: Some(Range {
                    typical: Some(iv),
                    ..Default::default()
                }),
                sequence: vec!["search".into()],
                length_ms: None,
            }),
            notes: None,
            sources: vec![],
            one_source: vec![],
            forms: vec![],
        }
    }

    #[test]
    fn layout_fits_calls_at_the_view_scale() {
        // 1000 px per second: 6 ms call = 6 px, 100 ms apart = 100 px.
        let s = sketch(Some(100.0), 6.0);
        let l = layout(s.search(), 1000.0, 400.0);
        assert_eq!(l.calls.len(), 4);
        assert_eq!(l.calls[1].x - l.calls[0].x, 100.0);
        assert_eq!(l.calls[0].width, 6.0);
        assert!(!l.too_small);

        // A narrow strip keeps as many as fit, at least one.
        let l = layout(s.search(), 1000.0, 150.0);
        assert_eq!(l.calls.len(), 2);
        let l = layout(s.search(), 10_000.0, 150.0);
        assert_eq!(l.calls.len(), 1);
    }

    #[test]
    fn layout_flags_calls_too_small_to_see() {
        let s = sketch(Some(100.0), 6.0);
        assert!(layout(s.search(), 200.0, 400.0).too_small);
    }

    #[test]
    fn khz_extent_covers_every_shape_point() {
        let s = sketch(Some(100.0), 6.0);
        assert_eq!(s.search().khz_extent(), Some((40.0, 60.0)));
    }

    #[test]
    fn no_interval_draws_one_call() {
        let s = sketch(None, 20.0);
        assert_eq!(layout(s.search(), 1000.0, 400.0).calls.len(), 1);
    }

    const FORMS: &str = r#"{
      "id": "x", "scientific_name": "X y", "call_type": "FM-QCF",
      "calls": [{"name": "search", "duration_ms": {"typical": 6}, "shape": [[0, 60], [1, 45]]}],
      "train": {"pattern": "regular", "interval_ms": {"typical": 90}, "sequence": ["search"]},
      "forms": [
        {"kind": "approach",
         "calls": [{"name": "first", "duration_ms": {"typical": 3}, "shape": [[0, 70], [1, 47]]},
                   {"name": "last", "duration_ms": {"typical": 1}, "shape": [[0, 80], [1, 40]]}],
         "train": {"pattern": "ramp", "sequence": ["first", "last"],
                   "interval_ms": {"min": 12, "max": 60}, "length_ms": {"typical": 300}},
         "one_source": ["interval_ms"]},
        {"kind": "buzz",
         "calls": [{"name": "buzz", "duration_ms": {"typical": 0.6}, "shape": [[0, 60], [1, 30]]}],
         "train": {"pattern": "regular", "sequence": ["buzz"],
                   "interval_ms": {"typical": 5.5}, "length_ms": {"typical": 300}}},
        {"kind": "social", "label": "Song",
         "calls": [{"name": "song", "duration_ms": {"typical": 35}, "peak_khz": {"typical": 18}}]}
      ],
      "sources": [
        {"citation": "A", "fields": ["duration_ms", "buzz.interval_ms"]},
        {"citation": "B", "fields": ["social.Song.peak_khz"]},
        {"citation": "C", "fields": ["approach.interval_ms"]}
      ]
    }"#;

    #[test]
    fn forms_come_after_search_in_file_order() {
        let s: CallSketch = serde_json::from_str(FORMS).unwrap();
        let names: Vec<&str> = s.forms().iter().map(|f| f.name()).collect();
        assert_eq!(names, ["Search calls", "Approach", "Feeding buzz", "Song"]);
        let song = s.forms()[3];
        assert!(
            !song.drawable(),
            "a call without a shape is listed, not drawn"
        );
        assert!(s.forms()[1].is_one_source("interval_ms"));
        assert!(!s.search().is_one_source("interval_ms"));
        let key = s.forms()[2].key();
        assert_eq!(s.form(&key).map(|f| f.kind), Some(FormKind::Buzz));
    }

    #[test]
    fn form_sources_follow_field_prefixes() {
        let s: CallSketch = serde_json::from_str(FORMS).unwrap();
        let cites = |f: Form| -> Vec<String> {
            f.sources(&s.sources)
                .iter()
                .map(|x| x.citation.clone())
                .collect()
        };
        assert_eq!(cites(s.search()), ["A"]);
        assert_eq!(cites(s.forms()[1]), ["C"]);
        assert_eq!(cites(s.forms()[2]), ["A"]);
        assert_eq!(cites(s.forms()[3]), ["B"]);
    }

    #[test]
    fn buzz_draws_every_call_of_its_length() {
        let s: CallSketch = serde_json::from_str(FORMS).unwrap();
        // 10 000 px/s: 300 ms buzz = 3000 px; 5.5 ms apart; 55 calls.
        let l = layout(s.forms()[2], 10_000.0, 4000.0);
        assert_eq!(l.calls.len(), 55);
        assert!(!l.cut_off);
        let l = layout(s.forms()[2], 10_000.0, 500.0);
        assert!(l.cut_off && l.calls.len() < 55);
    }

    #[test]
    fn ramp_shrinks_calls_and_intervals() {
        let s: CallSketch = serde_json::from_str(FORMS).unwrap();
        let l = layout(s.forms()[1], 1000.0, 10_000.0);
        let first = &l.calls[0];
        let last = l.calls.last().unwrap();
        assert!((first.width - 3.0).abs() < 1e-9);
        assert!(last.width < 1.5, "{}", last.width);
        let gap0 = l.calls[1].x - l.calls[0].x;
        let n = l.calls.len();
        let gap_end = l.calls[n - 1].x - l.calls[n - 2].x;
        assert!(gap0 > gap_end, "{gap0} {gap_end}");
        // Shape blends from the first call towards the last.
        assert!((first.khz_at(0.0) - 70.0).abs() < 1e-9);
        assert!(last.khz_at(0.0) > 75.0);
    }
}

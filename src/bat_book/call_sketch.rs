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
    /// `[time 0..1, kHz]` points.
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
}

impl CallSketch {
    /// True when there is enough to draw it to scale: every call in the train
    /// has a length and a shape.
    pub fn drawable(&self) -> bool {
        let seq = self.sequence();
        !seq.is_empty()
            && seq.iter().all(|c| {
                c.duration_ms.as_ref().and_then(Range::value).is_some() && c.shape.len() >= 2
            })
    }

    /// The calls in train order (the first call alone when there is no train).
    pub fn sequence(&self) -> Vec<&SketchCall> {
        let names = self
            .train
            .as_ref()
            .map(|t| t.sequence.as_slice())
            .unwrap_or(&[]);
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

    /// Start-to-start time between calls, seconds.
    pub fn interval_s(&self) -> Option<f64> {
        self.train
            .as_ref()?
            .interval_ms
            .as_ref()?
            .value()
            .map(|ms| ms / 1000.0)
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
    /// Left edge, CSS px from the strip's left edge.
    pub x: f64,
    /// Width, CSS px.
    pub width: f64,
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
}

/// Space left of the first call and right of the last, CSS px.
pub const STRIP_PAD: f64 = 12.0;
/// Narrowest strip, CSS px: room for its heading.
pub const STRIP_MIN_WIDTH: f64 = 150.0;

pub fn layout(sketch: &CallSketch, px_per_s: f64, max_width: f64) -> StripLayout<'_> {
    let seq = sketch.sequence();
    let durations: Vec<f64> = seq
        .iter()
        .map(|c| c.duration_ms.as_ref().and_then(Range::value).unwrap_or(0.0) / 1000.0)
        .collect();
    let narrowest = durations.iter().cloned().fold(f64::INFINITY, f64::min) * px_per_s;
    let too_small = !narrowest.is_finite() || narrowest < 2.0;

    let interval = sketch.interval_s();
    let mut calls = Vec::new();
    let mut x = STRIP_PAD;
    let inner_max = (max_width - 2.0 * STRIP_PAD).max(0.0);
    // Up to 4 calls, as many as fit; always at least one.
    for k in 0..4 {
        let i = k % seq.len().max(1);
        let Some(call) = seq.get(i) else { break };
        let w = durations[i] * px_per_s;
        if k > 0 && x + w - STRIP_PAD > inner_max {
            break;
        }
        calls.push(PlacedCall { call, x, width: w });
        match interval {
            Some(iv) => x += iv * px_per_s,
            None => break,
        }
    }
    let right = calls.last().map(|c| c.x + c.width).unwrap_or(STRIP_PAD) + STRIP_PAD;
    StripLayout {
        calls,
        width: right
            .max(STRIP_MIN_WIDTH)
            .min(max_width.max(STRIP_MIN_WIDTH)),
        too_small,
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
            }),
            notes: None,
            sources: vec![],
        }
    }

    #[test]
    fn layout_fits_calls_at_the_view_scale() {
        // 1000 px per second: 6 ms call = 6 px, 100 ms apart = 100 px.
        let s = sketch(Some(100.0), 6.0);
        let l = layout(&s, 1000.0, 400.0);
        assert_eq!(l.calls.len(), 4);
        assert_eq!(l.calls[1].x - l.calls[0].x, 100.0);
        assert_eq!(l.calls[0].width, 6.0);
        assert!(!l.too_small);

        // A narrow strip keeps as many as fit, at least one.
        let l = layout(&s, 1000.0, 150.0);
        assert_eq!(l.calls.len(), 2);
        let l = layout(&s, 10_000.0, 150.0);
        assert_eq!(l.calls.len(), 1);
    }

    #[test]
    fn layout_flags_calls_too_small_to_see() {
        let s = sketch(Some(100.0), 6.0);
        assert!(layout(&s, 200.0, 400.0).too_small);
    }

    #[test]
    fn no_interval_draws_one_call() {
        let s = sketch(None, 20.0);
        assert_eq!(layout(&s, 1000.0, 400.0).calls.len(), 1);
    }
}

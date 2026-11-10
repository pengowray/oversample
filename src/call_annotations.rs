//! Turning the call detector's results into annotations, and the names the UI
//! gives each kind of detection.
//!
//! Annotations made here carry the tag `auto=<detector id>`
//! ([`DETECTOR_ID`]), so a later run can find and replace them, including
//! those from earlier versions of the detector (`auto=calls-1`). Annotations
//! without such a tag are never removed.

use crate::annotations::{Annotation, AnnotationId, AnnotationKind, Region};
use oversample_core::annotation_tags::{self, Tag, AUTO, CALL, TRAIN};
use oversample_core::dsp::call_detect::{
    is_detector_id, CallKind, CallTrain, DetectedCall, DETECTOR_ID,
};

/// What to add as annotations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotateWhat {
    /// One region per call.
    Calls,
    /// One region per train.
    Trains,
    /// Train regions with their calls as children. Calls in no train are
    /// added at the top level.
    TrainsWithCalls,
}

/// Annotations ready to add, with how many of each.
#[derive(Clone, Debug)]
pub struct BuiltAnnotations {
    pub annotations: Vec<Annotation>,
    pub calls: usize,
    pub trains: usize,
}

/// Name of a kind of detection, as shown in the list.
pub fn kind_label(kind: CallKind) -> &'static str {
    match kind {
        CallKind::Bat => "Bat call",
        CallKind::Single => "Single call",
        CallKind::Tone => "Tone",
        CallKind::Pulses => "Periodic pulse",
        CallKind::Broadband => "Broadband noise",
        CallKind::TooLow => "Low frequency",
        CallKind::Audible => "Audible sound",
    }
}

/// Label of a call annotation: `Call: 45.2 kHz`, or the kind for a detection
/// that is not a bat call (`Tone: 22.1 kHz`).
pub fn call_label(call: &DetectedCall) -> String {
    let kind = match call.kind {
        CallKind::Bat => "Call",
        other => kind_label(other),
    };
    format!("{kind}: {:.1} kHz", call.event.peak_freq_hz / 1000.0)
}

/// Label of a train annotation: `Train: 8 calls, median 45.2 kHz`.
pub fn train_label(train: &CallTrain, calls: &[DetectedCall]) -> String {
    let n = train.calls.len();
    format!(
        "Train: {n} {}, median {:.1} kHz",
        if n == 1 { "call" } else { "calls" },
        train.median_peak_hz(calls) / 1000.0
    )
}

/// The stored tags for an annotation of this kind (`call` or `train`) made by
/// the detector.
fn detector_tags(kind: &str) -> Vec<String> {
    vec![
        kind.to_string(),
        Tag::with_value(AUTO, DETECTOR_ID).to_stored(),
    ]
}

/// True for an annotation made by any version of this detector (tagged
/// `auto=calls-1`, `auto=calls-2` and so on).
pub fn is_from_detector(annotation: &Annotation) -> bool {
    annotation_tags::value_of(&annotation.tags, None, AUTO).is_some_and(|v| is_detector_id(&v))
}

/// The `auto` tags on annotations from this detector, each once, in order,
/// as the user sees them (`#auto=calls-1`).
pub fn detector_tags_in(annotations: &[Annotation]) -> Vec<String> {
    let mut values: Vec<String> = annotations
        .iter()
        .filter_map(|a| annotation_tags::value_of(&a.tags, None, AUTO))
        .filter(|v| is_detector_id(v))
        .collect();
    values.sort();
    values.dedup();
    values
        .into_iter()
        .map(|v| format!("#{}", Tag::with_value(AUTO, &v).to_stored()))
        .collect()
}

/// Build annotations for `calls` and `trains` (the trains' indices point
/// into `calls`). With `bat_calls_only`, calls of other kinds are left out.
/// `now` is the creation time to record; `new_id` makes each annotation's id.
pub fn build_call_annotations(
    calls: &[DetectedCall],
    trains: &[CallTrain],
    what: AnnotateWhat,
    bat_calls_only: bool,
    now: &str,
    mut new_id: impl FnMut() -> AnnotationId,
) -> BuiltAnnotations {
    let wanted = |c: &DetectedCall| !bat_calls_only || c.kind == CallKind::Bat;
    let mut out = BuiltAnnotations {
        annotations: Vec::new(),
        calls: 0,
        trains: 0,
    };
    let mut make = |kind: AnnotationKind,
                    tag: &str,
                    parent: Option<AnnotationId>,
                    order: usize,
                    out: &mut BuiltAnnotations| {
        let id = new_id();
        out.annotations.push(Annotation {
            id: id.clone(),
            kind,
            created_at: now.to_string(),
            modified_at: now.to_string(),
            notes: None,
            parent_id: parent,
            sort_order: Some(order as f64),
            tags: detector_tags(tag),
            label_default: None,
        });
        id
    };
    let call_region = |c: &DetectedCall| {
        AnnotationKind::Region(Region {
            time_start: c.start_s,
            time_end: c.end_s,
            freq_low: Some(c.event.min_freq_hz as f64),
            freq_high: Some(c.event.max_freq_hz as f64),
            label: Some(call_label(c)),
            color: None,
            locked: None,
        })
    };

    if what != AnnotateWhat::Calls {
        for (t, train) in trains.iter().enumerate() {
            let region = AnnotationKind::Region(Region {
                time_start: train.start_s,
                time_end: train.end_s,
                freq_low: Some(train.min_freq_hz as f64),
                freq_high: Some(train.max_freq_hz as f64),
                label: Some(train_label(train, calls)),
                color: None,
                locked: None,
            });
            let train_id = make(region, TRAIN, None, t, &mut out);
            out.trains += 1;
            if what == AnnotateWhat::TrainsWithCalls {
                for (order, &i) in train.calls.iter().enumerate() {
                    let Some(c) = calls.get(i) else { continue };
                    make(
                        call_region(c),
                        CALL,
                        Some(train_id.clone()),
                        order,
                        &mut out,
                    );
                    out.calls += 1;
                }
            }
        }
    }
    if what != AnnotateWhat::Trains {
        let in_train: std::collections::HashSet<usize> = if what == AnnotateWhat::TrainsWithCalls {
            trains
                .iter()
                .flat_map(|t| t.calls.iter().copied())
                .collect()
        } else {
            Default::default()
        };
        let base = trains.len();
        let loose = calls
            .iter()
            .enumerate()
            .filter(|(i, c)| !in_train.contains(i) && wanted(c));
        for (order, (_, c)) in loose.enumerate() {
            make(call_region(c), CALL, None, base + order, &mut out);
            out.calls += 1;
        }
    }
    out
}

/// How many call and train annotations [`build_call_annotations`] would make,
/// without building them.
pub fn count_call_annotations(
    calls: &[DetectedCall],
    trains: &[CallTrain],
    what: AnnotateWhat,
    bat_calls_only: bool,
) -> (usize, usize) {
    let wanted = |c: &DetectedCall| !bat_calls_only || c.kind == CallKind::Bat;
    match what {
        AnnotateWhat::Calls => (calls.iter().filter(|c| wanted(c)).count(), 0),
        AnnotateWhat::Trains => (0, trains.len()),
        AnnotateWhat::TrainsWithCalls => {
            let in_train: std::collections::HashSet<usize> = trains
                .iter()
                .flat_map(|t| t.calls.iter().copied())
                .collect();
            let loose = calls
                .iter()
                .enumerate()
                .filter(|(i, c)| !in_train.contains(i) && wanted(c))
                .count();
            (in_train.len() + loose, trains.len())
        }
    }
}

/// How many annotations were made by this detector.
pub fn count_detector_annotations(annotations: &[Annotation]) -> usize {
    annotations.iter().filter(|a| is_from_detector(a)).count()
}

/// Remove every annotation made by this detector, and return how many were
/// removed. An annotation without the tag whose parent is removed moves to
/// that parent's nearest remaining ancestor, or to the top level.
pub fn remove_detector_annotations(annotations: &mut Vec<Annotation>) -> usize {
    use std::collections::HashMap;
    let removed: HashMap<AnnotationId, Option<AnnotationId>> = annotations
        .iter()
        .filter(|a| is_from_detector(a))
        .map(|a| (a.id.clone(), a.parent_id.clone()))
        .collect();
    if removed.is_empty() {
        return 0;
    }
    annotations.retain(|a| !removed.contains_key(&a.id));
    for a in annotations.iter_mut() {
        let mut parent = a.parent_id.clone();
        // Walk up past removed parents. The bound guards against a cycle.
        for _ in 0..=removed.len() {
            match parent.as_ref().and_then(|p| removed.get(p)) {
                Some(up) => parent = up.clone(),
                None => break,
            }
        }
        a.parent_id = parent;
    }
    removed.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oversample_core::dsp::call_detect::{group_trains, CallEvent, TrainParams};

    fn call(at_s: f64, khz: f32, kind: CallKind) -> DetectedCall {
        let fs = 192_000.0;
        let start = (at_s * fs) as u64;
        DetectedCall {
            event: CallEvent {
                start_sample: start,
                end_sample: start + 960,
                peak_freq_hz: khz * 1000.0,
                peak_db_above_floor: 30.0,
                audible_db_above_floor: -3.0,
                min_freq_hz: (khz - 2.0) * 1000.0,
                max_freq_hz: (khz + 10.0) * 1000.0,
            },
            start_s: start as f64 / fs,
            end_s: (start + 960) as f64 / fs,
            kind,
            contour: Vec::new(),
            steady_s: 0.0,
        }
    }

    /// Two trains of 3 bat calls, a tone between them, and a lone bat call.
    fn sample() -> (Vec<DetectedCall>, Vec<CallTrain>) {
        let mut calls = vec![
            call(1.0, 45.0, CallKind::Bat),
            call(1.1, 45.5, CallKind::Bat),
            call(1.2, 44.8, CallKind::Bat),
            call(1.5, 22.0, CallKind::Tone),
            call(3.0, 30.0, CallKind::Bat),
            call(3.1, 30.2, CallKind::Bat),
            call(3.2, 30.1, CallKind::Bat),
            call(6.0, 50.0, CallKind::Bat),
        ];
        calls.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
        let trains = group_trains(&calls, &TrainParams::default());
        (calls, trains)
    }

    fn ids() -> impl FnMut() -> AnnotationId {
        let mut n = 0;
        move || {
            n += 1;
            AnnotationId::from(format!("id{n}"))
        }
    }

    fn label(a: &Annotation) -> &str {
        match &a.kind {
            AnnotationKind::Region(r) => r.label.as_deref().unwrap_or(""),
            _ => "",
        }
    }

    #[test]
    fn calls_are_bat_calls_only_by_default() {
        let (calls, trains) = sample();
        let b = build_call_annotations(&calls, &trains, AnnotateWhat::Calls, true, "t", ids());
        assert_eq!((b.calls, b.trains), (7, 0));
        assert!(b.annotations.iter().all(|a| a.parent_id.is_none()));
        assert!(b
            .annotations
            .iter()
            .all(|a| a.tags == ["call".to_string(), format!("auto={DETECTOR_ID}")]));
        assert_eq!(label(&b.annotations[0]), "Call: 45.0 kHz");

        let all = build_call_annotations(&calls, &trains, AnnotateWhat::Calls, false, "t", ids());
        assert_eq!(all.calls, 8);
        assert!(all.annotations.iter().any(|a| label(a) == "Tone: 22.0 kHz"));
    }

    #[test]
    fn trains_span_their_calls() {
        let (calls, trains) = sample();
        let b = build_call_annotations(&calls, &trains, AnnotateWhat::Trains, true, "t", ids());
        assert_eq!((b.calls, b.trains), (0, 2));
        let a = &b.annotations[0];
        assert_eq!(a.tags, ["train".to_string(), format!("auto={DETECTOR_ID}")]);
        assert_eq!(label(a), "Train: 3 calls, median 45.0 kHz");
        let AnnotationKind::Region(r) = &a.kind else {
            panic!("not a region")
        };
        assert_eq!(r.time_start, calls[0].start_s);
        assert_eq!(r.time_end, calls[2].end_s);
        assert_eq!(r.freq_low, Some(42_800.0));
        assert_eq!(r.freq_high, Some(55_500.0));
    }

    #[test]
    fn trains_with_calls_nest_the_calls_and_keep_lone_calls_at_the_top() {
        let (calls, trains) = sample();
        let b = build_call_annotations(
            &calls,
            &trains,
            AnnotateWhat::TrainsWithCalls,
            true,
            "t",
            ids(),
        );
        assert_eq!((b.calls, b.trains), (7, 2));
        let train_ids: Vec<_> = b
            .annotations
            .iter()
            .filter(|a| annotation_tags::has_flag(&a.tags, TRAIN))
            .map(|a| a.id.clone())
            .collect();
        for t in &train_ids {
            let children = b
                .annotations
                .iter()
                .filter(|a| a.parent_id.as_ref() == Some(t))
                .count();
            assert_eq!(children, 3);
        }
        let lone: Vec<_> = b
            .annotations
            .iter()
            .filter(|a| a.parent_id.is_none() && annotation_tags::has_flag(&a.tags, CALL))
            .collect();
        assert_eq!(lone.len(), 1);
        assert_eq!(label(lone[0]), "Call: 50.0 kHz");
    }

    #[test]
    fn running_again_replaces_only_the_detectors_annotations() {
        let (calls, trains) = sample();
        let mut set = build_call_annotations(
            &calls,
            &trains,
            AnnotateWhat::TrainsWithCalls,
            true,
            "t",
            ids(),
        )
        .annotations;
        let train_id = set[0].id.clone();
        // A hand-made annotation inside the first train, and one at the top.
        let mut hand = set[1].clone();
        hand.id = AnnotationId::from("hand-inside");
        hand.tags = vec!["call".into(), "species=Myotis daubentonii".into()];
        hand.parent_id = Some(train_id);
        set.push(hand);
        let mut top = set[1].clone();
        top.id = AnnotationId::from("hand-top");
        top.tags = Vec::new();
        top.parent_id = None;
        set.push(top);

        assert_eq!(count_detector_annotations(&set), 9);
        assert_eq!(remove_detector_annotations(&mut set), 9);
        assert_eq!(set.len(), 2);
        assert!(set.iter().all(|a| a.parent_id.is_none()));
        assert_eq!(remove_detector_annotations(&mut set), 0);
    }

    #[test]
    fn annotations_from_an_earlier_detector_version_are_replaced_too() {
        let (calls, trains) = sample();
        let mut set =
            build_call_annotations(&calls, &trains, AnnotateWhat::Calls, true, "t", ids())
                .annotations;
        set[0].tags = vec!["call".into(), "auto=calls-1".into()];
        let mut other = set[1].clone();
        other.id = AnnotationId::from("other-detector");
        other.tags = vec!["call".into(), "auto=birdnet-1".into()];
        set.push(other);
        assert_eq!(
            detector_tags_in(&set),
            ["#auto=calls-1".to_string(), format!("#auto={DETECTOR_ID}")]
        );
        assert_eq!(count_detector_annotations(&set), 7);
        assert_eq!(remove_detector_annotations(&mut set), 7);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn counts_match_what_is_built() {
        let (calls, trains) = sample();
        for what in [
            AnnotateWhat::Calls,
            AnnotateWhat::Trains,
            AnnotateWhat::TrainsWithCalls,
        ] {
            for bat_only in [true, false] {
                let b = build_call_annotations(&calls, &trains, what, bat_only, "t", ids());
                assert_eq!(
                    count_call_annotations(&calls, &trains, what, bat_only),
                    (b.calls, b.trains),
                    "{what:?} {bat_only}"
                );
                assert_eq!(b.annotations.len(), b.calls + b.trains);
            }
        }
    }

    #[test]
    fn kind_labels_are_distinct() {
        let kinds = [
            CallKind::Bat,
            CallKind::Single,
            CallKind::Tone,
            CallKind::Pulses,
            CallKind::Broadband,
            CallKind::TooLow,
            CallKind::Audible,
        ];
        let names: std::collections::HashSet<_> = kinds.iter().map(|&k| kind_label(k)).collect();
        assert_eq!(names.len(), kinds.len());
    }
}

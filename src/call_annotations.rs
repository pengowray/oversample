//! Turning the call detector's results into annotations, and the names the UI
//! gives each kind of detection.
//!
//! Annotations made here carry the tag `auto=<detector id>`
//! ([`DETECTOR_ID`]), so a later run can find and replace them, including
//! those from earlier versions of the detector (`auto=calls-1`). Annotations
//! without such a tag are never removed.

use oversample_core::erased_sort::SortErased;
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
        CallKind::PeriodicPulse => "Periodic pulse",
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
    values.sort_erased();
    values.dedup();
    values
        .into_iter()
        .map(|v| format!("#{}", Tag::with_value(AUTO, &v).to_stored()))
        .collect()
}

/// A choice in "Annotate Calls and Trains" that the pointer or keyboard focus
/// is on. The spectrogram outlines the regions this choice would add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnnotatePreview {
    pub what: AnnotateWhat,
    pub bat_calls_only: bool,
}

/// Which detection a planned region comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionSource {
    /// Index into the calls.
    Call(usize),
    /// Index into the trains.
    Train(usize),
}

/// One region that [`build_call_annotations`] turns into an annotation.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedRegion {
    pub source: RegionSource,
    pub time_start: f64,
    pub time_end: f64,
    pub freq_low: f64,
    pub freq_high: f64,
    /// For a call inside a train: the train's index in the planned list.
    pub parent: Option<usize>,
    /// Position among the annotations with the same parent.
    pub sort_order: usize,
}

/// The regions [`build_call_annotations`] makes for `calls` and `trains`, in
/// the same order, without labels or ids. The Annotate preview draws these.
/// With `bat_calls_only`, calls of other kinds are left out. Trains contain
/// only bat calls, so the calls nested in a train are always kept.
pub fn plan_call_regions(
    calls: &[DetectedCall],
    trains: &[CallTrain],
    what: AnnotateWhat,
    bat_calls_only: bool,
) -> Vec<PlannedRegion> {
    let wanted = |c: &DetectedCall| !bat_calls_only || c.kind == CallKind::Bat;
    let call_region =
        |i: usize, c: &DetectedCall, parent: Option<usize>, order: usize| PlannedRegion {
            source: RegionSource::Call(i),
            time_start: c.start_s,
            time_end: c.end_s,
            freq_low: c.event.min_freq_hz as f64,
            freq_high: c.event.max_freq_hz as f64,
            parent,
            sort_order: order,
        };
    let mut out = Vec::new();
    if what != AnnotateWhat::Calls {
        for (t, train) in trains.iter().enumerate() {
            let train_at = out.len();
            out.push(PlannedRegion {
                source: RegionSource::Train(t),
                time_start: train.start_s,
                time_end: train.end_s,
                freq_low: train.min_freq_hz as f64,
                freq_high: train.max_freq_hz as f64,
                parent: None,
                sort_order: t,
            });
            if what == AnnotateWhat::TrainsWithCalls {
                for (order, &i) in train.calls.iter().enumerate() {
                    let Some(c) = calls.get(i) else { continue };
                    out.push(call_region(i, c, Some(train_at), order));
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
        for (order, (i, c)) in loose.enumerate() {
            out.push(call_region(i, c, None, base + order));
        }
    }
    out
}

/// Build annotations for `calls` and `trains` (the trains' indices point
/// into `calls`), one per region from [`plan_call_regions`]. `now` is the
/// creation time to record; `new_id` makes each annotation's id.
pub fn build_call_annotations(
    calls: &[DetectedCall],
    trains: &[CallTrain],
    what: AnnotateWhat,
    bat_calls_only: bool,
    now: &str,
    mut new_id: impl FnMut() -> AnnotationId,
) -> BuiltAnnotations {
    let plan = plan_call_regions(calls, trains, what, bat_calls_only);
    let mut out = BuiltAnnotations {
        annotations: Vec::with_capacity(plan.len()),
        calls: 0,
        trains: 0,
    };
    for p in &plan {
        let (label, tag) = match p.source {
            RegionSource::Call(i) => {
                out.calls += 1;
                (call_label(&calls[i]), CALL)
            }
            RegionSource::Train(t) => {
                out.trains += 1;
                (train_label(&trains[t], calls), TRAIN)
            }
        };
        // A parent is always planned before its children.
        let parent_id = p.parent.map(|at| out.annotations[at].id.clone());
        out.annotations.push(Annotation {
            id: new_id(),
            kind: AnnotationKind::Region(Region {
                time_start: p.time_start,
                time_end: p.time_end,
                freq_low: Some(p.freq_low),
                freq_high: Some(p.freq_high),
                label: Some(label),
                color: None,
                locked: None,
            }),
            created_at: now.to_string(),
            modified_at: now.to_string(),
            notes: None,
            parent_id,
            sort_order: Some(p.sort_order as f64),
            tags: detector_tags(tag),
            label_default: None,
            play: None,
        });
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
    let plan = plan_call_regions(calls, trains, what, bat_calls_only);
    let trains = plan
        .iter()
        .filter(|p| matches!(p.source, RegionSource::Train(_)))
        .count();
    (plan.len() - trains, trains)
}

/// How many annotations were made by this detector.
pub fn count_detector_annotations(annotations: &[Annotation]) -> usize {
    annotations.iter().filter(|a| is_from_detector(a)).count()
}

/// Remove every annotation made by this detector, and return how many were
/// removed. An annotation without the tag whose parent is removed moves to
/// that parent's nearest remaining ancestor, or to the top level.
pub fn remove_detector_annotations(annotations: &mut Vec<Annotation>) -> usize {
    remove_annotations_where(annotations, is_from_detector)
}

/// Remove every annotation `remove` picks, and return how many were removed.
/// An annotation that is kept and whose parent is removed moves to that
/// parent's nearest remaining ancestor, or to the top level.
pub fn remove_annotations_where(
    annotations: &mut Vec<Annotation>,
    remove: impl Fn(&Annotation) -> bool,
) -> usize {
    use std::collections::HashMap;
    let removed: HashMap<AnnotationId, Option<AnnotationId>> = annotations
        .iter()
        .filter(|a| remove(a))
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
        calls.sort_erased_by(|a, b| a.start_s.total_cmp(&b.start_s));
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
    fn planned_regions_match_the_built_annotations() {
        let (calls, trains) = sample();
        for what in [
            AnnotateWhat::Calls,
            AnnotateWhat::Trains,
            AnnotateWhat::TrainsWithCalls,
        ] {
            for bat_only in [true, false] {
                let plan = plan_call_regions(&calls, &trains, what, bat_only);
                let built = build_call_annotations(&calls, &trains, what, bat_only, "t", ids());
                assert_eq!(plan.len(), built.annotations.len(), "{what:?} {bat_only}");
                for (p, a) in plan.iter().zip(&built.annotations) {
                    let AnnotationKind::Region(r) = &a.kind else {
                        panic!("not a region")
                    };
                    assert_eq!(
                        (r.time_start, r.time_end, r.freq_low, r.freq_high),
                        (
                            p.time_start,
                            p.time_end,
                            Some(p.freq_low),
                            Some(p.freq_high)
                        )
                    );
                    let parent_id = p.parent.map(|at| built.annotations[at].id.clone());
                    assert_eq!(a.parent_id, parent_id);
                    assert_eq!(a.sort_order, Some(p.sort_order as f64));
                }
            }
        }
    }

    #[test]
    fn planned_trains_with_calls_nest_the_calls_inside_their_train() {
        let (calls, trains) = sample();
        let plan = plan_call_regions(&calls, &trains, AnnotateWhat::TrainsWithCalls, true);
        // Two trains of 3 calls, then the lone bat call; the tone is left out.
        let sources: Vec<_> = plan.iter().map(|p| p.source).collect();
        assert_eq!(
            sources,
            [
                RegionSource::Train(0),
                RegionSource::Call(0),
                RegionSource::Call(1),
                RegionSource::Call(2),
                RegionSource::Train(1),
                RegionSource::Call(4),
                RegionSource::Call(5),
                RegionSource::Call(6),
                RegionSource::Call(7),
            ]
        );
        for p in &plan {
            let Some(at) = p.parent else { continue };
            let train = &plan[at];
            assert!(matches!(train.source, RegionSource::Train(_)));
            assert!(train.time_start <= p.time_start && p.time_end <= train.time_end);
            assert!(train.freq_low <= p.freq_low && p.freq_high <= train.freq_high);
        }
        assert_eq!(plan[8].parent, None);

        // Without "Bat calls only", the tone is added at the top level too.
        let all = plan_call_regions(&calls, &trains, AnnotateWhat::TrainsWithCalls, false);
        assert_eq!(all.len(), 10);
        assert!(all
            .iter()
            .any(|p| p.source == RegionSource::Call(3) && p.parent.is_none()));
    }

    #[test]
    fn planned_calls_and_trains_have_no_parents() {
        let (calls, trains) = sample();
        let calls_only = plan_call_regions(&calls, &trains, AnnotateWhat::Calls, true);
        assert_eq!(calls_only.len(), 7);
        assert!(calls_only
            .iter()
            .all(|p| p.parent.is_none() && matches!(p.source, RegionSource::Call(_))));
        let trains_only = plan_call_regions(&calls, &trains, AnnotateWhat::Trains, true);
        assert_eq!(
            trains_only.iter().map(|p| p.source).collect::<Vec<_>>(),
            [RegionSource::Train(0), RegionSource::Train(1)]
        );
    }

    #[test]
    fn kind_labels_are_distinct() {
        let kinds = [
            CallKind::Bat,
            CallKind::Single,
            CallKind::Tone,
            CallKind::PeriodicPulse,
            CallKind::Broadband,
            CallKind::TooLow,
            CallKind::Audible,
        ];
        let names: std::collections::HashSet<_> = kinds.iter().map(|&k| kind_label(k)).collect();
        assert_eq!(names.len(), kinds.len());
    }
}

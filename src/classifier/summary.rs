//! What the Call detection tab shows for a BirdNET, BattyBirdNET or Perch run: the
//! windows at or above the minimum confidence, the species they name, and the
//! annotations made from them.

use super::{ClassifierRun, Method, SpeciesLabel};
use crate::annotations::{Annotation, AnnotationId, AnnotationKind, Region};
use oversample_core::annotation_tags::{self, Tag, AUTO, UNLIKELY_HERE};

/// How a window's top class compares with the minimum confidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    /// The top class is a species (or a named BirdNET sound) at or above the
    /// minimum confidence.
    Species,
    /// The top class is one of BattyBirdNET's classes that are not a species
    /// ("noise", "background", ...), at or above the minimum confidence.
    NotSpecies,
    /// The top class is below the minimum confidence.
    Below,
}

/// Which kind window `i` of `run` is.
pub fn window_kind(run: &ClassifierRun, i: usize, min_confidence: f32) -> WindowKind {
    let Some(top) = run.windows.get(i).and_then(|w| w.top.first()) else {
        return WindowKind::Below;
    };
    if top.confidence < min_confidence {
        WindowKind::Below
    } else if run.label(top).is_some_and(|l| l.is_species) {
        WindowKind::Species
    } else {
        WindowKind::NotSpecies
    }
}

/// The species that are some window's top class, best confidence first: the
/// order their strip colours are given out in. A window shows when its top
/// confidence is at or above the minimum, so the species shown at any minimum
/// are the start of this list, and raising the minimum never recolours the
/// species that stay.
pub fn colour_order(run: &ClassifierRun) -> Vec<u16> {
    let mut best: Vec<(u16, f32)> = Vec::new();
    for w in &run.windows {
        let Some(top) = w.top.first() else { continue };
        if !run.label(top).is_some_and(|l| l.is_species) {
            continue;
        }
        match best.iter_mut().find(|(c, _)| *c == top.class) {
            Some((_, b)) => *b = b.max(top.confidence),
            None => best.push((top.class, top.confidence)),
        }
    }
    best.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    best.into_iter().map(|(c, _)| c).collect()
}

/// One species in the "By species" list.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesRow {
    pub class: u16,
    /// Windows whose top class is this species, at or above the minimum.
    pub windows: usize,
    /// The length of those windows' cells (`ClassifierRun::cells`), in
    /// seconds: the time to show when windows overlap.
    pub secs: f64,
    pub best: f32,
    /// The window with the best confidence.
    pub best_window: usize,
}

/// Counts for the summary line. Each count of windows comes with the length
/// of their cells (`ClassifierRun::cells`) in seconds, shown instead of the
/// count when `overlapping`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// Species, most time first, then by best confidence.
    pub species: Vec<SpeciesRow>,
    /// Windows whose top class is a species at or above the minimum.
    pub species_windows: usize,
    pub species_secs: f64,
    /// Windows whose top class is not a species, at or above the minimum.
    pub not_species_windows: usize,
    pub not_species_secs: f64,
    pub below_windows: usize,
    pub below_secs: f64,
    /// The highest confidence of any species in any window, with its class:
    /// for the line shown when nothing reaches the minimum.
    pub highest: Option<(u16, f32)>,
    /// Some windows overlap (`ClassifierRun::overlapping`): show times, not
    /// counts of windows.
    pub overlapping: bool,
}

pub fn summarise(run: &ClassifierRun, min_confidence: f32) -> Summary {
    let mut out = Summary {
        overlapping: run.overlapping(),
        ..Summary::default()
    };
    let cells = run.cells();
    let mut rows: Vec<SpeciesRow> = Vec::new();
    for (i, w) in run.windows.iter().enumerate() {
        let secs = cells.get(i).map_or(0.0, |(s, e)| e - s);
        if let Some(best) = w
            .top
            .iter()
            .find(|s| run.label(s).is_some_and(|l| l.is_species))
        {
            if out.highest.is_none_or(|(_, c)| best.confidence > c) {
                out.highest = Some((best.class, best.confidence));
            }
        }
        match window_kind(run, i, min_confidence) {
            WindowKind::Below => {
                out.below_windows += 1;
                out.below_secs += secs;
            }
            WindowKind::NotSpecies => {
                out.not_species_windows += 1;
                out.not_species_secs += secs;
            }
            WindowKind::Species => {
                out.species_windows += 1;
                out.species_secs += secs;
                let top = w.top[0];
                match rows.iter_mut().find(|r| r.class == top.class) {
                    Some(r) => {
                        r.windows += 1;
                        r.secs += secs;
                        if top.confidence > r.best {
                            r.best = top.confidence;
                            r.best_window = i;
                        }
                    }
                    None => rows.push(SpeciesRow {
                        class: top.class,
                        windows: 1,
                        secs,
                        best: top.confidence,
                        best_window: i,
                    }),
                }
            }
        }
    }
    // Most windows first, or with overlapping windows most time first.
    rows.sort_by(|a, b| {
        let more = if out.overlapping {
            b.secs.total_cmp(&a.secs)
        } else {
            b.windows.cmp(&a.windows)
        };
        more.then(b.best.total_cmp(&a.best))
    });
    out.species = rows;
    out
}

/// A stretch of windows with the same top species, over their cells
/// (`ClassifierRun::cells`).
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub class: u16,
    pub start_s: f64,
    pub end_s: f64,
    pub best: f32,
    pub windows: usize,
}

/// One span per species window at or above the minimum, or, with `merge`, one
/// per run of neighbouring windows with the same top species. Each span
/// covers its windows' cells, so spans never overlap, even when the windows
/// do; for windows end to end the cells are the windows.
pub fn spans(run: &ClassifierRun, min_confidence: f32, merge: bool) -> Vec<Span> {
    let cells = run.cells();
    let mut out: Vec<Span> = Vec::new();
    let mut last: Option<usize> = None;
    for (i, w) in run.windows.iter().enumerate() {
        if window_kind(run, i, min_confidence) != WindowKind::Species {
            continue;
        }
        let top = w.top[0];
        let (start_s, end_s) = cells[i];
        let joins = merge
            && last == Some(i.wrapping_sub(1))
            && out.last().is_some_and(|s| s.class == top.class);
        if joins {
            let s = out.last_mut().expect("checked above");
            s.end_s = end_s;
            s.best = s.best.max(top.confidence);
            s.windows += 1;
        } else {
            out.push(Span {
                class: top.class,
                start_s,
                end_s,
                best: top.confidence,
                windows: 1,
            });
        }
        last = Some(i);
    }
    out
}

/// An amount of time for the lists, with a space before the unit: "2.5 s"
/// (one decimal under 10 s), "14 s", "1 min 4 s", "2 h 5 min".
pub fn time_amount(secs: f64) -> String {
    let secs = secs.max(0.0);
    if secs < 10.0 {
        let t = (secs * 10.0).round() / 10.0;
        if t.fract() == 0.0 {
            format!("{t:.0} s")
        } else {
            format!("{t:.1} s")
        }
    } else if secs < 59.5 {
        format!("{secs:.0} s")
    } else if secs < 3570.0 {
        let s = secs.round() as u64;
        match (s / 60, s % 60) {
            (m, 0) => format!("{m} min"),
            (m, r) => format!("{m} min {r} s"),
        }
    } else {
        let m = (secs / 60.0).round() as u64;
        match (m / 60, m % 60) {
            (h, 0) => format!("{h} h"),
            (h, r) => format!("{h} h {r} min"),
        }
    }
}

/// A step or window length as on the Step buttons: "3 s", "1 s", "0.5 s",
/// "1.5 s", "0.5625 s".
pub fn step_text(secs: f64) -> String {
    let t = (secs * 10_000.0).round() / 10_000.0;
    format!("{t} s")
}

/// The Step picked above the Analyse button, as saved
/// (`settings::keys::CLASSIFIER_STEP`): the model's whole window, or a step
/// in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepPick {
    Window,
    Secs(f64),
}

impl StepPick {
    /// The saved value: "window", or seconds ("1", "0.5"). Anything else is
    /// the whole window.
    pub fn from_stored(raw: Option<&str>) -> StepPick {
        match raw.map(str::trim).and_then(|r| r.parse::<f64>().ok()) {
            Some(s) if s.is_finite() && s > 0.0 => StepPick::Secs(s),
            _ => StepPick::Window,
        }
    }

    pub fn to_stored(self) -> String {
        match self {
            StepPick::Window => "window".to_string(),
            StepPick::Secs(s) => s.to_string(),
        }
    }

    /// The step this pick gives `method`, in seconds (`super::step_for`): a
    /// step that is not shorter than the method's window, or does not fit a
    /// whole number of times in it, is the whole window.
    pub fn secs(self, method: Method) -> f64 {
        match self {
            StepPick::Window => method.window_secs(),
            StepPick::Secs(s) => super::step_for(method, s),
        }
    }
}

/// The Step buttons for `method`: the whole window, 1 s and 0.5 s, as picks
/// with their step in seconds. None for BattyBirdNET, which always steps a
/// whole window.
pub fn step_choices(method: Method) -> Vec<(StepPick, f64)> {
    if method == Method::Bats {
        return Vec::new();
    }
    let mut out = vec![(StepPick::Window, method.window_secs())];
    for s in [1.0, 0.5] {
        if super::step_for(method, s) == s {
            out.push((StepPick::Secs(s), s));
        }
    }
    out
}

/// The `auto` tag value on annotations from this run: `birdnet` (V2.4),
/// `birdnet-v3`, `perch2`, or
/// `battybirdnet-uk` (one per region and `battybirdnet-all` for all regions,
/// so adding another region's results keeps the first region's annotations).
pub fn auto_tag_value(run: &ClassifierRun) -> String {
    match (run.method, run.region) {
        (Method::BirdNet, _) => "birdnet".to_string(),
        (Method::Perch, _) => "perch2".to_string(),
        (Method::BirdNetV3, _) => "birdnet-v3".to_string(),
        (Method::Bats, Some(r)) => format!("battybirdnet-{}", r.stem().to_lowercase()),
        (Method::Bats, None) => "battybirdnet".to_string(),
    }
}

/// True for an annotation tagged `auto=<value>`.
pub fn has_auto_tag(annotation: &Annotation, value: &str) -> bool {
    annotation_tags::value_of(&annotation.tags, None, AUTO).is_some_and(|v| v == value)
}

/// Annotations for `spans`: a time region labelled with the species and its
/// best confidence ("Song thrush 0.91"), with the scientific name in the notes.
/// A species `unlikely` returns true for also gets the `#unlikely-here` tag.
pub fn build_annotations(
    run: &ClassifierRun,
    spans: &[Span],
    now: &str,
    mut new_id: impl FnMut() -> AnnotationId,
    unlikely: impl Fn(&SpeciesLabel) -> bool,
) -> Vec<Annotation> {
    let tag = Tag::with_value(AUTO, auto_tag_value(run)).to_stored();
    let unlikely_tag = Tag::flag(UNLIKELY_HERE).to_stored();
    spans
        .iter()
        .enumerate()
        .map(|(order, s)| {
            let label = run.labels.get(s.class as usize);
            let common = label.map_or("Unknown", |l| l.common.as_str());
            let mut tags = vec![tag.clone()];
            if label.is_some_and(&unlikely) {
                tags.push(unlikely_tag.clone());
            }
            Annotation {
                id: new_id(),
                kind: AnnotationKind::Region(Region {
                    time_start: s.start_s,
                    time_end: s.end_s,
                    freq_low: None,
                    freq_high: None,
                    label: Some(format!("{common} {:.2}", s.best)),
                    color: None,
                    locked: None,
                }),
                created_at: now.to_string(),
                modified_at: now.to_string(),
                notes: label
                    .filter(|l| l.scientific != l.common)
                    .map(|l| l.scientific.clone()),
                parent_id: None,
                sort_order: Some(order as f64),
                tags,
                label_default: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::types::{parse_labels, WindowScores};
    use crate::classifier::{catalog, BatRegion, BatScope, Score};
    use std::sync::Arc;

    fn run(tops: &[(u16, f32)]) -> ClassifierRun {
        ClassifierRun {
            method: Method::Bats,
            region: Some(BatScope::Region(BatRegion::Uk)),
            backbone: &catalog::BACKBONE,
            head: None,
            source_rate: 384_000,
            model_rate: 256_000,
            window_secs: 0.5,
            step_secs: 0.5,
            spacing_secs: 0.5,
            labels: Arc::new(parse_labels(
                "Pipistrellus pipistrellus_Common pipistrelle\nPipistrellus pygmaeus_Soprano pipistrelle\nnoise\n",
            )),
            windows: tops
                .iter()
                .enumerate()
                .map(|(i, &(class, confidence))| WindowScores {
                    start_s: i as f64 * 0.5,
                    end_s: (i + 1) as f64 * 0.5,
                    slot: i as u32,
                    top: vec![Score { class, confidence }],
                })
                .collect(),
            whole_file: true,
            complete: true,
        }
    }

    #[test]
    fn colour_order_is_best_confidence_first_and_skips_noise() {
        let r = run(&[(0, 0.6), (2, 0.99), (1, 0.8), (0, 0.7)]);
        assert_eq!(colour_order(&r), vec![1, 0]);
    }

    #[test]
    fn summary_counts_each_kind() {
        let r = run(&[(0, 0.9), (0, 0.6), (1, 0.4), (2, 0.95), (1, 0.7)]);
        let s = summarise(&r, 0.5);
        assert_eq!(s.species_windows, 3);
        assert_eq!(s.not_species_windows, 1);
        assert_eq!(s.below_windows, 1);
        assert_eq!(s.species.len(), 2);
        assert_eq!(s.species[0].class, 0);
        assert_eq!(s.species[0].windows, 2);
        assert_eq!(s.species[0].best_window, 0);
        assert_eq!(s.highest, Some((0, 0.9)));
    }

    #[test]
    fn spans_merge_only_neighbours() {
        let r = run(&[(0, 0.9), (0, 0.6), (1, 0.4), (0, 0.7), (1, 0.8)]);
        let merged = spans(&r, 0.5, true);
        assert_eq!(merged.len(), 3);
        assert_eq!((merged[0].start_s, merged[0].end_s), (0.0, 1.0));
        assert_eq!(merged[0].windows, 2);
        assert_eq!(spans(&r, 0.5, false).len(), 4);
    }

    /// BirdNET windows (3 s) starting at `starts`, each with one top class.
    fn overlapping(starts: &[f64], tops: &[(u16, f32)]) -> ClassifierRun {
        let mut r = run(tops);
        r.method = Method::BirdNet;
        r.window_secs = 3.0;
        r.step_secs = 1.0;
        for (w, &s) in r.windows.iter_mut().zip(starts) {
            w.start_s = s;
            w.end_s = s + 3.0;
        }
        r
    }

    #[test]
    fn overlapping_windows_merge_over_their_cells() {
        // Every 1 s from 0 to 5; cells 0-2, 2-3, 3-4, 4-5, 5-6, 6-8.
        let r = overlapping(
            &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
            &[(0, 0.9), (0, 0.8), (1, 0.7), (1, 0.95), (1, 0.6), (0, 0.6)],
        );
        let merged = spans(&r, 0.5, true);
        let got: Vec<(u16, f64, f64)> = merged
            .iter()
            .map(|s| (s.class, s.start_s, s.end_s))
            .collect();
        assert_eq!(got, vec![(0, 0.0, 3.0), (1, 3.0, 6.0), (0, 6.0, 8.0)]);
        assert_eq!(merged[1].windows, 3);
        assert_eq!(merged[1].best, 0.95);
        // One per window: each window's cell, side by side, none overlapping.
        let single = spans(&r, 0.5, false);
        assert_eq!(single.len(), 6);
        for p in single.windows(2) {
            assert_eq!(p[0].end_s, p[1].start_s);
        }
        assert_eq!((single[0].start_s, single[0].end_s), (0.0, 2.0));
        assert_eq!(spans(&r, 0.65, true).len(), 2);
    }

    #[test]
    fn overlapping_windows_count_time() {
        let r = overlapping(
            &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
            &[(0, 0.9), (0, 0.8), (1, 0.7), (1, 0.95), (2, 0.2), (2, 0.99)],
        );
        let s = summarise(&r, 0.5);
        assert!(s.overlapping);
        assert_eq!((s.species_windows, s.below_windows), (4, 1));
        // Cells 0-2 and 2-3 for species 0, 3-4 and 4-5 for species 1.
        assert_eq!(s.species_secs, 5.0);
        assert_eq!(s.below_secs, 1.0);
        assert_eq!(s.not_species_secs, 2.0);
        assert_eq!(s.species[0].class, 0);
        assert_eq!(s.species[0].secs, 3.0);
        assert_eq!(s.species[1].secs, 2.0);
        // Windows end to end do not overlap; their cells are the windows.
        let s = summarise(&run(&[(0, 0.9), (0, 0.6)]), 0.5);
        assert!(!s.overlapping);
        assert_eq!(s.species_secs, 1.0);
    }

    #[test]
    fn overlapping_windows_give_annotations_side_by_side() {
        let r = overlapping(&[0.0, 1.0, 2.0], &[(0, 0.9), (1, 0.8), (0, 0.7)]);
        let new_id = || AnnotationId::from("a".to_string());
        let a = build_annotations(&r, &spans(&r, 0.5, false), "now", new_id, |_| false);
        let times: Vec<(f64, f64)> = a
            .iter()
            .map(|a| match &a.kind {
                AnnotationKind::Region(r) => (r.time_start, r.time_end),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(times, vec![(0.0, 2.0), (2.0, 3.0), (3.0, 5.0)]);
    }

    #[test]
    fn time_amounts() {
        assert_eq!(time_amount(0.0), "0 s");
        assert_eq!(time_amount(2.5), "2.5 s");
        assert_eq!(time_amount(3.0), "3 s");
        assert_eq!(time_amount(9.96), "10 s");
        assert_eq!(time_amount(14.25), "14 s");
        assert_eq!(time_amount(60.0), "1 min");
        assert_eq!(time_amount(64.4), "1 min 4 s");
        assert_eq!(time_amount(3599.0), "1 h");
        assert_eq!(time_amount(7500.0), "2 h 5 min");
        assert_eq!(step_text(3.0), "3 s");
        assert_eq!(step_text(0.5), "0.5 s");
        assert_eq!(step_text(1.5), "1.5 s");
        assert_eq!(step_text(0.5625), "0.5625 s");
    }

    #[test]
    fn saved_steps_fall_back_to_the_whole_window() {
        let pick = |raw: Option<&str>| StepPick::from_stored(raw);
        assert_eq!(pick(None), StepPick::Window);
        assert_eq!(pick(Some("window")), StepPick::Window);
        assert_eq!(pick(Some("junk")), StepPick::Window);
        assert_eq!(pick(Some("-1")), StepPick::Window);
        assert_eq!(pick(Some("1")), StepPick::Secs(1.0));
        for p in [StepPick::Window, StepPick::Secs(0.5), StepPick::Secs(1.0)] {
            assert_eq!(pick(Some(&p.to_stored())), p);
        }
        assert_eq!(StepPick::Window.secs(Method::Perch), 5.0);
        assert_eq!(StepPick::Secs(1.0).secs(Method::Perch), 1.0);
        assert_eq!(StepPick::Secs(0.5).secs(Method::BirdNetV3), 0.5);
        // Longer than the window, not fitting it, or bats: the whole window.
        assert_eq!(StepPick::Secs(4.0).secs(Method::BirdNet), 3.0);
        assert_eq!(StepPick::Secs(0.7).secs(Method::BirdNet), 3.0);
        assert_eq!(StepPick::Secs(0.5).secs(Method::Bats), 0.5625);
        let steps = |m| step_choices(m).iter().map(|c| c.1).collect::<Vec<_>>();
        assert_eq!(steps(Method::BirdNet), vec![3.0, 1.0, 0.5]);
        assert_eq!(steps(Method::Perch), vec![5.0, 1.0, 0.5]);
        assert!(steps(Method::Bats).is_empty());
    }

    #[test]
    fn annotations_carry_region_tag() {
        let r = run(&[(1, 0.8)]);
        let new_id = || AnnotationId::from("a".to_string());
        let a = build_annotations(&r, &spans(&r, 0.5, true), "now", new_id, |_| false);
        assert_eq!(a.len(), 1);
        assert!(has_auto_tag(&a[0], "battybirdnet-uk"));
        assert!(!annotation_tags::has_flag(&a[0].tags, UNLIKELY_HERE));
        let a = build_annotations(&r, &spans(&r, 0.5, true), "now", new_id, |_| true);
        assert!(annotation_tags::has_flag(&a[0].tags, UNLIKELY_HERE));
        assert!(has_auto_tag(&a[0], "battybirdnet-uk"));
        assert_eq!(a[0].notes.as_deref(), Some("Pipistrellus pygmaeus"));
    }

    /// BirdNET+ V3.0 gives a window of digital silence no scores: it counts
    /// as below the minimum and splits a span, and its annotations get their
    /// own tag, so they never replace BirdNET V2.4's.
    #[test]
    fn unscored_windows_count_as_below() {
        let mut r = run(&[(0, 0.9), (0, 0.8), (0, 0.9)]);
        r.method = Method::BirdNetV3;
        r.region = None;
        r.windows[1].top.clear();
        assert_eq!(window_kind(&r, 1, 0.0), WindowKind::Below);
        let s = summarise(&r, 0.25);
        assert_eq!((s.species_windows, s.below_windows), (2, 1));
        assert_eq!(spans(&r, 0.25, true).len(), 2);
        assert_eq!(auto_tag_value(&r), "birdnet-v3");
        r.method = Method::BirdNet;
        assert_eq!(auto_tag_value(&r), "birdnet");
    }
}

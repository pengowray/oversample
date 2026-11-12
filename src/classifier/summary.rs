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
    pub best: f32,
    /// The window with the best confidence.
    pub best_window: usize,
}

/// Counts for the summary line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// Species, most windows first, then by best confidence.
    pub species: Vec<SpeciesRow>,
    /// Windows whose top class is a species at or above the minimum.
    pub species_windows: usize,
    /// Windows whose top class is not a species, at or above the minimum.
    pub not_species_windows: usize,
    pub below_windows: usize,
    /// The highest confidence of any species in any window, with its class:
    /// for the line shown when nothing reaches the minimum.
    pub highest: Option<(u16, f32)>,
}

pub fn summarise(run: &ClassifierRun, min_confidence: f32) -> Summary {
    let mut out = Summary::default();
    let mut rows: Vec<SpeciesRow> = Vec::new();
    for (i, w) in run.windows.iter().enumerate() {
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
            WindowKind::Below => out.below_windows += 1,
            WindowKind::NotSpecies => out.not_species_windows += 1,
            WindowKind::Species => {
                out.species_windows += 1;
                let top = w.top[0];
                match rows.iter_mut().find(|r| r.class == top.class) {
                    Some(r) => {
                        r.windows += 1;
                        if top.confidence > r.best {
                            r.best = top.confidence;
                            r.best_window = i;
                        }
                    }
                    None => rows.push(SpeciesRow {
                        class: top.class,
                        windows: 1,
                        best: top.confidence,
                        best_window: i,
                    }),
                }
            }
        }
    }
    rows.sort_by(|a, b| b.windows.cmp(&a.windows).then(b.best.total_cmp(&a.best)));
    out.species = rows;
    out
}

/// A stretch of windows with the same top species.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub class: u16,
    pub start_s: f64,
    pub end_s: f64,
    pub best: f32,
    pub windows: usize,
}

/// One span per species window at or above the minimum, or, with `merge`, one
/// per run of neighbouring windows with the same top species.
pub fn spans(run: &ClassifierRun, min_confidence: f32, merge: bool) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut last: Option<usize> = None;
    for (i, w) in run.windows.iter().enumerate() {
        if window_kind(run, i, min_confidence) != WindowKind::Species {
            continue;
        }
        let top = w.top[0];
        let joins = merge
            && last == Some(i.wrapping_sub(1))
            && out.last().is_some_and(|s| s.class == top.class);
        if joins {
            let s = out.last_mut().expect("checked above");
            s.end_s = w.end_s;
            s.best = s.best.max(top.confidence);
            s.windows += 1;
        } else {
            out.push(Span {
                class: top.class,
                start_s: w.start_s,
                end_s: w.end_s,
                best: top.confidence,
                windows: 1,
            });
        }
        last = Some(i);
    }
    out
}

/// The `auto` tag value on annotations from this run: `birdnet`, `perch2`, or
/// `battybirdnet-uk` (one per region, so adding another region's results
/// keeps the first region's annotations).
pub fn auto_tag_value(run: &ClassifierRun) -> String {
    match (run.method, run.region) {
        (Method::BirdNet, _) => "birdnet".to_string(),
        (Method::Perch, _) => "perch2".to_string(),
        (Method::Bats, Some(r)) => format!("battybirdnet-{}", r.file_stem().to_lowercase()),
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
    use crate::classifier::{catalog, BatRegion, Score};
    use std::sync::Arc;

    fn run(tops: &[(u16, f32)]) -> ClassifierRun {
        ClassifierRun {
            method: Method::Bats,
            region: Some(BatRegion::Uk),
            backbone: &catalog::BACKBONE,
            head: None,
            source_rate: 384_000,
            model_rate: 256_000,
            window_secs: 0.5,
            labels: Arc::new(parse_labels(
                "Pipistrellus pipistrellus_Common pipistrelle\nPipistrellus pygmaeus_Soprano pipistrelle\nnoise\n",
            )),
            windows: tops
                .iter()
                .enumerate()
                .map(|(i, &(class, confidence))| WindowScores {
                    start_s: i as f64 * 0.5,
                    end_s: (i + 1) as f64 * 0.5,
                    top: vec![Score { class, confidence }],
                })
                .collect(),
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
}

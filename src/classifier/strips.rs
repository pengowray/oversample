//! Score strips: rows above the spectrogram that show, for the whole
//! recording, one picked species' score per window, or where each window's
//! BirdNET embedding falls on one principal component. The same readings the
//! live BirdNET view draws for the last 20 s (`crate::live_embedding`), here
//! from a finished (or running) classifier run.
//!
//! Pure functions; the canvas drawing is `crate::canvas::overlays::
//! draw_score_strips` and the component is `crate::components::score_strips`.
//!
//! A run keeps only each window's top [`super::TOP_K`] scores, so a picked
//! species outside a window's top 10 has no score there: its cell is `None`
//! and is drawn empty, apart from a stored low score.

use super::{ClassifierRun, Method};

/// A species picked for a score strip: the method whose results it comes from,
/// and its scientific name. Not the class number: BattyBirdNET's numbers
/// change with the region.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StripSpecies {
    pub method: Method,
    pub scientific: String,
}

/// What a strip row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScoreRowKind {
    /// A species' score, 0 to 1.
    Species,
    /// The window's percentile (0 to 1) on a principal component, among the
    /// reference recordings (`live_embedding::Components::percentiles`).
    Component,
}

/// One strip row: its label and one cell per window of the run.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreRow {
    pub kind: ScoreRowKind,
    /// "Song Thrush", "PC1".
    pub label: String,
    /// The scientific name, for the hover text; empty for a component.
    pub scientific: String,
    /// A bird the location list says is unlikely at the recording's place and
    /// month: the flag is drawn before the label.
    pub unlikely: bool,
    /// Per window: the value, or `None` when the run has none (outside the
    /// window's top 10, or the window's embedding not in yet).
    pub cells: Vec<Option<f32>>,
}

/// Everything the strips draw for the current file.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreStrips {
    /// Each window's start and end, in seconds.
    pub windows: Vec<(f64, f64)>,
    pub rows: Vec<ScoreRow>,
    /// Per window, the lowest stored score when the window holds a full top
    /// 10: a species missing from it scored below this (`window_floors`).
    pub floors: Vec<Option<f32>>,
    /// The minimum confidence: species cells at or above it are drawn
    /// stronger, as in the live view's strip.
    pub min_confidence: f32,
}

/// The class of `run` with this scientific name.
pub fn class_of(run: &ClassifierRun, scientific: &str) -> Option<u16> {
    run.labels
        .iter()
        .position(|l| l.scientific == scientific)
        .and_then(|i| u16::try_from(i).ok())
}

/// Class `class`'s score in each window, `None` where it is not among the
/// window's stored top scores.
pub fn species_cells(run: &ClassifierRun, class: u16) -> Vec<Option<f32>> {
    run.windows
        .iter()
        .map(|w| {
            w.top
                .iter()
                .find(|s| s.class == class)
                .map(|s| s.confidence)
        })
        .collect()
}

/// Per window, the lowest of its stored scores when it holds
/// [`super::TOP_K`] of them, so a class not stored scored below it. `None`
/// when the window holds fewer (every class above zero is then stored).
pub fn window_floors(run: &ClassifierRun) -> Vec<Option<f32>> {
    run.windows
        .iter()
        .map(|w| {
            (w.top.len() >= super::TOP_K).then(|| {
                w.top
                    .iter()
                    .map(|s| s.confidence)
                    .fold(f32::INFINITY, f32::min)
            })
        })
        .collect()
}

/// A class's name as shown: its common name, with a capital first letter
/// (BattyBirdNET's classes such as "noise" are lower case).
pub fn display_name(label: &super::SpeciesLabel) -> String {
    let mut c = label.common.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// For each component in `picked` (0 for PC1), each window's percentile.
/// `embeddings` holds `method`'s embeddings back to back in window order
/// (`ClassifierState::embeddings`); a window without one yet gets `None`.
/// `percentiles` maps one embedding to its percentile on every component.
pub fn component_cells(
    embeddings: &[f32],
    method: Method,
    windows: usize,
    picked: &[usize],
    percentiles: impl Fn(&[f32]) -> Vec<f32>,
) -> Vec<Vec<Option<f32>>> {
    let mut rows = vec![Vec::with_capacity(windows); picked.len()];
    for i in 0..windows {
        let p = super::types::embedding(embeddings, method, i).map(&percentiles);
        for (row, &k) in rows.iter_mut().zip(picked) {
            row.push(p.as_ref().and_then(|p| p.get(k).copied()));
        }
    }
    rows
}

/// The window that time `t` (seconds) falls in. Windows are in time order and
/// do not overlap.
pub fn window_at(windows: &[(f64, f64)], t: f64) -> Option<usize> {
    let i = windows.partition_point(|&(start, _)| start <= t);
    let i = i.checked_sub(1)?;
    (t < windows[i].1).then_some(i)
}

/// When windows are narrower than a pixel or two, one value per pixel column
/// instead of one per window: the highest value of the windows that touch the
/// column. Drawing each window as its own translucent 1 px cell would stack
/// neighbours' opacity, so faint scores would look like hits. `x_of` maps a
/// time in seconds to a canvas x; columns run from 0 to `width`.
pub fn column_max(
    windows: &[(f64, f64)],
    cells: &[Option<f32>],
    x_of: impl Fn(f64) -> f64,
    width: usize,
) -> Vec<Option<f32>> {
    let mut out: Vec<Option<f32>> = vec![None; width];
    for (&(s, e), cell) in windows.iter().zip(cells) {
        let Some(v) = *cell else { continue };
        let x0 = x_of(s).floor().max(0.0);
        let x1 = x_of(e).ceil().min(width as f64);
        if x1 <= x0 {
            continue;
        }
        for c in &mut out[x0 as usize..x1 as usize] {
            *c = Some(c.map_or(v, |old| old.max(v)));
        }
    }
    out
}

/// Opacity of a species cell: higher for higher scores, with a step at the
/// minimum confidence so a hit stands out from a near miss. The live view's
/// strip uses the same rule (`cellColor` in `live-embedding.js`).
pub fn species_alpha(score: f32, min_confidence: f32) -> f64 {
    let s = score as f64;
    let min = min_confidence as f64;
    if s >= min {
        0.6 + 0.4 * s
    } else {
        0.08 + 0.4 * (s / min.max(0.01))
    }
}

/// Which fifth (0 lowest to 4 highest) a percentile falls in. The component
/// rows colour each window by its fifth, as the live view's graphs do.
pub fn fifth(percentile: f32) -> usize {
    ((percentile * 5.0).floor().max(0.0) as usize).min(4)
}

/// `1st`, `2nd`, `73rd`, `11th`.
pub fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// Add `item` to `picked` when it is not there, or remove it when it is.
pub fn toggle<T: PartialEq>(picked: &mut Vec<T>, item: T) {
    match picked.iter().position(|x| *x == item) {
        Some(i) => {
            picked.remove(i);
        }
        None => picked.push(item),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::{catalog, Score, SpeciesLabel, WindowScores};
    use std::sync::Arc;

    fn run(windows: Vec<Vec<(u16, f32)>>) -> ClassifierRun {
        let labels = [
            "Turdus philomelos_Song Thrush",
            "Erithacus rubecula_European Robin",
        ]
        .iter()
        .map(|l| SpeciesLabel::parse(l))
        .collect();
        ClassifierRun {
            method: Method::BirdNet,
            region: None,
            backbone: &catalog::BACKBONE,
            head: None,
            source_rate: 48_000,
            model_rate: 48_000,
            window_secs: 3.0,
            labels: Arc::new(labels),
            windows: windows
                .into_iter()
                .enumerate()
                .map(|(i, top)| WindowScores {
                    start_s: i as f64 * 3.0,
                    end_s: (i + 1) as f64 * 3.0,
                    top: top
                        .into_iter()
                        .map(|(class, confidence)| Score { class, confidence })
                        .collect(),
                })
                .collect(),
            complete: true,
        }
    }

    #[test]
    fn class_by_scientific_name() {
        let r = run(vec![]);
        assert_eq!(class_of(&r, "Erithacus rubecula"), Some(1));
        assert_eq!(class_of(&r, "Apus apus"), None);
    }

    #[test]
    fn species_score_or_none() {
        let r = run(vec![vec![(0, 0.9), (1, 0.2)], vec![(1, 0.7)], vec![]]);
        assert_eq!(species_cells(&r, 0), vec![Some(0.9), None, None]);
        assert_eq!(species_cells(&r, 1), vec![Some(0.2), Some(0.7), None]);
    }

    #[test]
    fn component_rows_per_window() {
        let len = Method::BirdNet.embedding_len();
        // Two windows of embeddings for three windows: the last is missing.
        let mut e = vec![0.0; len];
        e[0] = 0.25;
        e.extend(vec![0.0; len]);
        e[len] = 0.75;
        // A stand-in for the percentiles: component k is the first value + k/10.
        let p = |x: &[f32]| (0..3).map(|k| x[0] + k as f32 / 10.0).collect::<Vec<f32>>();
        let rows = component_cells(&e, Method::BirdNet, 3, &[2, 0], p);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec![Some(0.45), Some(0.95), None]);
        assert_eq!(rows[1], vec![Some(0.25), Some(0.75), None]);
        // A component past the file's count has no value.
        let rows = component_cells(&e, Method::BirdNet, 1, &[7], p);
        assert_eq!(rows[0], vec![None]);
    }

    #[test]
    fn floors_need_a_full_top_k() {
        let full: Vec<(u16, f32)> = (0..crate::classifier::TOP_K as u16)
            .map(|c| (c % 2, 0.5 - c as f32 * 0.01))
            .collect();
        let r = run(vec![full, vec![(0, 0.3)]]);
        let f = window_floors(&r);
        assert!(
            (f[0].unwrap() - (0.5 - (crate::classifier::TOP_K - 1) as f32 * 0.01)).abs() < 1e-6
        );
        assert_eq!(f[1], None);
    }

    #[test]
    fn names_get_a_capital() {
        assert_eq!(display_name(&SpeciesLabel::parse("noise")), "Noise");
        assert_eq!(
            display_name(&SpeciesLabel::parse("Turdus philomelos_Song Thrush")),
            "Song Thrush"
        );
    }

    #[test]
    fn columns_keep_the_highest() {
        // Four 0.5 s windows at 1 px per second: two per column.
        let w = [(0.0, 0.5), (0.5, 1.0), (1.0, 1.5), (1.5, 2.0)];
        let cells = [Some(0.1), Some(0.8), None, Some(0.3)];
        let cols = column_max(&w, &cells, |t| t, 3);
        assert_eq!(cols, vec![Some(0.8), Some(0.3), None]);
        // Scrolled 1 s to the right: the first two windows are off screen.
        let cols = column_max(&w, &cells, |t| t - 1.0, 2);
        assert_eq!(cols, vec![Some(0.3), None]);
    }

    #[test]
    fn window_lookup() {
        let w = [(0.0, 3.0), (3.0, 6.0), (6.0, 7.5)];
        assert_eq!(window_at(&w, 0.0), Some(0));
        assert_eq!(window_at(&w, 2.99), Some(0));
        assert_eq!(window_at(&w, 3.0), Some(1));
        assert_eq!(window_at(&w, 7.4), Some(2));
        assert_eq!(window_at(&w, 7.5), None);
        assert_eq!(window_at(&w, -1.0), None);
        assert_eq!(window_at(&[], 1.0), None);
    }

    #[test]
    fn alpha_steps_at_minimum() {
        let below = species_alpha(0.49, 0.5);
        let at = species_alpha(0.5, 0.5);
        assert!(below < 0.5 && at >= 0.8);
        assert!(species_alpha(0.0, 0.5) > 0.0);
        assert!((species_alpha(1.0, 0.5) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn fifths_and_ordinals() {
        assert_eq!(fifth(0.0), 0);
        assert_eq!(fifth(0.19), 0);
        assert_eq!(fifth(0.2), 1);
        assert_eq!(fifth(0.99), 4);
        assert_eq!(fifth(1.0), 4);
        assert_eq!(ordinal(1), "1st");
        assert_eq!(ordinal(2), "2nd");
        assert_eq!(ordinal(3), "3rd");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(12), "12th");
        assert_eq!(ordinal(73), "73rd");
        assert_eq!(ordinal(100), "100th");
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut v = vec![1, 2];
        toggle(&mut v, 3);
        assert_eq!(v, [1, 2, 3]);
        toggle(&mut v, 1);
        assert_eq!(v, [2, 3]);
    }
}

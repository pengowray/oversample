//! Score strips: rows above the spectrogram that show, for the whole
//! recording, one picked species' score per window, or where each window's
//! BirdNET embedding falls on one principal component. The same readings the
//! live BirdNET view draws for the last 20 s (`crate::live_embedding`), here
//! from a finished (or running) classifier run.
//!
//! Pure functions; the canvas drawing is `crate::canvas::overlays::
//! draw_score_strips` and the component is `crate::components::score_strips`.
//! Also here: the saved form of the picks (`to_stored`, `from_stored`) and
//! the name matching behind the species search (`find_classes`).
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
    /// The window's sound group (`super::sound_groups`): the cell holds the
    /// group's index as a float, 0.0 for Group 1.
    Groups,
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
    /// A species row's colour slot (`canvas::overlays::species_colour`), the
    /// same as its swatch in the results lists; `None` for the shared grey
    /// and for rows that are not a species.
    pub colour: Option<usize>,
    /// Per window: the value, or `None` when the run has none (outside the
    /// window's top 10, or the window's embedding not in yet).
    pub cells: Vec<Option<f32>>,
}

/// Everything the strips draw for the current file.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreStrips {
    /// Each window's cell (`ClassifierRun::cells`), in seconds: where its
    /// values are drawn and hovered. Cells never overlap, also when the
    /// windows do.
    pub windows: Vec<(f64, f64)>,
    /// Each window's own start and end, in seconds: the audio the model
    /// heard, for the hover text.
    pub heard: Vec<(f64, f64)>,
    /// Windows are numbered in the hover text: false when they overlap,
    /// since a later pass of a run renumbers them.
    pub numbered: bool,
    pub rows: Vec<ScoreRow>,
    /// Per window, the lowest stored score when the window holds a full top
    /// 10: a species missing from it scored below this (`window_floors`).
    pub floors: Vec<Option<f32>>,
    /// The minimum confidence: species cells at or above it are drawn
    /// stronger, as in the live view's strip.
    pub min_confidence: f32,
    /// For the sound groups row, each group's name and hint as hovered,
    /// Group 1 first: "Group 2: mostly Gentoo Penguin". Empty without the row.
    pub group_names: Vec<String>,
    /// For the sound groups row, per window: quiet, so it has no group and
    /// its cell is empty. Empty without the row.
    pub quiet: Vec<bool>,
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
/// `embeddings` holds `method`'s embeddings back to back in the order the
/// windows arrived (`ClassifierState::embeddings`); `slots` is each window's
/// place in it (`WindowScores::slot`), in window order. A window without one
/// yet gets `None`. `percentiles` maps one embedding to its percentile on
/// every component.
pub fn component_cells(
    embeddings: &[f32],
    method: Method,
    slots: &[u32],
    picked: &[usize],
    percentiles: impl Fn(&[f32]) -> Vec<f32>,
) -> Vec<Vec<Option<f32>>> {
    let mut rows = vec![Vec::with_capacity(slots.len()); picked.len()];
    for &slot in slots {
        let p = super::types::embedding(embeddings, method, slot as usize).map(&percentiles);
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

/// The window starting at `start` (within `types::SAME_WINDOW_S`), given
/// each window's start and end in time order.
pub fn start_index(windows: &[(f64, f64)], start: f64) -> Option<usize> {
    let at = windows.partition_point(|&(s, _)| s < start);
    [at.checked_sub(1), Some(at)]
        .into_iter()
        .flatten()
        .find(|&i| {
            windows
                .get(i)
                .is_some_and(|&(s, _)| (s - start).abs() < super::types::SAME_WINDOW_S)
        })
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

/// When windows are narrower than a pixel or two, one value per pixel column:
/// the value of the window under the column's centre. For rows whose values
/// are categories (sound groups), which must not be mixed or ranked. `t_of`
/// maps a canvas x to a time in seconds; columns run from 0 to `width`.
pub fn column_centre(
    windows: &[(f64, f64)],
    cells: &[Option<f32>],
    t_of: impl Fn(f64) -> f64,
    width: usize,
) -> Vec<Option<f32>> {
    (0..width)
        .map(|x| {
            window_at(windows, t_of(x as f64 + 0.5)).and_then(|i| cells.get(i).copied().flatten())
        })
        .collect()
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

// ── Saved picks ─────────────────────────────────────────────────────────────

/// Most species rows and components kept from a saved value, so a damaged
/// value cannot fill the screen with rows.
const MAX_SAVED_SPECIES: usize = 50;
const MAX_SAVED_COMPONENT: usize = 255;

/// The name a method is saved under. Fixed strings, so renaming the enum's
/// variants does not lose saved picks.
fn method_key(method: Method) -> &'static str {
    match method {
        Method::BirdNet => "birdnet",
        Method::Bats => "bats",
        Method::Perch => "perch",
        Method::BirdNetV3 => "birdnet-v3",
    }
}

fn method_from_key(key: &str) -> Option<Method> {
    match key {
        "birdnet" => Some(Method::BirdNet),
        "bats" => Some(Method::Bats),
        "perch" => Some(Method::Perch),
        "birdnet-v3" => Some(Method::BirdNetV3),
        _ => None,
    }
}

/// The rows picked to show above the spectrogram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StripPicks {
    pub species: Vec<StripSpecies>,
    /// BirdNET principal components, 0 for PC1.
    pub components: Vec<usize>,
    /// The "Sound groups" row is on.
    pub sound_groups: bool,
    /// Number of sound groups, one of `sound_groups::GROUP_CHOICES`.
    pub groups: usize,
}

impl Default for StripPicks {
    fn default() -> Self {
        Self {
            species: Vec::new(),
            components: Vec::new(),
            sound_groups: false,
            groups: super::sound_groups::DEFAULT_GROUPS,
        }
    }
}

/// The picks as saved in `settings::keys::SCORE_STRIPS`:
/// `{"species":[{"method":"birdnet","scientific":"Turdus merula"}],"components":[0,2],"sound_groups":true,"groups":3}`.
pub fn to_stored(picks: &StripPicks) -> String {
    let species: Vec<serde_json::Value> = picks
        .species
        .iter()
        .map(|s| serde_json::json!({ "method": method_key(s.method), "scientific": s.scientific }))
        .collect();
    serde_json::json!({
        "species": species,
        "components": picks.components,
        "sound_groups": picks.sound_groups,
        "groups": picks.groups,
    })
    .to_string()
}

/// The picks from a saved value. Anything unreadable is skipped: a value that
/// is not JSON gives no picks, and an entry with an unknown method, a missing
/// name or a component that is not a small whole number is dropped while the
/// rest are kept. Repeats are dropped too. Values saved before the sound
/// groups row existed have no `sound_groups` or `groups`: the row is off,
/// with the default number of groups, as is a number of groups not offered.
pub fn from_stored(raw: &str) -> StripPicks {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return StripPicks::default();
    };
    let mut species: Vec<StripSpecies> = Vec::new();
    for entry in value["species"].as_array().into_iter().flatten() {
        let method = entry["method"].as_str().and_then(method_from_key);
        let scientific = entry["scientific"].as_str().map(str::trim).unwrap_or("");
        let (Some(method), false) = (method, scientific.is_empty()) else {
            continue;
        };
        let s = StripSpecies {
            method,
            scientific: scientific.to_string(),
        };
        if !species.contains(&s) && species.len() < MAX_SAVED_SPECIES {
            species.push(s);
        }
    }
    let mut components: Vec<usize> = Vec::new();
    for k in value["components"].as_array().into_iter().flatten() {
        let Some(k) = k.as_u64().and_then(|k| usize::try_from(k).ok()) else {
            continue;
        };
        if k <= MAX_SAVED_COMPONENT && !components.contains(&k) {
            components.push(k);
        }
    }
    let groups = value["groups"]
        .as_u64()
        .and_then(|g| usize::try_from(g).ok())
        .filter(|g| super::sound_groups::GROUP_CHOICES.contains(g))
        .unwrap_or(super::sound_groups::DEFAULT_GROUPS);
    StripPicks {
        species,
        components,
        sound_groups: value["sound_groups"].as_bool().unwrap_or(false),
        groups,
    }
}

// ── Species search ──────────────────────────────────────────────────────────

/// A name made ready for matching: lower case, accents removed, hyphens as
/// spaces and apostrophes dropped, so "bewicks wren" finds "Bewick's Wren"
/// and "black capped" finds "Black-capped Chickadee".
pub fn fold_name(name: &str) -> String {
    xc_lib::group_words::fold(name)
        .chars()
        // Combining accents, for text typed in decomposed form.
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .filter(|c| !matches!(c, '\'' | '\u{2019}'))
        .map(|c| if c == '-' { ' ' } else { c })
        .collect()
}

/// Each label's common and scientific name, folded with [`fold_name`]. Built
/// once per label list and passed to [`find_classes`].
pub fn fold_labels(labels: &[super::SpeciesLabel]) -> Vec<(String, String)> {
    labels
        .iter()
        .map(|l| (fold_name(&l.common), fold_name(&l.scientific)))
        .collect()
}

/// How well a folded name matches the folded query: 0 when the name starts
/// with it, 1 when each word of the query starts a word of the name, 2 when
/// the name contains it, `None` otherwise.
fn match_rank(name: &str, query: &str, words: &[&str]) -> Option<u8> {
    if name.starts_with(query) {
        return Some(0);
    }
    let name_words: Vec<&str> = name.split_whitespace().collect();
    if words
        .iter()
        .all(|q| name_words.iter().any(|w| w.starts_with(q)))
    {
        return Some(1);
    }
    name.contains(query).then_some(2)
}

/// The classes whose common or scientific name matches `query`, best matches
/// first and then in label order, at most `limit` of them. Labels that are not
/// a species (BattyBirdNET's "noise") are left out. `folded` is
/// [`fold_labels`] of `labels`.
pub fn find_classes(
    labels: &[super::SpeciesLabel],
    folded: &[(String, String)],
    query: &str,
    limit: usize,
) -> Vec<u16> {
    let query = fold_name(query);
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() {
        return Vec::new();
    }
    let words: Vec<&str> = query.split(' ').collect();
    let mut hits: Vec<(u8, usize)> = labels
        .iter()
        .zip(folded)
        .enumerate()
        .filter(|(_, (l, _))| l.is_species)
        .filter_map(|(i, (_, (common, scientific)))| {
            let rank = [common, scientific]
                .iter()
                .filter_map(|n| match_rank(n, &query, &words))
                .min()?;
            Some((rank, i))
        })
        .collect();
    hits.sort_unstable();
    hits.into_iter()
        .take(limit)
        .filter_map(|(_, i)| u16::try_from(i).ok())
        .collect()
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
            step_secs: 3.0,
            spacing_secs: 3.0,
            labels: Arc::new(labels),
            windows: windows
                .into_iter()
                .enumerate()
                .map(|(i, top)| WindowScores {
                    start_s: i as f64 * 3.0,
                    end_s: (i + 1) as f64 * 3.0,
                    slot: i as u32,
                    live: false,
                    top: top
                        .into_iter()
                        .map(|(class, confidence)| Score { class, confidence })
                        .collect(),
                })
                .collect(),
            whole_file: true,
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
        let rows = component_cells(&e, Method::BirdNet, &[0, 1, 2], &[2, 0], p);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec![Some(0.45), Some(0.95), None]);
        assert_eq!(rows[1], vec![Some(0.25), Some(0.75), None]);
        // A component past the file's count has no value.
        let rows = component_cells(&e, Method::BirdNet, &[0], &[7], p);
        assert_eq!(rows[0], vec![None]);
        // Windows whose embeddings arrived in another order.
        let rows = component_cells(&e, Method::BirdNet, &[1, 0], &[0], p);
        assert_eq!(rows[0], vec![Some(0.75), Some(0.25)]);
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
    fn saved_picks_round_trip() {
        let species = vec![
            StripSpecies {
                method: Method::BirdNet,
                scientific: "Turdus philomelos".into(),
            },
            StripSpecies {
                method: Method::Bats,
                scientific: "Pipistrellus pipistrellus".into(),
            },
            StripSpecies {
                method: Method::Perch,
                scientific: "Erithacus rubecula".into(),
            },
            StripSpecies {
                method: Method::BirdNetV3,
                scientific: "Pygoscelis papua".into(),
            },
        ];
        let picks = StripPicks {
            species,
            components: vec![2, 0],
            sound_groups: true,
            groups: 3,
        };
        assert_eq!(from_stored(&to_stored(&picks)), picks);
        let picks = StripPicks::default();
        assert_eq!(from_stored(&to_stored(&picks)), picks);
        for groups in crate::classifier::sound_groups::GROUP_CHOICES {
            let picks = StripPicks {
                sound_groups: groups % 2 == 0,
                groups,
                ..StripPicks::default()
            };
            assert_eq!(from_stored(&to_stored(&picks)), picks);
        }
    }

    #[test]
    fn picks_saved_before_sound_groups_load() {
        let old =
            r#"{"species":[{"method":"birdnet","scientific":"Turdus merula"}],"components":[0,2]}"#;
        let picks = from_stored(old);
        assert_eq!(picks.species.len(), 1);
        assert_eq!(picks.components, vec![0, 2]);
        assert!(!picks.sound_groups);
        assert_eq!(
            picks.groups,
            crate::classifier::sound_groups::DEFAULT_GROUPS
        );
        // A number of groups not offered, or not a number: the default.
        for raw in [
            r#"{"sound_groups":true,"groups":7}"#,
            r#"{"sound_groups":true,"groups":1}"#,
            r#"{"sound_groups":true,"groups":"3"}"#,
            r#"{"sound_groups":true,"groups":2.5}"#,
        ] {
            let p = from_stored(raw);
            assert!(p.sound_groups, "{raw}");
            assert_eq!(
                p.groups,
                crate::classifier::sound_groups::DEFAULT_GROUPS,
                "{raw}"
            );
        }
        assert!(!from_stored(r#"{"sound_groups":"yes"}"#).sound_groups);
    }

    #[test]
    fn bad_saved_picks_are_skipped() {
        for raw in ["", "not json", "null", "[]", "42", r#"{"species":"x"}"#] {
            assert_eq!(from_stored(raw), StripPicks::default(), "{raw}");
        }
        let raw = r#"{
            "species": [
                {"method": "birdnet", "scientific": "Turdus merula"},
                {"method": "BirdNet", "scientific": "Apus apus"},
                {"method": "perch"},
                {"method": "perch", "scientific": "  "},
                {"method": "bats", "scientific": 3},
                "Turdus merula",
                {"method": "birdnet", "scientific": "Turdus merula"},
                {"method": "perch", "scientific": "Turdus merula", "extra": true}
            ],
            "components": [1, -1, 1.5, "2", 1, 9999, 3]
        }"#;
        let StripPicks {
            species,
            components,
            ..
        } = from_stored(raw);
        assert_eq!(
            species,
            vec![
                StripSpecies {
                    method: Method::BirdNet,
                    scientific: "Turdus merula".into()
                },
                StripSpecies {
                    method: Method::Perch,
                    scientific: "Turdus merula".into()
                },
            ]
        );
        assert_eq!(components, vec![1, 3]);
        // Only one of the two parts present.
        assert_eq!(
            from_stored(r#"{"components":[4]}"#),
            StripPicks {
                components: vec![4],
                ..StripPicks::default()
            }
        );
    }

    #[test]
    fn group_columns_take_the_centre_window() {
        // Four 0.5 s windows at 1 px per second: column 0 is centred at 0.5 s
        // (window 1), column 1 at 1.5 s (window 3).
        let w = [(0.0, 0.5), (0.5, 1.0), (1.0, 1.5), (1.5, 2.0)];
        let cells = [Some(0.0), Some(1.0), Some(2.0), None];
        assert_eq!(
            column_centre(&w, &cells, |x| x, 3),
            vec![Some(1.0), None, None]
        );
        assert_eq!(
            column_centre(&w, &cells, |x| x * 0.25, 8),
            vec![
                Some(0.0),
                Some(0.0),
                Some(1.0),
                Some(1.0),
                Some(2.0),
                Some(2.0),
                None,
                None
            ]
        );
    }

    fn labels(lines: &[&str]) -> Vec<SpeciesLabel> {
        lines.iter().map(|l| SpeciesLabel::parse(l)).collect()
    }

    fn find(labels: &[SpeciesLabel], query: &str) -> Vec<u16> {
        find_classes(labels, &fold_labels(labels), query, 8)
    }

    #[test]
    fn names_fold_for_matching() {
        assert_eq!(fold_name("Bewick's Wren"), "bewicks wren");
        assert_eq!(fold_name("Bewick\u{2019}s Wren"), "bewicks wren");
        assert_eq!(
            fold_name("Black-capped Chickadee"),
            "black capped chickadee"
        );
        assert_eq!(fold_name("Pouillot fitis \u{e9}"), "pouillot fitis e");
        // "e" and a combining acute accent.
        assert_eq!(fold_name("Me\u{301}sange"), "mesange");
    }

    #[test]
    fn search_both_names_ignoring_case_and_accents() {
        let l = labels(&[
            "Turdus merula_Eurasian Blackbird",
            "Turdus philomelos_Song Thrush",
            "Erithacus rubecula_European Robin",
            "Poecile atricapillus_Black-capped Chickadee",
            "Thryomanes bewickii_Bewick's Wren",
            "Cyanistes caeruleus_Mésange bleue",
        ]);
        assert_eq!(find(&l, "song"), vec![1]);
        assert_eq!(find(&l, "SONG THRUSH"), vec![1]);
        assert_eq!(find(&l, "turdus"), vec![0, 1]);
        assert_eq!(find(&l, "erithacus rub"), vec![2]);
        assert_eq!(find(&l, "black capped"), vec![3]);
        assert_eq!(find(&l, "bewicks"), vec![4]);
        assert_eq!(find(&l, "mesange"), vec![5]);
        assert_eq!(find(&l, "MÉSANGE"), vec![5]);
        // Word starts in any order, across the common name.
        assert_eq!(find(&l, "rob eur"), vec![2]);
        assert!(find(&l, "").is_empty());
        assert!(find(&l, "   ").is_empty());
        assert!(find(&l, "zebra").is_empty());
    }

    #[test]
    fn search_ranks_and_caps() {
        let l = labels(&[
            "Turdus migratorius_American Robin",
            "Erithacus rubecula_European Robin",
            "Robinia x_Robin Hood",
            "Aaa bbb_Mr Probin",
            "noise",
        ]);
        // Name starts with the query, then a word starts with it, then the
        // name contains it.
        assert_eq!(find(&l, "robin"), vec![2, 0, 1, 3]);
        // Non-species labels are never offered.
        assert!(find(&l, "noise").is_empty());
        assert_eq!(find_classes(&l, &fold_labels(&l), "robin", 2), vec![2, 0]);
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

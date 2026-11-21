//! Bird boxes: BirdBox's Just-Bird finds each bird sound as a box (start, end,
//! lowest and highest frequency), and BirdNET+ V3.0's windows name it.
//!
//! A BirdNET+ V3.0 Analyse with "Find bird boxes" on also runs Just-Bird in the
//! worker, on the same 32 kHz audio (`birdnet-worker/src/birdbox/`). Its boxes
//! of each 3 s window, kept from a score of 0.05 (`boxes::KEEP_CONF`), are
//! stored per file in a [`BoxRun`]; the boxes drawn are merged from them
//! ([`BoxRun::merged`]) with the minimum confidence and box size picked now, so
//! a change shows at once without running the model again.
//!
//! The species of a box is the rule of the BirdNET website demo (BirdBox's
//! `docs/models-and-metrics/just-bird-species-from-birdnet.md`, placement
//! `grid15`, ported from `scripts/birdnet_labels/label_windows.py`): BirdNET+
//! V3.0's windows on BirdBox's own grid, one every 1.5 s. A box up to 1.5 s
//! long takes the window that owns its centre (a window owns its middle 1.5 s);
//! a longer box takes the windows from the one round its first 1.5 s to the one
//! round its last ([`grid_windows`]). Each bird keeps its highest score over
//! those windows, and only birds count. The box is named after the highest
//! from a score of [`NAME_FROM`]; the list shows up to [`LIST_MAX`] birds from
//! [`LIST_FROM`] ([`box_species`]). The run makes sure the 1.5 s grid is
//! scored (`super::run_task`).

use super::{ClassifierRun, Score, SpeciesLabel};
use oversample_ipc::classifier::boxes::{
    fuse, limit_to_content, padded_end_s, MergedBox, RawBox, CLIP_HOP_S, CLIP_S,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// A box is named after its highest-scoring bird from this score on;
/// below it the box is drawn as "Bird", as the BirdNET demo does.
pub const NAME_FROM: f32 = 0.35;
/// The birds listed for a box score at least this.
pub const LIST_FROM: f32 = 0.05;
/// At most this many birds are listed for a box.
pub const LIST_MAX: usize = 5;
/// The label of a box no bird reached [`NAME_FROM`] for: birders' "sp."
/// for a bird whose species is not known.
pub const UNNAMED: &str = "Bird sp.";
/// The card of a box whose windows score no bird from [`LIST_FROM`].
pub const CARD_NONE: &str = "No bird species scored 0.05 or more";

/// The first line of a box's card: Just-Bird's confidence for it.
pub fn card_first(confidence: f32) -> String {
    format!("Box confidence {confidence:.2}")
}

/// The rate Just-Bird hears.
const RATE: f64 = 32_000.0;
/// Samples in one window at [`RATE`].
const CLIP_SAMPLES: u64 = 96_000;
const HOP_SAMPLES: u64 = 48_000;

/// One file's bird boxes, as the worker found them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoxRun {
    /// The recording's sample rate when they were found: a rate change
    /// (time expansion) moves every box, so they are not shown after one.
    pub source_rate: u32,
    /// Each window's boxes after NMS, from a score of 0.05.
    pub raw: Vec<RawBox>,
    /// Windows done (one every 1.5 s).
    pub windows: u32,
    /// Samples of 32 kHz audio in the recording, once the last window is done.
    pub samples: Option<u64>,
}

impl BoxRun {
    pub fn new(source_rate: u32) -> BoxRun {
        BoxRun {
            source_rate,
            ..Default::default()
        }
    }

    /// True once every window is done.
    pub fn complete(&self) -> bool {
        self.samples.is_some()
    }

    /// The boxes merged across windows: window boxes scoring above
    /// `min_conf`, then BirdBox's window fusion, joining boxes in one band less
    /// than `phrase_gap` seconds apart (0 keeps the boxes of single calls).
    /// While windows are still coming, the newest window does not own the end
    /// of its audio yet, so a box there waits for the next window.
    pub fn merged(&self, min_conf: f32, phrase_gap: f64) -> Vec<MergedBox> {
        let keep = |b: &&RawBox| b.conf > min_conf as f64;
        match self.samples {
            Some(n) => {
                let end = padded_end_s(n);
                let raw = limit_to_content(
                    self.raw.iter().filter(keep).copied().collect(),
                    n as f64 / RATE,
                    end,
                );
                fuse(&raw, end, phrase_gap)
            }
            None => {
                let raw: Vec<RawBox> = self.raw.iter().filter(keep).copied().collect();
                fuse(&raw, f64::INFINITY, phrase_gap)
            }
        }
    }
}

/// The windows on BirdBox's 1.5 s grid whose BirdNET+ V3.0 scores name a box
/// from `t0` to `t1` (seconds), in a recording of `samples` samples at 32 kHz:
/// window `k` starts at `1.5 k` s. A box up to 1.5 s long takes the window
/// that owns its centre; a longer box the windows from the one that owns the
/// point 0.75 s after its start to the one that owns the point 0.75 s before
/// its end. A window owns its middle 1.5 s; the first also owns the start, the
/// last (the first that reaches the end) the end.
pub fn grid_windows(t0: f64, t1: f64, samples: u64) -> std::ops::RangeInclusive<usize> {
    let last = samples.saturating_sub(CLIP_SAMPLES).div_ceil(HOP_SAMPLES) as usize;
    let margin = (CLIP_S - CLIP_HOP_S) / 2.0;
    let owner = |time: f64| {
        let k = ((time - margin) / CLIP_HOP_S).floor();
        (k.max(0.0) as usize).min(last)
    };
    if t1 - t0 <= CLIP_S - CLIP_HOP_S {
        let k = owner((t0 + t1) / 2.0);
        k..=k
    } else {
        owner(t0 + margin)..=owner(t1 - margin)
    }
}

/// The birds of a box: for each class `is_bird` accepts, its highest score
/// over the scores of `windows` (each window's top classes; a window with no
/// scores adds nothing), from [`LIST_FROM`] on, highest first, at most
/// [`LIST_MAX`].
pub fn box_species<'a>(
    windows: impl IntoIterator<Item = &'a [Score]>,
    is_bird: impl Fn(u16) -> bool,
) -> Vec<Score> {
    let mut best: Vec<Score> = Vec::new();
    for top in windows {
        for s in top.iter().filter(|s| is_bird(s.class)) {
            match best.iter_mut().find(|b| b.class == s.class) {
                Some(b) => b.confidence = b.confidence.max(s.confidence),
                None => best.push(*s),
            }
        }
    }
    best.retain(|s| s.confidence >= LIST_FROM);
    best.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    best.truncate(LIST_MAX);
    best
}

/// The bird a box is named after: its highest, from [`NAME_FROM`] on.
pub fn named(species: &[Score]) -> Option<Score> {
    species
        .first()
        .copied()
        .filter(|s| s.confidence >= NAME_FROM)
}

/// The scores of `run`'s windows on the grid that name a box from `t0` to
/// `t1` ([`grid_windows`]). A window the run does not have (one not done yet,
/// or the last one, which BirdNET+ V3.0 drops when it holds less than a
/// second) is left out.
pub fn grid_scores<'a>(
    run: &'a ClassifierRun,
    t0: f64,
    t1: f64,
    samples: u64,
) -> impl Iterator<Item = &'a [Score]> + 'a {
    grid_windows(t0, t1, samples).filter_map(move |k| {
        run.window_at_start(k as f64 * CLIP_HOP_S)
            .map(|i| run.windows[i].top.as_slice())
    })
}

thread_local! {
    /// BirdNET+ V3.0's labels as read, and which of their classes are birds
    /// (taxon group Aves). Only birds name a box.
    static V3_BIRDS: RefCell<Option<(Arc<Vec<SpeciesLabel>>, Rc<Vec<bool>>)>> =
        const { RefCell::new(None) };
    /// The boxes drawn last, for finding the one under the pointer without
    /// merging them again.
    static DRAWN: RefCell<Arc<Vec<DrawnBox>>> = RefCell::new(Arc::new(Vec::new()));
}

/// Remember which of BirdNET+ V3.0's classes `labels` are birds.
pub fn set_v3_birds(labels: Arc<Vec<SpeciesLabel>>, birds: Vec<bool>) {
    V3_BIRDS.with(|b| *b.borrow_mut() = Some((labels, Rc::new(birds))));
}

/// A test for "is this class of `labels` a bird": BirdNET+ V3.0's taxon groups
/// for its own labels; every class for other lists (the test hook's made-up
/// results).
pub fn bird_test(labels: &Arc<Vec<SpeciesLabel>>) -> impl Fn(u16) -> bool {
    let mask = V3_BIRDS.with(|b| {
        b.borrow()
            .as_ref()
            .filter(|(l, _)| Arc::ptr_eq(l, labels))
            .map(|(_, m)| m.clone())
    });
    move |c| {
        mask.as_ref()
            .is_none_or(|m| m.get(c as usize).copied().unwrap_or(false))
    }
}

/// One box as the spectrogram draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawnBox {
    pub t0: f64,
    pub t1: f64,
    pub f0: f64,
    pub f1: f64,
    /// Just-Bird's highest confidence for the box.
    pub confidence: f32,
    /// The bird it is named after, from [`NAME_FROM`].
    pub name: Option<String>,
    /// Its birds, highest first ([`box_species`]): English name and score.
    pub species: Vec<(String, f32)>,
}

impl DrawnBox {
    /// What picks it out: start time and lowest frequency.
    pub fn key(&self) -> (f64, f64) {
        (self.t0, self.f0)
    }
}

/// Name each merged box from `run`'s windows (none without a run).
pub fn name_boxes(
    merged: &[MergedBox],
    run: Option<&ClassifierRun>,
    samples: u64,
) -> Vec<DrawnBox> {
    let is_bird = run.map(|r| bird_test(&r.labels));
    merged
        .iter()
        .map(|b| {
            let species = match (run, &is_bird) {
                (Some(r), Some(test)) => box_species(grid_scores(r, b.t0, b.t1, samples), test),
                _ => Vec::new(),
            };
            let name_of = |s: &Score| {
                run.and_then(|r| r.label(s))
                    .map_or_else(|| "Unknown".to_string(), |l| l.common.clone())
            };
            DrawnBox {
                t0: b.t0,
                t1: b.t1,
                f0: b.f0,
                f1: b.f1,
                confidence: b.max_conf as f32,
                name: named(&species).map(|s| name_of(&s)),
                species: species.iter().map(|s| (name_of(s), s.confidence)).collect(),
            }
        })
        .collect()
}

/// The phrase gap for the box size picked: 0 for single calls, BirdBox's
/// 0.05 s for phrases.
pub fn phrase_gap(calls: bool) -> f64 {
    if calls {
        0.0
    } else {
        oversample_ipc::classifier::boxes::DEFAULT_PHRASE_GAP
    }
}

/// True when bird boxes are made and shown for this recording: not for a
/// heterodyne recording, whose sounds are shifted down from the bats' calls.
pub fn applies_to(file: &crate::state::LoadedFile) -> bool {
    !file
        .heterodyne_hint
        .as_ref()
        .is_some_and(crate::heterodyne::shows_badge)
}

/// The boxes to draw on the current file, named: when BirdNET+ V3.0 is the
/// method picked, "Find bird boxes" is on and the file has boxes found at its
/// present sample rate. Reactive. Also kept for [`box_at`].
pub fn drawn(state: crate::state::AppState) -> Arc<Vec<DrawnBox>> {
    let out = Arc::new(drawn_now(state));
    DRAWN.with(|d| *d.borrow_mut() = out.clone());
    out
}

fn drawn_now(state: crate::state::AppState) -> Vec<DrawnBox> {
    use crate::state::store_fields::*;
    use crate::state::ClassifierStateStoreFields;
    use leptos::prelude::*;
    let cd = state.call_detection;
    if cd.method().get() != crate::state::DetectionMethod::BirdNetV3 || !cd.bird_boxes().get() {
        return Vec::new();
    }
    let Some(file) = state.current_file_id_tracked() else {
        return Vec::new();
    };
    let info = state.library.files().with(|files| {
        files
            .iter()
            .find(|f| f.id == file)
            .map(|f| (f.audio.sample_rate, f.audio.duration_secs, applies_to(f)))
    });
    let Some((rate, duration, true)) = info else {
        return Vec::new();
    };
    let min = cd.box_min_confidence().get();
    let gap = phrase_gap(cd.box_calls().get());
    let merged = state.classifier.boxes().with(|m| {
        m.get(&file)
            .filter(|b| b.source_rate == rate)
            .map(|b| (b.merged(min, gap), b.samples))
    });
    let Some((merged, samples)) = merged else {
        return Vec::new();
    };
    let samples = samples.unwrap_or((duration * RATE).round() as u64);
    state.classifier.results().with(|m| {
        let run = m
            .get(&(file, super::Method::BirdNetV3))
            .filter(|r| r.source_rate == rate);
        name_boxes(&merged, run, samples)
    })
}

/// The boxes drawn last ([`drawn`]), without tracking.
pub fn last_drawn() -> Arc<Vec<DrawnBox>> {
    DRAWN.with(|d| d.borrow().clone())
}

/// The drawn box at `t` seconds and `freq` Hz, the smallest when boxes
/// overlap.
pub fn box_at(t: f64, freq: f64) -> Option<(f64, f64)> {
    let boxes = last_drawn();
    boxes
        .iter()
        .filter(|b| b.t0 <= t && t <= b.t1 && b.f0 <= freq && freq <= b.f1)
        .min_by(|a, b| ((a.t1 - a.t0) * (a.f1 - a.f0)).total_cmp(&((b.t1 - b.t0) * (b.f1 - b.f0))))
        .map(DrawnBox::key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(class: u16, confidence: f32) -> Score {
        Score { class, confidence }
    }

    #[test]
    fn a_short_box_takes_the_window_that_owns_its_centre() {
        let n = 60 * 32_000;
        // Window k owns 1.5 k + 0.75 to 1.5 k + 2.25 s.
        assert_eq!(grid_windows(1.0, 1.4, n), 0..=0);
        assert_eq!(grid_windows(2.1, 2.5, n), 1..=1);
        assert_eq!(grid_windows(3.7, 3.8, n), 2..=2);
        assert_eq!(grid_windows(3.0, 4.5, n), 2..=2);
        // The first window owns the start of the file.
        assert_eq!(grid_windows(0.0, 0.2, n), 0..=0);
    }

    #[test]
    fn a_long_box_takes_the_windows_round_its_first_and_last_1_5_s() {
        let n = 60 * 32_000;
        // 2 s from 3 s: 3.75 s is in window 2, 4.25 s also in window 2.
        assert_eq!(grid_windows(3.0, 5.0, n), 2..=2);
        // 16 s from 10 s: 10.75 s is in window 6, 25.25 s in window 16.
        assert_eq!(grid_windows(10.0, 26.0, n), 6..=16);
        // The last window owns the end: 60 s has its last window at 57 s (38).
        assert_eq!(grid_windows(50.0, 60.0, n), 33..=38);
        assert_eq!(grid_windows(59.5, 59.9, n), 38..=38);
        // A recording shorter than a window has one.
        assert_eq!(grid_windows(0.5, 2.5, 50_000), 0..=0);
    }

    #[test]
    fn each_bird_keeps_its_highest_score_and_only_birds_count() {
        let w1 = [sc(1, 0.30), sc(2, 0.20), sc(9, 0.90)];
        let w2 = [sc(2, 0.60), sc(3, 0.04), sc(4, 0.10)];
        let empty: [Score; 0] = [];
        let birds = |c: u16| c != 9; // class 9 is a frog
        let s = box_species([&w1[..], &w2[..], &empty[..]], birds);
        assert_eq!(s, vec![sc(2, 0.60), sc(1, 0.30), sc(4, 0.10)]);
        assert_eq!(named(&s), Some(sc(2, 0.60)));
        // Below 0.35 the box has no name.
        let s = box_species([&w1[..]], birds);
        assert_eq!(named(&s), None);
        assert_eq!(s[0], sc(1, 0.30));
        // At most five.
        let many: Vec<Score> = (0..8).map(|c| sc(c, 0.5 - c as f32 * 0.01)).collect();
        assert_eq!(box_species([&many[..]], |_| true).len(), 5);
        assert!(box_species(std::iter::empty::<&[Score]>(), |_| true).is_empty());
    }

    #[test]
    fn boxes_are_named_from_the_runs_grid_windows() {
        use oversample_ipc::classifier::Method;
        let labels = Arc::new(vec![
            SpeciesLabel::parse("Turdus merula_Common Blackbird"),
            SpeciesLabel::parse("Erithacus rubecula_European Robin"),
        ]);
        let mut run = ClassifierRun::empty(
            Method::BirdNetV3,
            None,
            &super::super::catalog::BIRDNET_V3,
            None,
            32_000,
            labels,
            3.0,
        );
        let mut e = Vec::new();
        // Windows at 0, 1.5, 3 and 4.5 s; 1.5 s has the robin.
        for (start, top) in [
            (0.0, vec![sc(0, 0.8)]),
            (1.5, vec![sc(1, 0.5), sc(0, 0.2)]),
            (3.0, vec![sc(0, 0.1)]),
            (4.5, vec![]),
        ] {
            run.insert_reading(&mut e, start, start + 3.0, top, &[]);
        }
        let mb = |t0: f64, t1: f64| MergedBox {
            t0,
            t1,
            f0: 2000.0,
            f1: 4000.0,
            avg_conf: 0.6,
            max_conf: 0.7,
            merged: 1.0,
        };
        let n = 9 * 32_000;
        let d = name_boxes(
            &[mb(2.4, 2.8), mb(0.2, 0.6), mb(3.9, 4.2), mb(6.5, 6.9)],
            Some(&run),
            n,
        );
        assert_eq!(d[0].name.as_deref(), Some("European Robin"));
        assert_eq!(
            d[0].species,
            vec![
                ("European Robin".to_string(), 0.5),
                ("Common Blackbird".to_string(), 0.2)
            ]
        );
        assert_eq!(d[1].name.as_deref(), Some("Common Blackbird"));
        // Window 2 (3 s) scores the blackbird 0.10: listed, not named.
        assert_eq!(d[2].name, None);
        assert_eq!(d[2].species.len(), 1);
        // Its window, 3 (4.5 s), has no scores.
        assert!(d[3].species.is_empty());
        assert_eq!(d[3].confidence, 0.7);
        // Without a run, boxes have no names.
        assert!(name_boxes(&[mb(2.4, 2.8)], None, n)[0].name.is_none());
    }

    #[test]
    fn merging_follows_the_minimum_and_the_box_size() {
        let b = |t0: f64, t1: f64, conf: f64| RawBox {
            clip_start: 0.0,
            t0,
            t1,
            f0: 1000.0,
            f1: 2000.0,
            conf,
        };
        let run = BoxRun {
            source_rate: 48_000,
            raw: vec![b(0.2, 0.5, 0.4), b(0.52, 0.8, 0.4), b(1.5, 1.7, 0.1)],
            windows: 1,
            samples: Some(80_000),
        };
        // The two calls 0.02 s apart are one phrase; single calls keep two.
        assert_eq!(run.merged(0.05, phrase_gap(false)).len(), 2);
        assert_eq!(run.merged(0.05, phrase_gap(true)).len(), 3);
        assert_eq!(run.merged(0.3, phrase_gap(true)).len(), 2);
        assert!(run.complete());
    }
}

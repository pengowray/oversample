//! Readings the live view (`crate::live_embedding`) makes of a file, kept in
//! that file's results for BirdNET V2.4 or Perch 2.0, so the rows above the
//! spectrogram, the overlay, the overview marks and Sound groups fill in
//! wherever the file has been played, without Analyse.
//!
//! Which readings. The live view reads the current file's own samples (mono
//! mix at the file's sample rate, as Analyse reads them) for the window that
//! ends at the playhead, which is in file seconds in every playback mode.
//! Time expansion, pitch shift, heterodyne, filters and the other playback
//! settings change what is heard, never the samples read, so every playback
//! mode gives readings of the raw audio. Left out: the microphone, timelines
//! (their playhead is not a file's time), readings before a whole window has
//! played (zero-padded at the front, which Analyse never does), files shorter
//! than a window, and sample rates whose windows a cut cannot line up with
//! Analyse's (`oversample_ipc::classifier::live_cut`, `exact`).
//!
//! Where. While a file plays, the window starts on a grid of [`GRID_S`] in
//! file time ([`snap_start`]), so a part played again finds the readings made
//! the first time instead of running the model again, and playback adds at
//! most one reading per [`GRID_S`] of the file. The audio is cut with
//! `live_cut`, which gives the window the samples Analyse gives a window with
//! the same start, and the worker works out the top classes as for Analyse,
//! so a kept reading is the one Analyse makes.
//!
//! When. Readings wait here and go into `results` at most every
//! [`FLUSH_MS`], and at once when playback stops, the file changes or the live
//! view restarts ([`flush`]). [`stored`] looks at the waiting readings too.

use super::{catalog, ClassifierRun, Method, Score, TOP_K};
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// Grid of window starts while a file plays, in seconds: the window read is
/// the one starting at the last grid point a whole window before the
/// playhead. The live view's model reads about 4 times a second (BirdNET),
/// so this keeps about one reading per grid point at normal speed, and lands
/// on Analyse's window starts for steps of 0.5, 1, 3 and 5 s.
pub const GRID_S: f64 = 0.25;

/// Shortest gap between two moves of waiting readings into `results`, in ms.
/// Each move redraws the rows, the overlay and the lists.
pub const FLUSH_MS: f64 = 500.0;

/// A stored reading keeps its [`TOP_K`] best scores only. The live view's
/// strip shows every class's score to 2 decimals, so a stored reading stands
/// in for a new one when its lowest kept score is under this: every class
/// left out then shows as 0.00.
pub const REUSE_TAIL_MAX: f32 = 0.005;

type Key = (u64, Method);

/// A reading on its way into a file's results.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub file_id: u64,
    pub method: Method,
    /// The file's sample rate when its audio was read.
    pub source_rate: u32,
    /// The window in the file, in seconds.
    pub start_s: f64,
    pub end_s: f64,
    /// The best [`TOP_K`] classes, as Analyse gives them.
    pub top: Vec<Score>,
    pub embedding: Vec<f32>,
}

thread_local! {
    static WAITING: RefCell<Vec<Reading>> = const { RefCell::new(Vec::new()) };
    static LAST_FLUSH: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
}

/// The start of the window the live view reads while a file plays with the
/// playhead at `playhead_s`, in file seconds: the last grid point ([`GRID_S`])
/// at least `window_s` before the playhead. `None` until a whole window has
/// played.
pub fn snap_start(playhead_s: f64, window_s: f64) -> Option<f64> {
    let s = playhead_s - window_s;
    // `!(s >= 0)` also catches NaN.
    if !(s >= 0.0) {
        return None;
    }
    Some((s / GRID_S + 1e-9).floor() * GRID_S)
}

/// Every class's score from a stored reading's best classes, with 0 for the
/// rest, when that shows the same as the full scores would
/// ([`REUSE_TAIL_MAX`]). `None` otherwise: the live view runs the model.
pub fn full_scores(top: &[Score], classes: usize) -> Option<Vec<f32>> {
    if classes == 0 || top.len() < TOP_K.min(classes) {
        return None;
    }
    if top.len() < classes && top.last()?.confidence >= REUSE_TAIL_MAX {
        return None;
    }
    fill_scores(top, classes)
}

/// Every class's score, from `top` with 0 for the rest: enough for what
/// uses the best few classes only. `None` when `top` has a class past
/// `classes` or no scores at all.
pub fn fill_scores(top: &[Score], classes: usize) -> Option<Vec<f32>> {
    if top.is_empty() {
        return None;
    }
    let mut scores = vec![0.0; classes];
    for s in top {
        *scores.get_mut(s.class as usize)? = s.confidence;
    }
    Some(scores)
}

/// A reading of `file_id`'s window starting at `start_s` (within
/// `SAME_WINDOW_S`) for `method`, already kept: its top classes and its
/// embedding. From the waiting readings or the results; only a whole
/// window, from a run of the file at `source_rate`.
pub fn stored(
    state: AppState,
    file_id: u64,
    method: Method,
    source_rate: u32,
    start_s: f64,
) -> Option<(Vec<Score>, Vec<f32>)> {
    let key = (file_id, method);
    WAITING.with(|w| {
        state.classifier.results().with_untracked(|results| {
            state.classifier.embeddings().with_untracked(|embeddings| {
                find(
                    &w.borrow(),
                    results.get(&key),
                    embeddings.get(&key).map(Vec::as_slice),
                    (key, source_rate, start_s),
                )
            })
        })
    })
}

/// [`stored`] in `waiting` (readings not in the results yet), then in the
/// file's `run` and its `embeddings`, for the window `(key, source_rate,
/// start_s)`.
pub fn find(
    waiting: &[Reading],
    run: Option<&ClassifierRun>,
    embeddings: Option<&[f32]>,
    (key, source_rate, start_s): (Key, u32, f64),
) -> Option<(Vec<Score>, Vec<f32>)> {
    let same = |r: &Reading| {
        (r.file_id, r.method) == key
            && r.source_rate == source_rate
            && (r.start_s - start_s).abs() < super::types::SAME_WINDOW_S
    };
    if let Some(r) = waiting.iter().find(|r| same(r)) {
        return Some((r.top.clone(), r.embedding.clone()));
    }
    let run = run.filter(|r| r.source_rate == source_rate)?;
    let w = run.full_window_at(start_s)?;
    let e = super::types::embedding(embeddings?, key.1, w.slot as usize)?;
    Some((w.top.clone(), e.to_vec()))
}

/// Keep `reading`. It goes into the results with the next move, at most
/// [`FLUSH_MS`] after the last one; `now` is `performance.now()`.
pub fn keep(state: AppState, reading: Reading, now: f64) {
    WAITING.with(|w| w.borrow_mut().push(reading));
    if flush_due(now, LAST_FLUSH.with(Cell::get)) {
        flush(state, now);
    }
}

/// True when a reading kept at `now` moves the waiting ones into the
/// results: [`FLUSH_MS`] or more after the last move at `last` (ms).
pub fn flush_due(now: f64, last: f64) -> bool {
    now - last >= FLUSH_MS
}

/// True while readings wait to go into the results.
pub fn waiting() -> bool {
    WAITING.with(|w| !w.borrow().is_empty())
}

/// Move the waiting readings into the results now.
pub fn flush(state: AppState, now: f64) {
    LAST_FLUSH.with(|l| l.set(now));
    let readings = WAITING.with(|w| std::mem::take(&mut *w.borrow_mut()));
    if readings.is_empty() {
        return;
    }
    let mut embeddings = state
        .classifier
        .embeddings()
        .try_update_untracked(std::mem::take)
        .unwrap_or_default();
    let mut changed = false;
    state.classifier.results().maybe_update(|results| {
        changed = merge(results, &mut embeddings, readings, new_run);
        changed
    });
    state.classifier.embeddings().maybe_update(|m| {
        *m = embeddings;
        changed
    });
}

/// The run a file's first kept reading starts for `method`: the same as
/// Analyse starts with for windows end to end, with no windows. `None` for a
/// method the live view does not run, or before its labels are read.
fn new_run(r: &Reading) -> Option<ClassifierRun> {
    let choice = match r.method {
        Method::BirdNet => catalog::ModelChoice::BirdNet,
        Method::Perch => catalog::ModelChoice::Perch,
        Method::Bats | Method::BirdNetV3 => return None,
    };
    let labels = super::cached_labels(match r.method {
        Method::Perch => &catalog::PERCH_LABELS,
        _ => &catalog::BIRDNET_LABELS,
    })?;
    let (backbone, head) = super::model_files(choice);
    Some(ClassifierRun::empty(
        r.method,
        None,
        backbone,
        head,
        r.source_rate,
        labels,
        r.method.window_secs(),
    ))
}

/// Put `readings` into `results` and `embeddings`, each marked as the live
/// view's, starting a run with `new_run` for a file and method with none.
/// A reading is left out when its run is of the file at another sample rate
/// or already has a window with its start. Returns true when any went in.
pub fn merge(
    results: &mut HashMap<Key, ClassifierRun>,
    embeddings: &mut HashMap<Key, Vec<f32>>,
    readings: Vec<Reading>,
    mut new_run: impl FnMut(&Reading) -> Option<ClassifierRun>,
) -> bool {
    let mut changed = false;
    for r in readings {
        let key = (r.file_id, r.method);
        if !results.contains_key(&key) {
            let Some(run) = new_run(&r) else { continue };
            results.insert(key, run);
            embeddings.insert(key, Vec::new());
            changed = true;
        }
        let Some(run) = results.get_mut(&key) else {
            continue;
        };
        if run.source_rate != r.source_rate {
            continue;
        }
        let emb = embeddings.entry(key).or_default();
        if let Some(i) = run.insert_reading(emb, r.start_s, r.end_s, r.top, &r.embedding) {
            run.windows[i].live = true;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::types::{cells_secs, embedding};
    use std::sync::Arc;

    #[test]
    fn a_reading_starts_on_the_grid_a_window_before_the_playhead() {
        assert_eq!(snap_start(2.9, 3.0), None);
        assert_eq!(snap_start(f64::NAN, 3.0), None);
        assert_eq!(snap_start(3.0, 3.0), Some(0.0));
        assert_eq!(snap_start(3.24, 3.0), Some(0.0));
        assert_eq!(snap_start(3.25, 3.0), Some(0.25));
        assert_eq!(snap_start(10.6, 3.0), Some(7.5));
        assert_eq!(snap_start(10.6, 5.0), Some(5.5));
        // The playhead runs at a quarter speed (time expansion): the same
        // window for a second of playback, then the next.
        let starts: Vec<f64> = (0..8)
            .filter_map(|k| snap_start(12.0 + k as f64 * 0.0625, 3.0))
            .collect();
        assert_eq!(starts, vec![9.0, 9.0, 9.0, 9.0, 9.25, 9.25, 9.25, 9.25]);
    }

    /// A playhead that moves on by the time between readings (a little over
    /// the model's 250 ms) gives a reading for most grid points, never two at
    /// one, and every start is a whole window behind the playhead, at most
    /// one grid step more.
    #[test]
    fn readings_while_playing_follow_the_playhead() {
        let mut last = None;
        let mut t = 3.0;
        while t < 30.0 {
            let s = snap_start(t, 3.0).unwrap();
            let behind = t - 3.0 - s;
            assert!(behind > -1e-6 && behind < GRID_S + 1e-6, "{t} {s}");
            assert_ne!(Some(s), last);
            last = Some(s);
            t += 0.27;
        }
    }

    /// Readings about 4 times a second move into the results about twice a
    /// second, 2 at a time; the first goes in at once.
    #[test]
    fn readings_move_into_the_results_at_most_twice_a_second() {
        let mut last = f64::NEG_INFINITY;
        let mut moves = Vec::new();
        let mut waiting = 0;
        for k in 0..40 {
            let now = 1000.0 + k as f64 * 260.0;
            waiting += 1;
            if flush_due(now, last) {
                moves.push(waiting);
                waiting = 0;
                last = now;
            }
        }
        assert_eq!(moves[0], 1);
        assert!(moves[1..].iter().all(|&n| n == 2), "{moves:?}");
        // 40 readings over 10.4 s: 20 moves (the last reading waits).
        assert_eq!(moves.len(), 20);
        assert!(!flush_due(1000.0 + FLUSH_MS - 1.0, 1000.0));
        assert!(flush_due(1000.0 + FLUSH_MS, 1000.0));
    }

    fn score(class: u16, confidence: f32) -> Score {
        Score { class, confidence }
    }

    #[test]
    fn stored_scores_stand_in_only_when_the_rest_show_as_zero() {
        let mut top: Vec<Score> = (0..10).map(|k| score(k, 0.5 - k as f32 * 0.05)).collect();
        // The 10th best is 0.05: classes left out could be up to 0.05.
        assert_eq!(full_scores(&top, 6522), None);
        top[9].confidence = 0.004;
        let full = full_scores(&top, 6522).unwrap();
        assert_eq!(full.len(), 6522);
        assert_eq!(full[0], 0.5);
        assert_eq!(full[9], 0.004);
        assert_eq!(full[100], 0.0);
        // A reading with no scores (not scored) or too few never stands in.
        assert_eq!(full_scores(&[], 6522), None);
        assert_eq!(full_scores(&top[..5], 6522), None);
        // A model with 10 classes or fewer: the top classes are all of them.
        let few = [score(1, 0.9), score(0, 0.3)];
        assert_eq!(full_scores(&few, 2), Some(vec![0.3, 0.9]));
        // A class number past the model's classes: not this model's reading.
        top[0].class = 7000;
        assert_eq!(full_scores(&top, 6522), None);
    }

    fn labels() -> Arc<Vec<crate::classifier::SpeciesLabel>> {
        Arc::new(crate::classifier::types::parse_labels("A a_A\nB b_B\n"))
    }

    fn run_for(r: &Reading) -> Option<ClassifierRun> {
        Some(ClassifierRun::empty(
            r.method,
            None,
            &catalog::BACKBONE,
            None,
            r.source_rate,
            labels(),
            r.method.window_secs(),
        ))
    }

    fn reading(file_id: u64, start_s: f64, class: u16) -> Reading {
        Reading {
            file_id,
            method: Method::BirdNet,
            source_rate: 44_100,
            start_s,
            end_s: start_s + 3.0,
            top: vec![score(class, 0.8)],
            embedding: vec![class as f32; Method::BirdNet.embedding_len()],
        }
    }

    /// The first readings of a file start a run that is the same shape as
    /// Analyse's, says no more than it knows (not the whole file, not
    /// complete, no step asked for), and counts time only where it has
    /// readings.
    #[test]
    fn readings_start_a_run_that_claims_only_what_was_played() {
        let mut results = HashMap::new();
        let mut embeddings = HashMap::new();
        let readings: Vec<Reading> = (0..8)
            .map(|k| reading(7, 20.0 + k as f64 * 0.25, 1))
            .collect();
        assert!(merge(&mut results, &mut embeddings, readings, run_for));
        let run = &results[&(7, Method::BirdNet)];
        assert!(!run.whole_file && !run.complete);
        assert_eq!(run.step_secs, run.window_secs);
        assert_eq!(run.spacing_secs, run.window_secs);
        assert_eq!(run.windows.len(), 8);
        assert!(run.windows.iter().all(|w| w.live));
        assert_eq!(run.analysed_windows(), 0);
        // 20 s to 24.75 s were heard: that much time, no more.
        let c = run.cells();
        assert_eq!(c.first().unwrap().0, 20.0);
        assert_eq!(c.last().unwrap().1, 24.75);
        assert!((cells_secs(c.iter()) - 4.75).abs() < 1e-9);
        assert!(run.overlapping());
        assert_eq!(
            embeddings[&(7, Method::BirdNet)].len(),
            8 * Method::BirdNet.embedding_len()
        );
    }

    #[test]
    fn readings_go_in_once_and_only_into_a_run_of_the_same_audio() {
        let mut results = HashMap::new();
        let mut embeddings = HashMap::new();
        assert!(merge(
            &mut results,
            &mut embeddings,
            vec![reading(1, 3.0, 0)],
            run_for
        ));
        // The same start again (a part played twice): nothing new.
        assert!(!merge(
            &mut results,
            &mut embeddings,
            vec![reading(1, 3.01, 1)],
            run_for
        ));
        // The file's rate changed since (a rate correction): left out.
        let mut other = reading(1, 9.0, 1);
        other.source_rate = 441_000;
        assert!(!merge(&mut results, &mut embeddings, vec![other], run_for));
        // A method the live view does not keep: no run is started.
        let mut bats = reading(2, 0.0, 0);
        bats.method = Method::Bats;
        assert!(!merge(&mut results, &mut embeddings, vec![bats], |_| None));
        assert_eq!(results.len(), 1);
        let run = &results[&(1, Method::BirdNet)];
        assert_eq!(run.windows.len(), 1);
        assert_eq!(run.windows[0].top[0].class, 0);
    }

    /// Which stored reading the live view shows instead of running the model.
    #[test]
    fn stored_readings_are_found_waiting_or_in_the_results() {
        let key = (1, Method::BirdNet);
        let mut results = HashMap::new();
        let mut embeddings = HashMap::new();
        merge(
            &mut results,
            &mut embeddings,
            vec![reading(1, 6.0, 4)],
            run_for,
        );
        // A run's zero-padded last window, 9 s to 10.5 s.
        {
            let run = results.get_mut(&key).unwrap();
            let e = embeddings.get_mut(&key).unwrap();
            run.insert_reading(e, 9.0, 10.5, vec![score(8, 0.9)], &[8.0; 1024]);
        }
        let waiting = vec![reading(1, 2.5, 7)];
        let look = |start: f64, rate: u32| {
            find(
                &waiting,
                results.get(&key),
                embeddings.get(&key).map(Vec::as_slice),
                (key, rate, start),
            )
            .map(|(top, e)| (top[0].class, e[0]))
        };
        // Waiting to go into the results, and within 20 ms of the start.
        assert_eq!(look(2.51, 44_100), Some((7, 7.0)));
        // In the results, with its own embedding.
        assert_eq!(look(5.99, 44_100), Some((4, 4.0)));
        // 30 ms off: another window.
        assert_eq!(look(6.03, 44_100), None);
        // A zero-padded window is not the whole window that starts there.
        assert_eq!(look(9.0, 44_100), None);
        // The file at another sample rate (a rate correction since).
        assert_eq!(look(6.0, 441_000), None);
        assert_eq!(look(2.5, 441_000), None);
        // Another file.
        let other = find(&waiting, None, None, ((2, Method::BirdNet), 44_100, 2.5));
        assert_eq!(other, None);
    }

    /// Analyse keeps the live view's readings, with their embeddings, and
    /// drops the windows of the Analyse before; its own windows then go in
    /// around them, and one at a kept reading's start is not added twice.
    #[test]
    fn analyse_keeps_the_readings_made_while_playing() {
        let mut results = HashMap::new();
        let mut embeddings = HashMap::new();
        let key = (1, Method::BirdNet);
        merge(
            &mut results,
            &mut embeddings,
            vec![reading(1, 6.0, 4), reading(1, 1.25, 5)],
            run_for,
        );
        // An earlier Analyse's windows, end to end.
        {
            let run = results.get_mut(&key).unwrap();
            let e = embeddings.get_mut(&key).unwrap();
            for k in 0..4 {
                let s = k as f64 * 3.0;
                run.insert_reading(e, s, s + 3.0, vec![score(9, 0.7)], &[9.0; 1024]);
            }
        }
        let old = &results[&key];
        assert_eq!(old.windows.len(), 5, "6.0 was already there");
        assert_eq!(old.analysed_windows(), 3);
        let (kept, kept_emb) = old.live_windows(&embeddings[&key]);
        let starts: Vec<f64> = kept.iter().map(|w| w.start_s).collect();
        assert_eq!(starts, vec![1.25, 6.0]);
        // The embeddings keep the order they arrived in: 6.0 s first.
        assert_eq!(kept.iter().map(|w| w.slot).collect::<Vec<_>>(), vec![1, 0]);
        for w in &kept {
            let want = w.top[0].class as f32;
            assert!(embedding(&kept_emb, Method::BirdNet, w.slot as usize)
                .unwrap()
                .iter()
                .all(|&v| v == want));
        }
        assert_eq!(kept_emb.len(), 2 * 1024);
        // The new Analyse starts from them.
        let mut run = run_for(&reading(1, 0.0, 0)).unwrap();
        run.windows = kept;
        let mut e = kept_emb;
        for k in 0..4 {
            let s = k as f64 * 3.0;
            run.insert_reading(&mut e, s, s + 3.0, vec![score(2, 0.6)], &[2.0; 1024]);
        }
        run.whole_file = true;
        assert_eq!(run.windows.len(), 5);
        assert_eq!(run.analysed_windows(), 3);
        // The window at 6 s is the kept reading.
        let w = run.full_window_at(6.0).unwrap();
        assert!(w.live && w.top[0].class == 4);
        let mut slots: Vec<u32> = run.windows.iter().map(|w| w.slot).collect();
        slots.sort_unstable();
        assert_eq!(slots, (0..5).collect::<Vec<u32>>());
    }
}

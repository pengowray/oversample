//! Data the classifier keeps in [`crate::state::ClassifierState`].

use super::catalog::ModelFile;
use super::{BatScope, ClassifierError, Method, Score};
use std::sync::Arc;

/// Whether one model file is on this device.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum DownloadStatus {
    /// Not checked yet. [`super::refresh_downloads`] checks every file.
    #[default]
    Unknown,
    NotDownloaded,
    /// `fraction` is 0 to 1.
    Downloading {
        fraction: f64,
    },
    Downloaded,
    Failed(ClassifierError),
}

/// What the classifier is doing. One run or rescore at a time.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum RunStatus {
    #[default]
    Idle,
    /// Reading the model from storage and building it in the worker (about 1
    /// to 2 s; longer the first time, while the worker's WASM loads).
    LoadingModel {
        file_id: u64,
        method: Method,
    },
    /// Analysing; `fraction` (0 to 1) is the share of the recording done
    /// in this pass. Results fill in as it goes. `refining` is `None` for the
    /// windows end to end, and for each later pass of a run with a shorter
    /// step, the step between window starts it brings the whole recording to
    /// (`oversample_ipc::classifier::refine_passes`), in seconds.
    Running {
        file_id: u64,
        method: Method,
        fraction: f64,
        refining: Option<f64>,
    },
    /// Scoring stored BattyBirdNET embeddings with another region's head, or
    /// with all of them.
    Rescoring {
        file_id: u64,
        scope: BatScope,
    },
    Done {
        file_id: u64,
        method: Method,
    },
    Failed {
        file_id: u64,
        method: Method,
        error: ClassifierError,
    },
}

impl RunStatus {
    /// True while a run or rescore is going.
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            RunStatus::LoadingModel { .. }
                | RunStatus::Running { .. }
                | RunStatus::Rescoring { .. }
        )
    }
}

/// One line of a label file: "Pipistrellus pipistrellus_Common pipistrelle".
/// A line without an underscore (BattyBirdNET's "noise", "background",
/// "silence", "audiomoth", "other") has the whole line in both fields and
/// `is_species` false.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeciesLabel {
    pub scientific: String,
    pub common: String,
    /// False for BattyBirdNET's classes that are not a species, such as
    /// "noise". BirdNET's non-bird classes ("Dog_Dog") count as species here:
    /// they are named sounds.
    pub is_species: bool,
    /// A named sound rather than an animal: BirdNET's "Dog_Dog" and
    /// "Engine_Engine" (the same name twice), and Perch's sound events
    /// ("Thunder"). Listed with the species, but counted as sounds.
    pub is_sound: bool,
}

impl SpeciesLabel {
    pub fn parse(line: &str) -> SpeciesLabel {
        let line = line.trim();
        match line.split_once('_') {
            Some((s, c)) => SpeciesLabel {
                scientific: s.to_string(),
                common: c.to_string(),
                is_species: true,
                is_sound: s == c,
            },
            None => SpeciesLabel {
                scientific: line.to_string(),
                common: line.to_string(),
                is_species: false,
                is_sound: false,
            },
        }
    }

    /// True for BattyBirdNET's "NOISE" class.
    pub fn is_noise(&self) -> bool {
        self.scientific.eq_ignore_ascii_case("noise")
    }
}

/// Parse a label file, one label per non-empty line.
pub fn parse_labels(text: &str) -> Vec<SpeciesLabel> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(SpeciesLabel::parse)
        .collect()
}

/// Parse Perch's `labels.csv`: a header line, then one class per line, with no
/// common names. Species are scientific names ("Apus apus"); the 198 sound
/// events are FSD50K names, where "_and_" stands for a comma and "_" for a space
/// ("Splash_and_splatter" becomes "Splash, splatter"). Every class counts as a
/// species here, as BirdNET's named sounds ("Dog", "Engine") do, so thunder or
/// traffic is listed and can be added as an annotation.
pub fn parse_perch_labels(text: &str) -> Vec<SpeciesLabel> {
    text.lines()
        .skip(1)
        .map(|l| {
            let l = l.trim();
            let is_sound = !l.contains(' ');
            let name = if is_sound {
                l.replace("_and_", ", ").replace('_', " ")
            } else {
                l.to_string()
            };
            SpeciesLabel {
                scientific: name.clone(),
                common: name,
                is_species: true,
                is_sound,
            }
        })
        .collect()
}

/// Parse BirdNET+ V3.0's trimmed labels (`catalog::BIRDNET_V3_LABELS_ASSET`):
/// one class per line, tab-separated: BirdNET id, scientific name, English
/// name, taxon group. A line with no English name gets the scientific name.
/// The taxon groups Anthropogenic, Geophony and Music ("Engine", "Fire",
/// "Music") are named sounds; every other class, Human included, is a species.
pub fn parse_birdnet_v3_labels(text: &str) -> Vec<SpeciesLabel> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut f = l.split('\t').map(str::trim);
            let _id = f.next();
            let scientific = f.next().unwrap_or_default().to_string();
            let common = f
                .next()
                .filter(|c| !c.is_empty())
                .map_or_else(|| scientific.clone(), str::to_string);
            let group = f.next().unwrap_or_default();
            SpeciesLabel {
                scientific,
                common,
                is_species: true,
                is_sound: matches!(group, "Anthropogenic" | "Geophony" | "Music"),
            }
        })
        .collect()
}

/// Give each label whose scientific name is in `names` ("Genus species" to an
/// English name) that common name. Others keep the scientific name.
pub fn add_common_names(
    labels: &mut [SpeciesLabel],
    names: &std::collections::HashMap<String, String>,
) {
    for l in labels {
        if let Some(en) = names.get(&l.scientific) {
            l.common = en.clone();
        }
    }
}

/// One window's scores.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowScores {
    /// Start in the recording, in seconds. With a step shorter than the
    /// window, windows overlap; their starts can be anywhere.
    pub start_s: f64,
    /// End in the recording, in seconds: the start plus the window length,
    /// or the end of the recording for a zero-padded last window.
    pub end_s: f64,
    /// Up to [`super::TOP_K`] classes, highest confidence first. `class` indexes
    /// [`ClassifierRun::labels`]. Empty for a window that was not scored
    /// (BirdNET+ V3.0 on digital silence), which counts as below any minimum.
    pub top: Vec<Score>,
    /// Where its embedding is in the run's embeddings
    /// (`ClassifierState::embeddings`), which are kept in the order the
    /// windows arrived ([`embedding`]).
    pub slot: u32,
    /// Made by the live view from the file's own audio, while the file
    /// played or of the window Play starts with (`crate::classifier::
    /// played`), not by Analyse. The scores are the ones Analyse gives a
    /// window with the same start.
    pub live: bool,
}

/// Two window starts closer than this, in seconds, are the same window:
/// [`ClassifierRun::insert_reading`] keeps the first.
pub const SAME_WINDOW_S: f64 = 0.02;

/// The result of one run on one file. Its embeddings are kept separately, in
/// `ClassifierState::embeddings` under the same key, so code that only shows
/// scores never touches megabytes of floats.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassifierRun {
    pub method: Method,
    /// The region whose head gave the scores, or all regions (BattyBirdNET
    /// only).
    pub region: Option<BatScope>,
    /// The model that took the audio: BirdNET's backbone, Perch's embedding
    /// model or BirdNET+ V3.0 (its URL holds the pinned revision).
    pub backbone: &'static ModelFile,
    /// The model that scored it: a BattyBirdNET regional head or Perch's
    /// classifier. `None` for BirdNET, and for BattyBirdNET's "All regions",
    /// which scores with every regional head.
    pub head: Option<&'static ModelFile>,
    /// The recording's sample rate.
    pub source_rate: u32,
    /// The rate the audio was resampled to (48, 256 or 32 kHz).
    pub model_rate: u32,
    /// Length of one window in the recording: 3 s, 0.5625 s or 5 s.
    pub window_secs: f64,
    /// The step between window starts that the Analyse asked for, in
    /// seconds: `window_secs` for windows end to end. Only what was asked:
    /// nothing assumes the windows sit on this step.
    pub step_secs: f64,
    /// The step between window starts over the whole recording so far: the
    /// window length after the windows end to end, then shorter after each
    /// later pass (`oversample_ipc::classifier::refine_passes`).
    pub spacing_secs: f64,
    /// Names for `Score::class`, in the model's class order.
    pub labels: Arc<Vec<SpeciesLabel>>,
    /// Windows in time order of their starts. Windows can overlap, and later
    /// passes insert windows between earlier ones, so a window's place in
    /// the list changes; its start is what stays.
    pub windows: Vec<WindowScores>,
    /// The windows end to end are done, so the whole recording has results.
    pub whole_file: bool,
    /// False while the run is going, and for a run that was cancelled or failed
    /// part way (its windows so far are kept).
    pub complete: bool,
}

impl ClassifierRun {
    /// The label for a score.
    pub fn label(&self, score: &Score) -> Option<&SpeciesLabel> {
        self.labels.get(score.class as usize)
    }

    /// Add one window of results at any start time, keeping the windows in
    /// time order and `embeddings` (this run's, `ClassifierState::
    /// embeddings`) lined up with them. `embedding` is the window's
    /// embedding; one of the wrong length is stored as zeros. A start within
    /// [`SAME_WINDOW_S`] of a window already there is the same window: nothing
    /// is added and `None` is returned. Otherwise returns the new window's
    /// place in [`Self::windows`].
    pub fn insert_reading(
        &mut self,
        embeddings: &mut Vec<f32>,
        start_s: f64,
        end_s: f64,
        top: Vec<Score>,
        embedding: &[f32],
    ) -> Option<usize> {
        let at = self.windows.partition_point(|w| w.start_s < start_s);
        let near = |i: usize| {
            self.windows
                .get(i)
                .is_some_and(|w| (w.start_s - start_s).abs() < SAME_WINDOW_S)
        };
        if near(at) || (at > 0 && near(at - 1)) {
            return None;
        }
        let len = self.method.embedding_len();
        let slot = (embeddings.len() / len) as u32;
        embeddings.truncate(slot as usize * len);
        if embedding.len() == len {
            embeddings.extend_from_slice(embedding);
        } else {
            embeddings.resize((slot as usize + 1) * len, 0.0);
        }
        self.windows.insert(
            at,
            WindowScores {
                start_s,
                end_s,
                top,
                slot,
                live: false,
            },
        );
        Some(at)
    }

    /// A run with no windows yet: what Analyse starts with, and what the live
    /// view starts a file's results with (`crate::classifier::played`).
    /// `step_secs` is the step Analyse asked for, the window length for
    /// windows end to end and for a run with no Analyse.
    pub fn empty(
        method: Method,
        region: Option<BatScope>,
        backbone: &'static ModelFile,
        head: Option<&'static ModelFile>,
        source_rate: u32,
        labels: Arc<Vec<SpeciesLabel>>,
        step_secs: f64,
    ) -> ClassifierRun {
        ClassifierRun {
            method,
            region,
            backbone,
            head,
            source_rate,
            model_rate: method.model_rate(),
            window_secs: method.window_secs(),
            step_secs,
            spacing_secs: method.window_secs(),
            labels,
            windows: Vec::new(),
            whole_file: false,
            complete: false,
        }
    }

    /// The windows the live view made ([`WindowScores::live`]) and their
    /// embeddings, from `embeddings` (this run's), as a new run starts with
    /// them: the windows in time order, their embeddings in the order they
    /// arrived, as before, in slots 0, 1, 2...
    pub fn live_windows(&self, embeddings: &[f32]) -> (Vec<WindowScores>, Vec<f32>) {
        let len = self.method.embedding_len();
        let mut old_slots: Vec<u32> = self
            .windows
            .iter()
            .filter(|w| w.live)
            .map(|w| w.slot)
            .collect();
        old_slots.sort_unstable();
        let mut emb = Vec::with_capacity(old_slots.len() * len);
        for &s in &old_slots {
            match embedding(embeddings, self.method, s as usize) {
                Some(e) => emb.extend_from_slice(e),
                None => emb.resize(emb.len() + len, 0.0),
            }
        }
        let kept = self
            .windows
            .iter()
            .filter(|w| w.live)
            .map(|w| WindowScores {
                slot: old_slots.partition_point(|&s| s < w.slot) as u32,
                ..w.clone()
            })
            .collect();
        (kept, emb)
    }

    /// Windows Analyse made (not [`WindowScores::live`]).
    pub fn analysed_windows(&self) -> usize {
        self.windows.iter().filter(|w| !w.live).count()
    }

    /// The whole window starting at `start_s` (within [`SAME_WINDOW_S`]):
    /// one of a full window's length, not a run's zero-padded last window.
    pub fn full_window_at(&self, start_s: f64) -> Option<&WindowScores> {
        let w = self.windows.get(self.window_at_start(start_s)?)?;
        (w.end_s - w.start_s >= self.window_secs - 1e-6).then_some(w)
    }

    /// The window starting at `start_s` (within [`SAME_WINDOW_S`]).
    pub fn window_at_start(&self, start_s: f64) -> Option<usize> {
        let at = self.windows.partition_point(|w| w.start_s < start_s);
        [at.checked_sub(1), Some(at)]
            .into_iter()
            .flatten()
            .find(|&i| {
                self.windows
                    .get(i)
                    .is_some_and(|w| (w.start_s - start_s).abs() < SAME_WINDOW_S)
            })
    }

    /// Each window's cell: the part of the timeline it is drawn on
    /// ([`cells`]).
    pub fn cells(&self) -> Vec<(f64, f64)> {
        cells(
            self.windows.iter().map(|w| (w.start_s, w.end_s)),
            self.window_secs,
        )
    }

    /// True when any window overlaps the next: counts of windows would count
    /// the same audio more than once, so times are shown instead.
    pub fn overlapping(&self) -> bool {
        self.windows
            .windows(2)
            .any(|p| p[1].start_s < p[0].end_s - SAME_WINDOW_S)
    }
}

/// The cell of each window, for drawing and for counting time: one part of
/// the timeline per window, in order, with no gaps or overlaps between
/// windows that overlap or touch. `windows` are each window's start and end
/// in time order; `window_secs` is the full window length.
///
/// A cell is centred on its window: the boundary between two neighbours is
/// halfway between their middles, taking each middle as start + half a
/// window (also for a short last window, so windows end to end get cells
/// equal to the windows). The first cell starts at the first window's start
/// and the last ends at the last window's end, and no cell goes past its
/// own window, so windows with a gap between them keep the gap.
pub fn cells(windows: impl Iterator<Item = (f64, f64)>, window_secs: f64) -> Vec<(f64, f64)> {
    let w: Vec<(f64, f64)> = windows.collect();
    let n = w.len();
    let boundary = |i: usize| 0.5 * (w[i].0 + w[i + 1].0 + window_secs);
    (0..n)
        .map(|i| {
            let (s, e) = w[i];
            let lo = if i == 0 { s } else { boundary(i - 1).max(s) };
            let hi = if i + 1 == n { e } else { boundary(i).min(e) };
            (lo.min(e), hi.max(lo.min(e)))
        })
        .collect()
}

/// Total length of `cells`, in seconds.
pub fn cells_secs<'a>(cells: impl Iterator<Item = &'a (f64, f64)>) -> f64 {
    cells.map(|(s, e)| e - s).sum()
}

/// The embedding of the window at `slot` ([`WindowScores::slot`]) in a run's
/// embeddings (`ClassifierState::embeddings`): `method.embedding_len()`
/// floats per window (1,024 for BirdNET, 1,536 for Perch, 1,280 for BirdNET+
/// V3.0), back to back, in the order the windows arrived. A BirdNET+ V3.0
/// window of digital silence has an embedding of zeros (it is not run).
pub fn embedding(embeddings: &[f32], method: Method, slot: usize) -> Option<&[f32]> {
    let len = method.embedding_len();
    embeddings.get(slot * len..(slot + 1) * len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_parse() {
        let l = parse_labels("Pipistrellus pipistrellus_Common pipistrelle\nNOISE\nDog_Dog\n\n");
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].scientific, "Pipistrellus pipistrellus");
        assert_eq!(l[0].common, "Common pipistrelle");
        assert!(!l[0].is_sound);
        assert!(l[1].is_noise() && !l[1].is_sound);
        assert!(l[2].is_species && l[2].is_sound);
    }

    #[test]
    fn perch_labels_parse() {
        let l = parse_perch_labels(
            "inat2024_fsd50k\nApus apus\nSplash_and_splatter\nMotor_vehicle_(road)\n",
        );
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].common, "Apus apus");
        assert_eq!(l[1].common, "Splash, splatter");
        assert_eq!(l[2].common, "Motor vehicle (road)");
        assert!(l.iter().all(|x| x.is_species));
        assert!(!l[0].is_sound && l[1].is_sound && l[2].is_sound);
    }

    #[test]
    fn birdnet_v3_labels_parse() {
        let l = parse_birdnet_v3_labels(
            "BN00001\tAbeillia abeillei\tEmerald-chinned Hummingbird\tAves\n\
             BN30001\tAcanthacris ruficornis\t\tInsecta\n\
             BN21000\tHomo sapiens\tHuman\tMammalia\n\
             BN90057\tFire\tFire\tGeophony\n\
             BN90050\tEngine\tEngine\tAnthropogenic\n\
             BN90081\tMusic\tMusic\tMusic\n\n",
        );
        assert_eq!(l.len(), 6);
        assert_eq!(l[0].scientific, "Abeillia abeillei");
        assert_eq!(l[0].common, "Emerald-chinned Hummingbird");
        // No English name: the scientific name stands in.
        assert_eq!(l[1].common, "Acanthacris ruficornis");
        assert!(l.iter().all(|x| x.is_species));
        assert!(!l[0].is_sound && !l[1].is_sound && !l[2].is_sound);
        assert!(l[3].is_sound && l[4].is_sound && l[5].is_sound);
    }

    #[test]
    fn birdnet_v3_label_file_lines_up_with_the_model() {
        let text = include_str!("../../research/birdnet-v3-bench/birdnet-v3-labels.tsv");
        let l = parse_birdnet_v3_labels(text);
        assert_eq!(l.len(), super::super::V3_CLASSES);
        assert_eq!(l.iter().filter(|x| x.is_sound).count(), 16);
        assert!(l
            .iter()
            .all(|x| !x.scientific.is_empty() && !x.common.is_empty()));
        let human = l.iter().find(|x| x.scientific == "Homo sapiens").unwrap();
        assert!(!human.is_sound && human.common == "Human");
    }

    fn empty_run(method: Method) -> ClassifierRun {
        ClassifierRun {
            method,
            region: None,
            backbone: &super::super::catalog::BACKBONE,
            head: None,
            source_rate: 48_000,
            model_rate: method.model_rate(),
            window_secs: method.window_secs(),
            step_secs: method.window_secs(),
            spacing_secs: method.window_secs(),
            labels: Arc::new(Vec::new()),
            windows: Vec::new(),
            whole_file: false,
            complete: false,
        }
    }

    fn score(class: u16) -> Vec<Score> {
        vec![Score {
            class,
            confidence: 0.5,
        }]
    }

    /// An embedding whose every value is `v`.
    fn emb(v: f32) -> Vec<f32> {
        vec![v; Method::BirdNet.embedding_len()]
    }

    fn assert_tiled(cells: &[(f64, f64)]) {
        for p in cells.windows(2) {
            assert!(p[0].0 <= p[0].1, "{cells:?}");
            assert!((p[0].1 - p[1].0).abs() < 1e-9, "gap or overlap: {cells:?}");
        }
    }

    #[test]
    fn readings_go_in_at_any_time_with_their_embeddings() {
        let mut r = empty_run(Method::BirdNet);
        let mut e = Vec::new();
        // Off-grid times, out of order, as the live view would give them.
        for (start, class) in [(6.0, 2), (0.0, 0), (3.137, 1), (1.25, 9), (4.71, 5)] {
            let i = r
                .insert_reading(&mut e, start, start + 3.0, score(class), &emb(class as f32))
                .unwrap();
            assert_eq!(r.windows[i].start_s, start);
        }
        let starts: Vec<f64> = r.windows.iter().map(|w| w.start_s).collect();
        assert_eq!(starts, vec![0.0, 1.25, 3.137, 4.71, 6.0]);
        // Each window's embedding is the one it came with.
        for w in &r.windows {
            let got = embedding(&e, Method::BirdNet, w.slot as usize).unwrap();
            assert!(got.iter().all(|&v| v == w.top[0].class as f32));
        }
        // Within 20 ms of a window already there: the same window.
        assert_eq!(
            r.insert_reading(&mut e, 3.15, 6.15, score(7), &emb(7.0)),
            None
        );
        assert_eq!(
            r.insert_reading(&mut e, 1.231, 4.231, score(7), &emb(7.0)),
            None
        );
        assert_eq!(r.windows.len(), 5);
        assert_eq!(e.len(), 5 * Method::BirdNet.embedding_len());
        assert_eq!(r.window_at_start(3.14), Some(2));
        assert_eq!(r.window_at_start(3.2), None);
        // A wrong-length embedding is stored as zeros, keeping the rest lined up.
        let i = r
            .insert_reading(&mut e, 9.0, 12.0, score(3), &[1.0])
            .unwrap();
        let slot = r.windows[i].slot as usize;
        assert!(embedding(&e, Method::BirdNet, slot)
            .unwrap()
            .iter()
            .all(|&v| v == 0.0));
        // Every slot belongs to exactly one window.
        let mut slots: Vec<u32> = r.windows.iter().map(|w| w.slot).collect();
        slots.sort_unstable();
        assert_eq!(slots, (0..r.windows.len() as u32).collect::<Vec<_>>());
        assert_tiled(&r.cells());
    }

    #[test]
    fn cells_of_windows_end_to_end_are_the_windows() {
        let w = [(0.0, 3.0), (3.0, 6.0), (6.0, 9.0), (9.0, 10.5)];
        assert_eq!(cells(w.into_iter(), 3.0), w.to_vec());
        assert_eq!(cells(std::iter::once((0.0, 0.4)), 3.0), vec![(0.0, 0.4)]);
        assert!(cells(std::iter::empty(), 3.0).is_empty());
    }

    #[test]
    fn cells_of_a_1_s_step_are_centred() {
        // Windows every 1 s from 0 to 7, then the short last one at 9.
        let mut w: Vec<(f64, f64)> = (0..=7).map(|k| (k as f64, k as f64 + 3.0)).collect();
        w.push((9.0, 10.5));
        let c = cells(w.into_iter(), 3.0);
        assert_tiled(&c);
        // The first window alone hears 0 to 1.5 s; each other cell is 1 s
        // round its window's middle.
        assert_eq!(c[0], (0.0, 2.0));
        assert_eq!(c[1], (2.0, 3.0));
        assert_eq!(c[6], (7.0, 8.0));
        assert_eq!(c[7], (8.0, 9.5));
        assert_eq!(c[8], (9.5, 10.5));
        assert!((cells_secs(c.iter()) - 10.5).abs() < 1e-9);
    }

    #[test]
    fn cells_narrow_as_windows_come_in_between() {
        let mut r = empty_run(Method::BirdNet);
        let mut e = Vec::new();
        for k in 0..4 {
            let s = k as f64 * 3.0;
            r.insert_reading(&mut e, s, s + 3.0, score(0), &emb(0.0));
        }
        assert_eq!(r.cells()[1], (3.0, 6.0));
        assert!(!r.overlapping());
        // The 1.5 s pass, then the rest of a 0.5 s step, front to back.
        let mut fill: Vec<f64> = (0..3).map(|k| k as f64 * 3.0 + 1.5).collect();
        fill.extend((0..18).map(|k| k as f64 * 0.5).filter(|s| s % 1.5 != 0.0));
        for s in fill {
            r.insert_reading(&mut e, s, s + 3.0, score(0), &emb(0.0));
            let c = r.cells();
            assert_tiled(&c);
            assert_eq!(c[0].0, 0.0);
            assert_eq!(c.last().unwrap().1, 12.0);
            assert!((cells_secs(c.iter()) - 12.0).abs() < 1e-9);
        }
        assert!(r.overlapping());
        assert_eq!(r.windows.len(), 19);
        assert_eq!(r.cells()[5], (3.75, 4.25));
    }

    #[test]
    fn cells_keep_a_gap_between_windows_that_do_not_touch() {
        let c = cells([(0.0, 3.0), (5.0, 8.0)].into_iter(), 3.0);
        assert_eq!(c, vec![(0.0, 3.0), (5.0, 8.0)]);
    }

    #[test]
    fn perch_common_names() {
        let mut l = parse_perch_labels("h\nApus apus\nAbavorana luctuosa\nThunder\n");
        let names = [("Apus apus".to_string(), "Common Swift".to_string())]
            .into_iter()
            .collect();
        add_common_names(&mut l, &names);
        assert_eq!(l[0].common, "Common Swift");
        assert_eq!(l[0].scientific, "Apus apus");
        assert_eq!(l[1].common, "Abavorana luctuosa");
        assert_eq!(l[2].common, "Thunder");
    }
}

//! Data the classifier keeps in [`crate::state::ClassifierState`].

use super::catalog::ModelFile;
use super::{BatRegion, ClassifierError, Method, Score};
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
    /// Analysing; `fraction` (0 to 1) is the share of the recording done.
    /// Results fill in as it goes.
    Running {
        file_id: u64,
        method: Method,
        fraction: f64,
    },
    /// Scoring stored BattyBirdNET embeddings with another region's head.
    Rescoring {
        file_id: u64,
        region: BatRegion,
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
/// A line without an underscore (BattyBirdNET's "NOISE") has the whole line in
/// both fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeciesLabel {
    pub scientific: String,
    pub common: String,
}

impl SpeciesLabel {
    pub fn parse(line: &str) -> SpeciesLabel {
        let line = line.trim();
        match line.split_once('_') {
            Some((s, c)) => SpeciesLabel {
                scientific: s.to_string(),
                common: c.to_string(),
            },
            None => SpeciesLabel {
                scientific: line.to_string(),
                common: line.to_string(),
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

/// One window's scores.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowScores {
    /// Start in the recording, in seconds (window `i` starts at
    /// `i * window_secs`).
    pub start_s: f64,
    /// End in the recording, in seconds. The last window ends at the end of the
    /// recording.
    pub end_s: f64,
    /// Up to [`super::TOP_K`] classes, highest confidence first. `class` indexes
    /// [`ClassifierRun::labels`].
    pub top: Vec<Score>,
}

/// The result of one run on one file. Its embeddings are kept separately, in
/// `ClassifierState::embeddings` under the same key, so code that only shows
/// scores never touches megabytes of floats.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassifierRun {
    pub method: Method,
    /// The region whose head gave the scores (BattyBirdNET only).
    pub region: Option<BatRegion>,
    /// The backbone file (its URL holds the pinned revision).
    pub backbone: &'static ModelFile,
    /// The regional head file (BattyBirdNET only).
    pub head: Option<&'static ModelFile>,
    /// The recording's sample rate.
    pub source_rate: u32,
    /// The rate the audio was resampled to (48 kHz or 256 kHz).
    pub model_rate: u32,
    /// Length of one window in the recording: 3 s or 0.5625 s.
    pub window_secs: f64,
    /// Names for `Score::class`, in the model's class order.
    pub labels: Arc<Vec<SpeciesLabel>>,
    /// Windows in time order; `windows[i]` is window `i`.
    pub windows: Vec<WindowScores>,
    /// False while the run is going, and for a run that was cancelled or failed
    /// part way (its windows so far are kept).
    pub complete: bool,
}

impl ClassifierRun {
    /// The label for a score.
    pub fn label(&self, score: &Score) -> Option<&SpeciesLabel> {
        self.labels.get(score.class as usize)
    }
}

/// Window `i`'s embedding in a run's embeddings
/// (`ClassifierState::embeddings`): `EMBEDDING_LEN` floats per window, back to
/// back, in window order.
pub fn embedding(embeddings: &[f32], i: usize) -> Option<&[f32]> {
    embeddings.get(i * super::EMBEDDING_LEN..(i + 1) * super::EMBEDDING_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_parse() {
        let l = parse_labels("Pipistrellus pipistrellus_Common pipistrelle\nNOISE\n\n");
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].scientific, "Pipistrellus pipistrellus");
        assert_eq!(l[0].common, "Common pipistrelle");
        assert!(l[1].is_noise());
    }
}

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
}

impl SpeciesLabel {
    pub fn parse(line: &str) -> SpeciesLabel {
        let line = line.trim();
        match line.split_once('_') {
            Some((s, c)) => SpeciesLabel {
                scientific: s.to_string(),
                common: c.to_string(),
                is_species: true,
            },
            None => SpeciesLabel {
                scientific: line.to_string(),
                common: line.to_string(),
                is_species: false,
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
            let name = if l.contains(' ') {
                l.to_string()
            } else {
                l.replace("_and_", ", ").replace('_', " ")
            };
            SpeciesLabel {
                scientific: name.clone(),
                common: name,
                is_species: true,
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
    /// The model that took the audio: BirdNET's backbone or Perch's embedding
    /// model (its URL holds the pinned revision).
    pub backbone: &'static ModelFile,
    /// The model that scored it: a BattyBirdNET regional head or Perch's
    /// classifier. `None` for BirdNET.
    pub head: Option<&'static ModelFile>,
    /// The recording's sample rate.
    pub source_rate: u32,
    /// The rate the audio was resampled to (48, 256 or 32 kHz).
    pub model_rate: u32,
    /// Length of one window in the recording: 3 s, 0.5625 s or 5 s.
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
/// (`ClassifierState::embeddings`): `method.embedding_len()` floats per window
/// (1,024 for BirdNET, 1,536 for Perch), back to back, in window order.
pub fn embedding(embeddings: &[f32], method: Method, i: usize) -> Option<&[f32]> {
    let len = method.embedding_len();
    embeddings.get(i * len..(i + 1) * len)
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

//! Types shared by the app and the BirdNET worker (`birdnet-worker`).
//!
//! The app (`src/classifier/`) and the worker exchange these messages with
//! `postMessage`. Each posted object holds the message (through
//! `serde_wasm_bindgen`) under [`attach::MSG`]; large arrays (model bytes,
//! audio, embeddings) travel beside it as transferred typed arrays, under the
//! other names in [`attach`].
//!
//! The first two methods use the same model, BirdNET V2.4 with an embeddings
//! output:
//!
//! - **BirdNET (birds):** audio at 48 kHz, 3 s windows of 144,000 samples. The
//!   model's 6,522 logits go through a sigmoid to give a confidence per class.
//! - **BattyBirdNET (bats):** audio at 256 kHz, cut into the same 144,000-sample
//!   windows (0.5625 s of real time), so the model hears the calls 5.33 times
//!   lower. The 1,024-value embedding goes through a small regional classifier
//!   (a "head"), then a sigmoid.
//!
//! The third is Google's Perch 2.0, as two files:
//!
//! - **Perch (birds, frogs, insects, some mammals and everyday sounds):** audio
//!   at 32 kHz, 5 s windows of 160,000 samples. The embedding model gives a
//!   1,536-value embedding and a 16 x 4 grid of them; the classifier scores the
//!   grid for 14,795 classes. Its logits go through a softmax over all classes,
//!   so a confidence is a share of 1 (the classes compete).

use serde::{Deserialize, Serialize};

pub mod catalog;

/// Samples per BirdNET window (BirdNET and BattyBirdNET). Perch's is
/// [`PERCH_WINDOW_SAMPLES`]; see [`Method::window_samples`].
pub const WINDOW_SAMPLES: usize = 144_000;
/// Length of BirdNET's embedding. Perch's is [`PERCH_EMBEDDING_LEN`]; see
/// [`Method::embedding_len`].
pub const EMBEDDING_LEN: usize = 1024;
/// Classes the backbone scores (the BirdNET V2.4 label list).
pub const BIRDNET_CLASSES: usize = 6522;
/// Samples per Perch window: 5 s at 32 kHz.
pub const PERCH_WINDOW_SAMPLES: usize = 160_000;
/// Length of Perch's embedding.
pub const PERCH_EMBEDDING_LEN: usize = 1536;
/// Classes Perch's classifier scores.
pub const PERCH_CLASSES: usize = 14_795;
/// Classes kept per window, highest confidence first.
pub const TOP_K: usize = 10;
/// Lowest sample rate BattyBirdNET accepts. Bat calls above 48 kHz are missing
/// from anything recorded below this.
pub const BATS_MIN_SOURCE_RATE: u32 = 96_000;

/// The Geomodel's week of a date: 4 per month, days 1 to 7 in the first, 8 to
/// 14 in the second, 15 to 21 in the third and the rest in the fourth (1 to 48).
pub fn week_of(month: u32, day: u32) -> u32 {
    (month.clamp(1, 12) - 1) * 4 + ((day.max(1) - 1) / 7 + 1).min(4)
}

/// What a run looks for. Results are kept per file and method; a BattyBirdNET
/// run can be scored again with another region without running the backbone
/// again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Method {
    /// Birds, with BirdNET V2.4's own 6,522 classes.
    BirdNet,
    /// Bats, with a BattyBirdNET regional head on the BirdNET embedding.
    Bats,
    /// Google's Perch 2.0: birds, frogs, insects, some mammals and everyday
    /// sounds, 14,795 classes.
    Perch,
}

impl Method {
    /// The rate the audio is resampled to before it is cut into windows.
    pub fn model_rate(self) -> u32 {
        match self {
            Method::BirdNet => 48_000,
            Method::Bats => 256_000,
            Method::Perch => 32_000,
        }
    }

    /// Samples per model window, at [`Self::model_rate`].
    pub fn window_samples(self) -> usize {
        match self {
            Method::BirdNet | Method::Bats => WINDOW_SAMPLES,
            Method::Perch => PERCH_WINDOW_SAMPLES,
        }
    }

    /// Floats per window in the embeddings a run keeps.
    pub fn embedding_len(self) -> usize {
        match self {
            Method::BirdNet | Method::Bats => EMBEDDING_LEN,
            Method::Perch => PERCH_EMBEDDING_LEN,
        }
    }

    /// Length of one window in the recording, in seconds (3 s, 0.5625 s or 5 s).
    pub fn window_secs(self) -> f64 {
        self.window_samples() as f64 / self.model_rate() as f64
    }

    /// Lowest source sample rate this method accepts, if any.
    pub fn min_source_rate(self) -> Option<u32> {
        match self {
            Method::BirdNet | Method::Perch => None,
            Method::Bats => Some(BATS_MIN_SOURCE_RATE),
        }
    }
}

/// What a model file is to the worker. The worker holds one model of each
/// [`ModelSlot`]; a model of another family (BirdNET or Perch) replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelPart {
    /// BirdNET V2.4 with an embeddings output.
    Backbone,
    /// A BattyBirdNET regional head.
    BatHead,
    /// Perch 2.0's embedding model.
    PerchEmbedding,
    /// Perch 2.0's int8 classifier.
    PerchClassifier,
}

/// Where a model sits in the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelSlot {
    /// The model that takes the audio: BirdNET's backbone or Perch's embedding
    /// model.
    Audio,
    /// The model that scores its output: a BattyBirdNET head or Perch's
    /// classifier.
    Head,
}

impl ModelPart {
    pub fn slot(self) -> ModelSlot {
        match self {
            ModelPart::Backbone | ModelPart::PerchEmbedding => ModelSlot::Audio,
            ModelPart::BatHead | ModelPart::PerchClassifier => ModelSlot::Head,
        }
    }

    /// True for Perch's files. The worker restarts when the family changes,
    /// since its memory never shrinks.
    pub fn is_perch(self) -> bool {
        matches!(self, ModelPart::PerchEmbedding | ModelPart::PerchClassifier)
    }
}

/// The BattyBirdNET regional heads. Each covers the bat species of one region.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BatRegion {
    Bavaria,
    /// Bavaria, trained on recordings with a higher sample rate.
    BavariaHigh,
    Eu,
    Scotland,
    SouthWales,
    Sweden,
    Uk,
    Usa,
    UsaEast,
    /// USA East, trained on recordings with a higher sample rate.
    UsaEastHigh,
    UsaWest,
}

impl BatRegion {
    pub const ALL: [BatRegion; 11] = [
        BatRegion::Bavaria,
        BatRegion::BavariaHigh,
        BatRegion::Eu,
        BatRegion::Scotland,
        BatRegion::SouthWales,
        BatRegion::Sweden,
        BatRegion::Uk,
        BatRegion::Usa,
        BatRegion::UsaEast,
        BatRegion::UsaEastHigh,
        BatRegion::UsaWest,
    ];

    /// The region's part of the BattyBirdNET file names, e.g. "USA-EAST" or
    /// "Bavaria-256kHz-high".
    pub fn file_stem(self) -> &'static str {
        match self {
            BatRegion::Bavaria => "Bavaria",
            BatRegion::BavariaHigh => "Bavaria-256kHz-high",
            BatRegion::Eu => "EU",
            BatRegion::Scotland => "Scotland",
            BatRegion::SouthWales => "SouthWales",
            BatRegion::Sweden => "Sweden",
            BatRegion::Uk => "UK",
            BatRegion::Usa => "USA",
            BatRegion::UsaEast => "USA-EAST",
            BatRegion::UsaEastHigh => "USA-EAST-256kHz-high",
            BatRegion::UsaWest => "USA-WEST",
        }
    }

    /// A name to show, e.g. "USA East (high sample rate)".
    pub fn label(self) -> &'static str {
        match self {
            BatRegion::Bavaria => "Bavaria",
            BatRegion::BavariaHigh => "Bavaria (high sample rate)",
            BatRegion::Eu => "Europe",
            BatRegion::Scotland => "Scotland",
            BatRegion::SouthWales => "South Wales",
            BatRegion::Sweden => "Sweden",
            BatRegion::Uk => "UK",
            BatRegion::Usa => "USA",
            BatRegion::UsaEast => "USA East",
            BatRegion::UsaEastHigh => "USA East (high sample rate)",
            BatRegion::UsaWest => "USA West",
        }
    }
}

/// One class and its confidence, 0 to 1. `class` indexes the label list of the
/// model that scored it. BirdNET and BattyBirdNET: the sigmoid of the logit.
/// Perch: the softmax over all its classes, so the confidences of one window
/// add up to at most 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Score {
    pub class: u16,
    pub confidence: f32,
}

/// Why a download, run or rescore failed. `Display` gives a sentence the UI
/// can show as is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClassifierError {
    /// The file's sample rate is below what the method needs (bats: 96 kHz).
    SampleRateTooLow { rate: u32, min: u32 },
    /// A model file the run needs has not been downloaded.
    NotDownloaded { file: String },
    /// The download failed (network or HTTP status).
    Download { file: String, message: String },
    /// The downloaded file's size or SHA-256 did not match the catalog.
    Corrupt { file: String },
    /// Saving, reading or deleting a model file failed.
    Storage { file: String, message: String },
    /// The file is no longer open.
    FileNotFound,
    /// Another run or rescore is still going.
    Busy,
    /// There are no BattyBirdNET results to score again.
    NoResults,
    /// The worker could not start, or stopped.
    Worker { message: String },
    /// tract could not load a model.
    ModelLoad { message: String },
    /// tract failed while running a window.
    Inference { message: String },
}

impl std::fmt::Display for ClassifierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClassifierError::SampleRateTooLow { rate, min } => write!(
                f,
                "Bat analysis needs a sample rate of at least {} kHz. This file is {} kHz.",
                min / 1000,
                fmt_khz(*rate)
            ),
            ClassifierError::NotDownloaded { file } => write!(f, "{file} is not downloaded."),
            ClassifierError::Download { file, message } => {
                write!(f, "Could not download {file}: {message}")
            }
            ClassifierError::Corrupt { file } => write!(
                f,
                "{file} did not match its expected checksum, so it was not saved."
            ),
            ClassifierError::Storage { file, message } => {
                write!(f, "Could not store {file}: {message}")
            }
            ClassifierError::FileNotFound => write!(f, "The recording is no longer open."),
            ClassifierError::Busy => write!(f, "Another analysis is still running."),
            ClassifierError::NoResults => write!(f, "Run bat analysis on this file first."),
            ClassifierError::Worker { message } => {
                write!(f, "The analysis worker stopped: {message}")
            }
            ClassifierError::ModelLoad { message } => {
                write!(f, "Could not load the model: {message}")
            }
            ClassifierError::Inference { message } => write!(f, "Analysis failed: {message}"),
        }
    }
}

impl std::error::Error for ClassifierError {}

fn fmt_khz(rate: u32) -> String {
    if rate % 1000 == 0 {
        format!("{}", rate / 1000)
    } else {
        format!("{:.1}", rate as f64 / 1000.0)
    }
}

/// Property names in each posted object. The serde message sits under
/// [`attach::MSG`] and the typed arrays beside it, because
/// `serde_wasm_bindgen` fails on an object that holds a typed array, even in a
/// field it would skip.
pub mod attach {
    /// The [`super::ToWorker`] or [`super::FromWorker`] message.
    pub const MSG: &str = "msg";
    /// `ArrayBuffer` of model bytes, on [`super::ToWorker::Load`].
    pub const BYTES: &str = "bytes";
    /// `Float32Array` of source-rate mono samples, on [`super::ToWorker::Chunk`].
    pub const SAMPLES: &str = "samples";
    /// `Float32Array` of the method's [`super::Method::embedding_len`] values per
    /// window, on [`super::FromWorker::Window`] (one window) and
    /// [`super::ToWorker::Rescore`] (all windows, back to back).
    pub const EMBEDDING: &str = "embedding";
}

/// App to worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "cmd")]
pub enum ToWorker {
    /// Build a model from [`attach::BYTES`] into its part's slot, replacing what
    /// was there. `key` names it, so the app can tell whether the worker
    /// already holds it. Answered by `Loaded`.
    Load { key: String, part: ModelPart },
    /// Start a run. Any run before it is dropped.
    Start {
        run: u32,
        method: Method,
        source_rate: u32,
    },
    /// The next piece of source-rate mono audio, in [`attach::SAMPLES`].
    /// Answered by `ChunkDone` after the windows it completed.
    Chunk { run: u32, seq: u32 },
    /// No more audio: score the last, zero-padded window and answer `Done`.
    Finish { run: u32 },
    /// Drop the run. Nothing is answered.
    Cancel { run: u32 },
    /// Score `windows` stored embeddings ([`attach::EMBEDDING`]) with the loaded
    /// head. Answered by `Rescored`.
    Rescore { run: u32, windows: u32 },
    /// Build the Geomodel from [`attach::BYTES`], score the place and week, and
    /// drop the model again (it stays out of the two model slots). `week` is
    /// 1 to 48 (4 per month), or 0 for the whole year: the highest score over
    /// all 48 weeks. Answered by `Occurrence`.
    Occurrence {
        req: u32,
        lat: f32,
        lon: f32,
        week: u32,
    },
}

/// Worker to app.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FromWorker {
    /// The worker's WASM has started and it takes messages from now on.
    Ready,
    /// A model was built; `ms` is the time tract took.
    Loaded { key: String, ms: f64 },
    /// One window's result. The embedding is in [`attach::EMBEDDING`].
    Window {
        run: u32,
        index: u32,
        start_s: f64,
        end_s: f64,
        top: Vec<Score>,
    },
    /// `Chunk` number `seq` has been used.
    ChunkDone { run: u32, seq: u32 },
    /// The run is complete with `windows` windows.
    Done { run: u32, windows: u32 },
    /// New scores for every stored window, in window order.
    Rescored { run: u32, top: Vec<Vec<Score>> },
    /// The Geomodel's 0 to 1 occurrence score for each of its species, in the
    /// order of its labels file.
    Occurrence { req: u32, scores: Vec<f32> },
    /// Something failed. `run` is `None` for model loading.
    Error {
        run: Option<u32>,
        error: ClassifierError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_lengths() {
        assert_eq!(Method::BirdNet.window_secs(), 3.0);
        assert_eq!(Method::Bats.window_secs(), 0.5625);
        assert_eq!(Method::Perch.window_secs(), 5.0);
    }

    #[test]
    fn weeks() {
        assert_eq!(week_of(1, 1), 1);
        assert_eq!(week_of(1, 8), 2);
        assert_eq!(week_of(1, 31), 4);
        assert_eq!(week_of(6, 1), 21);
        assert_eq!(week_of(12, 31), 48);
    }

    #[test]
    fn rate_error_text() {
        let e = ClassifierError::SampleRateTooLow {
            rate: 44_100,
            min: BATS_MIN_SOURCE_RATE,
        };
        assert_eq!(
            e.to_string(),
            "Bat analysis needs a sample rate of at least 96 kHz. This file is 44.1 kHz."
        );
    }
}

//! Types shared by the app and the BirdNET worker (`birdnet-worker`).
//!
//! The app (`src/classifier/`) and the worker exchange these messages with
//! `postMessage`. Each posted object holds the message (through
//! `serde_wasm_bindgen`) under [`attach::MSG`]; large arrays (model bytes,
//! audio, embeddings) travel beside it as transferred typed arrays, under the
//! other names in [`attach`].
//!
//! Both methods use the same model, BirdNET V2.4 with an embeddings output:
//!
//! - **BirdNET (birds):** audio at 48 kHz, 3 s windows of 144,000 samples. The
//!   model's 6,522 logits go through a sigmoid to give a confidence per class.
//! - **BattyBirdNET (bats):** audio at 256 kHz, cut into the same 144,000-sample
//!   windows (0.5625 s of real time), so the model hears the calls 5.33 times
//!   lower. The 1,024-value embedding goes through a small regional classifier
//!   (a "head"), then a sigmoid.

use serde::{Deserialize, Serialize};

pub mod catalog;

/// Samples per model window, at the method's model rate.
pub const WINDOW_SAMPLES: usize = 144_000;
/// Length of the embedding the backbone gives for each window.
pub const EMBEDDING_LEN: usize = 1024;
/// Classes the backbone scores (the BirdNET V2.4 label list).
pub const BIRDNET_CLASSES: usize = 6522;
/// Classes kept per window, highest confidence first.
pub const TOP_K: usize = 10;
/// Lowest sample rate BattyBirdNET accepts. Bat calls above 48 kHz are missing
/// from anything recorded below this.
pub const BATS_MIN_SOURCE_RATE: u32 = 96_000;

/// What a run looks for. Results are kept per file and method; a BattyBirdNET
/// run can be scored again with another region without running the backbone
/// again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Method {
    /// Birds, with BirdNET V2.4's own 6,522 classes.
    BirdNet,
    /// Bats, with a BattyBirdNET regional head on the BirdNET embedding.
    Bats,
}

impl Method {
    /// The rate the audio is resampled to before it is cut into windows.
    pub fn model_rate(self) -> u32 {
        match self {
            Method::BirdNet => 48_000,
            Method::Bats => 256_000,
        }
    }

    /// Length of one window in the recording, in seconds (3 s or 0.5625 s).
    pub fn window_secs(self) -> f64 {
        WINDOW_SAMPLES as f64 / self.model_rate() as f64
    }

    /// Lowest source sample rate this method accepts, if any.
    pub fn min_source_rate(self) -> Option<u32> {
        match self {
            Method::BirdNet => None,
            Method::Bats => Some(BATS_MIN_SOURCE_RATE),
        }
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

/// One class and its confidence (sigmoid of the logit, 0 to 1). `class` indexes
/// the label list of the model that scored it.
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
    /// `ArrayBuffer` of model bytes, on [`super::ToWorker::LoadBackbone`] and
    /// [`super::ToWorker::LoadHead`].
    pub const BYTES: &str = "bytes";
    /// `Float32Array` of source-rate mono samples, on [`super::ToWorker::Chunk`].
    pub const SAMPLES: &str = "samples";
    /// `Float32Array` of [`super::EMBEDDING_LEN`] values per window, on
    /// [`super::FromWorker::Window`] (one window) and
    /// [`super::ToWorker::Rescore`] (all windows, back to back).
    pub const EMBEDDING: &str = "embedding";
}

/// App to worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "cmd")]
pub enum ToWorker {
    /// Build the backbone from [`attach::BYTES`]. `key` names it, so the app can
    /// tell whether the worker already holds it. Answered by `Loaded`.
    LoadBackbone { key: String },
    /// Build a regional head from [`attach::BYTES`]. Answered by `Loaded`.
    LoadHead { key: String },
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

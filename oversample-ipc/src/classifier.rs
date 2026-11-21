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
//!
//! The fourth is BirdNET+ V3.0 (the pruned float16 file from Zenodo, patched
//! to float32 in the worker; `research/birdnet-v3-bench`):
//!
//! - **BirdNET+ V3.0 (birds, insects, frogs and mammals):** audio at 32 kHz,
//!   3 s windows of 96,000 samples. The model gives a 1,280-value embedding and
//!   11,725 confidences that have already been through a sigmoid. Windows that
//!   are digital silence are not scored: the model gives them false birds.

use serde::{Deserialize, Serialize};

mod bat_merge;
pub mod boxes;
pub mod catalog;

pub use bat_merge::{merge_bat_labels, merge_bat_logits, MergedBatLabels};

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
/// Samples per BirdNET+ V3.0 window: 3 s at 32 kHz.
pub const V3_WINDOW_SAMPLES: usize = 96_000;
/// Length of BirdNET+ V3.0's embedding.
pub const V3_EMBEDDING_LEN: usize = 1280;
/// Classes BirdNET+ V3.0 (the pruned file) scores.
pub const V3_CLASSES: usize = 11_725;
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
    /// BirdNET+ V3.0: birds, insects, frogs, mammals and some other sounds,
    /// 11,725 classes.
    BirdNetV3,
}

impl Method {
    /// The rate the audio is resampled to before it is cut into windows.
    pub fn model_rate(self) -> u32 {
        match self {
            Method::BirdNet => 48_000,
            Method::Bats => 256_000,
            Method::Perch | Method::BirdNetV3 => 32_000,
        }
    }

    /// Samples per model window, at [`Self::model_rate`].
    pub fn window_samples(self) -> usize {
        match self {
            Method::BirdNet | Method::Bats => WINDOW_SAMPLES,
            Method::Perch => PERCH_WINDOW_SAMPLES,
            Method::BirdNetV3 => V3_WINDOW_SAMPLES,
        }
    }

    /// Floats per window in the embeddings a run keeps.
    pub fn embedding_len(self) -> usize {
        match self {
            Method::BirdNet | Method::Bats => EMBEDDING_LEN,
            Method::Perch => PERCH_EMBEDDING_LEN,
            Method::BirdNetV3 => V3_EMBEDDING_LEN,
        }
    }

    /// Length of one window in the recording, in seconds (3 s, 0.5625 s or 5 s).
    pub fn window_secs(self) -> f64 {
        self.window_samples() as f64 / self.model_rate() as f64
    }

    /// Lowest source sample rate this method accepts, if any.
    pub fn min_source_rate(self) -> Option<u32> {
        match self {
            Method::BirdNet | Method::Perch | Method::BirdNetV3 => None,
            Method::Bats => Some(BATS_MIN_SOURCE_RATE),
        }
    }

    /// `secs` as a step between window starts, in model samples, when it is
    /// shorter than a window and a whole number of steps fills one window
    /// (1 s and 0.5 s for BirdNET, BirdNET+ V3.0 and Perch). `None` for
    /// anything else, which means the whole window.
    pub fn step_samples(self, secs: f64) -> Option<u32> {
        if !secs.is_finite() || secs <= 0.0 {
            return None;
        }
        let step = (secs * self.model_rate() as f64).round();
        let win = self.window_samples() as f64;
        (step >= 1.0
            && step < win
            && (step - secs * self.model_rate() as f64).abs() < 1e-6
            && (win as u64) % (step as u64) == 0)
            .then_some(step as u32)
    }
}

/// One pass of a run over the whole recording: the windows starting at
/// `k * window + offset` for every whole `k` and every one of `offsets`
/// (model samples, each less than a window), front to back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pass {
    pub offsets: Vec<u32>,
    /// The gap between window starts once this pass is done, in model
    /// samples.
    pub spacing: u32,
}

/// The passes of a run with a step of `step` model samples between window
/// starts, for windows of `window` samples. The first pass is always the
/// windows end to end (offset 0, the same windows a run without a step
/// gives), so the whole recording has results first. Each later pass covers
/// the whole recording again and halves every gap between window starts,
/// while half a gap is a whole number of steps; one last pass then takes
/// every start left. Every start on the step's grid is in exactly one pass.
///
/// Window 3 s, step 1 s: [0], [1 s, 2 s]. Step 0.5 s: [0], [1.5 s], [0.5 s,
/// 1 s, 2 s, 2.5 s]. Window 5 s, step 1 s: [0], [1 s to 4 s]. Step 0.5 s:
/// [0], [2.5 s], the other eight. A step that does not fit a whole number of
/// times in the window, or is not shorter than it, gives the first pass only.
pub fn refine_passes(window: u32, step: u32) -> Vec<Pass> {
    let mut passes = vec![Pass {
        offsets: vec![0],
        spacing: window,
    }];
    if step == 0 || step >= window || window % step != 0 {
        return passes;
    }
    let n = window / step;
    // The gap between done starts, in steps; every gap is the same.
    let mut gap = n;
    while gap > 1 {
        let (offsets, next): (Vec<u32>, u32) = if gap % 2 == 0 {
            let half = gap / 2;
            (
                (0..n / gap).map(|j| (j * gap + half) * step).collect(),
                half,
            )
        } else {
            (
                (0..n).filter(|j| j % gap != 0).map(|j| j * step).collect(),
                1,
            )
        };
        gap = next;
        passes.push(Pass {
            offsets,
            spacing: gap * step,
        });
    }
    passes
}

/// Audio sent with a window cut out of a recording ([`live_cut`]) before and
/// after the window, at least, in seconds, so the resampler gives the window
/// the same samples a run over the whole recording gives it.
pub const CUT_MARGIN_S: f64 = 0.25;

/// One window of a recording, cut out to be scored on its own
/// (`ToWorker::Live` with `at`). See [`live_cut`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveCut {
    /// First sample of the recording to send.
    pub from: u64,
    /// Samples to send, from `from` on: the window and the margins each side
    /// of it.
    pub len: u64,
    /// The window starts this many model-rate samples into the resampled
    /// audio sent.
    pub skip: u32,
    /// The window's start and end in the recording, in seconds. The start is
    /// a model-rate sample time, as a run's window starts are.
    pub start_s: f64,
    pub end_s: f64,
}

fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The cut that scores `method`'s window starting at `start_s` (at the
/// nearest model-rate sample) in a recording of `total` samples at
/// `source_rate`. `None` when the window is not wholly inside the recording:
/// the only window of a run with zero padding is its last one end to end,
/// which a cut never gives.
///
/// The resampler's output lines up with a run's over the whole recording
/// when the cut starts on a multiple of `source_rate / gcd(source_rate,
/// model_rate)` samples (3.3 ms at 44.1 kHz for BirdNET, never more than
/// 1 s), so the cut starts on the last such sample at least
/// [`CUT_MARGIN_S`] before the window. Its window then has the samples the
/// run gives a window with the same start, to float rounding.
pub fn live_cut(method: Method, source_rate: u32, start_s: f64, total: u64) -> Option<LiveCut> {
    if source_rate == 0 || !start_s.is_finite() || start_s < 0.0 {
        return None;
    }
    let (inn, out) = (source_rate as u128, method.model_rate() as u128);
    let g = gcd_u64(inn as u64, out as u64) as u128;
    let (qa, qm) = (inn / g, out / g);
    let win = method.window_samples() as u128;
    let total = total as u128;
    // The window in model samples, and in source samples (rounded outwards).
    let s = (start_s * out as f64).round() as u128;
    if (s + win) * inn > total * out {
        return None;
    }
    let first = s * inn / out;
    let last = ((s + win) * inn).div_ceil(out);
    let margin = (CUT_MARGIN_S * inn as f64).ceil() as u128;
    // The last grid point at or before the margin: the run's model sample
    // `k * qm` lines up with the cut's first sample.
    let k = first.saturating_sub(margin) / qa;
    let from = k * qa;
    let to = (last + margin).min(total);
    let start = s as f64 / out as f64;
    Some(LiveCut {
        from: from as u64,
        len: (to - from) as u64,
        skip: (s - k * qm) as u32,
        start_s: start,
        end_s: start + method.window_secs(),
    })
}

/// What a model file is to the worker. The worker holds one model of each
/// [`ModelSlot`]; a model of another [`ModelFamily`] replaces it.
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
    /// BirdNET+ V3.0 as published (float16 weights), which the worker
    /// converts to float32 as it loads it.
    BirdNetV3,
    /// The learned denoiser for bat and bird recordings. It is not a
    /// classifier and runs in a worker of its own (see `src/denoise.rs` in the
    /// app).
    Denoiser,
    /// BirdBox's Just-Bird network, which draws a box round each bird sound.
    /// Either BirdBox's all-in-one demo graph, which the worker cuts the
    /// network out of, or the release file; float16 weights become float32
    /// as it loads.
    JustBird,
}

/// Where a model sits in the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelSlot {
    /// The model that takes the audio: BirdNET's backbone, BirdNET+ V3.0 or
    /// Perch's embedding model.
    Audio,
    /// The model that scores its output: a BattyBirdNET head or Perch's
    /// classifier.
    Head,
    /// The denoiser.
    Denoise,
    /// The bird box detector (BirdBox's Just-Bird), which runs beside
    /// BirdNET+ V3.0 on the same audio.
    Boxes,
}

impl ModelPart {
    pub fn slot(self) -> ModelSlot {
        match self {
            ModelPart::Backbone | ModelPart::PerchEmbedding | ModelPart::BirdNetV3 => {
                ModelSlot::Audio
            }
            ModelPart::BatHead | ModelPart::PerchClassifier => ModelSlot::Head,
            ModelPart::Denoiser => ModelSlot::Denoise,
            ModelPart::JustBird => ModelSlot::Boxes,
        }
    }

    /// The family the model belongs to; `None` for the denoiser, which runs
    /// in a worker of its own.
    pub fn family(self) -> Option<ModelFamily> {
        match self {
            ModelPart::Backbone | ModelPart::BatHead => Some(ModelFamily::BirdNet),
            ModelPart::PerchEmbedding | ModelPart::PerchClassifier => Some(ModelFamily::Perch),
            // Its boxes are named by BirdNET+ V3.0, so it shares that worker.
            ModelPart::BirdNetV3 | ModelPart::JustBird => Some(ModelFamily::BirdNetV3),
            ModelPart::Denoiser => None,
        }
    }
}

/// Models that share the classifier worker without restarting it. The worker
/// restarts when the family changes, since its memory never shrinks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelFamily {
    /// BirdNET V2.4 and the BattyBirdNET heads on its embedding.
    BirdNet,
    Perch,
    BirdNetV3,
}

impl Method {
    /// The family of the models a run of this method uses.
    pub fn family(self) -> ModelFamily {
        match self {
            Method::BirdNet | Method::Bats => ModelFamily::BirdNet,
            Method::Perch => ModelFamily::Perch,
            Method::BirdNetV3 => ModelFamily::BirdNetV3,
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

/// The regional heads a BattyBirdNET run scores with: one region's, or all 11
/// merged into one list of species.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BatScope {
    /// Every head. A species gets the highest confidence any head gives it
    /// (see [`merge_bat_logits`]).
    All,
    Region(BatRegion),
}

impl BatScope {
    /// A name to show, e.g. "All regions" or "USA East".
    pub fn label(self) -> &'static str {
        match self {
            BatScope::All => "All regions",
            BatScope::Region(r) => r.label(),
        }
    }

    /// A short name for tags and tests: "all", or the region's
    /// [`BatRegion::file_stem`].
    pub fn stem(self) -> &'static str {
        match self {
            BatScope::All => "all",
            BatScope::Region(r) => r.file_stem(),
        }
    }

    /// The regions whose heads it uses.
    pub fn regions(self) -> Vec<BatRegion> {
        match self {
            BatScope::All => BatRegion::ALL.to_vec(),
            BatScope::Region(r) => vec![r],
        }
    }
}

/// One head of [`ToWorker::LoadBatHeads`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatHeadPart {
    /// Bytes of its model file in [`attach::BYTES`], where the heads' files
    /// sit back to back in the order of the message.
    pub len: u32,
    /// For each of the head's outputs, its class in the merged list.
    pub classes: Vec<u16>,
}

/// One class and its confidence, 0 to 1. `class` indexes the label list of the
/// model that scored it. BirdNET and BattyBirdNET: the sigmoid of the logit.
/// Perch: the softmax over all its classes, so the confidences of one window
/// add up to at most 1. BirdNET+ V3.0: the model's own output, already a
/// sigmoid.
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
    /// `Float32Array` of source-rate mono samples, on [`super::ToWorker::Chunk`]
    /// and [`super::ToWorker::DenoiseStart`] (the whole recording), and the
    /// denoised recording on [`super::FromWorker::Denoised`]. With
    /// [`super::ToWorker::DenoiseStartChannels`] the recording and its
    /// denoised copy have their channels interleaved. Also a piece of a
    /// recording on [`super::ToWorker::DenoiseMeasure`] and
    /// [`super::ToWorker::DenoisePiece`], and denoised samples on
    /// [`super::FromWorker::DenoisedPiece`].
    pub const SAMPLES: &str = "samples";
    /// `Float32Array` of the method's [`super::Method::embedding_len`] values per
    /// window, on [`super::FromWorker::Window`] (one window; zeros for a
    /// BirdNET+ V3.0 window of digital silence, which is not run) and
    /// [`super::ToWorker::Rescore`] (all windows, back to back).
    pub const EMBEDDING: &str = "embedding";
}

/// What the denoiser does to its network's gain `g = 2 * sigmoid(logit) - 1`
/// before the gain goes on the recording. The steps are in
/// `birdnet-worker/src/denoise/chain.rs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DenoiseGain {
    /// `max(g, 0.1)`: the gain as the network gives it, at least -20 dB.
    #[default]
    Plain,
    /// `g` smoothed by the median of 5 bins x `smooth_frames` frames (a field
    /// of the start message, 9 when it is left out), capped at the largest
    /// `g` of the cell's 3 x 3 neighbours, at least -20 dB. It leaves fewer
    /// patches of noise.
    Smoothed,
    /// The smoothed gain squared, falling by at most 0.16 dB a frame, at
    /// least -40 dB. It takes out more of the noise.
    Heavy,
}

/// Frames in the smoothed and heavy chains' median when a start message does
/// not say: 9 frames of 128 samples. The app asks for 3 at sample rates over
/// 96 kHz, where 9 frames are longer than a short bat call.
pub const DEFAULT_SMOOTH_FRAMES: u32 = 9;

fn default_smooth_frames() -> u32 {
    DEFAULT_SMOOTH_FRAMES
}

/// App to worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "cmd")]
pub enum ToWorker {
    /// Build a model from [`attach::BYTES`] into its part's slot, replacing what
    /// was there. `key` names it, so the app can tell whether the worker
    /// already holds it. Answered by `Loaded`.
    Load { key: String, part: ModelPart },
    /// Build several BattyBirdNET heads from [`attach::BYTES`] (their files
    /// back to back) into the head slot, replacing what was there. Their
    /// scores are merged with [`merge_bat_logits`] into `classes` classes, the
    /// first `species` of them species. Answered by `Loaded`.
    LoadBatHeads {
        key: String,
        heads: Vec<BatHeadPart>,
        species: u16,
        classes: u16,
    },
    /// Start a run, or one pass of a run (see [`refine_passes`]). Any run
    /// before it is dropped. `offsets` picks the windows: those starting at
    /// `k * window + offset` model samples, for each offset (each less than a
    /// window). Empty or `[0]` is the windows end to end, with the last,
    /// zero-padded window kept when it holds at least a third of a window.
    /// Any other offsets take only the windows that fit inside the audio.
    ///
    /// With `boxes` (BirdNET+ V3.0 and the windows end to end only), the
    /// bird box detector in the boxes slot also runs on the audio, and
    /// answers `Boxes` for each of its windows and `BoxesDone` before `Done`.
    Start {
        run: u32,
        method: Method,
        source_rate: u32,
        #[serde(default)]
        offsets: Vec<u32>,
        #[serde(default)]
        boxes: bool,
    },
    /// The next piece of source-rate mono audio, in [`attach::SAMPLES`].
    /// Answered by `ChunkDone` after the windows it completed.
    Chunk { run: u32, seq: u32 },
    /// No more audio: score the last windows (for the windows end to end, the
    /// last, zero-padded one) and answer `Done`.
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
    /// Take a whole recording ([`attach::SAMPLES`], mono, at its own rate) for
    /// the loaded denoiser. Any recording before it is dropped. Answered by
    /// `DenoiseProgress` with `done` 0. `gain` picks what is done to the
    /// network's gain. `smooth_frames` is the number of frames in the
    /// smoothed and heavy chains' median: an odd number from 3 to 31, 9 when
    /// it is left out. The plain chain does not use it.
    DenoiseStart {
        run: u32,
        #[serde(default)]
        gain: DenoiseGain,
        #[serde(default = "default_smooth_frames")]
        smooth_frames: u32,
    },
    /// `DenoiseStart` for a recording of `channels` channels, its samples
    /// interleaved in [`attach::SAMPLES`]. Every channel gets the same gains,
    /// and `Denoised` carries the channels interleaved again.
    DenoiseStartChannels {
        run: u32,
        channels: u32,
        #[serde(default)]
        gain: DenoiseGain,
        #[serde(default = "default_smooth_frames")]
        smooth_frames: u32,
    },
    /// Denoise the next block. Answered by `DenoiseProgress`, or by `Denoised`
    /// after the last block. One block per message, so a `Cancel` between two
    /// of them stops the work.
    DenoiseStep { run: u32 },
    /// Begin a recording that arrives in pieces and is never held whole:
    /// `frames` samples in each of `channels` channels. Any recording before
    /// it is dropped. Answered by `DenoiseProgress` with `done` 0. The
    /// recording then comes twice, start to end both times: first as
    /// `DenoiseMeasure` pieces, then as `DenoisePiece` pieces. The denoised
    /// samples are the same as `DenoiseStart` gives for the whole recording.
    /// `frames` is a float because a 64-bit integer does not cross to
    /// JavaScript as a number. `gain` and `smooth_frames` as for
    /// `DenoiseStart`.
    DenoiseStreamStart {
        run: u32,
        channels: u32,
        frames: f64,
        #[serde(default)]
        gain: DenoiseGain,
        #[serde(default = "default_smooth_frames")]
        smooth_frames: u32,
    },
    /// The recording's next samples ([`attach::SAMPLES`], channels
    /// interleaved), for the noise scale, which is measured over the whole
    /// recording before any of it is denoised. Answered by `DenoiseMeasured`.
    DenoiseMeasure { run: u32 },
    /// The recording's next samples ([`attach::SAMPLES`], channels
    /// interleaved), to denoise. Answered by `DenoisedPiece`.
    DenoisePiece { run: u32 },
    /// Score one window of the most recent audio, for the live view.
    /// [`attach::SAMPLES`] holds mono audio at `source_rate` that ends now; the
    /// last window of it is used (3 s for BirdNET and BirdNET+ V3.0, 5 s for
    /// Perch), padded with zeros at the front when shorter. The model in the
    /// audio slot picks which: the BirdNET backbone, BirdNET+ V3.0, or Perch's
    /// embedding model, which also needs Perch's classifier in the head slot.
    /// Answered by `Live`. It leaves any run alone.
    ///
    /// With `at`, the audio is a cut out of a recording ([`live_cut`]) and
    /// the window is the one starting `at` model-rate samples into it once
    /// resampled, padded with zeros at the end if the audio stops short.
    Live {
        req: u32,
        source_rate: u32,
        #[serde(default)]
        at: Option<u32>,
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
    /// One window's result. The embedding is in [`attach::EMBEDDING`]. `top`
    /// is empty for a window that was not scored (BirdNET+ V3.0 on digital
    /// silence).
    Window {
        run: u32,
        index: u32,
        start_s: f64,
        end_s: f64,
        top: Vec<Score>,
    },
    /// `Chunk` number `seq` has been used.
    ChunkDone { run: u32, seq: u32 },
    /// The bird box detector's boxes in its window `index` (3 s from
    /// `index * 1.5` s), at [`boxes::KEEP_CONF`] or above, after NMS. Sent
    /// for every window, with no boxes too, so the app can tell how far it
    /// has got.
    Boxes {
        run: u32,
        index: u32,
        boxes: Vec<boxes::RawBox>,
    },
    /// The bird box detector has done its last window: `windows` windows
    /// over `samples` samples of 32 kHz audio (a float, as a 64-bit integer
    /// does not cross to JavaScript as a number).
    BoxesDone { run: u32, windows: u32, samples: f64 },
    /// The run is complete with `windows` windows.
    Done { run: u32, windows: u32 },
    /// New scores for every stored window, in window order.
    Rescored { run: u32, top: Vec<Vec<Score>> },
    /// The Geomodel's 0 to 1 occurrence score for each of its species, in the
    /// order of its labels file.
    Occurrence { req: u32, scores: Vec<f32> },
    /// `done` of the recording's `total` blocks are denoised.
    DenoiseProgress { run: u32, done: u32, total: u32 },
    /// The denoised recording is in [`attach::SAMPLES`], the same length as
    /// what was sent.
    Denoised { run: u32 },
    /// A `DenoiseMeasure` piece has been used.
    DenoiseMeasured { run: u32 },
    /// A `DenoisePiece` has been used. The denoised samples that are
    /// finished are in [`attach::SAMPLES`], channels interleaved; they follow
    /// on from the last answer's, run about one block behind the pieces, and
    /// can be none. `done` of the recording's `total` blocks are denoised,
    /// and when the two are equal the recording is complete.
    DenoisedPiece { run: u32, done: u32, total: u32 },
    /// The answer to `Live`. [`attach::EMBEDDING`] holds the embedding
    /// (1,024 values for BirdNET, 1,536 for Perch, 1,280 for BirdNET+ V3.0)
    /// followed by the confidence (0 to 1) of every class in label order: a
    /// sigmoid for BirdNET and BirdNET+ V3.0, Perch's softmax share over all
    /// its classes, as in a run's results. `ms` is the
    /// time the resampling and the model took. `top` is the window's
    /// [`TOP_K`] classes worked out exactly as a run's `Window` gives them.
    Live {
        req: u32,
        ms: f64,
        #[serde(default)]
        top: Vec<Score>,
    },
    /// Something failed. `run` is `None` for model loading.
    Error {
        run: Option<u32>,
        error: ClassifierError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a cut lies, at rates whose grid of lined-up starts is short
    /// (44.1 kHz: 147 samples) and long (192,001 Hz: 1 s).
    #[test]
    fn cuts_hold_the_window_and_start_on_the_grid() {
        for (method, rate) in [
            (Method::BirdNet, 44_100u32),
            (Method::BirdNet, 48_000),
            (Method::BirdNet, 192_001),
            (Method::Perch, 22_050),
        ] {
            let total = rate as u64 * 60;
            let g = gcd_u64(rate as u64, method.model_rate() as u64);
            let qa = rate as u64 / g;
            for start in [0.0, 0.1, 7.25, 31.123_45, 60.0 - method.window_secs()] {
                let c = live_cut(method, rate, start, total).unwrap();
                assert_eq!(c.from % qa, 0, "{rate} Hz from {start}");
                // The start is the nearest model sample.
                assert!((c.start_s - start).abs() <= 0.5 / method.model_rate() as f64);
                assert_eq!(c.end_s, c.start_s + method.window_secs());
                // The audio sent holds the window, with the margin each side
                // where the recording has it.
                let win_from = (c.start_s * rate as f64).floor() as u64;
                let win_to = (c.end_s * rate as f64).ceil() as u64;
                let margin = (CUT_MARGIN_S * rate as f64) as u64;
                assert!(c.from <= win_from.saturating_sub(margin));
                assert!(win_from - c.from <= margin + qa);
                assert!(c.from + c.len >= (win_to + margin).min(total));
                assert!(c.from + c.len <= total);
                // `skip` model samples in is the window's start.
                let skip_s = c.skip as f64 / method.model_rate() as f64;
                assert!((c.from as f64 / rate as f64 + skip_s - c.start_s).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn cuts_are_whole_windows_inside_the_recording() {
        let total = 44_100 * 10;
        assert!(live_cut(Method::BirdNet, 44_100, 7.0, total).is_some());
        assert!(live_cut(Method::BirdNet, 44_100, 7.01, total).is_none());
        assert!(live_cut(Method::BirdNet, 44_100, -0.5, total).is_none());
        assert!(live_cut(Method::BirdNet, 44_100, f64::NAN, total).is_none());
        assert!(live_cut(Method::BirdNet, 0, 1.0, total).is_none());
        // Shorter than a window: no cut at all.
        assert!(live_cut(Method::Perch, 44_100, 0.0, 44_100 * 4).is_none());
        // Near the end the margin after the window is cut short.
        let c = live_cut(Method::BirdNet, 44_100, 7.0, total).unwrap();
        assert_eq!(c.from + c.len, total);
    }

    #[test]
    fn window_lengths() {
        assert_eq!(Method::BirdNet.window_secs(), 3.0);
        assert_eq!(Method::Bats.window_secs(), 0.5625);
        assert_eq!(Method::Perch.window_secs(), 5.0);
        assert_eq!(Method::BirdNetV3.window_secs(), 3.0);
    }

    fn secs(method: Method, passes: &[Pass]) -> Vec<(Vec<f64>, f64)> {
        let rate = method.model_rate() as f64;
        passes
            .iter()
            .map(|p| {
                (
                    p.offsets.iter().map(|&o| o as f64 / rate).collect(),
                    p.spacing as f64 / rate,
                )
            })
            .collect()
    }

    fn plan(method: Method, step: f64) -> Vec<(Vec<f64>, f64)> {
        let win = method.window_samples() as u32;
        let step = method.step_samples(step).unwrap_or(win);
        secs(method, &refine_passes(win, step))
    }

    #[test]
    fn passes_halve_while_they_can() {
        assert_eq!(
            plan(Method::BirdNet, 1.0),
            vec![(vec![0.0], 3.0), (vec![1.0, 2.0], 1.0)]
        );
        assert_eq!(
            plan(Method::BirdNet, 0.5),
            vec![
                (vec![0.0], 3.0),
                (vec![1.5], 1.5),
                (vec![0.5, 1.0, 2.0, 2.5], 0.5)
            ]
        );
        assert_eq!(plan(Method::BirdNetV3, 0.5), plan(Method::BirdNet, 0.5));
        assert_eq!(
            plan(Method::Perch, 1.0),
            vec![(vec![0.0], 5.0), (vec![1.0, 2.0, 3.0, 4.0], 1.0)]
        );
        assert_eq!(
            plan(Method::Perch, 0.5),
            vec![
                (vec![0.0], 5.0),
                (vec![2.5], 2.5),
                (vec![0.5, 1.0, 1.5, 2.0, 3.0, 3.5, 4.0, 4.5], 0.5)
            ]
        );
        // The whole window, or a step that does not fit: one pass.
        assert_eq!(plan(Method::BirdNet, 3.0), vec![(vec![0.0], 3.0)]);
        assert_eq!(plan(Method::BirdNet, 0.7), vec![(vec![0.0], 3.0)]);
        assert_eq!(refine_passes(8, 1).len(), 4);
    }

    #[test]
    fn every_start_once() {
        for (win, step) in [
            (6u32, 1u32),
            (6, 2),
            (6, 3),
            (10, 1),
            (10, 2),
            (8, 1),
            (12, 1),
        ] {
            let mut all: Vec<u32> = refine_passes(win, step)
                .into_iter()
                .flat_map(|p| p.offsets)
                .collect();
            all.sort_unstable();
            assert_eq!(all, (0..win / step).map(|j| j * step).collect::<Vec<_>>());
        }
    }

    #[test]
    fn steps_that_fit() {
        assert_eq!(Method::BirdNet.step_samples(1.0), Some(48_000));
        assert_eq!(Method::BirdNet.step_samples(0.5), Some(24_000));
        assert_eq!(Method::Perch.step_samples(0.5), Some(16_000));
        assert_eq!(Method::BirdNetV3.step_samples(1.0), Some(32_000));
        assert_eq!(Method::BirdNet.step_samples(3.0), None);
        assert_eq!(Method::BirdNet.step_samples(5.0), None);
        assert_eq!(Method::BirdNet.step_samples(0.7), None);
        assert_eq!(Method::BirdNet.step_samples(0.0), None);
        assert_eq!(Method::BirdNet.step_samples(f64::NAN), None);
    }

    #[test]
    fn a_start_message_without_offsets_is_the_whole_window() {
        let msg: ToWorker =
            serde_json::from_str(r#"{"cmd":"Start","run":1,"method":"Perch","source_rate":48000}"#)
                .unwrap();
        assert!(
            matches!(msg, ToWorker::Start { offsets, boxes: false, .. } if offsets.is_empty())
        );
    }

    #[test]
    fn families() {
        assert_eq!(ModelPart::BatHead.family(), Method::Bats.family().into());
        assert_eq!(ModelPart::Backbone.family(), Some(ModelFamily::BirdNet));
        assert_eq!(ModelPart::BirdNetV3.family(), Some(ModelFamily::BirdNetV3));
        assert_eq!(
            ModelPart::PerchClassifier.family(),
            Some(ModelFamily::Perch)
        );
        assert_eq!(ModelPart::Denoiser.family(), None);
        assert_eq!(ModelPart::BirdNetV3.slot(), ModelSlot::Audio);
        // The box detector sits beside BirdNET+ V3.0, in a slot of its own.
        assert_eq!(ModelPart::JustBird.family(), Some(ModelFamily::BirdNetV3));
        assert_eq!(ModelPart::JustBird.slot(), ModelSlot::Boxes);
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
    fn a_start_message_without_smooth_frames_gets_9() {
        let msg: ToWorker =
            serde_json::from_str(r#"{"cmd":"DenoiseStart","run":1,"gain":"Smoothed"}"#).unwrap();
        assert!(matches!(
            msg,
            ToWorker::DenoiseStart {
                gain: DenoiseGain::Smoothed,
                smooth_frames: 9,
                ..
            }
        ));
        let msg: ToWorker = serde_json::from_str(
            r#"{"cmd":"DenoiseStreamStart","run":1,"channels":2,"frames":10.0,"smooth_frames":3}"#,
        )
        .unwrap();
        assert!(matches!(
            msg,
            ToWorker::DenoiseStreamStart {
                gain: DenoiseGain::Plain,
                smooth_frames: 3,
                ..
            }
        ));
        let msg: ToWorker =
            serde_json::from_str(r#"{"cmd":"DenoiseStartChannels","run":1,"channels":2}"#).unwrap();
        assert!(matches!(
            msg,
            ToWorker::DenoiseStartChannels {
                smooth_frames: DEFAULT_SMOOTH_FRAMES,
                ..
            }
        ));
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

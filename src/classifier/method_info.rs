//! The facts about each classifier method that the app reads in more than
//! one place: its name and description in the Call detection tab, which
//! parts of the tab it has, the band its model looks at, and its default
//! minimum confidence. `MethodFacts::info` builds the table with an
//! exhaustive `match`, so a new `Method` does not compile until its facts
//! are given here.
//!
//! The model's own numbers (sample rate, window, embedding length) are on
//! `Method` in `oversample-ipc`, since the worker uses them too. Facts read
//! in one place only stay with the code that reads them. Bird boxes are not
//! here: they are built on BirdNET+ V3.0's own windows and labels
//! (`classifier::boxes`), so the code asks for `Method::BirdNetV3` by name.

use super::species_map::{MapSpec, BIRDNET_MAP, BIRDNET_V3_MAP, PERCH_MAP};
use super::Method;
use crate::live_embedding::{LiveModel, BIRDNET, BIRDNET_V3, PERCH};

/// What differs between the classifier methods, for the app.
pub struct MethodInfo {
    /// The name the method drop-down shows.
    pub name: &'static str,
    /// The line under the name in the method drop-down.
    pub description: &'static str,
    /// The first words of the attribution line at the foot of the panel.
    pub byline: &'static str,
    /// The live view's model: the panel shows readings at the play position,
    /// and readings of a file that plays go into its results. `None` for a
    /// method without a live view.
    pub live_model: Option<&'static LiveModel>,
    /// The species map, if the method has one.
    pub species_map: Option<&'static MapSpec>,
    /// The principal components of its embedding can be rows above the
    /// spectrogram (`components::score_strips::components_for`). Needs a
    /// live model, whose components file is used.
    pub component_rows: bool,
    /// "Mark birds unlikely here" applies: the BirdNET+ Geomodel's scores
    /// mark birds unlikely at the recording's place and month.
    pub marks_unlikely: bool,
    /// The Step setting applies: an Analyse can start windows less than a
    /// window apart. Without it every run steps a whole window.
    pub step_setting: bool,
    /// The in-built call detector's boxes stay on the spectrogram while this
    /// method is picked, each under the species label of its window.
    pub call_boxes: bool,
    /// The band the quiet check measures (`classifier::quiet::model_band`),
    /// in Hz: what the model's own spectrograms cover, less the rumble under
    /// 250 Hz (a choice made here, not the models'). Read from the mel
    /// filter banks inside the model files (the 2-D constant matrices of the
    /// ONNX graphs).
    pub band: (f32, f32),
    /// The minimum confidence the tab starts with. BirdNET V2.4 and
    /// BattyBirdNET share one setting (`classifier::min_confidence`), so
    /// theirs are the same.
    pub default_min_confidence: f32,
}

/// `method.info()`. `Method` is defined in `oversample-ipc`, so the table is
/// reached through this trait.
pub trait MethodFacts {
    fn info(self) -> &'static MethodInfo;
}

impl MethodFacts for Method {
    fn info(self) -> &'static MethodInfo {
        match self {
            Method::BirdNet => &BIRDNET_INFO,
            Method::BirdNetV3 => &BIRDNET_V3_INFO,
            Method::Perch => &PERCH_INFO,
            Method::Bats => &BATS_INFO,
        }
    }
}

/// BattyBirdNET gives BirdNET audio at 256 kHz as if it were 48 kHz, so its
/// band is BirdNET's times 256 / 48.
const BATS_SCALE: f32 = 256_000.0 / 48_000.0;

static BIRDNET_INFO: MethodInfo = MethodInfo {
    name: "BirdNET V2.4",
    description: "Bird species, plus some other sounds such as dog and engine",
    byline: "Powered by BirdNET V2.4.",
    live_model: Some(&BIRDNET),
    species_map: Some(&BIRDNET_MAP),
    component_rows: true,
    marks_unlikely: true,
    step_setting: true,
    call_boxes: false,
    // `birdnet-v2.4-embeddings-fp32.onnx` (also the backbone under
    // BattyBirdNET's heads) makes two mel spectrograms of its 48 kHz input:
    // 96 bands over 23 to 2,977 Hz (2048-point FFT) and 96 bands over 516 to
    // 14,953 Hz (1024-point FFT). BirdNET-Analyzer 1.5.1's `config.py`
    // agrees: `SIG_FMIN = 0`, `SIG_FMAX = 15000`, "model specific".
    band: (250.0, 15_000.0),
    default_min_confidence: 0.05,
};

static BIRDNET_V3_INFO: MethodInfo = MethodInfo {
    name: "BirdNET+ V3.0",
    description: "11,725 birds, insects, frogs, mammals and other sounds. Slower than V2.4.",
    byline: "Powered by BirdNET+ V3.0.",
    live_model: Some(&BIRDNET_V3),
    species_map: Some(&BIRDNET_V3_MAP),
    component_rows: true,
    marks_unlikely: true,
    step_setting: true,
    call_boxes: false,
    // Its mel bands also stop near 15 kHz (its docs/model.md).
    band: (250.0, 15_000.0),
    // The BirdNET team's suggestion: the pruned model's scores run up to
    // 0.16 lower than the full model's (`research/birdnet-v3-bench/README.md`).
    default_min_confidence: 0.25,
};

static PERCH_INFO: MethodInfo = MethodInfo {
    name: "Perch 2.0",
    description: "Birds, frogs and insects, plus sounds such as thunder and traffic",
    byline: "Perch 2.0 by Google DeepMind.",
    live_model: Some(&PERCH),
    species_map: Some(&PERCH_MAP),
    // Not offered yet.
    component_rows: false,
    marks_unlikely: true,
    step_setting: true,
    call_boxes: false,
    // `perch_v2_embed.tract.onnx` makes one mel spectrogram of its 32 kHz
    // input: 128 bands over 62 to 15,969 Hz (1024-point FFT), as
    // `research/perch.md` says (60 Hz to 16 kHz).
    band: (250.0, 16_000.0),
    // Its confidences are softmax shares and run lower. At 0.30, 5 of 348
    // windows of rain, thunder, traffic, wind and voices still show an
    // animal, and 61 of 81 correct windows stay (`research/perch.md`,
    // "Sound events").
    default_min_confidence: 0.3,
};

static BATS_INFO: MethodInfo = MethodInfo {
    name: "BattyBirdNET",
    description: "Bat species of Europe and the USA",
    byline: "BattyBirdNET by rdz-oss, powered by BirdNET.",
    live_model: None,
    // Its regional classifiers give no usable map
    // (research/species-map/README.md).
    species_map: None,
    // Its embeddings have V2.4's length but come from 256 kHz audio.
    component_rows: false,
    marks_unlikely: false,
    step_setting: false,
    call_boxes: true,
    // BirdNET's band times 256 / 48: up to 80 kHz. Its head files have no
    // spectrogram of their own.
    band: (250.0 * BATS_SCALE, 15_000.0 * BATS_SCALE),
    default_min_confidence: 0.05,
};

/// Every method, in the order `Method` lists them.
#[cfg(test)]
const ALL: [Method; 4] = [
    Method::BirdNet,
    Method::Bats,
    Method::Perch,
    Method::BirdNetV3,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_models_match_their_method() {
        for m in ALL {
            if let Some(live) = m.info().live_model {
                assert_eq!(live.method, m);
            }
        }
    }

    #[test]
    fn component_rows_need_a_live_model() {
        for m in ALL {
            assert!(!m.info().component_rows || m.info().live_model.is_some());
        }
    }

    #[test]
    fn shared_min_confidence_has_one_default() {
        assert_eq!(
            Method::BirdNet.info().default_min_confidence,
            Method::Bats.info().default_min_confidence
        );
    }

    #[test]
    fn bats_band_follows_its_model_rate() {
        let scale = Method::Bats.model_rate() as f32 / 48_000.0;
        assert_eq!(Method::Bats.info().band, (250.0 * scale, 15_000.0 * scale));
    }
}

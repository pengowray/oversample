//! The model files BirdNET, BattyBirdNET and Perch 2.0 need: where to download
//! them, their size and SHA-256, and the attribution to show with results.
//!
//! The URLs name a fixed Hugging Face revision, so a file never changes under
//! the same name. Hugging Face sends CORS headers, so the browser can fetch them
//! directly. Nothing here is bundled with the app: files are downloaded when the
//! user asks, then kept (OPFS in the browser, the app data folder in Tauri).

use super::{BatRegion, Method, ModelPart};

/// What a file is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// BirdNET V2.4 with an embeddings output; used by both methods.
    Backbone,
    /// BirdNET's 6,522 labels, in the order of the backbone's logits.
    BirdNetLabels,
    /// A BattyBirdNET regional head.
    BatHead(BatRegion),
    /// The labels of a regional head, in the order of its outputs.
    BatLabels(BatRegion),
    /// Perch 2.0's embedding part, for tract: 5 s at 32 kHz in, the embedding and
    /// the spatial embedding (the classifier's input) out.
    PerchEmbedding,
    /// Perch 2.0's classifier for its 14,795 classes, as int8 prototypes (format
    /// in the file's model card).
    PerchClassifier,
    /// Perch 2.0's class names (one header line, then one per class), in the
    /// order of the classifier's outputs.
    PerchLabels,
    /// The BirdNET+ Geomodel: latitude, longitude and week in, a 0 to 1
    /// occurrence score for each of its 14,082 species out. Used to mark birds
    /// not expected at a recording's place and time.
    Geomodel,
    /// The Geomodel's species, one per line in output order: species code (an
    /// eBird code for birds, a number for others), scientific name, English
    /// name, tab-separated.
    GeomodelLabels,
}

/// One downloadable file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelFile {
    /// The name it is stored under. Unique; also the key for download status.
    pub id: &'static str,
    pub url: &'static str,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256, lowercase hex.
    pub sha256: &'static str,
    pub kind: FileKind,
}

impl ModelFile {
    /// What the worker builds from this file; `None` for label lists.
    pub fn part(&self) -> Option<ModelPart> {
        match self.kind {
            FileKind::Backbone => Some(ModelPart::Backbone),
            FileKind::BatHead(_) => Some(ModelPart::BatHead),
            FileKind::PerchEmbedding => Some(ModelPart::PerchEmbedding),
            FileKind::PerchClassifier => Some(ModelPart::PerchClassifier),
            // Run on its own, outside the worker's two slots.
            FileKind::Geomodel => None,
            FileKind::BirdNetLabels
            | FileKind::BatLabels(_)
            | FileKind::PerchLabels
            | FileKind::GeomodelLabels => None,
        }
    }

    /// True for Perch's files.
    pub fn is_perch(&self) -> bool {
        matches!(
            self.kind,
            FileKind::PerchEmbedding | FileKind::PerchClassifier | FileKind::PerchLabels
        )
    }
}

const BATTY_REV: &str = "f685d7f7de1805ef83f71ffaad7440a8f40b4e10";
const BIRDNET_REV: &str = "883825b65a370caff2974e02f31a15b02104ef0f";
/// The revision of huggingface.co/pengowray/perch-v2-tract: Google's Perch 2.0
/// split for tract by `research/perch-bench` (see `research/perch.md`). For
/// Perch's attribution, once it can be run.
pub const PERCH_REVISION: &str = "f378e7f4f98ce6298091e5e1768ec6d73466be89";
/// The revision of Perch's label list (huggingface.co/cgeorgiaw/Perch).
pub const PERCH_LABELS_REVISION: &str = "c3c3c816976a6c1bb75140b8fbaef7397a6f708f";
/// The revision of huggingface.co/tphakala/BirdNET-Geomodel (BirdNET+ Geomodel
/// v3.0.4 from birdnet-team/geomodel, Apache 2.0).
pub const GEOMODEL_REVISION: &str = "e4a58d521af276de0c69a9cf8348d431e97854e3";

pub const GEOMODEL: ModelFile = ModelFile {
    id: "birdnet-geomodel-v3.0.4-fp32-b1.onnx",
    url: "https://huggingface.co/tphakala/BirdNET-Geomodel/resolve/e4a58d521af276de0c69a9cf8348d431e97854e3/birdnet-geomodel-v3.0.4-fp32-b1.onnx",
    size: 15_503_473,
    sha256: "0de81d222c23dcb6fa428e958b4dac978783191357e01b7268a103fc6f08e61a",
    kind: FileKind::Geomodel,
};

pub const GEOMODEL_LABELS: ModelFile = ModelFile {
    id: "birdnet-geomodel-v3.0.4-labels.txt",
    url: "https://huggingface.co/tphakala/BirdNET-Geomodel/resolve/e4a58d521af276de0c69a9cf8348d431e97854e3/BirdNET%2B_Geomodel_V3.0.4_Global_14K_Labels.txt",
    size: 671_823,
    sha256: "8250b457e45d43fc3e77b5cbd06a1d311baf585ab9c51ed8d42e011d98534835",
    kind: FileKind::GeomodelLabels,
};

/// The location list's files, shared by every method; "Delete model" leaves them.
pub const GEOMODEL_FILES: [&ModelFile; 2] = [&GEOMODEL, &GEOMODEL_LABELS];

pub const PERCH_EMBEDDING: ModelFile = ModelFile {
    id: "perch_v2_embed.tract.onnx",
    url: "https://huggingface.co/pengowray/perch-v2-tract/resolve/f378e7f4f98ce6298091e5e1768ec6d73466be89/perch_v2_embed.tract.onnx",
    size: 44_718_343,
    sha256: "834e554388700e6671912446e250ae78db38b9d1e758ee06931b362e009fa472",
    kind: FileKind::PerchEmbedding,
};

pub const PERCH_CLASSIFIER: ModelFile = ModelFile {
    id: "perch_v2_head.int8.bin",
    url: "https://huggingface.co/pengowray/perch-v2-tract/resolve/f378e7f4f98ce6298091e5e1768ec6d73466be89/perch_v2_head.int8.bin",
    size: 91_433_120,
    sha256: "1e5500603a9a04c5ecc5c9669c09344a85af8f7c67664930062d78c91b9cbe25",
    kind: FileKind::PerchClassifier,
};

pub const PERCH_LABELS: ModelFile = ModelFile {
    id: "perch-v2-labels.csv",
    url: "https://huggingface.co/cgeorgiaw/Perch/resolve/c3c3c816976a6c1bb75140b8fbaef7397a6f708f/assets/labels.csv",
    size: 312_716,
    sha256: "e4d5c0397d8fb08bf90c6b13a34810af53504faad927e472fcc567793c9de057",
    kind: FileKind::PerchLabels,
};

/// Everything Perch needs, largest first.
pub const PERCH_FILES: [&ModelFile; 3] = [&PERCH_CLASSIFIER, &PERCH_EMBEDDING, &PERCH_LABELS];

pub const BACKBONE: ModelFile = ModelFile {
    id: "birdnet-v2.4-embeddings-fp32.onnx",
    url: "https://huggingface.co/tphakala/BattyBirdNET-onnx/resolve/f685d7f7de1805ef83f71ffaad7440a8f40b4e10/birdnet-v2.4-embeddings-fp32.onnx",
    size: 66_932_350,
    sha256: "b6b8f24dc9c3d43f2deb14a6f2c7b5b233e7477b6baf1b52341291e714903fb0",
    kind: FileKind::Backbone,
};

pub const BIRDNET_LABELS: ModelFile = ModelFile {
    id: "birdnet-v2.4-labels.txt",
    url: "https://huggingface.co/tphakala/BirdNET-v2.4/resolve/883825b65a370caff2974e02f31a15b02104ef0f/labels.txt",
    size: 259_894,
    sha256: "487937b6ad132b8506215523209a87b86adc9dce8e5ed3048ce9268189dddd3d",
    kind: FileKind::BirdNetLabels,
};

macro_rules! region_files {
    ($( $region:ident, $stem:literal,
        $head_size:literal, $head_sha:literal,
        $labels_size:literal, $labels_sha:literal; )*) => {
        /// Regional heads, in [`BatRegion::ALL`] order.
        pub const BAT_HEADS: [ModelFile; 11] = [$(
            ModelFile {
                id: concat!("BattyBirdNET-", $stem, "_fp32.onnx"),
                url: concat!(
                    "https://huggingface.co/tphakala/BattyBirdNET-onnx/resolve/",
                    "f685d7f7de1805ef83f71ffaad7440a8f40b4e10",
                    "/fp32/BattyBirdNET-", $stem, "_fp32.onnx"
                ),
                size: $head_size,
                sha256: $head_sha,
                kind: FileKind::BatHead(BatRegion::$region),
            },
        )*];
        /// Regional label lists, in [`BatRegion::ALL`] order.
        pub const BAT_LABELS: [ModelFile; 11] = [$(
            ModelFile {
                id: concat!("BattyBirdNET-", $stem, "_Labels.txt"),
                url: concat!(
                    "https://huggingface.co/tphakala/BattyBirdNET-onnx/resolve/",
                    "f685d7f7de1805ef83f71ffaad7440a8f40b4e10",
                    "/labels/BattyBirdNET-", $stem, "_Labels.txt"
                ),
                size: $labels_size,
                sha256: $labels_sha,
                kind: FileKind::BatLabels(BatRegion::$region),
            },
        )*];
    };
}

region_files! {
    Bavaria, "Bavaria-256kHz",
        131827, "7ee3936621d180b9fe42f3732703339662b154135ce205f711797bca7daa44ea",
        966, "ff4a3f9a351f202c8712c807c6bb8b29df0b1c75ddab48e543ec76a88a42715c";
    BavariaHigh, "Bavaria-256kHz-high",
        99026, "3d1d5bc174ed70bfc22a53439fe468a2a4aa317b755600d1e193cefbae307a30",
        904, "26bc12ecf6c5ca9ce8837cd1bebe6e1cb2ce95f0261355a827383c85a0dd9d96";
    Eu, "EU-256kHz",
        123626, "f316073482ab95f48d65ca76e8b2aaa572019b3d286ab07a68ba57cea52d12f7",
        1081, "9ad705d4bcd93040929a059854df968acebefee9f7513e97a558871c3997e65e";
    Scotland, "Scotland-256kHz",
        45725, "003e3da16d3607d52dd5c963d71eec89fdfd58224dccc02bc6a27d58d21cbd85",
        356, "3dc657a38f691c20f351fa19e36b9919927aec2e30dc32f61ae9fd9bb319331b";
    SouthWales, "SouthWales-256kHz",
        119526, "14534d34fc54b0bc267ba07a6eaddc10e360195b11d0c4b5f47460a4f1d5aea4",
        1072, "fc7ed8bd55c28b66cdcecc8d8acb8ea05850d9301aa65467fd5d192ee00e8214";
    Sweden, "Sweden-256kHz",
        94926, "85fe47431c275b5370e0c8d0aa9b049f54d32035f736afcec4ac5d62c1adb591",
        737, "c43042ebd458eed4cc7258fcd6526e0299a61e27146e3ca989300f696d1f2e02";
    Uk, "UK-256kHz",
        82625, "aa9d45a5e3e64b6c28a131d16a98346ee1095c2d4c9f4785e2ff1d5a6e4b27b6",
        649, "4cc63b7cfd0a8e4380857fc3f5d576e8ec48d80cbdc9060873abb20c4ef78740";
    Usa, "USA-256kHz",
        156427, "9230fb49c87b9953f311fa1d408eac8359a1c8761264204f51b796406bcfcc63",
        1222, "3cf597702b5f0f558b227df3a01648da7eb52cc632ec70a148fd159763ba4399";
    UsaEast, "USA-EAST-256kHz",
        94926, "403901ce25c3daecdbd4d83017da8ff54c802f0feb78fc66355677f5c8905241",
        663, "db88ade98f2680af786911f1de49e5c29425335bbbc814c4a40c1e71ef888713";
    UsaEastHigh, "USA-EAST-256kHz-high",
        70325, "cb3fd538fb8adc87f775fad4fe5f9b3e1f56e78c5c7acd9abed4da7034e39772",
        613, "438b01d917b3833f707cdf9e9d13f0b13eee2318ad23eeb34089b76b9f22e566";
    UsaWest, "USA-WEST-256kHz",
        115426, "d1d3573a379e9e8561a66dc27ab768342d7d3823268440ec9ab624b8fb4640fa",
        867, "f01a993b749f455636de5811e6ab9de96537f05dc191ec1919acf4365a6e6386";
}

fn region_index(region: BatRegion) -> usize {
    BatRegion::ALL
        .iter()
        .position(|r| *r == region)
        .expect("every region is in ALL")
}

/// The head for `region`.
pub fn bat_head(region: BatRegion) -> &'static ModelFile {
    &BAT_HEADS[region_index(region)]
}

/// The label list for `region`'s head.
pub fn bat_labels(region: BatRegion) -> &'static ModelFile {
    &BAT_LABELS[region_index(region)]
}

/// What to run: BirdNET, BattyBirdNET with one region's head, or Perch 2.0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelChoice {
    BirdNet,
    Bats(BatRegion),
    Perch,
}

impl ModelChoice {
    pub fn method(self) -> Method {
        match self {
            ModelChoice::BirdNet => Method::BirdNet,
            ModelChoice::Bats(_) => Method::Bats,
            ModelChoice::Perch => Method::Perch,
        }
    }

    /// Every file a run needs, largest first.
    pub fn files(self) -> Vec<&'static ModelFile> {
        match self {
            ModelChoice::BirdNet => vec![&BACKBONE, &BIRDNET_LABELS],
            ModelChoice::Bats(r) => vec![&BACKBONE, bat_head(r), bat_labels(r)],
            ModelChoice::Perch => PERCH_FILES.to_vec(),
        }
    }

    /// The models the worker needs, in load order: the audio model, then any
    /// head.
    pub fn models(self) -> Vec<&'static ModelFile> {
        match self {
            ModelChoice::BirdNet => vec![&BACKBONE],
            ModelChoice::Bats(r) => vec![&BACKBONE, bat_head(r)],
            ModelChoice::Perch => vec![&PERCH_EMBEDDING, &PERCH_CLASSIFIER],
        }
    }

    /// The label list for the classes this choice scores.
    pub fn labels(self) -> &'static ModelFile {
        match self {
            ModelChoice::BirdNet => &BIRDNET_LABELS,
            ModelChoice::Bats(r) => bat_labels(r),
            ModelChoice::Perch => &PERCH_LABELS,
        }
    }
}

/// Every file in the catalog.
pub fn all_files() -> impl Iterator<Item = &'static ModelFile> {
    birdnet_files().chain(PERCH_FILES).chain(GEOMODEL_FILES)
}

/// BirdNET's and BattyBirdNET's files (they share the backbone), without
/// Perch's.
pub fn birdnet_files() -> impl Iterator<Item = &'static ModelFile> {
    [&BACKBONE, &BIRDNET_LABELS]
        .into_iter()
        .chain(BAT_HEADS.iter())
        .chain(BAT_LABELS.iter())
}

/// The catalog entry stored under `id`, if any. The Tauri commands accept only
/// these names.
pub fn file_by_id(id: &str) -> Option<&'static ModelFile> {
    all_files().find(|f| f.id == id)
}

/// Licence and credit for a method, to show with its results and in About.
#[derive(Clone, Copy, Debug)]
pub struct Attribution {
    /// The model's name, e.g. "BirdNET V2.4".
    pub name: &'static str,
    /// Line required wherever the results appear.
    pub powered_by: &'static str,
    pub powered_by_url: &'static str,
    pub licence: &'static str,
    pub licence_url: &'static str,
    /// Who made the model and the files, one line each.
    pub credits: &'static [&'static str],
    /// The paper to cite.
    pub citation: &'static str,
    /// Where the files come from, and the revision they are pinned to.
    pub source_url: &'static str,
    pub revision: &'static str,
}

const BIRDNET_CREDITS: &[&str] = &[
    "BirdNET was developed by the K. Lisa Yang Center for Conservation Bioacoustics at the Cornell Lab of Ornithology and Chemnitz University of Technology.",
    "ONNX conversion by tphakala (huggingface.co/tphakala).",
];

const BATTY_CREDITS: &[&str] = &[
    "BirdNET was developed by the K. Lisa Yang Center for Conservation Bioacoustics at the Cornell Lab of Ornithology and Chemnitz University of Technology.",
    "BattyBirdNET bat classifiers by rdz-oss (Zinck 2023), github.com/rdz-oss/BattyBirdNET-Analyzer.",
    "ONNX conversion by tphakala (huggingface.co/tphakala).",
];

const CITATION: &str = "Kahl, S., Wood, C. M., Eibl, M., & Klinck, H. (2021). BirdNET: A deep learning solution for avian diversity monitoring. Ecological Informatics, 61, 101236.";

const PERCH_CREDITS: &[&str] = &[
    "Perch 2.0 was developed by Google DeepMind.",
    "Split into two smaller files for this app by Oversample (huggingface.co/pengowray/perch-v2-tract).",
];

const PERCH_CITATION: &str =
    "Perch 2.0: The Bittern Lesson for Bioacoustics (2025). arXiv:2508.04665.";

pub fn attribution(method: Method) -> Attribution {
    match method {
        Method::BirdNet => Attribution {
            name: "BirdNET V2.4",
            powered_by: "Powered by BirdNET",
            powered_by_url: "https://birdnet.cornell.edu/",
            licence: "CC BY-NC-SA 4.0",
            licence_url: "https://creativecommons.org/licenses/by-nc-sa/4.0/",
            credits: BIRDNET_CREDITS,
            citation: CITATION,
            source_url: "https://huggingface.co/tphakala/BattyBirdNET-onnx",
            revision: BATTY_REV,
        },
        Method::Bats => Attribution {
            name: "BattyBirdNET",
            powered_by: "Powered by BirdNET",
            powered_by_url: "https://birdnet.cornell.edu/",
            licence: "CC BY-NC-SA 4.0",
            licence_url: "https://creativecommons.org/licenses/by-nc-sa/4.0/",
            credits: BATTY_CREDITS,
            citation: CITATION,
            source_url: "https://huggingface.co/tphakala/BattyBirdNET-onnx",
            revision: BATTY_REV,
        },
        Method::Perch => Attribution {
            name: "Perch 2.0",
            powered_by: "Perch 2.0 by Google DeepMind",
            powered_by_url: "https://www.kaggle.com/models/google/bird-vocalization-classifier",
            licence: "Apache 2.0",
            licence_url: "https://www.apache.org/licenses/LICENSE-2.0",
            credits: PERCH_CREDITS,
            citation: PERCH_CITATION,
            source_url: "https://huggingface.co/pengowray/perch-v2-tract",
            revision: PERCH_REVISION,
        },
    }
}

/// The revision of the BirdNET label list (a different repository from the
/// model files).
pub const BIRDNET_LABELS_REVISION: &str = BIRDNET_REV;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_safe() {
        let ids: Vec<&str> = all_files().map(|f| f.id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
        for id in ids {
            assert!(id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')));
        }
    }

    #[test]
    fn region_files_line_up() {
        for r in BatRegion::ALL {
            assert_eq!(bat_head(r).kind, FileKind::BatHead(r));
            assert_eq!(bat_labels(r).kind, FileKind::BatLabels(r));
            assert!(bat_head(r).id.contains(r.file_stem()));
            assert!(bat_head(r).url.ends_with(bat_head(r).id));
        }
        assert_eq!(
            bat_head(BatRegion::BavariaHigh).id,
            "BattyBirdNET-Bavaria-256kHz-high_fp32.onnx"
        );
        assert_eq!(
            bat_labels(BatRegion::UsaEast).url,
            "https://huggingface.co/tphakala/BattyBirdNET-onnx/resolve/f685d7f7de1805ef83f71ffaad7440a8f40b4e10/labels/BattyBirdNET-USA-EAST-256kHz_Labels.txt"
        );
    }

    #[test]
    fn perch_files_are_pinned() {
        for f in PERCH_FILES {
            assert!(file_by_id(f.id).is_some());
            assert_eq!(f.sha256.len(), 64);
        }
        for f in [&PERCH_EMBEDDING, &PERCH_CLASSIFIER] {
            assert!(f.url.contains(PERCH_REVISION));
            assert!(f.url.ends_with(f.id));
        }
        assert!(PERCH_LABELS.url.contains(PERCH_LABELS_REVISION));
    }
}

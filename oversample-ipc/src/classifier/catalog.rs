//! The model files BirdNET and BattyBirdNET need: where to download them, their
//! size and SHA-256, and the attribution to show with results.
//!
//! The URLs name a fixed Hugging Face revision, so a file never changes under
//! the same name. Hugging Face sends CORS headers, so the browser can fetch them
//! directly. Nothing here is bundled with the app: files are downloaded when the
//! user asks, then kept (OPFS in the browser, the app data folder in Tauri).

use super::{BatRegion, Method};

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

const BATTY_REV: &str = "f685d7f7de1805ef83f71ffaad7440a8f40b4e10";
const BIRDNET_REV: &str = "883825b65a370caff2974e02f31a15b02104ef0f";

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

/// What to run: BirdNET, or BattyBirdNET with one region's head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelChoice {
    BirdNet,
    Bats(BatRegion),
}

impl ModelChoice {
    pub fn method(self) -> Method {
        match self {
            ModelChoice::BirdNet => Method::BirdNet,
            ModelChoice::Bats(_) => Method::Bats,
        }
    }

    /// Every file a run needs, largest first.
    pub fn files(self) -> Vec<&'static ModelFile> {
        match self {
            ModelChoice::BirdNet => vec![&BACKBONE, &BIRDNET_LABELS],
            ModelChoice::Bats(r) => vec![&BACKBONE, bat_head(r), bat_labels(r)],
        }
    }

    /// The label list for the classes this choice scores.
    pub fn labels(self) -> &'static ModelFile {
        match self {
            ModelChoice::BirdNet => &BIRDNET_LABELS,
            ModelChoice::Bats(r) => bat_labels(r),
        }
    }
}

/// Every file in the catalog.
pub fn all_files() -> impl Iterator<Item = &'static ModelFile> {
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
}

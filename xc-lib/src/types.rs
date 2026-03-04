use serde::{Deserialize, Serialize};

/// A single recording from the XC API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct XcRecording {
    pub id: u64,
    /// Genus
    #[serde(rename = "gen")]
    pub genus: String,
    /// Species epithet
    pub sp: String,
    /// English common name
    pub en: String,
    /// Group (bats, birds, frogs, grasshoppers, land mammals)
    pub grp: String,
    /// Family (e.g. Vespertilionidae)
    pub fam: String,
    /// Recordist
    pub rec: String,
    /// Country
    pub cnt: String,
    /// Location description
    pub loc: String,
    /// Latitude
    pub lat: String,
    /// Longitude
    pub lon: String,
    /// Date (YYYY-MM-DD)
    pub date: String,
    /// Time (HH:MM)
    pub time: String,
    /// Sound type (e.g. "echolocation, feeding buzz")
    #[serde(rename = "type")]
    pub sound_type: String,
    /// Quality rating (A-E)
    pub q: String,
    /// Duration string (e.g. "0:16")
    pub length: String,
    /// Sample rate (as string from API, e.g. "256000")
    pub smp: String,
    /// License URL
    pub lic: String,
    /// Download URL for audio file
    pub file_url: String,
    /// Original filename from XC
    pub file_name: String,
    /// Subspecies
    pub ssp: String,
    /// Remarks
    pub rmk: String,
}

/// Species summary extracted from recordings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct XcSpecies {
    #[serde(rename = "gen")]
    pub genus: String,
    pub sp: String,
    pub en: String,
    pub fam: String,
    pub recording_count: u32,
}

/// Cached taxonomy for a group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct XcGroupTaxonomy {
    pub group: String,
    pub country: Option<String>,
    pub species: Vec<XcSpecies>,
    pub total_recordings: u32,
    pub last_updated: String,
}

/// Paginated search result from the API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct XcSearchResult {
    pub num_recordings: u32,
    pub num_species: u32,
    pub num_pages: u32,
    pub page: u32,
    pub recordings: Vec<XcRecording>,
}

/// Index entry for a cached recording (compatible with demo-sounds index.json).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct XcIndexEntry {
    pub filename: String,
    pub metadata: String,
    pub xc_id: u64,
    pub en: String,
    pub species: String,
    pub source: String,
}

/// Available groups on xeno-canto.
pub const XC_GROUPS: &[&str] = &["bats", "birds", "frogs", "grasshoppers", "land mammals"];

use serde::{Deserialize, Serialize};

/// A single recording from the XC API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct XcRecording {
    /// Recording ID (string in API, stored as String for fidelity)
    pub id: String,
    /// Genus
    #[serde(rename = "gen")]
    pub genus: String,
    /// Species epithet
    pub sp: String,
    /// Subspecies
    pub ssp: String,
    /// English common name
    pub en: String,
    /// Group (bats, birds, frogs, grasshoppers, land mammals)
    pub grp: String,
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
    /// Altitude (metres)
    pub alt: String,
    /// Sound type (e.g. "echolocation, feeding buzz")
    #[serde(rename = "type")]
    pub sound_type: String,
    /// Sex of animal
    pub sex: String,
    /// Life stage
    pub stage: String,
    /// Recording method (e.g. "field recording")
    pub method: String,
    /// Recording page URL
    pub url: String,
    /// Download URL for audio file (API field: "file")
    pub file_url: String,
    /// Original filename from XC (API field: "file-name")
    pub file_name: String,
    /// License URL
    pub lic: String,
    /// Quality rating (A-E)
    pub q: String,
    /// Duration string (e.g. "0:16")
    pub length: String,
    /// Time (HH:MM)
    pub time: String,
    /// Date (YYYY-MM-DD)
    pub date: String,
    /// Upload date (YYYY-MM-DD)
    pub uploaded: String,
    /// Other species heard
    pub also: Vec<String>,
    /// Remarks
    pub rmk: String,
    /// Whether the animal was seen ("yes"/"no")
    pub animal_seen: String,
    /// Whether playback was used ("yes"/"no")
    pub playback_used: String,
    /// Temperature
    pub temp: String,
    /// Registration number
    pub regnr: String,
    /// Automatic recording? ("yes"/"no")
    pub auto_rec: String,
    /// Recording device (e.g. "Olympus LS-14")
    pub dvc: String,
    /// Microphone (e.g. "Sennheiser ME66")
    pub mic: String,
    /// Sample rate (as string from API, e.g. "256000")
    pub smp: String,
}

impl XcRecording {
    /// Parse the ID as a u64 (for numeric contexts like sidecar JSON `xc_id`).
    pub fn id_num(&self) -> u64 {
        self.id.parse().unwrap_or(0)
    }
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

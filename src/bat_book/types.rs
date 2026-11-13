use serde::Deserialize;

/// How common a species is in a given region.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Commonness {
    VeryCommon,
    Common,
    Uncommon,
    Rare,
    Endangered,
    Vagrant,
}

impl Commonness {
    pub fn label(self) -> &'static str {
        match self {
            Self::VeryCommon => "Very Common",
            Self::Common => "Common",
            Self::Uncommon => "Uncommon",
            Self::Rare => "Rare",
            Self::Endangered => "Endangered",
            Self::Vagrant => "Vagrant",
        }
    }
}

/// Base bat species/family data, defined once and reused across books.
/// Loaded from `src/data/bat_book.json` (see `data.rs`).
#[derive(Debug, Deserialize)]
pub struct BatSpecies {
    /// Unique identifier, e.g. "vespertilionidae" or "chalinolobus_gouldii"
    pub id: String,
    /// Display name, e.g. "Vesper Bats" or "Gould's Wattled Bat"
    pub name: String,
    /// Scientific (binomial) name, e.g. "Chalinolobus gouldii" (empty for family-level)
    pub scientific_name: String,
    /// Taxonomic family name
    pub family: String,
    /// Call type abbreviation (CF, FM, QCF, CF-FM, clicks, none)
    pub call_type: String,
    /// Lower bound of typical echolocation frequency range (Hz)
    pub freq_lo_hz: f64,
    /// Upper bound of typical echolocation frequency range (Hz)
    pub freq_hi_hz: f64,
    /// Short description (generic / species-level)
    pub description: String,
    /// Whether this species uses echolocation (false for flying foxes etc.)
    pub echolocates: bool,
    /// Other scientific names for this species, such as an older genus
    /// (`Lasiurus cinereus` for `Aeorestes cinereus`). Metadata often uses them.
    #[serde(default)]
    pub synonyms: Vec<String>,
}

/// A book entry definition: a species id with optional regional overrides.
#[derive(Debug, Deserialize)]
pub struct BookEntryDef {
    /// Species id (matches `BatSpecies::id`)
    pub id: String,
    /// Regional commonness (None for family-level entries)
    #[serde(default)]
    pub commonness: Option<Commonness>,
    /// Override description for this book (None = use species default)
    #[serde(default)]
    pub description: Option<String>,
    /// Override display name for this book (None = use species default)
    #[serde(default)]
    pub name: Option<String>,
    /// Override frequency range (lo, hi) in Hz for this book (None = use the
    /// species range). The species range covers every region the sources
    /// cover; a regional book may narrow it to the calls heard there.
    #[serde(default)]
    pub freq_hz: Option<(f64, f64)>,
}

impl BookEntryDef {
    /// Materialize into a full BatBookEntry by resolving overrides.
    /// Both live in the static bat book data, so the entry borrows from them.
    pub fn materialize(&'static self, s: &'static BatSpecies) -> BatBookEntry {
        let (freq_lo_hz, freq_hi_hz) = self.freq_hz.unwrap_or((s.freq_lo_hz, s.freq_hi_hz));
        BatBookEntry {
            id: &s.id,
            name: self.name.as_deref().unwrap_or(&s.name),
            scientific_name: &s.scientific_name,
            family: &s.family,
            call_type: &s.call_type,
            freq_lo_hz,
            freq_hi_hz,
            description: self.description.as_deref().unwrap_or(&s.description),
            commonness: self.commonness,
            echolocates: s.echolocates,
        }
    }
}

/// A bat book manifest containing entries for a region.
#[derive(Clone, Debug, PartialEq)]
pub struct BatBookManifest {
    pub region: String,
    pub entries: Vec<BatBookEntry>,
}

/// A single bat book entry — materialized from species + book overrides.
#[derive(Clone, Debug, PartialEq)]
pub struct BatBookEntry {
    /// Unique identifier, e.g. "vespertilionidae" or "chalinolobus_gouldii"
    pub id: &'static str,
    /// Display name, e.g. "Vesper Bats" or "Gould's Wattled Bat"
    pub name: &'static str,
    /// Scientific (binomial) name, e.g. "Chalinolobus gouldii" (empty for family-level)
    pub scientific_name: &'static str,
    /// Taxonomic family name
    pub family: &'static str,
    /// Call type abbreviation (CF, FM, QCF, CF-FM, clicks, none)
    pub call_type: &'static str,
    /// Lower bound of typical echolocation frequency range (Hz)
    pub freq_lo_hz: f64,
    /// Upper bound of typical echolocation frequency range (Hz)
    pub freq_hi_hz: f64,
    /// Short description
    pub description: &'static str,
    /// How common the species is in the region (None for family-level entries)
    pub commonness: Option<Commonness>,
    /// Whether this species uses echolocation
    pub echolocates: bool,
}

/// Format Hz as kHz, showing one decimal only when it carries information
/// (45000 -> "45", 45500 -> "45.5") so sub-kHz precision isn't truncated away.
fn fmt_khz(hz: f64) -> String {
    let khz = hz / 1000.0;
    if (khz - khz.round()).abs() < 0.05 {
        format!("{}", khz.round() as i64)
    } else {
        format!("{khz:.1}")
    }
}

impl BatBookEntry {
    /// Format frequency range as "XX\u{2013}YY kHz"
    pub fn freq_range_label(&self) -> String {
        if self.freq_lo_hz == 0.0 && self.freq_hi_hz == 0.0 {
            return "\u{2014}".to_string(); // em dash for no echolocation
        }
        format!(
            "{}\u{2013}{} kHz",
            fmt_khz(self.freq_lo_hz),
            fmt_khz(self.freq_hi_hz)
        )
    }
}

/// Available bat book regions.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BatBookRegion {
    #[default]
    Global,
    Europe,
    NorthAmerica,
    Australia,
    VicAustralia,
    Africa,
    SoutheastAsia,
    SouthAsia,
    EastAsia,
    MiddleEast,
    SouthAmerica,
    CostaRica,
    Japan,
    UK,
    Antarctica,
    // Country-specific books
    Greece,
    Sweden,
    Netherlands,
    Canada,
    UnitedStates,
    Mexico,
    Kenya,
    Eswatini,
}

impl BatBookRegion {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "Global (All Families)",
            Self::Europe => "Europe",
            Self::NorthAmerica => "North America",
            Self::Australia => "Australia",
            Self::VicAustralia => "VIC, Australia",
            Self::Africa => "Africa",
            Self::SoutheastAsia => "Southeast Asia",
            Self::SouthAsia => "South Asia",
            Self::EastAsia => "East Asia",
            Self::MiddleEast => "Middle East",
            Self::SouthAmerica => "South America",
            Self::CostaRica => "Costa Rica",
            Self::Japan => "Japan",
            Self::UK => "United Kingdom",
            Self::Antarctica => "Antarctica",
            Self::Greece => "Greece",
            Self::Sweden => "Sweden",
            Self::Netherlands => "Netherlands",
            Self::Canada => "Canada",
            Self::UnitedStates => "United States",
            Self::Mexico => "Mexico",
            Self::Kenya => "Kenya",
            Self::Eswatini => "Eswatini",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::Europe => "Europe",
            Self::NorthAmerica => "N. America",
            Self::Australia => "Australia",
            Self::VicAustralia => "VIC, AU",
            Self::Africa => "Africa",
            Self::SoutheastAsia => "SE Asia",
            Self::SouthAsia => "S. Asia",
            Self::EastAsia => "E. Asia",
            Self::MiddleEast => "Middle East",
            Self::SouthAmerica => "S. America",
            Self::CostaRica => "Costa Rica",
            Self::Japan => "Japan",
            Self::UK => "UK",
            Self::Antarctica => "Antarctica",
            Self::Greece => "Greece",
            Self::Sweden => "Sweden",
            Self::Netherlands => "NL",
            Self::Canada => "Canada",
            Self::UnitedStates => "USA",
            Self::Mexico => "Mexico",
            Self::Kenya => "Kenya",
            Self::Eswatini => "Eswatini",
        }
    }

    pub const ALL: &'static [BatBookRegion] = &[
        Self::Global,
        Self::UK,
        Self::Greece,
        Self::Sweden,
        Self::Netherlands,
        Self::Europe,
        Self::Canada,
        Self::UnitedStates,
        Self::Mexico,
        Self::NorthAmerica,
        Self::CostaRica,
        Self::SouthAmerica,
        Self::Kenya,
        Self::Eswatini,
        Self::Africa,
        Self::MiddleEast,
        Self::SouthAsia,
        Self::SoutheastAsia,
        Self::EastAsia,
        Self::Japan,
        Self::Australia,
        Self::VicAustralia,
        Self::Antarctica,
    ];

    /// Stable snake_case key: the region's file name in the bat book data
    /// (`book/regions/<key>.json`). Same as `storage_key`.
    pub fn key(self) -> &'static str {
        self.storage_key()
    }

    pub fn storage_key(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Europe => "europe",
            Self::NorthAmerica => "north_america",
            Self::Australia => "australia",
            Self::VicAustralia => "vic_australia",
            Self::Africa => "africa",
            Self::SoutheastAsia => "southeast_asia",
            Self::SouthAsia => "south_asia",
            Self::EastAsia => "east_asia",
            Self::MiddleEast => "middle_east",
            Self::SouthAmerica => "south_america",
            Self::CostaRica => "costa_rica",
            Self::Japan => "japan",
            Self::UK => "uk",
            Self::Antarctica => "antarctica",
            Self::Greece => "greece",
            Self::Sweden => "sweden",
            Self::Netherlands => "netherlands",
            Self::Canada => "canada",
            Self::UnitedStates => "united_states",
            Self::Mexico => "mexico",
            Self::Kenya => "kenya",
            Self::Eswatini => "eswatini",
        }
    }

    pub fn from_storage_key(key: &str) -> Option<Self> {
        Self::ALL.iter().find(|r| r.storage_key() == key).copied()
    }

    /// Whether this region is a part of `other` that has a book of its own
    /// but no outline to find it from a position (VIC within Australia). A
    /// hand-picked part is kept for a recording whose place gives `other`.
    pub fn is_part_of(self, other: BatBookRegion) -> bool {
        matches!((self, other), (Self::VicAustralia, Self::Australia))
    }
}

/// Whether the bat book is in Auto mode (resolve from file metadata) or manually pinned.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BatBookMode {
    #[default]
    Auto,
    Manual(BatBookRegion),
}

impl BatBookMode {
    pub fn storage_key(&self) -> &str {
        match self {
            Self::Auto => "auto",
            Self::Manual(r) => r.storage_key(),
        }
    }

    pub fn from_storage_key(key: &str) -> Self {
        if key == "auto" {
            Self::Auto
        } else {
            BatBookRegion::from_storage_key(key)
                .map(Self::Manual)
                .unwrap_or(Self::Auto)
        }
    }
}

/// Result of auto-resolving the bat book from file metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoResolved {
    /// The region chosen by auto-resolve.
    pub region: BatBookRegion,
    /// Species id matched from metadata (if found in a book).
    pub matched_species_id: Option<String>,
    /// Human-readable label for what Auto resolved to, e.g. "UK", "France", "Global".
    pub source_label: String,
    /// True when the region was chosen because it's the user's favourite (fallback).
    pub from_favourite: bool,
    /// The recording's own place, when the region was picked from it. A
    /// recording with a place of its own keeps its region even when another
    /// region is picked in the menu.
    pub recording_place: Option<RecordingPlace>,
}

/// Where a recording's country came from.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaceSource {
    /// Its GPS position (GUANO or source-site coordinates), looked up offline.
    Gps { lat: f32, lon: f32 },
    /// The "Country" field of the site the recording came from, named by
    /// `source` ("Xeno-canto", "iNaturalist", ...).
    Metadata { source: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordingPlace {
    pub source: PlaceSource,
    pub country: String,
}

impl RecordingPlace {
    /// Where the place came from, e.g. "From GPS position 52.48, -1.90" (with
    /// the country added when the region is not the country itself) or "From
    /// iNaturalist country: Australia".
    pub fn detail(&self, region: BatBookRegion) -> String {
        match &self.source {
            &PlaceSource::Gps { lat, lon } if self.country == region.label() => {
                format!("From GPS position {lat:.2}, {lon:.2}")
            }
            &PlaceSource::Gps { lat, lon } => {
                format!("From GPS position {lat:.2}, {lon:.2} ({})", self.country)
            }
            PlaceSource::Metadata { source } => format!("From {source} country: {}", self.country),
        }
    }
}

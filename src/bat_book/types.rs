/// A bat book manifest containing entries for a region.
#[derive(Clone, Debug, PartialEq)]
pub struct BatBookManifest {
    pub region: String,
    pub entries: Vec<BatBookEntry>,
}

/// A single bat book entry (typically a family or broad category).
#[derive(Clone, Debug, PartialEq)]
pub struct BatBookEntry {
    /// Unique identifier, e.g. "vespertilionidae"
    pub id: &'static str,
    /// Display name, e.g. "Vesper Bats"
    pub name: &'static str,
    /// Taxonomic family name
    pub family: &'static str,
    /// Call type abbreviation (CF, FM, QCF, CF-FM, clicks)
    pub call_type: &'static str,
    /// Lower bound of typical echolocation frequency range (Hz)
    pub freq_lo_hz: f64,
    /// Upper bound of typical echolocation frequency range (Hz)
    pub freq_hi_hz: f64,
    /// Short description
    pub description: &'static str,
}

impl BatBookEntry {
    /// Format frequency range as "XX–YY kHz"
    pub fn freq_range_label(&self) -> String {
        format!(
            "{}\u{2013}{} kHz",
            (self.freq_lo_hz / 1000.0) as u32,
            (self.freq_hi_hz / 1000.0) as u32,
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
    Africa,
    Asia,
    SouthAmerica,
}

impl BatBookRegion {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "Global (All Families)",
            Self::Europe => "Europe",
            Self::NorthAmerica => "North America",
            Self::Australia => "Australia",
            Self::Africa => "Africa",
            Self::Asia => "Asia",
            Self::SouthAmerica => "South America",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::Europe => "Europe",
            Self::NorthAmerica => "N. America",
            Self::Australia => "Australia",
            Self::Africa => "Africa",
            Self::Asia => "Asia",
            Self::SouthAmerica => "S. America",
        }
    }

    pub const ALL: &'static [BatBookRegion] = &[
        Self::Global,
        Self::Europe,
        Self::NorthAmerica,
        Self::SouthAmerica,
        Self::Africa,
        Self::Asia,
        Self::Australia,
    ];
}

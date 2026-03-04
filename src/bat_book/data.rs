use super::types::{BatBookEntry, BatBookManifest, BatBookRegion};

/// Hardcoded global bat book with researched frequency ranges.
///
/// Sources:
/// - Jones & Barlow (2004) JEB: Scaling of echolocation call parameters
/// - Jung et al. (2014) PMC: Molossidae call design
/// - Shi et al. (2024) PMC: Correlated evolution body size & echolocation
/// - Jones & Rayner (1989) Springer: Horseshoe bat foraging ecology
/// - Collen (2012) BioOne: Rhinolophidae & Hipposideridae comparative ecology
const GLOBAL_ENTRIES: &[BatBookEntry] = &[
    BatBookEntry {
        id: "rhinolophidae",
        name: "Horseshoe Bats",
        family: "Rhinolophidae",
        call_type: "CF",
        freq_lo_hz: 30_000.0,
        freq_hi_hz: 120_000.0,
        description: "Constant-frequency calls; species range ~30 kHz (large) to ~112 kHz (lesser horseshoe)",
    },
    BatBookEntry {
        id: "hipposideridae",
        name: "Roundleaf Bats",
        family: "Hipposideridae",
        call_type: "CF",
        freq_lo_hz: 60_000.0,
        freq_hi_hz: 160_000.0,
        description: "CF calls; Cleotis percivalis reaches 212 kHz, the highest known bat frequency",
    },
    BatBookEntry {
        id: "vespertilionidae",
        name: "Vesper Bats",
        family: "Vespertilionidae",
        call_type: "FM",
        freq_lo_hz: 15_000.0,
        freq_hi_hz: 120_000.0,
        description: "Broadest family; FM sweeps; most species 20\u{2013}60 kHz peak",
    },
    BatBookEntry {
        id: "molossidae",
        name: "Free-tailed Bats",
        family: "Molossidae",
        call_type: "QCF",
        freq_lo_hz: 10_000.0,
        freq_hi_hz: 45_000.0,
        description: "Narrowband, long-duration QCF calls; 16\u{2013}44 kHz peak typical",
    },
    BatBookEntry {
        id: "emballonuridae",
        name: "Sheath-tailed Bats",
        family: "Emballonuridae",
        call_type: "QCF",
        freq_lo_hz: 20_000.0,
        freq_hi_hz: 55_000.0,
        description: "Quasi-constant-frequency calls; some species sweep 40\u{2013}100 kHz",
    },
    BatBookEntry {
        id: "phyllostomidae",
        name: "Leaf-nosed Bats",
        family: "Phyllostomidae",
        call_type: "FM",
        freq_lo_hz: 40_000.0,
        freq_hi_hz: 120_000.0,
        description: "Low-intensity \"whispering\" bats; multi-harmonic FM calls",
    },
    BatBookEntry {
        id: "mormoopidae",
        name: "Ghost-faced Bats",
        family: "Mormoopidae",
        call_type: "CF-FM",
        freq_lo_hz: 45_000.0,
        freq_hi_hz: 65_000.0,
        description: "P. parnellii CF at ~63 kHz with FM sweep to ~54 kHz",
    },
    BatBookEntry {
        id: "miniopteridae",
        name: "Bent-winged Bats",
        family: "Miniopteridae",
        call_type: "FM",
        freq_lo_hz: 45_000.0,
        freq_hi_hz: 85_000.0,
        description: "FM-dominated calls; formerly classified within Vespertilionidae",
    },
    BatBookEntry {
        id: "nycteridae",
        name: "Slit-faced Bats",
        family: "Nycteridae",
        call_type: "FM",
        freq_lo_hz: 30_000.0,
        freq_hi_hz: 80_000.0,
        description: "Low-intensity, multi-harmonic FM calls emitted through nostrils",
    },
    BatBookEntry {
        id: "megadermatidae",
        name: "False Vampires",
        family: "Megadermatidae",
        call_type: "FM",
        freq_lo_hz: 20_000.0,
        freq_hi_hz: 110_000.0,
        description: "Low-intensity broadband FM; large carnivorous bats",
    },
    BatBookEntry {
        id: "pteropodidae",
        name: "Fruit Bats",
        family: "Pteropodidae",
        call_type: "clicks",
        freq_lo_hz: 10_000.0,
        freq_hi_hz: 100_000.0,
        description: "Most don't echolocate; Rousettus uses tongue clicks for cave navigation",
    },
];

/// Get the bat book manifest for a given region.
/// Currently returns the same global data for all regions.
pub fn get_manifest(region: BatBookRegion) -> BatBookManifest {
    BatBookManifest {
        region: region.short_label().to_string(),
        entries: GLOBAL_ENTRIES.to_vec(),
    }
}

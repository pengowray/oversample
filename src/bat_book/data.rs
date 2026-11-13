//! Bat book data: species and regional books.
//!
//! The data is authored in the `bat-call-sketches` repository (a sibling
//! checkout at `../batchi-extras/bat-call-sketches`, files under `book/`,
//! format in its FORMAT.md). Its `tools/bundle.py` checks the files and
//! writes `src/data/bat_book.json`, which is compiled in here.

use super::types::{BatBookEntry, BatBookManifest, BatBookRegion, BatSpecies, BookEntryDef};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

const BUNDLE: &str = include_str!("../data/bat_book.json");

#[derive(Debug, Default, Deserialize)]
struct BookData {
    species: Vec<BatSpecies>,
    /// Region key (`BatBookRegion::key`) to its entries, in authored order.
    regions: HashMap<String, Vec<BookEntryDef>>,
}

fn data() -> &'static BookData {
    static DATA: OnceLock<BookData> = OnceLock::new();
    DATA.get_or_init(|| match serde_json::from_str(BUNDLE) {
        Ok(v) => v,
        Err(e) => {
            log::error!("bat_book.json: {e}");
            BookData::default()
        }
    })
}

/// Every species in the bat book, in no particular order.
pub fn all_species() -> &'static [BatSpecies] {
    &data().species
}

/// The species with this id.
pub fn species(id: &str) -> Option<&'static BatSpecies> {
    static BY_ID: OnceLock<HashMap<&'static str, &'static BatSpecies>> = OnceLock::new();
    BY_ID
        .get_or_init(|| all_species().iter().map(|s| (s.id.as_str(), s)).collect())
        .get(id)
        .copied()
}

/// A scientific name to look up in the bat book: trimmed and lower-cased,
/// with the binomial of a trinomial as a second form, so "Myotis lucifugus
/// alascensis" also matches "Myotis lucifugus".
pub struct NameQuery {
    full: String,
    binomial: Option<String>,
}

impl NameQuery {
    /// `None` for a blank name.
    pub fn new(name: &str) -> Option<Self> {
        let full = name.trim().to_lowercase();
        if full.is_empty() {
            return None;
        }
        let parts: Vec<&str> = full.split_whitespace().collect();
        let binomial = (parts.len() > 2).then(|| format!("{} {}", parts[0], parts[1]));
        Some(Self { full, binomial })
    }

    /// The species' scientific name and synonyms, lower-cased.
    fn names(s: &BatSpecies) -> impl Iterator<Item = String> + '_ {
        std::iter::once(s.scientific_name.as_str())
            .chain(s.synonyms.iter().map(String::as_str))
            .map(str::to_lowercase)
    }

    /// Whether the species' scientific name or a synonym is the whole name.
    pub fn matches_full(&self, s: &BatSpecies) -> bool {
        Self::names(s).any(|n| n == self.full)
    }

    /// Whether the species' scientific name or a synonym is the whole name,
    /// or the binomial of a trinomial.
    pub fn matches(&self, s: &BatSpecies) -> bool {
        Self::names(s).any(|n| n == self.full || self.binomial.as_deref() == Some(n.as_str()))
    }
}

/// The species with this scientific name or synonym, from every species in
/// the bat book, including those in no regional book (`Pipistrellus
/// hanaki`). A match on the whole name wins over a trinomial's binomial. Two
/// species can share a name (`eptesicus_nilssonii` and its Japanese entry
/// `eptesicus_nilssonii_jp`); then the one with the lowest call frequency
/// wins, so the order of the bundle doesn't matter.
///
/// Callers that know the recording's region use
/// `auto_resolve::find_species`, which prefers the region's own entry.
pub fn species_by_scientific_name(name: &str) -> Option<&'static BatSpecies> {
    let query = NameQuery::new(name)?;
    let lowest = |matching: &dyn Fn(&BatSpecies) -> bool| {
        all_species()
            .iter()
            .filter(|s| matching(s))
            .min_by(|a, b| a.freq_lo_hz.total_cmp(&b.freq_lo_hz))
    };
    lowest(&|s| query.matches_full(s)).or_else(|| lowest(&|s| query.matches(s)))
}

/// Materialized entries for each region key, with non-echolocating species
/// sorted to the end. Built once.
fn books() -> &'static HashMap<&'static str, Vec<BatBookEntry>> {
    static BOOKS: OnceLock<HashMap<&'static str, Vec<BatBookEntry>>> = OnceLock::new();
    BOOKS.get_or_init(|| {
        let d = data();
        let by_id: HashMap<&str, &BatSpecies> =
            d.species.iter().map(|s| (s.id.as_str(), s)).collect();
        d.regions
            .iter()
            .map(|(key, defs)| {
                let mut entries: Vec<BatBookEntry> = defs
                    .iter()
                    .filter_map(|def| match by_id.get(def.id.as_str()) {
                        Some(s) => Some(def.materialize(s)),
                        None => {
                            log::error!("bat_book.json: {key} lists unknown species {}", def.id);
                            None
                        }
                    })
                    .collect();
                // Stable sort: echolocating first, non-echolocating last
                entries.sort_by_key(|e| if e.echolocates { 0u8 } else { 1 });
                (key.as_str(), entries)
            })
            .collect()
    })
}

/// Get the bat book manifest for a given region.
///
/// Non-echolocating species are always sorted to the end (stable sort preserves
/// relative order within each group). A region with no book falls back to the
/// Global book.
pub fn get_manifest(region: BatBookRegion) -> BatBookManifest {
    let b = books();
    let entries = b
        .get(region.key())
        .or_else(|| b.get(BatBookRegion::Global.key()))
        .cloned()
        .unwrap_or_default();
    BatBookManifest {
        region: region.short_label().to_string(),
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn range(region: BatBookRegion, id: &str) -> (f64, f64) {
        let e = get_manifest(region)
            .entries
            .into_iter()
            .find(|e| e.id == id)
            .unwrap();
        (e.freq_lo_hz, e.freq_hi_hz)
    }

    #[test]
    fn bundle_parses() {
        let d: BookData = serde_json::from_str(BUNDLE).expect("bat_book.json parses");
        assert!(!d.species.is_empty());
    }

    #[test]
    fn species_ids_are_unique() {
        let mut seen = HashSet::new();
        for s in all_species() {
            assert!(
                seen.insert(s.id.as_str()),
                "species {} is listed twice",
                s.id
            );
        }
    }

    #[test]
    fn region_keys_match_the_enum() {
        let all: HashSet<&str> = BatBookRegion::ALL.iter().map(|r| r.key()).collect();
        assert_eq!(
            all.len(),
            BatBookRegion::ALL.len(),
            "duplicate region in ALL"
        );
        for &r in BatBookRegion::ALL {
            assert_eq!(BatBookRegion::from_storage_key(r.key()), Some(r));
        }
        let data_keys: HashSet<&str> = data().regions.keys().map(String::as_str).collect();
        assert_eq!(
            data_keys, all,
            "bat_book.json regions differ from BatBookRegion::ALL"
        );
    }

    #[test]
    fn every_region_has_a_book() {
        for &r in BatBookRegion::ALL {
            let defs = &data().regions[r.key()];
            assert!(!defs.is_empty(), "{r:?} has no entries");
            assert_eq!(
                get_manifest(r).entries.len(),
                defs.len(),
                "{r:?}: an entry did not resolve to a species"
            );
        }
    }

    #[test]
    fn entries_resolve_and_are_unique_per_region() {
        let ids: HashSet<&str> = all_species().iter().map(|s| s.id.as_str()).collect();
        for (key, defs) in &data().regions {
            let mut seen = HashSet::new();
            for d in defs {
                assert!(
                    ids.contains(d.id.as_str()),
                    "{key}: unknown species {}",
                    d.id
                );
                assert!(seen.insert(d.id.as_str()), "{key}: {} listed twice", d.id);
            }
        }
    }

    #[test]
    fn species_by_scientific_name_reads_every_species() {
        let id = |name: &str| species_by_scientific_name(name).map(|s| s.id.as_str());
        // In no regional book.
        assert_eq!(id("Pipistrellus hanaki"), Some("pipistrellus_hanaki"));
        // A synonym, any case, with spaces around it.
        assert_eq!(id("  lasiurus CINEREUS "), Some("aeorestes_cinereus"));
        // A trinomial in the book matches itself, not its binomial; one that
        // isn't matches its binomial.
        assert_eq!(
            id("Rhinolophus ferrumequinum nippon"),
            Some("rhinolophus_ferrumequinum_nippon")
        );
        assert_eq!(
            id("Rhinolophus ferrumequinum"),
            Some("rhinolophus_ferrumequinum")
        );
        assert_eq!(id("Myotis lucifugus alascensis"), Some("myotis_lucifugus"));
        // Two species share the name: the lower call frequency wins.
        assert_eq!(id("Eptesicus nilssonii"), Some("eptesicus_nilssonii_jp"));
        // Codes and placeholders match nothing.
        for name in ["PIPPIP", "Mystery mystery", "Myotis", "", "  "] {
            assert_eq!(id(name), None, "{name:?}");
        }
    }

    #[test]
    fn species_ranges_are_valid() {
        for s in all_species() {
            assert!(
                0.0 <= s.freq_lo_hz && s.freq_lo_hz <= s.freq_hi_hz,
                "{}: bad range",
                s.id
            );
        }
    }

    #[test]
    fn vic_book_overrides_the_species_range() {
        assert_eq!(
            range(BatBookRegion::Australia, "vespadelus_vulturnus"),
            (42_500.0, 53_500.0)
        );
        assert_eq!(
            range(BatBookRegion::VicAustralia, "vespadelus_vulturnus"),
            (42_500.0, 51_000.0)
        );
        // No override: the VIC book uses the species range.
        assert_eq!(
            range(BatBookRegion::VicAustralia, "vespadelus_regulus"),
            range(BatBookRegion::Australia, "vespadelus_regulus")
        );
    }

    #[test]
    fn overrides_are_valid_ranges() {
        for &region in BatBookRegion::ALL {
            for e in get_manifest(region).entries {
                assert!(
                    e.freq_lo_hz <= e.freq_hi_hz,
                    "{} in {region:?}: lo above hi",
                    e.id
                );
            }
        }
    }
}

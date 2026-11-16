use super::country_map::country_to_region;
use super::country_map::RegionMatch;
use super::data::{get_manifest, species, species_matching, NameQuery};
use super::types::{AutoResolved, BatBookRegion, BatSpecies};
use crate::recording_place::{areas, countries, recording_place, Area};
use crate::state::LoadedFile;

/// Search all regional bat books for a species by scientific name.
/// Returns `(region, species_id)` for the best match.
/// Prefers `preferred_region` if the species appears there; otherwise picks the
/// first non-Global match. A match on the whole name in any book wins over a
/// trinomial's binomial ("Myotis lucifugus alascensis" matching "Myotis
/// lucifugus"), so a subspecies with its own entry finds that entry.
fn find_species_across_books(
    scientific_name: &str,
    preferred_region: Option<BatBookRegion>,
) -> Option<(BatBookRegion, String)> {
    let query = NameQuery::new(scientific_name)?;
    find_in_books(|s| query.matches_full(s), preferred_region)
        .or_else(|| find_in_books(|s| query.matches(s), preferred_region))
}

/// The first species in the regional books that `matching` accepts, as
/// `(region, species_id)`: the one in `preferred_region` if that book has
/// one, else the first non-Global book's.
fn find_in_books(
    matching: impl Fn(&BatSpecies) -> bool,
    preferred_region: Option<BatBookRegion>,
) -> Option<(BatBookRegion, String)> {
    let mut best: Option<(BatBookRegion, String)> = None;

    for &region in BatBookRegion::ALL {
        if region == BatBookRegion::Global {
            continue; // Global only has family-level entries, skip
        }
        let manifest = get_manifest(region);
        for entry in &manifest.entries {
            if species(entry.id).is_some_and(&matching) {
                if preferred_region == Some(region) {
                    // Best possible: species is in the country's own region
                    return Some((region, entry.id.to_string()));
                }
                if best.is_none() {
                    best = Some((region, entry.id.to_string()));
                }
            }
        }
    }

    best
}

/// The bat book species of a recording labelled `scientific_name`: the
/// typical-calls strip's match, which prefers the entry in the book of the
/// recording's region (`preferred_region`), or else the first regional book
/// that has it. A species in no regional book (`Pipistrellus hanaki`) is
/// looked up among every species instead. A whole-name match anywhere wins
/// over a trinomial's binomial.
pub(crate) fn find_species(
    scientific_name: &str,
    preferred_region: Option<BatBookRegion>,
) -> Option<&'static BatSpecies> {
    let query = NameQuery::new(scientific_name)?;
    let in_books = |matching: &dyn Fn(&BatSpecies) -> bool| {
        find_in_books(matching, preferred_region).and_then(|(_, id)| species(&id))
    };
    let full = |s: &BatSpecies| query.matches_full(s);
    let any = |s: &BatSpecies| query.matches(s);
    in_books(&full)
        .or_else(|| species_matching(full))
        .or_else(|| in_books(&any))
        .or_else(|| species_matching(any))
}

/// The bat book region for a country, or for an area inside it that has a
/// book of its own (Victoria).
fn place_to_region(country: &str, area: Option<Area>) -> Option<RegionMatch> {
    match area {
        Some(Area::Victoria) => Some(RegionMatch {
            region: BatBookRegion::VicAustralia,
            approximate: false,
        }),
        _ => country_to_region(country),
    }
}

/// The region of the recording's place (source site country or GPS
/// position), if the region map knows it.
pub(crate) fn country_region(file: &LoadedFile) -> Option<BatBookRegion> {
    let place = recording_place(file)?;
    place_to_region(&place.country, place.area).map(|m| m.region)
}

/// Extract species scientific name from file metadata.
/// Tries XC "Scientific name" first, then GUANO Species|Manual, then Species|Auto.
pub(crate) fn get_scientific_name(file: &LoadedFile) -> Option<String> {
    // Source-site metadata
    if let Some(meta) = &file.site_metadata {
        if let Some(val) = meta
            .fields
            .iter()
            .find(|(k, _)| k == "Scientific name")
            .map(|(_, v)| v.clone())
        {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    // GUANO fields
    if let Some(guano) = &file.audio.metadata.guano {
        // Prefer manual over auto
        for key in &["Species|Manual", "Species|Auto"] {
            if let Some(val) = guano
                .fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
            {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// The region for a position, through the country it is in.
#[derive(Clone, Debug, PartialEq)]
pub enum PositionRegion {
    /// The region and the country's name.
    Found(BatBookRegion, String),
    /// The position is outside every country outline (at sea, or a small
    /// island missing at 1:110m).
    NoCountry,
    /// A country with no region in the region map.
    NoRegion(String),
}

pub fn region_at(lat: f32, lon: f32) -> PositionRegion {
    let Some(names) = countries::country_names_at(lat, lon) else {
        return PositionRegion::NoCountry;
    };
    match names
        .iter()
        .find_map(|n| place_to_region(n, areas::area_at(n, lat, lon)).map(|m| (m.region, n)))
    {
        Some((region, name)) => PositionRegion::Found(region, name.clone()),
        None => PositionRegion::NoRegion(names[0].clone()),
    }
}

/// The region a recording's own place gives (Xeno-Canto country or GPS), if
/// it has one the region map knows. This wins over a region picked in the
/// menu, which is for recordings without a place.
pub fn recording_region(file: &LoadedFile, favourites: &[BatBookRegion]) -> Option<AutoResolved> {
    let resolved = resolve_auto(Some(file), favourites);
    resolved.recording_place.is_some().then_some(resolved)
}

/// Resolve the bat book automatically from the current file's metadata.
///
/// Priority:
/// 1. Species + country → use country's region, highlight matched species
/// 2. Species only → use region where species was found
/// 3. Country only → use country's region
/// 4. No metadata → first favourite, then Global
pub fn resolve_auto(file: Option<&LoadedFile>, favourites: &[BatBookRegion]) -> AutoResolved {
    let fallback_region = favourites.first().copied().unwrap_or(BatBookRegion::Global);

    let Some(file) = file else {
        let is_fav = favourites.first().is_some();
        return AutoResolved {
            region: fallback_region,
            matched_species_id: None,
            source_label: fallback_region.short_label().to_string(),
            from_favourite: is_fav,
            recording_place: None,
        };
    };

    let scientific_name = get_scientific_name(file);
    let place = recording_place(file);
    let country = place.as_ref().map(|p| p.country.clone());
    let country_match = place
        .as_ref()
        .and_then(|p| place_to_region(&p.country, p.area));
    // Kept only when the region comes from the recording's country.
    let recording_place = country_match.and(place);
    let country_region = country_match.map(|m| m.region);
    // Marker appended to the country in the source label when the country was
    // routed to a region only APPROXIMATELY (no dedicated/continental book).
    let approx_suffix = if country_match.is_some_and(|m| m.approximate) {
        ", approx."
    } else {
        ""
    };

    // Try species lookup
    if let Some(ref sci) = scientific_name {
        if let Some((region, species_id)) = find_species_across_books(sci, country_region) {
            // Use the country's region if available, otherwise use the region where species was found
            let effective_region = country_region.unwrap_or(region);
            // But verify the species exists in the effective region too
            let (final_region, final_id) = if effective_region != region {
                // Check if species is also in the country's region
                if let Some((_, id)) = find_species_across_books(sci, Some(effective_region)) {
                    (effective_region, id)
                } else {
                    // Species not in the country's region — show country's book anyway,
                    // but still highlight the species from whichever book had it
                    (effective_region, species_id)
                }
            } else {
                (region, species_id)
            };
            let label = if let Some(ref cnt) = country {
                format!("{} ({}{})", final_region.short_label(), cnt, approx_suffix)
            } else {
                final_region.short_label().to_string()
            };
            return AutoResolved {
                region: final_region,
                matched_species_id: Some(final_id),
                source_label: label,
                from_favourite: false,
                recording_place: country_region.and(recording_place),
            };
        }
    }

    // No species match — try country only
    if let Some(region) = country_region {
        let label = if let Some(ref cnt) = country {
            format!("{} ({}{})", region.short_label(), cnt, approx_suffix)
        } else {
            region.short_label().to_string()
        };
        return AutoResolved {
            region,
            matched_species_id: None,
            source_label: label,
            from_favourite: false,
            recording_place,
        };
    }

    // Nothing useful — fallback
    let is_fav = favourites.first().is_some();
    AutoResolved {
        region: fallback_region,
        matched_species_id: None,
        source_label: fallback_region.short_label().to_string(),
        from_favourite: is_fav,
        recording_place: None,
    }
}

/// Find the BatBookEntry for a species id in a given region's manifest.
/// Used by the strip to render the auto-matched species chip.
pub fn find_entry_in_manifest(
    region: BatBookRegion,
    species_id: &str,
) -> Option<super::types::BatBookEntry> {
    let manifest = get_manifest(region);
    manifest.entries.into_iter().find(|e| e.id == species_id)
}

/// Try to find the entry across ALL books (for when the matched species
/// came from a different region than the displayed one).
pub fn find_entry_any_book(species_id: &str) -> Option<super::types::BatBookEntry> {
    for &region in BatBookRegion::ALL {
        let manifest = get_manifest(region);
        if let Some(entry) = manifest.entries.into_iter().find(|e| e.id == species_id) {
            return Some(entry);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Metadata (Xeno-canto included) often uses an older genus; it should
    /// still find the merged species.
    #[test]
    fn synonyms_find_the_merged_species() {
        for (name, id) in [
            ("Lasiurus cinereus", "aeorestes_cinereus"),
            ("Chaerephon pumilus", "mops_pumilus"),
            ("Lasiurus ega", "dasypterus_ega"),
            ("Lophostoma silvicolum", "lophostoma_silvicola"),
            ("Lasiurus cinereus semotus", "aeorestes_cinereus"),
        ] {
            let found = find_species_across_books(name, None).map(|(_, id)| id);
            assert_eq!(found.as_deref(), Some(id), "{name}");
        }
    }

    /// A subspecies with its own entry finds that entry, not its species,
    /// whichever region the recording is from.
    #[test]
    fn a_whole_name_match_beats_a_binomial_match() {
        let name = "Rhinolophus ferrumequinum nippon";
        for region in [
            None,
            Some(BatBookRegion::Europe),
            Some(BatBookRegion::SoutheastAsia),
            Some(BatBookRegion::Australia),
            Some(BatBookRegion::Japan),
        ] {
            assert_eq!(
                find_species(name, region).map(|s| s.id.as_str()),
                Some("rhinolophus_ferrumequinum_nippon"),
                "{region:?}"
            );
            assert_eq!(
                find_species_across_books(name, region).map(|(_, id)| id),
                Some("rhinolophus_ferrumequinum_nippon".to_string()),
                "{region:?}"
            );
        }
        // A trinomial with no entry of its own still finds its species.
        assert_eq!(
            find_species("Rhinolophus ferrumequinum creticum", None).map(|s| s.id.as_str()),
            Some("rhinolophus_ferrumequinum")
        );
        // Extra spaces between the words don't matter.
        assert_eq!(
            find_species(" Pipistrellus  pipistrellus ", None).map(|s| s.id.as_str()),
            Some("pipistrellus_pipistrellus")
        );
    }

    #[test]
    fn positions_find_regions() {
        // Victoria has a book of its own; the rest of Australia has one book.
        assert_eq!(
            region_at(-37.81, 144.96),
            PositionRegion::Found(BatBookRegion::VicAustralia, "Australia".to_string())
        );
        assert_eq!(
            region_at(-33.87, 151.21),
            PositionRegion::Found(BatBookRegion::Australia, "Australia".to_string())
        );
        let found = |lat, lon| match region_at(lat, lon) {
            PositionRegion::Found(r, _) => Some(r),
            _ => None,
        };
        assert_eq!(found(52.48, -1.90), Some(BatBookRegion::UK));
        assert_eq!(found(37.98, 23.73), Some(BatBookRegion::Greece));
        assert_eq!(found(9.93, -84.08), Some(BatBookRegion::CostaRica));
        assert_eq!(region_at(0.0, -30.0), PositionRegion::NoCountry);
    }

    /// Every country in the outlines should reach some region, so a GPS
    /// position on land never falls back to a favourite or Global unnoticed.
    #[test]
    fn every_outlined_country_has_a_region() {
        let missing: Vec<&str> = countries::all_country_names()
            .filter(|names| !names.iter().any(|n| country_to_region(n).is_some()))
            .map(|names| names[0].as_str())
            .collect();
        assert!(missing.len() < 200, "outlines not loaded");
        assert!(missing.is_empty(), "no region for: {missing:?}");
    }
}

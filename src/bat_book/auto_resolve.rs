use super::country_map::country_to_region;
use super::data::{get_manifest, species, species_by_scientific_name, NameQuery};
use super::types::{AutoResolved, BatBookRegion, BatSpecies, PlaceSource, RecordingPlace};
use crate::state::LoadedFile;

/// Search all regional bat books for a species by scientific name.
/// Returns `(region, species_id)` for the best match.
/// Prefers `preferred_region` if the species appears there; otherwise picks the
/// first non-Global match.
fn find_species_across_books(
    scientific_name: &str,
    preferred_region: Option<BatBookRegion>,
) -> Option<(BatBookRegion, String)> {
    // Also matches the binomial of a trinomial ("Myotis lucifugus alascensis").
    let query = NameQuery::new(scientific_name)?;

    let mut best: Option<(BatBookRegion, String)> = None;

    for &region in BatBookRegion::ALL {
        if region == BatBookRegion::Global {
            continue; // Global only has family-level entries, skip
        }
        let manifest = get_manifest(region);
        for entry in &manifest.entries {
            if species(entry.id).is_some_and(|s| query.matches(s)) {
                // Exact match in this region
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
/// looked up among every species instead.
pub(crate) fn find_species(
    scientific_name: &str,
    preferred_region: Option<BatBookRegion>,
) -> Option<&'static BatSpecies> {
    find_species_across_books(scientific_name, preferred_region)
        .and_then(|(_, id)| species(&id))
        .or_else(|| species_by_scientific_name(scientific_name))
}

/// The region of the recording's country (source site or GPS position), if
/// the region map knows it.
pub(crate) fn country_region(file: &LoadedFile) -> Option<BatBookRegion> {
    let place = get_country(file)?;
    country_to_region(&place.country).map(|m| m.region)
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

/// The recording's country and where it came from: the source site's
/// "Country" (Xeno-canto, iNaturalist, ...), or else the country at its GPS
/// position (GUANO or source-site coordinates),
/// looked up offline. Of a country's name variants, the one the region map
/// knows is used.
fn get_country(file: &LoadedFile) -> Option<RecordingPlace> {
    let from_site = file.site_metadata.as_ref().and_then(|m| {
        let country = m
            .fields
            .iter()
            .find(|(k, _)| k == "Country")
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())?;
        Some((m.site.clone(), country))
    });
    // A source-site country the region map doesn't know falls through to the
    // coordinates.
    if let Some((site, country)) = from_site.filter(|(_, c)| country_to_region(c).is_some()) {
        return Some(RecordingPlace {
            source: PlaceSource::Metadata { site },
            country,
        });
    }
    let place = crate::classifier::place::file_place(file)?;
    let names = super::countries::country_names_at(place.lat, place.lon)?;
    let country = names
        .iter()
        .find(|n| country_to_region(n).is_some())
        .unwrap_or(&names[0])
        .clone();
    Some(RecordingPlace {
        source: PlaceSource::Gps {
            lat: place.lat,
            lon: place.lon,
        },
        country,
    })
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
    let Some(names) = super::countries::country_names_at(lat, lon) else {
        return PositionRegion::NoCountry;
    };
    match names
        .iter()
        .find_map(|n| country_to_region(n).map(|m| (m.region, n)))
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
    let place = get_country(file);
    let country = place.as_ref().map(|p| p.country.clone());
    let country_match = country.as_deref().and_then(country_to_region);
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

    #[test]
    fn positions_find_regions() {
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
        let missing: Vec<&str> = super::super::countries::all_country_names()
            .filter(|names| !names.iter().any(|n| country_to_region(n).is_some()))
            .map(|names| names[0].as_str())
            .collect();
        assert!(missing.len() < 200, "outlines not loaded");
        assert!(missing.is_empty(), "no region for: {missing:?}");
    }
}

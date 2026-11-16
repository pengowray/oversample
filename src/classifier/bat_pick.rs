//! Which regional classifier BattyBirdNET uses: one picked in the tab, or with
//! "Auto" the one for the place the recording was made.

use super::BatScope;
use crate::bat_book::auto_resolve;
use crate::bat_book::country_map::country_to_region;
use crate::bat_book::types::BatBookRegion;
use crate::state::store_fields::*;
use crate::state::AppState;
use crate::state::LoadedFile;
use leptos::prelude::*;
use oversample_ipc::classifier::BatRegion;

/// What is picked in the tab's Region list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BatPick {
    /// The region for the recording's place ([`auto_scope`]).
    #[default]
    Auto,
    Fixed(BatScope),
}

/// The classifier for a recording made in a bat book region, which is as fine
/// as a country ([`finer_scope`] narrows it with a position). A region with
/// no classifier of its own gets all regions.
pub fn scope_for_book_region(region: BatBookRegion) -> BatScope {
    match region {
        BatBookRegion::UK => BatScope::Region(BatRegion::Uk),
        BatBookRegion::Sweden => BatScope::Region(BatRegion::Sweden),
        BatBookRegion::Europe | BatBookRegion::Greece | BatBookRegion::Netherlands => {
            BatScope::Region(BatRegion::Eu)
        }
        // Canada's bats are all in the USA classifier.
        BatBookRegion::UnitedStates | BatBookRegion::Canada => BatScope::Region(BatRegion::Usa),
        _ => BatScope::All,
    }
}

/// What "Auto" gives a recording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoScope {
    pub scope: BatScope,
    /// Where the choice came from, to show under the Region list, e.g. "From
    /// GPS position 52.48, -1.90".
    pub note: String,
}

/// The classifier "Auto" gives `file`: the one for its country (from the
/// source site, or its GPS position, as the bat book works it out), narrowed
/// to a smaller region when it has a position, or all regions for a
/// recording with no place or a place with no classifier.
pub fn auto_scope(file: Option<&LoadedFile>) -> AutoScope {
    let Some(place) = file.and_then(auto_resolve::get_country) else {
        return AutoScope {
            scope: BatScope::All,
            note: "This recording has no location".to_string(),
        };
    };
    let book = country_to_region(&place.country).map(|m| m.region);
    let Some((book, country_scope @ BatScope::Region(_))) =
        book.map(|b| (b, scope_for_book_region(b)))
    else {
        return AutoScope {
            scope: BatScope::All,
            note: format!("No BattyBirdNET region for {}", place.country),
        };
    };
    // With a position, a smaller region inside the country.
    let position = file
        .and_then(super::place::file_place)
        .map(|p| (p.lat, p.lon));
    match position.map(|(lat, lon)| {
        (
            lat,
            lon,
            finer_scope(country_scope, &place.country, lat, lon),
        )
    }) {
        Some((lat, lon, scope)) if scope != country_scope => AutoScope {
            scope,
            note: format!("From GPS position {lat:.2}, {lon:.2}"),
        },
        _ => AutoScope {
            scope: country_scope,
            note: place.detail(book),
        },
    }
}

/// The smaller region a position is in, for the countries that have one:
/// Scotland or South Wales in the UK, Bavaria in Germany, and the USA's (or
/// Canada's) east or west. The outlines are rough boxes and lines, a few tens
/// of km out in places; a position outside them keeps the country's region.
fn finer_scope(country_scope: BatScope, country: &str, lat: f32, lon: f32) -> BatScope {
    let region = match country_scope {
        BatScope::Region(BatRegion::Uk) => {
            // The border runs from the Solway Firth (54.98 N, 3.05 W) to
            // Berwick (55.80 N, 2.03 W). Northern Ireland is west of 5.4 W.
            let border = 54.98 + 0.8 * (lon.clamp(-3.05, -2.03) + 3.05);
            let northern_ireland = lon < -5.4 && lat < 55.35;
            if lat > border && !northern_ireland {
                BatRegion::Scotland
            } else if (51.35..52.1).contains(&lat) && (-5.4..-2.65).contains(&lon) {
                BatRegion::SouthWales
            } else {
                BatRegion::Uk
            }
        }
        BatScope::Region(BatRegion::Eu) if country.trim().eq_ignore_ascii_case("germany") => {
            let south = (47.2..49.7).contains(&lat) && (10.1..13.9).contains(&lon);
            let north = (49.7..50.6).contains(&lat) && (8.95..12.3).contains(&lon);
            if south || north {
                BatRegion::Bavaria
            } else {
                BatRegion::Eu
            }
        }
        // East and west of 100 W. Alaska and Hawaii keep the whole-USA list.
        BatScope::Region(BatRegion::Usa) if lon >= -100.0 => BatRegion::UsaEast,
        BatScope::Region(BatRegion::Usa) if lon >= -130.0 => BatRegion::UsaWest,
        _ => return country_scope,
    };
    BatScope::Region(region)
}

/// [`auto_scope`] for the current file. Reactive.
pub fn current_auto_scope(state: AppState) -> AutoScope {
    let idx = state.library.current_index().get();
    state
        .library
        .files()
        .with(|files| auto_scope(idx.and_then(|i| files.get(i))))
}

/// The classifier BattyBirdNET uses for the current file now. Reactive.
pub fn bat_scope(state: AppState) -> BatScope {
    match state.call_detection.bat_pick().get() {
        BatPick::Fixed(scope) => scope,
        BatPick::Auto => current_auto_scope(state).scope,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn for_country(country: &str) -> BatScope {
        country_to_region(country).map_or(BatScope::All, |m| scope_for_book_region(m.region))
    }

    #[test]
    fn positions_get_the_smaller_region() {
        let uk = BatScope::Region(BatRegion::Uk);
        let eu = BatScope::Region(BatRegion::Eu);
        let usa = BatScope::Region(BatRegion::Usa);
        let at = |scope, country, lat, lon| match finer_scope(scope, country, lat, lon) {
            BatScope::Region(r) => r,
            BatScope::All => panic!("all regions"),
        };
        assert_eq!(at(uk, "United Kingdom", 55.95, -3.19), BatRegion::Scotland); // Edinburgh
        assert_eq!(at(uk, "United Kingdom", 57.48, -4.22), BatRegion::Scotland); // Inverness
        assert_eq!(at(uk, "United Kingdom", 54.97, -1.61), BatRegion::Uk); // Newcastle
        assert_eq!(at(uk, "United Kingdom", 55.77, -2.00), BatRegion::Uk); // Berwick
        assert_eq!(at(uk, "United Kingdom", 54.60, -5.93), BatRegion::Uk); // Belfast
        assert_eq!(
            at(uk, "United Kingdom", 51.48, -3.18),
            BatRegion::SouthWales
        ); // Cardiff
        assert_eq!(at(uk, "United Kingdom", 51.45, -2.59), BatRegion::Uk); // Bristol
        assert_eq!(at(uk, "United Kingdom", 53.14, -4.27), BatRegion::Uk); // Caernarfon
        assert_eq!(at(eu, "Germany", 48.14, 11.58), BatRegion::Bavaria); // Munich
        assert_eq!(at(eu, "Germany", 49.79, 9.93), BatRegion::Bavaria); // Würzburg
        assert_eq!(at(eu, "Germany", 48.78, 9.18), BatRegion::Eu); // Stuttgart
        assert_eq!(at(eu, "Germany", 52.52, 13.40), BatRegion::Eu); // Berlin
        assert_eq!(at(eu, "Austria", 47.80, 13.04), BatRegion::Eu); // Salzburg
        assert_eq!(at(usa, "United States", 40.71, -74.01), BatRegion::UsaEast); // New York
        assert_eq!(at(usa, "United States", 39.74, -104.99), BatRegion::UsaWest); // Denver
        assert_eq!(at(usa, "Canada", 49.28, -123.12), BatRegion::UsaWest); // Vancouver
        assert_eq!(at(usa, "United States", 61.22, -149.90), BatRegion::Usa); // Anchorage
        assert_eq!(
            finer_scope(BatScope::Region(BatRegion::Sweden), "Sweden", 59.33, 18.07),
            BatScope::Region(BatRegion::Sweden)
        );
    }

    #[test]
    fn countries_get_their_classifier() {
        assert_eq!(
            for_country("United Kingdom"),
            BatScope::Region(BatRegion::Uk)
        );
        assert_eq!(for_country("Sweden"), BatScope::Region(BatRegion::Sweden));
        assert_eq!(for_country("Germany"), BatScope::Region(BatRegion::Eu));
        assert_eq!(for_country("Greece"), BatScope::Region(BatRegion::Eu));
        assert_eq!(
            for_country("United States"),
            BatScope::Region(BatRegion::Usa)
        );
        assert_eq!(for_country("Canada"), BatScope::Region(BatRegion::Usa));
        assert_eq!(for_country("Australia"), BatScope::All);
        assert_eq!(for_country("Costa Rica"), BatScope::All);
        assert_eq!(for_country("Atlantis"), BatScope::All);
        let none = auto_scope(None);
        assert_eq!(none.scope, BatScope::All);
        assert_eq!(none.note, "This recording has no location");
    }
}

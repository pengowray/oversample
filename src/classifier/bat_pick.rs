//! Which regional classifier BattyBirdNET uses: one picked in the tab, or with
//! "Auto" the one for the place the recording was made.

use super::BatScope;
use crate::bat_book::country_map::country_to_region;
use crate::bat_book::types::BatBookRegion;
use crate::recording_place::{recording_place, Area};
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
/// as a country ([`scope_for_area`] is for a smaller area). A region with no
/// classifier of its own gets all regions.
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

/// The classifier for an area inside a country, if it has one of its own.
fn scope_for_area(area: Area) -> Option<BatScope> {
    let region = match area {
        Area::Scotland => BatRegion::Scotland,
        Area::SouthWales => BatRegion::SouthWales,
        Area::Bavaria => BatRegion::Bavaria,
        Area::NorthAmericaEast => BatRegion::UsaEast,
        Area::NorthAmericaWest => BatRegion::UsaWest,
        Area::Victoria => return None,
    };
    Some(BatScope::Region(region))
}

/// What "Auto" gives a recording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoScope {
    pub scope: BatScope,
    /// Where the choice came from, to show under the Region list, e.g. "From
    /// GPS position 52.48, -1.90".
    pub note: String,
}

/// The classifier "Auto" gives `file`: the one for the area its position is
/// in (Scotland, Bavaria, ...), or else for its country, or all regions for a
/// recording with no place or a place with no classifier.
pub fn auto_scope(file: Option<&LoadedFile>) -> AutoScope {
    let Some(place) = file.and_then(recording_place) else {
        return AutoScope {
            scope: BatScope::All,
            note: "This recording has no location".to_string(),
        };
    };
    if let (Some(scope), Some(position)) =
        (place.area.and_then(scope_for_area), place.position_phrase())
    {
        return AutoScope {
            scope,
            note: format!("From {position}"),
        };
    }
    let book = country_to_region(&place.country).map(|m| m.region);
    match book.map(|b| (b, scope_for_book_region(b))) {
        Some((b, scope @ BatScope::Region(_))) => AutoScope {
            scope,
            note: place.detail(b.label()),
        },
        _ => AutoScope {
            scope: BatScope::All,
            note: format!("No BattyBirdNET region for {}", place.country),
        },
    }
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
    fn areas_get_their_classifier() {
        assert_eq!(
            scope_for_area(Area::Scotland),
            Some(BatScope::Region(BatRegion::Scotland))
        );
        assert_eq!(
            scope_for_area(Area::NorthAmericaWest),
            Some(BatScope::Region(BatRegion::UsaWest))
        );
        assert_eq!(scope_for_area(Area::Victoria), None);
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

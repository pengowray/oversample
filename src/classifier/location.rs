//! The location list: which birds the BirdNET+ Geomodel expects at a
//! recording's place and week, so BirdNET's and Perch's results can mark the
//! unlikely ones. Birds only: in `research/perch.md`'s tests the Geomodel's
//! insect ranges would have marked correct results. Runs on the device; the
//! place is never sent anywhere.

use super::place::{file_place, Place};
use super::{catalog, store, worker, ClassifierError, Method, SpeciesLabel};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

/// A bird scoring below this at the place and week is marked unlikely there.
/// Upstream's region lists use 0.01; at 0.03 the tests lost true species.
pub const MIN_OCCURRENCE: f32 = 0.01;

/// The location list for one file.
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub place: Place,
    /// The Geomodel's score per species, in its labels order; `None` while it
    /// is worked out, or if that failed.
    pub scores: Option<Arc<Vec<f32>>>,
}

/// One of the Geomodel's species.
#[derive(Clone, Debug)]
pub struct GeoSpecies {
    pub scientific: String,
    pub english: String,
    /// Birds have an eBird code; the others a number.
    pub bird: bool,
}

struct GeoNames {
    species: Vec<GeoSpecies>,
    by_name: HashMap<String, u32>,
}

thread_local! {
    static GEO: RefCell<Option<Arc<GeoNames>>> = const { RefCell::new(None) };
    static CACHE: RefCell<HashMap<(i32, i32, u32), Arc<Vec<f32>>>> = RefCell::new(HashMap::new());
    static NEXT_REQ: Cell<u32> = const { Cell::new(1) };
}

/// Parse the Geomodel's labels file: code, scientific name, English name.
fn parse_geo_labels(text: &str) -> GeoNames {
    let species: Vec<GeoSpecies> = text
        .lines()
        .map(|l| {
            let mut f = l.split('\t');
            let code = f.next().unwrap_or("").trim();
            GeoSpecies {
                bird: !code.is_empty() && !code.chars().all(|c| c.is_ascii_digit()),
                scientific: f.next().unwrap_or("").trim().to_string(),
                english: f.next().unwrap_or("").trim().to_string(),
            }
        })
        .collect();
    let by_name = species
        .iter()
        .enumerate()
        .map(|(i, s)| (s.scientific.clone(), i as u32))
        .collect();
    GeoNames { species, by_name }
}

async fn geo_names() -> Result<Arc<GeoNames>, ClassifierError> {
    if let Some(g) = GEO.with(|g| g.borrow().clone()) {
        return Ok(g);
    }
    let bytes = store::read(&catalog::GEOMODEL_LABELS).await?;
    let g = Arc::new(parse_geo_labels(&String::from_utf8_lossy(&bytes)));
    GEO.with(|c| *c.borrow_mut() = Some(g.clone()));
    Ok(g)
}

/// Methods whose results can be marked.
pub fn applies_to(method: Method) -> bool {
    matches!(method, Method::BirdNet | Method::Perch)
}

/// Work out the location list for file `file_id` if its metadata has a place,
/// downloading the Geomodel (16 MB) the first time. Does nothing when it is
/// already worked out for the same place and week.
pub fn update(state: AppState, file_id: u64) {
    let place = state
        .library
        .files()
        .with_untracked(|files| files.iter().find(|f| f.id == file_id).and_then(file_place));
    let Some(place) = place else {
        state.classifier.occurrence().update(|m| {
            m.remove(&file_id);
        });
        return;
    };
    let have = state
        .classifier
        .occurrence()
        .with_untracked(|m| m.get(&file_id).map(|o| o.place));
    if have == Some(place) {
        return;
    }
    let cached = CACHE.with(|c| c.borrow().get(&place.key()).cloned());
    state.classifier.occurrence().update(|m| {
        m.insert(
            file_id,
            Occurrence {
                place,
                scores: cached.clone(),
            },
        );
    });
    if cached.is_some() {
        return;
    }
    spawn_local(async move {
        match scores(state, place).await {
            Ok(s) => {
                CACHE.with(|c| c.borrow_mut().insert(place.key(), s.clone()));
                state.classifier.occurrence().update(|m| {
                    if let Some(o) = m.get_mut(&file_id).filter(|o| o.place == place) {
                        o.scores = Some(s);
                    }
                });
            }
            Err(e) => log::warn!("location list: {e}"),
        }
    });
}

async fn scores(state: AppState, place: Place) -> Result<Arc<Vec<f32>>, ClassifierError> {
    for f in catalog::GEOMODEL_FILES {
        super::download_file(state, f).await?;
    }
    geo_names().await?;
    let bytes = store::read_js(&catalog::GEOMODEL).await?;
    let stopped = |s: worker::Stopped| match s {
        worker::Stopped::Failed(e) => e,
        worker::Stopped::Superseded => ClassifierError::Worker {
            message: "the worker stopped".into(),
        },
    };
    worker::ensure_started(state).await.map_err(stopped)?;
    let req = NEXT_REQ.with(|n| n.replace(n.get().wrapping_add(1)));
    let waiter =
        worker::occurrence(req, place.lat, place.lon, place.week, &bytes).map_err(stopped)?;
    match worker::wait(waiter).await.map_err(stopped)? {
        worker::Reply::Occurrence(s) => Ok(Arc::new(s)),
        _ => Err(ClassifierError::Worker {
            message: "unexpected reply".into(),
        }),
    }
}

/// Whether `label` (a class of a BirdNET or Perch run) is a bird the
/// Geomodel scores below [`MIN_OCCURRENCE`] at the place: `Some(true)` unlikely,
/// `Some(false)` expected, `None` when it can't say (not a bird, a class the
/// Geomodel doesn't know, or no scores yet).
pub fn unlikely(occurrence: &Occurrence, label: &SpeciesLabel) -> Option<bool> {
    let scores = occurrence.scores.as_ref()?;
    let geo = GEO.with(|g| g.borrow().clone())?;
    let i = *geo.by_name.get(&label.scientific)? as usize;
    if !geo.species.get(i)?.bird {
        return None;
    }
    Some(*scores.get(i)? < MIN_OCCURRENCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_parse() {
        let g = parse_geo_labels(
            "100034\tEpiaeschna heros\tSwamp Darner\ncomswi\tApus apus\tCommon Swift\n",
        );
        assert!(!g.species[0].bird);
        assert!(g.species[1].bird);
        assert_eq!(g.by_name["Apus apus"], 1);
        assert_eq!(g.species[1].english, "Common Swift");
    }
}

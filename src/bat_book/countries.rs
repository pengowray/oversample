//! Which country a GPS position is in, offline, from Natural Earth's 1:110m
//! country outlines (public domain), built into the app by
//! `scripts/make-countries.py` as `src/data/countries-110m.json` (136 KB).
//! At this scale borders are rough to a few km, and small islands and
//! territories may be missing; a position near a border or off the coast may
//! find no country, or the neighbour.

use std::sync::OnceLock;

struct Country {
    /// English name variants: NAME, NAME_LONG, ADMIN, NAME_EN.
    names: Vec<String>,
    /// Rings as flat [lon, lat, lon, lat, ...].
    rings: Vec<Vec<f32>>,
    /// lon_min, lat_min, lon_max, lat_max over all rings.
    bbox: [f32; 4],
}

fn countries() -> &'static [Country] {
    static C: OnceLock<Vec<Country>> = OnceLock::new();
    C.get_or_init(|| {
        let raw: Vec<(Vec<String>, Vec<Vec<f32>>)> =
            serde_json::from_str(include_str!("../data/countries-110m.json")).unwrap_or_default();
        raw.into_iter()
            .map(|(names, rings)| {
                let mut bbox = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                for r in &rings {
                    for p in r.chunks_exact(2) {
                        bbox[0] = bbox[0].min(p[0]);
                        bbox[1] = bbox[1].min(p[1]);
                        bbox[2] = bbox[2].max(p[0]);
                        bbox[3] = bbox[3].max(p[1]);
                    }
                }
                Country { names, rings, bbox }
            })
            .collect()
    })
}

/// Even-odd point in polygon over all of a country's rings (holes are rings
/// too, so a point in a hole crosses an even number of edges).
fn contains(c: &Country, lon: f32, lat: f32) -> bool {
    if lon < c.bbox[0] || lon > c.bbox[2] || lat < c.bbox[1] || lat > c.bbox[3] {
        return false;
    }
    let mut inside = false;
    for ring in &c.rings {
        let n = ring.len() / 2;
        let mut j = n.wrapping_sub(1);
        for i in 0..n {
            let (xi, yi) = (ring[2 * i], ring[2 * i + 1]);
            let (xj, yj) = (ring[2 * j], ring[2 * j + 1]);
            if (yi > lat) != (yj > lat) && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi {
                inside = !inside;
            }
            j = i;
        }
    }
    inside
}

/// The English name variants of the country at `lat`, `lon`, if any.
pub fn country_names_at(lat: f32, lon: f32) -> Option<&'static [String]> {
    countries()
        .iter()
        .find(|c| contains(c, lon, lat))
        .map(|c| c.names.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(lat: f32, lon: f32) -> Option<&'static str> {
        country_names_at(lat, lon).map(|n| n[0].as_str())
    }

    #[test]
    fn known_places() {
        assert_eq!(first(52.48, -1.90), Some("United Kingdom"));
        assert_eq!(first(37.98, 23.73), Some("Greece"));
        assert_eq!(first(-33.87, 151.21), Some("Australia"));
        assert_eq!(first(5.24, -76.09), Some("Colombia"));
        assert_eq!(first(0.0, -30.0), None);
    }
}

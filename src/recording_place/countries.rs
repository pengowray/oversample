//! Which country a GPS position is in, offline, from Natural Earth's 1:50m
//! country outlines (public domain), built into the app by
//! `scripts/make-countries.py` as `src/data/countries-50m.bin` (409 KB; the
//! script describes the format). Points are rounded to 0.01 degrees, so a
//! position within about 1 km of a coast or border may find no country, or the
//! neighbour; islands smaller than a few km may be missing.

use std::sync::OnceLock;

struct Country {
    /// English name variants: NAME, NAME_LONG, ADMIN, NAME_EN.
    names: Vec<String>,
    /// Rings as flat [lon, lat, lon, lat, ...] in degrees.
    rings: Vec<Vec<f32>>,
    /// lon_min, lat_min, lon_max, lat_max over all rings.
    bbox: [f32; 4],
}

/// Reads the little-endian outline file; None if it is cut short.
fn parse(mut b: &[u8]) -> Option<Vec<Country>> {
    fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
        let (head, rest) = (b.get(..n)?, b.get(n..)?);
        *b = rest;
        Some(head)
    }
    let u16_at = |b: &mut &[u8]| take(b, 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let count = u16_at(&mut b)?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let n_names = take(&mut b, 1)?[0];
        let mut names = Vec::with_capacity(n_names as usize);
        for _ in 0..n_names {
            let len = take(&mut b, 1)?[0] as usize;
            names.push(String::from_utf8_lossy(take(&mut b, len)?).into_owned());
        }
        let n_rings = u16_at(&mut b)?;
        let mut rings = Vec::with_capacity(n_rings as usize);
        let mut bbox = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for _ in 0..n_rings {
            let s = take(&mut b, 4)?;
            let n = u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize;
            let raw = take(&mut b, n * 4)?;
            let ring: Vec<f32> = raw
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 100.0)
                .collect();
            for p in ring.chunks_exact(2) {
                bbox[0] = bbox[0].min(p[0]);
                bbox[1] = bbox[1].min(p[1]);
                bbox[2] = bbox[2].max(p[0]);
                bbox[3] = bbox[3].max(p[1]);
            }
            rings.push(ring);
        }
        out.push(Country { names, rings, bbox });
    }
    Some(out)
}

fn countries() -> &'static [Country] {
    static C: OnceLock<Vec<Country>> = OnceLock::new();
    C.get_or_init(|| parse(include_bytes!("../data/countries-50m.bin")).unwrap_or_default())
}

/// Every country's English name variants, for tests of the region map.
pub fn all_country_names() -> impl Iterator<Item = &'static [String]> {
    countries().iter().map(|c| c.names.as_slice())
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

/// How far off a country's outline a position may be and still count as in
/// it: 0.15 degrees of latitude, about 17 km. Recordings are often made on a
/// shore or a small island that the 1:50m outline misses.
const NEAR_DEG: f32 = 0.15;

/// Squared distance, in degrees of latitude, from a position to the nearest
/// edge of a country's outline. Longitude is scaled by cos(latitude).
fn distance2(c: &Country, lon: f32, lat: f32) -> f32 {
    let k = lat.to_radians().cos();
    let mut best = f32::MAX;
    for ring in &c.rings {
        for seg in ring.windows(4).step_by(2) {
            let (ax, ay) = ((seg[0] - lon) * k, seg[1] - lat);
            let (bx, by) = ((seg[2] - lon) * k, seg[3] - lat);
            let (dx, dy) = (bx - ax, by - ay);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 {
                (-(ax * dx + ay * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (px, py) = (ax + t * dx, ay + t * dy);
            best = best.min(px * px + py * py);
        }
    }
    best
}

/// The English name variants of the country at `lat`, `lon`: the country
/// whose outline contains it, or else the nearest within `NEAR_DEG`.
pub fn country_names_at(lat: f32, lon: f32) -> Option<&'static [String]> {
    let all = countries();
    if let Some(c) = all.iter().find(|c| contains(c, lon, lat)) {
        return Some(&c.names);
    }
    let margin_lon = NEAR_DEG / lat.to_radians().cos().max(0.05);
    all.iter()
        .filter(|c| {
            lon >= c.bbox[0] - margin_lon
                && lon <= c.bbox[2] + margin_lon
                && lat >= c.bbox[1] - NEAR_DEG
                && lat <= c.bbox[3] + NEAR_DEG
        })
        .map(|c| (distance2(c, lon, lat), c))
        .filter(|(d2, _)| *d2 <= NEAR_DEG * NEAR_DEG)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| c.names.as_slice())
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

    /// Coasts and islands, weighted to the countries with their own books.
    #[test]
    fn coasts_and_islands() {
        let points: &[(&str, f32, f32, &str)] = &[
            ("Stockholm", 59.33, 18.07, "Sweden"),
            ("Gotland", 57.64, 18.30, "Sweden"),
            ("Amsterdam", 52.37, 4.90, "Netherlands"),
            ("Texel", 53.05, 4.80, "Netherlands"),
            ("Corfu", 39.62, 19.92, "Greece"),
            ("Rhodes", 36.43, 28.22, "Greece"),
            ("Crete", 35.34, 25.13, "Greece"),
            ("Brighton", 50.82, -0.14, "United Kingdom"),
            ("Orkney", 58.98, -2.96, "United Kingdom"),
            ("Mombasa", -4.04, 39.67, "Kenya"),
            ("Vancouver Island", 48.43, -123.37, "Canada"),
            ("Miami", 25.76, -80.19, "United States"),
            ("Okinawa", 26.21, 127.68, "Japan"),
            ("Melbourne", -37.81, 144.96, "Australia"),
            ("Hobart", -42.88, 147.33, "Australia"),
            ("Singapore", 1.35, 103.82, "Singapore"),
            ("Malta", 35.90, 14.51, "Malta"),
            ("Hong Kong", 22.32, 114.17, "Hong Kong"),
            ("Puerto Limon", 9.99, -83.03, "Costa Rica"),
            ("Veracruz", 19.17, -96.13, "Mexico"),
        ];
        let misses: Vec<String> = points
            .iter()
            .filter(|(_, lat, lon, want)| {
                !country_names_at(*lat, *lon).is_some_and(|n| n.iter().any(|x| x == want))
            })
            .map(|(name, lat, lon, _)| format!("{name}: {:?}", first(*lat, *lon)))
            .collect();
        assert!(misses.is_empty(), "{misses:?}");
    }
}

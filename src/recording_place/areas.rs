//! Areas inside a country that have a region of their own somewhere in the
//! app: a bat book (Victoria) or a BattyBirdNET classifier (the rest).
//!
//! The outlines are rough: a line, a box or two, or a polygon of a dozen
//! points, a few tens of km out in places. A position near an edge can land
//! on the wrong side.

/// An area inside a country.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Area {
    /// Victoria, Australia.
    Victoria,
    /// Scotland, UK.
    Scotland,
    /// South Wales, UK.
    SouthWales,
    /// Bavaria, Germany.
    Bavaria,
    /// The USA and Canada from 100 W eastwards.
    NorthAmericaEast,
    /// The USA and Canada from 100 W to 130 W (so not Alaska or Hawaii).
    NorthAmericaWest,
}

/// Victoria as [lon, lat] points: the Murray from the South Australian
/// border to its source, the straight border to Cape Howe, then out to sea
/// (the coast is the country outline's job) and back along 140.96 E.
const VICTORIA: &[[f32; 2]] = &[
    [140.96, -33.98],
    [141.90, -34.10],
    [142.16, -34.17], // Mildura
    [142.77, -34.58], // Robinvale
    [143.34, -35.03], // Tooleybuc
    [143.55, -35.34], // Swan Hill
    [144.13, -35.63], // Barham
    [144.75, -36.13], // Echuca
    [145.57, -35.81], // Tocumwal
    [146.00, -36.01], // Yarrawonga
    [146.92, -36.08], // Albury
    [147.70, -35.93], // Jingellic
    [147.98, -36.12], // Towong
    [148.13, -36.54], // Tom Groggin
    [148.19, -36.80], // The Murray's source
    [149.98, -37.51], // Cape Howe
    [149.98, -39.30],
    [140.96, -39.30],
];

/// Even-odd point in polygon; `ring` is [lon, lat] points.
fn in_polygon(ring: &[[f32; 2]], lat: f32, lon: f32) -> bool {
    let mut inside = false;
    let mut j = ring.len().wrapping_sub(1);
    for i in 0..ring.len() {
        let ([xi, yi], [xj, yj]) = (ring[i], ring[j]);
        if (yi > lat) != (yj > lat) && lon < (xj - xi) * (lat - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The outlined area of `country` (an English name, any case) that a position
/// is in, if any.
pub fn area_at(country: &str, lat: f32, lon: f32) -> Option<Area> {
    let country = country.trim().to_lowercase();
    match country.as_str() {
        "australia" => in_polygon(VICTORIA, lat, lon).then_some(Area::Victoria),
        "united kingdom" | "uk" => {
            // The border runs from the Solway Firth (54.98 N, 3.05 W) to
            // Berwick (55.80 N, 2.03 W). Northern Ireland is west of 5.4 W.
            let border = 54.98 + 0.8 * (lon.clamp(-3.05, -2.03) + 3.05);
            let northern_ireland = lon < -5.4 && lat < 55.35;
            if lat > border && !northern_ireland {
                Some(Area::Scotland)
            } else if (51.35..52.1).contains(&lat) && (-5.4..-2.65).contains(&lon) {
                Some(Area::SouthWales)
            } else {
                None
            }
        }
        "germany" => {
            let south = (47.2..49.7).contains(&lat) && (10.1..13.9).contains(&lon);
            let north = (49.7..50.6).contains(&lat) && (8.95..12.3).contains(&lon);
            (south || north).then_some(Area::Bavaria)
        }
        "united states" | "united states of america" | "usa" | "canada" => {
            if lon >= -100.0 {
                Some(Area::NorthAmericaEast)
            } else if lon >= -130.0 {
                Some(Area::NorthAmericaWest)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn victoria() {
        let vic = |lat, lon| area_at("Australia", lat, lon) == Some(Area::Victoria);
        assert!(vic(-37.81, 144.96)); // Melbourne
        assert!(vic(-36.76, 144.28)); // Bendigo
        assert!(vic(-36.12, 146.89)); // Wodonga
        assert!(vic(-37.56, 149.76)); // Mallacoota
        assert!(vic(-38.38, 141.60)); // Portland
        assert!(vic(-34.60, 141.50)); // Murray-Sunset
        assert!(vic(-39.03, 146.32)); // Wilsons Promontory
        assert!(!vic(-35.53, 144.96)); // Deniliquin, NSW
        assert!(!vic(-36.91, 149.24)); // Bombala, NSW
        assert!(!vic(-37.07, 149.90)); // Eden, NSW
        assert!(!vic(-35.28, 149.13)); // Canberra
        assert!(!vic(-33.87, 151.21)); // Sydney
        assert!(!vic(-37.83, 140.78)); // Mount Gambier, SA
        assert!(!vic(-39.93, 143.85)); // King Island, Tasmania
        assert!(!vic(-41.44, 147.14)); // Launceston
                                       // The outline is only looked at for Australia.
        assert_eq!(area_at("New Zealand", -37.81, 144.96), None);
    }

    #[test]
    fn areas_of_the_uk_germany_and_north_america() {
        let uk = |lat, lon| area_at("United Kingdom", lat, lon);
        assert_eq!(uk(55.95, -3.19), Some(Area::Scotland)); // Edinburgh
        assert_eq!(uk(57.48, -4.22), Some(Area::Scotland)); // Inverness
        assert_eq!(uk(54.97, -1.61), None); // Newcastle
        assert_eq!(uk(55.77, -2.00), None); // Berwick
        assert_eq!(uk(54.60, -5.93), None); // Belfast
        assert_eq!(uk(51.48, -3.18), Some(Area::SouthWales)); // Cardiff
        assert_eq!(uk(51.45, -2.59), None); // Bristol
        assert_eq!(uk(53.14, -4.27), None); // Caernarfon
        let de = |lat, lon| area_at("Germany", lat, lon);
        assert_eq!(de(48.14, 11.58), Some(Area::Bavaria)); // Munich
        assert_eq!(de(49.79, 9.93), Some(Area::Bavaria)); // Würzburg
        assert_eq!(de(48.78, 9.18), None); // Stuttgart
        assert_eq!(de(52.52, 13.40), None); // Berlin
        assert_eq!(area_at("Austria", 47.80, 13.04), None); // Salzburg
        let us = |lat, lon| area_at("United States", lat, lon);
        assert_eq!(us(40.71, -74.01), Some(Area::NorthAmericaEast)); // New York
        assert_eq!(us(39.74, -104.99), Some(Area::NorthAmericaWest)); // Denver
        assert_eq!(us(61.22, -149.90), None); // Anchorage
        assert_eq!(
            area_at("Canada", 49.28, -123.12),
            Some(Area::NorthAmericaWest)
        ); // Vancouver
        assert_eq!(area_at("Sweden", 59.33, 18.07), None);
    }
}

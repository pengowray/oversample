//! Positions written as text in recording metadata.

/// "lat lon", "lat,lon" or "lat, lon" as decimal degrees, in range; anything
/// after the two numbers is ignored. Exactly 0, 0 counts as no position:
/// detectors write it when they have no GPS fix.
pub fn parse_lat_lon(value: &str) -> Option<(f64, f64)> {
    let s = value.trim();
    let parts: Vec<&str> = if s.contains(',') {
        s.split(',').collect()
    } else {
        s.split_whitespace().collect()
    };
    if parts.len() < 2 {
        return None;
    }
    let lat: f64 = parts[0].trim().parse().ok()?;
    let lon: f64 = parts[1].trim().parse().ok()?;
    let no_fix = lat == 0.0 && lon == 0.0;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) && !no_fix)
        .then_some((lat, lon))
}

#[cfg(test)]
mod tests {
    use super::parse_lat_lon;

    #[test]
    fn decimal_pairs() {
        assert_eq!(parse_lat_lon("52.5 13.4"), Some((52.5, 13.4)));
        assert_eq!(parse_lat_lon("-33.9, 151.2"), Some((-33.9, 151.2)));
        assert_eq!(parse_lat_lon("-33.9,151.2,950"), Some((-33.9, 151.2)));
        assert_eq!(parse_lat_lon("95 13"), None);
        assert_eq!(parse_lat_lon("13 181"), None);
        assert_eq!(parse_lat_lon("0 0"), None);
        assert_eq!(parse_lat_lon("0.0, 0.0"), None);
        assert_eq!(parse_lat_lon("0, 13.4"), Some((0.0, 13.4)));
        assert_eq!(parse_lat_lon("north"), None);
        assert_eq!(parse_lat_lon("NaN, 13.4"), None);
    }
}

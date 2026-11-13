//! Where and when a recording was made, from its own metadata, for the
//! location list: the BirdNET+ Geomodel's species expected at that place and
//! week. Nothing here leaves the device.
//!
//! Sources, first found wins:
//! - Place: GUANO `Loc Position` ("lat lon"), or `Loc|Lat` with `Loc|Lon`;
//!   then a Wildlife Acoustics `wamd` chunk's GPS position; then Xeno-Canto's
//!   "Coordinates" ("lat, lon").
//! - Date: GUANO `Timestamp`, then the `wamd` timestamp, then Xeno-Canto's
//!   "Date" (all start "YYYY-MM-DD"). Without a date the whole year is used (week 0). A file's
//!   modified time is not used: copying a file changes it.

use crate::state::LoadedFile;

/// A recording's place and week.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub lat: f32,
    pub lon: f32,
    /// 1 to 48 (4 per month), or 0 when the date is unknown (the whole year).
    pub week: u32,
}

impl Place {
    /// The place rounded to about 1 km, with the week: the key for cached
    /// scores.
    pub fn key(&self) -> (i32, i32, u32) {
        (
            (self.lat * 100.0).round() as i32,
            (self.lon * 100.0).round() as i32,
            self.week,
        )
    }
}

/// "lat lon", "lat,lon" or "lat, lon" as decimal degrees, in range. Exactly
/// 0, 0 counts as no position: detectors write it when they have no GPS fix.
pub fn parse_lat_lon(value: &str) -> Option<(f32, f32)> {
    let s = value.trim();
    let parts: Vec<&str> = if s.contains(',') {
        s.split(',').collect()
    } else {
        s.split_whitespace().collect()
    };
    if parts.len() < 2 {
        return None;
    }
    let lat: f32 = parts[0].trim().parse().ok()?;
    let lon: f32 = parts[1].trim().parse().ok()?;
    let no_fix = lat == 0.0 && lon == 0.0;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) && !no_fix)
        .then_some((lat, lon))
}

/// The week of a date that starts "YYYY-MM-DD"; `None` for an unknown month or
/// day (Xeno-Canto writes "2019-00-00" when it has only a year).
pub fn week_of_date(date: &str) -> Option<u32> {
    let mut it = date.trim().get(..10)?.split('-');
    let (_y, m, d) = (it.next()?, it.next()?, it.next()?);
    let (m, d): (u32, u32) = (m.parse().ok()?, d.parse().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d))
        .then(|| oversample_ipc::classifier::week_of(m, d))
}

fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

/// The fields of the file's Wildlife Acoustics `wamd` chunk, as the metadata
/// panel shows them ("GPS Position" is "lat, lon").
fn wamd_fields(file: &LoadedFile) -> Option<&[(String, String)]> {
    let wav = file.audio.metadata.wav.as_ref()?;
    wav.wamd.as_ref().map(|b| b.fields.as_slice())
}

/// The recording's place and week from its own metadata, if it has a place.
pub fn file_place(file: &LoadedFile) -> Option<Place> {
    let guano = file
        .audio
        .metadata
        .guano
        .as_ref()
        .map(|g| g.fields.as_slice());
    let xc = file.site_metadata.as_ref().map(|m| m.fields.as_slice());
    let from_guano = guano.and_then(|g| {
        field(g, "Loc Position")
            .and_then(parse_lat_lon)
            .or_else(|| {
                let lat = field(g, "Loc|Lat")?.trim();
                let lon = field(g, "Loc|Lon")?.trim();
                parse_lat_lon(&format!("{lat} {lon}"))
            })
    });
    let from_wamd = || {
        wamd_fields(file)
            .and_then(|w| field(w, "GPS Position"))
            .and_then(parse_lat_lon)
    };
    let (lat, lon) = from_guano.or_else(from_wamd).or_else(|| {
        xc.and_then(|x| field(x, "Coordinates"))
            .and_then(parse_lat_lon)
    })?;
    Some(Place {
        lat,
        lon,
        week: file_week(file),
    })
}

/// The recording's week from its metadata date, or 0 (the whole year).
pub fn file_week(file: &LoadedFile) -> u32 {
    let guano = file
        .audio
        .metadata
        .guano
        .as_ref()
        .map(|g| g.fields.as_slice());
    let xc = file.site_metadata.as_ref().map(|m| m.fields.as_slice());
    guano
        .and_then(|g| field(g, "Timestamp"))
        .and_then(week_of_date)
        .or_else(|| {
            wamd_fields(file)
                .and_then(|w| field(w, "Timestamp"))
                .and_then(week_of_date)
        })
        .or_else(|| xc.and_then(|x| field(x, "Date")).and_then(week_of_date))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_and_dates() {
        assert_eq!(parse_lat_lon("52.5 13.4"), Some((52.5, 13.4)));
        assert_eq!(parse_lat_lon("-33.9, 151.2"), Some((-33.9, 151.2)));
        assert_eq!(parse_lat_lon("95 13"), None);
        assert_eq!(parse_lat_lon("0 0"), None);
        assert_eq!(parse_lat_lon("0.0, 0.0"), None);
        assert_eq!(parse_lat_lon("0, 13.4"), Some((0.0, 13.4)));
        assert_eq!(week_of_date("2023-06-01"), Some(21));
        assert_eq!(week_of_date("2023-07-15T22:30:45Z"), Some(27));
        assert_eq!(week_of_date("2019-00-00"), None);
        assert_eq!(week_of_date("bad"), None);
    }
}

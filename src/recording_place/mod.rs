//! Where and when a recording was made, from its own metadata. Nothing here
//! leaves the device.
//!
//! - [`file_place`]: the position and week, for the location list (the
//!   BirdNET+ Geomodel's species expected at that place and week).
//! - [`recording_place`]: the country, and an area inside it where
//!   [`areas`] has an outline (Victoria, Scotland, ...). The bat book and
//!   BattyBirdNET's "Auto" pick their region from it.
//! - [`countries`]: which country a position is in, offline.
//!
//! Sources, first found wins:
//! - Place ([`file_position`]): GUANO `Loc Position` ("lat lon"), or `Loc|Lat`
//!   with `Loc|Lon`; then a Wildlife Acoustics `wamd` chunk's GPS position; a
//!   Pettersson D500X block's LAT and LON; an Anabat header's GPS text; then
//!   the source website's "Coordinates" ("lat, lon").
//! - Date: GUANO `Timestamp`, then the `wamd` timestamp, then Xeno-Canto's
//!   "Date" (all start "YYYY-MM-DD"). Without a date the whole year is used (week 0). A file's
//!   modified time is not used: copying a file changes it.
//! - Country: the source site's "Country" (Xeno-canto, iNaturalist, ...),
//!   then the country at the position.

pub mod areas;
pub mod countries;

pub use areas::Area;

use oversample_core::position::parse_lat_lon;
use crate::bat_book::country_map::country_to_region;
use crate::source_site::SourceSite;
use crate::state::LoadedFile;
use countries::country_names_at;

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

/// Where a recording's position was read from.
#[derive(Clone, Debug, PartialEq)]
pub enum PositionSource {
    /// A position the recorder wrote: GUANO, a Wildlife Acoustics `wamd`
    /// chunk, an Anabat header.
    Recorder,
    /// A position the recorder's owner typed into it (a Pettersson D500X,
    /// which has no GPS).
    TypedIn,
    /// The coordinates on the website the recording came from.
    Site(SourceSite),
}

impl PositionSource {
    /// The position with what it is, to follow "From ": "GPS position 52.48,
    /// -1.90", "Xeno-canto coordinates 52.48, -1.90", "position typed into
    /// the recorder: 52.48, -1.90".
    pub fn phrase(&self, lat: f32, lon: f32) -> String {
        match self {
            Self::Recorder => format!("GPS position {lat:.2}, {lon:.2}"),
            Self::TypedIn => format!("position typed into the recorder: {lat:.2}, {lon:.2}"),
            Self::Site(site) => format!("{} coordinates {lat:.2}, {lon:.2}", site.name()),
        }
    }
}

/// A recording's position in decimal degrees, and where it was read from.
#[derive(Clone, Debug, PartialEq)]
pub struct FilePosition {
    pub lat: f64,
    pub lon: f64,
    pub source: PositionSource,
}

/// The recording's position from its own metadata. The first of these that
/// gives a position is used: GUANO, a Wildlife Acoustics `wamd` chunk, a
/// Pettersson D500X block, an Anabat zero-crossing header, then the source
/// website's coordinates. The Info panel's maps are drawn from the same
/// parsers.
pub fn file_position(file: &LoadedFile) -> Option<FilePosition> {
    let meta = &file.audio.metadata;
    let wav = meta.wav.as_ref();
    let from_guano = || {
        let g = meta.guano.as_ref()?.fields.as_slice();
        field(g, "Loc Position").and_then(parse_lat_lon).or_else(|| {
            let lat = field(g, "Loc|Lat")?.trim();
            let lon = field(g, "Loc|Lon")?.trim();
            parse_lat_lon(&format!("{lat} {lon}"))
        })
    };
    let from_wamd = || {
        wamd_fields(file)
            .and_then(|w| field(w, "GPS Position"))
            .and_then(parse_lat_lon)
    };
    let from_d500x = || {
        let block = wav?.recorder_block.as_ref()?;
        oversample_core::audio::wav::d500x_position(&block.fields)
    };
    let from_zc = || {
        let zc = meta.zc_data.as_ref()?;
        oversample_core::audio::zc::gps_position(&zc.metadata.gps)
    };
    let from_site = || {
        let site = file.site_metadata.as_ref()?;
        let position = field(&site.fields, "Coordinates").and_then(parse_lat_lon)?;
        Some((position, PositionSource::Site(site.site.clone())))
    };
    let recorder = |p| (p, PositionSource::Recorder);
    let ((lat, lon), source) = from_guano()
        .or_else(from_wamd)
        .map(recorder)
        .or_else(|| from_d500x().map(|p| (p, PositionSource::TypedIn)))
        .or_else(|| from_zc().map(recorder))
        .or_else(from_site)?;
    Some(FilePosition { lat, lon, source })
}

/// The recording's place and week from its own metadata, if it has a place.
pub fn file_place(file: &LoadedFile) -> Option<Place> {
    let position = file_position(file)?;
    Some(Place {
        lat: position.lat as f32,
        lon: position.lon as f32,
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

/// Where a recording's country came from.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaceSource {
    /// Its position ([`file_position`]), looked up offline.
    Gps { lat: f32, lon: f32 },
    /// The "Country" field of the site the recording came from.
    Metadata { site: SourceSite },
}

/// The country a recording was made in, and what else is known of the place.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordingPlace {
    pub source: PlaceSource,
    pub country: String,
    /// Latitude and longitude, when the recording has them (it can have a
    /// country from its source site and no position).
    pub position: Option<(f32, f32)>,
    /// Where `position` was read from.
    pub position_source: Option<PositionSource>,
    /// The area of the country the position is in, where one is outlined.
    pub area: Option<Area>,
}

impl RecordingPlace {
    /// The position with what it is, to follow "From ": "GPS position 52.48,
    /// -1.90" ([`PositionSource::phrase`]).
    pub fn position_phrase(&self) -> Option<String> {
        let (lat, lon) = self.position?;
        Some(self.position_source.as_ref()?.phrase(lat, lon))
    }

    /// Where the place came from, for a reader who is shown `region` (a
    /// region's name) beside it: "From GPS position 52.48, -1.90", with the
    /// country added when `region` is not the country itself, or "From
    /// iNaturalist country: Australia" (with ", GPS position -37.80, 145.01"
    /// added when the position is in an outlined area).
    pub fn detail(&self, region: &str) -> String {
        let position = self.position_phrase().unwrap_or_default();
        match &self.source {
            PlaceSource::Gps { .. } if self.country == region => format!("From {position}"),
            PlaceSource::Gps { .. } => format!("From {position} ({})", self.country),
            PlaceSource::Metadata { site } => {
                let mut text = format!("From {} country: {}", site.name(), self.country);
                // The position is what found an area inside the country.
                if self.area.is_some() && !position.is_empty() {
                    text.push_str(&format!(", {position}"));
                }
                text
            }
        }
    }
}

/// The recording's country and where it came from: the source site's
/// "Country" (Xeno-canto, iNaturalist, ...), or else the country at its GPS
/// position ([`file_place`]), looked up offline. Of a country's name variants,
/// the one the bat book's region map knows is used. With a position, also the
/// area of the country it is in ([`areas::area_at`]).
pub fn recording_place(file: &LoadedFile) -> Option<RecordingPlace> {
    let found = file_position(file);
    let position = found.as_ref().map(|p| (p.lat as f32, p.lon as f32));
    let position_source = found.map(|p| p.source);
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
    let (source, country) = match from_site.filter(|(_, c)| country_to_region(c).is_some()) {
        Some((site, country)) => (PlaceSource::Metadata { site }, country),
        None => {
            let (lat, lon) = position?;
            let names = country_names_at(lat, lon)?;
            let country = names
                .iter()
                .find(|n| country_to_region(n).is_some())
                .unwrap_or(&names[0])
                .clone();
            (PlaceSource::Gps { lat, lon }, country)
        }
    };
    let area = position.and_then(|(lat, lon)| areas::area_at(&country, lat, lon));
    Some(RecordingPlace {
        source,
        country,
        position,
        position_source,
        area,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(week_of_date("2023-06-01"), Some(21));
        assert_eq!(week_of_date("2023-07-15T22:30:45Z"), Some(27));
        assert_eq!(week_of_date("2019-00-00"), None);
        assert_eq!(week_of_date("bad"), None);
    }
}

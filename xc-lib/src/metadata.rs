//! Display metadata for XC recordings: the (label, value) rows shown in the
//! file's metadata panel. No network or file access, so the WASM frontend can
//! use it too.

use crate::types::XcRecording;

/// Metadata rows for a recording from an API search result.
pub fn recording_to_metadata(rec: &XcRecording) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    if !rec.en.is_empty() {
        fields.push(("Species".into(), rec.en.clone()));
    }
    if !rec.genus.is_empty() && !rec.sp.is_empty() {
        fields.push((
            "Scientific name".into(),
            format!("{} {}", rec.genus, rec.sp),
        ));
    }
    for (val, label) in [
        (&rec.rec, "Recordist"),
        (&rec.lic, "License"),
        (&rec.cnt, "Country"),
        (&rec.loc, "Location"),
    ] {
        if !val.is_empty() {
            fields.push((label.into(), val.clone()));
        }
    }
    let attribution = format!(
        "{}, XC{}. Accessible at www.xeno-canto.org/{}",
        rec.rec, rec.id, rec.id
    );
    fields.push(("Attribution".into(), attribution));
    if !rec.lat.is_empty() && !rec.lon.is_empty() {
        fields.push(("Coordinates".into(), format!("{}, {}", rec.lat, rec.lon)));
    }
    for (val, label) in [
        (&rec.date, "Date"),
        (&rec.sound_type, "Sound type"),
        (&rec.q, "Quality"),
        (&rec.dvc, "Device"),
        (&rec.mic, "Microphone"),
        (&rec.method, "Method"),
    ] {
        if !val.is_empty() {
            fields.push((label.into(), val.clone()));
        }
    }
    if !rec.url.is_empty() {
        fields.push(("URL".into(), rec.url.clone()));
    } else {
        fields.push((
            "URL".into(),
            format!("https://www.xeno-canto.org/{}", rec.id),
        ));
    }
    fields
}

/// Metadata rows for a recording from its `.xc.json` sidecar (as written by
/// [`crate::cache::build_metadata_json`]).
pub fn parse_xc_json_metadata(json: &serde_json::Value) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let s = |key: &str| json[key].as_str().unwrap_or("").to_string();

    let en = s("en");
    if !en.is_empty() {
        fields.push(("Species".into(), en));
    }
    let gen = s("gen");
    let sp = s("sp");
    if !gen.is_empty() && !sp.is_empty() {
        fields.push(("Scientific name".into(), format!("{gen} {sp}")));
    }
    for (key, label) in [
        ("rec", "Recordist"),
        ("lic", "License"),
        ("attribution", "Attribution"),
        ("cnt", "Country"),
        ("loc", "Location"),
    ] {
        let v = s(key);
        if !v.is_empty() {
            fields.push((label.into(), v));
        }
    }
    let lat = s("lat");
    let lon = s("lon");
    if !lat.is_empty() && !lon.is_empty() {
        fields.push(("Coordinates".into(), format!("{lat}, {lon}")));
    }
    for (key, label) in [
        ("date", "Date"),
        ("type", "Sound type"),
        ("q", "Quality"),
        ("dvc", "Device"),
        ("mic", "Microphone"),
        ("method", "Method"),
        ("url", "URL"),
    ] {
        let v = s(key);
        if !v.is_empty() {
            fields.push((label.into(), v));
        }
    }
    fields
}

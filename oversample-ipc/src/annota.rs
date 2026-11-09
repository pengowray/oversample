//! Annota-JSON: Xeno-canto's format for annotations of sound recordings.
//!
//! Spec: <https://xeno-canto.org/article/321> (status FINAL, version Fv0,
//! March 2026). Uploading and downloading: <https://xeno-canto.org/article/318>.
//!
//! A set holds metadata shared by its annotations, then the annotations: each
//! a time span (seconds from the start of the recording) and, ideally, a
//! frequency band (Hz), with the taxon and sound type.
//!
//! Reading is lenient, because what XC sends differs from the spec in places:
//! - `annotation-set` in a recording is `[]` when empty and an object otherwise.
//! - `original_set_metadata` is an array in the spec and an object in the API.
//! - `annotation_xc_id` is a number; `xc_nr` is a string; frequencies are
//!   numbers. Any of these may arrive as either.
//! - Missing values come as `null`.
//!
//! Every field is optional here; [`MUST_HAVE`] and [`SHOULD_HAVE`] list what XC
//! requires and what it warns about on upload.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Fields XC rejects an upload without.
pub const MUST_HAVE: &[&str] = &["xc_nr", "start_time", "end_time", "scientific_name"];

/// Fields XC warns about when missing.
pub const SHOULD_HAVE: &[&str] = &[
    "set_name",
    "set_creator",
    "set_owner",
    "set_license",
    "annotator",
    "frequency_high",
    "frequency_low",
    "sound_type",
    "sex",
    "life_stage",
    "annotation_remarks",
];

/// Licences for an annotation set. XC limits the choice to licences GBIF can
/// share; these are the ones GBIF accepts.
pub const LICENSES: &[&str] = &["CC0-1.0", "CC-BY-4.0", "CC-BY-NC-4.0"];

/// The XC endpoint that takes an annotation set as JSON (POST, with the
/// user's API key in a `key` header).
pub const UPLOAD_URL: &str = "https://xeno-canto.org/api/3/upload/annotation-set";

/// Metadata of an annotation set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotaSetMeta {
    /// Name of a findable resource the set is derived from.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_source: Option<String>,
    /// DOI or URL of the set. XC fills this in for sets it holds.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_uri: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_name: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub annotation_software_name_and_version: Option<String>,
    /// Person or organisation that made the set.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_creator: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_creator_id: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_creation_date: Option<String>,
    /// Legal owners.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_owner: Option<String>,
    /// One of [`LICENSES`].
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_license: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub project_uri: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub project_name: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub funding: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_remarks: Option<String>,
    #[serde(
        default,
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub scope: Vec<AnnotaScope>,
    /// Filled in by XC on export only.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub set_import_date: Option<String>,
}

/// What a set covers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotaScope {
    /// Focal taxon: species, genus, family, or a list of them.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub taxon_coverage: Option<String>,
    /// Whether all sounds of the focal taxon were annotated (`all` or `part`).
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub completeness: Option<String>,
}

/// An annotation set: metadata plus annotations.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotaSet {
    #[serde(flatten)]
    pub meta: AnnotaSetMeta,
    #[serde(default)]
    pub annotations: Vec<AnnotaAnnotation>,
}

/// One annotation: a time span, ideally with a frequency band, and what made
/// the sound.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotaAnnotation {
    /// XC's id for the annotation. Export only.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub annotation_xc_id: Option<String>,
    /// The annotation's id in the program that made it.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub annotation_source_id: Option<String>,
    /// Name of the original sound file.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub sound_file: Option<String>,
    /// XC number of the recording, without the `XC` prefix.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub xc_nr: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub annotator: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub annotator_xc_id: Option<String>,
    /// Top of the box, Hz.
    #[serde(
        default,
        deserialize_with = "loose_f64",
        serialize_with = "whole_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub frequency_high: Option<f64>,
    /// Bottom of the box, Hz. 0 is a real value, not "unknown".
    #[serde(
        default,
        deserialize_with = "loose_f64",
        serialize_with = "whole_number",
        skip_serializing_if = "Option::is_none"
    )]
    pub frequency_low: Option<f64>,
    /// Seconds from the start of the recording.
    #[serde(
        default,
        deserialize_with = "loose_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub start_time: Option<f64>,
    /// Seconds from the start of the recording.
    #[serde(
        default,
        deserialize_with = "loose_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub end_time: Option<f64>,
    /// Scientific name, or a note that the sound is not from an animal.
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub scientific_name: Option<String>,
    /// From XC's list for the species group (for bats: echolocation, feeding
    /// buzz, social call, distress call, song, uncertain).
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub sound_type: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub date_identified: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub sex: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub life_stage: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub animal_seen: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub playback_used: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub collection_date: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub collection_specimen: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub temperature: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub annotation_remarks: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub signal_noise_ratio: Option<String>,
    #[serde(
        default,
        deserialize_with = "loose_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub overlap: Option<String>,
    /// The set each annotation came from, when XC merges sets on export.
    #[serde(
        default,
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub original_set_metadata: Vec<AnnotaSetMeta>,
}

impl AnnotaAnnotation {
    /// The metadata of the set this annotation came from: its own
    /// `original_set_metadata` when XC merged sets, otherwise `fallback`.
    pub fn source_set<'a>(&'a self, fallback: &'a AnnotaSetMeta) -> &'a AnnotaSetMeta {
        self.original_set_metadata.first().unwrap_or(fallback)
    }
}

/// A recording's `annotation-set` field: `[]` when there are none.
#[derive(Deserialize)]
#[allow(dead_code)]
#[serde(untagged)]
enum SetOrEmpty {
    Set(AnnotaSet),
    Empty(Vec<serde::de::IgnoredAny>),
}

/// Read a recording's `annotation-set` field, which XC sends as `[]` when the
/// recording has no annotations. Gives `None` for that, for `null`, and for a
/// set with no annotations.
pub fn deserialize_recording_set<'de, D>(d: D) -> Result<Option<AnnotaSet>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<SetOrEmpty>::deserialize(d)? {
        Some(SetOrEmpty::Set(s)) if !s.annotations.is_empty() => Some(s),
        _ => None,
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Loose {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
}

fn loose_string<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<Loose>::deserialize(d).unwrap_or(None) {
        None => None,
        Some(Loose::Str(s)) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        Some(Loose::Int(i)) => Some(i.to_string()),
        Some(Loose::Float(f)) => Some(f.to_string()),
        Some(Loose::Bool(b)) => Some(if b { "yes" } else { "no" }.to_string()),
    })
}

fn loose_f64<'de, D>(d: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<Loose>::deserialize(d).unwrap_or(None) {
        Some(Loose::Int(i)) => Some(i as f64),
        Some(Loose::Float(f)) if f.is_finite() => Some(f),
        Some(Loose::Str(s)) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        _ => None,
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    Many(Vec<T>),
    One(T),
}

fn one_or_many<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(
        match Option::<OneOrMany<T>>::deserialize(d).unwrap_or(None) {
            None => Vec::new(),
            Some(OneOrMany::Many(v)) => v,
            Some(OneOrMany::One(t)) => vec![t],
        },
    )
}

/// Write whole numbers without a trailing `.0`, as XC does.
fn whole_number<S>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match v {
        Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => s.serialize_i64(*f as i64),
        Some(f) => s.serialize_f64(*f),
        None => s.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like XC's API output (numbers, strings and nulls as XC sends
    /// them, `original_set_metadata` as an object).
    const FROM_API: &str = r#"{
        "set_name": "Annotation set for XC1",
        "set_creator": null,
        "set_creation_date": "2026-11-09 04:20:46",
        "annotations": [
            {
                "annotation_xc_id": 70234,
                "xc_nr": "1",
                "scientific_name": "Nyctalus noctula",
                "annotator": "A. Recordist",
                "start_time": 9.72327,
                "end_time": 10.0553,
                "frequency_high": 46688,
                "frequency_low": 0,
                "sound_type": "feeding buzz",
                "sex": null,
                "original_set_metadata": {
                    "set_name": "Annotations for XC1",
                    "set_license": "CC-BY-4.0",
                    "set_uri": "https://xeno-canto.org/annotation/set/174"
                }
            }
        ]
    }"#;

    #[test]
    fn reads_api_shapes() {
        let set: AnnotaSet = serde_json::from_str(FROM_API).unwrap();
        assert_eq!(set.meta.set_creator, None);
        let a = &set.annotations[0];
        assert_eq!(a.annotation_xc_id.as_deref(), Some("70234"));
        assert_eq!(a.xc_nr.as_deref(), Some("1"));
        assert_eq!(a.frequency_low, Some(0.0));
        assert_eq!(a.frequency_high, Some(46688.0));
        assert_eq!(a.sex, None);
        let src = a.source_set(&set.meta);
        assert_eq!(src.set_license.as_deref(), Some("CC-BY-4.0"));
    }

    #[test]
    fn reads_spec_shapes() {
        // The spec has original_set_metadata and scope as arrays, and numbers
        // as strings in places.
        let json = r#"{
            "set_name": "s",
            "scope": [{"taxon_coverage": "Chiroptera", "completeness": "part"}],
            "annotations": [{
                "xc_nr": 23832,
                "start_time": "1.5",
                "end_time": 2,
                "scientific_name": "Myotis daubentonii",
                "original_set_metadata": [{"set_name": "orig"}]
            }]
        }"#;
        let set: AnnotaSet = serde_json::from_str(json).unwrap();
        assert_eq!(set.meta.scope[0].completeness.as_deref(), Some("part"));
        let a = &set.annotations[0];
        assert_eq!(a.xc_nr.as_deref(), Some("23832"));
        assert_eq!(a.start_time, Some(1.5));
        assert_eq!(a.end_time, Some(2.0));
        assert_eq!(a.source_set(&set.meta).set_name.as_deref(), Some("orig"));
    }

    #[derive(Deserialize)]
    struct Rec {
        #[serde(
            default,
            rename = "annotation-set",
            deserialize_with = "deserialize_recording_set"
        )]
        set: Option<AnnotaSet>,
    }

    #[test]
    fn recording_field_empty_array_is_none() {
        let r: Rec = serde_json::from_str(r#"{"annotation-set": []}"#).unwrap();
        assert!(r.set.is_none());
        let r: Rec = serde_json::from_str(r#"{"annotation-set": null}"#).unwrap();
        assert!(r.set.is_none());
        let r: Rec = serde_json::from_str(r#"{}"#).unwrap();
        assert!(r.set.is_none());
        let r: Rec = serde_json::from_str(&format!(r#"{{"annotation-set": {FROM_API}}}"#)).unwrap();
        assert_eq!(r.set.unwrap().annotations.len(), 1);
    }

    #[test]
    fn writes_whole_numbers_without_decimals() {
        let a = AnnotaAnnotation {
            xc_nr: Some("1".into()),
            start_time: Some(1.25),
            end_time: Some(2.0),
            frequency_low: Some(15000.0),
            frequency_high: Some(15000.5),
            ..Default::default()
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains(r#""frequency_low":15000,"#), "{json}");
        assert!(json.contains(r#""frequency_high":15000.5"#), "{json}");
        assert!(json.contains(r#""end_time":2.0"#), "{json}");
        assert!(!json.contains("sex"), "{json}");
    }
}

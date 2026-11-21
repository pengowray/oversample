//! The external site or archive a loaded file came from, and the label/value
//! metadata that came with it.
//!
//! Display names end up in saved file names (`find_recordings::file_name`) and
//! so in .batm and OPFS identities. Don't change their spelling.

use serde::{Deserialize, Serialize};

/// A site or archive recordings are loaded from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceSite {
    XenoCanto,
    INaturalist,
    ObservationOrg,
    Rbins,
    BorrorLab,
    HumboldtInstitute,
    WikimediaCommons,
    /// A file stored on a Wikipedia itself rather than on Commons, loaded
    /// through the Wikimedia import. The search server has no Wikipedia
    /// source, so it has no search key.
    Wikipedia,
    Gbif,
    OversampleDemos,
    /// A clip of an Australian Acoustic Observatory recording, opened from
    /// an A2O link (`crate::a2o`).
    AcousticObservatory,
    /// A search source this build doesn't know, by its search key.
    Other(String),
}

/// Each known site with its search key (as the search server sends it) and
/// display name.
static KNOWN: [(SourceSite, &str, &str); 10] = [
    (SourceSite::XenoCanto, "xc", "Xeno-canto"),
    (SourceSite::INaturalist, "inat", "iNaturalist"),
    (SourceSite::ObservationOrg, "obsorg", "Observation.org"),
    (SourceSite::Rbins, "rbins", "RBINS"),
    (SourceSite::BorrorLab, "borror", "Borror Lab"),
    (
        SourceSite::HumboldtInstitute,
        "humboldt",
        "Humboldt Institute",
    ),
    (SourceSite::WikimediaCommons, "commons", "Wikimedia Commons"),
    (SourceSite::Gbif, "gbif", "GBIF"),
    (SourceSite::OversampleDemos, "demo", "Oversample demos"),
    (
        SourceSite::AcousticObservatory,
        "a2o",
        "Australian Acoustic Observatory",
    ),
];

impl SourceSite {
    /// The site for a search key ("xc", "inat", ...). An unknown key becomes
    /// `Other(key)`.
    pub fn from_search_key(key: &str) -> Self {
        KNOWN
            .iter()
            .find(|(_, k, _)| *k == key)
            .map(|(site, _, _)| site.clone())
            .unwrap_or_else(|| SourceSite::Other(key.to_string()))
    }

    /// The key the search server uses for this site. `None` for Wikipedia,
    /// which is not a search source.
    pub fn search_key(&self) -> Option<&str> {
        match self {
            SourceSite::Other(key) => Some(key),
            SourceSite::Wikipedia => None,
            known => KNOWN
                .iter()
                .find(|(site, _, _)| site == known)
                .map(|(_, key, _)| *key),
        }
    }

    /// The site's display name ("Xeno-canto", "Wikimedia Commons", ...). An
    /// unknown site shows its search key.
    pub fn name(&self) -> &str {
        match self {
            SourceSite::Other(key) => key,
            SourceSite::Wikipedia => "Wikipedia",
            known => KNOWN
                .iter()
                .find(|(site, _, _)| site == known)
                .map(|(_, _, name)| *name)
                .unwrap_or_default(),
        }
    }

    /// The display name for a search key, or the key itself when it is unknown.
    pub fn name_for_key(key: &str) -> &str {
        KNOWN
            .iter()
            .find(|(_, k, _)| *k == key)
            .map(|(_, _, name)| *name)
            .unwrap_or(key)
    }

    /// The label to show for a metadata key from this site. iNaturalist
    /// calls its "Quality" value ("Research grade", "Needs ID") a quality
    /// grade. Only the shown label changes: code still looks the field up by
    /// its key.
    pub fn field_label<'a>(&self, key: &'a str) -> &'a str {
        match (self, key) {
            (SourceSite::INaturalist, "Quality") => "Quality grade",
            _ => key,
        }
    }
}

/// Label/value rows from the site a file came from. Keys are human-readable
/// labels ("Species", "License", "Country", ...), looked up by name by the
/// metadata panel, device hinting and bat-book auto-resolve.
#[derive(Clone, Debug, PartialEq)]
pub struct SiteMetadata {
    pub site: SourceSite,
    pub fields: Vec<(String, String)>,
}

impl SiteMetadata {
    pub fn new(site: SourceSite, fields: Vec<(String, String)>) -> Self {
        Self { site, fields }
    }

    pub fn is_xeno_canto(&self) -> bool {
        self.site == SourceSite::XenoCanto
    }

    /// The person the licence dialog credits, with the label to show:
    /// "Recordist" from Xeno-canto and the search sources, or "Author" from
    /// Wikimedia. `None` when the file has neither field.
    pub fn credited_person(&self) -> Option<(&'static str, String)> {
        ["Recordist", "Author"].into_iter().find_map(|key| {
            self.fields
                .iter()
                .find(|(k, v)| k == key && !v.trim().is_empty())
                .map(|(_, v)| (key, v.clone()))
        })
    }

    /// Xeno-canto's A to E quality rating. Other sites' "Quality"
    /// (iNaturalist's "Research grade") rates the identification, not the
    /// sound, so it gives `None`.
    pub fn xc_rating(&self) -> Option<char> {
        if !self.is_xeno_canto() {
            return None;
        }
        self.fields
            .iter()
            .find(|(k, _)| k == "Quality")
            .and_then(|(_, v)| xc_rating_letter(v))
    }
}

/// Where a recording came from, as saved in its `.batm` file (the `source`
/// block), so the Info panel can show it again when the audio is later
/// opened from disk. `site` and `fields` are the site's own data as it came;
/// the other fields are copies of a few of them for other programs reading
/// the file, and are not read back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordingSource {
    /// The site's search key (`xc`, `inat`, `commons`, ...), or `wikipedia`.
    pub site: String,
    /// The recording's id on the site, from its page URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The recordist, or the author for Wikimedia files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recordist: Option<String>,
    /// The licence as the site gives it: a code or a URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// When the site's data was fetched (ISO 8601). The later one wins when
    /// two copies of a set hold different sources.
    #[serde(default)]
    pub fetched_at: String,
    /// The site's label/value rows in order, each as a one-entry map.
    #[serde(default, with = "field_list")]
    pub fields: Vec<(String, String)>,
}

/// The saved key for Wikipedia, which has no search key.
const WIKIPEDIA_KEY: &str = "wikipedia";

impl RecordingSource {
    pub fn from_site(meta: &SiteMetadata, fetched_at: String) -> Self {
        let field = |key: &str| {
            meta.fields
                .iter()
                .find(|(k, v)| k == key && !v.trim().is_empty())
                .map(|(_, v)| v.clone())
        };
        let url = field("URL");
        RecordingSource {
            site: meta.site.search_key().unwrap_or(WIKIPEDIA_KEY).to_string(),
            id: url.as_deref().and_then(id_from_url),
            url,
            recordist: meta.credited_person().map(|(_, v)| v),
            license: field("License"),
            attribution: field("Attribution"),
            fetched_at,
            fields: meta.fields.clone(),
        }
    }

    /// The site metadata to show for a file opened without it.
    pub fn to_site_metadata(&self) -> SiteMetadata {
        let site = if self.site == WIKIPEDIA_KEY {
            SourceSite::Wikipedia
        } else {
            SourceSite::from_search_key(&self.site)
        };
        SiteMetadata::new(site, self.fields.clone())
    }

    /// Whether this holds the same site data as `meta` (whenever it was
    /// fetched).
    fn holds(&self, meta: &SiteMetadata) -> bool {
        self.to_site_metadata() == *meta
    }

    /// The source to save with a file's annotation set: the one already in
    /// the set when it holds the same data as the file's site metadata (so
    /// its fetch time stays), a new one from the file's site metadata
    /// otherwise, or the set's own when the file has none.
    pub fn to_save(
        saved: Option<&RecordingSource>,
        meta: Option<&SiteMetadata>,
        now: impl FnOnce() -> String,
    ) -> Option<RecordingSource> {
        match (saved, meta) {
            (_, None) => saved.cloned(),
            (Some(s), Some(m)) if s.holds(m) => Some(s.clone()),
            (_, Some(m)) => Some(RecordingSource::from_site(m, now())),
        }
    }
}

/// The last part of a page URL's path ("928094" from
/// "https://xeno-canto.org/928094", "File:Bat.wav" from a Commons page).
fn id_from_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let path = path.split_once("://").map_or(path, |(_, rest)| rest);
    let (_, path) = path.split_once('/')?;
    path.rsplit('/').find(|s| !s.is_empty()).map(str::to_string)
}

/// Label/value rows as a YAML list of one-entry maps (`- Species: ...`),
/// which keeps their order and reads like the Info panel.
mod field_list {
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(fields: &[(String, String)], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(fields.len()))?;
        for (k, v) in fields {
            seq.serialize_element(&BTreeMap::from([(k, v)]))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, String)>, D::Error> {
        let rows: Vec<BTreeMap<String, String>> = Deserialize::deserialize(d)?;
        Ok(rows.into_iter().flatten().collect())
    }
}

/// Xeno-canto's quality rating as a letter, when it is one of A to E.
/// Xeno-canto sends "no score" for unrated recordings.
fn xc_rating_letter(quality: &str) -> Option<char> {
    let mut chars = quality.trim().chars();
    let c = chars.next()?.to_ascii_uppercase();
    (chars.next().is_none() && ('A'..='E').contains(&c)).then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Search key and display name pairs as `search_client::source_label`
    /// returned them before this module existed.
    const EXPECTED: [(&str, &str); 9] = [
        ("xc", "Xeno-canto"),
        ("inat", "iNaturalist"),
        ("rbins", "RBINS"),
        ("obsorg", "Observation.org"),
        ("borror", "Borror Lab"),
        ("humboldt", "Humboldt Institute"),
        ("commons", "Wikimedia Commons"),
        ("gbif", "GBIF"),
        ("demo", "Oversample demos"),
    ];

    #[test]
    fn known_keys_round_trip() {
        for (key, _) in EXPECTED {
            let site = SourceSite::from_search_key(key);
            assert!(!matches!(site, SourceSite::Other(_)), "{key} is unknown");
            assert_eq!(site.search_key(), Some(key));
        }
        for (site, key, _) in KNOWN.iter() {
            assert_eq!(&SourceSite::from_search_key(key), site);
        }
    }

    #[test]
    fn credits_the_recordist_or_the_author() {
        let meta = |site: SourceSite, fields: &[(&str, &str)]| {
            SiteMetadata::new(
                site,
                fields
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            )
        };
        assert_eq!(
            meta(SourceSite::XenoCanto, &[("Recordist", "Ann Bee")]).credited_person(),
            Some(("Recordist", "Ann Bee".to_string()))
        );
        assert_eq!(
            meta(
                SourceSite::WikimediaCommons,
                &[("Description", "A bat"), ("Author", "Cy Dee")]
            )
            .credited_person(),
            Some(("Author", "Cy Dee".to_string()))
        );
        assert_eq!(
            meta(
                SourceSite::Gbif,
                &[("Recordist", "  "), ("Author", "Ed Fox")]
            )
            .credited_person(),
            Some(("Author", "Ed Fox".to_string()))
        );
        assert_eq!(meta(SourceSite::Wikipedia, &[]).credited_person(), None);
    }

    #[test]
    fn wikipedia_has_a_name_and_no_search_key() {
        assert_eq!(SourceSite::Wikipedia.name(), "Wikipedia");
        assert_eq!(SourceSite::Wikipedia.search_key(), None);
        for key in ["wikipedia", "Wikipedia", "enwiki"] {
            assert_eq!(
                SourceSite::from_search_key(key),
                SourceSite::Other(key.to_string())
            );
        }
    }

    #[test]
    fn names_match_the_old_source_labels() {
        for (key, name) in EXPECTED {
            assert_eq!(SourceSite::from_search_key(key).name(), name);
            assert_eq!(SourceSite::name_for_key(key), name);
        }
        for key in ["", "foo", "Xeno-canto", "XC"] {
            let site = SourceSite::from_search_key(key);
            assert_eq!(site, SourceSite::Other(key.to_string()));
            assert_eq!(site.name(), key);
            assert_eq!(site.search_key(), Some(key));
            assert_eq!(SourceSite::name_for_key(key), key);
        }
    }

    #[test]
    fn inaturalist_quality_is_a_quality_grade() {
        assert_eq!(
            SourceSite::INaturalist.field_label("Quality"),
            "Quality grade"
        );
        assert_eq!(SourceSite::INaturalist.field_label("License"), "License");
        assert_eq!(SourceSite::XenoCanto.field_label("Quality"), "Quality");
        assert_eq!(SourceSite::Gbif.field_label("Quality"), "Quality");
    }

    #[test]
    fn rating_letter_is_a_to_e_only() {
        assert_eq!(xc_rating_letter("A"), Some('A'));
        assert_eq!(xc_rating_letter(" e "), Some('E'));
        assert_eq!(xc_rating_letter("no score"), None);
        assert_eq!(xc_rating_letter("Research grade"), None);
        assert_eq!(xc_rating_letter("F"), None);
        assert_eq!(xc_rating_letter(""), None);
    }

    #[test]
    fn rating_is_xeno_canto_only() {
        let fields = vec![("Quality".to_string(), "B".to_string())];
        let xc = SiteMetadata::new(SourceSite::XenoCanto, fields.clone());
        assert_eq!(xc.xc_rating(), Some('B'));
        let inat = SiteMetadata::new(SourceSite::INaturalist, fields);
        assert_eq!(inat.xc_rating(), None);
        let unrated = SiteMetadata::new(SourceSite::XenoCanto, vec![]);
        assert_eq!(unrated.xc_rating(), None);
    }
}

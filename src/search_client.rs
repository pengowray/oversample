//! Client for oversample-api's combined search (`/search/species`, `/search/recordings`,
//! `/search/facets`): species and recordings from every source the server indexes
//! (Xeno-canto now; GBIF, Wikimedia Commons and iNaturalist later), filtered by group,
//! country and distance from a point.
//!
//! The JSON shapes are fixed by the server's contract (batchi-backend DESIGN.md,
//! "Combined search"), so a new source adds values, not fields.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::xc_client::{self, ServerReply, XcError};

/// Radius choices for "near", in km. The server caps the radius at 500.
pub const RADII_KM: [u32; 5] = [25, 50, 100, 250, 500];
pub const DEFAULT_KM: u32 = 100;

/// Groups the search knows, in menu order, with their menu labels.
pub const GROUPS: [(&str, &str); 7] = [
    ("bats", "Bats"),
    ("birds", "Birds"),
    ("frogs", "Frogs"),
    ("grasshoppers", "Grasshoppers"),
    ("insects", "Other insects"),
    ("land mammals", "Land mammals"),
    ("other", "Other animals"),
];

/// The filters every search request takes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    /// `None` is every group.
    pub group: Option<String>,
    /// ISO 3166-1 alpha-2 code.
    pub country: Option<String>,
    /// Latitude and longitude, already rounded by [`round_point`].
    pub near: Option<(f64, f64)>,
    pub km: u32,
}

impl Filter {
    fn push_params(&self, url: &mut String) {
        if let Some(g) = &self.group {
            push(url, "group", g);
        }
        if let Some(c) = &self.country {
            push(url, "country", c);
        }
        if let Some((lat, lon)) = self.near {
            push(url, "near", &format!("{lat:.1},{lon:.1}"));
            push(url, "km", &self.km.to_string());
        }
    }
}

/// A position rounded to 0.1 degree (about 11 km), so the server never sees the user's
/// exact location.
pub fn round_point(lat: f64, lon: f64) -> (f64, f64) {
    ((lat * 10.0).round() / 10.0, (lon * 10.0).round() / 10.0)
}

fn push(url: &mut String, name: &str, value: &str) {
    url.push(if url.contains('?') { '&' } else { '?' });
    url.push_str(name);
    url.push('=');
    url.push_str(&String::from(js_sys::encode_uri_component(value)));
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct NameMatch {
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub exact: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SpeciesHit {
    /// Pass back as `species=` to list its recordings.
    pub key: String,
    pub gen: String,
    pub sp: String,
    /// 0 species, 1 genus only.
    #[serde(default)]
    pub rank: u8,
    pub grp: String,
    #[serde(default)]
    pub en: String,
    #[serde(default)]
    pub local: Option<String>,
    #[serde(default)]
    pub local_lang: Option<String>,
    /// Recordings under the current filters; 0 for a name match with none.
    pub n: u64,
    #[serde(default)]
    pub sources: BTreeMap<String, u64>,
    #[serde(default)]
    pub iucn_taxon_id: Option<i64>,
    #[serde(default)]
    pub wikidata: Option<String>,
    #[serde(default)]
    pub wiki_en: Option<String>,
    #[serde(default)]
    pub matched: Option<NameMatch>,
}

impl SpeciesHit {
    /// "Pipistrellus pipistrellus", or "Myotis sp." for a genus-only row.
    pub fn scientific(&self) -> String {
        if self.sp.is_empty() {
            format!("{} sp.", self.gen)
        } else {
            format!("{} {}", self.gen, self.sp)
        }
    }

    /// The name to show first: the name in the user's language, else English, else
    /// the scientific name.
    pub fn display_name(&self) -> String {
        self.local
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| Some(self.en.clone()).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| self.scientific())
    }

    /// The species' Wikipedia article: the stored English title when the server has
    /// one, else Wikipedia's "go" search, which opens the article whose title or
    /// redirect is the scientific name.
    pub fn wikipedia_url(&self) -> String {
        if let Some(title) = self.wiki_en.as_deref().filter(|t| !t.is_empty()) {
            return format!(
                "https://en.wikipedia.org/wiki/{}",
                String::from(js_sys::encode_uri_component(&title.replace(' ', "_")))
            );
        }
        let name = if self.sp.is_empty() {
            self.gen.clone()
        } else {
            format!("{} {}", self.gen, self.sp)
        };
        format!(
            "https://en.wikipedia.org/wiki/Special:Search?go=Go&search={}",
            String::from(js_sys::encode_uri_component(&name))
        )
    }

    /// The species' page on BeastieBot's species site.
    pub fn beastiebot_url(&self) -> String {
        match self.iucn_taxon_id.filter(|id| *id > 0) {
            Some(id) => format!("https://species.pengowray.com/species/{id}"),
            None => {
                let name = if self.sp.is_empty() {
                    self.gen.clone()
                } else {
                    format!("{}_{}", self.gen, self.sp)
                };
                format!(
                    "https://species.pengowray.com/name/{}",
                    String::from(js_sys::encode_uri_component(&name))
                )
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SpeciesPage {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub cursor: Option<String>,
    pub species: Vec<SpeciesHit>,
    #[serde(default)]
    pub status: BTreeMap<String, String>,
    /// Counts under `near` are summed over whole map cells, so they are close, not exact.
    #[serde(default)]
    pub approx: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct RecordingItem {
    /// Origin-prefixed id: "xc:928094", "inat:123/0".
    pub id: String,
    pub source: String,
    pub gen: String,
    pub sp: String,
    #[serde(default)]
    pub en: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub place: String,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
    /// Distance from the "near" point, only when one was given.
    #[serde(default)]
    pub km: Option<f64>,
    #[serde(default)]
    pub smp: Option<u32>,
    #[serde(default)]
    pub len_s: Option<f64>,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub recordist: String,
    #[serde(default)]
    pub licence: String,
    #[serde(default)]
    pub attribution: Option<String>,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub demo: bool,
    #[serde(default)]
    pub time_expansion: Option<f64>,
    #[serde(default)]
    pub page_url: String,
    #[serde(default)]
    pub audio_url: String,
    /// "xc": load by XC number; "direct": fetch `audio_url`; "proxy": fetch through
    /// the server.
    #[serde(default)]
    pub audio_via: String,
}

impl RecordingItem {
    /// The XC number of an "xc:" recording.
    pub fn xc_number(&self) -> Option<u64> {
        self.id.strip_prefix("xc:")?.parse().ok()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct RecordingsPage {
    #[serde(default)]
    pub total: u64,
    #[serde(default = "yes")]
    pub total_exact: bool,
    #[serde(default)]
    pub by_source: BTreeMap<String, u64>,
    #[serde(default)]
    pub cursor: Option<String>,
    /// The "near" search stopped after its candidate limit; more recordings may exist
    /// further out.
    #[serde(default)]
    pub truncated: bool,
    pub items: Vec<RecordingItem>,
    #[serde(default)]
    pub status: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct GroupCount {
    pub grp: String,
    pub n: u64,
    #[serde(default)]
    pub species: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CountryCount {
    pub cc: String,
    pub name: String,
    pub n: u64,
    #[serde(default)]
    pub species: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SourceCount {
    pub source: String,
    pub n: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Facets {
    #[serde(default)]
    pub groups: Vec<GroupCount>,
    #[serde(default)]
    pub countries: Vec<CountryCount>,
    #[serde(default)]
    pub sources: Vec<SourceCount>,
    #[serde(default)]
    pub status: BTreeMap<String, String>,
}

/// Why a search request gave no result.
#[derive(Clone, Debug, PartialEq)]
pub enum SearchError {
    /// The server has no combined search (an older server: 404), or could not be
    /// reached. The app falls back to the separate Xeno-canto and Commons browsers.
    Unavailable,
    /// The server is still building its index; ask again later.
    Indexing,
    Other(String),
}

impl From<XcError> for SearchError {
    fn from(e: XcError) -> Self {
        match e {
            XcError::Network | XcError::Http { status: 404, .. } => SearchError::Unavailable,
            XcError::Http {
                message: Some(m), ..
            }
            | XcError::Busy(Some(m)) => SearchError::Other(m),
            XcError::RateLimited => {
                SearchError::Other("Too many searches. Wait a few seconds.".into())
            }
            other => SearchError::Other(format!("{other:?}")),
        }
    }
}

async fn get<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T, SearchError> {
    match xc_client::server_get(url, false).await? {
        ServerReply::Ready(body) => serde_json::from_str(&body)
            .map_err(|e| SearchError::Other(format!("unexpected reply: {e}"))),
        ServerReply::Indexing { .. } => Err(SearchError::Indexing),
    }
}

fn base(path: &str) -> String {
    format!("{}/search/{path}", xc_client::proxy_base())
}

/// Species matching `q` (any name, any language), or with no `q` the species with
/// recordings under `filter`, most recordings first.
pub async fn species(
    q: &str,
    filter: &Filter,
    cursor: Option<&str>,
) -> Result<SpeciesPage, SearchError> {
    let mut url = base("species");
    if !q.trim().is_empty() {
        push(&mut url, "q", q.trim());
    }
    filter.push_params(&mut url);
    let langs = xc_client::names_language().codes();
    if !langs.is_empty() {
        push(&mut url, "lang", &langs.join(","));
    }
    if let Some(c) = cursor {
        push(&mut url, "cursor", c);
    }
    get(&url).await
}

/// Recordings under `filter`, of one species when `species` is a key from
/// [`species`].
pub async fn recordings(
    species: Option<&str>,
    filter: &Filter,
    cursor: Option<&str>,
) -> Result<RecordingsPage, SearchError> {
    let mut url = base("recordings");
    if let Some(s) = species {
        push(&mut url, "species", s);
    }
    filter.push_params(&mut url);
    if let Some(c) = cursor {
        push(&mut url, "cursor", c);
    }
    get(&url).await
}

/// Counts per group, country and source under `filter`, for the filter menus.
pub async fn facets(filter: &Filter) -> Result<Facets, SearchError> {
    let mut url = base("facets");
    filter.push_params(&mut url);
    get(&url).await
}

/// The English name of an ISO country code, from the browser's `Intl.DisplayNames`;
/// the code itself if the browser has no name for it.
pub fn country_name(code: &str) -> String {
    use wasm_bindgen::JsValue;
    let name = (|| {
        let intl = js_sys::Reflect::get(&js_sys::global(), &"Intl".into()).ok()?;
        let ctor: js_sys::Function = wasm_bindgen::JsCast::dyn_into(
            js_sys::Reflect::get(&intl, &"DisplayNames".into()).ok()?,
        )
        .ok()?;
        let opts = js_sys::Object::new();
        js_sys::Reflect::set(&opts, &"type".into(), &"region".into()).ok()?;
        let locales = js_sys::Array::of1(&JsValue::from_str("en"));
        let names = js_sys::Reflect::construct(&ctor, &js_sys::Array::of2(&locales, &opts)).ok()?;
        let of: js_sys::Function =
            wasm_bindgen::JsCast::dyn_into(js_sys::Reflect::get(&names, &"of".into()).ok()?)
                .ok()?;
        of.call1(&names, &JsValue::from_str(code)).ok()?.as_string()
    })();
    name.unwrap_or_else(|| code.to_string())
}

/// The short name of a source, for the source column.
pub fn source_label(source: &str) -> &str {
    match source {
        "xc" => "Xeno-canto",
        "inat" => "iNaturalist",
        "rbins" => "RBINS",
        "obsorg" => "Observation.org",
        "borror" => "Borror Lab",
        "humboldt" => "Humboldt Institute",
        "commons" => "Wikimedia Commons",
        "gbif" => "GBIF",
        "demo" => "Oversample demos",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_a_tenth_of_a_degree() {
        assert_eq!(round_point(51.5074, -0.1278), (51.5, -0.1));
        assert_eq!(round_point(-33.86, 151.21), (-33.9, 151.2));
    }

    #[test]
    fn parses_contract_shapes() {
        let species: SpeciesPage = serde_json::from_str(
            r#"{"query":"pip","total":1,"cursor":null,"species":[{"key":"t:17317","gen":"Pipistrellus","sp":"pipistrellus","rank":0,"grp":"bats","en":"Common Pipistrelle","local":null,"local_lang":null,"n":0,"sources":{},"iucn_taxon_id":17317,"wikidata":null,"wiki_en":null,"matched":{"name":"pipistrelle","type":"common","language":"en","exact":false}}],"status":{"xc":"ready"},"names":true}"#,
        )
        .unwrap();
        assert_eq!(species.species[0].n, 0);
        assert_eq!(species.species[0].scientific(), "Pipistrellus pipistrellus");

        let recs: RecordingsPage = serde_json::from_str(
            r#"{"total":1,"total_exact":true,"by_source":{"xc":1},"cursor":null,"truncated":false,"items":[{"id":"xc:928094","source":"xc","gen":"Pipistrellus","sp":"pipistrellus","en":"Common Pipistrelle","cc":"GB","country":"United Kingdom","place":"Kew","lat":51.48,"lon":-0.29,"km":12.3,"smp":384000,"len_s":9.6,"date":"2024-06-01","recordist":"A","licence":"x","attribution":null,"quality":"A","demo":false,"time_expansion":null,"page_url":"p","audio_url":"a","audio_via":"xc"}],"status":{"xc":"ready"}}"#,
        )
        .unwrap();
        assert_eq!(recs.items[0].xc_number(), Some(928094));
    }
}

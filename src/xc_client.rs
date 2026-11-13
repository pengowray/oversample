//! Where the Xeno-Canto browser (`components::xc_browser`) gets its data.
//!
//! Three sources, chosen by [`resolve_source`]:
//!
//! - [`XcSource::Native`]: the desktop/Android app with a key saved natively.
//!   Calls the Tauri `xc_*` commands (reqwest, on-disk cache), as before.
//! - [`XcSource::Direct`]: the web build with the user's own key saved in
//!   localStorage. Calls `xeno-canto.org/api/3` from the browser. XC echoes
//!   the request Origin in `access-control-allow-origin` for the API and sends
//!   `*` for audio downloads, so no proxy is needed.
//! - [`XcSource::Proxy`]: any platform, no key of the user's own. Calls the
//!   Oversample proxy, which adds a server key. The proxy rejects any request
//!   that carries a `key` parameter, so none is ever sent to it.
//!
//! The Oversample server also keeps an index of every recording in each
//! group. Species lists come from its `/xc/taxonomy/{group}` endpoint in all
//! three modes, after the local caches. While the server is still building a
//! group's index it answers 202 with its progress; in Proxy mode
//! [`browse_group`] asks again every few seconds. With the user's own key, a
//! 202, a timeout or any failure from the server moves straight on to the
//! local build, and Refresh skips the server, so those users never depend on
//! the Oversample server. In Proxy mode the species' recordings also come
//! from the index (`/xc/recordings/{group}`), and the country picker uses
//! `/xc/countries/{group}` in every mode.
//!
//! Direct and Proxy share one code path: the same JSON parsing as the native
//! side (`xc_lib::api`), a species-list builder (`xc_lib::taxonomy`) driven
//! page by page from here, and a taxonomy cache in OPFS that all sources use.
//! Native mode also reads the Tauri disk cache and falls back to the Tauri
//! commands.
//!
//! The pure helpers (URL building, error classification, fallback decisions)
//! are unit-tested on the host; the fetch wrappers are thin.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use oversample_ipc::xc::{
    XcCachedFile, XcGroupArgs, XcGroupTaxonomy, XcIdArgs, XcRecording, XcSearchArgs,
    XcSearchResult, XcSetApiKeyArgs, XcSpeciesArgs,
};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::settings::{self, keys};
use crate::tauri_bridge::{tauri_invoke, tauri_invoke_typed_args, tauri_invoke_with_args};

/// Xeno-Canto's search endpoint, used with the user's own key.
const XC_API_URL: &str = "https://xeno-canto.org/api/3/recordings";

/// Proxy base URL baked in at build time. Set `OVERSAMPLE_API_BASE` when
/// building to point a build at another proxy.
const DEFAULT_API_BASE: &str = match option_env!("OVERSAMPLE_API_BASE") {
    Some(base) => base,
    None => "https://api.oversample.com",
};

/// Results per page for searches and species lists (matches the native side).
const PER_PAGE: u32 = 100;
/// Results per page when building a group's species list (the API maximum).
const TAXONOMY_PER_PAGE: u32 = 500;
/// Minimum gap between requests from the browser. XC asks for at most one
/// request per second; the proxy refills one search token per second.
const MIN_REQUEST_GAP_MS: f64 = 1100.0;
/// Seconds to wait before asking the server again while it builds an index,
/// when its reply has no usable `Retry-After` header.
const DEFAULT_RETRY_SECS: u32 = 5;
/// Limits on the server's `Retry-After`, so a bad value neither floods the
/// server nor leaves the user waiting for minutes.
const MIN_RETRY_SECS: u32 = 2;
const MAX_RETRY_SECS: u32 = 60;
/// How long a request to one of the server's index endpoints may take
/// before it is cancelled.
const SERVER_TIMEOUT_MS: i32 = 10_000;
/// With the user's own key, a species list estimated to take longer than
/// this stops after page 1 and returns [`XcError::LongBuild`] so the user can
/// confirm.
const LONG_BUILD_SECS: f64 = 600.0;
/// OPFS folder for cached species lists (web paths only).
const TAXONOMY_OPFS_DIR: &str = "oversample-xc";

/// Where XC requests go. `Debug` hides the key so it never reaches a log.
#[derive(Clone, PartialEq)]
pub enum XcSource {
    /// Tauri commands with the natively saved key.
    Native,
    /// Browser fetch to xeno-canto.org with the user's key.
    Direct { key: String },
    /// Browser fetch to the Oversample proxy, no user key.
    Proxy,
}

impl std::fmt::Debug for XcSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XcSource::Native => f.write_str("Native"),
            XcSource::Direct { .. } => f.write_str("Direct { key: <hidden> }"),
            XcSource::Proxy => f.write_str("Proxy"),
        }
    }
}

impl XcSource {
    /// True when requests use a key the user entered (so a 401/403 means
    /// that key is wrong).
    pub fn uses_own_key(&self) -> bool {
        !matches!(self, XcSource::Proxy)
    }
}

/// What went wrong, classified so the browser can show a specific message.
#[derive(Clone, Debug, PartialEq)]
pub enum XcError {
    /// The request never got a response (offline, DNS, blocked by CORS).
    Network,
    /// HTTP 429.
    RateLimited,
    /// HTTP 503: the proxy is busy or has no server key configured.
    Busy(Option<String>),
    /// HTTP 502: the proxy could not reach Xeno-Canto, or XC failed.
    Upstream(Option<String>),
    /// HTTP 401/403 while using the user's own key.
    BadKey(Option<String>),
    /// HTTP 413: the file is larger than the proxy will relay. [`download`]
    /// fills in the recording's id.
    TooLarge { id: Option<u64> },
    /// Own key on the web: building this species list would take longer than
    /// [`LONG_BUILD_SECS`]. Call [`browse_group`] again with `allow_long` to
    /// go ahead.
    LongBuild {
        group: String,
        country: Option<String>,
        recordings: u32,
        est_secs: f64,
    },
    /// The caller stopped a species-list build (dialog closed, new request).
    Cancelled,
    /// Any other non-2xx response, with the server's message if it gave one.
    Http {
        status: u16,
        message: Option<String>,
    },
    /// Everything else: native command errors, unexpected JSON.
    Other(String),
}

/// Result of downloading one recording, ready for `load_named_bytes`.
pub struct DownloadedRecording {
    pub filename: String,
    pub bytes: Vec<u8>,
    pub metadata: Vec<(String, String)>,
    /// Reference hashes from the native cache's sidecar. `None` on the web
    /// paths: the loader computes the file's identity from the bytes itself,
    /// and hashing a fresh download to compare it with itself proves nothing.
    pub hashes: Option<crate::state::SidecarHashes>,
}

/// Progress of a species-list build on the web paths, reported after each page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildProgress {
    pub pages_done: u32,
    pub total_pages: u32,
    /// Recordings read so far (the species list counts each one).
    pub recordings_done: u32,
    pub total_recordings: u32,
    /// Estimated time to fetch the remaining pages, from the average time per
    /// page so far in this build.
    pub secs_left: f64,
}

/// The Oversample server's progress while it builds a group's index (the
/// body of its 202 reply). `pages_total` is 0 until the server knows it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
pub struct ServerIndexing {
    pub pages_done: u32,
    pub pages_total: u32,
}

/// Progress of a species list, for the progress line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TaxonomyProgress {
    /// The list is being built here, page by page (own key, web).
    Local(BuildProgress),
    /// The Oversample server is still indexing the group; we wait for it.
    Server(ServerIndexing),
}

/// One country in a group, from the server's `/xc/countries/{group}`.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct CountryCount {
    /// XC's country name, as in a recording's `cnt` field.
    pub country: String,
    pub recordings: u32,
    pub species: u32,
}

/// A species the Oversample server found by name (`/xc/species`).
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct NamedSpecies {
    /// XC group, such as "bats" or "land mammals".
    pub group: String,
    #[serde(rename = "gen")]
    pub genus: String,
    pub sp: String,
    /// XC's English name.
    pub en: String,
    pub recording_count: u32,
    /// The common name in the user's language, when asked for and known.
    #[serde(default)]
    pub local: Option<String>,
    /// The language of `local`.
    #[serde(default)]
    pub local_lang: Option<String>,
    /// The name that matched the query.
    #[serde(rename = "match")]
    pub matched: NameMatch,
}

/// The name a species was found by.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct NameMatch {
    pub name: String,
    /// "scientific", "synonym" or "common".
    #[serde(rename = "type")]
    pub kind: String,
    /// Language code of a common name ("de"), when known.
    pub language: Option<String>,
    /// "xc" for XC's own names, else where the name comes from.
    pub source: String,
}

/// Species found by name, best match first.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct SpeciesSearch {
    pub species: Vec<NamedSpecies>,
    /// Groups the server is still indexing for the first time.
    #[serde(default)]
    pub groups_indexing: Vec<String>,
}

/// True for text written in XC's query syntax (`grp:bats cnt:Australia`),
/// which goes to Xeno-canto's own search rather than the name search.
pub fn is_xc_query(text: &str) -> bool {
    text.contains(':')
}

/// Species whose scientific name, synonym or common name in any language
/// matches `query`, in `group` or in all groups, from the Oversample server.
/// Works with any source: the server answers from its own index and never
/// contacts Xeno-canto.
pub async fn find_species(query: &str, group: Option<&str>) -> Result<SpeciesSearch, XcError> {
    let mut url = format!(
        "{}/xc/species?q={}",
        proxy_base(),
        String::from(js_sys::encode_uri_component(query))
    );
    if let Some(g) = group {
        url.push_str("&group=");
        url.push_str(&String::from(js_sys::encode_uri_component(g)));
    }
    let langs = names_language().codes();
    if !langs.is_empty() {
        url.push_str("&lang=");
        url.push_str(&langs.join(","));
    }
    // Not throttled: it runs as the user types, and the server rate-limits it.
    match server_get(&url, false).await? {
        ServerReply::Ready(body) => serde_json::from_str(&body)
            .map_err(|e| XcError::Other(format!("unexpected species search reply: {e}"))),
        ServerReply::Indexing { .. } => Ok(SpeciesSearch {
            species: Vec::new(),
            groups_indexing: Vec::new(),
        }),
    }
}

/// Which language Explore Xeno-canto shows species names in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamesLanguage {
    /// The browser's preferred languages, in order, up to the first English
    /// one. The default.
    Browser,
    /// Xeno-canto's English names only.
    English,
    /// One language, such as "de", then English.
    One(String),
}

impl NamesLanguage {
    /// From the stored setting: absent or "browser", "en", or a language code.
    pub fn from_stored(v: Option<&str>) -> Self {
        match v.map(str::trim) {
            None | Some("") | Some("browser") => Self::Browser,
            Some(v) => match language_code(v) {
                Some(code) => Self::One(code),
                None if v.eq_ignore_ascii_case("en") => Self::English,
                None => Self::Browser,
            },
        }
    }

    /// The value stored, and used as the `<select>` value.
    pub fn stored(&self) -> &str {
        match self {
            Self::Browser => "browser",
            Self::English => "en",
            Self::One(code) => code,
        }
    }

    /// The language codes to ask the server for, in order. Empty for English.
    pub fn codes(&self) -> Vec<String> {
        match self {
            Self::Browser => browser_languages(),
            Self::English => Vec::new(),
            Self::One(code) => vec![code.clone()],
        }
    }
}

pub fn names_language() -> NamesLanguage {
    NamesLanguage::from_stored(
        crate::settings::get_raw(crate::settings::keys::XC_NAMES_LANGUAGE).as_deref(),
    )
}

pub fn set_names_language(choice: &NamesLanguage) {
    match choice {
        NamesLanguage::Browser => {
            crate::settings::remove_raw(crate::settings::keys::XC_NAMES_LANGUAGE)
        }
        other => crate::settings::set_raw(crate::settings::keys::XC_NAMES_LANGUAGE, other.stored()),
    }
}

/// Most languages sent to the server; it uses no more.
const MAX_LANGUAGES: usize = 5;

/// The browser's preferred languages as language codes, in order, without
/// repeats, up to the first English one ("nl-BE, nl, de, en-US, fr" gives
/// "nl", "de"). Empty when English comes first.
pub fn browser_languages() -> Vec<String> {
    let Some(nav) = web_sys::window().map(|w| w.navigator()) else {
        return Vec::new();
    };
    let mut tags: Vec<String> = nav
        .languages()
        .iter()
        .filter_map(|v| v.as_string())
        .collect();
    if tags.is_empty() {
        tags.extend(nav.language());
    }
    languages_from_tags(&tags)
}

fn languages_from_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let primary = tag.trim().split(['-', '_']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("en") {
            break;
        }
        if let Some(code) = language_code(tag) {
            if !out.contains(&code) {
                out.push(code);
            }
        }
    }
    out.truncate(MAX_LANGUAGES);
    out
}

/// "de-AT" gives "de"; English, or a tag that doesn't start with a 2 or 3
/// letter code, gives `None`.
fn language_code(tag: &str) -> Option<String> {
    let primary = tag.trim().split(['-', '_']).next()?.to_ascii_lowercase();
    let valid =
        (2..=3).contains(&primary.len()) && primary.chars().all(|c| c.is_ascii_alphabetic());
    (valid && primary != "en").then_some(primary)
}

/// A species' common name in a language other than English.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalName {
    pub name: String,
    /// Its language code, such as "de".
    pub lang: String,
}

#[derive(serde::Deserialize)]
struct CommonNamesReply {
    names: std::collections::HashMap<String, String>,
    /// The language of each name not in the first language asked for.
    #[serde(default)]
    languages: std::collections::HashMap<String, String>,
}

/// The common names of the species of `group` (keyed "Genus species", as
/// Xeno-canto writes them), each in the first of `langs` that has one, from
/// the Oversample server. Works with any source. `None` if the server can't
/// give them (unreachable, an older server, or still indexing the group).
pub async fn common_names(
    group: &str,
    langs: &[String],
) -> Option<std::collections::HashMap<String, LocalName>> {
    let first = langs.first()?;
    let url = format!(
        "{}/xc/common-names/{}?lang={}",
        proxy_base(),
        String::from(js_sys::encode_uri_component(group)),
        langs.join(",")
    );
    match server_get(&url, false).await {
        Ok(ServerReply::Ready(body)) => match serde_json::from_str::<CommonNamesReply>(&body) {
            Ok(mut r) => Some(
                r.names
                    .into_iter()
                    .map(|(sp, name)| {
                        let lang = r.languages.remove(&sp).unwrap_or_else(|| first.clone());
                        (sp, LocalName { name, lang })
                    })
                    .collect(),
            ),
            Err(e) => {
                log::warn!("Common names from server not read: {e}");
                None
            }
        },
        Ok(ServerReply::Indexing { .. }) => None,
        Err(e) => {
            log::warn!("Common names from server failed: {e:?}");
            None
        }
    }
}

#[derive(serde::Deserialize)]
struct NameLanguagesReply {
    languages: Vec<NameLanguageEntry>,
}

#[derive(serde::Deserialize)]
struct NameLanguageEntry {
    code: String,
}

/// Xeno-Canto's English names, keyed by "Genus species", for every species in
/// `groups` that has recordings there, from the species lists the XC browser
/// keeps in OPFS. With `fetch`, a group missing there comes from the Oversample
/// server (not waiting while it is still indexing the group) and is saved for
/// next time; this takes a few seconds, since server requests are throttled.
/// Returns the names, and whether every group was found. Used to name Perch's
/// results, whose labels are scientific names only.
pub async fn english_names(groups: &[&str], fetch: bool) -> (HashMap<String, String>, bool) {
    let mut out = HashMap::new();
    let mut complete = true;
    for group in groups {
        let key = taxonomy_cache_key(group, None);
        let mut taxonomy = load_cached_taxonomy(&key).await;
        if taxonomy.is_none() && fetch {
            match server_taxonomy(group, None, false, &mut |_| {}, &|| true).await {
                Ok(Some(t)) => {
                    save_cached_taxonomy(&key, &t).await;
                    taxonomy = Some(t);
                }
                Ok(None) => {}
                Err(e) => log::info!("no English names for {group}: {e:?}"),
            }
        }
        complete &= taxonomy.is_some();
        for s in taxonomy.iter().flat_map(|t| &t.species) {
            if !s.en.is_empty() {
                out.insert(format!("{} {}", s.genus, s.sp), s.en.clone());
            }
        }
    }
    (out, complete)
}

/// English names from the Oversample server's names file
/// (`/names/english/{group}`): every species of a group it knows, with or
/// without Xeno-Canto recordings, keyed by lowercase "genus species" (accepted
/// names and synonyms of one species). Stored in OPFS like the species lists;
/// with `fetch`, a group missing there is fetched and saved. Returns the names
/// and whether every group was found. An older server without the route gives
/// none.
pub async fn names_file_english(groups: &[&str], fetch: bool) -> (HashMap<String, String>, bool) {
    #[derive(serde::Deserialize)]
    struct Reply {
        names: HashMap<String, String>,
    }
    let mut out = HashMap::new();
    let mut complete = true;
    for group in groups {
        let key = format!("english-{}", taxonomy_cache_key(group, None));
        let mut body = crate::opfs::opfs_load_in(TAXONOMY_OPFS_DIR, &key)
            .await
            .ok()
            .flatten();
        if body.is_none() && fetch {
            let url = format!(
                "{}/names/english/{}",
                proxy_base(),
                group.replace(' ', "%20")
            );
            match server_get(&url, false).await {
                Ok(ServerReply::Ready(b)) if serde_json::from_str::<Reply>(&b).is_ok() => {
                    if let Err(e) = crate::opfs::opfs_save_in(TAXONOMY_OPFS_DIR, &key, &b).await {
                        log::warn!("English names not cached: {e}");
                    }
                    body = Some(b);
                }
                Ok(_) => {}
                Err(e) => log::info!("no names-file English names for {group}: {e:?}"),
            }
        }
        match body.and_then(|b| serde_json::from_str::<Reply>(&b).ok()) {
            Some(r) => out.extend(r.names),
            None => complete = false,
        }
    }
    (out, complete)
}

/// The language codes the server has common names in. `None` if the server
/// can't say (unreachable, or an older server).
pub async fn name_language_list() -> Option<Vec<String>> {
    let url = format!("{}/xc/name-languages", proxy_base());
    match server_get(&url, false).await {
        Ok(ServerReply::Ready(body)) => serde_json::from_str::<NameLanguagesReply>(&body)
            .ok()
            .map(|r| r.languages.into_iter().map(|l| l.code).collect()),
        _ => None,
    }
}

/// A reply from one of the server's index endpoints.
#[derive(Debug, PartialEq)]
pub(crate) enum ServerReply {
    /// HTTP 200 with this body.
    Ready(String),
    /// HTTP 202: the group is still being indexed. Ask again after
    /// `retry_secs` seconds.
    Indexing {
        status: ServerIndexing,
        retry_secs: u32,
    },
}

// ── Source selection and key storage ────────────────────────────────

/// The proxy base URL: the dev override from settings if set, else the build
/// default. No trailing slash.
pub fn proxy_base() -> String {
    let base = settings::get_raw(keys::XC_API_BASE_OVERRIDE)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    base.trim_end_matches('/').to_string()
}

/// The source that uses the user's own saved key, if one is saved.
pub async fn own_key_source(is_tauri: bool) -> Option<XcSource> {
    if is_tauri {
        let val = crate::tauri_bridge::tauri_invoke_no_args("xc_get_api_key")
            .await
            .ok()?;
        let key = val.as_string().unwrap_or_default();
        (!key.trim().is_empty()).then_some(XcSource::Native)
    } else {
        let key = settings::get_raw(keys::XC_WEB_API_KEY)?;
        let key = key.trim().to_string();
        (!key.is_empty()).then_some(XcSource::Direct { key })
    }
}

/// The source to use now: the proxy if the user chose it, else their own key,
/// else `None` (ask for a key or offer the proxy).
pub async fn resolve_source(is_tauri: bool) -> Option<XcSource> {
    if settings::get_bool(keys::XC_USE_PROXY, false) {
        return Some(XcSource::Proxy);
    }
    own_key_source(is_tauri).await
}

/// Remember whether the user wants the proxy instead of their own key.
pub fn set_use_proxy(use_proxy: bool) {
    if use_proxy {
        settings::set_bool(keys::XC_USE_PROXY, true);
    } else {
        settings::remove_raw(keys::XC_USE_PROXY);
    }
}

/// Save the user's key and switch to using it.
///
/// Checks the key first with one small search sent from the browser (XC
/// allows that from any origin, including the Tauri webview). Returns
/// [`XcError::BadKey`] without saving if XC rejects it. Any other failure,
/// such as being offline, is logged and the key is saved anyway.
pub async fn save_key(is_tauri: bool, key: &str) -> Result<XcSource, XcError> {
    let key = key.trim().to_string();
    let probe = XcSource::Direct { key: key.clone() };
    match web_search(&probe, "grp:bats", 1, 50).await {
        Ok(_) => {}
        Err(e @ XcError::BadKey(_)) => return Err(e),
        Err(e) => log::warn!("XC key not checked: {e:?}"),
    }
    let source = if is_tauri {
        tauri_invoke_with_args("xc_set_api_key", &XcSetApiKeyArgs { key })
            .await
            .map_err(XcError::Other)?;
        XcSource::Native
    } else {
        settings::set_raw(keys::XC_WEB_API_KEY, &key);
        XcSource::Direct { key }
    };
    set_use_proxy(false);
    Ok(source)
}

/// Forget the user's key. On desktop this deletes the key file shared with
/// `xc-cli`.
pub async fn remove_key(is_tauri: bool) -> Result<(), XcError> {
    if is_tauri {
        crate::tauri_bridge::tauri_invoke_no_args("xc_delete_api_key")
            .await
            .map_err(XcError::Other)?;
    } else {
        settings::remove_raw(keys::XC_WEB_API_KEY);
    }
    Ok(())
}

// ── Operations ───────────────────────────────────────────────────────

/// Search with XC query syntax. Plain text is treated as a species name.
pub async fn search(src: &XcSource, query: &str, page: u32) -> Result<XcSearchResult, XcError> {
    match src {
        XcSource::Native => tauri_invoke_typed_args(
            "xc_search",
            &XcSearchArgs {
                query: query.to_string(),
                page: Some(page),
            },
        )
        .await
        .map_err(XcError::Other),
        _ => web_search(src, query, page, PER_PAGE).await,
    }
}

/// One recording by its XC number, from whichever source the user chose.
/// The proxy answers from its index (`/xc/recording/{id}`) and asks XC only
/// for recordings it has not indexed; the other sources search `nr:{id}`.
pub async fn recording_by_id(src: &XcSource, id: u64) -> Result<XcRecording, XcError> {
    if matches!(src, XcSource::Proxy) {
        let url = format!("{}/xc/recording/{id}", proxy_base());
        if let ServerReply::Ready(body) = server_get(&url, false).await? {
            let value: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| XcError::Other(format!("unexpected response: {e}")))?;
            return xc_lib::api::parse_recording(&value)
                .ok_or_else(|| XcError::Other(format!("XC{id}: unexpected record")));
        }
    }
    search(src, &format!("nr:{id}"), 1)
        .await?
        .recordings
        .into_iter()
        .next()
        .ok_or_else(|| XcError::Http {
            status: 404,
            message: Some(format!("XC{id} was not found on Xeno-canto")),
        })
}

/// One page of recordings of a species in `group`.
///
/// Proxy mode reads the server's index (`/xc/recordings/{group}`), so
/// browsing does not send searches to Xeno-canto. If the server is still
/// indexing the group (202) or has no such endpoint (404), it falls back to a
/// live search through the proxy. A 503 (the server has no index for the
/// group) does the same. Native and Direct search XC as before.
///
/// `country` limits the list to recordings from that country (the country
/// the species list was built for).
pub async fn species_recordings(
    src: &XcSource,
    group: &str,
    genus: &str,
    species: &str,
    country: Option<&str>,
    page: u32,
) -> Result<XcSearchResult, XcError> {
    let query = species_query(genus, species, country);
    match src {
        // The Tauri species command has no country argument, so a country
        // goes through the general search command instead.
        XcSource::Native if country.is_some() => search(src, &query, page).await,
        XcSource::Native => tauri_invoke_typed_args(
            "xc_species_recordings",
            &XcSpeciesArgs {
                genus: genus.to_string(),
                species: species.to_string(),
                page: Some(page),
            },
        )
        .await
        .map_err(XcError::Other),
        XcSource::Proxy => {
            let url = server_recordings_url(
                &proxy_base(),
                group,
                genus,
                species,
                country,
                page,
                PER_PAGE,
            );
            match server_get(&url, true).await {
                Ok(ServerReply::Ready(body)) => parse_search_body(&body),
                Ok(ServerReply::Indexing { .. }) => web_search(src, &query, page, PER_PAGE).await,
                Err(e) if index_unavailable(&e) => {
                    log::warn!("Server recordings index unavailable, searching live: {e:?}");
                    web_search(src, &query, page, PER_PAGE).await
                }
                Err(e) => Err(e),
            }
        }
        XcSource::Direct { .. } => web_search(src, &query, page, PER_PAGE).await,
    }
}

/// The species list for a group, optionally limited to one country.
///
/// Order of sources:
/// 1. Unless `refresh` is set: the cached list (OPFS), then in Native mode
///    the Tauri disk cache (shared with `xc-cli`).
/// 2. The Oversample server's index (`/xc/taxonomy/{group}`). It is skipped
///    when the user has their own key and pressed Refresh, because the
///    server updates its index only once a day.
///    - Proxy mode: while the server answers 202 it reports
///      [`TaxonomyProgress::Server`] and asks again every few seconds until
///      the list is ready or `keep_going()` returns false
///      ([`XcError::Cancelled`]). A server failure is returned as the error.
///    - Own key (Native, Direct): a 202 or any failure (see
///      [`falls_back_to_local`]) moves on to step 3 straight away, so these
///      users never wait for the server.
/// 3. With the user's own key, the list is built locally. Native calls the
///    Tauri command. Direct pages through every recording in the group from
///    the browser, reporting [`TaxonomyProgress::Local`] after each page;
///    a build that would take over [`LONG_BUILD_SECS`] stops after page 1
///    with [`XcError::LongBuild`] unless `allow_long` is set.
///
/// Every list returned from steps 2 and 3 is saved in OPFS.
#[allow(clippy::too_many_arguments)]
pub async fn browse_group(
    src: &XcSource,
    group: &str,
    country: Option<&str>,
    refresh: bool,
    allow_long: bool,
    mut progress: impl FnMut(TaxonomyProgress),
    keep_going: impl Fn() -> bool,
) -> Result<XcGroupTaxonomy, XcError> {
    let cache_key = taxonomy_cache_key(group, country);
    if !refresh {
        if let Some(cached) = load_cached_taxonomy(&cache_key).await {
            return Ok(cached);
        }
        if *src == XcSource::Native && native_cache_exists(group, country).await {
            return native_taxonomy("xc_browse_group", group, country).await;
        }
    }

    if uses_server_taxonomy(src, refresh) {
        let wait = !src.uses_own_key();
        match server_taxonomy(group, country, wait, &mut progress, &keep_going).await {
            Ok(Some(taxonomy)) => {
                save_cached_taxonomy(&cache_key, &taxonomy).await;
                return Ok(taxonomy);
            }
            Ok(None) => {
                log::info!("Oversample server is still indexing {group}, building the list here");
            }
            Err(e) if falls_back_to_local(src, &e) => {
                log::warn!("Oversample server species list failed, building it here: {e:?}");
            }
            Err(e) => return Err(e),
        }
    }

    if *src == XcSource::Native {
        let cmd = if refresh {
            "xc_refresh_taxonomy"
        } else {
            "xc_browse_group"
        };
        let taxonomy = native_taxonomy(cmd, group, country).await?;
        save_cached_taxonomy(&cache_key, &taxonomy).await;
        return Ok(taxonomy);
    }

    let query = xc_lib::taxonomy::group_query(group, country);
    let acc = build_taxonomy(
        allow_long,
        group,
        country,
        |page| web_search(src, &query, page, TAXONOMY_PER_PAGE),
        js_sys::Date::now,
        &mut |p| progress(TaxonomyProgress::Local(p)),
        &keep_going,
    )
    .await?;

    let now: String = js_sys::Date::new_0().to_iso_string().into();
    let taxonomy = acc.finish(now);
    save_cached_taxonomy(&cache_key, &taxonomy).await;
    Ok(taxonomy)
}

/// Seconds since `taxonomy` was built, from its `last_updated` timestamp.
/// The server, xc-lib and the browser build all write RFC 3339 times. For a
/// list from the server this is the time of the server's last update of the
/// group, not the time it was downloaded.
pub fn taxonomy_age_secs(taxonomy: &XcGroupTaxonomy) -> Option<f64> {
    let built_ms = js_sys::Date::parse(&taxonomy.last_updated);
    if built_ms.is_nan() {
        return None;
    }
    Some(((js_sys::Date::now() - built_ms) / 1000.0).max(0.0))
}

/// The countries with recordings in `group`, with their counts, from the
/// server's index. `None` if the server is unreachable, fails, or is still
/// indexing the group; the caller then shows its built-in country list.
pub async fn group_countries(group: &str) -> Option<Vec<CountryCount>> {
    let url = server_countries_url(&proxy_base(), group);
    match server_get(&url, true).await {
        Ok(ServerReply::Ready(body)) => match parse_countries(&body) {
            Ok(list) => Some(list),
            Err(e) => {
                log::warn!("Country list from server not read: {e}");
                None
            }
        },
        Ok(ServerReply::Indexing { .. }) => None,
        Err(e) => {
            log::warn!("Country list from server failed: {e:?}");
            None
        }
    }
}

/// Download a recording's audio with everything the loader needs.
pub async fn download(src: &XcSource, rec: &XcRecording) -> Result<DownloadedRecording, XcError> {
    let id = rec.id_num();
    if *src == XcSource::Native {
        let cached: XcCachedFile = tauri_invoke_typed_args("xc_download", &XcIdArgs { id })
            .await
            .map_err(XcError::Other)?;
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &"path".into(), &cached.path.as_str().into()).ok();
        let val = tauri_invoke("read_file_bytes", &args.into())
            .await
            .map_err(XcError::Other)?;
        let ab: js_sys::ArrayBuffer = val
            .dyn_into()
            .map_err(|_| XcError::Other("read_file_bytes did not return ArrayBuffer".into()))?;
        return Ok(DownloadedRecording {
            filename: cached.filename,
            bytes: js_sys::Uint8Array::new(&ab).to_vec(),
            metadata: cached.metadata,
            hashes: cached.hashes,
        });
    }

    let url = audio_url(src, &proxy_base(), rec);
    let bytes = get_bytes(&url, src.uses_own_key())
        .await
        .map_err(|e| match e {
            XcError::TooLarge { .. } => XcError::TooLarge { id: Some(id) },
            e => e,
        })?;
    Ok(DownloadedRecording {
        filename: format!(
            "{}.{}",
            xc_lib::cache::recording_stem(rec),
            xc_lib::cache::audio_extension(rec)
        ),
        bytes,
        metadata: xc_lib::metadata::recording_to_metadata(rec),
        hashes: None,
    })
}

/// Which of `ids` are already saved locally. Only the native app keeps
/// downloads, so the web paths always return an empty set.
pub async fn cached_ids(src: &XcSource, ids: Vec<u64>) -> HashSet<u64> {
    let mut set = HashSet::new();
    if *src != XcSource::Native {
        return set;
    }
    for id in ids {
        if let Ok(val) = tauri_invoke_with_args("xc_is_cached", &XcIdArgs { id }).await {
            if val.as_bool().unwrap_or(false) {
                set.insert(id);
            }
        }
    }
    set
}

// ── Web paths ────────────────────────────────────────────────────────

async fn web_search(
    src: &XcSource,
    query: &str,
    page: u32,
    per_page: u32,
) -> Result<XcSearchResult, XcError> {
    let url = search_url(src, &proxy_base(), query, page, per_page)
        .ok_or_else(|| XcError::Other("no web source".into()))?;
    let text = get_text(&url, src.uses_own_key()).await?;
    parse_search_body(&text)
}

/// Parse an XC search response, or the server's `/xc/recordings` reply,
/// which has the same shape.
fn parse_search_body(text: &str) -> Result<XcSearchResult, XcError> {
    let body: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| XcError::Other(format!("unexpected response: {e}")))?;
    xc_lib::api::parse_search_response(&body).map_err(XcError::Other)
}

/// Native mode: run a Tauri species-list command (`xc_browse_group` or
/// `xc_refresh_taxonomy`).
async fn native_taxonomy(
    cmd: &str,
    group: &str,
    country: Option<&str>,
) -> Result<XcGroupTaxonomy, XcError> {
    tauri_invoke_typed_args(
        cmd,
        &XcGroupArgs {
            group: group.to_string(),
            country: country.map(str::to_string),
        },
    )
    .await
    .map_err(XcError::Other)
}

/// Native mode: whether the Tauri disk cache has a list for this group and
/// country. `xc_taxonomy_age` reads the cached file and returns `None` when
/// there is none.
async fn native_cache_exists(group: &str, country: Option<&str>) -> bool {
    let age: Result<Option<String>, String> = tauri_invoke_typed_args(
        "xc_taxonomy_age",
        &XcGroupArgs {
            group: group.to_string(),
            country: country.map(str::to_string),
        },
    )
    .await;
    matches!(age, Ok(Some(_)))
}

/// The species list from the server's index. With `wait`, asks again while
/// the server answers 202. Without it, a 202 returns `Ok(None)`.
async fn server_taxonomy(
    group: &str,
    country: Option<&str>,
    wait: bool,
    progress: &mut impl FnMut(TaxonomyProgress),
    keep_going: &impl Fn() -> bool,
) -> Result<Option<XcGroupTaxonomy>, XcError> {
    let url = server_taxonomy_url(&proxy_base(), group, country);
    let Some(body) = poll_until_ready(
        || server_get(&url, true),
        |secs| crate::web_util::sleep_ms((secs * 1000) as i32),
        wait,
        progress,
        keep_going,
    )
    .await?
    else {
        return Ok(None);
    };
    serde_json::from_str(&body)
        .map(Some)
        .map_err(|e| XcError::Other(format!("unexpected species list from server: {e}")))
}

/// GET one of the server's index endpoints. A 202 is returned as
/// [`ServerReply::Indexing`]; any other non-2xx status is an error. The
/// server never sees the user's key, so a 401/403 is not
/// [`XcError::BadKey`].
///
/// The request is cancelled after [`SERVER_TIMEOUT_MS`] (counted after the
/// throttle wait) and reported as [`XcError::Network`], so with the user's
/// own key a slow server does not hold up the local build for long.
///
/// `throttled` waits for the shared request slot first, as XC requests do.
pub(crate) async fn server_get(url: &str, throttled: bool) -> Result<ServerReply, XcError> {
    use wasm_bindgen::closure::Closure;

    if throttled {
        throttle().await;
    }
    let window = web_sys::window().ok_or_else(|| XcError::Other("no window".into()))?;
    let controller =
        web_sys::AbortController::new().map_err(|_| XcError::Other("no AbortController".into()))?;
    let signal = controller.signal();
    let abort = Closure::once(move || controller.abort());
    let timer = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            abort.as_ref().unchecked_ref(),
            SERVER_TIMEOUT_MS,
        )
        .ok();

    let result = async {
        let resp = fetch_unthrottled(&window, url, Some(&signal)).await?;
        let status = resp.status();
        // Only readable if the server lists it in Access-Control-Expose-Headers.
        let retry_after = resp.headers().get("Retry-After").ok().flatten();
        let promise = resp.text().map_err(|_| XcError::Network)?;
        // A timeout while the body is read rejects this promise.
        let body = JsFuture::from(promise)
            .await
            .map_err(|_| XcError::Network)?
            .as_string()
            .unwrap_or_default();
        server_reply(status, body, retry_after.as_deref())
    }
    .await;

    if let Some(t) = timer {
        window.clear_timeout_with_handle(t);
    }
    drop(abort);
    result
}

async fn save_cached_taxonomy(cache_key: &str, taxonomy: &XcGroupTaxonomy) {
    match serde_json::to_string(taxonomy) {
        Ok(json) => {
            if let Err(e) = crate::opfs::opfs_save_in(TAXONOMY_OPFS_DIR, cache_key, &json).await {
                log::warn!("XC species list not cached: {e}");
            }
        }
        Err(e) => log::warn!("XC species list not cached: {e}"),
    }
}

async fn load_cached_taxonomy(cache_key: &str) -> Option<XcGroupTaxonomy> {
    let json = crate::opfs::opfs_load_in(TAXONOMY_OPFS_DIR, cache_key)
        .await
        .ok()??;
    serde_json::from_str(&json).ok()
}

thread_local! {
    /// When the next browser request may start (ms since epoch).
    static NEXT_REQUEST_MS: Cell<f64> = const { Cell::new(0.0) };
}

/// Wait for this request's slot so requests start at least
/// [`MIN_REQUEST_GAP_MS`] apart, even when several are in flight.
async fn throttle() {
    let now = js_sys::Date::now();
    let slot = NEXT_REQUEST_MS.with(|next| {
        let (slot, new_next) = reserve_slot(next.get(), now, MIN_REQUEST_GAP_MS);
        next.set(new_next);
        slot
    });
    let wait = slot - now;
    if wait > 0.0 {
        crate::web_util::sleep_ms(wait.ceil() as i32).await;
    }
}

/// Wait for a request slot, then GET `url`. Any HTTP status is returned as
/// a response; only a failed connection is an error.
async fn fetch(url: &str) -> Result<web_sys::Response, XcError> {
    throttle().await;
    let window = web_sys::window().ok_or_else(|| XcError::Other("no window".into()))?;
    fetch_unthrottled(&window, url, None).await
}

/// GET `url` now, with an optional abort signal. An aborted request is
/// [`XcError::Network`].
async fn fetch_unthrottled(
    window: &web_sys::Window,
    url: &str,
    signal: Option<&web_sys::AbortSignal>,
) -> Result<web_sys::Response, XcError> {
    let promise = match signal {
        Some(signal) => {
            let init = web_sys::RequestInit::new();
            init.set_signal(Some(signal));
            window.fetch_with_str_and_init(url, &init)
        }
        None => window.fetch_with_str(url),
    };
    JsFuture::from(promise)
        .await
        .map_err(|_| XcError::Network)?
        .dyn_into()
        .map_err(|_| XcError::Other("fetch did not return a Response".into()))
}

/// The response body as text, or an empty string if it can't be read.
async fn read_text(resp: &web_sys::Response) -> String {
    match resp.text() {
        Ok(p) => JsFuture::from(p)
            .await
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// GET `url`; on a non-2xx status, read the body for the error message.
async fn get_ok(url: &str, own_key: bool) -> Result<web_sys::Response, XcError> {
    let resp = fetch(url).await?;
    if resp.ok() {
        return Ok(resp);
    }
    let body = read_text(&resp).await;
    Err(classify_http_error(resp.status(), &body, own_key))
}

async fn get_text(url: &str, own_key: bool) -> Result<String, XcError> {
    let resp = get_ok(url, own_key).await?;
    let promise = resp.text().map_err(|_| XcError::Network)?;
    let val = JsFuture::from(promise)
        .await
        .map_err(|_| XcError::Network)?;
    val.as_string()
        .ok_or_else(|| XcError::Other("response was not text".into()))
}

async fn get_bytes(url: &str, own_key: bool) -> Result<Vec<u8>, XcError> {
    let resp = get_ok(url, own_key).await?;
    let promise = resp.array_buffer().map_err(|_| XcError::Network)?;
    let buf = JsFuture::from(promise)
        .await
        .map_err(|_| XcError::Network)?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

// ── Species-list loop (host-tested with a fake fetch) ───────────────

/// What the species-list loop does after a page arrives.
#[derive(Debug, PartialEq)]
enum Step {
    /// Fetch this page next.
    Next(u32),
    /// That was the last page.
    Done,
    /// Own key: the whole build would take `est_secs`, over [`LONG_BUILD_SECS`].
    AskFirst { est_secs: f64 },
}

/// Decide the next step after `page` of `num_pages` arrived. `ms_per_page` is
/// the average time per page so far. Only page 1 can return `AskFirst`.
fn after_page(allow_long: bool, page: u32, num_pages: u32, ms_per_page: f64) -> Step {
    if page >= num_pages {
        return Step::Done;
    }
    if page == 1 && !allow_long {
        let est_secs = estimate_secs(ms_per_page, num_pages);
        if est_secs > LONG_BUILD_SECS {
            return Step::AskFirst { est_secs };
        }
    }
    Step::Next(page + 1)
}

/// Seconds to fetch `pages` pages at `ms_per_page` each. No page can take
/// less than the throttle gap.
fn estimate_secs(ms_per_page: f64, pages: u32) -> f64 {
    ms_per_page.max(MIN_REQUEST_GAP_MS) * pages as f64 / 1000.0
}

/// Page through a group search and count its species. `fetch_page(n)` gets
/// page `n`; `now_ms` reads the clock, for the time estimates.
async fn build_taxonomy<F, Fut>(
    allow_long: bool,
    group: &str,
    country: Option<&str>,
    mut fetch_page: F,
    now_ms: impl Fn() -> f64,
    progress: &mut impl FnMut(BuildProgress),
    keep_going: &impl Fn() -> bool,
) -> Result<xc_lib::taxonomy::SpeciesAccumulator, XcError>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<XcSearchResult, XcError>>,
{
    let mut acc = xc_lib::taxonomy::SpeciesAccumulator::new(group, country);
    let started = now_ms();
    let mut recordings_done = 0u32;
    let mut page = 1u32;
    loop {
        if !keep_going() {
            return Err(XcError::Cancelled);
        }
        let result = fetch_page(page).await?;
        if !keep_going() {
            return Err(XcError::Cancelled);
        }
        let ms_per_page = (now_ms() - started) / page as f64;
        let step = after_page(allow_long, page, result.num_pages, ms_per_page);
        match step {
            Step::AskFirst { est_secs } => {
                return Err(XcError::LongBuild {
                    group: group.to_string(),
                    country: country.map(str::to_string),
                    recordings: result.num_recordings,
                    est_secs,
                });
            }
            Step::Next(_) | Step::Done => {}
        }
        acc.add_page(&result);
        recordings_done = recordings_done.saturating_add(result.recordings.len() as u32);
        let total_pages = result.num_pages.max(page);
        progress(BuildProgress {
            pages_done: page,
            total_pages,
            recordings_done,
            total_recordings: result.num_recordings.max(recordings_done),
            secs_left: estimate_secs(ms_per_page, total_pages - page),
        });
        match step {
            Step::Next(next) => page = next,
            _ => return Ok(acc),
        }
    }
}

/// Ask the server until it has the data. `fetch()` makes one request;
/// `sleep(secs)` waits between them. Each 202 reply is reported to
/// `progress`. Without `wait`, the first 202 returns `Ok(None)` instead and
/// nothing is reported. Stops with [`XcError::Cancelled`] as soon as
/// `keep_going()` returns false.
async fn poll_until_ready<F, Fut, S, SFut>(
    mut fetch: F,
    mut sleep: S,
    wait: bool,
    progress: &mut impl FnMut(TaxonomyProgress),
    keep_going: &impl Fn() -> bool,
) -> Result<Option<String>, XcError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<ServerReply, XcError>>,
    S: FnMut(u32) -> SFut,
    SFut: std::future::Future<Output = ()>,
{
    loop {
        if !keep_going() {
            return Err(XcError::Cancelled);
        }
        let reply = fetch().await?;
        if !keep_going() {
            return Err(XcError::Cancelled);
        }
        match reply {
            ServerReply::Ready(body) => return Ok(Some(body)),
            ServerReply::Indexing { .. } if !wait => return Ok(None),
            ServerReply::Indexing { status, retry_secs } => {
                progress(TaxonomyProgress::Server(status));
                sleep(retry_secs).await;
            }
        }
    }
}

// ── Pure helpers (host-tested) ──────────────────────────────────────

/// With the user's own key, whether a failed server request should be
/// replaced by a local build. Any failure counts (unreachable, 404, 5xx,
/// rate limit, unreadable reply), so these users never depend on the
/// Oversample server. In Proxy mode there is nothing to fall back to.
fn falls_back_to_local(src: &XcSource, e: &XcError) -> bool {
    src.uses_own_key() && *e != XcError::Cancelled
}

/// Whether to ask the server for a species list. With the user's own key,
/// Refresh skips the server and gets the list from Xeno-canto, because the
/// server updates its index only once a day.
fn uses_server_taxonomy(src: &XcSource, refresh: bool) -> bool {
    !(refresh && src.uses_own_key())
}

/// Whether a server error from `/xc/recordings` means a live search should
/// be used instead: 404 (an older server without the endpoint) or 503 (the
/// server has no index for this group).
fn index_unavailable(e: &XcError) -> bool {
    matches!(e, XcError::Http { status: 404, .. } | XcError::Busy(_))
}

/// `{base}/xc/taxonomy/{group}`, with `?country=` when one is chosen.
fn server_taxonomy_url(base: &str, group: &str, country: Option<&str>) -> String {
    use crate::web_util::encode_component as enc;
    let mut url = format!("{base}/xc/taxonomy/{}", enc(group));
    if let Some(c) = country {
        url.push_str(&format!("?country={}", enc(c)));
    }
    url
}

/// `{base}/xc/countries/{group}`.
fn server_countries_url(base: &str, group: &str) -> String {
    format!(
        "{base}/xc/countries/{}",
        crate::web_util::encode_component(group)
    )
}

/// `{base}/xc/recordings/{group}?gen=..&sp=..[&country=..]&page=..&per_page=..`.
/// `per_page` is clamped to the server's range, 50 to 500.
fn server_recordings_url(
    base: &str,
    group: &str,
    genus: &str,
    species: &str,
    country: Option<&str>,
    page: u32,
    per_page: u32,
) -> String {
    use crate::web_util::encode_component as enc;
    let mut url = format!(
        "{base}/xc/recordings/{}?gen={}&sp={}",
        enc(group),
        enc(genus),
        enc(species)
    );
    if let Some(c) = country {
        url.push_str(&format!("&country={}", enc(c)));
    }
    url.push_str(&format!(
        "&page={}&per_page={}",
        page.max(1),
        per_page.clamp(50, 500)
    ));
    url
}

/// Sort a server reply into [`ServerReply`] or an error. `retry_after` is
/// the `Retry-After` header, if the browser let us read it.
fn server_reply(
    status: u16,
    body: String,
    retry_after: Option<&str>,
) -> Result<ServerReply, XcError> {
    match status {
        202 => Ok(ServerReply::Indexing {
            status: parse_indexing(&body),
            retry_secs: retry_secs(retry_after),
        }),
        200..=299 => Ok(ServerReply::Ready(body)),
        _ => Err(classify_http_error(status, &body, false)),
    }
}

/// The progress in a 202 body: `{"status":"indexing","pages_done":3,"pages_total":23}`.
/// An unreadable body counts as no progress yet.
fn parse_indexing(body: &str) -> ServerIndexing {
    serde_json::from_str(body).unwrap_or(ServerIndexing {
        pages_done: 0,
        pages_total: 0,
    })
}

/// Seconds to wait from a `Retry-After` header given in seconds, kept
/// between [`MIN_RETRY_SECS`] and [`MAX_RETRY_SECS`].
fn retry_secs(header: Option<&str>) -> u32 {
    header
        .and_then(|h| h.trim().parse::<u32>().ok())
        .unwrap_or(DEFAULT_RETRY_SECS)
        .clamp(MIN_RETRY_SECS, MAX_RETRY_SECS)
}

/// The body of `/xc/countries/{group}`: `[{"country":..,"recordings":..,"species":..}]`.
/// Entries with an empty name or no recordings are dropped.
fn parse_countries(body: &str) -> Result<Vec<CountryCount>, String> {
    let list: Vec<CountryCount> = serde_json::from_str(body).map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .filter(|c| !c.country.trim().is_empty() && c.recordings > 0)
        .collect())
}

/// Book the next request slot. Returns `(slot, new_next)`: the request may
/// start at `slot`, and the one after it at `new_next`.
fn reserve_slot(next_ms: f64, now_ms: f64, gap_ms: f64) -> (f64, f64) {
    let slot = next_ms.max(now_ms);
    (slot, slot + gap_ms)
}

fn species_query(genus: &str, species: &str, country: Option<&str>) -> String {
    match country {
        Some(c) => format!("gen:{genus} sp:{species} cnt:\"{c}\""),
        None => format!("gen:{genus} sp:{species}"),
    }
}

/// The search URL for a web source, or `None` for [`XcSource::Native`].
/// The query is normalized here because the proxy does not do it.
fn search_url(
    src: &XcSource,
    proxy_base: &str,
    query: &str,
    page: u32,
    per_page: u32,
) -> Option<String> {
    use crate::web_util::encode_component as enc;
    let query = enc(&xc_lib::api::normalize_query(query));
    let per_page = per_page.clamp(50, 500);
    match src {
        XcSource::Native => None,
        XcSource::Direct { key } => Some(format!(
            "{XC_API_URL}?query={query}&key={}&page={page}&per_page={per_page}",
            enc(key)
        )),
        XcSource::Proxy => Some(format!(
            "{proxy_base}/xc/api/3/recordings?query={query}&page={page}&per_page={per_page}"
        )),
    }
}

/// Where to download a recording's audio on a web source.
fn audio_url(src: &XcSource, proxy_base: &str, rec: &XcRecording) -> String {
    match src {
        XcSource::Proxy => format!("{proxy_base}/xc/audio/{}", rec.id_num()),
        _ => rec.file_url.clone(),
    }
}

/// OPFS file name for a cached species list. Independent of the source, so
/// a list built with a key is reused through the proxy and the other way.
fn taxonomy_cache_key(group: &str, country: Option<&str>) -> String {
    let safe = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    match country {
        Some(c) => format!("taxonomy_{}__{}.json", safe(group), safe(c)),
        None => format!("taxonomy_{}.json", safe(group)),
    }
}

/// The human-readable message in an error body. XC sends
/// `{"error": "client_error", "message": "..."}`; the proxy sends
/// `{"error": "..."}`. Prefer `message`, then `error`.
fn error_body_message(body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    ["message", "error"]
        .iter()
        .filter_map(|k| json.get(*k).and_then(|v| v.as_str()))
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
}

fn classify_http_error(status: u16, body: &str, own_key: bool) -> XcError {
    let message = error_body_message(body);
    match status {
        429 => XcError::RateLimited,
        503 => XcError::Busy(message),
        502 => XcError::Upstream(message),
        413 => XcError::TooLarge { id: None },
        401 | 403 if own_key => XcError::BadKey(message),
        _ => XcError::Http { status, message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes() {
        assert_eq!(language_code("de-AT").as_deref(), Some("de"));
        assert_eq!(language_code("pt_BR").as_deref(), Some("pt"));
        assert_eq!(language_code("en-GB"), None);
        assert_eq!(language_code("EN"), None);
        assert_eq!(language_code("*"), None);
    }

    #[test]
    fn browser_language_lists() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            languages_from_tags(&tags(&["nl-BE", "nl", "de", "en-US", "fr"])),
            ["nl", "de"]
        );
        assert_eq!(
            languages_from_tags(&tags(&["en-GB", "de"])),
            Vec::<String>::new()
        );
        assert_eq!(languages_from_tags(&tags(&["*", "ja"])), ["ja"]);
    }

    #[test]
    fn names_language_setting() {
        assert_eq!(NamesLanguage::from_stored(None), NamesLanguage::Browser);
        assert_eq!(
            NamesLanguage::from_stored(Some("browser")),
            NamesLanguage::Browser
        );
        assert_eq!(
            NamesLanguage::from_stored(Some("en")),
            NamesLanguage::English
        );
        assert_eq!(
            NamesLanguage::from_stored(Some("de")),
            NamesLanguage::One("de".into())
        );
        assert_eq!(
            NamesLanguage::from_stored(Some("!!")),
            NamesLanguage::Browser
        );
        assert_eq!(NamesLanguage::One("de".into()).stored(), "de");
        assert_eq!(NamesLanguage::English.codes(), Vec::<String>::new());
    }

    #[test]
    fn species_query_adds_quoted_country() {
        assert_eq!(
            species_query("Myotis", "daubentonii", None),
            "gen:Myotis sp:daubentonii"
        );
        assert_eq!(
            species_query("Myotis", "daubentonii", Some("Bosnia Herzegovina")),
            "gen:Myotis sp:daubentonii cnt:\"Bosnia Herzegovina\""
        );
    }

    fn rec(id: &str, file: &str) -> XcRecording {
        xc_lib::api::parse_recording(&serde_json::json!({
            "id": id, "gen": "Myotis", "sp": "daubentonii", "file": file,
        }))
        .unwrap()
    }

    #[test]
    fn direct_url_carries_key_and_normalized_query() {
        let src = XcSource::Direct { key: "a b".into() };
        let url = search_url(&src, "https://p.example", "Myotis", 2, 100).unwrap();
        assert_eq!(
            url,
            "https://xeno-canto.org/api/3/recordings?query=sp%3AMyotis&key=a%20b&page=2&per_page=100"
        );
    }

    #[test]
    fn proxy_url_never_has_key() {
        let url = search_url(&XcSource::Proxy, "https://p.example", "grp:bats", 1, 500).unwrap();
        assert_eq!(
            url,
            "https://p.example/xc/api/3/recordings?query=grp%3Abats&page=1&per_page=500"
        );
        assert!(!url.contains("key"));
    }

    #[test]
    fn native_has_no_url() {
        assert!(search_url(&XcSource::Native, "", "x", 1, 100).is_none());
    }

    #[test]
    fn per_page_is_clamped() {
        let url = search_url(&XcSource::Proxy, "b", "grp:bats", 1, 10).unwrap();
        assert!(url.ends_with("per_page=50"));
    }

    #[test]
    fn audio_url_by_source() {
        let r = rec("928094", "https://xeno-canto.org/928094/download");
        assert_eq!(
            audio_url(&XcSource::Proxy, "https://p.example", &r),
            "https://p.example/xc/audio/928094"
        );
        assert_eq!(
            audio_url(
                &XcSource::Direct { key: "k".into() },
                "https://p.example",
                &r
            ),
            "https://xeno-canto.org/928094/download"
        );
    }

    #[test]
    fn cache_key_is_source_independent_and_safe() {
        assert_eq!(taxonomy_cache_key("bats", None), "taxonomy_bats.json");
        assert_eq!(
            taxonomy_cache_key("land mammals", Some("Côte d'Ivoire")),
            "taxonomy_land_mammals__Côte_d_Ivoire.json"
        );
    }

    #[test]
    fn error_message_prefers_message_over_error() {
        let xc = r#"{"error":"client_error","message":"Missing or invalid 'key' parameter."}"#;
        assert_eq!(
            error_body_message(xc).as_deref(),
            Some("Missing or invalid 'key' parameter.")
        );
        assert_eq!(
            error_body_message(r#"{"error":"Proxy busy"}"#).as_deref(),
            Some("Proxy busy")
        );
        assert_eq!(error_body_message("<html>"), None);
    }

    #[test]
    fn classify_statuses() {
        assert_eq!(classify_http_error(429, "", false), XcError::RateLimited);
        assert_eq!(
            classify_http_error(503, r#"{"error":"busy"}"#, false),
            XcError::Busy(Some("busy".into()))
        );
        assert_eq!(classify_http_error(502, "", false), XcError::Upstream(None));
        assert_eq!(
            classify_http_error(413, "", false),
            XcError::TooLarge { id: None }
        );
        assert_eq!(classify_http_error(401, "", true), XcError::BadKey(None));
        assert_eq!(
            classify_http_error(403, "", false),
            XcError::Http {
                status: 403,
                message: None
            }
        );
        assert_eq!(
            classify_http_error(400, r#"{"error":"bad query"}"#, false),
            XcError::Http {
                status: 400,
                message: Some("bad query".into())
            }
        );
    }

    // ── Species-list loop ──

    /// Poll a future that never waits (the fake fetches are ready at once).
    fn run<T>(fut: impl std::future::Future<Output = T>) -> T {
        let mut fut = std::pin::pin!(fut);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match fut.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("fake fetch should not wait"),
        }
    }

    fn result_page(page: u32, num_pages: u32, num_recordings: u32) -> XcSearchResult {
        XcSearchResult {
            num_recordings,
            num_species: 1,
            num_pages,
            page,
            recordings: vec![rec(&page.to_string(), "https://x/a.mp3")],
        }
    }

    struct Run {
        result: Result<(), XcError>,
        fetched: Vec<u32>,
        progress: Vec<BuildProgress>,
    }

    /// Build a list whose search has `num_pages` pages. Each fetch takes
    /// `ms_per_page` on a fake clock. `stop_after` makes keep_going() false
    /// once that many pages have been fetched.
    fn build(allow_long: bool, num_pages: u32, ms_per_page: f64, stop_after: Option<usize>) -> Run {
        let fetched = std::cell::RefCell::new(Vec::new());
        let clock = Cell::new(0.0);
        let mut progress = Vec::new();
        let result = run(build_taxonomy(
            allow_long,
            "bats",
            None,
            |page| {
                fetched.borrow_mut().push(page);
                clock.set(clock.get() + ms_per_page);
                std::future::ready(Ok(result_page(page, num_pages, num_pages * 500)))
            },
            || clock.get(),
            &mut |p| progress.push(p),
            &|| stop_after.is_none_or(|n| fetched.borrow().len() < n),
        ))
        .map(|_| ());
        Run {
            result,
            fetched: fetched.into_inner(),
            progress,
        }
    }

    #[test]
    fn zero_or_one_page_stops_after_one_request() {
        for pages in [0, 1] {
            let r = build(false, pages, 5000.0, None);
            assert_eq!(r.result, Ok(()));
            assert_eq!(r.fetched, [1]);
            assert_eq!(r.progress.len(), 1);
            assert_eq!(r.progress[0].total_pages, 1);
            assert_eq!(r.progress[0].secs_left, 0.0);
        }
    }

    #[test]
    fn direct_has_no_page_limit() {
        // 61 pages at the 1.1 s minimum is about 67 s, under the long-build limit.
        let r = build(false, 61, 0.0, None);
        assert_eq!(r.result, Ok(()));
        assert_eq!(r.fetched.len(), 61);
        let last = r.progress.last().unwrap();
        assert_eq!((last.pages_done, last.total_pages), (61, 61));
        assert_eq!(last.recordings_done, 61);
    }

    #[test]
    fn long_direct_build_asks_after_page_one() {
        // 2105 pages at 5 s each is about 2.9 hours.
        let r = build(false, 2105, 5000.0, None);
        match r.result {
            Err(XcError::LongBuild {
                recordings,
                est_secs,
                ..
            }) => {
                assert_eq!(recordings, 2105 * 500);
                assert!((est_secs - 2105.0 * 5.0).abs() < 1e-6);
            }
            other => panic!("expected LongBuild, got {other:?}"),
        }
        assert_eq!(r.fetched, [1]);

        // allow_long goes ahead (stopped early here to keep the test short).
        let r = build(true, 2105, 5000.0, Some(3));
        assert_eq!(r.result, Err(XcError::Cancelled));
        assert_eq!(r.fetched, [1, 2, 3]);
    }

    #[test]
    fn progress_estimate_uses_measured_time() {
        let r = build(false, 4, 5000.0, None);
        let secs: Vec<f64> = r.progress.iter().map(|p| p.secs_left).collect();
        assert_eq!(secs, [15.0, 10.0, 5.0, 0.0]);
    }

    #[test]
    fn cancel_after_fetch_adds_no_page() {
        let r = build(false, 5, 0.0, Some(2));
        assert_eq!(r.result, Err(XcError::Cancelled));
        assert_eq!(r.fetched, [1, 2]);
        assert_eq!(r.progress.len(), 1, "page 2 was fetched but not added");
    }

    #[test]
    fn reserve_slot_spaces_requests() {
        let now = 10_000.0;
        let (a, next) = reserve_slot(0.0, now, 1100.0);
        let (b, next) = reserve_slot(next, now, 1100.0);
        let (c, next) = reserve_slot(next, now, 1100.0);
        assert_eq!([a, b, c], [now, now + 1100.0, now + 2200.0]);
        // After an idle gap the next request starts at once.
        let later = next + 5000.0;
        assert_eq!(reserve_slot(next, later, 1100.0), (later, later + 1100.0));
    }

    #[test]
    fn source_debug_hides_key() {
        let s = format!(
            "{:?}",
            XcSource::Direct {
                key: "secret".into()
            }
        );
        assert!(!s.contains("secret"));
    }

    // ── Server index endpoints ──

    #[test]
    fn server_urls_encode_group_and_country() {
        assert_eq!(
            server_taxonomy_url("https://p.example", "land mammals", None),
            "https://p.example/xc/taxonomy/land%20mammals"
        );
        assert_eq!(
            server_taxonomy_url("https://p.example", "bats", Some("Côte d'Ivoire")),
            "https://p.example/xc/taxonomy/bats?country=C%C3%B4te%20d%27Ivoire"
        );
        assert_eq!(
            server_countries_url("https://p.example", "land mammals"),
            "https://p.example/xc/countries/land%20mammals"
        );
    }

    #[test]
    fn server_recordings_url_params() {
        assert_eq!(
            server_recordings_url("b", "bats", "Myotis", "daubentonii", None, 2, 100),
            "b/xc/recordings/bats?gen=Myotis&sp=daubentonii&page=2&per_page=100"
        );
        assert_eq!(
            server_recordings_url(
                "b",
                "land mammals",
                "Vulpes",
                "vulpes",
                Some("United States"),
                0,
                10
            ),
            "b/xc/recordings/land%20mammals?gen=Vulpes&sp=vulpes&country=United%20States\
             &page=1&per_page=50"
        );
        assert!(!server_recordings_url("b", "bats", "a", "b", None, 1, 100).contains("key"));
    }

    #[test]
    fn server_reply_by_status() {
        assert_eq!(
            server_reply(200, "[]".into(), None),
            Ok(ServerReply::Ready("[]".into()))
        );
        assert_eq!(
            server_reply(
                202,
                r#"{"status":"indexing","pages_done":3,"pages_total":23}"#.into(),
                Some("5")
            ),
            Ok(ServerReply::Indexing {
                status: ServerIndexing {
                    pages_done: 3,
                    pages_total: 23
                },
                retry_secs: 5
            })
        );
        // A 401 from the server is never about the user's own key.
        assert_eq!(
            server_reply(401, String::new(), None),
            Err(XcError::Http {
                status: 401,
                message: None
            })
        );
        assert_eq!(
            server_reply(404, r#"{"error":"unknown group"}"#.into(), None),
            Err(XcError::Http {
                status: 404,
                message: Some("unknown group".into())
            })
        );
        assert_eq!(
            server_reply(503, String::new(), None),
            Err(XcError::Busy(None))
        );
    }

    #[test]
    fn indexing_progress_parsing() {
        assert_eq!(
            parse_indexing(r#"{"status":"indexing","pages_done":0,"pages_total":0}"#),
            ServerIndexing {
                pages_done: 0,
                pages_total: 0
            }
        );
        // An unreadable body counts as no progress yet.
        assert_eq!(
            parse_indexing("<html>"),
            ServerIndexing {
                pages_done: 0,
                pages_total: 0
            }
        );
    }

    #[test]
    fn retry_after_is_clamped() {
        assert_eq!(retry_secs(None), 5);
        assert_eq!(retry_secs(Some(" 10 ")), 10);
        assert_eq!(retry_secs(Some("0")), MIN_RETRY_SECS);
        assert_eq!(retry_secs(Some("3600")), MAX_RETRY_SECS);
        // HTTP-date form is not used by the server; use the default.
        assert_eq!(retry_secs(Some("Wed, 21 Oct 2026 07:28:00 GMT")), 5);
    }

    #[test]
    fn countries_parsing() {
        let body = r#"[{"country":"Australia","recordings":22,"species":9},
                      {"country":"","recordings":3,"species":1},
                      {"country":"Brazil","recordings":0,"species":0}]"#;
        assert_eq!(
            parse_countries(body),
            Ok(vec![CountryCount {
                country: "Australia".into(),
                recordings: 22,
                species: 9
            }])
        );
        assert!(parse_countries(r#"{"error":"x"}"#).is_err());
    }

    #[test]
    fn fallback_decisions() {
        let direct = XcSource::Direct { key: "k".into() };
        let http = |status| XcError::Http {
            status,
            message: None,
        };
        for e in [
            XcError::Network,
            XcError::Busy(None),
            XcError::Upstream(None),
            XcError::RateLimited,
            http(404),
            http(500),
            XcError::Other("bad json".into()),
        ] {
            assert!(falls_back_to_local(&direct, &e), "{e:?}");
            assert!(falls_back_to_local(&XcSource::Native, &e), "{e:?}");
            assert!(!falls_back_to_local(&XcSource::Proxy, &e), "{e:?}");
        }
        assert!(!falls_back_to_local(&direct, &XcError::Cancelled));

        assert!(index_unavailable(&http(404)));
        assert!(index_unavailable(&XcError::Busy(None)));
        assert!(!index_unavailable(&http(500)));
        assert!(!index_unavailable(&XcError::Network));

        assert!(uses_server_taxonomy(&XcSource::Proxy, true));
        assert!(uses_server_taxonomy(&XcSource::Proxy, false));
        assert!(uses_server_taxonomy(&direct, false));
        assert!(uses_server_taxonomy(&XcSource::Native, false));
        assert!(!uses_server_taxonomy(&direct, true));
        assert!(!uses_server_taxonomy(&XcSource::Native, true));
    }

    fn indexing(done: u32, total: u32) -> Result<ServerReply, XcError> {
        Ok(ServerReply::Indexing {
            status: ServerIndexing {
                pages_done: done,
                pages_total: total,
            },
            retry_secs: 5,
        })
    }

    #[test]
    fn poll_waits_while_indexing() {
        let replies = std::cell::RefCell::new(vec![
            indexing(1, 23),
            indexing(5, 23),
            Ok(ServerReply::Ready("{}".into())),
        ]);
        let slept = std::cell::RefCell::new(Vec::new());
        let mut progress = Vec::new();
        let body = run(poll_until_ready(
            || std::future::ready(replies.borrow_mut().remove(0)),
            |secs| {
                slept.borrow_mut().push(secs);
                std::future::ready(())
            },
            true,
            &mut |p| progress.push(p),
            &|| true,
        ));
        assert_eq!(body, Ok(Some("{}".to_string())));
        assert_eq!(slept.into_inner(), [5, 5]);
        assert_eq!(
            progress,
            [
                TaxonomyProgress::Server(ServerIndexing {
                    pages_done: 1,
                    pages_total: 23
                }),
                TaxonomyProgress::Server(ServerIndexing {
                    pages_done: 5,
                    pages_total: 23
                }),
            ]
        );
    }

    #[test]
    fn poll_stops_on_cancel_and_on_error() {
        let calls = Cell::new(0);
        let result = run(poll_until_ready(
            || {
                calls.set(calls.get() + 1);
                std::future::ready(indexing(1, 23))
            },
            |_| std::future::ready(()),
            true,
            &mut |_| {},
            &|| calls.get() < 2,
        ));
        assert_eq!(result, Err(XcError::Cancelled));
        assert_eq!(calls.get(), 2, "the reply after the stop is not reported");

        let result = run(poll_until_ready(
            || std::future::ready(Err(XcError::Network)),
            |_| std::future::ready(()),
            true,
            &mut |_| {},
            &|| true,
        ));
        assert_eq!(result, Err(XcError::Network));
    }

    #[test]
    fn poll_without_wait_stops_at_first_202() {
        let calls = Cell::new(0);
        let mut progress = Vec::new();
        let result = run(poll_until_ready(
            || {
                calls.set(calls.get() + 1);
                std::future::ready(indexing(0, 0))
            },
            |_| -> std::future::Ready<()> { panic!("must not sleep") },
            false,
            &mut |p| progress.push(p),
            &|| true,
        ));
        assert_eq!(result, Ok(None));
        assert_eq!(calls.get(), 1);
        assert!(progress.is_empty());

        let result = run(poll_until_ready(
            || std::future::ready(Ok(ServerReply::Ready("{}".into()))),
            |_| std::future::ready(()),
            false,
            &mut |_| {},
            &|| true,
        ));
        assert_eq!(result, Ok(Some("{}".to_string())));
    }

    #[test]
    fn server_taxonomy_body_matches_frontend_type() {
        let body = r#"{"group":"bats","country":null,"species":[{"gen":"Myotis",
            "sp":"daubentonii","en":"Daubenton's Bat","fam":"","recording_count":12}],
            "total_recordings":11494,"last_updated":"2026-11-09T05:00:00Z"}"#;
        let t: XcGroupTaxonomy = serde_json::from_str(body).unwrap();
        assert_eq!(t.species[0].genus, "Myotis");
        assert_eq!(t.total_recordings, 11494);
    }
}

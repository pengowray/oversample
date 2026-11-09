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
//! Direct and Proxy share one code path: the same JSON parsing as the native
//! side (`xc_lib::api`), a species-list builder (`xc_lib::taxonomy`) driven
//! page by page from here, and a taxonomy cache in OPFS that both sources use.
//!
//! The pure helpers (URL building, error classification) are unit-tested on
//! the host; the fetch wrappers are thin.

use std::cell::Cell;
use std::collections::HashSet;

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
/// The proxy will page through at most this many pages for one species list.
/// Larger groups (birds, grasshoppers) need a country filter or the user's
/// own key. At 1.1 s per page this is just over a minute.
pub const PROXY_MAX_TAXONOMY_PAGES: u32 = 60;
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
    /// HTTP 413: the file is larger than the proxy will relay.
    TooLarge,
    /// Proxy only: this group needs more than [`PROXY_MAX_TAXONOMY_PAGES`].
    GroupTooLarge { pages: u32 },
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

/// How old a species list is.
pub enum TaxonomyAge {
    /// Already formatted by the native cache (`xc_lib::cache`).
    Text(String),
    /// Seconds since the list was built (web paths).
    Seconds(f64),
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
pub async fn save_key(is_tauri: bool, key: &str) -> Result<XcSource, XcError> {
    let key = key.trim().to_string();
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

/// One page of recordings of a species.
pub async fn species_recordings(
    src: &XcSource,
    genus: &str,
    species: &str,
    page: u32,
) -> Result<XcSearchResult, XcError> {
    match src {
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
        _ => web_search(src, &species_query(genus, species), page, PER_PAGE).await,
    }
}

/// The species list for a group, optionally limited to one country.
///
/// Uses the cache unless `refresh` is set. On the web paths a build pages
/// through every recording in the group, one request per 1.1 s, calling
/// `progress(pages_done, total_pages)` after each page and stopping with
/// [`XcError::Cancelled`] as soon as `keep_going()` returns false. The native
/// path reports no progress.
pub async fn browse_group(
    src: &XcSource,
    group: &str,
    country: Option<&str>,
    refresh: bool,
    mut progress: impl FnMut(u32, u32),
    keep_going: impl Fn() -> bool,
) -> Result<XcGroupTaxonomy, XcError> {
    if *src == XcSource::Native {
        let cmd = if refresh {
            "xc_refresh_taxonomy"
        } else {
            "xc_browse_group"
        };
        return tauri_invoke_typed_args(
            cmd,
            &XcGroupArgs {
                group: group.to_string(),
                country: country.map(str::to_string),
            },
        )
        .await
        .map_err(XcError::Other);
    }

    let cache_key = taxonomy_cache_key(group, country);
    if !refresh {
        if let Some(cached) = load_cached_taxonomy(&cache_key).await {
            return Ok(cached);
        }
    }

    let query = xc_lib::taxonomy::group_query(group, country);
    let mut acc = xc_lib::taxonomy::SpeciesAccumulator::new(group, country);
    let mut page = 1u32;
    loop {
        if !keep_going() {
            return Err(XcError::Cancelled);
        }
        let result = web_search(src, &query, page, TAXONOMY_PER_PAGE).await?;
        if !keep_going() {
            return Err(XcError::Cancelled);
        }
        if *src == XcSource::Proxy && result.num_pages > PROXY_MAX_TAXONOMY_PAGES {
            return Err(XcError::GroupTooLarge {
                pages: result.num_pages,
            });
        }
        acc.add_page(&result);
        progress(page, result.num_pages.max(page));
        if page >= result.num_pages {
            break;
        }
        page += 1;
    }

    let now: String = js_sys::Date::new_0().to_iso_string().into();
    let taxonomy = acc.finish(now);
    match serde_json::to_string(&taxonomy) {
        Ok(json) => {
            if let Err(e) = crate::opfs::opfs_save_in(TAXONOMY_OPFS_DIR, &cache_key, &json).await {
                log::warn!("XC species list not cached: {e}");
            }
        }
        Err(e) => log::warn!("XC species list not cached: {e}"),
    }
    Ok(taxonomy)
}

/// How old the species list for this group is. `taxonomy` is the list just
/// returned by [`browse_group`]; the web paths read its timestamp.
pub async fn taxonomy_age(
    src: &XcSource,
    group: &str,
    country: Option<&str>,
    taxonomy: &XcGroupTaxonomy,
) -> Option<TaxonomyAge> {
    if *src == XcSource::Native {
        let val = tauri_invoke_with_args(
            "xc_taxonomy_age",
            &XcGroupArgs {
                group: group.to_string(),
                country: country.map(str::to_string),
            },
        )
        .await
        .ok()?;
        return val.as_string().map(TaxonomyAge::Text);
    }
    let built_ms = js_sys::Date::parse(&taxonomy.last_updated);
    if built_ms.is_nan() {
        return None;
    }
    let secs = ((js_sys::Date::now() - built_ms) / 1000.0).max(0.0);
    Some(TaxonomyAge::Seconds(secs))
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
    let bytes = get_bytes(&url, src.uses_own_key()).await?;
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
    let body: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| XcError::Other(format!("unexpected response: {e}")))?;
    xc_lib::api::parse_search_response(&body).map_err(XcError::Other)
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
        let slot = next.get().max(now);
        next.set(slot + MIN_REQUEST_GAP_MS);
        slot
    });
    let wait = slot - now;
    if wait > 0.0 {
        crate::web_util::sleep_ms(wait.ceil() as i32).await;
    }
}

/// GET `url`; on a non-2xx status, read the body for the error message.
async fn get_ok(url: &str, own_key: bool) -> Result<web_sys::Response, XcError> {
    throttle().await;
    let window = web_sys::window().ok_or_else(|| XcError::Other("no window".into()))?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|_| XcError::Network)?
        .dyn_into()
        .map_err(|_| XcError::Other("fetch did not return a Response".into()))?;
    if resp.ok() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = match resp.text() {
        Ok(p) => JsFuture::from(p)
            .await
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default(),
        Err(_) => String::new(),
    };
    Err(classify_http_error(status, &body, own_key))
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

// ── Pure helpers (host-tested) ──────────────────────────────────────

fn species_query(genus: &str, species: &str) -> String {
    format!("gen:{genus} sp:{species}")
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
        413 => XcError::TooLarge,
        401 | 403 if own_key => XcError::BadKey(message),
        _ => XcError::Http { status, message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(classify_http_error(413, "", false), XcError::TooLarge);
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
}

//! Importing audio from Wikimedia Commons and the other MediaWiki wikis.
//!
//! Unlike Xeno-Canto (see `components::xc_browser`), Wikimedia needs no API
//! key and no native proxy: `commons.wikimedia.org/w/api.php` answers
//! anonymous cross-origin requests when passed `origin=*`, and
//! `upload.wikimedia.org` serves media with `Access-Control-Allow-Origin: *`.
//! So one code path — `fetch` from WASM — covers web, desktop and Android.
//!
//! The interesting metadata is the structured-data statement
//! [P14424](https://www.wikidata.org/wiki/Property:P14424), "time expansion
//! factor". A bat detector that records in 10x time expansion uploads a file
//! whose stored sample rate is a tenth of the real one, so every frequency in
//! it reads ten times too low. When a file carries P14424 we hand the factor
//! to the loader, which reinterprets the samples at the true rate.

use crate::source_site::{SiteMetadata, SourceSite};
use crate::state::AppState;

/// Host used for structured data and as the default for bare file titles.
/// A file link from another wiki is looked up on that wiki first, in case the
/// wiki has its own upload under that title (see `fetch_file`).
const COMMONS: &str = "commons.wikimedia.org";

/// Wikidata property id for "time expansion factor" (quantity).
const P_TIME_EXPANSION: &str = "P14424";

/// Wikimedia domains we'll talk to. Everything under these answers CORS
/// preflight-free GETs with `origin=*`; an arbitrary third-party MediaWiki
/// generally does not, so pasting one gets a clear error rather than an
/// opaque network failure.
const ALLOWED_SUFFIXES: &[&str] = &[
    ".wikipedia.org",
    ".wikimedia.org",
    ".wikibooks.org",
    ".wikisource.org",
    ".wikiversity.org",
    ".wikivoyage.org",
    ".wiktionary.org",
    ".wikinews.org",
    ".wikiquote.org",
    "wikidata.org",
    "wikimedia.org",
];

/// File extensions worth offering. The decoder handles WAV, FLAC, OGG
/// (Vorbis), MP3 and M4A; Opus and WebM are listed because Commons hosts
/// them and a decode failure is a clearer answer than silently hiding them.
/// An Ogg video (.ogv) is listed for its Vorbis sound track.
const AUDIO_EXTS: &[&str] = &[
    "wav", "flac", "mp3", "ogg", "oga", "ogv", "opus", "m4a", "mp4", "aac", "webm",
];

/// What the user's pasted text resolved to.
#[derive(Clone, Debug, PartialEq)]
pub enum WikiTarget {
    /// A single media file, e.g. `File:Bat feeding buzz.wav`. `api_host` is
    /// the wiki the link came from, which may store its own file under that
    /// title; Commons for a bare title.
    File { api_host: String, title: String },
    /// An article — list the audio files it uses.
    Page { api_host: String, title: String },
    /// Free text — search Commons for matching audio.
    Search { query: String },
}

/// One media file as described by the API.
#[derive(Clone, Debug, PartialEq)]
pub struct WikiFile {
    /// Full page title including namespace, e.g. `File:Bat feeding buzz.wav`.
    pub title: String,
    /// Title without the namespace prefix — used as the loaded file's name.
    pub name: String,
    /// Direct media URL on `upload.wikimedia.org`.
    pub url: String,
    pub mime: String,
    pub size: u64,
    /// Playing time as stored, i.e. before any time-expansion correction.
    pub duration_secs: Option<f64>,
    /// Human-facing file description page.
    pub description_url: String,
    /// P14424, when the file declares one.
    pub time_expansion: Option<f64>,
    /// Label/value rows for the metadata panel.
    pub fields: Vec<(String, String)>,
    /// The wiki that stores the file: Commons, or the wiki that has its own
    /// upload under this title (see [`stored_on`]).
    pub host: String,
}

impl WikiFile {
    /// The site to show as the file's source.
    pub fn site(&self) -> SourceSite {
        site_for_host(&self.host)
    }

    /// Whether the file is stored on Commons, where its structured data
    /// (P14424) is.
    fn on_commons(&self) -> bool {
        self.host == COMMONS
    }
    /// Playing time once a time-expansion correction is applied.
    pub fn corrected_duration_secs(&self) -> Option<f64> {
        let d = self.duration_secs?;
        Some(match self.time_expansion {
            Some(te) if te.is_finite() && te > 0.0 => d / te,
            _ => d,
        })
    }
}

// ── Input parsing ────────────────────────────────────────────────────────

fn host_allowed(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    ALLOWED_SUFFIXES
        .iter()
        .any(|s| host == s.trim_start_matches('.') || host.ends_with(s))
}

/// Percent-decode, then turn MediaWiki's `_` word separator back into a space.
pub fn decode_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'_' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_file_title(title: &str) -> bool {
    // The API normalises localised namespaces for us, so only the canonical
    // English prefixes need recognising up front.
    let lower = title.to_ascii_lowercase();
    lower.starts_with("file:") || lower.starts_with("image:") || lower.starts_with("media:")
}

fn has_audio_ext(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .map(|e| {
            let e = e.to_ascii_lowercase();
            AUDIO_EXTS.contains(&e.as_str())
        })
        .unwrap_or(false)
}

/// Strip a namespace prefix, giving the bare filename.
fn strip_namespace(title: &str) -> String {
    match title.split_once(':') {
        Some((ns, rest)) if is_file_title(&format!("{ns}:")) => rest.trim().to_string(),
        _ => title.trim().to_string(),
    }
}

/// Work out what the user meant by whatever they pasted.
///
/// Accepts a Commons or Wikipedia file page URL, a direct
/// `upload.wikimedia.org` media URL, an article URL, a `File:…` title, a bare
/// filename ending in an audio extension, or free text to search for.
pub fn parse_input(input: &str) -> Result<WikiTarget, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a Wikimedia file, article or search term".into());
    }

    if let Some(rest) = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))
    {
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = host.split('@').next_back().unwrap_or(host);
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        if !host_allowed(&host) {
            return Err(format!(
                "{host} isn't a Wikimedia site \u{2014} paste a Wikipedia or Wikimedia Commons link"
            ));
        }
        // Mobile links (en.m.wikipedia.org, commons.m.wikimedia.org) name the
        // same wiki as the desktop host.
        let host = host.replacen(".m.", ".", 1);
        let (path, _query) = path.split_once('?').unwrap_or((path, ""));
        let path = path.split('#').next().unwrap_or(path);

        // Direct media URL: .../wikipedia/commons/e/e7/Bat_feeding_buzz.wav
        // and its /thumb/ variant, which appends a rendition filename. A
        // Wikipedia's own uploads are under its language code instead of
        // "commons": .../wikipedia/en/2/2b/Foo.ogg. Other wikis' folders
        // there (meta, species, ...) are looked up on Commons.
        if host == "upload.wikimedia.org" {
            let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            let file = segments
                .iter()
                .find(|s| has_audio_ext(s))
                .ok_or("That upload.wikimedia.org link doesn't point at an audio file")?;
            let api_host = match segments.as_slice() {
                // be-tarask.wikipedia.org keeps its uploads under its old
                // name. be-x-old.wikipedia.org redirects to it without the
                // CORS headers a browser needs, so ask be-tarask directly.
                ["wikipedia", "be-x-old", ..] => "be-tarask.wikipedia.org".to_string(),
                ["wikipedia", lang, ..] if is_wikipedia_org_folder(lang) => {
                    format!("{lang}.wikipedia.org")
                }
                _ => COMMONS.to_string(),
            };
            return Ok(WikiTarget::File {
                api_host,
                title: format!("File:{}", decode_component(file)),
            });
        }

        // Query-string form: /w/index.php?title=File:Foo.wav
        if let Some(q) = rest.split_once('?').map(|(_, q)| q) {
            for pair in q.split('&') {
                if let Some(v) = pair.strip_prefix("title=") {
                    let title = decode_component(v);
                    return Ok(classify(host, title));
                }
            }
        }

        // `path` here has no leading slash — it's whatever followed the host.
        let title = path
            .split_once("wiki/")
            .map(|(_, t)| t)
            .unwrap_or(path)
            .trim_matches('/');
        if title.is_empty() {
            return Err("That link has no page title in it".into());
        }
        return Ok(classify(host, decode_component(title)));
    }

    if is_file_title(input) {
        return Ok(WikiTarget::File {
            api_host: COMMONS.to_string(),
            title: normalise_file_title(&input.replace('_', " ")),
        });
    }
    if has_audio_ext(input) && !input.contains(' ') {
        return Ok(WikiTarget::File {
            api_host: COMMONS.to_string(),
            title: format!("File:{input}"),
        });
    }
    Ok(WikiTarget::Search {
        query: input.to_string(),
    })
}

/// Rewrite `Image:`/`Media:` onto the canonical `File:` prefix.
fn normalise_file_title(title: &str) -> String {
    format!("File:{}", strip_namespace(title))
}

/// Whether a folder under upload.wikimedia.org/wikipedia/ holds the uploads
/// of a wiki at wikipedia.org. That wiki is at <folder>.wikipedia.org, except
/// for be-x-old, whose wiki is be-tarask.wikipedia.org (see `parse_input`).
/// Those folders are a Wikipedia's language code ("en", "simple", "zh-yue",
/// "zh-min-nan") or have the same form: 2 or 3 letters, optionally followed
/// by hyphenated parts.
/// ten (ten.wikipedia.org) and wg-en (wg-en.wikipedia.org) are wikis of that
/// form that are not a Wikipedia in one language. test and test2 hold the
/// uploads of test.wikipedia.org and test2.wikipedia.org. The folders of
/// other wikis (commons, meta, species, mediawiki, foundation, sources,
/// incubator, wikimania2014, ...) don't have that form, except fdc, whose
/// wiki is fdc.wikimedia.org. Checked against the 364 Wikipedias and the
/// special wikis in Wikimedia's site matrix on 2026-11-14.
fn is_wikipedia_org_folder(s: &str) -> bool {
    let letters = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase());
    let mut parts = s.split('-');
    let first = parts.next().unwrap_or_default();
    let code_form =
        s == "simple" || ((2..=3).contains(&first.len()) && letters(first) && parts.all(letters));
    (code_form && s != "fdc") || s == "test" || s == "test2"
}

fn classify(host: String, title: String) -> WikiTarget {
    if is_file_title(&title) {
        // Keep the wiki the link came from: it may have its own upload under
        // this title, which `fetch_file` checks for before asking Commons.
        WikiTarget::File {
            api_host: host,
            title: normalise_file_title(&title),
        }
    } else {
        WikiTarget::Page {
            api_host: host,
            title,
        }
    }
}

// ── API ──────────────────────────────────────────────────────────────────

async fn api_get(host: &str, params: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let mut url =
        format!("https://{host}/w/api.php?action=query&format=json&formatversion=2&origin=*");
    for (k, v) in params {
        url.push('&');
        url.push_str(k);
        url.push('=');
        url.push_str(&crate::web_util::encode_component(v));
    }
    let text = crate::components::file_sidebar::fetch_text(&url).await?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("Wikimedia API returned invalid JSON: {e}"))?;
    if let Some(err) = json["error"]["info"].as_str() {
        return Err(format!("Wikimedia API error: {err}"));
    }
    Ok(json)
}

/// The media URL and description fields, available from any wiki (a local
/// wiki reports Commons-hosted files too, flagged `imagerepository: shared`).
const FILE_PARAMS: &[(&str, &str)] = &[
    ("iiprop", "url|size|mime|extmetadata"),
    (
        "iiextmetadatafilter",
        "ImageDescription|Artist|LicenseShortName|LicenseUrl|DateTimeOriginal|Credit",
    ),
];

/// The MediaInfo slot, where structured-data statements (P14424) live. Only
/// Commons has that slot.
///
/// It is asked for on its own, never with `imageinfo`: Commons lets its CDN
/// and the browser keep an `imageinfo` answer for an hour, so a statement
/// added in that hour would be missing. An answer to a `revisions` query
/// alone is not kept (`cache-control: private, max-age=0`).
const MEDIAINFO_PARAMS: &[(&str, &str)] = &[
    ("prop", "revisions"),
    ("rvprop", "content"),
    ("rvslots", "mediainfo"),
];

/// Build an `imageinfo` parameter list.
fn params_for<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut v: Vec<(&str, &str)> = vec![("prop", "imageinfo")];
    v.extend_from_slice(FILE_PARAMS);
    v.extend_from_slice(extra);
    v
}

/// Put `row` in `fields` before the first row with one of the labels in
/// `before`, or at the end. A row already there with the same label goes.
fn insert_field(fields: &mut Vec<(String, String)>, row: (&str, String), before: &[&str]) {
    fields.retain(|(label, _)| label != row.0);
    let at = fields
        .iter()
        .position(|(label, _)| before.contains(&label.as_str()))
        .unwrap_or(fields.len());
    fields.insert(at, (row.0.to_string(), row.1));
}

impl WikiFile {
    /// Take the time expansion factor, the coordinates and (when the file
    /// page gave none) the date from the file's structured data.
    fn apply_statements(&mut self, statements: &serde_json::Value) {
        const TAIL: &[&str] = &["License", "License URL", "Time expansion", "URL"];
        if !self.fields.iter().any(|(label, _)| label == "Date") {
            if let Some(date) = date_from(statements) {
                insert_field(&mut self.fields, ("Date", date), TAIL);
            }
        }
        // The label the recording search uses, which the map reads.
        if let Some(v) = coordinates_from(statements) {
            insert_field(&mut self.fields, ("Coordinates", v), TAIL);
        }
        self.time_expansion = time_expansion_from(statements);
        self.fields.retain(|(label, _)| label != "Time expansion");
        if let Some(te) = self.time_expansion {
            let row = format!("{} (P14424)", format_factor(te));
            insert_field(&mut self.fields, ("Time expansion", row), &["URL"]);
        }
    }
}

/// Fetch the structured data of the Commons files in `files` and apply it
/// ([`WikiFile::apply_statements`]). A file Commons does not answer for keeps
/// what it has.
async fn add_structured_data(mut files: Vec<WikiFile>) -> Vec<WikiFile> {
    let titles: Vec<String> = files
        .iter()
        .filter(|f| f.on_commons())
        .map(|f| f.title.clone())
        .collect();
    for chunk in titles.chunks(TITLE_BATCH) {
        let joined = chunk.join("|");
        let mut params = MEDIAINFO_PARAMS.to_vec();
        params.push(("titles", &joined));
        match api_get(COMMONS, &params).await {
            Ok(json) => {
                for page in pages_of(&json) {
                    let Some(statements) = mediainfo_statements(&page) else {
                        continue;
                    };
                    let title = page["title"].as_str().unwrap_or_default();
                    for file in files
                        .iter_mut()
                        .filter(|f| f.on_commons() && f.title == title)
                    {
                        file.apply_statements(&statements);
                    }
                }
            }
            Err(e) => log::warn!("Commons structured data lookup failed: {e}"),
        }
    }
    files
}

/// The structured-data statements of a file page, from its MediaInfo slot.
/// `None` when the page came without one (any wiki other than Commons).
fn mediainfo_statements(page: &serde_json::Value) -> Option<serde_json::Value> {
    let content = page["revisions"][0]["slots"]["mediainfo"]["content"]
        .as_str()
        .or_else(|| page["revisions"][0]["slots"]["mediainfo"]["*"].as_str())?;
    let mut mediainfo: serde_json::Value = serde_json::from_str(content).ok()?;
    Some(mediainfo["statements"].take())
}

/// The value of property `prop`: a preferred-rank statement, else the first
/// that is not deprecated. `None` for a `somevalue` or `novalue` statement.
fn statement_value<'a>(
    statements: &'a serde_json::Value,
    prop: &str,
) -> Option<&'a serde_json::Value> {
    let list = statements[prop].as_array()?;
    let live = || {
        list.iter()
            .filter(|s| s["rank"].as_str() != Some("deprecated"))
    };
    let pick = live()
        .find(|s| s["rank"].as_str() == Some("preferred"))
        .or_else(|| live().next())?;
    let snak = &pick["mainsnak"];
    (snak["snaktype"].as_str() == Some("value")).then(|| &snak["datavalue"]["value"])
}

/// P14424 from a page's structured data.
#[cfg(test)]
fn time_expansion_from_mediainfo(page: &serde_json::Value) -> Option<f64> {
    time_expansion_from(&mediainfo_statements(page)?)
}

fn time_expansion_from(statements: &serde_json::Value) -> Option<f64> {
    // Quantities arrive as a signed decimal string, e.g. "+10" or "+2.5".
    let amount = statement_value(statements, P_TIME_EXPANSION)?["amount"].as_str()?;
    amount.trim_start_matches('+').parse::<f64>().ok()
}

/// Where the recording was made, as "lat, lon": the recorder's position
/// (P1259), else the place recorded (P9149), else P625.
fn coordinates_from(statements: &serde_json::Value) -> Option<String> {
    let value = ["P1259", "P9149", "P625"]
        .iter()
        .find_map(|p| statement_value(statements, p))?;
    let (lat, lon) = (value["latitude"].as_f64()?, value["longitude"].as_f64()?);
    Some(format!("{lat}, {lon}"))
}

/// The date the recording was made (P571), cut to the precision it states:
/// a day, a month or a year.
fn date_from(statements: &serde_json::Value) -> Option<String> {
    let value = statement_value(statements, "P571")?;
    let time = value["time"].as_str()?.trim_start_matches('+');
    let len = match value["precision"].as_u64()? {
        9 => 4,
        10 => 7,
        11.. => 10,
        _ => return None,
    };
    time.get(..len).map(str::to_string)
}

/// Strip the HTML that `extmetadata` descriptions and credits are wrapped in.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The wiki that stores a file, from a page `api_host`'s API returned. A wiki
/// other than Commons flags each file page with `imagerepository`: "local"
/// for its own upload, "shared" for a Commons file it shows. Commons flags its
/// own files "local".
fn stored_on(api_host: &str, page: &serde_json::Value) -> String {
    if page["imagerepository"].as_str() == Some("local") {
        api_host.to_string()
    } else {
        COMMONS.to_string()
    }
}

/// The site to show for a file stored on `host`. Local uploads on the other
/// sister wikis (Wikibooks, Wikisource, ...) are rare and keep the Wikimedia
/// Commons label.
fn site_for_host(host: &str) -> SourceSite {
    if host.ends_with(".wikipedia.org") {
        SourceSite::Wikipedia
    } else {
        SourceSite::WikimediaCommons
    }
}

/// One file page from `api_host`'s API.
fn parse_file_page(page: &serde_json::Value, api_host: &str) -> Option<WikiFile> {
    let title = page["title"].as_str()?.to_string();
    // Don't test `missing` here: a local wiki marks every Commons-hosted file
    // missing (it has no local description page) while still returning full
    // imageinfo. Absent imageinfo is the real "not here" signal.
    let ii = page["imageinfo"].get(0)?;
    let url = ii["url"].as_str()?.to_string();
    let mime = ii["mime"].as_str().unwrap_or("").to_string();
    let size = ii["size"].as_u64().unwrap_or(0);
    let duration_secs = ii["duration"].as_f64();
    let description_url = ii["descriptionurl"].as_str().unwrap_or("").to_string();

    let ext = |key: &str| {
        ii["extmetadata"][key]["value"]
            .as_str()
            .map(strip_html)
            .filter(|v| !v.trim().is_empty())
    };

    let mut fields = Vec::new();
    if let Some(v) = ext("ImageDescription") {
        fields.push(("Description".to_string(), v));
    }
    if let Some(v) = ext("Artist") {
        fields.push(("Author".to_string(), v));
    }
    if let Some(v) = ext("Credit") {
        fields.push(("Source".to_string(), v));
    }
    if let Some(v) = ext("DateTimeOriginal") {
        fields.push(("Date".to_string(), v));
    }
    if let Some(v) = ext("LicenseShortName") {
        fields.push(("License".to_string(), v));
    }
    // The licence dialog links to this when License is a short name, so
    // licences that are not Creative Commons (GFDL, ...) get a link too.
    if let Some(v) = ext("LicenseUrl") {
        fields.push(("License URL".to_string(), v));
    }
    if !description_url.is_empty() {
        fields.push(("URL".to_string(), description_url.clone()));
    }

    let mut file = WikiFile {
        name: strip_namespace(&title),
        title,
        url,
        mime,
        size,
        duration_secs,
        description_url,
        time_expansion: None,
        fields,
        host: stored_on(api_host, page),
    };
    // A page fetched with its MediaInfo slot. The app's own queries leave
    // the slot out and add it afterwards (`add_structured_data`).
    if let Some(statements) = mediainfo_statements(page) {
        file.apply_statements(&statements);
    }
    Some(file)
}

/// Render a factor the way people write it: `\u{00D7}10`, `\u{00D7}2.5`.
pub fn format_factor(te: f64) -> String {
    if (te - te.round()).abs() < 1e-9 {
        format!("\u{00D7}{}", te.round() as i64)
    } else {
        format!("\u{00D7}{te}")
    }
}

fn pages_of(json: &serde_json::Value) -> Vec<serde_json::Value> {
    json["query"]["pages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Look up one file. When the link came from another wiki, ask that wiki
/// first: its own upload takes the title's place there, even when Commons has
/// a different file of the same name. A Commons file is fetched from Commons,
/// with its structured data (P14424). When the wiki can't be reached, Commons
/// is still asked.
pub async fn fetch_file(api_host: &str, title: &str) -> Result<WikiFile, String> {
    if api_host != COMMONS {
        match api_get(api_host, &params_for(&[("titles", title)])).await {
            Ok(json) => {
                let file = pages_of(&json)
                    .first()
                    .and_then(|p| parse_file_page(p, api_host));
                if let Some(file) = file.filter(|f| !f.on_commons()) {
                    return Ok(file);
                }
            }
            Err(e) => log::warn!("Looking up {title} on {api_host} failed: {e}"),
        }
    }
    let json = api_get(COMMONS, &params_for(&[("titles", title)])).await?;
    if let Some(file) = pages_of(&json)
        .first()
        .and_then(|p| parse_file_page(p, COMMONS))
    {
        return Ok(add_structured_data(vec![file]).await.remove(0));
    }
    Err(if api_host == COMMONS {
        format!("{title} isn't on Wikimedia Commons")
    } else {
        format!("{title} isn't on {api_host} or Wikimedia Commons")
    })
}

/// How many titles one `titles=` query may name.
const TITLE_BATCH: usize = 50;

fn collect_audio(json: &serde_json::Value, api_host: &str) -> Vec<WikiFile> {
    let mut files: Vec<WikiFile> = pages_of(json)
        .iter()
        .filter_map(|p| parse_file_page(p, api_host))
        .filter(|f| f.mime.starts_with("audio/") || has_audio_ext(&f.name))
        .collect();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

/// List the audio files an article uses.
pub async fn list_page_audio(api_host: &str, title: &str) -> Result<Vec<WikiFile>, String> {
    let params = params_for(&[
        ("generator", "images"),
        ("gimlimit", "500"),
        ("titles", title),
    ]);
    let json = api_get(api_host, &params).await?;
    let files = collect_audio(&json, api_host);
    if files.is_empty() {
        return Ok(files);
    }
    // Files another wiki lists from Commons get their structured data too.
    Ok(add_structured_data(files).await)
}

/// Search Commons for audio files matching free text.
pub async fn search_audio(query: &str) -> Result<Vec<WikiFile>, String> {
    let search = format!("filetype:audio {query}");
    let params = params_for(&[
        ("generator", "search"),
        ("gsrsearch", &search),
        ("gsrnamespace", "6"),
        ("gsrlimit", "40"),
    ]);
    let json = api_get(COMMONS, &params).await?;
    Ok(add_structured_data(collect_audio(&json, COMMONS)).await)
}

/// Resolve whatever the user typed. A file resolves to a single-entry list;
/// an article or a search resolves to everything that matched.
pub async fn resolve(target: &WikiTarget) -> Result<Vec<WikiFile>, String> {
    match target {
        WikiTarget::File { api_host, title } => Ok(vec![fetch_file(api_host, title).await?]),
        WikiTarget::Page { api_host, title } => list_page_audio(api_host, title).await,
        WikiTarget::Search { query } => search_audio(query).await,
    }
}

// ── Loading ──────────────────────────────────────────────────────────────

/// Download a file and hand it to the normal loader, correcting the sample
/// rate when the file declares a time expansion factor.
pub async fn load_file(state: AppState, file: &WikiFile) -> Result<(), String> {
    use crate::state::store_fields::*;
    use leptos::prelude::*;

    let load_id = state.loading_start(&file.name);
    let result = load_file_inner(state, file, load_id).await;
    state.loading_done(load_id);
    match result {
        Err(ref e) => state.show_error_toast(format!("Could not load {}: {e}", file.name)),
        Ok(file_id) => {
            // The factor the file was read with, which is not P14424 when
            // the loader could not use it.
            let applied = state.library.files().with_untracked(|files| {
                files
                    .iter()
                    .find(|f| f.id == file_id)
                    .and_then(|f| f.audio.metadata.time_expansion)
            });
            match (file.time_expansion, applied) {
                (Some(_), Some(te)) => state.show_info_toast(format!(
                    "{} \u{2014} corrected for {} time expansion",
                    file.name,
                    format_factor(te.factor)
                )),
                (Some(te), None) => state.show_error_toast(format!(
                    "{}: the {} time expansion Commons gives could not be applied",
                    file.name,
                    format_factor(te)
                )),
                (None, _) => {}
            }
        }
    }
    result.map(|_| ())
}

/// The time expansion factor (P14424) Commons holds for a file, found from
/// the first of `urls` that names a file: a Commons file page, or a media
/// URL on upload.wikimedia.org. `None` when no URL names a file, the file
/// has no factor, or Commons can't be reached.
pub async fn time_expansion_for_urls(urls: &[&str]) -> Option<f64> {
    let (api_host, title) = urls.iter().find_map(|url| match parse_input(url) {
        Ok(WikiTarget::File { api_host, title }) => Some((api_host, title)),
        _ => None,
    })?;
    match fetch_file(&api_host, &title).await {
        Ok(file) => file.time_expansion,
        Err(e) => {
            log::warn!("No structured data for {title}: {e}");
            None
        }
    }
}

async fn load_file_inner(state: AppState, file: &WikiFile, load_id: u64) -> Result<u64, String> {
    let bytes = crate::components::file_sidebar::fetch_bytes(&file.url).await?;
    crate::components::file_sidebar::load_named_bytes_id(
        file.name.clone(),
        &bytes,
        Some(SiteMetadata::new(file.site(), file.fields.clone())),
        None,
        file.time_expansion,
        state,
        load_id,
        false,
    )
    .await
}

/// Resolve and load the first match in one step — used by the URL-hash
/// deep link. Reports its own failures, since there's no UI to hand them to.
pub async fn load_from_input(state: AppState, input: &str) {
    let resolved = async {
        let target = parse_input(input)?;
        let files = resolve(&target).await?;
        files
            .into_iter()
            .next()
            .ok_or_else(|| format!("No audio found for \u{201C}{input}\u{201D}"))
    }
    .await;
    match resolved {
        // load_file surfaces its own errors.
        Ok(file) => {
            let _ = load_file(state, &file).await;
        }
        Err(e) => {
            log::error!("Wikimedia deep link {input:?} failed: {e}");
            state.show_error_toast(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commons_file_page_url() {
        assert_eq!(
            parse_input("https://commons.wikimedia.org/wiki/File:Bat_feeding_buzz.wav").unwrap(),
            WikiTarget::File {
                api_host: COMMONS.to_string(),
                title: "File:Bat feeding buzz.wav".to_string(),
            }
        );
    }

    #[test]
    fn file_page_on_a_local_wiki_keeps_that_wiki() {
        let target = WikiTarget::File {
            api_host: "en.wikipedia.org".to_string(),
            title: "File:Bat out of Hell - chorus.ogg".to_string(),
        };
        for link in [
            "https://en.wikipedia.org/wiki/File:Bat_out_of_Hell_-_chorus.ogg",
            "https://en.m.wikipedia.org/wiki/File:Bat_out_of_Hell_-_chorus.ogg",
            "https://en.wikipedia.org/w/index.php?title=File:Bat_out_of_Hell_-_chorus.ogg",
        ] {
            assert_eq!(parse_input(link).unwrap(), target, "{link}");
        }
    }

    #[test]
    fn mobile_commons_links_name_commons() {
        assert_eq!(
            parse_input("https://commons.m.wikimedia.org/wiki/File:Bat_feeding_buzz.wav").unwrap(),
            WikiTarget::File {
                api_host: COMMONS.to_string(),
                title: "File:Bat feeding buzz.wav".to_string(),
            }
        );
    }

    #[test]
    fn parses_direct_upload_url() {
        assert_eq!(
            parse_input("https://upload.wikimedia.org/wikipedia/commons/e/e7/Bat_feeding_buzz.wav")
                .unwrap(),
            WikiTarget::File {
                api_host: COMMONS.to_string(),
                title: "File:Bat feeding buzz.wav".to_string(),
            }
        );
        assert_eq!(
            parse_input(
                "https://upload.wikimedia.org/wikipedia/commons/thumb/e/e7/Bat_feeding_buzz.wav/x.wav"
            )
            .unwrap(),
            WikiTarget::File {
                api_host: COMMONS.to_string(),
                title: "File:Bat feeding buzz.wav".to_string(),
            }
        );
    }

    #[test]
    fn direct_upload_url_of_a_wikipedia_file_names_that_wikipedia() {
        assert_eq!(
            parse_input(
                "https://upload.wikimedia.org/wikipedia/en/1/1d/Bat_out_of_Hell_-_chorus.ogg"
            )
            .unwrap(),
            WikiTarget::File {
                api_host: "en.wikipedia.org".to_string(),
                title: "File:Bat out of Hell - chorus.ogg".to_string(),
            }
        );
    }

    #[test]
    fn direct_upload_urls_of_other_wikis_go_to_commons() {
        for dir in [
            "commons",
            "meta",
            "species",
            "mediawiki",
            "foundation",
            "sources",
            "incubator",
            "wikimania2014",
            "fdc",
            "arbcom-en",
            "En",
        ] {
            assert_eq!(
                parse_input(&format!(
                    "https://upload.wikimedia.org/wikipedia/{dir}/1/1d/Bat_out_of_Hell_-_chorus.ogg"
                ))
                .unwrap(),
                WikiTarget::File {
                    api_host: COMMONS.to_string(),
                    title: "File:Bat out of Hell - chorus.ogg".to_string(),
                },
                "{dir}"
            );
        }
        for lang in [
            "en",
            "de",
            "simple",
            "zh-yue",
            "zh-min-nan",
            "zh-classical",
            "bat-smg",
            // Wikis at ten.wikipedia.org and wg-en.wikipedia.org.
            "ten",
            "wg-en",
            // The test wikis at test.wikipedia.org and test2.wikipedia.org.
            "test",
            "test2",
        ] {
            assert_eq!(
                parse_input(&format!(
                    "https://upload.wikimedia.org/wikipedia/{lang}/1/1d/Bat_out_of_Hell_-_chorus.ogg"
                ))
                .unwrap(),
                WikiTarget::File {
                    api_host: format!("{lang}.wikipedia.org"),
                    title: "File:Bat out of Hell - chorus.ogg".to_string(),
                },
                "{lang}"
            );
        }
        // be-tarask.wikipedia.org keeps its uploads in the be-x-old folder.
        assert_eq!(
            parse_input(
                "https://upload.wikimedia.org/wikipedia/be-x-old/1/1d/Bat_out_of_Hell_-_chorus.ogg"
            )
            .unwrap(),
            WikiTarget::File {
                api_host: "be-tarask.wikipedia.org".to_string(),
                title: "File:Bat out of Hell - chorus.ogg".to_string(),
            }
        );
    }

    #[test]
    fn direct_upload_url_of_the_ten_wiki_names_that_wiki() {
        // Commons reports this file missing; ten.wikipedia.org has it.
        assert_eq!(
            parse_input("https://upload.wikimedia.org/wikipedia/ten/a/a5/The_Wikipedia_Song.ogg")
                .unwrap(),
            WikiTarget::File {
                api_host: "ten.wikipedia.org".to_string(),
                title: "File:The Wikipedia Song.ogg".to_string(),
            }
        );
    }

    #[test]
    fn direct_upload_url_of_the_test_wiki_names_that_wiki() {
        // Commons reports this file missing; test.wikipedia.org has it.
        assert_eq!(
            parse_input("https://upload.wikimedia.org/wikipedia/test/9/93/Amir.ogg").unwrap(),
            WikiTarget::File {
                api_host: "test.wikipedia.org".to_string(),
                title: "File:Amir.ogg".to_string(),
            }
        );
    }

    /// A file page as an API returns it, with `imagerepository` when given.
    fn file_page(repository: Option<&str>, description_url: &str) -> serde_json::Value {
        let mut page = serde_json::json!({
            "title": "File:Bat out of Hell - chorus.ogg",
            "imageinfo": [{
                "url": "https://upload.wikimedia.org/x.ogg",
                "descriptionurl": description_url,
                "extmetadata": { "LicenseShortName": { "value": "Fair use" } }
            }]
        });
        if let Some(r) = repository {
            page["imagerepository"] = r.into();
        }
        page
    }

    #[test]
    fn a_wikis_own_upload_is_stored_on_that_wiki() {
        let page = file_page(
            Some("local"),
            "https://en.wikipedia.org/wiki/File:Bat_out_of_Hell_-_chorus.ogg",
        );
        let file = parse_file_page(&page, "en.wikipedia.org").unwrap();
        assert_eq!(file.host, "en.wikipedia.org");
        assert!(!file.on_commons());
        assert_eq!(file.site(), SourceSite::Wikipedia);
        assert_eq!(file.site().name(), "Wikipedia");
    }

    #[test]
    fn a_commons_file_shown_on_a_wiki_is_stored_on_commons() {
        let page = file_page(
            Some("shared"),
            "https://commons.wikimedia.org/wiki/File:Bat_feeding_buzz.wav",
        );
        let file = parse_file_page(&page, "en.wikipedia.org").unwrap();
        assert_eq!(file.host, COMMONS);
        assert!(file.on_commons());
        assert_eq!(file.site(), SourceSite::WikimediaCommons);
    }

    #[test]
    fn commons_own_files_are_stored_on_commons() {
        // Commons calls its own files "local".
        let page = file_page(
            Some("local"),
            "https://commons.wikimedia.org/wiki/File:Bat_feeding_buzz.wav",
        );
        let file = parse_file_page(&page, COMMONS).unwrap();
        assert_eq!(file.host, COMMONS);
        assert_eq!(file.site(), SourceSite::WikimediaCommons);
    }

    #[test]
    fn a_file_without_a_repository_is_stored_on_commons() {
        let page = file_page(None, "");
        assert_eq!(
            parse_file_page(&page, "en.wikipedia.org").unwrap().host,
            COMMONS
        );
        assert_eq!(parse_file_page(&page, COMMONS).unwrap().host, COMMONS);
    }

    #[test]
    fn other_sister_wikis_keep_the_commons_label() {
        let page = file_page(Some("local"), "");
        let file = parse_file_page(&page, "en.wikibooks.org").unwrap();
        assert_eq!(file.host, "en.wikibooks.org");
        assert!(!file.on_commons());
        assert_eq!(file.site(), SourceSite::WikimediaCommons);
    }

    #[test]
    fn parses_article_url_as_a_page() {
        assert_eq!(
            parse_input("https://en.wikipedia.org/wiki/Bat").unwrap(),
            WikiTarget::Page {
                api_host: "en.wikipedia.org".to_string(),
                title: "Bat".to_string(),
            }
        );
    }

    #[test]
    fn bare_titles_and_filenames_go_to_commons() {
        assert!(matches!(
            parse_input("File:Bat feeding buzz.wav").unwrap(),
            WikiTarget::File { .. }
        ));
        assert!(matches!(
            parse_input("Bat_feeding_buzz.wav").unwrap(),
            WikiTarget::File { .. }
        ));
    }

    #[test]
    fn free_text_becomes_a_search() {
        assert_eq!(
            parse_input("hoary bat echolocation").unwrap(),
            WikiTarget::Search {
                query: "hoary bat echolocation".to_string()
            }
        );
    }

    #[test]
    fn non_wikimedia_hosts_are_rejected() {
        let err = parse_input("https://example.com/wiki/File:Foo.wav").unwrap_err();
        assert!(err.contains("example.com"), "{err}");
    }

    #[test]
    fn empty_input_is_rejected() {
        assert!(parse_input("   ").is_err());
    }

    #[test]
    fn reads_p14424_from_a_mediainfo_slot() {
        let page = serde_json::json!({
            "revisions": [{ "slots": { "mediainfo": { "content":
                r#"{"statements":{"P14424":[{"mainsnak":{"snaktype":"value",
                   "datavalue":{"value":{"amount":"+10","unit":"1"},
                   "type":"quantity"}},"rank":"normal"}]}}"#
            }}}]
        });
        assert_eq!(time_expansion_from_mediainfo(&page), Some(10.0));
    }

    #[test]
    fn prefers_a_preferred_rank_statement_and_skips_deprecated() {
        let page = serde_json::json!({
            "revisions": [{ "slots": { "mediainfo": { "content":
                r#"{"statements":{"P14424":[
                   {"mainsnak":{"snaktype":"value","datavalue":{"value":{"amount":"+3"}}},"rank":"deprecated"},
                   {"mainsnak":{"snaktype":"value","datavalue":{"value":{"amount":"+8"}}},"rank":"normal"},
                   {"mainsnak":{"snaktype":"value","datavalue":{"value":{"amount":"+20"}}},"rank":"preferred"}
                   ]}}"#
            }}}]
        });
        assert_eq!(time_expansion_from_mediainfo(&page), Some(20.0));
    }

    #[test]
    fn missing_or_valueless_statements_yield_none() {
        let none = serde_json::json!({ "revisions": [{ "slots": { "mediainfo": {
            "content": r#"{"statements":{}}"# }}}] });
        assert_eq!(time_expansion_from_mediainfo(&none), None);

        let somevalue = serde_json::json!({ "revisions": [{ "slots": { "mediainfo": { "content":
            r#"{"statements":{"P14424":[{"mainsnak":{"snaktype":"somevalue"},"rank":"normal"}]}}"#
        }}}] });
        assert_eq!(time_expansion_from_mediainfo(&somevalue), None);
    }

    /// Statements as Commons returns them for File:Chirps190918-22s.mp3.
    #[test]
    fn reads_the_date_and_coordinates_from_structured_data() {
        let statements = serde_json::json!({
            "P571": [{ "mainsnak": { "snaktype": "value", "datavalue": { "value": {
                "time": "+2018-09-19T00:00:00Z", "precision": 11 } } }, "rank": "normal" }],
            "P1259": [{ "mainsnak": { "snaktype": "value", "datavalue": { "value": {
                "latitude": 49.510472, "longitude": 10.959275 } } }, "rank": "normal" }],
            "P14424": [{ "mainsnak": { "snaktype": "value", "datavalue": { "value": {
                "amount": "+256", "unit": "1" } } }, "rank": "normal" }],
        });
        assert_eq!(date_from(&statements).as_deref(), Some("2018-09-19"));
        assert_eq!(
            coordinates_from(&statements).as_deref(),
            Some("49.510472, 10.959275")
        );
        assert_eq!(time_expansion_from(&statements), Some(256.0));

        let year = serde_json::json!({ "P571": [{ "mainsnak": { "snaktype": "value",
            "datavalue": { "value": { "time": "+1998-00-00T00:00:00Z", "precision": 9 } } },
            "rank": "normal" }] });
        assert_eq!(date_from(&year).as_deref(), Some("1998"));
        assert_eq!(coordinates_from(&year), None);
    }

    #[test]
    fn an_ogg_video_counts_as_a_file_with_sound() {
        assert!(has_audio_ext("Bats.sv001.ogv"));
    }

    #[test]
    fn keeps_the_license_url() {
        let page = serde_json::json!({
            "title": "File:Bat feeding buzz.wav",
            "imagerepository": "local",
            "imageinfo": [{
                "url": "https://upload.wikimedia.org/wikipedia/commons/e/e7/Bat_feeding_buzz.wav",
                "descriptionurl": "https://commons.wikimedia.org/wiki/File:Bat_feeding_buzz.wav",
                "extmetadata": {
                    "LicenseShortName": { "value": "CC0" },
                    "LicenseUrl": { "value": "http://creativecommons.org/publicdomain/zero/1.0/deed.en" }
                }
            }]
        });
        let file = parse_file_page(&page, COMMONS).unwrap();
        let field = |key: &str| {
            file.fields
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(field("License"), Some("CC0"));
        assert_eq!(
            field("License URL"),
            Some("http://creativecommons.org/publicdomain/zero/1.0/deed.en")
        );
    }

    #[test]
    fn strips_html_from_descriptions() {
        assert_eq!(
            strip_html("<p>Echolocation of <i>Pipistrellus</i> &amp; friends</p>"),
            "Echolocation of Pipistrellus & friends"
        );
    }

    #[test]
    fn corrected_duration_divides_by_the_factor() {
        let mut f = WikiFile {
            title: "File:X.wav".into(),
            name: "X.wav".into(),
            url: String::new(),
            mime: "audio/wav".into(),
            size: 0,
            duration_secs: Some(30.0),
            description_url: String::new(),
            time_expansion: Some(10.0),
            fields: Vec::new(),
            host: COMMONS.to_string(),
        };
        assert_eq!(f.corrected_duration_secs(), Some(3.0));
        f.time_expansion = None;
        assert_eq!(f.corrected_duration_secs(), Some(30.0));
    }

    #[test]
    fn formats_factors_the_way_people_write_them() {
        assert_eq!(format_factor(10.0), "\u{00D7}10");
        assert_eq!(format_factor(2.5), "\u{00D7}2.5");
    }
}

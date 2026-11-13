//! Find recordings: one dialog to find a species and load a recording of it, from every
//! source the server's combined search indexes, plus the demo recordings.
//!
//! Replaces the separate "Load demo", "Explore XC" and "From Wikipedia" buttons. The
//! older Xeno-canto and Commons browsers stay reachable from the footer, and take over
//! when the server has no combined search (an older server, or offline).
//!
//! Layout: search box, filter row (group, country, sample rate, near + radius), then a
//! species list (left) and the selected entry's recordings (right). On a phone the two
//! panes show one at a time. Both lists load their next page as they are scrolled near
//! the end; the server sorts them (column headings), since it sends them a page at a time.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};

use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::components::file_sidebar::{
    fetch_bytes, fetch_demo_index, find_open_demo, load_named_bytes, load_single_demo, DemoEntry,
};
use crate::search_client::{
    self as search, Facets, Filter, RecSort, RecordingItem, SearchError, SpeciesHit, SpeciesSort,
    DEFAULT_KM, GROUPS, MIN_RATES, RADII_KM,
};
use crate::source_site::{SiteMetadata, SourceSite};
use crate::state::store_fields::*;
use crate::state::AppState;
use crate::web_util::sleep_ms;
use crate::xc_client::{self, XcSource};

// ── Strings ─────────────────────────────────────────────────────────────────

const TITLE: &str = "Find recordings";
const SEARCH_PLACEHOLDER: &str =
    "Species name in any language, or a Xeno-canto, Wikimedia Commons or Wikipedia link";
/// For screens too narrow for [`SEARCH_PLACEHOLDER`].
const SEARCH_PLACEHOLDER_SHORT: &str = "Species name or link";
const ALL_SPECIES: &str = "All species";
const ALL_COUNTRIES: &str = "All countries";
const NEAR_ANYWHERE: &str = "Anywhere";
const NEAR_ME: &str = "Me";
const NEAR_FILE: &str = "Current recording";
const NEAR_FILE_NO_LOCATION: &str = "Current recording (no GPS location)";
const NEAR_FILE_NONE_OPEN: &str = "Current recording (no recording open)";
const LOCATING: &str = "Finding your location\u{2026}";
const UNAVAILABLE: &str =
    "Species search is unavailable: the search server cannot be reached or is out of date.";
const INDEXING: &str =
    "Incomplete results. The search server is adding {sources} recordings, which can take hours.";
const NO_SPECIES_MATCH: &str = "No species match \u{201C}{query}\u{201D}.";
const NO_RECORDINGS_SPECIES: &str = "No recordings of this species found on {sources}.";
const NO_RECORDINGS_FILTERS: &str = "No recordings match the filters above.";
const LOCATION_DENIED: &str = "Location access denied. Near filter set to Anywhere.";
const LOCATION_FAILED: &str = "Could not get your location: {reason}. Near filter set to Anywhere. Choose \u{201C}Me\u{201D} to try again.";
const TRUNCATED: &str =
    "Only the nearest recordings within {km} km are listed (search limit: about 5,000 recordings).";
const APPROX_TIP: &str =
    "Approximate count. It may include some recordings outside the {km} km radius.";
const LOAD_FAILED: &str = "Could not load recording: {name} ({reason})";
/// End of a list that has more pages: "100 of 848 species".
const SHOWN_OF: &str = "{shown} of {total}";
/// When the server only estimated the total.
const SHOWN_OF_ABOUT: &str = "{shown} of about {total}";
const LOADING_MORE: &str = "Loading more\u{2026}";
const MORE_FAILED: &str = "Couldn't load more: {reason}";
const TRY_AGAIN: &str = "Try again";
const RATE_LABEL: &str = "Sample rate";
const RATE_ANY: &str = "Any";
/// A sample-rate menu entry: "96 kHz or more".
const RATE_OPTION: &str = "{khz} kHz or more";
/// Under the recordings heading when the sample-rate filter is on.
const NO_RATE_NOTE: &str = "Left out: {count} with an unknown sample rate ({sources}).";
/// Before the sort buttons on a phone, where the column headings are hidden.
const SORT_BY: &str = "Sort by";
const LIST_HEAD_SPECIES: &str = "Species";
const LIST_HEAD_COUNT: &str = "Recordings";
const NO_RECORDINGS_HEADING: &str = "Species with no recordings";
const LOAD_WIKI_PAGE: &str = "List the audio files of this Wikipedia page";
const DEMOS_HEADING: &str = "Demo recordings";
const DEMOS_NOTE: &str = "Hand-picked recordings, mostly of bats. They load quickly.";
const ALL_RECORDINGS: &str = "All recordings";
const XC_BROWSER: &str = "Explore Xeno-canto";
const COMMONS_BROWSER: &str = "Search Wikimedia Commons";

/// Country names that take "the" in a sentence ("in the United States").
const COUNTRIES_WITH_THE: [&str; 14] = [
    "US", "GB", "NL", "PH", "BS", "GM", "CZ", "DO", "CF", "CD", "AE", "MV", "MH", "SB",
];

/// "Xeno-canto", "Xeno-canto or iNaturalist", "A, B or C" (or "and").
fn join_list(items: &[String], word: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} {word} {last}", rest.join(", ")),
    }
}

/// The names of the sources whose status is one of `states`, or None if there are none.
/// The server also lists sources it does not index ("off").
fn sources_in(
    status: &std::collections::BTreeMap<String, String>,
    states: &[&str],
) -> Option<Vec<String>> {
    let names: Vec<String> = status
        .iter()
        .filter(|(_, s)| states.contains(&s.as_str()))
        .map(|(k, _)| search::source_label(k).to_string())
        .collect();
    (!names.is_empty()).then_some(names)
}

fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut s = template.to_string();
    for (k, v) in pairs {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", with_commas(n))
    }
}

/// "1,204 recordings", or "about 1,204 recordings" when the server estimated it.
fn count_text(n: u64, exact: bool) -> String {
    let text = plural(n, "recording", "recordings");
    if exact {
        text
    } else {
        format!("about {text}")
    }
}

fn with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ── Model ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NearMode {
    Anywhere,
    Me,
    File,
}

/// What the right pane shows.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
enum Selected {
    Demos,
    All,
    Species(SpeciesHit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Availability {
    Unknown,
    Ready,
    Unavailable,
}

/// The dialog's choices, kept while the app runs so reopening the dialog shows the same
/// search. Not saved across reloads: "All species", all countries, anywhere is the
/// starting point.
#[derive(Clone)]
struct Memory {
    query: String,
    group: Option<String>,
    country: Option<String>,
    near_mode: NearMode,
    near_point: Option<(f64, f64)>,
    km: u32,
    min_smp: Option<u32>,
    rec_sort: RecSort,
    species_sort: SpeciesSort,
    selected: Selected,
}

thread_local! {
    static MEMORY: RefCell<Option<Memory>> = const { RefCell::new(None) };
}

/// A pasted link that loads one recording directly.
#[derive(Clone, Debug, PartialEq)]
enum Link {
    Xc(u64),
    /// A Commons or Wikipedia file: loads directly.
    Wikimedia(String),
    /// A Wikipedia article, which may have several audio files: listed in the
    /// Commons dialog.
    WikiPage(String),
}

fn detect_link(text: &str) -> Option<Link> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    // A bare number is not taken as an XC number: it could be a typo in a name.
    if lower.starts_with("xc") || lower.contains("xeno-canto.org/") {
        if let Ok(n) = xc_lib::api::parse_xc_number(&t.replace(' ', "")) {
            return Some(Link::Xc(n));
        }
    }
    match crate::wikimedia::parse_input(t) {
        Ok(crate::wikimedia::WikiTarget::File { .. }) => Some(Link::Wikimedia(t.to_string())),
        Ok(crate::wikimedia::WikiTarget::Page { .. }) => Some(Link::WikiPage(t.to_string())),
        _ => None,
    }
}

/// "CC BY-NC-SA 4.0" from a Creative Commons licence URL; the input otherwise.
fn licence_short(licence: &str) -> String {
    let l = licence.trim();
    if let Some(idx) = l.find("creativecommons.org/") {
        let rest = &l[idx + "creativecommons.org/".len()..];
        let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
        return match parts.as_slice() {
            ["publicdomain", "zero", v, ..] => format!("CC0 {v}"),
            ["publicdomain", "mark", ..] => "Public domain".to_string(),
            ["licenses", kind, v, ..] => format!("CC {} {v}", kind.to_ascii_uppercase()),
            ["licenses", kind] => format!("CC {}", kind.to_ascii_uppercase()),
            _ => l.to_string(),
        };
    }
    match l.to_ascii_lowercase().as_str() {
        "cc0" => "CC0".into(),
        "cc-by" => "CC BY".into(),
        "cc-by-nc" => "CC BY-NC".into(),
        "cc-by-sa" => "CC BY-SA".into(),
        "cc-by-nc-sa" => "CC BY-NC-SA".into(),
        "cc-by-nd" => "CC BY-ND".into(),
        "cc-by-nc-nd" => "CC BY-NC-ND".into(),
        _ => l.to_string(),
    }
}

fn quality_label(q: &str) -> String {
    match q {
        "research" => "Research grade".into(),
        "needs_id" => "Needs ID".into(),
        other => other.to_string(),
    }
}

/// "96 kHz or more".
fn rate_label(min_smp: u32) -> String {
    fill(RATE_OPTION, &[("khz", &(min_smp / 1000).to_string())])
}

/// What a click on a recordings column heading does, as its tooltip. On the sorted
/// heading that is the other direction.
fn sort_tip(sort: RecSort) -> &'static str {
    match sort {
        RecSort::Default | RecSort::Best => "Sort: best quality first",
        RecSort::Nearest => "Sort: nearest first",
        RecSort::Newest => "Sort: newest first",
        RecSort::Oldest => "Sort: oldest first",
        RecSort::Longest => "Sort: longest first",
        RecSort::Shortest => "Sort: shortest first",
        RecSort::RateHigh => "Sort: highest sample rate first",
        RecSort::RateLow => "Sort: lowest sample rate first",
    }
}

/// The heading arrow of an order: up for smallest first.
fn sort_arrow(sort: RecSort) -> &'static str {
    match sort {
        RecSort::Oldest | RecSort::Shortest | RecSort::RateLow | RecSort::Nearest => "\u{25B2}",
        _ => "\u{25BC}",
    }
}

/// Why a later page did not load, after "Couldn't load more: ".
fn failure_text(e: SearchError) -> String {
    match e {
        SearchError::Unavailable => "the search server can't be reached".into(),
        SearchError::Indexing => "the search server is updating its index".into(),
        SearchError::Other(m) => m,
    }
}

/// How far from the end of a pane (px) the next page starts loading: about ten rows.
const LOAD_AHEAD_PX: i32 = 600;

/// Whether a pane is scrolled near its end, so its next page should load. A hidden pane
/// (the other pane on a phone) has no height and never asks.
fn near_end(el: &web_sys::Element) -> bool {
    let h = el.client_height();
    h > 0 && el.scroll_top() + h >= el.scroll_height() - LOAD_AHEAD_PX
}

fn group_label(grp: &str) -> &str {
    GROUPS
        .iter()
        .find(|(k, _)| *k == grp)
        .map(|(_, label)| *label)
        .unwrap_or(grp)
}

/// The open recording's position, from its GUANO or source metadata.
fn open_file_point(state: AppState) -> Result<(f64, f64), &'static str> {
    let idx = state
        .library
        .current_index()
        .get()
        .ok_or(NEAR_FILE_NONE_OPEN)?;
    state
        .library
        .files()
        .with(|files| {
            files
                .get(idx)
                .and_then(crate::classifier::place::file_place)
                .map(|p| (p.lat as f64, p.lon as f64))
        })
        .ok_or(NEAR_FILE_NO_LOCATION)
}

/// Whether the window is phone-width, where the long search placeholder is cut off.
fn narrow_screen() -> bool {
    web_sys::window()
        .and_then(|w| w.inner_width().ok())
        .and_then(|w| w.as_f64())
        .is_some_and(|w| w < 600.0)
}

// ── Loading ─────────────────────────────────────────────────────────────────

/// Downloads `item` and adds it to the library as the current file.
async fn load_item(state: AppState, item: RecordingItem) -> Result<(), String> {
    let label = if item.en.is_empty() {
        format!("{} {}", item.gen, item.sp)
    } else {
        item.en.clone()
    };
    match item.audio_via.as_str() {
        "xc" => {
            let id = item
                .xc_number()
                .ok_or_else(|| format!("unexpected id {}", item.id))?;
            load_xc(state, id).await
        }
        via => {
            let url = if via == "proxy" {
                format!(
                    "{}/search/audio/{}",
                    xc_client::proxy_base(),
                    String::from(js_sys::encode_uri_component(&item.id))
                )
            } else {
                item.audio_url.clone()
            };
            let bytes = fetch_bytes(&url).await?;
            let name = file_name(&item);
            let load_id = state.loading_start(&label);
            let site = SourceSite::from_search_key(&item.source);
            let result = load_named_bytes(
                name,
                &bytes,
                Some(SiteMetadata::new(site, item_metadata(&item))),
                None,
                item.time_expansion,
                state,
                load_id,
                false,
            )
            .await;
            state.loading_done(load_id);
            result?;
            select_last_file(state);
            Ok(())
        }
    }
}

/// "Nathusius's Pipistrelle - Pipistrellus nathusii (RBINS).wav": the same shape as
/// Xeno-canto's names, since the sources' own file names are often codes. The
/// extension comes from the file URL; RBINS's ".raw" files are WAV.
fn file_name(item: &RecordingItem) -> String {
    let path = item.audio_url.split(['?', '#']).next().unwrap_or("");
    let ext = path
        .rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase())
        .filter(|e| (2..=4).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| if e == "raw" { "wav".to_string() } else { e })
        .unwrap_or_else(|| "wav".to_string());
    let sci = format!("{} {}", item.gen, item.sp).trim().to_string();
    let source = search::source_label(&item.source);
    let base = if item.en.is_empty() {
        format!("{sci} ({source})")
    } else {
        format!("{} - {sci} ({source})", item.en)
    };
    format!("{}.{ext}", base.replace(['/', '\\'], "-"))
}

/// Label/value rows for a recording from a source other than Xeno-canto. The labels
/// match the ones other parts of the app look up (Scientific name, Country,
/// Coordinates, License, Date, URL).
fn item_metadata(item: &RecordingItem) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut add = |label: &str, value: String| {
        if !value.trim().is_empty() {
            rows.push((label.to_string(), value));
        }
    };
    add("Species", item.en.clone());
    add(
        "Scientific name",
        format!("{} {}", item.gen, item.sp).trim().to_string(),
    );
    add("Recordist", item.recordist.clone());
    add("License", item.licence.clone());
    add("Attribution", item.attribution.clone().unwrap_or_default());
    add("Country", item.country.clone());
    add("Location", item.place.clone());
    if let (Some(lat), Some(lon)) = (item.lat, item.lon) {
        add("Coordinates", format!("{lat}, {lon}"));
    }
    add("Date", item.date.clone());
    add("Quality", quality_label(&item.quality));
    add("URL", item.page_url.clone());
    rows
}

async fn load_xc(state: AppState, id: u64) -> Result<(), String> {
    let src = xc_client::resolve_source(state.is_tauri)
        .await
        .unwrap_or(XcSource::Proxy);
    let rec = xc_client::recording_by_id(&src, id)
        .await
        .map_err(|e| crate::components::xc_browser::error_text(&e, &src))?;
    let dl = xc_client::download(&src, &rec)
        .await
        .map_err(|e| crate::components::xc_browser::error_text(&e, &src))?;
    let load_id = state.loading_start(&dl.filename);
    let result = load_named_bytes(
        dl.filename.clone(),
        &dl.bytes,
        Some(SiteMetadata::new(SourceSite::XenoCanto, dl.metadata)),
        dl.hashes,
        None,
        state,
        load_id,
        false,
    )
    .await;
    state.loading_done(load_id);
    result?;
    let count = select_last_file(state);
    if let (Some(set), Some(last)) = (rec.annotation_set, count.checked_sub(1)) {
        if let Some(file_id) = state.file_id_at(last) {
            crate::annota::attach_xc_set(state, file_id, set);
        }
    }
    Ok(())
}

/// Makes the newest file current; returns the number of files.
fn select_last_file(state: AppState) -> usize {
    let count = state.library.files().with_untracked(|f| f.len());
    if count > 0 {
        state.library.current_index().set(Some(count - 1));
    }
    count
}

// ── Paged lists ─────────────────────────────────────────────────────────────

/// A list the server sends a page at a time: species, or recordings.
struct Pages<T: Send + Sync + 'static> {
    items: RwSignal<Vec<T>>,
    total: RwSignal<u64>,
    cursor: RwSignal<Option<String>>,
    /// The first page is loading.
    loading: RwSignal<bool>,
    /// A later page is loading.
    loading_more: RwSignal<bool>,
    /// Why the last later page failed to load; the list end offers Try again.
    more_error: RwSignal<Option<String>>,
    /// Bumped each time the list is replaced, so its rows are built again rather than
    /// reused: a species keeps its key under new filters, but not its count.
    epoch: RwSignal<u64>,
    /// Request generation: an answer to an older request is dropped.
    gen: StoredValue<u64>,
}

impl<T: Send + Sync + 'static> Clone for Pages<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Send + Sync + 'static> Copy for Pages<T> {}

impl<T: Clone + Send + Sync + 'static> Pages<T> {
    fn new() -> Self {
        Pages {
            items: RwSignal::new(Vec::new()),
            total: RwSignal::new(0),
            cursor: RwSignal::new(None),
            loading: RwSignal::new(false),
            loading_more: RwSignal::new(false),
            more_error: RwSignal::new(None),
            epoch: RwSignal::new(0),
            gen: StoredValue::new(0),
        }
    }

    /// Starts a request for a new first page; returns its generation.
    fn start(&self) -> u64 {
        self.gen.update_value(|g| *g += 1);
        self.loading.set(true);
        self.loading_more.set(false);
        self.more_error.set(None);
        self.gen.get_value()
    }

    fn is_current(&self, gen: u64) -> bool {
        self.gen.try_get_value() == Some(gen)
    }

    /// Empties the list, for a request whose old rows no longer apply.
    fn clear(&self) {
        self.epoch.update(|e| *e += 1);
        self.items.set(Vec::new());
        self.total.set(0);
        self.cursor.set(None);
    }

    /// Replaces the list with a first page.
    fn replace(&self, items: Vec<T>, total: u64, cursor: Option<String>) {
        self.epoch.try_update(|e| *e += 1);
        self.total.try_set(total);
        self.cursor.try_set(cursor);
        self.items.try_set(items);
    }

    /// Adds a later page, leaving out rows already listed: species pages are offsets,
    /// which repeat a species when the server's counts change between pages.
    fn extend(&self, more: Vec<T>, cursor: Option<String>, key: impl Fn(&T) -> &str) {
        self.cursor.try_set(cursor);
        self.items.try_update(|items| {
            let seen: HashSet<String> = items.iter().map(|i| key(i).to_string()).collect();
            items.extend(more.into_iter().filter(|i| !seen.contains(key(i))));
        });
    }

    /// Whether a later page can be asked for now.
    fn can_load_more(&self) -> bool {
        self.gen.try_get_value().is_some()
            && self.cursor.with_untracked(Option::is_some)
            && !self.loading.get_untracked()
            && !self.loading_more.get_untracked()
            && self.more_error.with_untracked(Option::is_none)
    }

    /// The text at the end of the list: "100 of 848 species", or "848 species" when
    /// every row is listed.
    fn end_text(&self, one: &str, many: &str, exact: bool) -> Option<String> {
        let shown = self.items.with(Vec::len) as u64;
        let total = self.total.get();
        if shown == 0 {
            return None;
        }
        let more = self.cursor.with(Option::is_some);
        Some(if !more || (exact && shown >= total) {
            plural(shown, one, many)
        } else if total <= shown {
            // An estimated total the list has already passed.
            format!("{} so far", plural(shown, one, many))
        } else {
            let template = if exact { SHOWN_OF } else { SHOWN_OF_ABOUT };
            format!(
                "{} {}",
                fill(
                    template,
                    &[
                        ("shown", &with_commas(shown)),
                        ("total", &with_commas(total)),
                    ],
                ),
                if total == 1 { one } else { many }
            )
        })
    }
}

// ── Component ───────────────────────────────────────────────────────────────

#[component]
pub fn FindRecordings() -> impl IntoView {
    let state = expect_context::<AppState>();
    let memory = MEMORY.with(|m| m.borrow().clone());

    let query = RwSignal::new(memory.as_ref().map(|m| m.query.clone()).unwrap_or_default());
    let group = RwSignal::new(memory.as_ref().and_then(|m| m.group.clone()));
    let country = RwSignal::new(memory.as_ref().and_then(|m| m.country.clone()));
    let near_mode = RwSignal::new(
        memory
            .as_ref()
            .map(|m| m.near_mode)
            .unwrap_or(NearMode::Anywhere),
    );
    let near_point = RwSignal::new(memory.as_ref().and_then(|m| m.near_point));
    let km = RwSignal::new(memory.as_ref().map(|m| m.km).unwrap_or(DEFAULT_KM));
    let min_smp = RwSignal::new(memory.as_ref().and_then(|m| m.min_smp));
    let rec_sort = RwSignal::new(memory.as_ref().map(|m| m.rec_sort).unwrap_or_default());
    let species_sort = RwSignal::new(memory.as_ref().map(|m| m.species_sort).unwrap_or_default());
    let selected = RwSignal::new(
        memory
            .as_ref()
            .map(|m| m.selected.clone())
            .unwrap_or(Selected::Demos),
    );
    // On a phone, whether the right pane (recordings) is showing instead of the list.
    let show_detail = RwSignal::new(false);

    let availability = RwSignal::new(Availability::Unknown);
    let status: RwSignal<BTreeMap<String, String>> = RwSignal::new(Default::default());
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let locating = RwSignal::new(false);
    // Bumped by "Try again" to repeat the requests after the server was unavailable.
    let retry = RwSignal::new(0u32);

    let species: Pages<SpeciesHit> = Pages::new();
    let species_approx = RwSignal::new(false);

    let recs: Pages<RecordingItem> = Pages::new();
    // False when the server estimated the total (dense "near" searches).
    let recs_exact = RwSignal::new(true);
    let recs_truncated = RwSignal::new(false);
    // With the sample-rate filter: recordings left out for having no known rate.
    let no_rate: RwSignal<BTreeMap<String, u64>> = RwSignal::new(BTreeMap::new());

    let facets: RwSignal<Option<Facets>> = RwSignal::new(None);
    let facets_gen = StoredValue::new(0u64);

    let demos: RwSignal<Vec<DemoEntry>> = RwSignal::new(Vec::new());
    let demos_error: RwSignal<Option<String>> = RwSignal::new(None);
    // Id of the recording (or demo file name) being loaded, so its button shows progress.
    let loading: RwSignal<Option<String>> = RwSignal::new(None);

    // The two scrolling panes, watched for their end.
    let species_el = NodeRef::<html::Div>::new();
    let detail_el = NodeRef::<html::Div>::new();

    // "Current recording" from the last time the dialog was open may be another file
    // now: take its position again, or go back to Anywhere if it has none.
    if near_mode.get_untracked() == NearMode::File {
        match open_file_point(state) {
            Ok((lat, lon)) => near_point.set(Some(search::round_point(lat, lon))),
            Err(_) => {
                near_mode.set(NearMode::Anywhere);
                near_point.set(None);
            }
        }
    }

    let filter = Memo::new(move |_| Filter {
        group: group.get(),
        country: country.get(),
        near: if near_mode.get() == NearMode::Anywhere {
            None
        } else {
            near_point.get()
        },
        km: km.get(),
        min_smp: min_smp.get(),
    });
    let near_on = move || filter.with(|f| f.near.is_some());

    // Servers before the sample-rate filter refuse `sort` and `min_smp`; their facets have
    // no `rates`. Until the facets arrive the controls stay hidden.
    let supports_sort =
        Memo::new(move |_| facets.with(|f| f.as_ref().is_some_and(Facets::supports_sorting)));
    // The orders sent: the server's default order goes without a `sort` parameter.
    let species_sort_sent = Memo::new(move |_| {
        if supports_sort.get() {
            species_sort.get()
        } else {
            SpeciesSort::Count
        }
    });
    let rec_sort_sent = Memo::new(move |_| {
        if !supports_sort.get() {
            return RecSort::Default;
        }
        match (rec_sort.get(), near_on()) {
            (RecSort::Default | RecSort::Nearest, _) | (RecSort::Best, false) => RecSort::Default,
            (s, _) => s,
        }
    });

    // Remember the choices on every change, so closing by Escape or the overlay keeps them.
    Effect::new(move |_| {
        let mem = Memory {
            query: query.get(),
            group: group.get(),
            country: country.get(),
            near_mode: near_mode.get(),
            near_point: near_point.get(),
            km: km.get(),
            min_smp: min_smp.get(),
            rec_sort: rec_sort.get(),
            species_sort: species_sort.get(),
            selected: selected.get(),
        };
        MEMORY.with(|m| *m.borrow_mut() = Some(mem));
    });

    let close = move || state.dialogs.find_recordings_open().set(false);

    let note_status = move |s: &BTreeMap<String, String>| {
        status.try_set(s.clone());
    };

    // Species list: runs on every change of the query, filters or order; typing waits
    // 250 ms.
    Effect::new(move |_| {
        retry.track();
        let q = query.get();
        let f = filter.get();
        let sort = species_sort_sent.get();
        // Near with no point yet (still locating) has nothing to search.
        if near_mode.get() != NearMode::Anywhere && f.near.is_none() {
            return;
        }
        let my = species.start();
        if detect_link(&q).is_some() {
            species.clear();
            species.loading.set(false);
            return;
        }
        spawn_local(async move {
            if !q.trim().is_empty() {
                sleep_ms(250).await;
            }
            if !species.is_current(my) {
                return;
            }
            let result = search::species(&q, &f, sort, None).await;
            if !species.is_current(my) {
                return;
            }
            match result {
                Ok(page) => {
                    availability.try_set(Availability::Ready);
                    note_status(&page.status);
                    species_approx.try_set(page.approx);
                    species.replace(page.species, page.total, page.cursor);
                    if let Some(el) = species_el.try_get_untracked().flatten() {
                        el.set_scroll_top(0);
                    }
                }
                Err(SearchError::Unavailable) => {
                    availability.try_set(Availability::Unavailable);
                    species.replace(Vec::new(), 0, None);
                }
                Err(SearchError::Indexing) => {
                    availability.try_set(Availability::Ready);
                    status.try_set([("xc".to_string(), "indexing".to_string())].into());
                    species.replace(Vec::new(), 0, None);
                }
                Err(SearchError::Other(m)) => {
                    error.try_set(Some(m));
                }
            }
            species.loading.try_set(false);
        });
    });

    // Facets for the Group, Country and Sample rate menus.
    Effect::new(move |_| {
        retry.track();
        let f = filter.get();
        facets_gen.update_value(|g| *g += 1);
        let my = facets_gen.get_value();
        spawn_local(async move {
            let result = search::facets(&f).await;
            if facets_gen.try_get_value() != Some(my) {
                return;
            }
            if let Ok(fc) = result {
                facets.try_set(Some(fc));
            }
        });
    });

    // Recordings of the selected species, or of every species under the filters.
    Effect::new(move |_| {
        let sel = selected.get();
        let f = filter.get();
        let sort = rec_sort_sent.get();
        let my = recs.start();
        recs.clear();
        no_rate.set(BTreeMap::new());
        let key = match &sel {
            Selected::Demos => {
                recs.loading.set(false);
                return;
            }
            Selected::All => None,
            // Always asked: the species' count in the list was taken under the
            // filters at the time, which may have changed since.
            Selected::Species(hit) => Some(hit.key.clone()),
        };
        // Near with no point yet (still locating): wait. Tracked, so that going back to
        // Anywhere (location refused) asks again; the filter does not change then.
        if near_mode.get() != NearMode::Anywhere && f.near.is_none() {
            return;
        }
        if let Some(el) = detail_el.get_untracked() {
            el.set_scroll_top(0);
        }
        spawn_local(async move {
            let result = search::recordings(key.as_deref(), &f, sort, None).await;
            if !recs.is_current(my) {
                return;
            }
            match result {
                Ok(page) => {
                    recs_exact.try_set(page.total_exact);
                    recs_truncated.try_set(page.truncated);
                    no_rate.try_set(page.no_rate);
                    recs.replace(page.items, page.total, page.cursor);
                }
                Err(SearchError::Unavailable) => {
                    availability.try_set(Availability::Unavailable);
                }
                Err(SearchError::Indexing) => {}
                Err(SearchError::Other(m)) => {
                    error.try_set(Some(m));
                }
            }
            recs.loading.try_set(false);
        });
    });

    // With no search, the demo list is all there is: on a phone, show it straight away.
    Effect::new(move |_| {
        if availability.get() == Availability::Unavailable
            && selected.get_untracked() == Selected::Demos
        {
            show_detail.set(true);
        }
    });

    // Demo index, fetched once per dialog.
    spawn_local(async move {
        match fetch_demo_index().await {
            Ok(entries) => {
                demos.try_set(entries);
            }
            Err(e) => {
                demos_error.try_set(Some(e));
            }
        }
    });

    let more_species = move || {
        if !species.can_load_more() {
            return;
        }
        let Some(cursor) = species.cursor.get_untracked() else {
            return;
        };
        let q = query.get_untracked();
        let f = filter.get_untracked();
        let sort = species_sort_sent.get_untracked();
        let my = species.gen.get_value();
        species.loading_more.set(true);
        spawn_local(async move {
            let result = search::species(&q, &f, sort, Some(&cursor)).await;
            if !species.is_current(my) {
                return;
            }
            match result {
                Ok(page) => species.extend(page.species, page.cursor, |h| h.key.as_str()),
                Err(e) => {
                    species.more_error.try_set(Some(failure_text(e)));
                }
            }
            species.loading_more.try_set(false);
        });
    };

    let more_recs = move || {
        if !recs.can_load_more() {
            return;
        }
        let Some(cursor) = recs.cursor.get_untracked() else {
            return;
        };
        let key = match selected.get_untracked() {
            Selected::Species(hit) => Some(hit.key),
            Selected::All => None,
            Selected::Demos => return,
        };
        let f = filter.get_untracked();
        let sort = rec_sort_sent.get_untracked();
        let my = recs.gen.get_value();
        recs.loading_more.set(true);
        spawn_local(async move {
            let result = search::recordings(key.as_deref(), &f, sort, Some(&cursor)).await;
            if !recs.is_current(my) {
                return;
            }
            match result {
                Ok(page) => {
                    recs_truncated.try_set(page.truncated);
                    recs.extend(page.items, page.cursor, |r| r.id.as_str());
                }
                Err(e) => {
                    recs.more_error.try_set(Some(failure_text(e)));
                }
            }
            recs.loading_more.try_set(false);
        });
    };

    // The next page loads when a pane is scrolled near its end.
    let check_species_end = move || {
        if let Some(el) = species_el.try_get_untracked().flatten() {
            if near_end(&el) {
                more_species();
            }
        }
    };
    let check_recs_end = move || {
        if let Some(el) = detail_el.try_get_untracked().flatten() {
            if near_end(&el) {
                more_recs();
            }
        }
    };
    // After a page lands (or the phone swaps panes), the pane may still end within view: a
    // tall window, or a short page. Check again once the rows are drawn.
    Effect::new(move |_| {
        species.items.track();
        show_detail.track();
        spawn_local(async move {
            sleep_ms(50).await;
            check_species_end();
        });
    });
    Effect::new(move |_| {
        recs.items.track();
        show_detail.track();
        spawn_local(async move {
            sleep_ms(50).await;
            check_recs_end();
        });
    });

    let set_near_mode = move |mode: NearMode| {
        error.set(None);
        near_mode.set(mode);
        match mode {
            NearMode::Anywhere => near_point.set(None),
            NearMode::File => match open_file_point(state) {
                Ok((lat, lon)) => near_point.set(Some(search::round_point(lat, lon))),
                Err(_) => {
                    near_mode.set(NearMode::Anywhere);
                    near_point.set(None);
                }
            },
            NearMode::Me => {
                near_point.set(None);
                locating.set(true);
                spawn_local(async move {
                    let result = crate::device_location::ask(state).await;
                    locating.try_set(false);
                    match result {
                        Ok((lat, lon)) => {
                            near_point.try_set(Some(search::round_point(lat, lon)));
                        }
                        Err(e) => {
                            near_mode.try_set(NearMode::Anywhere);
                            let msg = match e {
                                crate::device_location::LocationError::Denied => {
                                    LOCATION_DENIED.to_string()
                                }
                                crate::device_location::LocationError::Unavailable(r) => {
                                    fill(LOCATION_FAILED, &[("reason", &r)])
                                }
                            };
                            error.try_set(Some(msg));
                        }
                    }
                });
            }
        }
    };

    let start_load = move |id: String, item: RecordingItem| {
        if loading.get_untracked().is_some() {
            return;
        }
        loading.set(Some(id));
        let species_name = if item.en.is_empty() {
            format!("{} {}", item.gen, item.sp)
        } else {
            item.en.clone()
        };
        let name = match item.xc_number() {
            Some(n) => format!("XC{n}, {species_name}"),
            None => species_name,
        };
        spawn_local(async move {
            let result = load_item(state, item).await;
            loading.try_set(None);
            match result {
                Ok(()) => close(),
                Err(reason) => {
                    log::error!("Find recordings: {name}: {reason}");
                    error.try_set(Some(fill(
                        LOAD_FAILED,
                        &[("name", &name), ("reason", &reason)],
                    )));
                }
            }
        });
    };

    let load_demo = move |entry: DemoEntry| {
        if let Some(idx) = find_open_demo(state, &entry.filename) {
            state.library.current_index().set(Some(idx));
            close();
            return;
        }
        if loading.get_untracked().is_some() {
            return;
        }
        loading.set(Some(entry.filename.clone()));
        spawn_local(async move {
            let load_id = state.loading_start(&entry.filename);
            let result = load_single_demo(&entry, state, load_id).await;
            state.loading_done(load_id);
            loading.try_set(None);
            match result {
                Ok(()) => close(),
                Err(reason) => {
                    let name = entry.en.clone().unwrap_or_else(|| entry.filename.clone());
                    error.try_set(Some(fill(
                        LOAD_FAILED,
                        &[("name", &name), ("reason", &reason)],
                    )));
                }
            }
        });
    };

    let load_link = move |link: Link| {
        if let Link::WikiPage(text) = &link {
            crate::components::wiki_browser::open_with(state, text);
            state.dialogs.find_recordings_open().set(false);
            return;
        }
        if loading.get_untracked().is_some() {
            return;
        }
        loading.set(Some("link".into()));
        spawn_local(async move {
            let result = match link {
                Link::Xc(id) => load_xc(state, id).await,
                Link::Wikimedia(text) => {
                    crate::wikimedia::load_from_input(state, &text).await;
                    Ok(())
                }
                Link::WikiPage(_) => Ok(()),
            };
            loading.try_set(None);
            match result {
                Ok(()) => close(),
                Err(reason) => {
                    error.try_set(Some(fill(
                        LOAD_FAILED,
                        &[("name", "the link"), ("reason", &reason)],
                    )));
                }
            }
        });
    };

    let open_xc_browser = move || {
        close();
        state.dialogs.xc_browser_open().set(true);
    };
    let open_wiki_browser = move || {
        close();
        state.dialogs.wiki_browser_open().set(true);
    };

    let select = move |s: Selected| {
        selected.set(s);
        show_detail.set(true);
    };

    // ── Filter row ──────────────────────────────────────────────────────

    let group_menu = move || {
        let counts = facets.get().map(|f| f.groups).unwrap_or_default();
        let current = group.get();
        let options = GROUPS
            .iter()
            .filter(|(key, _)| {
                counts.is_empty()
                    || counts.iter().any(|c| c.grp == *key && c.n > 0)
                    || current.as_deref() == Some(*key)
            })
            .map(|(key, label)| {
                let n = counts.iter().find(|c| c.grp == *key).map(|c| c.n);
                let text = match n {
                    Some(n) => format!("{label} ({})", with_commas(n)),
                    None => label.to_string(),
                };
                view! {
                    <option value=*key selected=current.as_deref() == Some(*key)>{text}</option>
                }
            })
            .collect::<Vec<_>>();
        view! {
            <option value="" selected=current.is_none()>{ALL_SPECIES}</option>
            {options}
        }
    };

    let country_menu = move || {
        let mut list = facets.get().map(|f| f.countries).unwrap_or_default();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        let current = country.get();
        let mut options: Vec<_> = list
            .iter()
            .map(|c| {
                let text = format!("{} ({})", c.name, with_commas(c.n));
                let is = current.as_deref() == Some(c.cc.as_str());
                view! { <option value=c.cc.clone() selected=is>{text}</option> }
            })
            .collect();
        if let Some(cur) = current.clone() {
            if !list.iter().any(|c| c.cc == cur) {
                let name = crate::search_client::country_name(&cur);
                options.insert(
                    0,
                    view! { <option value=cur.clone() selected=true>{name}</option> },
                );
            }
        }
        view! {
            <option value="" selected=current.is_none()>{ALL_COUNTRIES}</option>
            {options}
        }
    };

    // Rates with no recordings under the other filters are left out, as in the Group menu.
    let rate_menu = move || {
        let counts = facets.with(|f| f.as_ref().and_then(|f| f.rates.clone()).unwrap_or_default());
        let current = min_smp.get();
        let options = MIN_RATES
            .iter()
            .filter_map(|r| {
                let n = counts.iter().find(|c| c.min_smp == *r).map(|c| c.n);
                if n == Some(0) && current != Some(*r) {
                    return None;
                }
                let text = match n {
                    Some(n) => format!("{} ({})", rate_label(*r), with_commas(n)),
                    None => rate_label(*r),
                };
                Some(view! {
                    <option value=r.to_string() selected=current == Some(*r)>{text}</option>
                })
            })
            .collect::<Vec<_>>();
        view! {
            <option value="" selected=current.is_none()>{RATE_ANY}</option>
            {options}
        }
    };

    let file_point = Memo::new(move |_| open_file_point(state));

    // ── Species pane ────────────────────────────────────────────────────

    // Name matches with no recordings come last; a subheading marks where they start.
    let first_zero = Memo::new(move |_| {
        species
            .items
            .with(|l| l.iter().find(|h| h.n == 0).map(|h| h.key.clone()))
    });

    let species_row = move |hit: SpeciesHit| {
        let key = hit.key.clone();
        let key_sel = key.clone();
        let name = hit.display_name();
        let sci = hit.scientific();
        let grp = group_label(&hit.grp).to_string();
        let n = hit.n;
        let approx = species_approx.get_untracked() && filter.with_untracked(|f| f.near.is_some());
        let count = if n == 0 {
            view! { <span class="find-sp-count find-zero">"0"</span> }.into_any()
        } else if approx {
            view! { <span class="find-sp-count" title=fill(APPROX_TIP, &[("km", &km.get_untracked().to_string())])>{format!("~{}", with_commas(n))}</span> }.into_any()
        } else {
            view! { <span class="find-sp-count">{with_commas(n)}</span> }.into_any()
        };
        // Sorted by scientific name, the row leads with it, so the order can be read.
        let (first, second) = if species_sort_sent.get_untracked() == SpeciesSort::Name {
            let rest = if name == sci {
                grp
            } else {
                format!("{name} \u{00B7} {grp}")
            };
            (
                view! { <i>{sci}</i> }.into_any(),
                view! { {rest} }.into_any(),
            )
        } else {
            (
                view! { {name} }.into_any(),
                view! { <i>{sci}</i>" \u{00B7} "{grp} }.into_any(),
            )
        };
        let for_click = hit.clone();
        view! {
            {move || (first_zero.get().as_deref() == Some(key.as_str())).then(|| view! {
                <div class="find-list-subhead">{NO_RECORDINGS_HEADING}</div>
            })}
            <button
                class="find-species-row"
                class:selected=move || selected.with(|s| matches!(s, Selected::Species(h) if h.key == key_sel))
                class:find-no-recordings=n == 0
                on:click=move |_| select(Selected::Species(for_click.clone()))
            >
                <span class="find-sp-name">{first}</span>
                {count}
                <span class="find-sp-sci">{second}</span>
            </button>
        }
    };

    // A column heading that sorts the species list.
    let species_sort_button = move |label: &'static str, sort: SpeciesSort| {
        move || {
            if !supports_sort.get() {
                return view! { <span>{label}</span> }.into_any();
            }
            let active = species_sort_sent.get() == sort;
            let (arrow, tip) = match sort {
                SpeciesSort::Name => ("\u{25B2}", "Sort: by scientific name"),
                SpeciesSort::Count => ("\u{25BC}", "Sort: most recordings first"),
            };
            view! {
                <button class="find-sort" class:active=active title=tip on:click=move |_| species_sort.set(sort)>
                    {label}
                    {active.then(|| view! { <span class="find-sort-arrow">{arrow}</span> })}
                </button>
            }
            .into_any()
        }
    };

    let link_row = move || {
        detect_link(&query.get()).map(|l| {
            let text = match &l {
                Link::Xc(n) => format!("Load XC{n}"),
                Link::Wikimedia(_) => "Load from Wikimedia".to_string(),
                Link::WikiPage(_) => LOAD_WIKI_PAGE.to_string(),
            };
            view! {
                <button class="find-link-row" on:click=move |_| load_link(l.clone())>
                    {move || if loading.get().as_deref() == Some("link") { "Loading\u{2026}".to_string() } else { text.clone() }}
                </button>
            }
        })
    };

    let fixed_rows = move || {
        query.with(|q| q.trim().is_empty()).then(|| {
            let demos_n = demos.with(Vec::len);
            let all_n = facets
                .with(|f| f.as_ref().map(|f| f.sources.iter().map(|s| s.n).sum::<u64>()))
                .filter(|n| *n > 0);
            view! {
                <button
                    class="find-species-row find-fixed-row"
                    class:selected=move || selected.with(|s| *s == Selected::Demos)
                    on:click=move |_| select(Selected::Demos)
                >
                    <span class="find-sp-name">{DEMOS_HEADING}</span>
                    <span class="find-sp-count">{(demos_n > 0).then(|| with_commas(demos_n as u64))}</span>
                </button>
                {(availability.get() != Availability::Unavailable).then(|| view! {
                    <button
                        class="find-species-row find-fixed-row"
                        class:selected=move || selected.with(|s| *s == Selected::All)
                        on:click=move |_| select(Selected::All)
                    >
                        <span class="find-sp-name">{ALL_RECORDINGS}</span>
                        <span class="find-sp-count">{all_n.map(with_commas)}</span>
                    </button>
                })}
            }
        })
    };

    let unavailable_notice = move || {
        (availability.get() == Availability::Unavailable).then(|| view! {
            <div class="find-notice">
                <p>{UNAVAILABLE}</p>
                <div class="find-notice-actions">
                    <button class="xc-btn" on:click=move |_| {
                        availability.set(Availability::Unknown);
                        show_detail.set(false);
                        retry.update(|r| *r += 1);
                    }>{TRY_AGAIN}</button>
                    <button class="xc-btn" on:click=move |_| open_xc_browser()>{XC_BROWSER}</button>
                    <button class="xc-btn" on:click=move |_| open_wiki_browser()>{COMMONS_BROWSER}</button>
                </div>
            </div>
        })
    };

    // The end of a list: how many rows are listed out of how many, and the state of the
    // next page.
    let list_end = move |text: Option<String>,
                         loading_more: RwSignal<bool>,
                         more_error: RwSignal<Option<String>>,
                         retry_more: Box<dyn Fn() + Send + Sync>| {
        text.map(|text| {
            view! {
                <div class="find-list-end">
                    <span>{text}</span>
                    {move || loading_more.get().then(|| view! { <span>{LOADING_MORE}</span> })}
                    {more_error.get().map(|reason| view! {
                        <span class="find-more-error">{fill(MORE_FAILED, &[("reason", &reason)])}</span>
                        <button class="find-more" on:click=move |_| retry_more()>{TRY_AGAIN}</button>
                    })}
                </div>
            }
        })
    };

    let species_end = move || {
        list_end(
            species.end_text("species", "species", true),
            species.loading_more,
            species.more_error,
            Box::new(move || {
                species.more_error.set(None);
                more_species();
            }),
        )
    };

    let species_empty = move || {
        let q = query.get();
        (species.items.with(Vec::is_empty)
            && !species.loading.get()
            && !q.trim().is_empty()
            && detect_link(&q).is_none()
            && availability.get() != Availability::Unavailable)
            .then(|| {
                view! {
                    <p class="find-empty">{fill(NO_SPECIES_MATCH, &[("query", q.trim())])}</p>
                }
            })
    };

    let species_pane = view! {
        {move || sources_in(&status.get(), &["indexing"]).map(|names| view! {
            <p class="find-notice find-notice-small">{fill(INDEXING, &[("sources", &join_list(&names, "and"))])}</p>
        })}
        {link_row}
        {fixed_rows}
        {unavailable_notice}
        {move || species.items.with(|l| !l.is_empty()).then(|| view! {
            <div class="find-list-head">
                {species_sort_button(LIST_HEAD_SPECIES, SpeciesSort::Name)}
                {species_sort_button(LIST_HEAD_COUNT, SpeciesSort::Count)}
            </div>
        })}
        <For
            each=move || {
                let e = species.epoch.get();
                species.items.get().into_iter().map(move |h| (e, h)).collect::<Vec<_>>()
            }
            key=|(e, h)| (*e, h.key.clone())
            children=move |(_, hit)| species_row(hit)
        />
        {species_empty}
        {species_end}
        {move || species.loading.get().then(|| view! { <p class="xc-loading">"Searching\u{2026}"</p> })}
    };

    // ── Detail pane ─────────────────────────────────────────────────────

    // A column heading that sorts the recordings: the first click gives `first`, a click
    // on the sorted heading gives `second` (the other direction).
    let rec_sort_button = move |label: &'static str, first: RecSort, second: RecSort| {
        move || {
            if !supports_sort.get() {
                return view! { <span>{label}</span> }.into_any();
            }
            let current = rec_sort_sent.get().resolved(near_on());
            let active = current == first || current == second;
            let next = if active && current == first {
                second
            } else {
                first
            };
            view! {
                <button class="find-sort" class:active=active title=sort_tip(next) on:click=move |_| rec_sort.set(next)>
                    {label}
                    {active.then(|| view! { <span class="find-sort-arrow">{sort_arrow(current)}</span> })}
                </button>
            }
            .into_any()
        }
    };

    let rec_head = move |show_species: bool| {
        view! {
            <div class="find-rec-head" class:with-species=show_species class:sortable=move || supports_sort.get()>
                <span class="find-sort-label">{SORT_BY}</span>
                {show_species.then(|| view! { <span class="find-head-plain">"Species"</span> })}
                <span class="find-head-place">
                    {move || if near_on() {
                        view! {
                            {rec_sort_button("Distance", RecSort::Nearest, RecSort::Nearest)}
                            <span class="find-head-plain">", place"</span>
                        }
                        .into_any()
                    } else {
                        view! { <span class="find-head-plain">"Place"</span> }.into_any()
                    }}
                </span>
                <span>{rec_sort_button("Date", RecSort::Newest, RecSort::Oldest)}</span>
                <span class="find-rec-num">{rec_sort_button("Length", RecSort::Longest, RecSort::Shortest)}</span>
                <span class="find-rec-num">{rec_sort_button("Sample rate", RecSort::RateHigh, RecSort::RateLow)}</span>
                <span>{rec_sort_button("Quality", RecSort::Best, RecSort::Best)}</span>
                <span class="find-head-plain">"Source"</span>
                <span class="find-head-plain"></span>
            </div>
        }
    };

    let rec_row = move |item: RecordingItem, show_species: bool| {
        let id = item.id.clone();
        let id_for_btn = id.clone();
        let for_click = item.clone();
        let place = {
            let mut parts = Vec::new();
            if !item.country.is_empty() {
                parts.push(item.country.clone());
            }
            if !item.place.is_empty() {
                parts.push(item.place.clone());
            }
            parts.join(", ")
        };
        let place_title = place.clone();
        let dist = item.km.map(|k| {
            if k < 10.0 {
                format!("{k:.1} km")
            } else {
                format!("{} km", k.round() as i64)
            }
        });
        // Xeno-canto gives whole seconds, so 0 is a recording under a second.
        let length = item
            .len_s
            .map(|s| {
                if s < 1.0 {
                    "<1s".to_string()
                } else {
                    crate::format_time::format_duration(s, 0)
                }
            })
            .unwrap_or_default();
        let rate = item
            .smp
            .map(|r| format!("{} kHz", (r as f64 / 1000.0).round() as i64))
            .unwrap_or_default();
        let source = search::source_label(&item.source).to_string();
        let detail = {
            let mut parts = Vec::new();
            if !item.recordist.is_empty() {
                parts.push(item.recordist.clone());
            }
            if !item.licence.is_empty() {
                parts.push(licence_short(&item.licence));
            }
            parts.join(" \u{00B7} ")
        };
        let species_cell = show_species.then(|| {
            let name = if item.en.is_empty() {
                format!("{} {}", item.gen, item.sp)
            } else {
                item.en.clone()
            };
            view! { <span class="find-rec-species">{name}</span> }
        });
        view! {
            <div class="find-rec-row" class:with-species=show_species>
                {species_cell}
                <span class="find-rec-place-cell">
                    {move || near_on().then(|| dist.clone().map(|d| view! { <span class="find-rec-dist">{d}</span> }))}
                    <span class="find-rec-place" title=place_title>{place}</span>
                </span>
                <span class="find-rec-date">{item.date.clone()}</span>
                <span class="find-rec-num">{length}</span>
                <span class="find-rec-num">{rate}</span>
                <span class="find-rec-q">{quality_label(&item.quality)}</span>
                <a class="find-rec-source" href=item.page_url.clone() target="_blank" rel="noopener">{source}</a>
                <button
                    class="xc-btn xc-btn-small find-rec-load"
                    disabled=move || loading.get().is_some()
                    on:click=move |_| start_load(id_for_btn.clone(), for_click.clone())
                >
                    {move || if loading.get().as_deref() == Some(id.as_str()) { "Loading\u{2026}" } else { "Load" }}
                </button>
                <span class="find-rec-detail">{detail}</span>
            </div>
        }
    };

    let recs_end = move || {
        list_end(
            recs.end_text("recording", "recordings", recs_exact.get()),
            recs.loading_more,
            recs.more_error,
            Box::new(move || {
                recs.more_error.set(None);
                more_recs();
            }),
        )
    };

    let recordings_table = move |show_species: bool| {
        view! {
            {rec_head(show_species)}
            <For
                each=move || {
                    let e = recs.epoch.get();
                    recs.items.get().into_iter().map(move |i| (e, i)).collect::<Vec<_>>()
                }
                key=|(e, i)| (*e, i.id.clone())
                children=move |(_, item)| rec_row(item, show_species)
            />
            {move || (recs_truncated.get() && near_on()).then(|| view! {
                <p class="find-empty">{fill(TRUNCATED, &[("km", &km.get().to_string())])}</p>
            })}
            {recs_end}
        }
    };

    let filter_summary = move || {
        let f = filter.get();
        let mut s = String::new();
        if let Some(g) = &f.group {
            s.push_str(&format!(" of {}", group_label(g).to_lowercase()));
        }
        if let Some(c) = &f.country {
            let the = if COUNTRIES_WITH_THE.contains(&c.as_str()) {
                "the "
            } else {
                ""
            };
            s.push_str(&format!(" in {the}{}", search::country_name(c)));
        }
        if let Some(r) = f.min_smp {
            s.push_str(&format!(" at {}", rate_label(r)));
        }
        if f.near.is_some() {
            let whom = match near_mode.get() {
                NearMode::Me => "you",
                _ => "the current recording",
            };
            s.push_str(&format!(" within {} km of {whom}", f.km));
        }
        s
    };

    // With the sample-rate filter on, the recordings it left out for having no known rate.
    let no_rate_note = move || {
        no_rate.with(|nr| {
            let n: u64 = nr.values().sum();
            (n > 0).then(|| {
                let sources: Vec<String> = nr
                    .keys()
                    .map(|k| search::source_label(k).to_string())
                    .collect();
                view! {
                    <p class="find-detail-sub">
                        {fill(
                            NO_RATE_NOTE,
                            &[
                                ("count", &plural(n, "recording", "recordings")),
                                ("sources", &join_list(&sources, "and")),
                            ],
                        )}
                    </p>
                }
            })
        })
    };

    let detail_pane = move || {
        let sel = selected.get();
        let back = view! {
            <button class="xc-btn xc-btn-small find-back" on:click=move |_| show_detail.set(false)>
                {"\u{2190} Species"}
            </button>
        };
        match sel {
            Selected::Demos => {
                let entries = demos.get();
                let bats: Vec<DemoEntry> = entries.iter().filter(|e| e.is_bat()).cloned().collect();
                let random = (!bats.is_empty()).then(|| {
                    view! {
                        <button
                            class="xc-btn demo-random-bat"
                            disabled=move || loading.get().is_some()
                            on:click=move |_| {
                                let unopened: Vec<DemoEntry> = bats
                                    .iter()
                                    .filter(|e| find_open_demo(state, &e.filename).is_none())
                                    .cloned()
                                    .collect();
                                let pool = if unopened.is_empty() { bats.clone() } else { unopened };
                                let idx = (js_sys::Math::random() * pool.len() as f64) as usize;
                                load_demo(pool[idx.min(pool.len() - 1)].clone());
                            }
                        >"Random bat"</button>
                    }
                });
                let rows = entries
                    .into_iter()
                    .map(|entry| {
                        let name = entry.en.clone().unwrap_or_else(|| entry.filename.clone());
                        let sci = entry.species.clone().unwrap_or_default();
                        let file = entry.filename.clone();
                        view! {
                            <div class="find-demo-row">
                                <span class="find-sp-name">{name}</span>
                                <span class="find-sp-sci"><i>{sci}</i></span>
                                <button
                                    class="xc-btn xc-btn-small find-rec-load"
                                    disabled=move || loading.get().is_some()
                                    on:click=move |_| load_demo(entry.clone())
                                >
                                    {move || if loading.get().as_deref() == Some(file.as_str()) { "Loading\u{2026}" } else { "Load" }}
                                </button>
                            </div>
                        }
                    })
                    .collect::<Vec<_>>();
                view! {
                    {back}
                    <div class="find-detail-head">
                        <h3>{DEMOS_HEADING}</h3>
                        <p class="find-detail-sub">{DEMOS_NOTE}</p>
                        {random}
                    </div>
                    {demos_error.get().map(|e| view! { <p class="find-empty">{e}</p> })}
                    <div class="find-demo-list">{rows}</div>
                }
                .into_any()
            }
            Selected::All => view! {
                {back}
                <div class="find-detail-head">
                    <h3>{ALL_RECORDINGS}</h3>
                    <p class="find-detail-sub">
                        {move || format!("{}{}", count_text(recs.total.get(), recs_exact.get()), filter_summary())}
                    </p>
                    {no_rate_note}
                </div>
                {move || (recs.total.get() == 0 && !recs.loading.get()).then(|| view! { <p class="find-empty">{NO_RECORDINGS_FILTERS}</p> })}
                {recordings_table(true)}
                {move || recs.loading.get().then(|| view! { <p class="xc-loading">"Loading\u{2026}"</p> })}
            }
            .into_any(),
            Selected::Species(hit) => {
                let name = hit.display_name();
                let sci = hit.scientific();
                let wiki = hit.wikipedia_url();
                let bb = hit.beastiebot_url();
                let any_filter = move || {
                    let f = filter.get();
                    f.group.is_some() || f.country.is_some() || f.near.is_some() || f.min_smp.is_some()
                };
                view! {
                    {back}
                    <div class="find-detail-head">
                        <h3>{name}" "<i class="find-detail-sci">{sci}</i></h3>
                        <p class="find-detail-sub">
                            {move || match recs.total.get() {
                                0 => String::new(),
                                t => format!("{}{} \u{00B7} ", count_text(t, recs_exact.get()), filter_summary()),
                            }}
                            <a href=wiki target="_blank" rel="noopener">"Wikipedia"</a>
                            " \u{00B7} "
                            <a href=bb target="_blank" rel="noopener">"BeastieBot"</a>
                        </p>
                        {no_rate_note}
                    </div>
                    // With a filter set, no recordings may only mean none under the
                    // filter; without one, the species has none in any indexed source.
                    {move || (recs.total.get() == 0 && !recs.loading.get()).then(|| {
                        let text = if any_filter() {
                            NO_RECORDINGS_FILTERS.to_string()
                        } else {
                            let sources = sources_in(&status.get(), &["ready", "indexing"])
                                .unwrap_or_else(|| vec![SourceSite::XenoCanto.name().to_string()]);
                            fill(NO_RECORDINGS_SPECIES, &[("sources", &join_list(&sources, "or"))])
                        };
                        view! { <p class="find-empty">{text}</p> }
                    })}
                    {move || (recs.total.get() > 0).then(|| recordings_table(false))}
                    {move || recs.loading.get().then(|| view! { <p class="xc-loading">"Loading\u{2026}"</p> })}
                }
                .into_any()
            }
        }
    };

    view! {
        <div class="xc-modal-overlay" on:click=move |_| close()>
            <div class="xc-modal find-modal" on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()>
                <div class="xc-modal-header">
                    <span class="xc-modal-title">{TITLE}</span>
                    <button class="xc-modal-close" title="Close" on:click=move |_| close()>{"\u{00D7}"}</button>
                </div>

                <div class="find-controls">
                    <input
                        class="xc-input find-search"
                        r#type="search"
                        placeholder=if narrow_screen() { SEARCH_PLACEHOLDER_SHORT } else { SEARCH_PLACEHOLDER }
                        prop:value=move || query.get()
                        on:input=move |ev| query.set(event_target_value(&ev))
                        on:keydown=move |ev: web_sys::KeyboardEvent| {
                            if ev.key() == "Enter" {
                                if let Some(l) = detect_link(&query.get_untracked()) {
                                    load_link(l);
                                }
                            }
                        }
                    />
                    <div class="find-filters">
                        <label>
                            "Group "
                            <select class="xc-select" on:change=move |ev| {
                                let v = event_target_value(&ev);
                                group.set((!v.is_empty()).then_some(v));
                            }>{group_menu}</select>
                        </label>
                        <label>
                            "Country "
                            <select class="xc-select" on:change=move |ev| {
                                let v = event_target_value(&ev);
                                country.set((!v.is_empty()).then_some(v));
                            }>{country_menu}</select>
                        </label>
                        // Before Near, whose radius menu comes and goes beside it.
                        {move || (supports_sort.get() || min_smp.get().is_some()).then(|| view! {
                            <label>
                                {RATE_LABEL}" "
                                <select class="xc-select find-rate-select" on:change=move |ev| {
                                    min_smp.set(event_target_value(&ev).parse::<u32>().ok());
                                }>{rate_menu}</select>
                            </label>
                        })}
                        <label>
                            "Near "
                            <select class="xc-select" on:change=move |ev| {
                                let mode = match event_target_value(&ev).as_str() {
                                    "me" => NearMode::Me,
                                    "file" => NearMode::File,
                                    _ => NearMode::Anywhere,
                                };
                                set_near_mode(mode);
                            }>
                                <option value="" selected=move || near_mode.get() == NearMode::Anywhere>{NEAR_ANYWHERE}</option>
                                <option value="me" selected=move || near_mode.get() == NearMode::Me>{NEAR_ME}</option>
                                <option
                                    value="file"
                                    disabled=move || file_point.get().is_err()
                                    selected=move || near_mode.get() == NearMode::File
                                >
                                    {move || match file_point.get() {
                                        Ok(_) => NEAR_FILE,
                                        Err(why) => why,
                                    }}
                                </option>
                            </select>
                        </label>
                        {move || (near_mode.get() != NearMode::Anywhere).then(|| view! {
                            <label>
                                "within "
                                <select class="xc-select" on:change=move |ev| {
                                    if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                                        km.set(v);
                                    }
                                }>
                                    {RADII_KM.iter().map(|r| view! {
                                        <option value=r.to_string() selected=move || km.get() == *r>{format!("{r} km")}</option>
                                    }).collect::<Vec<_>>()}
                                </select>
                            </label>
                        })}
                        {move || locating.get().then(|| view! { <span class="find-locating">{LOCATING}</span> })}
                    </div>
                </div>

                {move || error.get().map(|msg| view! {
                    <div class="xc-error">
                        <span>{msg}</span>
                        <button class="xc-error-dismiss" title="Dismiss" on:click=move |_| error.set(None)>{"\u{00D7}"}</button>
                    </div>
                })}

                <div class="find-body" class:show-detail=move || show_detail.get()>
                    <div class="find-species" node_ref=species_el on:scroll=move |_| check_species_end()>
                        {species_pane}
                    </div>
                    <div class="find-detail" node_ref=detail_el on:scroll=move |_| check_recs_end()>
                        {detail_pane}
                    </div>
                </div>

                // The unavailable notice shows the same two buttons, so the footer hides then.
                {move || (availability.get() != Availability::Unavailable).then(|| view! {
                    <div class="find-footer">
                        <button class="xc-link-btn" on:click=move |_| open_xc_browser()>{XC_BROWSER}</button>
                        <button class="xc-link-btn" on:click=move |_| open_wiki_browser()>{COMMONS_BROWSER}</button>
                    </div>
                })}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_licences() {
        assert_eq!(
            licence_short("//creativecommons.org/licenses/by-nc-sa/4.0/"),
            "CC BY-NC-SA 4.0"
        );
        assert_eq!(
            licence_short("http://creativecommons.org/publicdomain/zero/1.0/legalcode"),
            "CC0 1.0"
        );
        assert_eq!(licence_short("cc-by"), "CC BY");
    }

    #[test]
    fn readable_file_names() {
        let item: RecordingItem = serde_json::from_str(
            r#"{"id":"rbins:x/0","source":"rbins","gen":"Pipistrellus","sp":"nathusii","en":"Nathusius's Pipistrelle","audio_url":"https://dasa.naturalsciences.be/a/b/2024.raw?x=1"}"#,
        )
        .unwrap();
        assert_eq!(
            file_name(&item),
            "Nathusius's Pipistrelle - Pipistrellus nathusii (RBINS).wav"
        );
        let item: RecordingItem = serde_json::from_str(
            r#"{"id":"inat:1/0","source":"inat","gen":"Myotis","sp":"","en":"","audio_url":"https://static.inaturalist.org/sounds/1.m4a?17"}"#,
        )
        .unwrap();
        assert_eq!(file_name(&item), "Myotis (iNaturalist).m4a");
    }

    #[test]
    fn commas() {
        assert_eq!(with_commas(0), "0");
        assert_eq!(with_commas(1234), "1,234");
        assert_eq!(with_commas(1234567), "1,234,567");
        assert_eq!(plural(1, "recording", "recordings"), "1 recording");
    }
}

//! Xeno-Canto browser: search XC, list a group's species, and load
//! recordings. Works in the web build and the desktop/Android app.
//!
//! All network work goes through `crate::xc_client`, which picks one of three
//! sources: the native Tauri `xc_*` commands (desktop with a saved key), a
//! browser fetch to xeno-canto.org with the user's own key (web), or the
//! Oversample proxy, which needs no key (any platform). XC's API and audio
//! downloads send CORS headers that allow a browser to call them directly.
//!
//! The dialog shows which source is active in a line under the title, with
//! links to add, change or remove the key, or to switch to the proxy.
//!
//! The `#XC<id>` URL-hash deep-link (app.rs) is separate: it searches the
//! curated demo index, not the live API.

use crate::state::store_fields::*;
use crate::state::AppState;
use crate::xc_client::{
    self, CountryCount, LocalName, NamedSpecies, NamesLanguage, ServerIndexing, SpeciesSearch,
    TaxonomyProgress, XcError, XcSource,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_ipc::xc::{XcGroupTaxonomy, XcRecording, XcSearchResult, XcSpecies};
use wasm_bindgen::prelude::*;

// ── UI text ──────────────────────────────────────────────────────────
//
// Every string added or changed for the web/proxy work is here, so the
// wording can be replaced in one place. Strings that existed before and are
// still correct stay inline in the view below.

/// Dialog title.
const TITLE: &str = "Explore Xeno-canto";

/// Source line label when requests go through the Oversample proxy.
const SOURCE_PROXY: &str = "Source: Oversample's server (no API key)";
/// Source line label when requests use the user's own key (web or desktop).
const SOURCE_OWN_KEY: &str = "Source: Xeno-canto, with your API key";
/// Opens the key form (proxy mode, no key saved; also in error messages).
const BTN_ADD_KEY: &str = "Add API key";
/// Switches from the proxy back to the saved key.
const BTN_USE_OWN_KEY: &str = "Use your API key";
/// Opens the key form to replace the saved key.
const BTN_CHANGE_KEY: &str = "Change key";
/// Forgets the saved key.
const BTN_REMOVE_KEY: &str = "Remove key";
/// Chooses the proxy, on the first screen and in the source line.
const BTN_CONTINUE_WITHOUT_KEY: &str = "Continue without key";
const BTN_CANCEL: &str = "Cancel";
const BTN_SAVE_KEY: &str = "Save key";

/// First screen: heading above the two options.
const CHOOSE_HEADING: &str = "Choose how Oversample gets recordings from Xeno-canto.";
const NO_KEY_HEADING: &str = "Without an API key";
const OWN_KEY_HEADING: &str = "With your own API key";
/// Key option text, before the xeno-canto.org link.
const KEY_INTRO_BEFORE_LINK: &str = "Oversample connects to Xeno-canto directly. An API key is \
     a code from your Xeno-canto account. To get one, create a free account at ";
/// Key option text, after the link.
const KEY_INTRO_AFTER_LINK: &str = " and open your account settings.";
const KEY_FIELD_LABEL: &str = "Xeno-canto API key";
const KEY_FIELD_PLACEHOLDER: &str = "Paste your API key";
/// Under the key field: where the key is kept.
const KEY_STORED_WEB: &str = "Your key is saved in this browser only.";
const KEY_STORED_APP: &str = "Your key is saved on this device.";

/// Desktop only: asked before deleting the key file that xc-cli also reads.
const CONFIRM_REMOVE_DESKTOP: &str = "Remove your Xeno-canto API key from this computer? The \
     xc-cli command uses the same key and will stop working too.";
const NOTICE_KEY_REMOVED: &str = "API key removed.";

const SEARCH_PLACEHOLDER: &str =
    "Species name in any language, or an XC query such as grp:bats cnt:Australia";

/// Wait after the last key press before searching names, so typing a word
/// sends one request rather than one per letter.
const NAME_SEARCH_DEBOUNCE_MS: i32 = 250;

/// Heading over the species a name search found. The server sends at most 50.
fn text_name_results(count: usize, query: &str) -> String {
    match count {
        1 => format!("1 species matches \u{201C}{query}\u{201D}"),
        50.. => format!("Best {count} matches for \u{201C}{query}\u{201D}"),
        n => format!("{n} species match \u{201C}{query}\u{201D}"),
    }
}

fn text_no_name_results(query: &str) -> String {
    format!(
        "No species match \u{201C}{query}\u{201D}. Press Search to look for recordings on \
         Xeno-canto."
    )
}

/// "Birds are still being indexed, so some may be missing."
fn text_groups_indexing(groups: &[String]) -> Option<String> {
    let names: Vec<String> = groups.iter().map(|g| group_title(g, None)).collect();
    match names.as_slice() {
        [] => None,
        [one] => Some(format!(
            "{one} are still being indexed, so some may be missing."
        )),
        [rest @ .., last] => Some(format!(
            "{} and {} are still being indexed, so some may be missing.",
            rest.join(", "),
            last.to_lowercase()
        )),
    }
}

#[wasm_bindgen(inline_js = "export function language_name(code) {
    try { return new Intl.DisplayNames(['en'], { type: 'language' }).of(code) || code; }
    catch (e) { return code; }
}")]
extern "C" {
    /// English name of a language code ("de" gives "German"); the code itself
    /// if the browser doesn't know it.
    fn language_name(code: &str) -> String;
}

#[wasm_bindgen(inline_js = "export function language_autonym(code) {
    try { return new Intl.DisplayNames([code], { type: 'language' }).of(code) || code; }
    catch (e) { return code; }
}")]
extern "C" {
    /// A language's name in that language ("de" gives "Deutsch").
    fn language_autonym(code: &str) -> String;
}

/// A language in the names menu: "German (Deutsch)", or just "German" when
/// both names are the same, the browser doesn't know the language's own name,
/// or the English name already has brackets ("Serbian (Latin)").
fn text_language_option(english: &str, own: &str) -> String {
    if own.is_empty() || own.eq_ignore_ascii_case(english) || english.contains('(') {
        english.to_string()
    } else {
        format!("{english} ({own})")
    }
}

/// The names menu's first option, naming the browser's languages:
/// "Browser languages (Dutch, German)", or "Browser language (English)".
fn text_browser_option(names: &[String]) -> String {
    match names {
        [] => "Browser language (English)".to_string(),
        [one] => format!("Browser language ({one})"),
        many => format!("Browser languages ({})", many.join(", ")),
    }
}

/// "German", "Dutch or German", "Dutch, German or French".
fn text_or_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// The species list's count line: "1,350 species, 1,102 with German names".
/// `local` is `None` when the names couldn't be loaded. `langs` are language
/// names; empty for English.
fn text_species_count(count: usize, local: Option<usize>, langs: &[String]) -> String {
    let species = format!("{} species", with_commas(count as u32));
    if langs.is_empty() {
        return species;
    }
    let langs = text_or_list(langs);
    match local {
        None => format!("{species}. Couldn't load {langs} names."),
        Some(0) => format!("{species}, none with {langs} names"),
        Some(n) => format!("{species}, {} with {langs} names", with_commas(n as u32)),
    }
}

/// The name to show for a species: its common name in the user's language from
/// `local` (keyed "Genus species"), else Xeno-canto's English name.
fn display_name(sp: &XcSpecies, local: &std::collections::HashMap<String, LocalName>) -> String {
    local
        .get(&format!("{} {}", sp.genus, sp.sp))
        .map(|l| l.name.clone())
        .unwrap_or_else(|| sp.en.clone())
}

/// The common names of a group's species in `langs`, for the species list.
/// An empty map for English; `None` if the server can't give them.
async fn local_names_for(
    group: &str,
    langs: &[String],
) -> Option<std::collections::HashMap<String, LocalName>> {
    if langs.is_empty() {
        return Some(Default::default());
    }
    xc_client::common_names(group, langs).await
}

/// The line under a found species giving the name that matched, such as
/// "Zwergfledermaus (German)", or `None` when
/// it matched the English or scientific name already shown in the row.
/// `lang` turns a language code into its name.
fn text_match_note(hit: &NamedSpecies, lang: impl Fn(&str) -> String) -> Option<String> {
    let m = &hit.matched;
    let shown = |name: &str| {
        name.eq_ignore_ascii_case(&hit.en)
            || name.eq_ignore_ascii_case(&format!("{} {}", hit.genus, hit.sp))
            || hit
                .local
                .as_deref()
                .is_some_and(|l| l.to_lowercase() == name.to_lowercase())
    };
    if m.source == "xc" || shown(&m.name) {
        return None;
    }
    let kind = match (m.kind.as_str(), m.language.as_deref()) {
        ("synonym", _) => "synonym".to_string(),
        ("scientific", _) => "scientific name".to_string(),
        (_, Some(code)) => lang(code),
        _ => "common name".to_string(),
    };
    Some(format!("{} ({kind})", m.name))
}

/// Stops a species list that is still loading.
const BTN_STOP: &str = "Stop";
/// Starts a long species-list build after the warning.
const BTN_LOAD_ALL: &str = "Load all";

/// Without your own key: what the proxy option means.
const NO_KEY_INTRO: &str = "Oversample's server sends your requests to Xeno-canto and saves the \
     results, so the newest recordings may be missing.";
/// After Stop while waiting for the server to build a species list.
const STOPPED_WAITING_FOR_SERVER: &str = "Stopped waiting for the species list. Oversample's \
     server is still building it. Click Go to check again.";

const ERR_NETWORK_PROXY: &str = "Couldn't connect to Oversample's server. Check your internet \
     connection, or add your own API key.";
const ERR_NETWORK_DIRECT: &str =
    "Couldn't connect to xeno-canto.org. Check your internet connection.";
const ERR_RATE_LIMITED: &str = "Too many requests. Wait a few seconds and try again.";
const ERR_BUSY_PROXY: &str = "Oversample's server is busy or unavailable. Try again later, or \
     add your own API key.";
const ERR_BUSY_DIRECT: &str = "Xeno-canto is busy or unavailable (HTTP 503). Try again later.";
const ERR_UPSTREAM_PROXY: &str =
    "Oversample's server couldn't get a response from Xeno-canto. Try again later.";
const ERR_UPSTREAM_DIRECT: &str = "Xeno-canto had a server error (HTTP 502). Try again later.";
/// A search or download failed because XC rejected the saved key.
const ERR_BAD_KEY: &str = "Xeno-canto rejected your API key. Copy the key again from your \
     Xeno-canto account settings, then change it here.";
/// Save key: XC rejected the key just entered, so it was not saved.
const ERR_BAD_NEW_KEY: &str = "Xeno-canto rejected this API key. Copy the key again from your \
     Xeno-canto account settings.";

/// Recording too large for the proxy. `id` is unknown only if the error
/// came from somewhere other than a download.
fn text_too_large(id: Option<u64>) -> String {
    let what = match id {
        Some(id) => format!("XC{id}"),
        None => "This recording".to_string(),
    };
    format!(
        "{what} is too large to load through Oversample's server. Add your own API key, or \
         download the file from xeno-canto.org and open it in Oversample."
    )
}

/// "Birds" or "Birds in Spain", for the start of a sentence.
fn group_title(group: &str, country: Option<&str>) -> String {
    let mut chars = group.chars();
    let mut out = match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    if let Some(c) = country {
        out.push_str(" in ");
        out.push_str(c);
    }
    out
}

/// "birds" or "birds in Spain", for the middle of a sentence.
fn group_phrase(group: &str, country: Option<&str>) -> String {
    match country {
        Some(c) => format!("{group} in {c}"),
        None => group.to_string(),
    }
}

/// 1052345 -> "1,052,345".
fn with_commas(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// "about 3 hours", "about 12 minutes", "about 1 minute".
fn text_duration(secs: f64) -> String {
    let mins = (secs / 60.0).ceil().max(1.0) as u32;
    if mins >= 90 {
        let hours = (secs / 3600.0).round().max(2.0) as u32;
        format!("about {hours} hours")
    } else if mins == 1 {
        "about 1 minute".to_string()
    } else {
        format!("about {mins} minutes")
    }
}

/// Loading line while a species list loads. `None` before the first reply.
fn text_progress(group: &str, country: Option<&str>, p: Option<TaxonomyProgress>) -> String {
    let what = group_phrase(group, country);
    match p {
        None => format!("Loading species list for {what}..."),
        Some(TaxonomyProgress::Server(s)) => text_server_progress(group, s),
        Some(TaxonomyProgress::Local(p)) if p.secs_left >= 30.0 => format!(
            "Loading species list for {what}: {} of {} recordings, {} left",
            with_commas(p.recordings_done),
            with_commas(p.total_recordings),
            text_duration(p.secs_left)
        ),
        Some(TaxonomyProgress::Local(p)) => format!(
            "Loading species list for {what}: {} of {} recordings",
            with_commas(p.recordings_done),
            with_commas(p.total_recordings)
        ),
    }
}

/// While the server collects every recording in `group` from Xeno-canto
/// (Proxy mode only). The server collects the whole group, so the country
/// is left out. Percent rather than pages, because the user never sees the
/// server's pages. `pages_total` is 0 while the group waits in the server's
/// queue behind other groups.
fn text_server_progress(group: &str, s: ServerIndexing) -> String {
    if s.pages_total == 0 {
        return format!(
            "Waiting for Oversample's server to start collecting recordings of {group} for the species list..."
        );
    }
    let pct = (s.pages_done.min(s.pages_total) as u64 * 100 / s.pages_total as u64) as u32;
    format!(
        "Oversample's server is collecting recordings of {group} for the species list: {pct}% done"
    )
}

/// After Stop. `p` is the last progress seen, if any reply had arrived.
fn text_stopped(country: Option<&str>, p: Option<TaxonomyProgress>) -> String {
    match p {
        Some(TaxonomyProgress::Server(_)) => STOPPED_WAITING_FOR_SERVER.to_string(),
        Some(TaxonomyProgress::Local(p)) => {
            let what = format!(
                "Species list stopped after {} of {} recordings.",
                with_commas(p.recordings_done),
                with_commas(p.total_recordings)
            );
            if country.is_some() {
                format!("{what} Click Go to start again.")
            } else {
                format!("{what} Choose a country for a shorter list, or click Go to start again.")
            }
        }
        None => "Species list stopped. Click Go to start again.".to_string(),
    }
}

/// Own key: a species list that would take a long time, before it starts.
fn text_long_build(group: &str, country: Option<&str>, recordings: u32, est_secs: f64) -> String {
    let base = format!(
        "{} has {} recordings. Loading the species list takes {}.",
        group_title(group, country),
        with_commas(recordings),
        text_duration(est_secs)
    );
    if country.is_some() {
        base
    } else {
        format!("{base} Choose a country for a shorter list.")
    }
}

/// Country picker: the number of recordings beside a country, matching the
/// "Recs" column of the species list.
fn text_country_recs(recordings: u32) -> String {
    if recordings == 1 {
        "1 rec".to_string()
    } else {
        format!("{} recs", with_commas(recordings))
    }
}

/// Country picker: hover text for a country with its counts. The name is
/// included because a long one is cut off in the row.
fn text_country_counts_title(country: &str, recordings: u32, species: u32) -> String {
    format!(
        "{country}: {} recording{}, {} species",
        with_commas(recordings),
        if recordings == 1 { "" } else { "s" },
        with_commas(species)
    )
}

/// Any other HTTP error when the server gave no message.
fn text_http_failed(status: u16) -> String {
    format!("Request failed (HTTP {status})")
}

/// Age of a species list built in the browser. Same wording as the desktop
/// cache.
fn text_age(secs: f64) -> String {
    xc_lib::cache::age_text((secs / 3600.0).floor() as i64)
}

/// Buttons shown inside an error message.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ErrorActions {
    None,
    /// [Add API key]
    AddKey,
    /// [Change key] [Remove key]
    FixKey,
}

/// Text for an error, worded for the source that produced it. Server
/// details for Busy, Upstream and BadKey go to the log, not the message.
fn error_text(e: &XcError, src: &XcSource) -> String {
    let is_proxy = *src == XcSource::Proxy;
    match e {
        XcError::Network if is_proxy => ERR_NETWORK_PROXY.to_string(),
        XcError::Network => ERR_NETWORK_DIRECT.to_string(),
        XcError::RateLimited => ERR_RATE_LIMITED.to_string(),
        XcError::Busy(_) if is_proxy => ERR_BUSY_PROXY.to_string(),
        XcError::Busy(_) => ERR_BUSY_DIRECT.to_string(),
        XcError::Upstream(_) if is_proxy => ERR_UPSTREAM_PROXY.to_string(),
        XcError::Upstream(_) => ERR_UPSTREAM_DIRECT.to_string(),
        XcError::BadKey(_) => ERR_BAD_KEY.to_string(),
        XcError::TooLarge { id } => text_too_large(*id),
        XcError::LongBuild {
            group,
            country,
            recordings,
            est_secs,
        } => text_long_build(group, country.as_deref(), *recordings, *est_secs),
        XcError::Http {
            message: Some(m), ..
        } => m.clone(),
        XcError::Http {
            status,
            message: None,
        } => text_http_failed(*status),
        XcError::Cancelled => String::new(),
        XcError::Other(m) => m.clone(),
    }
}

/// Which buttons an error gets.
fn error_actions(e: &XcError, src: &XcSource) -> ErrorActions {
    let is_proxy = *src == XcSource::Proxy;
    match e {
        XcError::TooLarge { .. } | XcError::Busy(_) if is_proxy => ErrorActions::AddKey,
        XcError::BadKey(_) => ErrorActions::FixKey,
        _ => ErrorActions::None,
    }
}

/// Log the server's own message for errors whose text leaves it out.
fn log_detail(e: &XcError) {
    if let XcError::Busy(Some(d)) | XcError::Upstream(Some(d)) | XcError::BadKey(Some(d)) = e {
        log::warn!("XC request failed: {d}");
    }
}

// ── Static data ──────────────────────────────────────────────────────

const XC_GROUPS: &[&str] = &["bats", "birds", "frogs", "grasshoppers", "land mammals"];

const XC_COUNTRIES_RAW: &str = include_str!("../data/countries.txt");

fn xc_countries() -> Vec<&'static str> {
    XC_COUNTRIES_RAW.lines().filter(|l| !l.is_empty()).collect()
}

/// One row of the country picker. `counts` is (recordings, species) when the
/// server's country list for the group is loaded.
#[derive(Clone, Debug, PartialEq)]
struct CountryOption {
    name: String,
    counts: Option<(u32, u32)>,
}

/// The rows of the country picker that match `filter` (case-insensitive
/// substring). With the server's list for the group, only countries with
/// recordings are listed, with their counts. Without it, the built-in list.
fn country_options(
    server: Option<&[CountryCount]>,
    builtin: &[&str],
    filter: &str,
) -> Vec<CountryOption> {
    let filter = filter.trim().to_lowercase();
    let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
    match server {
        Some(list) if !list.is_empty() => list
            .iter()
            .filter(|c| matches(&c.country))
            .map(|c| CountryOption {
                name: c.country.clone(),
                counts: Some((c.recordings, c.species)),
            })
            .collect(),
        _ => builtin
            .iter()
            .filter(|name| matches(name))
            .map(|name| CountryOption {
                name: name.to_string(),
                counts: None,
            })
            .collect(),
    }
}

fn format_sample_rate(smp: &str) -> String {
    match smp.parse::<u64>() {
        Ok(hz) if hz >= 1000 => format!("{}kHz", hz / 1000),
        Ok(hz) => format!("{hz}Hz"),
        Err(_) => smp.to_string(),
    }
}

// ── View states ──────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum BrowserView {
    /// Working out which source to use (desktop asks the native side).
    Starting,
    /// Key form plus the no-key option.
    KeySetup,
    GroupBrowse,
    SpeciesRecordings {
        group: String,
        country: Option<String>,
        genus: String,
        species: String,
        en: String,
    },
    SearchResults,
}

// ── Component ────────────────────────────────────────────────────────

#[component]
pub fn XcBrowser() -> impl IntoView {
    let state = expect_context::<AppState>();
    let is_tauri = state.is_tauri;

    let view = RwSignal::new(BrowserView::Starting);
    let api_key_input = RwSignal::new(String::new());
    // The source in use, and the source the saved key would give (if any).
    let source: RwSignal<Option<XcSource>> = RwSignal::new(None);
    let own_key: RwSignal<Option<XcSource>> = RwSignal::new(None);
    let selected_group = RwSignal::new("bats".to_string());
    let country_input = RwSignal::new(String::new());
    let species_list: RwSignal<Vec<XcSpecies>> = RwSignal::new(Vec::new());
    // Common names in the user's language for `species_list`, keyed "Genus
    // species". Empty for English; `None` when the server can't give them.
    let species_local_names: RwSignal<Option<std::collections::HashMap<String, LocalName>>> =
        RwSignal::new(Some(Default::default()));
    // The language species names are shown in (the names menu).
    let names_language = RwSignal::new(xc_client::names_language());
    // Languages the server has names in, for the names menu. Empty until
    // loaded, or if the server can't say.
    let language_choices: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    // The languages `species_local_names` was asked for, for the count line.
    let species_local_langs: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    // Counts reloads of the species list's names after a language change.
    let local_names_gen = RwSignal::new(0u64);
    // The group of the species list on screen. Set together with
    // `species_list`; `taxonomy_target` names the group being loaded, which
    // can differ during a Refresh.
    let species_list_group = RwSignal::new(String::new());
    // Country the species list was built for, so a species' recordings are
    // limited to the same country.
    let species_list_country: RwSignal<Option<String>> = RwSignal::new(None);
    let recordings: RwSignal<Vec<XcRecording>> = RwSignal::new(Vec::new());
    // A page of recordings (species view or search results) is loading.
    let loading = RwSignal::new(false);
    // The error message and the buttons shown with it.
    let error_msg: RwSignal<Option<(String, ErrorActions)>> = RwSignal::new(None);
    // A status message that is not an error ("API key removed.").
    let notice: RwSignal<Option<String>> = RwSignal::new(None);
    // Own key: a long species list is waiting for [Load all] or [Cancel].
    let long_build_prompt: RwSignal<Option<String>> = RwSignal::new(None);
    // Desktop: asking before the key file is deleted.
    let confirm_remove = RwSignal::new(false);
    let taxonomy_age: RwSignal<Option<String>> = RwSignal::new(None);
    // A species list is being built or loaded. Separate from `loading`, so a
    // search during a build leaves the progress line and Stop in place.
    let taxonomy_loading = RwSignal::new(false);
    // The group and country of the build in progress, for its progress line.
    let taxonomy_target: RwSignal<(String, Option<String>)> = RwSignal::new((String::new(), None));
    // Progress of a species list: pages built in the browser, or the
    // server's progress while it indexes the group.
    let taxonomy_progress: RwSignal<Option<TaxonomyProgress>> = RwSignal::new(None);
    // Bumped to cancel a running species-list build.
    let taxonomy_gen = RwSignal::new(0u64);
    // Bumped for each recordings request, so an older response is dropped.
    let results_gen = RwSignal::new(0u64);
    let search_input = RwSignal::new(String::new());
    // Species found by name for the text in the search box: (query, result).
    // `None` while the box is empty, too short, or holds an XC query, and when
    // the name search failed (a server without `/xc/species`, or offline):
    // then the dialog works as it did before name search.
    let name_results: RwSignal<Option<(String, SpeciesSearch)>> = RwSignal::new(None);
    // Bumped on each key press in the search box, so only the last one searches.
    let name_search_gen = RwSignal::new(0u64);
    let recordings_page = RwSignal::new(1u32);
    let recordings_total_pages = RwSignal::new(1u32);
    let downloading: RwSignal<Option<u64>> = RwSignal::new(None);
    let recordings_total: RwSignal<u32> = RwSignal::new(0);
    let cached_ids: RwSignal<std::collections::HashSet<u64>> =
        RwSignal::new(std::collections::HashSet::new());
    let is_desktop = is_tauri && !state.is_mobile_platform;

    // Country combobox state
    let country_dropdown_open = RwSignal::new(false);
    let country_filter_text = RwSignal::new(String::new());
    let country_highlight_idx = RwSignal::new(0usize);
    let countries = xc_countries();
    // The server's countries for each group, once fetched. A group whose
    // fetch failed has no entry, and the picker shows the built-in list.
    let country_counts: RwSignal<std::collections::HashMap<String, Vec<CountryCount>>> =
        RwSignal::new(std::collections::HashMap::new());
    // Bumped for each country-list request, so a slow reply for a group the
    // user has already left is dropped.
    let countries_gen = RwSignal::new(0u64);
    let filtered_countries = Memo::new(move |_| {
        let filter = country_filter_text.get();
        let group = selected_group.get();
        country_counts
            .with(|m| country_options(m.get(&group).map(Vec::as_slice), &countries, &filter))
    });

    // Fetch the server's country list for the selected group, once per
    // group. Failures are ignored.
    Effect::new(move |_| {
        let group = selected_group.get();
        if source.get().is_none() || country_counts.with_untracked(|m| m.contains_key(&group)) {
            return;
        }
        let my_gen = countries_gen.get_untracked() + 1;
        countries_gen.set(my_gen);
        spawn_local(async move {
            let list = xc_client::group_countries(&group).await;
            if countries_gen.try_get_untracked() != Some(my_gen) {
                return;
            }
            if let Some(list) = list {
                country_counts.try_update(|m| {
                    m.insert(group, list);
                });
            }
        });
    });

    // Pick the source: the proxy if chosen before, else a saved key, else ask.
    spawn_local(async move {
        let own = xc_client::own_key_source(is_tauri).await;
        let src = xc_client::resolve_source(is_tauri).await;
        own_key.try_set(own);
        let next = if src.is_some() {
            BrowserView::GroupBrowse
        } else {
            BrowserView::KeySetup
        };
        source.try_set(src);
        view.try_set(next);
    });

    let clear_messages = move || {
        error_msg.set(None);
        notice.set(None);
        long_build_prompt.set(None);
        confirm_remove.set(false);
    };

    // Show an XC error in the error box, worded for `src`.
    let show_error = move |e: &XcError, src: &XcSource| {
        log_detail(e);
        error_msg.try_set(Some((error_text(e, src), error_actions(e, src))));
    };

    let on_close = move |_: web_sys::MouseEvent| {
        state.dialogs.xc_browser_open().set(false);
    };

    // Prevent click on modal content from closing it
    let on_content_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
    };

    let on_save_key = move |_: web_sys::MouseEvent| {
        let key = api_key_input.get_untracked().trim().to_string();
        if key.is_empty() {
            return;
        }
        clear_messages();
        spawn_local(async move {
            match xc_client::save_key(is_tauri, &key).await {
                Ok(src) => {
                    own_key.try_set(Some(src.clone()));
                    source.try_set(Some(src));
                    api_key_input.try_set(String::new());
                    view.try_set(BrowserView::GroupBrowse);
                    error_msg.try_set(None);
                }
                Err(XcError::BadKey(d)) => {
                    log_detail(&XcError::BadKey(d));
                    error_msg.try_set(Some((ERR_BAD_NEW_KEY.to_string(), ErrorActions::None)));
                }
                Err(e) => {
                    let text = format!("Failed to save key: {}", error_text(&e, &XcSource::Native));
                    error_msg.try_set(Some((text, ErrorActions::None)));
                }
            }
        });
    };

    let do_remove_key = move || {
        confirm_remove.set(false);
        spawn_local(async move {
            match xc_client::remove_key(is_tauri).await {
                Ok(()) => {
                    own_key.try_set(None);
                    // Using the key until now: there is no source left, so
                    // go back to the first screen.
                    if source.try_get_untracked().flatten() != Some(XcSource::Proxy) {
                        source.try_set(None);
                        view.try_set(BrowserView::KeySetup);
                    }
                    error_msg.try_set(None);
                    notice.try_set(Some(NOTICE_KEY_REMOVED.to_string()));
                }
                Err(e) => {
                    show_error(&e, &XcSource::Native);
                }
            }
        });
    };

    // Desktop asks first, because the key file is shared with xc-cli.
    let on_remove_key = move |_: web_sys::MouseEvent| {
        clear_messages();
        if is_desktop {
            confirm_remove.set(true);
        } else {
            do_remove_key();
        }
    };

    let on_use_proxy = move |_: web_sys::MouseEvent| {
        xc_client::set_use_proxy(true);
        source.set(Some(XcSource::Proxy));
        clear_messages();
        if view.get_untracked() == BrowserView::KeySetup {
            view.set(BrowserView::GroupBrowse);
        }
    };

    let on_use_own_key = move |_: web_sys::MouseEvent| {
        if let Some(src) = own_key.get_untracked() {
            xc_client::set_use_proxy(false);
            source.set(Some(src));
            clear_messages();
        }
    };

    let on_open_key_form = move |_: web_sys::MouseEvent| {
        clear_messages();
        view.set(BrowserView::KeySetup);
    };

    let on_cancel_key_form = move |_: web_sys::MouseEvent| {
        error_msg.set(None);
        view.set(BrowserView::GroupBrowse);
    };

    // Build or reload the species list for the selected group and country.
    // `allow_long` skips the warning for a long build (own key, web).
    let load_group = move |refresh: bool, allow_long: bool| {
        let Some(src) = source.get_untracked() else {
            return;
        };
        let group = selected_group.get_untracked();
        let country = {
            let c = country_input.get_untracked().trim().to_string();
            if c.is_empty() {
                None
            } else {
                Some(c)
            }
        };
        let my_gen = taxonomy_gen.get_untracked() + 1;
        taxonomy_gen.set(my_gen);
        taxonomy_loading.set(true);
        clear_messages();
        taxonomy_age.set(None);
        taxonomy_progress.set(None);
        taxonomy_target.set((group.clone(), country.clone()));
        if !refresh {
            species_list.set(Vec::new());
        }

        let is_current = move || taxonomy_gen.try_get_untracked() == Some(my_gen);
        spawn_local(async move {
            let list_group = group.clone();
            let list_country = country.clone();
            let result = xc_client::browse_group(
                &src,
                &group,
                country.as_deref(),
                refresh,
                allow_long,
                move |p| {
                    if is_current() {
                        taxonomy_progress.try_set(Some(p));
                    }
                },
                is_current,
            )
            .await;
            if !is_current() {
                return;
            }
            let taxonomy: Option<XcGroupTaxonomy> = match result {
                Ok(t) => Some(t),
                Err(XcError::Cancelled) => return,
                Err(e @ XcError::LongBuild { .. }) => {
                    long_build_prompt.try_set(Some(error_text(&e, &src)));
                    None
                }
                Err(e) => {
                    show_error(&e, &src);
                    None
                }
            };
            if let Some(mut t) = taxonomy {
                // Names in the user's language, fetched before the list is shown
                // so it is not sorted again under the reader.
                let mut langs = names_language.get_untracked().codes();
                let mut local = local_names_for(&list_group, &langs).await;
                // The names menu may have changed while the names loaded.
                while is_current() && names_language.get_untracked().codes() != langs {
                    langs = names_language.get_untracked().codes();
                    local = local_names_for(&list_group, &langs).await;
                }
                if !is_current() {
                    return;
                }
                if let Some(local) = local.as_ref().filter(|l| !l.is_empty()) {
                    t.species
                        .sort_by_cached_key(|s| display_name(s, local).to_lowercase());
                }
                taxonomy_age.try_set(xc_client::taxonomy_age_secs(&t).map(text_age));
                species_list_group.try_set(list_group);
                species_list_country.try_set(list_country);
                species_local_names.try_set(local);
                species_local_langs.try_set(langs);
                species_list.try_set(t.species);
            }
            taxonomy_progress.try_set(None);
            taxonomy_loading.try_set(false);
        });
    };

    let on_stop_taxonomy = move |_: web_sys::MouseEvent| {
        taxonomy_gen.update(|g| *g += 1);
        let last = taxonomy_progress.get_untracked();
        let (_, country) = taxonomy_target.get_untracked();
        taxonomy_progress.set(None);
        taxonomy_loading.set(false);
        notice.set(Some(text_stopped(country.as_deref(), last)));
    };

    let on_load_all = move |_: web_sys::MouseEvent| {
        load_group(false, true);
    };

    let on_cancel_long_build = move |_: web_sys::MouseEvent| {
        long_build_prompt.set(None);
    };

    let on_load_group = move |_: web_sys::MouseEvent| {
        // Commit any pending filter text
        let text = country_filter_text.get_untracked().trim().to_string();
        country_input.set(text);
        country_dropdown_open.set(false);
        load_group(false, false);
    };

    let on_refresh = move |_: web_sys::MouseEvent| {
        load_group(true, false);
    };

    let check_cached = move |src: XcSource, ids: Vec<u64>| {
        spawn_local(async move {
            let set = xc_client::cached_ids(&src, ids).await;
            cached_ids.try_set(set);
        });
    };

    // Start a recordings request: returns its generation number. Any older
    // request still in flight is ignored when it finishes.
    let begin_results = move || {
        let my_gen = results_gen.get_untracked() + 1;
        results_gen.set(my_gen);
        loading.set(true);
        recordings.set(Vec::new());
        clear_messages();
        my_gen
    };

    // Show one page of recordings (species view or search results), unless
    // a newer request has started since this one.
    let show_results =
        move |my_gen: u64, src: XcSource, result: Result<XcSearchResult, XcError>| {
            if results_gen.try_get_untracked() != Some(my_gen) {
                return;
            }
            match result {
                Ok(r) => {
                    let ids: Vec<u64> = r.recordings.iter().map(|rec| rec.id_num()).collect();
                    recordings_page.try_set(r.page);
                    recordings_total_pages.try_set(r.num_pages);
                    recordings_total.try_set(r.num_recordings);
                    recordings.try_set(r.recordings);
                    check_cached(src, ids);
                }
                Err(e) => show_error(&e, &src),
            }
            loading.try_set(false);
        };

    // Show a species' recordings, limited to `country` if given.
    let load_species_recordings = move |group: String,
                                        country: Option<String>,
                                        genus: String,
                                        species: String,
                                        en: String| {
        let Some(src) = source.get_untracked() else {
            return;
        };
        view.set(BrowserView::SpeciesRecordings {
            group: group.clone(),
            country: country.clone(),
            genus: genus.clone(),
            species: species.clone(),
            en,
        });
        let my_gen = begin_results();
        recordings_page.set(1);

        spawn_local(async move {
            let result = xc_client::species_recordings(
                &src,
                &group,
                &genus,
                &species,
                country.as_deref(),
                1,
            )
            .await;
            show_results(my_gen, src, result);
        });
    };

    let on_search = move |_: web_sys::MouseEvent| {
        let query = search_input.get_untracked().trim().to_string();
        if query.is_empty() {
            return;
        }
        let Some(src) = source.get_untracked() else {
            return;
        };
        view.set(BrowserView::SearchResults);
        let my_gen = begin_results();

        spawn_local(async move {
            let result = xc_client::search(&src, &query, 1).await;
            show_results(my_gen, src, result);
        });
    };

    let on_search_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "Enter" => on_search(ev.unchecked_into()),
        "Escape" if !search_input.get_untracked().is_empty() => {
            // Clear the box rather than close the dialog.
            ev.stop_propagation();
            search_input.set(String::new());
            name_search_gen.update(|g| *g += 1);
            name_results.set(None);
        }
        _ => {}
    };

    // Search species names as the user types. Plain text only: an XC query
    // (with ":") waits for Search, as before.
    let on_search_input = move |text: String| {
        search_input.set(text.clone());
        let my_gen = name_search_gen.get_untracked() + 1;
        name_search_gen.set(my_gen);
        let query = text.trim().to_string();
        if query.chars().count() < 2 || xc_client::is_xc_query(&query) {
            name_results.set(None);
            return;
        }
        spawn_local(async move {
            crate::web_util::sleep_ms(NAME_SEARCH_DEBOUNCE_MS).await;
            if name_search_gen.try_get_untracked() != Some(my_gen) {
                return;
            }
            let result = xc_client::find_species(&query).await;
            if name_search_gen.try_get_untracked() != Some(my_gen) {
                return;
            }
            match result {
                Ok(found) => {
                    // Typing a new name in a recordings view goes back to the
                    // species list, where the results are shown.
                    if matches!(
                        view.get_untracked(),
                        BrowserView::SpeciesRecordings { .. } | BrowserView::SearchResults
                    ) {
                        results_gen.update(|g| *g += 1);
                        loading.set(false);
                        recordings.set(Vec::new());
                        view.set(BrowserView::GroupBrowse);
                    }
                    name_results.try_set(Some((query, found)));
                }
                Err(e) => {
                    log::warn!("Species name search failed: {e:?}");
                    name_results.try_set(None);
                }
            }
        });
    };

    // The languages the server has names in, for the names menu.
    spawn_local(async move {
        if let Some(codes) = xc_client::name_language_list().await {
            language_choices.try_set(codes);
        }
    });

    // A new choice in the names menu: saved, then the names on screen are
    // loaded again in the new language.
    let on_names_language = move |ev: web_sys::Event| {
        let choice = NamesLanguage::from_stored(Some(&event_target_value(&ev)));
        xc_client::set_names_language(&choice);
        names_language.set(choice.clone());
        if name_results.with_untracked(Option::is_some) {
            on_search_input(search_input.get_untracked());
        }
        let list_group = species_list_group.get_untracked();
        if list_group.is_empty() || species_list.with_untracked(Vec::is_empty) {
            return;
        }
        let my_gen = local_names_gen.get_untracked() + 1;
        local_names_gen.set(my_gen);
        let list_gen = taxonomy_gen.get_untracked();
        spawn_local(async move {
            let langs = choice.codes();
            let local = local_names_for(&list_group, &langs).await;
            if local_names_gen.try_get_untracked() != Some(my_gen)
                || taxonomy_gen.try_get_untracked() != Some(list_gen)
            {
                return;
            }
            let mut list = species_list.get_untracked();
            // Sorted by the names shown, as when the list was loaded; by
            // English name (Xeno-canto's order) when no other names are shown.
            match local.as_ref().filter(|l| !l.is_empty()) {
                Some(l) => list.sort_by_cached_key(|s| display_name(s, l).to_lowercase()),
                None => list.sort_by_cached_key(|s| s.en.to_lowercase()),
            }
            species_local_names.set(local);
            species_local_langs.set(langs);
            species_list.set(list);
        });
    };

    // The count line of the species list on screen; `None` when no list with
    // species is shown.
    let count_line = Memo::new(move |_| {
        if view.get() != BrowserView::GroupBrowse {
            return None;
        }
        if let Some((query, found)) = name_results.get() {
            return (!found.species.is_empty())
                .then(|| text_name_results(found.species.len(), &query));
        }
        let count = species_list.with(Vec::len);
        if count == 0 && !taxonomy_loading.get() {
            return None;
        }
        let local_count = species_local_names.with(|m| {
            let m = m.as_ref()?;
            Some(species_list.with(|list| {
                list.iter()
                    .filter(|s| m.contains_key(&format!("{} {}", s.genus, s.sp)))
                    .count()
            }))
        });
        let langs: Vec<String> =
            species_local_langs.with(|l| l.iter().map(|c| language_name(c)).collect());
        Some(text_species_count(count, local_count, &langs))
    });

    // The names menu, on the count line of both species lists.
    let names_menu = move || {
        let browser: Vec<String> = xc_client::browser_languages()
            .iter()
            .map(|c| language_name(c))
            .collect();
        let is = move |v: &'static str| move || names_language.get().stored() == v;
        view! {
            <label class="xc-names-menu">
                "Names: "
                <select class="xc-select" on:change=on_names_language>
                    <option value="browser" prop:selected=is("browser")>{text_browser_option(&browser)}</option>
                    <option value="en" prop:selected=is("en")>"English"</option>
                    {move || {
                        let mut codes = language_choices.get();
                        if let NamesLanguage::One(c) = names_language.get() {
                            if !codes.contains(&c) {
                                codes.push(c);
                            }
                        }
                        // Languages the browser can't name are left out (it
                        // gives back a code, such as "arz", or "chm" for
                        // "mhr"), and so is a
                        // second code with the same name ("mhr" and "chm").
                        // Codes come most names first, so the code kept has
                        // more names.
                        let mut options: Vec<(String, String)> = Vec::new();
                        for c in codes {
                            let english = language_name(&c);
                            let unnamed = english.len() <= 3 && english.chars().all(|ch| ch.is_ascii_lowercase());
                            if unnamed && names_language.get().stored() != c {
                                continue;
                            }
                            let label = text_language_option(&english, &language_autonym(&c));
                            if options.iter().all(|(l, _)| *l != label) {
                                options.push((label, c));
                            }
                        }
                        options.sort_by_cached_key(|(label, _)| label.to_lowercase());
                        (!options.is_empty()).then(|| view! {
                            <optgroup label="Other languages">
                                {options.into_iter().map(|(label, code)| {
                                    let selected = {
                                        let code = code.clone();
                                        move || names_language.get().stored() == code
                                    };
                                    view! { <option value=code prop:selected=selected>{label}</option> }
                                }).collect::<Vec<_>>()}
                            </optgroup>
                        })
                    }}
                </select>
            </label>
        }
    };

    let on_country_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "Enter" => {
            if country_dropdown_open.get_untracked() {
                let list = filtered_countries.get_untracked();
                let idx = country_highlight_idx.get_untracked();
                if idx == 0 {
                    country_input.set(String::new());
                    country_filter_text.set(String::new());
                } else if let Some(opt) = list.get(idx - 1) {
                    country_input.set(opt.name.clone());
                    country_filter_text.set(opt.name.clone());
                }
                country_dropdown_open.set(false);
            } else {
                let text = country_filter_text.get_untracked().trim().to_string();
                country_input.set(text);
            }
            load_group(false, false);
        }
        "Escape" => {
            country_dropdown_open.set(false);
            country_filter_text.set(country_input.get_untracked());
        }
        "ArrowDown" => {
            ev.prevent_default();
            if !country_dropdown_open.get_untracked() {
                country_dropdown_open.set(true);
            }
            let max = filtered_countries.get_untracked().len();
            country_highlight_idx.update(|i| *i = (*i + 1).min(max));
        }
        "ArrowUp" => {
            ev.prevent_default();
            country_highlight_idx.update(|i| *i = i.saturating_sub(1));
        }
        _ => {}
    };

    let on_back = move |_: web_sys::MouseEvent| {
        // Drop any recordings request still in flight.
        results_gen.update(|g| *g += 1);
        loading.set(false);
        view.set(BrowserView::GroupBrowse);
        recordings.set(Vec::new());
    };

    let load_recordings_page = move |page_num: u32| {
        let current_view = view.get_untracked();
        let Some(src) = source.get_untracked() else {
            return;
        };
        let my_gen = begin_results();

        spawn_local(async move {
            let result = match &current_view {
                BrowserView::SpeciesRecordings {
                    group,
                    country,
                    genus,
                    species,
                    ..
                } => {
                    xc_client::species_recordings(
                        &src,
                        group,
                        genus,
                        species,
                        country.as_deref(),
                        page_num,
                    )
                    .await
                }
                BrowserView::SearchResults => {
                    let query = search_input.get_untracked();
                    xc_client::search(&src, &query, page_num).await
                }
                _ => {
                    loading.try_set(false);
                    return;
                }
            };
            show_results(my_gen, src, result);
        });
    };

    let download_and_load = move |rec: XcRecording| {
        let Some(src) = source.get_untracked() else {
            return;
        };
        let id = rec.id_num();
        downloading.set(Some(id));
        clear_messages();
        spawn_local(async move {
            let dl = match xc_client::download(&src, &rec).await {
                Ok(dl) => dl,
                Err(e) => {
                    log::error!("Failed to download XC{id}: {e:?}");
                    show_error(&e, &src);
                    downloading.try_set(None);
                    return;
                }
            };
            let result: Result<(), String> = async {
                // Use the standard loading pipeline (WASM-side decode, spectrogram, etc.)
                let load_id = state.loading_start(&dl.filename);
                let load_result = crate::components::file_sidebar::load_named_bytes(
                    dl.filename.clone(),
                    &dl.bytes,
                    Some(dl.metadata),
                    dl.hashes,
                    None,
                    None,
                    state,
                    load_id,
                    false,
                )
                .await;
                state.loading_done(load_id);
                load_result?;

                // Switch to the newly loaded file
                let file_count = state.library.files().with_untracked(|files| files.len());
                if file_count > 0 {
                    state.library.current_index().set(Some(file_count - 1));
                    if let (Some(set), Some(file_id)) =
                        (rec.annotation_set.clone(), state.file_id_at(file_count - 1))
                    {
                        crate::annota::attach_xc_set(state, file_id, set);
                    }
                }

                cached_ids.try_update(|s| {
                    s.insert(id);
                });
                state.dialogs.xc_browser_open().set(false);
                Ok(())
            }
            .await;

            if let Err(e) = result {
                log::error!("Failed to load XC{id}: {e}");
                error_msg.try_set(Some((format!("Failed to load: {e}"), ErrorActions::None)));
            }
            downloading.try_set(None);
        });
    };

    view! {
        <div class="xc-modal-overlay" on:click=on_close>
            <div class="xc-modal" on:click=on_content_click>
                <div class="xc-modal-header">
                    <span class="xc-modal-title">{TITLE}</span>
                    <button class="xc-modal-close" on:click=on_close>{"\u{00D7}"}</button>
                </div>

                // Which source is active, with the controls that change it.
                {move || {
                    let src = source.get()?;
                    if view.get() == BrowserView::KeySetup { return None; }
                    let is_proxy = src == XcSource::Proxy;
                    let has_own = own_key.get().is_some();
                    Some(view! {
                        <div class="xc-source-line">
                            <span class="xc-source-text">
                                {if is_proxy { SOURCE_PROXY } else { SOURCE_OWN_KEY }}
                            </span>
                            {if is_proxy {
                                if has_own {
                                    view! {
                                        <button class="xc-link-btn" on:click=on_use_own_key>{BTN_USE_OWN_KEY}</button>
                                        <button class="xc-link-btn" on:click=on_remove_key>{BTN_REMOVE_KEY}</button>
                                    }.into_any()
                                } else {
                                    view! {
                                        <button class="xc-link-btn" on:click=on_open_key_form>{BTN_ADD_KEY}</button>
                                    }.into_any()
                                }
                            } else {
                                view! {
                                    <button class="xc-link-btn" on:click=on_open_key_form>{BTN_CHANGE_KEY}</button>
                                    <button class="xc-link-btn" on:click=on_remove_key>{BTN_REMOVE_KEY}</button>
                                    <button class="xc-link-btn" on:click=on_use_proxy>{BTN_CONTINUE_WITHOUT_KEY}</button>
                                }.into_any()
                            }}
                        </div>
                    })
                }}

                // Desktop: confirm before deleting the key file shared with xc-cli.
                {move || confirm_remove.get().then(|| view! {
                    <div class="xc-confirm">
                        <span>{CONFIRM_REMOVE_DESKTOP}</span>
                        <div class="xc-confirm-buttons">
                            <button class="xc-btn" on:click=move |_| do_remove_key()>{BTN_REMOVE_KEY}</button>
                            <button class="xc-btn" on:click=move |_| confirm_remove.set(false)>{BTN_CANCEL}</button>
                        </div>
                    </div>
                })}

                // Error display, with buttons that fix the problem.
                {move || error_msg.get().filter(|(m, _)| !m.is_empty()).map(|(msg, actions)| view! {
                    <div class="xc-error">
                        <span>
                            {msg}
                            {match actions {
                                ErrorActions::None => None,
                                ErrorActions::AddKey => Some(view! {
                                    " "
                                    <button class="xc-link-btn" on:click=on_open_key_form>{BTN_ADD_KEY}</button>
                                }.into_any()),
                                ErrorActions::FixKey => Some(view! {
                                    " "
                                    <button class="xc-link-btn" on:click=on_open_key_form>{BTN_CHANGE_KEY}</button>
                                    <button class="xc-link-btn" on:click=on_remove_key>{BTN_REMOVE_KEY}</button>
                                }.into_any()),
                            }}
                        </span>
                        <button class="xc-error-dismiss" on:click=move |_| error_msg.set(None)>{"\u{00D7}"}</button>
                    </div>
                })}

                // Status messages that are not errors.
                {move || notice.get().map(|msg| view! {
                    <div class="xc-notice">
                        <span>{msg}</span>
                        <button class="xc-error-dismiss" on:click=move |_| notice.set(None)>{"\u{00D7}"}</button>
                    </div>
                })}

                // Own key: a long species list waits for the user to confirm.
                {move || long_build_prompt.get().map(|msg| view! {
                    <div class="xc-confirm">
                        <span>{msg}</span>
                        <div class="xc-confirm-buttons">
                            <button class="xc-btn" on:click=on_load_all>{BTN_LOAD_ALL}</button>
                            <button class="xc-btn" on:click=on_cancel_long_build>{BTN_CANCEL}</button>
                        </div>
                    </div>
                })}

                // Download progress indicator
                {move || downloading.get().map(|id| view! {
                    <div class="xc-downloading">{format!("Downloading XC{id}\u{2026}")}</div>
                })}

                // First screen: choose the proxy or your own key. From proxy
                // mode ("Add API key") only the key option is shown.
                {move || {
                    if view.get() != BrowserView::KeySetup { return None; }
                    let has_source = source.get().is_some();
                    let on_proxy = source.get() == Some(XcSource::Proxy);
                    let key_option = view! {
                        <div class="xc-option">
                            <h3 class="xc-option-heading">{OWN_KEY_HEADING}</h3>
                            <p class="xc-info">
                                {KEY_INTRO_BEFORE_LINK}
                                <a href="https://xeno-canto.org" target="_blank">"xeno-canto.org"</a>
                                {KEY_INTRO_AFTER_LINK}
                            </p>
                            <label class="xc-field-label" for="xc-api-key">{KEY_FIELD_LABEL}</label>
                            <div class="xc-key-form">
                                <input
                                    id="xc-api-key"
                                    type="password"
                                    class="xc-input"
                                    placeholder=KEY_FIELD_PLACEHOLDER
                                    prop:value=move || api_key_input.get()
                                    on:input=move |ev| {
                                        let val = event_target_value(&ev);
                                        api_key_input.set(val);
                                    }
                                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                                        if ev.key() == "Enter" {
                                            on_save_key(ev.unchecked_into());
                                        }
                                    }
                                />
                                <button class="xc-btn" on:click=on_save_key>{BTN_SAVE_KEY}</button>
                                {has_source.then(|| view! {
                                    <button class="xc-btn" on:click=on_cancel_key_form>{BTN_CANCEL}</button>
                                })}
                            </div>
                            <p class="xc-info xc-key-note">
                                {if is_tauri { KEY_STORED_APP } else { KEY_STORED_WEB }}
                            </p>
                        </div>
                    };
                    Some(view! {
                        <div class="xc-section">
                            {(!on_proxy).then(|| view! {
                                <p class="xc-choose-heading">{CHOOSE_HEADING}</p>
                                <div class="xc-option">
                                    <h3 class="xc-option-heading">{NO_KEY_HEADING}</h3>
                                    <p class="xc-info">{NO_KEY_INTRO}</p>
                                    <button class="xc-btn" on:click=on_use_proxy>{BTN_CONTINUE_WITHOUT_KEY}</button>
                                </div>
                            })}
                            {key_option}
                        </div>
                    })
                }}

                // Main browse/search view
                {move || {
                    let current_view = view.get();
                    if source.get().is_none()
                        || matches!(current_view, BrowserView::KeySetup | BrowserView::Starting)
                    {
                        return None;
                    }

                    Some(view! {
                        <div class="xc-section">
                            // Search bar
                            <div class="xc-search-bar">
                                <input
                                    type="text"
                                    class="xc-input xc-search-input"
                                    placeholder=SEARCH_PLACEHOLDER
                                    prop:value=move || search_input.get()
                                    on:input=move |ev| on_search_input(event_target_value(&ev))
                                    on:keydown=on_search_keydown
                                />
                                <button class="xc-btn" on:click=on_search>"Search"</button>
                            </div>

                            // Group/country filters
                            {move || {
                                // Group, Country and Refresh apply to the group's
                                // species list, so they are hidden while name
                                // results from all groups replace it.
                                if matches!(current_view, BrowserView::GroupBrowse)
                                    && name_results.with(Option::is_none)
                                {
                                    Some(view! {
                                        <div class="xc-filters">
                                            <label>"Group: "</label>
                                            <select
                                                class="xc-select"
                                                on:change=move |ev| {
                                                    selected_group.set(event_target_value(&ev));
                                                }
                                            >
                                                {XC_GROUPS.iter().map(|g| {
                                                    let g = g.to_string();
                                                    let g2 = g.clone();
                                                    let g3 = g.clone();
                                                    view! {
                                                        <option
                                                            value=g.clone()
                                                            selected=move || selected_group.get() == g2
                                                        >{g3}</option>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </select>
                                            <label>" Country: "</label>
                                            <div
                                                class="xc-combobox"
                                                tabindex="-1"
                                                on:focusout=move |ev: web_sys::FocusEvent| {
                                                    // Check if the new focus target is still inside this combobox
                                                    if let Some(related) = ev.related_target() {
                                                        if let Some(el) = ev.current_target() {
                                                            let container: web_sys::HtmlElement = el.unchecked_into();
                                                            let related_node: web_sys::Node = related.unchecked_into();
                                                            if container.contains(Some(&related_node)) {
                                                                return;
                                                            }
                                                        }
                                                    }
                                                    country_dropdown_open.set(false);
                                                    country_filter_text.set(country_input.get_untracked());
                                                }
                                            >
                                                <input
                                                    type="text"
                                                    class="xc-input xc-country-input"
                                                    placeholder="All"
                                                    prop:value=move || country_filter_text.get()
                                                    on:input=move |ev| {
                                                        let val = event_target_value(&ev);
                                                        country_filter_text.set(val);
                                                        country_highlight_idx.set(0);
                                                        country_dropdown_open.set(true);
                                                    }
                                                    on:focus=move |_| {
                                                        country_filter_text.set(country_input.get_untracked());
                                                        country_dropdown_open.set(true);
                                                    }
                                                    on:keydown=on_country_keydown
                                                />
                                                <button
                                                    class="xc-combobox-toggle"
                                                    tabindex="-1"
                                                    on:mousedown=move |ev: web_sys::MouseEvent| {
                                                        ev.prevent_default();
                                                        country_dropdown_open.update(|v| *v = !*v);
                                                    }
                                                >
                                                    {"\u{25BE}"}
                                                </button>
                                                {move || country_dropdown_open.get().then(|| {
                                                    let list = filtered_countries.get();
                                                    view! {
                                                        <div class="xc-combobox-dropdown">
                                                            <button
                                                                class=move || if country_input.get().is_empty() {
                                                                    "xc-combobox-option sel"
                                                                } else {
                                                                    "xc-combobox-option"
                                                                }
                                                                on:mousedown=move |ev: web_sys::MouseEvent| {
                                                                    ev.prevent_default();
                                                                    country_input.set(String::new());
                                                                    country_filter_text.set(String::new());
                                                                    country_dropdown_open.set(false);
                                                                }
                                                            >
                                                                "All (no filter)"
                                                            </button>
                                                            {list.into_iter().enumerate().map(|(i, opt)| {
                                                                let name_owned = opt.name;
                                                                let name_for_set = name_owned.clone();
                                                                let name_for_cls = name_owned.clone();
                                                                let title = match opt.counts {
                                                                    Some((r, s)) => text_country_counts_title(&name_owned, r, s),
                                                                    None => name_owned.clone(),
                                                                };
                                                                view! {
                                                                    <button
                                                                        title=title
                                                                        class=move || {
                                                                            let mut cls = "xc-combobox-option".to_string();
                                                                            if country_input.get() == name_for_cls {
                                                                                cls.push_str(" sel");
                                                                            }
                                                                            if country_highlight_idx.get() == i + 1 {
                                                                                cls.push_str(" highlight");
                                                                            }
                                                                            cls
                                                                        }
                                                                        on:mousedown=move |ev: web_sys::MouseEvent| {
                                                                            ev.prevent_default();
                                                                            country_input.set(name_for_set.clone());
                                                                            country_filter_text.set(name_for_set.clone());
                                                                            country_dropdown_open.set(false);
                                                                        }
                                                                    >
                                                                        <span class="xc-country-name">{name_owned}</span>
                                                                        {opt.counts.map(|(r, _)| view! {
                                                                            <span class="xc-country-count">{text_country_recs(r)}</span>
                                                                        })}
                                                                    </button>
                                                                }
                                                            }).collect::<Vec<_>>()}
                                                        </div>
                                                    }
                                                })}
                                            </div>
                                            <button class="xc-btn" on:click=on_load_group>"Go"</button>
                                        </div>
                                        <div class="xc-cache-info">
                                            {move || taxonomy_age.get().map(|age| view! {
                                                <span class="xc-cache-age">{"Cached: "}{age}</span>
                                            })}
                                            <button
                                                class="xc-btn xc-btn-small"
                                                disabled=move || taxonomy_loading.get()
                                                on:click=on_refresh
                                            >"Refresh"</button>
                                        </div>
                                    })
                                } else {
                                    None
                                }
                            }}

                            // Back button for sub-views
                            {move || {
                                if !matches!(view.get(), BrowserView::GroupBrowse) {
                                    Some(view! {
                                        <button class="xc-btn xc-btn-back" on:click=on_back>
                                            {"\u{2190} Back to species"}
                                        </button>
                                    })
                                } else {
                                    None
                                }
                            }}
                        </div>
                    })
                }}

                // A species list being built: what is loading, how far it
                // has got, and a Stop button. Shown in every view, so a search
                // during a build leaves it visible.
                {move || taxonomy_loading.get().then(|| {
                    let (group, country) = taxonomy_target.get();
                    view! {
                        <div class="xc-loading">
                            <span class="xc-progress-text">
                                {move || text_progress(&group, country.as_deref(), taxonomy_progress.get())}
                            </span>
                            " "
                            <button class="xc-btn xc-btn-small" on:click=on_stop_taxonomy>{BTN_STOP}</button>
                        </div>
                    }
                })}

                // A page of recordings is loading.
                {move || loading.get().then(|| view! {
                    <div class="xc-loading">"Loading..."</div>
                })}

                // The count line and the names menu, above either species list.
                // Made once, outside the lists, so the menu keeps keyboard
                // focus while a list is drawn again.
                <div
                    class="xc-result-summary"
                    style:display=move || if count_line.with(Option::is_some) { "" } else { "none" }
                >
                    <span>{move || count_line.get().unwrap_or_default()}</span>
                    {names_menu()}
                </div>

                // Species found by name (group browse view, while the search
                // box holds a name). Replaces the group's species list.
                {move || {
                    if view.get() != BrowserView::GroupBrowse { return None; }
                    let (query, found) = name_results.get()?;
                    let indexing = text_groups_indexing(&found.groups_indexing);
                    if found.species.is_empty() {
                        return Some(view! {
                            <div class="xc-result-summary">{text_no_name_results(&query)}</div>
                            {indexing.map(|t| view! { <div class="xc-result-summary">{t}</div> })}
                        }.into_any());
                    }
                    Some(view! {
                        {indexing.map(|t| view! { <div class="xc-result-summary">{t}</div> })}
                        <div class="xc-species-list">
                            <div class="xc-list-header xc-named">
                                <span class="xc-col-name">"Species"</span>
                                <span class="xc-col-sci">"Scientific name"</span>
                                <span class="xc-col-group">"Group"</span>
                                <span class="xc-col-count">"Recs"</span>
                            </div>
                            {found.species.into_iter().map(|hit| {
                                let note = text_match_note(&hit, language_name);
                                let sci = format!("{} {}", hit.genus, hit.sp);
                                let NamedSpecies { group, genus, sp, en, recording_count, local, local_lang, .. } = hit;
                                // The English name goes under a name in the user's language.
                                let english = local.as_ref().map(|_| en.clone());
                                let shown = local.unwrap_or(en);
                                let (group_shown, name_shown) = (group.clone(), shown.clone());
                                view! {
                                    <button
                                        class="xc-species-row xc-named"
                                        on:click=move |_| {
                                            load_species_recordings(
                                                group.clone(), None, genus.clone(), sp.clone(), shown.clone(),
                                            );
                                        }
                                    >
                                        <span class="xc-col-name">
                                            <span lang=local_lang>{name_shown}</span>
                                            {english.map(|n| view! { <span class="xc-name-note">{n}</span> })}
                                            {note.map(|n| view! { <span class="xc-name-note">{n}</span> })}
                                        </span>
                                        <span class="xc-col-sci">{sci}</span>
                                        <span class="xc-col-group">{group_shown}</span>
                                        <span class="xc-col-count">{with_commas(recording_count)}</span>
                                    </button>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any())
                }}

                // Species list (group browse view)
                {move || {
                    if view.get() != BrowserView::GroupBrowse { return None; }
                    if name_results.with(Option::is_some) { return None; }
                    let list = species_list.get();
                    if list.is_empty() && !taxonomy_loading.get() { return None; }

                    Some(view! {
                        <div class="xc-species-list">
                            <div class="xc-list-header">
                                <span class="xc-col-name">"Species"</span>
                                <span class="xc-col-sci">"Scientific name"</span>
                                <span class="xc-col-count">"Recs"</span>
                            </div>
                            {list.into_iter().map(|sp| {
                                let genus = sp.genus.clone();
                                let species = sp.sp.clone();
                                let local = species_local_names.with_untracked(|m| {
                                    m.as_ref()?.get(&format!("{} {}", sp.genus, sp.sp)).cloned()
                                });
                                // The English name goes under a name in the user's language.
                                let english = local.as_ref().map(|_| sp.en.clone());
                                let local_lang = local.as_ref().map(|l| l.lang.clone());
                                let en = local.map(|l| l.name).unwrap_or_else(|| sp.en.clone());
                                let name_shown = en.clone();
                                let load_sp = load_species_recordings;
                                view! {
                                    <button
                                        class="xc-species-row"
                                        on:click=move |_| {
                                            load_sp(
                                                species_list_group.get_untracked(),
                                                species_list_country.get_untracked(),
                                                genus.clone(),
                                                species.clone(),
                                                en.clone(),
                                            );
                                        }
                                    >
                                        <span class="xc-col-name">
                                            <span lang=local_lang>{name_shown}</span>
                                            {english.map(|n| view! { <span class="xc-name-note">{n}</span> })}
                                        </span>
                                        <span class="xc-col-sci">{format!("{} {}", sp.genus, sp.sp)}</span>
                                        <span class="xc-col-count">{sp.recording_count}</span>
                                    </button>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    })
                }}

                // Recordings list (species or search view)
                {move || {
                    let current_view = view.get();
                    if !matches!(current_view, BrowserView::SpeciesRecordings { .. } | BrowserView::SearchResults) {
                        return None;
                    }
                    let recs = recordings.get();
                    let title = match &current_view {
                        BrowserView::SpeciesRecordings { en, genus, species, country, .. } => {
                            match country {
                                Some(c) => format!("{en} ({genus} {species}) in {c}"),
                                None => format!("{en} ({genus} {species})"),
                            }
                        }
                        BrowserView::SearchResults => "Search results".to_string(),
                        _ => String::new(),
                    };

                    Some(view! {
                        <div class="xc-recordings-header">
                            {title}
                            <span class="xc-result-count">
                                {move || {
                                    let total = recordings_total.get();
                                    if total == 1 {
                                        " \u{2014} 1 recording".to_string()
                                    } else if total > 0 {
                                        format!(" \u{2014} {} recordings", total)
                                    } else {
                                        String::new()
                                    }
                                }}
                            </span>
                        </div>
                        <div class="xc-recordings-list">
                            <div class="xc-rec-header">
                                <span class="xc-rec-id">"ID"</span>
                                <span class="xc-rec-species">"Species"</span>
                                <span class="xc-rec-quality">"Q"</span>
                                <span class="xc-rec-length">"Len"</span>
                                <span class="xc-rec-loc">"Location"</span>
                                <span class="xc-rec-action"></span>
                            </div>
                            {recs.into_iter().map(|rec| {
                                let id = rec.id_num();
                                let rec_for_dl = rec.clone();
                                let dl = download_and_load;
                                let q_class = match rec.q.as_str() {
                                    "A" => "xc-rec-quality xc-q-a",
                                    "B" => "xc-rec-quality xc-q-b",
                                    "C" => "xc-rec-quality xc-q-c",
                                    "D" => "xc-rec-quality xc-q-d",
                                    "E" => "xc-rec-quality xc-q-e",
                                    _ => "xc-rec-quality",
                                };
                                let has_details = !rec.sound_type.is_empty()
                                    || !rec.smp.is_empty()
                                    || !rec.date.is_empty()
                                    || !rec.rec.is_empty()
                                    || !rec.dvc.is_empty()
                                    || !rec.mic.is_empty();
                                view! {
                                    <div class="xc-rec-row">
                                        <div class="xc-rec-main">
                                            <span class="xc-rec-id">
                                                <a
                                                    href=format!("https://xeno-canto.org/{}", rec.id)
                                                    target="_blank"
                                                    class="xc-rec-link"
                                                    on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()
                                                >
                                                    {format!("XC{}", rec.id)}
                                                </a>
                                            </span>
                                            <span class="xc-rec-species">{rec.en}</span>
                                            <span class=q_class>{rec.q}</span>
                                            <span class="xc-rec-length">{rec.length}</span>
                                            <span class="xc-rec-loc" title=rec.loc.clone()>{rec.cnt}</span>
                                            <span class="xc-rec-action">
                                                {move || cached_ids.get().contains(&id).then(|| view! {
                                                    <span class="xc-rec-cached" title="Cached locally">{"\u{2713}"}</span>
                                                })}
                                                <button
                                                    class="xc-btn xc-btn-load"
                                                    disabled=move || downloading.get().is_some()
                                                    on:click=move |_| dl(rec_for_dl.clone())
                                                >
                                                    {move || if downloading.get() == Some(id) { "Downloading\u{2026}" } else { "Load" }}
                                                </button>
                                            </span>
                                        </div>
                                        {has_details.then(|| view! {
                                            <div class="xc-rec-detail">
                                                {(!rec.sound_type.is_empty()).then(|| view! {
                                                    <span class="xc-rec-tag">{rec.sound_type}</span>
                                                })}
                                                {(!rec.smp.is_empty()).then(|| view! {
                                                    <span class="xc-rec-tag xc-rec-smp">{format_sample_rate(&rec.smp)}</span>
                                                })}
                                                {(!rec.date.is_empty()).then(|| view! {
                                                    <span class="xc-rec-tag">{rec.date}</span>
                                                })}
                                                {(!rec.rec.is_empty()).then(|| view! {
                                                    <span class="xc-rec-tag xc-rec-recordist">{rec.rec}</span>
                                                })}
                                                {(!rec.dvc.is_empty()).then(|| view! {
                                                    <span class="xc-rec-tag" title="Device">{rec.dvc}</span>
                                                })}
                                                {(!rec.mic.is_empty()).then(|| view! {
                                                    <span class="xc-rec-tag" title="Microphone">{rec.mic}</span>
                                                })}
                                            </div>
                                        })}
                                    </div>
                                }
                            }).collect::<Vec<_>>()}
                        </div>

                        // Pagination
                        {move || {
                            let total = recordings_total_pages.get();
                            if total <= 1 { return None; }
                            Some(view! {
                                <div class="xc-pagination">
                                    <button
                                        class="xc-btn xc-btn-small"
                                        disabled=move || recordings_page.get() <= 1
                                        on:click=move |_| load_recordings_page(recordings_page.get_untracked().saturating_sub(1))
                                    >
                                        {"\u{2190} Prev"}
                                    </button>
                                    <span class="xc-page-info">
                                        {move || format!("Page {} of {}", recordings_page.get(), recordings_total_pages.get())}
                                    </span>
                                    <button
                                        class="xc-btn xc-btn-small"
                                        disabled=move || { recordings_page.get() >= recordings_total_pages.get() }
                                        on:click=move |_| load_recordings_page(recordings_page.get_untracked() + 1)
                                    >
                                        {"Next \u{2192}"}
                                    </button>
                                </div>
                            })
                        }}
                    })
                }}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn species_count_line() {
        let de = ["German".to_string()];
        assert_eq!(
            text_species_count(1350, Some(1102), &de),
            "1,350 species, 1,102 with German names"
        );
        assert_eq!(
            text_species_count(1350, Some(0), &de),
            "1,350 species, none with German names"
        );
        assert_eq!(
            text_species_count(1350, None, &de),
            "1,350 species. Couldn't load German names."
        );
        assert_eq!(text_species_count(12, None, &[]), "12 species");
        let three = [
            "Dutch".to_string(),
            "German".to_string(),
            "French".to_string(),
        ];
        assert_eq!(
            text_species_count(5, Some(2), &three),
            "5 species, 2 with Dutch, German or French names"
        );
        assert_eq!(text_browser_option(&[]), "Browser language (English)");
        assert_eq!(
            text_browser_option(&three[..2]),
            "Browser languages (Dutch, German)"
        );
        assert_eq!(
            text_language_option("German", "Deutsch"),
            "German (Deutsch)"
        );
        assert_eq!(text_language_option("Esperanto", "esperanto"), "Esperanto");
        assert_eq!(
            text_language_option("Serbian (Latin)", "srpski (latinica)"),
            "Serbian (Latin)"
        );
    }

    fn hit(name: &str, kind: &str, language: Option<&str>, source: &str) -> NamedSpecies {
        NamedSpecies {
            group: "bats".into(),
            genus: "Pipistrellus".into(),
            sp: "pipistrellus".into(),
            en: "Common Pipistrelle".into(),
            recording_count: 3,
            local: None,
            local_lang: None,
            matched: xc_client::NameMatch {
                name: name.into(),
                kind: kind.into(),
                language: language.map(Into::into),
                source: source.into(),
            },
        }
    }

    #[test]
    fn match_note_names_what_matched_unless_it_is_shown() {
        let lang = |c: &str| {
            if c == "de" {
                "German".to_string()
            } else {
                c.to_string()
            }
        };
        let note = |h: NamedSpecies| text_match_note(&h, lang);
        assert_eq!(
            note(hit("Zwergfledermaus", "common", Some("de"), "col")).as_deref(),
            Some("Zwergfledermaus (German)")
        );
        assert_eq!(
            note(hit("Vespertilio pipistrellus", "synonym", None, "col")).as_deref(),
            Some("Vespertilio pipistrellus (synonym)")
        );
        assert_eq!(
            note(hit("Pipistrel", "common", None, "iucn")).as_deref(),
            Some("Pipistrel (common name)")
        );
        assert_eq!(
            note(hit("common pipistrelle", "common", Some("en"), "iucn")),
            None
        );
        assert_eq!(
            note(hit("Pipistrellus pipistrellus", "scientific", None, "iucn")),
            None
        );
        assert_eq!(note(hit("Anything", "common", Some("en"), "xc")), None);
        // Not repeated when it is the name shown in the user's language.
        let mut shown_locally = hit("zwergfledermaus", "common", Some("de"), "col");
        shown_locally.local = Some("Zwergfledermaus".into());
        assert_eq!(note(shown_locally), None);
    }

    #[test]
    fn name_search_headings() {
        assert_eq!(
            text_name_results(1, "noctule"),
            "1 species matches \u{201C}noctule\u{201D}"
        );
        assert_eq!(
            text_name_results(5, "noctule"),
            "5 species match \u{201C}noctule\u{201D}"
        );
        assert_eq!(
            text_name_results(50, "ba"),
            "Best 50 matches for \u{201C}ba\u{201D}"
        );
        assert_eq!(text_groups_indexing(&[]), None);
        assert_eq!(
            text_groups_indexing(&["birds".into()]).as_deref(),
            Some("Birds are still being indexed, so some may be missing.")
        );
        assert_eq!(
            text_groups_indexing(&["birds".into(), "frogs".into()]).as_deref(),
            Some("Birds and frogs are still being indexed, so some may be missing.")
        );
    }

    fn progress(done: u32, total: u32, secs_left: f64) -> TaxonomyProgress {
        TaxonomyProgress::Local(xc_client::BuildProgress {
            pages_done: 1,
            total_pages: 1,
            recordings_done: done,
            total_recordings: total,
            secs_left,
        })
    }

    #[test]
    fn commas() {
        assert_eq!(with_commas(0), "0");
        assert_eq!(with_commas(999), "999");
        assert_eq!(with_commas(30_000), "30,000");
        assert_eq!(with_commas(1_052_345), "1,052,345");
    }

    #[test]
    fn durations() {
        assert_eq!(text_duration(10.0), "about 1 minute");
        assert_eq!(text_duration(61.0), "about 2 minutes");
        assert_eq!(text_duration(89.0 * 60.0), "about 89 minutes");
        assert_eq!(text_duration(2105.0 * 5.0), "about 3 hours");
    }

    #[test]
    fn progress_text() {
        assert_eq!(
            text_progress("bats", None, None),
            "Loading species list for bats..."
        );
        // Short builds show no time estimate.
        assert_eq!(
            text_progress("bats", None, Some(progress(500, 11_200, 20.0))),
            "Loading species list for bats: 500 of 11,200 recordings"
        );
        assert_eq!(
            text_progress(
                "birds",
                Some("Spain"),
                Some(progress(500, 39_710, 79.0 * 5.0))
            ),
            "Loading species list for birds in Spain: 500 of 39,710 recordings, about 7 minutes left"
        );
    }

    #[test]
    fn server_progress_text() {
        let server = |done, total| {
            Some(TaxonomyProgress::Server(ServerIndexing {
                pages_done: done,
                pages_total: total,
            }))
        };
        assert_eq!(
            text_progress("bats", Some("Australia"), server(3, 23)),
            "Oversample's server is collecting recordings of bats for the species list: 13% done"
        );
        assert_eq!(
            text_progress("land mammals", None, server(0, 0)),
            "Waiting for Oversample's server to start collecting recordings of land mammals for the species list..."
        );
        assert_eq!(
            text_progress("bats", None, server(30, 23)),
            "Oversample's server is collecting recordings of bats for the species list: 100% done"
        );
    }

    #[test]
    fn country_options_prefer_server_list() {
        let builtin = ["Australia", "Brazil", "Spain"];
        let server = vec![
            CountryCount {
                country: "Australia".into(),
                recordings: 22,
                species: 9,
            },
            CountryCount {
                country: "Côte d'Ivoire".into(),
                recordings: 1,
                species: 1,
            },
        ];
        let names = |opts: Vec<CountryOption>| -> Vec<String> {
            opts.into_iter().map(|o| o.name).collect()
        };
        // No server data, or an empty list: the built-in list.
        assert_eq!(names(country_options(None, &builtin, "")), builtin);
        assert_eq!(names(country_options(Some(&[]), &builtin, "a")), builtin);
        // Server data: only countries with recordings, with counts.
        let opts = country_options(Some(&server), &builtin, "");
        assert_eq!(opts[0].counts, Some((22, 9)));
        assert_eq!(names(opts), ["Australia", "Côte d'Ivoire"]);
        assert_eq!(
            names(country_options(Some(&server), &builtin, " CÔTE")),
            ["Côte d'Ivoire"]
        );
    }

    #[test]
    fn country_count_text() {
        assert_eq!(text_country_recs(1), "1 rec");
        assert_eq!(text_country_recs(11_494), "11,494 recs");
        assert_eq!(
            text_country_counts_title("Chad", 1, 1),
            "Chad: 1 recording, 1 species"
        );
        assert_eq!(
            text_country_counts_title("Australia", 22, 9),
            "Australia: 22 recordings, 9 species"
        );
    }

    #[test]
    fn errors_by_source() {
        let proxy = XcSource::Proxy;
        let direct = XcSource::Direct { key: "k".into() };
        assert_eq!(error_text(&XcError::Network, &proxy), ERR_NETWORK_PROXY);
        assert_eq!(error_text(&XcError::Network, &direct), ERR_NETWORK_DIRECT);
        let up = XcError::Upstream(Some("Xeno-canto rejected this server's API key.".into()));
        assert_eq!(error_text(&up, &proxy), ERR_UPSTREAM_PROXY);
        assert_eq!(error_text(&up, &direct), ERR_UPSTREAM_DIRECT);
        // The proxy's own message is not appended.
        let busy = XcError::Busy(Some("Proxy is busy. Try again in a minute.".into()));
        assert_eq!(error_text(&busy, &proxy), ERR_BUSY_PROXY);
        assert_eq!(error_actions(&busy, &proxy), ErrorActions::AddKey);
        assert_eq!(error_actions(&busy, &direct), ErrorActions::None);
        assert_eq!(
            error_actions(&XcError::BadKey(None), &direct),
            ErrorActions::FixKey
        );
        assert_eq!(
            error_text(
                &XcError::Http {
                    status: 500,
                    message: None
                },
                &direct
            ),
            "Request failed (HTTP 500)"
        );
        assert_eq!(
            error_text(&XcError::TooLarge { id: Some(928094) }, &proxy),
            "XC928094 is too large to load through Oversample's server. Add your own API key, \
             or download the file from xeno-canto.org and open it in Oversample."
        );
    }

    #[test]
    fn stopped_text() {
        assert_eq!(
            text_stopped(None, Some(progress(1_000, 11_200, 0.0))),
            "Species list stopped after 1,000 of 11,200 recordings. Choose a country for a \
             shorter list, or click Go to start again."
        );
        assert_eq!(
            text_stopped(Some("Spain"), None),
            "Species list stopped. Click Go to start again."
        );
        assert_eq!(
            text_stopped(None, None),
            "Species list stopped. Click Go to start again."
        );
        assert_eq!(
            text_stopped(
                None,
                Some(TaxonomyProgress::Server(ServerIndexing {
                    pages_done: 3,
                    pages_total: 23
                }))
            ),
            STOPPED_WAITING_FOR_SERVER
        );
    }

    #[test]
    fn age_matches_desktop_wording() {
        assert_eq!(text_age(59.0 * 60.0), "just now");
        assert_eq!(text_age(3600.0), "1 hour ago");
        assert_eq!(text_age(48.0 * 3600.0), "2 days ago");
    }
}

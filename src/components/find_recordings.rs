//! Find recordings: one dialog to find a species and load a recording of it, from every
//! source the server's combined search indexes, plus the demo recordings.
//!
//! Replaces the separate "Load demo", "Explore XC" and "From Wikipedia" buttons. The
//! older Xeno-canto and Commons browsers stay reachable from the footer, and take over
//! when the server has no combined search (an older server, or offline).
//!
//! Layout: search box, filter row (group, country, near + radius), then a species list
//! (left) and the selected entry's recordings (right). On a phone the two panes show one
//! at a time.

use std::cell::RefCell;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::components::file_sidebar::{
    fetch_bytes, fetch_demo_index, find_open_demo, load_named_bytes, load_single_demo, DemoEntry,
};
use crate::search_client::{
    self as search, Facets, Filter, RecordingItem, SearchError, SpeciesHit, DEFAULT_KM, GROUPS,
    RADII_KM,
};
use crate::state::store_fields::*;
use crate::state::AppState;
use crate::web_util::sleep_ms;
use crate::xc_client::{self, XcSource};

// ── Strings ─────────────────────────────────────────────────────────────────

const TITLE: &str = "Find recordings";
const SEARCH_PLACEHOLDER: &str =
    "Species name in any language, or a Xeno-canto, Wikimedia Commons or Wikipedia link";
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
    "Incomplete results. The search server is adding Xeno-canto recordings, which can take hours.";
const NO_SPECIES_MATCH: &str = "No species match \u{201C}{query}\u{201D}.";
const NO_RECORDINGS_SPECIES: &str = "No recordings of this species found on {sources}.";
const NO_RECORDINGS_FILTERS: &str =
    "No recordings match the current filters. Change the Group, Country or Near filter.";
const LOCATION_DENIED: &str = "Location access denied. Near filter set to Anywhere.";
const LOCATION_FAILED: &str = "Could not get your location: {reason}. Near filter set to Anywhere. Choose \u{201C}Me\u{201D} to try again.";
const TRUNCATED: &str =
    "Only the nearest recordings within {km} km are listed (search limit: about 5,000 recordings).";
const APPROX_TIP: &str =
    "Approximate count. It may include some recordings outside the {km} km radius.";
const LOAD_FAILED: &str = "Could not load recording: {name} ({reason})";
const MORE_SPECIES: &str = "Show more species ({remaining} not shown)";
const MORE_RECORDINGS: &str = "Show more recordings ({remaining} not shown)";
const DEMOS_HEADING: &str = "Demo recordings";
const DEMOS_NOTE: &str = "Hand-picked recordings, mostly of bats. They load quickly.";
const ALL_RECORDINGS: &str = "All recordings";
const XC_BROWSER: &str = "Explore Xeno-canto";
const COMMONS_BROWSER: &str = "Search Wikimedia Commons";

/// Country names that take "the" in a sentence ("in the United States").
const COUNTRIES_WITH_THE: [&str; 14] = [
    "US", "GB", "NL", "PH", "BS", "GM", "CZ", "DO", "CF", "CD", "AE", "MV", "MH", "SB",
];

/// "Xeno-canto", "Xeno-canto or iNaturalist", "A, B or C".
fn join_or(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
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
    selected: Selected,
}

thread_local! {
    static MEMORY: RefCell<Option<Memory>> = const { RefCell::new(None) };
}

/// A pasted link that loads one recording directly.
#[derive(Clone, Debug, PartialEq)]
enum Link {
    Xc(u64),
    Wikimedia(String),
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
        Ok(
            crate::wikimedia::WikiTarget::File { .. } | crate::wikimedia::WikiTarget::Page { .. },
        ) => Some(Link::Wikimedia(t.to_string())),
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
            let name = item
                .audio_url
                .rsplit('/')
                .next()
                .map(|s| s.split('?').next().unwrap_or(s).to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| item.id.replace([':', '/'], "_"));
            let load_id = state.loading_start(&label);
            let result = load_named_bytes(
                name,
                &bytes,
                Some(item_metadata(&item)),
                None,
                item.time_expansion,
                Some(search::source_label(&item.source).to_string()),
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
    let selected = RwSignal::new(
        memory
            .as_ref()
            .map(|m| m.selected.clone())
            .unwrap_or(Selected::Demos),
    );
    // On a phone, whether the right pane (recordings) is showing instead of the list.
    let show_detail = RwSignal::new(false);

    let availability = RwSignal::new(Availability::Unknown);
    let status: RwSignal<std::collections::BTreeMap<String, String>> =
        RwSignal::new(Default::default());
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let locating = RwSignal::new(false);

    let species: RwSignal<Vec<SpeciesHit>> = RwSignal::new(Vec::new());
    let species_total = RwSignal::new(0u64);
    let species_cursor: RwSignal<Option<String>> = RwSignal::new(None);
    let species_loading = RwSignal::new(false);
    let species_approx = RwSignal::new(false);
    let species_gen = StoredValue::new(0u64);

    let recs: RwSignal<Vec<RecordingItem>> = RwSignal::new(Vec::new());
    let recs_total = RwSignal::new(0u64);
    let recs_cursor: RwSignal<Option<String>> = RwSignal::new(None);
    let recs_loading = RwSignal::new(false);
    let recs_truncated = RwSignal::new(false);
    let recs_gen = StoredValue::new(0u64);

    let facets: RwSignal<Option<Facets>> = RwSignal::new(None);
    let facets_gen = StoredValue::new(0u64);

    let demos: RwSignal<Vec<DemoEntry>> = RwSignal::new(Vec::new());
    let demos_error: RwSignal<Option<String>> = RwSignal::new(None);
    // Id of the recording (or demo file name) being loaded, so its button shows progress.
    let loading: RwSignal<Option<String>> = RwSignal::new(None);

    let filter = Memo::new(move |_| Filter {
        group: group.get(),
        country: country.get(),
        near: if near_mode.get() == NearMode::Anywhere {
            None
        } else {
            near_point.get()
        },
        km: km.get(),
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
            selected: selected.get(),
        };
        MEMORY.with(|m| *m.borrow_mut() = Some(mem));
    });

    let close = move || state.dialogs.find_recordings_open().set(false);

    let note_status = move |s: &std::collections::BTreeMap<String, String>| {
        status.try_set(s.clone());
    };

    // Species list: runs on every change of the query or filters; typing waits 250 ms.
    Effect::new(move |_| {
        let q = query.get();
        let f = filter.get();
        // Near with no point yet (still locating) has nothing to search.
        if near_mode.get() != NearMode::Anywhere && f.near.is_none() {
            return;
        }
        species_gen.update_value(|g| *g += 1);
        let my = species_gen.get_value();
        if detect_link(&q).is_some() {
            species.set(Vec::new());
            species_total.set(0);
            species_cursor.set(None);
            species_loading.set(false);
            return;
        }
        species_loading.set(true);
        spawn_local(async move {
            if !q.trim().is_empty() {
                sleep_ms(250).await;
            }
            if species_gen.try_get_value() != Some(my) {
                return;
            }
            let result = search::species(&q, &f, None).await;
            if species_gen.try_get_value() != Some(my) {
                return;
            }
            match result {
                Ok(page) => {
                    availability.try_set(Availability::Ready);
                    note_status(&page.status);
                    species_total.try_set(page.total);
                    species_cursor.try_set(page.cursor);
                    species_approx.try_set(page.approx);
                    species.try_set(page.species);
                }
                Err(SearchError::Unavailable) => {
                    availability.try_set(Availability::Unavailable);
                    species.try_set(Vec::new());
                }
                Err(SearchError::Indexing) => {
                    availability.try_set(Availability::Ready);
                    status.try_set([("xc".to_string(), "indexing".to_string())].into());
                    species.try_set(Vec::new());
                }
                Err(SearchError::Other(m)) => {
                    error.try_set(Some(m));
                }
            }
            species_loading.try_set(false);
        });
    });

    // Facets for the Group and Country menus.
    Effect::new(move |_| {
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
        recs_gen.update_value(|g| *g += 1);
        let my = recs_gen.get_value();
        let key = match &sel {
            Selected::Demos => {
                recs.set(Vec::new());
                return;
            }
            Selected::All => None,
            Selected::Species(hit) => {
                if hit.n == 0 {
                    recs.set(Vec::new());
                    recs_total.set(0);
                    recs_cursor.set(None);
                    return;
                }
                Some(hit.key.clone())
            }
        };
        if near_mode.get_untracked() != NearMode::Anywhere && f.near.is_none() {
            return;
        }
        recs_loading.set(true);
        recs.set(Vec::new());
        spawn_local(async move {
            let result = search::recordings(key.as_deref(), &f, None).await;
            if recs_gen.try_get_value() != Some(my) {
                return;
            }
            match result {
                Ok(page) => {
                    recs_total.try_set(page.total);
                    recs_cursor.try_set(page.cursor);
                    recs_truncated.try_set(page.truncated);
                    recs.try_set(page.items);
                }
                Err(SearchError::Unavailable) => {
                    availability.try_set(Availability::Unavailable);
                }
                Err(SearchError::Indexing) => {}
                Err(SearchError::Other(m)) => {
                    error.try_set(Some(m));
                }
            }
            recs_loading.try_set(false);
        });
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
        let Some(cursor) = species_cursor.get_untracked() else {
            return;
        };
        let q = query.get_untracked();
        let f = filter.get_untracked();
        let my = species_gen.get_value();
        species_loading.set(true);
        spawn_local(async move {
            let result = search::species(&q, &f, Some(&cursor)).await;
            if species_gen.try_get_value() != Some(my) {
                return;
            }
            if let Ok(page) = result {
                species_cursor.try_set(page.cursor);
                species.try_update(|s| s.extend(page.species));
            }
            species_loading.try_set(false);
        });
    };

    let more_recs = move || {
        let Some(cursor) = recs_cursor.get_untracked() else {
            return;
        };
        let key = match selected.get_untracked() {
            Selected::Species(hit) => Some(hit.key),
            _ => None,
        };
        let f = filter.get_untracked();
        let my = recs_gen.get_value();
        recs_loading.set(true);
        spawn_local(async move {
            let result = search::recordings(key.as_deref(), &f, Some(&cursor)).await;
            if recs_gen.try_get_value() != Some(my) {
                return;
            }
            if let Ok(page) = result {
                recs_cursor.try_set(page.cursor);
                recs_truncated.try_set(page.truncated);
                recs.try_update(|r| r.extend(page.items));
            }
            recs_loading.try_set(false);
        });
    };

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

    let file_point = Memo::new(move |_| open_file_point(state));

    // ── Species pane ────────────────────────────────────────────────────

    let species_pane = move || {
        let q = query.get();
        let link = detect_link(&q);
        let avail = availability.get();
        let sel = selected.get();
        let approx = species_approx.get() && filter.get().near.is_some();

        let link_row = link.clone().map(|l| {
            let text = match &l {
                Link::Xc(n) => format!("Load XC{n}"),
                Link::Wikimedia(_) => "Load from Wikimedia".to_string(),
            };
            view! {
                <button class="find-link-row" on:click=move |_| load_link(l.clone())>
                    {move || if loading.get().as_deref() == Some("link") { "Loading\u{2026}".to_string() } else { text.clone() }}
                </button>
            }
        });

        let fixed_rows = q.trim().is_empty().then(|| {
            let demos_n = demos.get().len();
            let all_n = facets
                .get()
                .map(|f| f.sources.iter().map(|s| s.n).sum::<u64>())
                .filter(|n| *n > 0);
            view! {
                <button
                    class="find-species-row find-fixed-row"
                    class:selected=sel == Selected::Demos
                    on:click=move |_| select(Selected::Demos)
                >
                    <span class="find-sp-name">{DEMOS_HEADING}</span>
                    <span class="find-sp-count">{(demos_n > 0).then(|| with_commas(demos_n as u64))}</span>
                </button>
                {(avail != Availability::Unavailable).then(|| view! {
                    <button
                        class="find-species-row find-fixed-row"
                        class:selected=sel == Selected::All
                        on:click=move |_| select(Selected::All)
                    >
                        <span class="find-sp-name">{ALL_RECORDINGS}</span>
                        <span class="find-sp-count">{all_n.map(with_commas)}</span>
                    </button>
                })}
            }
        });

        let body = match avail {
            Availability::Unavailable => view! {
                <div class="find-notice">
                    <p>{UNAVAILABLE}</p>
                    <div class="find-notice-actions">
                        <button class="xc-btn" on:click=move |_| open_xc_browser()>{XC_BROWSER}</button>
                        <button class="xc-btn" on:click=move |_| open_wiki_browser()>{COMMONS_BROWSER}</button>
                    </div>
                </div>
            }
            .into_any(),
            _ => {
                let list = species.get();
                let empty = list.is_empty() && !species_loading.get() && link.is_none();
                let rows = list
                    .into_iter()
                    .map(|hit| {
                        let is_sel = matches!(&sel, Selected::Species(s) if s.key == hit.key);
                        let name = hit.display_name();
                        let sci = hit.scientific();
                        let grp = group_label(&hit.grp).to_string();
                        let n = hit.n;
                        let count = if n == 0 {
                            view! { <span class="find-sp-count find-zero">"0"</span> }.into_any()
                        } else if approx {
                            view! { <span class="find-sp-count" title=fill(APPROX_TIP, &[("km", &km.get_untracked().to_string())])>{format!("~{}", with_commas(n))}</span> }.into_any()
                        } else {
                            view! { <span class="find-sp-count">{with_commas(n)}</span> }.into_any()
                        };
                        let for_click = hit.clone();
                        view! {
                            <button
                                class="find-species-row"
                                class:selected=is_sel
                                class:find-no-recordings=n == 0
                                on:click=move |_| select(Selected::Species(for_click.clone()))
                            >
                                <span class="find-sp-name">{name}</span>
                                {count}
                                <span class="find-sp-sci"><i>{sci}</i>" \u{00B7} "{grp}</span>
                            </button>
                        }
                    })
                    .collect::<Vec<_>>();
                let remaining = species_total.get().saturating_sub(species.with(|s| s.len() as u64));
                view! {
                    {rows}
                    {empty.then(|| (!q.trim().is_empty()).then(|| view! {
                        <p class="find-empty">{fill(NO_SPECIES_MATCH, &[("query", q.trim())])}</p>
                    }))}
                    {(species_cursor.get().is_some() && remaining > 0).then(|| view! {
                        <button class="find-more" disabled=move || species_loading.get() on:click=move |_| more_species()>
                            {fill(MORE_SPECIES, &[("remaining", &with_commas(remaining))])}
                        </button>
                    })}
                }
                .into_any()
            }
        };

        view! {
            {(status.get().values().any(|s| s == "indexing")).then(|| view! {
                <p class="find-notice find-notice-small">{INDEXING}</p>
            })}
            {link_row}
            {fixed_rows}
            {body}
        }
    };

    // ── Detail pane ─────────────────────────────────────────────────────

    let recordings_table = move |show_species: bool| {
        let items = recs.get();
        let near_on = filter.get().near.is_some();
        let rows = items
            .into_iter()
            .map(|item| {
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
                let dist = item.km.filter(|_| near_on).map(|k| {
                    if k < 10.0 {
                        format!("{k:.1} km")
                    } else {
                        format!("{} km", k.round() as i64)
                    }
                });
                let length = item
                    .len_s
                    .map(|s| crate::format_time::format_duration(s, 0))
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
                        <span class="find-rec-place">
                            {place}
                            {dist.map(|d| view! { <span class="find-rec-dist">{d}</span> })}
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
            })
            .collect::<Vec<_>>();
        let remaining = recs_total
            .get()
            .saturating_sub(recs.with(|r| r.len() as u64));
        view! {
            <div class="find-rec-head" class:with-species=show_species>
                {show_species.then(|| view! { <span>"Species"</span> })}
                <span>"Place"</span>
                <span>"Date"</span>
                <span class="find-rec-num">"Length"</span>
                <span class="find-rec-num">"Sample rate"</span>
                <span>"Quality"</span>
                <span>"Source"</span>
                <span></span>
            </div>
            {rows}
            {(recs_truncated.get() && near_on).then(|| view! {
                <p class="find-empty">{fill(TRUNCATED, &[("km", &km.get().to_string())])}</p>
            })}
            {(recs_cursor.get().is_some() && remaining > 0).then(|| view! {
                <button class="find-more" disabled=move || recs_loading.get() on:click=move |_| more_recs()>
                    {fill(MORE_RECORDINGS, &[("remaining", &with_commas(remaining))])}
                </button>
            })}
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
        if f.near.is_some() {
            let whom = match near_mode.get() {
                NearMode::Me => "you",
                _ => "the current recording",
            };
            s.push_str(&format!(" within {} km of {whom}", f.km));
        }
        s
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
            Selected::All => {
                let n = recs_total.get();
                view! {
                    {back}
                    <div class="find-detail-head">
                        <h3>{ALL_RECORDINGS}</h3>
                        <p class="find-detail-sub">
                            {move || format!("{}{}", plural(recs_total.get(), "recording", "recordings"), filter_summary())}
                        </p>
                    </div>
                    {(n == 0 && !recs_loading.get()).then(|| view! { <p class="find-empty">{NO_RECORDINGS_FILTERS}</p> })}
                    {recordings_table(true)}
                    {move || recs_loading.get().then(|| view! { <p class="xc-loading">"Loading\u{2026}"</p> })}
                }
                .into_any()
            }
            Selected::Species(hit) => {
                let name = hit.display_name();
                let sci = hit.scientific();
                let wiki = hit.wikipedia_url();
                let bb = hit.beastiebot_url();
                let zero = hit.n == 0;
                view! {
                    {back}
                    <div class="find-detail-head">
                        <h3>{name}" "<i class="find-detail-sci">{sci}</i></h3>
                        <p class="find-detail-sub">
                            {move || if zero {
                                String::new()
                            } else {
                                format!("{}{} \u{00B7} ", plural(recs_total.get(), "recording", "recordings"), filter_summary())
                            }}
                            <a href=wiki target="_blank" rel="noopener">"Wikipedia"</a>
                            " \u{00B7} "
                            <a href=bb target="_blank" rel="noopener">"BeastieBot"</a>
                        </p>
                    </div>
                    {zero.then(|| {
                        let sources: Vec<String> = status
                            .get()
                            .keys()
                            .map(|k| search::source_label(k).to_string())
                            .collect();
                        let sources = if sources.is_empty() { vec!["Xeno-canto".to_string()] } else { sources };
                        view! { <p class="find-empty">{fill(NO_RECORDINGS_SPECIES, &[("sources", &join_or(&sources))])}</p> }
                    })}
                    {(!zero).then(|| view! {
                        {move || (recs_total.get() == 0 && !recs_loading.get()).then(|| view! {
                            <p class="find-empty">{NO_RECORDINGS_FILTERS}</p>
                        })}
                        {move || recordings_table(false)}
                        {move || recs_loading.get().then(|| view! { <p class="xc-loading">"Loading\u{2026}"</p> })}
                    })}
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
                        placeholder=SEARCH_PLACEHOLDER
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
                    <div class="find-species">
                        {species_pane}
                        {move || species_loading.get().then(|| view! { <p class="xc-loading">"Searching\u{2026}"</p> })}
                    </div>
                    <div class="find-detail">{detail_pane}</div>
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
    fn commas() {
        assert_eq!(with_commas(0), "0");
        assert_eq!(with_commas(1234), "1,234");
        assert_eq!(with_commas(1234567), "1,234,567");
        assert_eq!(plural(1, "recording", "recordings"), "1 recording");
    }
}

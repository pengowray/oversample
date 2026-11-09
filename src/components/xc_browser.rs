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
use crate::xc_client::{self, TaxonomyAge, XcError, XcSource, PROXY_MAX_TAXONOMY_PAGES};
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_ipc::xc::{XcGroupTaxonomy, XcRecording, XcSearchResult, XcSpecies};
use wasm_bindgen::prelude::*;

// ── UI text ──────────────────────────────────────────────────────────
//
// Every string added or changed for the web/proxy work is here, so the
// wording can be replaced in one place. Strings that existed before and are
// still correct stay inline in the view below.

/// Source line when requests use the user's own key (web or desktop).
const SOURCE_OWN_KEY: &str = "Using your Xeno-Canto API key.";
/// Source line when requests go through the Oversample proxy.
const SOURCE_PROXY: &str =
    "Using Oversample's server, no key needed. Large groups need a country filter.";
/// Link in the source line: open the key form to replace the saved key.
const LINK_CHANGE_KEY: &str = "Change key";
/// Link in the source line: switch from the user's key to the proxy.
const LINK_USE_PROXY: &str = "Use without key";
/// Link in the source line (proxy mode, no key saved): open the key form.
const LINK_ADD_KEY: &str = "Add your key";
/// Link in the source line (proxy mode, key saved): switch back to the key.
const LINK_USE_OWN_KEY: &str = "Use my key";

/// Key form intro, before the xeno-canto.org link. (Existing text.)
const KEY_INTRO_BEFORE_LINK: &str =
    "Enter your Xeno-Canto API key. You can get one by creating a free account at ";
/// Key form intro, after the link. (Existing text.)
const KEY_INTRO_AFTER_LINK: &str = " and going to your account settings.";
/// Under the key form, web build only: where the key is kept.
const KEY_STORED_WEB: &str = "Your key is saved in this browser only.";
/// Key form button: forget the saved key.
const BTN_REMOVE_KEY: &str = "Remove key";
/// Key form button: leave the form without changes.
const BTN_CANCEL: &str = "Cancel";
/// Explains the no-key option, above its button.
const NO_KEY_INTRO: &str = "No key? You can search and load recordings through Oversample's \
     server instead. Large species lists need a country filter.";
/// Button that chooses the proxy.
const BTN_CONTINUE_WITHOUT_KEY: &str = "Continue without a key";

/// Button that stops a species list that is still loading.
const BTN_STOP: &str = "Stop";

const ERR_NETWORK: &str = "Couldn't reach the server. Check your internet connection.";
const ERR_RATE_LIMITED: &str = "Too many requests. Wait a few seconds and try again.";
const ERR_BUSY: &str = "Oversample's Xeno-Canto server is busy or unavailable. Try again \
     later, or add your own key.";
const ERR_UPSTREAM: &str = "Xeno-Canto didn't respond. Try again later.";
const ERR_BAD_KEY: &str =
    "Xeno-Canto didn't accept your API key. Check it on your xeno-canto.org account page.";
const ERR_TOO_LARGE: &str =
    "This recording is too large to download without a key. Add your own key to load it.";

/// Progress while a species list is built page by page.
fn text_progress(done: u32, total: u32) -> String {
    let secs_left = total.saturating_sub(done) as f64 * 1.1;
    let mins_left = (secs_left / 60.0).ceil() as u32;
    if mins_left >= 2 {
        format!("Loading species list: page {done} of {total}, about {mins_left} minutes left")
    } else {
        format!("Loading species list: page {done} of {total}")
    }
}

/// Any other HTTP error when the server gave no message.
fn text_http_failed(status: u16) -> String {
    format!("Request failed (HTTP {status})")
}

/// The group is too big to list through the proxy.
fn text_group_too_large(pages: u32) -> String {
    format!(
        "This species list needs {pages} pages of results. Without a key the limit is \
         {PROXY_MAX_TAXONOMY_PAGES}. Choose a country, or add your own Xeno-Canto key."
    )
}

/// Age of a species list built in the browser. Same wording as the
/// desktop cache (`xc_lib::cache::taxonomy_age_string`).
fn text_age(secs: f64) -> String {
    let hours = (secs / 3600.0).floor() as i64;
    if hours < 1 {
        "just now".to_string()
    } else if hours < 24 {
        format!("{hours} hour{} ago", if hours == 1 { "" } else { "s" })
    } else {
        let days = hours / 24;
        format!("{days} day{} ago", if days == 1 { "" } else { "s" })
    }
}

/// A server message added after one of the fixed error texts.
fn with_detail(text: &str, detail: &Option<String>) -> String {
    match detail {
        Some(d) => format!("{text} ({d})"),
        None => text.to_string(),
    }
}

fn error_text(e: &XcError) -> String {
    match e {
        XcError::Network => ERR_NETWORK.to_string(),
        XcError::RateLimited => ERR_RATE_LIMITED.to_string(),
        XcError::Busy(d) => with_detail(ERR_BUSY, d),
        XcError::Upstream(d) => with_detail(ERR_UPSTREAM, d),
        XcError::BadKey(d) => with_detail(ERR_BAD_KEY, d),
        XcError::TooLarge => ERR_TOO_LARGE.to_string(),
        XcError::GroupTooLarge { pages } => text_group_too_large(*pages),
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

// ── Static data ──────────────────────────────────────────────────────

const XC_GROUPS: &[&str] = &["bats", "birds", "frogs", "grasshoppers", "land mammals"];

const XC_COUNTRIES_RAW: &str = include_str!("../data/countries.txt");

fn xc_countries() -> Vec<&'static str> {
    XC_COUNTRIES_RAW.lines().filter(|l| !l.is_empty()).collect()
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
    let recordings: RwSignal<Vec<XcRecording>> = RwSignal::new(Vec::new());
    let loading = RwSignal::new(false);
    let error_msg: RwSignal<Option<String>> = RwSignal::new(None);
    let taxonomy_age: RwSignal<Option<String>> = RwSignal::new(None);
    // (pages done, total pages) while a species list is built in the browser.
    let taxonomy_progress: RwSignal<Option<(u32, u32)>> = RwSignal::new(None);
    // Bumped to cancel a running species-list build.
    let taxonomy_gen = RwSignal::new(0u64);
    let search_input = RwSignal::new(String::new());
    let recordings_page = RwSignal::new(1u32);
    let recordings_total_pages = RwSignal::new(1u32);
    let downloading: RwSignal<Option<u64>> = RwSignal::new(None);
    let recordings_total: RwSignal<u32> = RwSignal::new(0);
    let cached_ids: RwSignal<std::collections::HashSet<u64>> =
        RwSignal::new(std::collections::HashSet::new());

    // Country combobox state
    let country_dropdown_open = RwSignal::new(false);
    let country_filter_text = RwSignal::new(String::new());
    let country_highlight_idx = RwSignal::new(0usize);
    let countries = xc_countries();
    let filtered_countries = Memo::new(move |_| {
        let filter = country_filter_text.get().to_lowercase();
        if filter.is_empty() {
            countries.clone()
        } else {
            countries
                .iter()
                .copied()
                .filter(|c| c.to_lowercase().contains(&filter))
                .collect::<Vec<_>>()
        }
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
        spawn_local(async move {
            match xc_client::save_key(is_tauri, &key).await {
                Ok(src) => {
                    own_key.try_set(Some(src.clone()));
                    source.try_set(Some(src));
                    api_key_input.try_set(String::new());
                    view.try_set(BrowserView::GroupBrowse);
                    error_msg.try_set(None);
                }
                Err(e) => {
                    error_msg.try_set(Some(format!("Failed to save key: {}", error_text(&e))));
                }
            }
        });
    };

    let on_remove_key = move |_: web_sys::MouseEvent| {
        spawn_local(async move {
            match xc_client::remove_key(is_tauri).await {
                Ok(()) => {
                    own_key.try_set(None);
                    // Using the key until now: there is no source left.
                    if source.try_get_untracked().flatten() != Some(XcSource::Proxy) {
                        source.try_set(None);
                    }
                    error_msg.try_set(None);
                }
                Err(e) => {
                    error_msg.try_set(Some(error_text(&e)));
                }
            }
        });
    };

    let on_use_proxy = move |_: web_sys::MouseEvent| {
        xc_client::set_use_proxy(true);
        source.set(Some(XcSource::Proxy));
        error_msg.set(None);
        if view.get_untracked() == BrowserView::KeySetup {
            view.set(BrowserView::GroupBrowse);
        }
    };

    let on_use_own_key = move |_: web_sys::MouseEvent| {
        if let Some(src) = own_key.get_untracked() {
            xc_client::set_use_proxy(false);
            source.set(Some(src));
            error_msg.set(None);
        }
    };

    let on_open_key_form = move |_: web_sys::MouseEvent| {
        error_msg.set(None);
        view.set(BrowserView::KeySetup);
    };

    let on_cancel_key_form = move |_: web_sys::MouseEvent| {
        view.set(BrowserView::GroupBrowse);
    };

    // Build or reload the species list for the selected group and country.
    let load_group = move |refresh: bool| {
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
        loading.set(true);
        error_msg.set(None);
        taxonomy_age.set(None);
        taxonomy_progress.set(None);
        if !refresh {
            species_list.set(Vec::new());
        }

        let is_current = move || taxonomy_gen.try_get_untracked() == Some(my_gen);
        spawn_local(async move {
            let result = xc_client::browse_group(
                &src,
                &group,
                country.as_deref(),
                refresh,
                move |done, total| {
                    if is_current() {
                        taxonomy_progress.try_set(Some((done, total)));
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
                Err(e) => {
                    error_msg.try_set(Some(error_text(&e)));
                    None
                }
            };
            if let Some(t) = taxonomy {
                let age = xc_client::taxonomy_age(&src, &group, country.as_deref(), &t).await;
                if !is_current() {
                    return;
                }
                taxonomy_age.try_set(age.map(|a| match a {
                    TaxonomyAge::Text(s) => s,
                    TaxonomyAge::Seconds(secs) => text_age(secs),
                }));
                species_list.try_set(t.species);
            }
            taxonomy_progress.try_set(None);
            loading.try_set(false);
        });
    };

    let on_stop_taxonomy = move |_: web_sys::MouseEvent| {
        taxonomy_gen.update(|g| *g += 1);
        taxonomy_progress.set(None);
        loading.set(false);
    };

    let on_load_group = move |_: web_sys::MouseEvent| {
        // Commit any pending filter text
        let text = country_filter_text.get_untracked().trim().to_string();
        country_input.set(text);
        country_dropdown_open.set(false);
        load_group(false);
    };

    let on_refresh = move |_: web_sys::MouseEvent| {
        load_group(true);
    };

    let check_cached = move |src: XcSource, ids: Vec<u64>| {
        spawn_local(async move {
            let set = xc_client::cached_ids(&src, ids).await;
            cached_ids.try_set(set);
        });
    };

    // Show one page of recordings (species view or search results).
    let show_results = move |src: XcSource, result: Result<XcSearchResult, XcError>| {
        match result {
            Ok(r) => {
                let ids: Vec<u64> = r.recordings.iter().map(|rec| rec.id_num()).collect();
                recordings_page.try_set(r.page);
                recordings_total_pages.try_set(r.num_pages);
                recordings_total.try_set(r.num_recordings);
                recordings.try_set(r.recordings);
                check_cached(src, ids);
            }
            Err(e) => {
                error_msg.try_set(Some(error_text(&e)));
            }
        }
        loading.try_set(false);
    };

    let load_species_recordings = move |genus: String, species: String, en: String| {
        let Some(src) = source.get_untracked() else {
            return;
        };
        view.set(BrowserView::SpeciesRecordings {
            genus: genus.clone(),
            species: species.clone(),
            en,
        });
        loading.set(true);
        recordings.set(Vec::new());
        recordings_page.set(1);
        error_msg.set(None);

        spawn_local(async move {
            let result = xc_client::species_recordings(&src, &genus, &species, 1).await;
            show_results(src, result);
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
        loading.set(true);
        recordings.set(Vec::new());
        error_msg.set(None);

        spawn_local(async move {
            let result = xc_client::search(&src, &query, 1).await;
            show_results(src, result);
        });
    };

    let on_search_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" {
            on_search(ev.unchecked_into());
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
                } else if let Some(name) = list.get(idx - 1) {
                    country_input.set(name.to_string());
                    country_filter_text.set(name.to_string());
                }
                country_dropdown_open.set(false);
            } else {
                let text = country_filter_text.get_untracked().trim().to_string();
                country_input.set(text);
            }
            load_group(false);
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
        view.set(BrowserView::GroupBrowse);
        recordings.set(Vec::new());
    };

    let load_recordings_page = move |page_num: u32| {
        let current_view = view.get_untracked();
        let Some(src) = source.get_untracked() else {
            return;
        };
        loading.set(true);
        recordings.set(Vec::new());
        error_msg.set(None);

        spawn_local(async move {
            let result = match &current_view {
                BrowserView::SpeciesRecordings { genus, species, .. } => {
                    xc_client::species_recordings(&src, genus, species, page_num).await
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
            show_results(src, result);
        });
    };

    let download_and_load = move |rec: XcRecording| {
        let Some(src) = source.get_untracked() else {
            return;
        };
        let id = rec.id_num();
        downloading.set(Some(id));
        error_msg.set(None);
        spawn_local(async move {
            let result: Result<(), String> = async {
                let dl = xc_client::download(&src, &rec)
                    .await
                    .map_err(|e| error_text(&e))?;

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
                error_msg.try_set(Some(format!("Failed to load: {e}")));
            }
            downloading.try_set(None);
        });
    };

    view! {
        <div class="xc-modal-overlay" on:click=on_close>
            <div class="xc-modal" on:click=on_content_click>
                <div class="xc-modal-header">
                    <span class="xc-modal-title">"Explore Xeno-Canto"</span>
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
                                        <button class="xc-link-btn" on:click=on_use_own_key>{LINK_USE_OWN_KEY}</button>
                                    }.into_any()
                                } else {
                                    view! {
                                        <button class="xc-link-btn" on:click=on_open_key_form>{LINK_ADD_KEY}</button>
                                    }.into_any()
                                }
                            } else {
                                view! {
                                    <button class="xc-link-btn" on:click=on_open_key_form>{LINK_CHANGE_KEY}</button>
                                    <button class="xc-link-btn" on:click=on_use_proxy>{LINK_USE_PROXY}</button>
                                }.into_any()
                            }}
                        </div>
                    })
                }}

                // Error display
                {move || error_msg.get().filter(|m| !m.is_empty()).map(|msg| view! {
                    <div class="xc-error">
                        <span>{msg}</span>
                        <button class="xc-error-dismiss" on:click=move |_| error_msg.set(None)>{"\u{00D7}"}</button>
                    </div>
                })}

                // Download progress indicator
                {move || downloading.get().map(|id| view! {
                    <div class="xc-downloading">{format!("Downloading XC{id}\u{2026}")}</div>
                })}

                // API key form and the no-key option
                {move || {
                    if view.get() != BrowserView::KeySetup { return None; }
                    let has_own = own_key.get().is_some();
                    let has_source = source.get().is_some();
                    let on_proxy = source.get() == Some(XcSource::Proxy);
                    Some(view! {
                        <div class="xc-section">
                            <p class="xc-info">
                                {KEY_INTRO_BEFORE_LINK}
                                <a href="https://xeno-canto.org" target="_blank">"xeno-canto.org"</a>
                                {KEY_INTRO_AFTER_LINK}
                            </p>
                            <div class="xc-key-form">
                                <input
                                    type="password"
                                    class="xc-input"
                                    placeholder="API key"
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
                                <button class="xc-btn" on:click=on_save_key>"Save key"</button>
                                {has_own.then(|| view! {
                                    <button class="xc-btn" on:click=on_remove_key>{BTN_REMOVE_KEY}</button>
                                })}
                                {has_source.then(|| view! {
                                    <button class="xc-btn" on:click=on_cancel_key_form>{BTN_CANCEL}</button>
                                })}
                            </div>
                            {(!is_tauri).then(|| view! {
                                <p class="xc-info xc-key-note">{KEY_STORED_WEB}</p>
                            })}
                        </div>
                        // Hidden when already using the proxy (came here via "Add your key").
                        {(!on_proxy).then(|| view! {
                            <div class="xc-section">
                                <p class="xc-info">{NO_KEY_INTRO}</p>
                                <button class="xc-btn" on:click=on_use_proxy>{BTN_CONTINUE_WITHOUT_KEY}</button>
                            </div>
                        })}
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
                                    placeholder="Search (e.g. Myotis, bat, Australia...)"
                                    prop:value=move || search_input.get()
                                    on:input=move |ev| search_input.set(event_target_value(&ev))
                                    on:keydown=on_search_keydown
                                />
                                <button class="xc-btn" on:click=on_search>"Search"</button>
                            </div>

                            // Group/country filters
                            {move || {
                                if matches!(current_view, BrowserView::GroupBrowse) {
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
                                                            {list.into_iter().enumerate().map(|(i, name)| {
                                                                let name_owned = name.to_string();
                                                                let name_for_set = name_owned.clone();
                                                                let name_for_cls = name_owned.clone();
                                                                view! {
                                                                    <button
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
                                                                        {name_owned}
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
                                                disabled=move || loading.get()
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

                // Loading indicator. A species list built in the browser shows
                // its page count and can be stopped.
                {move || loading.get().then(|| match taxonomy_progress.get() {
                    Some((done, total)) => view! {
                        <div class="xc-loading">
                            <span class="xc-progress-text">{text_progress(done, total)}</span>
                            " "
                            <button class="xc-btn xc-btn-small" on:click=on_stop_taxonomy>{BTN_STOP}</button>
                        </div>
                    }.into_any(),
                    None => view! {
                        <div class="xc-loading">"Loading..."</div>
                    }.into_any(),
                })}

                // Species list (group browse view)
                {move || {
                    if view.get() != BrowserView::GroupBrowse { return None; }
                    let list = species_list.get();
                    if list.is_empty() && !loading.get() { return None; }

                    let count = list.len();
                    Some(view! {
                        <div class="xc-result-summary">{format!("{count} species")}</div>
                        <div class="xc-species-list">
                            <div class="xc-list-header">
                                <span class="xc-col-name">"Species"</span>
                                <span class="xc-col-sci">"Scientific name"</span>
                                <span class="xc-col-count">"Recs"</span>
                            </div>
                            {list.into_iter().map(|sp| {
                                let genus = sp.genus.clone();
                                let species = sp.sp.clone();
                                let en = sp.en.clone();
                                let load_sp = load_species_recordings;
                                view! {
                                    <button
                                        class="xc-species-row"
                                        on:click=move |_| {
                                            load_sp(genus.clone(), species.clone(), en.clone());
                                        }
                                    >
                                        <span class="xc-col-name">{sp.en}</span>
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
                        BrowserView::SpeciesRecordings { en, genus, species } => {
                            format!("{en} ({genus} {species})")
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
                                    if total > 0 {
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
                                        disabled=move || recordings_page.get() >= recordings_total_pages.get()
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

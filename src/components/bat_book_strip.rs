use crate::bat_book::auto_resolve;
use crate::bat_book::data::get_manifest;
use crate::bat_book::types::{BatBookEntry, BatBookMode, BatBookRegion};
use crate::components::icons::Icon;
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;

/// Persist bat book mode to localStorage.
fn persist_mode(mode: &BatBookMode) {
    crate::settings::set_raw(crate::settings::keys::BAT_BOOK_MODE, mode.storage_key());
}

/// Persist favourites to localStorage.
fn persist_favourites(favs: &[BatBookRegion]) {
    let val: String = favs
        .iter()
        .map(|r| r.storage_key())
        .collect::<Vec<_>>()
        .join(",");
    crate::settings::set_raw(crate::settings::keys::BAT_BOOK_FAVOURITES, &val);
}

/// Horizontal scrolling strip of bat family chips.
/// Sits between the main view and the bottom toolbar.
#[component]
pub fn BatBookStrip() -> impl IntoView {
    let state = expect_context::<AppState>();
    let region_menu_open = RwSignal::new(false);
    let scroll_ref = NodeRef::<leptos::html::Div>::new();

    // ── Auto-resolve Effect ──────────────────────────────────────────────
    // Watches mode, files, and current_file_index. Sets bat_book_region
    // and bat_book_auto_resolved so downstream code (manifest Memo, etc.)
    // continues to work unchanged.
    // A recording with its own place (GPS or Xeno-Canto country) always uses
    // that place's region; a region picked in the menu is for recordings
    // without one. The exception is a picked region that is part of the
    // recording's region (VIC within Australia), which is kept.
    Effect::new(move |_| {
        let mode = state.bat_book.mode().get();
        let files = state.library.files().get();
        let idx = state.library.current_index().get();
        let file = idx.and_then(|i| files.get(i));
        let favourites = state.bat_book.favourites().get_untracked();
        let resolved = match mode {
            BatBookMode::Auto => Some(auto_resolve::resolve_auto(file, &favourites)),
            BatBookMode::Manual(picked) => file
                .and_then(|f| auto_resolve::recording_region(f, &favourites))
                .filter(|r| !picked.is_part_of(r.region)),
        };
        let region = match (&resolved, mode) {
            (Some(r), _) => r.region,
            (None, BatBookMode::Manual(region)) => region,
            (None, BatBookMode::Auto) => BatBookRegion::Global,
        };
        state.bat_book.region().set(region);
        state.bat_book.auto_resolved().set(resolved);
    });

    let manifest = Memo::new(move |_| {
        let region = state.bat_book.region().get();
        get_manifest(region)
    });

    let on_close = move |_: web_sys::MouseEvent| {
        state.bat_book.open().set(false);
    };

    let on_title_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        state.bat_book.open().set(false);
    };

    let on_config = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        region_menu_open.update(|v| *v = !*v);
    };

    // Convert vertical scroll to horizontal scroll in the chip area
    let on_wheel = move |ev: web_sys::WheelEvent| {
        if let Some(el) = scroll_ref.get() {
            let el: &web_sys::HtmlElement = el.as_ref();
            let delta = ev.delta_y();
            if delta.abs() > 0.0 {
                ev.prevent_default();
                el.set_scroll_left(el.scroll_left() + delta as i32);
            }
        }
    };

    // ── Auto-select species once per file ──────────────────────────────
    // Tracks (file_index, species_id) so we only auto-select once per file.
    let auto_selected_for: RwSignal<Option<(usize, String)>> = RwSignal::new(None);

    Effect::new(move |_| {
        let is_open = state.bat_book.open().get();
        let resolved = state.bat_book.auto_resolved().get();
        let file_idx = state.library.current_index().get();

        let Some(idx) = file_idx else { return };
        let Some(ref res) = resolved else { return };
        let Some(ref species_id) = res.matched_species_id else {
            return;
        };

        // Only auto-select when the book is open. `auto_resolved` is set only
        // in Auto mode or when the region comes from the recording's place.
        if !is_open {
            return;
        }

        // Don't re-select if we already did for this file+species
        if auto_selected_for.get_untracked() == Some((idx, species_id.clone())) {
            return;
        }
        auto_selected_for.set(Some((idx, species_id.clone())));

        // Select the species
        state.bat_book.selected_ids().set(vec![species_id.clone()]);
        state
            .bat_book
            .last_clicked_id()
            .set(Some(species_id.clone()));
        state.bat_book.ref_open().set(true);
        if state.bat_book.auto_focus().get_untracked() {
            apply_bat_book_ff(&state);
        }
    });

    // The auto-matched species entry (if any) to show before the divider
    let auto_matched_entry = Memo::new(move |_| {
        let resolved = state.bat_book.auto_resolved().get()?;
        let species_id = resolved.matched_species_id.as_deref()?;
        // Try to find the entry in the current region's manifest first
        let region = state.bat_book.region().get();
        auto_resolve::find_entry_in_manifest(region, species_id)
            .or_else(|| auto_resolve::find_entry_any_book(species_id))
    });

    view! {
        <div class="bat-book-strip" on:click=move |_| { region_menu_open.set(false); }>
            <div class="bat-book-header">
                <span class="bat-book-title" on:click=on_title_click style="cursor:pointer">"Bat Book"</span>
                <span
                    class="bat-book-region-label"
                    title=move || {
                        let resolved = state.bat_book.auto_resolved().get()?;
                        let place = resolved.recording_place.as_ref()?;
                        let mut tip = place.detail(resolved.region);
                        if let BatBookMode::Manual(r) = state.bat_book.mode().get() {
                            if r != resolved.region {
                                tip.push_str(&format!(
                                    "\n{} is used for recordings without location data.",
                                    r.label()
                                ));
                            }
                        }
                        Some(tip)
                    }
                >
                    {move || {
                        let mode = state.bat_book.mode().get();
                        match (mode, state.bat_book.auto_resolved().get()) {
                            (_, Some(resolved)) if resolved.recording_place.is_some() => {
                                let place = resolved.recording_place.as_ref().unwrap();
                                format!("{}: {}", place.source_label(), resolved.source_label)
                            }
                            (BatBookMode::Auto, Some(resolved)) => {
                                if resolved.from_favourite {
                                    format!("Auto: {} \u{2605}", resolved.source_label)
                                } else {
                                    format!("Auto: {}", resolved.source_label)
                                }
                            }
                            (BatBookMode::Auto, None) => "Auto".to_string(),
                            (BatBookMode::Manual(r), _) => r.short_label().to_string(),
                        }
                    }}
                </span>
                <div class="bat-book-config-wrap">
                    <button
                        class="bat-book-config-btn"
                        on:click=on_config
                        title="Choose region"
                    >
                        <Icon kind=Icon::Settings />
                    </button>
                    <Show when=move || region_menu_open.get()>
                        // Invisible full-screen backdrop to catch outside clicks
                        <div class="bat-book-menu-backdrop"
                            on:click=move |ev: web_sys::MouseEvent| {
                                ev.stop_propagation();
                                region_menu_open.set(false);
                            }
                        ></div>
                        <RegionMenu region_menu_open=region_menu_open />
                    </Show>
                </div>
                <div style="flex:1"></div>
                <button
                    class="bat-book-close-btn"
                    on:click=on_close
                    title="Close bat book"
                >
                    "\u{00d7}"
                </button>
            </div>
            <div class="bat-book-scroll" node_ref=scroll_ref on:wheel=on_wheel>
                {move || {
                    let mut chips: Vec<leptos::tachys::view::any_view::AnyView> = Vec::new();

                    // Auto-matched species chip (shown first, before divider)
                    if let Some(entry) = auto_matched_entry.get() {
                        chips.push(view! {
                            <BatBookChip entry=entry.clone() is_auto_matched=true />
                        }.into_any());
                        chips.push(view! {
                            <div class="bat-book-auto-divider" title="Regional book"></div>
                        }.into_any());
                    }

                    // Full regional book
                    let m = manifest.get();
                    let has_non_echo = m.entries.iter().any(|e| !e.echolocates);
                    for (i, entry) in m.entries.iter().enumerate() {
                        let show_divider = has_non_echo && !entry.echolocates
                            && (i == 0 || m.entries[i - 1].echolocates);
                        if show_divider {
                            chips.push(view! {
                                <div class="bat-book-divider" title="Non-echolocating"></div>
                            }.into_any());
                        }
                        chips.push(view! {
                            <BatBookChip entry=entry.clone() is_auto_matched=false />
                        }.into_any());
                    }
                    chips
                }}
            </div>
        </div>
    }
}

/// Region selector dropdown with Auto, favourites, and all regions.
#[component]
fn RegionMenu(region_menu_open: RwSignal<bool>) -> impl IntoView {
    let state = expect_context::<AppState>();

    let select_auto = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        state.bat_book.mode().set(BatBookMode::Auto);
        persist_mode(&BatBookMode::Auto);
        region_menu_open.set(false);
    };

    let is_auto_active = move || state.bat_book.mode().get() == BatBookMode::Auto;

    // The open recording's own place, which wins over the options below.
    let recording = move || {
        let resolved = state.bat_book.auto_resolved().get()?;
        let place = resolved.recording_place.clone()?;
        Some((resolved.region, place))
    };

    // "Set region from my location": None = idle, Some(None) = waiting,
    // Some(Some(msg)) = failed with msg.
    let locating: RwSignal<Option<Option<String>>> = RwSignal::new(None);
    let on_locate = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        if locating.get_untracked() == Some(None) {
            return;
        }
        locating.set(Some(None));
        leptos::task::spawn_local(async move {
            use crate::bat_book::auto_resolve::{region_at, PositionRegion};
            use crate::device_location::{current, LocationError};
            // The position is used here, on the device, and then dropped.
            let msg = match current(state.is_tauri).await {
                Err(LocationError::Denied) => "Location permission denied".to_string(),
                Err(LocationError::Unavailable(m)) => format!("Location unavailable: {m}"),
                Ok((lat, lon)) => match region_at(lat as f32, lon as f32) {
                    PositionRegion::Found(r, _) => {
                        let mode = BatBookMode::Manual(r);
                        state.bat_book.mode().set(mode);
                        persist_mode(&mode);
                        locating.set(None);
                        region_menu_open.set(false);
                        return;
                    }
                    PositionRegion::NoCountry => {
                        "No country found for your location. Pick a region from the list."
                            .to_string()
                    }
                    PositionRegion::NoRegion(c) => {
                        format!("No Bat book region for {c}. Pick a region from the list.")
                    }
                },
            };
            locating.set(Some(Some(msg)));
            // The message is under the button at the end of a scrolling menu;
            // scroll it into view once it has been drawn.
            request_animation_frame(|| {
                if let Some(el) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.query_selector(".bat-book-region-error").ok().flatten())
                {
                    el.scroll_into_view_with_bool(false);
                }
            });
        });
    };

    view! {
        <div class="bat-book-region-menu" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
            // ── The recording's own place ────────────
            {move || recording().map(|(region, place)| view! {
                <div class="bat-book-region-recording">
                    <div>{format!("This recording: {}", region.label())}</div>
                    <div class="bat-book-region-recording-src">{place.detail(region)}</div>
                </div>
                <div class="bat-book-region-separator"></div>
                <div class="bat-book-region-heading">"For recordings without location data"</div>
            })}

            // ── Auto option ──────────────────────────
            <button
                class=move || if is_auto_active() { "bat-book-region-opt active" } else { "bat-book-region-opt" }
                on:click=select_auto
            >
                <span class="bat-book-star active">{"\u{2605}"}</span>
                "Auto"
            </button>

            // ── Favourites section ───────────────────
            {move || {
                let favs = state.bat_book.favourites().get();
                if favs.is_empty() {
                    return Vec::new();
                }
                let mut items: Vec<leptos::tachys::view::any_view::AnyView> = Vec::new();
                items.push(view! { <div class="bat-book-region-separator"></div> }.into_any());
                for &r in &favs {
                    items.push(view! { <RegionOption region=r is_favourite=true region_menu_open=region_menu_open /> }.into_any());
                }
                items
            }}

            // ── Separator ────────────────────────────
            <div class="bat-book-region-separator"></div>

            // ── All regions ──────────────────────────
            {BatBookRegion::ALL.iter().map(|&r| {
                view! { <RegionOption region=r is_favourite=false region_menu_open=region_menu_open /> }
            }).collect_view()}

            // ── This device's location ───────────────
            <div class="bat-book-region-separator"></div>
            <button
                class="bat-book-region-opt bat-book-region-locate"
                title="Your location stays on this device and is not saved."
                disabled=move || locating.get() == Some(None)
                on:click=on_locate
            >
                <Icon kind=Icon::MapPin />
                {move || if locating.get() == Some(None) {
                    "Getting location\u{2026}"
                } else {
                    "Set region from my location"
                }}
            </button>
            {move || locating.get().flatten().map(|msg| view! {
                <div class="bat-book-region-error" role="alert">{msg}</div>
            })}
        </div>
    }
}

/// A single region option in the dropdown, with star toggle and selection.
#[component]
fn RegionOption(
    region: BatBookRegion,
    /// If true, this is rendered in the favourites section (star already filled).
    is_favourite: bool,
    region_menu_open: RwSignal<bool>,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let r = region;

    let is_active = move || {
        match state.bat_book.mode().get() {
            BatBookMode::Manual(mr) => mr == r,
            BatBookMode::Auto => {
                // In auto mode, highlight the resolved region, unless it came
                // from the recording's place: these options are for
                // recordings without one.
                state
                    .bat_book
                    .auto_resolved()
                    .get()
                    .is_some_and(|res| res.region == r && res.recording_place.is_none())
            }
        }
    };

    let is_fav = move || {
        if is_favourite {
            true // already in fav section
        } else {
            state.bat_book.favourites().get().contains(&r)
        }
    };

    let on_star = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        state.bat_book.favourites().update(|favs| {
            if let Some(pos) = favs.iter().position(|&f| f == r) {
                favs.remove(pos);
            } else {
                favs.push(r);
            }
        });
        persist_favourites(&state.bat_book.favourites().get_untracked());
    };

    let on_select = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let mode = BatBookMode::Manual(r);
        state.bat_book.mode().set(mode);
        persist_mode(&mode);
        region_menu_open.set(false);
    };

    view! {
        <button
            class=move || if is_active() { "bat-book-region-opt active" } else { "bat-book-region-opt" }
            on:click=on_select
        >
            <span
                class=move || if is_fav() { "bat-book-star active" } else { "bat-book-star" }
                on:click=on_star
            >
                {move || if is_fav() { "\u{2605}" } else { "\u{2606}" }}
            </span>
            {r.label()}
        </button>
    }
}

/// Compute the combined BandFF range from all selected entries.
/// Returns (lo, hi) or None if nothing selected.
fn combined_band_ff_range(state: &AppState) -> Option<(f64, f64)> {
    let ids = state.bat_book.selected_ids().get_untracked();
    if ids.is_empty() {
        return None;
    }
    let region = state.bat_book.region().get_untracked();
    let manifest = get_manifest(region);

    // Also check auto-matched entry (may be from a different book)
    let auto_entry = state
        .bat_book
        .auto_resolved()
        .get_untracked()
        .and_then(|res| res.matched_species_id)
        .and_then(|id| auto_resolve::find_entry_any_book(&id));

    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for entry in &manifest.entries {
        if ids.iter().any(|id| id == entry.id) {
            lo = lo.min(entry.freq_lo_hz);
            hi = hi.max(entry.freq_hi_hz);
        }
    }
    // Include auto-matched entry if it's selected
    if let Some(ref entry) = auto_entry {
        if ids.iter().any(|id| id == entry.id) {
            lo = lo.min(entry.freq_lo_hz);
            hi = hi.max(entry.freq_hi_hz);
        }
    }
    if lo < hi {
        Some((lo, hi))
    } else {
        None
    }
}

/// Apply the combined BandFF range from selected bat book entries.
/// Shows toasts for out-of-range conditions.
/// Uses the focus stack to push/update the BatBook override layer.
fn apply_bat_book_ff(state: &AppState) {
    let Some((lo, hi)) = combined_band_ff_range(state) else {
        state.pop_bat_book_ff();
        return;
    };

    let files = state.library.files().get_untracked();
    let Some(idx) = state.library.current_index().get_untracked() else {
        return;
    };
    let Some(file) = files.get(idx) else { return };
    let nyquist = file.audio.sample_rate as f64 / 2.0;

    if lo >= nyquist {
        state.show_info_toast(format!(
            "Frequency range {}\u{2013}{} kHz is above this file's Nyquist ({} kHz)",
            (lo / 1000.0) as u32,
            (hi / 1000.0) as u32,
            (nyquist / 1000.0) as u32,
        ));
        return;
    }

    let clamped_hi = hi.min(nyquist);
    if clamped_hi < hi {
        state.show_info_toast(format!(
            "Frequency range clamped to {}\u{2013}{} kHz (file Nyquist: {} kHz)",
            (lo / 1000.0) as u32,
            (clamped_hi / 1000.0) as u32,
            (nyquist / 1000.0) as u32,
        ));
    }

    state.push_bat_book_ff(lo, clamped_hi);
}

#[component]
fn BatBookChip(
    entry: BatBookEntry,
    /// Whether this chip is the auto-matched species (shown before the divider).
    #[prop(default = false)]
    is_auto_matched: bool,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let entry_id = entry.id.to_string();
    let entry_id_for_click = entry_id.clone();
    let name = entry.name;
    let freq_label = entry.freq_range_label();
    let call_type = entry.call_type;

    let is_selected = {
        let eid = entry_id.clone();
        move || {
            state
                .bat_book
                .selected_ids()
                .get()
                .iter()
                .any(|id| id == &eid)
        }
    };

    let on_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let ctrl = ev.ctrl_key() || ev.meta_key();
        let shift = ev.shift_key();
        let eid = entry_id_for_click.clone();

        let was_selected = state
            .bat_book
            .selected_ids()
            .get_untracked()
            .iter()
            .any(|id| id == &eid);

        if was_selected && !ctrl && !shift {
            // Click selected bat again: deselect and restore previous BandFF
            state.bat_book.selected_ids().set(Vec::new());
            state.bat_book.ref_open().set(false);
            state.bat_book.last_clicked_id().set(None);
            state.pop_bat_book_ff();
            return;
        }

        if ctrl && was_selected {
            state
                .bat_book
                .selected_ids()
                .update(|ids| ids.retain(|id| id != &eid));
            if state.bat_book.selected_ids().get_untracked().is_empty() {
                state.bat_book.ref_open().set(false);
                state.bat_book.last_clicked_id().set(None);
                state.pop_bat_book_ff();
            } else if state.bat_book.auto_focus().get_untracked() {
                apply_bat_book_ff(&state);
            }
            state.bat_book.last_clicked_id().set(Some(eid));
            return;
        }

        if shift {
            let region = state.bat_book.region().get_untracked();
            let manifest = get_manifest(region);
            let last_id = state.bat_book.last_clicked_id().get_untracked();
            let anchor = last_id.as_deref().unwrap_or("");
            let anchor_idx = manifest.entries.iter().position(|e| e.id == anchor);
            let click_idx = manifest.entries.iter().position(|e| e.id == eid.as_str());

            if let (Some(a), Some(b)) = (anchor_idx, click_idx) {
                let lo = a.min(b);
                let hi = a.max(b);
                let range_ids: Vec<String> = manifest.entries[lo..=hi]
                    .iter()
                    .map(|e| e.id.to_string())
                    .collect();
                if ctrl {
                    state.bat_book.selected_ids().update(|ids| {
                        for rid in &range_ids {
                            if !ids.iter().any(|id| id == rid) {
                                ids.push(rid.clone());
                            }
                        }
                    });
                } else {
                    state.bat_book.selected_ids().set(range_ids);
                }
            } else {
                state.bat_book.selected_ids().set(vec![eid.clone()]);
            }
        } else if ctrl {
            state.bat_book.selected_ids().update(|ids| {
                if !ids.iter().any(|id| id == &eid) {
                    ids.push(eid.clone());
                }
            });
        } else {
            state.bat_book.selected_ids().set(vec![eid.clone()]);
        }

        state.bat_book.last_clicked_id().set(Some(eid));
        state.bat_book.ref_open().set(true);
        if state.bat_book.auto_focus().get_untracked() {
            apply_bat_book_ff(&state);
        }
    };

    let class = move || {
        let mut cls = if is_selected() {
            use crate::focus_stack::FocusSource;
            if state
                .viewmode
                .focus_stack()
                .get()
                .is_adopted(FocusSource::BatBook)
            {
                "bat-book-chip selected adopted".to_string()
            } else {
                "bat-book-chip selected".to_string()
            }
        } else {
            "bat-book-chip".to_string()
        };
        if is_auto_matched {
            cls.push_str(" auto-matched");
        }
        cls
    };

    view! {
        <button class=class on:click=on_click>
            <span class="bat-book-chip-name">{name}</span>
            <span class="bat-book-chip-freq">{freq_label} " " {call_type}</span>
        </button>
    }
}

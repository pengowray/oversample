use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::state::AppState;
use super::loading::{DemoDetails, DemoEntry, fetch_demo_details, fetch_demo_index, load_single_demo};

const PICK_COUNT: usize = 3;

#[derive(Clone)]
struct Suggestion {
    entry: DemoEntry,
    details: RwSignal<Option<DemoDetails>>,
}

fn format_max_freq(sample_rate_hz: u64) -> String {
    let khz = sample_rate_hz as f64 / 2000.0;
    if khz >= 100.0 {
        format!("{khz:.0} kHz")
    } else if khz >= 10.0 {
        format!("{khz:.0} kHz")
    } else {
        format!("{khz:.1} kHz")
    }
}

fn format_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{secs:.0}s")
    } else {
        let mins = (secs / 60.0).floor() as u32;
        let rem = (secs - mins as f64 * 60.0).round() as u32;
        if rem == 0 {
            format!("{mins}m")
        } else {
            format!("{mins}m{rem:02}s")
        }
    }
}

fn display_name(entry: &DemoEntry) -> String {
    entry.en.clone().unwrap_or_else(|| {
        entry.filename
            .trim_end_matches(".wav")
            .trim_end_matches(".w4v")
            .trim_end_matches(".flac")
            .trim_end_matches(".mp3")
            .to_string()
    })
}

/// Pick `count` unique random bat entries from the pool, sorted alphabetically by display name.
fn pick_suggestions(pool: &[DemoEntry], count: usize) -> Vec<DemoEntry> {
    let bats: Vec<&DemoEntry> = pool.iter().filter(|e| e.is_bat()).collect();
    if bats.is_empty() {
        return Vec::new();
    }
    let n = count.min(bats.len());
    let mut chosen_indices: Vec<usize> = Vec::with_capacity(n);
    let mut attempts = 0;
    while chosen_indices.len() < n && attempts < n * 20 {
        let idx = (js_sys::Math::random() * bats.len() as f64) as usize;
        let idx = idx.min(bats.len() - 1);
        if !chosen_indices.contains(&idx) {
            chosen_indices.push(idx);
        }
        attempts += 1;
    }
    let mut picked: Vec<DemoEntry> = chosen_indices.into_iter().map(|i| bats[i].clone()).collect();
    picked.sort_by(|a, b| display_name(a).to_lowercase().cmp(&display_name(b).to_lowercase()));
    picked
}

fn refresh_suggestions(
    pool: &[DemoEntry],
    suggestions: RwSignal<Vec<Suggestion>>,
) {
    let picks = pick_suggestions(pool, PICK_COUNT);
    let new_suggestions: Vec<Suggestion> = picks
        .into_iter()
        .map(|entry| Suggestion {
            entry,
            details: RwSignal::new(None),
        })
        .collect();

    // Kick off metadata fetches for each suggestion in the background.
    for s in &new_suggestions {
        let Some(meta_file) = s.entry.metadata_file.clone() else {
            s.details.set(Some(DemoDetails::default()));
            continue;
        };
        let signal = s.details;
        spawn_local(async move {
            let details = fetch_demo_details(&meta_file).await;
            signal.set(Some(details));
        });
    }

    suggestions.set(new_suggestions);
}

#[component]
pub(super) fn BatsForYou(
    demo_entries: RwSignal<Vec<DemoEntry>>,
    expanded: RwSignal<bool>,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let suggestions: RwSignal<Vec<Suggestion>> = RwSignal::new(Vec::new());
    let index_loading = RwSignal::new(false);

    // Kick off index fetch on mount if not already loaded.
    // This does not block the rest of the panel from rendering.
    Effect::new(move |_| {
        if !demo_entries.get_untracked().is_empty() {
            if suggestions.get_untracked().is_empty() {
                refresh_suggestions(&demo_entries.get_untracked(), suggestions);
            }
            return;
        }
        if index_loading.get_untracked() {
            return;
        }
        index_loading.set(true);
        spawn_local(async move {
            match fetch_demo_index().await {
                Ok(entries) => {
                    demo_entries.set(entries.clone());
                    refresh_suggestions(&entries, suggestions);
                }
                Err(e) => log::warn!("Failed to fetch demo index for suggestions: {e}"),
            }
            index_loading.set(false);
        });
    });

    let on_refresh = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let pool = demo_entries.get_untracked();
        if pool.is_empty() {
            return;
        }
        refresh_suggestions(&pool, suggestions);
    };

    let on_toggle = move |_: web_sys::MouseEvent| {
        expanded.update(|v| *v = !*v);
    };

    view! {
        <div class="bats-for-you">
            <div class="bats-for-you-header" on:click=on_toggle>
                <span class="bats-for-you-caret">
                    {move || if expanded.get() { "\u{25BE}" } else { "\u{25B8}" }}
                </span>
                <span class="bats-for-you-title">"Bats For You"</span>
                <button
                    class="bats-for-you-refresh"
                    title="Shuffle suggestions"
                    on:click=on_refresh
                >
                    "\u{21BB}"
                </button>
            </div>
            {move || expanded.get().then(|| {
                let items = suggestions.get();
                if items.is_empty() {
                    return view! {
                        <div class="bats-for-you-grid">
                            <div class="bats-for-you-empty">
                                {move || if index_loading.get() { "Loading suggestions\u{2026}" } else { " " }}
                            </div>
                        </div>
                    }.into_any();
                }
                let cards: Vec<_> = items.into_iter().map(|s| {
                    let entry_for_click = s.entry.clone();
                    let entry_for_title = s.entry.clone();
                    let details_signal = s.details;
                    let name = display_name(&entry_for_title);
                    let species = entry_for_title.species.clone();
                    let on_click = move |_: web_sys::MouseEvent| {
                        let entry = entry_for_click.clone();
                        let label = entry.en.clone().unwrap_or_else(|| entry.filename.clone());
                        let load_id = state.loading_start(&label);
                        spawn_local(async move {
                            if let Err(e) = load_single_demo(&entry, state, load_id).await {
                                log::error!("Failed to load suggested demo: {e}");
                            }
                            state.loading_done(load_id);
                        });
                    };
                    view! {
                        <button class="bats-for-you-card" on:click=on_click title=entry_for_title.filename.clone()>
                            <div class="bats-for-you-card-name">{name}</div>
                            {species.map(|sp| view! {
                                <div class="bats-for-you-card-species">{sp}</div>
                            })}
                            <div class="bats-for-you-card-details">
                                {move || {
                                    match details_signal.get() {
                                        None => view! { <span class="bats-for-you-card-placeholder">"\u{2026}"</span> }.into_any(),
                                        Some(d) => {
                                            let len = d.duration_secs.map(format_duration);
                                            let freq = d.sample_rate_hz.map(format_max_freq);
                                            let parts: Vec<String> = [len, freq].into_iter().flatten().collect();
                                            let text = if parts.is_empty() {
                                                "\u{00A0}".to_string()
                                            } else {
                                                parts.join(" \u{00B7} ")
                                            };
                                            view! { <span>{text}</span> }.into_any()
                                        }
                                    }
                                }}
                            </div>
                        </button>
                    }
                }).collect();
                view! {
                    <div class="bats-for-you-grid">
                        {cards}
                    </div>
                }.into_any()
            })}
        </div>
    }
}

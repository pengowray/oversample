//! Settings of the classifier section that take more than one line: the
//! Step buttons, and the Place row with "Mark birds unlikely here". Also the
//! place and month for the line of settings in force.

use super::rows::unlikely_flag;
use super::{count, event_target_checked};
use crate::classifier::{self, location, summary, Method};
use crate::recording_place as place;
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;

/// The Step setting: the time between the starts of windows for the next
/// Analyse, as a group of buttons like the number of Sound clusters. Saved
/// for every classifier. None for BattyBirdNET.
pub(super) fn step_row(method: Method, pick: RwSignal<summary::StepPick>) -> Option<impl IntoView> {
    let choices = summary::step_choices(method);
    if choices.is_empty() {
        return None;
    }
    let window = method.window_secs();
    let help = format!(
        "Time between the starts of windows. At 1 s a new {} window starts every second, so windows overlap. Applies to the next Analyse.",
        summary::step_text(window)
    );
    let buttons = choices
        .into_iter()
        .map(|(choice, secs)| {
            let on = move || pick.get().secs(method) == secs;
            let text = summary::step_text(secs);
            view! {
                <button
                    class=move || if on() { "psd-btn psd-btn-active" } else { "psd-btn" }
                    aria-pressed=move || on().to_string()
                    aria-label=format!("Step {text}")
                    on:click=move |_| {
                        pick.set(choice);
                        crate::settings::set_classifier_step(choice);
                    }
                >{text.clone()}</button>
            }
        })
        .collect::<Vec<_>>();
    Some(view! {
        <div class="setting-row detection-step-row" title=help>
            <span class="setting-label">"Step"</span>
            <div class="psd-btn-group" role="group" aria-label="Step">{buttons}</div>
        </div>
    })
}

/// How many of the species at or above the minimum confidence are birds
/// unlikely at the file's place and month. Reactive.
fn unlikely_species_count(state: AppState, file: u64, method: Method) -> usize {
    let min = classifier::min_confidence(state, method);
    state.classifier.results().with(|m| {
        m.get(&(file, method)).map_or(0, |run| {
            summary::summarise(run, min)
                .species
                .iter()
                .filter(|row| {
                    run.labels
                        .get(row.class as usize)
                        .is_some_and(|l| location::unlikely_here(state, file, method, l))
                })
                .count()
        })
    })
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The month of a location-list week (1 to 48), or `None` for any month (0).
fn month_of(week: u32) -> Option<&'static str> {
    (week > 0).then(|| MONTHS[((week - 1) / 4).min(11) as usize])
}

/// For the line of settings in force: the place and month birds are marked
/// unlikely for ("Antarctica in January", "Your location in June"), while
/// "Mark birds unlikely here" is on. Reactive.
pub(super) fn place_fact(state: AppState, method: Method, file: Option<u64>) -> Option<String> {
    if !location::applies_to(method) || !state.call_detection.mark_unexpected().get() {
        return None;
    }
    let file = file?;
    let (own, country, week) = state.library.files().with(|fs| {
        fs.iter().find(|f| f.id == file).map(|f| {
            (
                place::file_place(f),
                place::recording_place(f).map(|p| p.country),
                place::file_week(f),
            )
        })
    })?;
    let device = state.classifier.device_position().get();
    let where_ = match (own, device) {
        (Some(p), _) => country.unwrap_or_else(|| format!("{:.2}, {:.2}", p.lat, p.lon)),
        (None, Some(_)) => "Your location".to_string(),
        // No place: no bird is marked, so nothing about it is in force.
        (None, None) => return None,
    };
    Some(match month_of(week) {
        Some(m) => format!("{where_} in {m}"),
        None => format!("{where_}, any month"),
    })
}

/// The Place row and "Mark birds unlikely here", in the settings. The place
/// is the recording's own GPS, or else this device's position once the user
/// asks for it with "Use my location" (kept in memory for this session).
pub(super) fn place_rows(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
    locating: RwSignal<Option<Option<String>>>,
    has_results: Memo<bool>,
) -> Option<AnyView> {
    if !location::applies_to(method) {
        return None;
    }
    let file = current_file.get()?;
    let (own, week) = state.library.files().with(|fs| {
        fs.iter()
            .find(|f| f.id == file)
            .map(|f| (place::file_place(f), place::file_week(f)))
    })?;
    let device = state.classifier.device_position().get();
    let place = own.or_else(|| device.map(|(lat, lon)| place::Place { lat, lon, week }));
    let error = move || {
        locating
            .get()
            .flatten()
            .map(|msg| view! { <div class="setting-row setting-error" role="alert">{msg}</div> })
    };
    // Asks for this device's position: "Use my location", or "Update" once
    // it is known.
    let on_locate = move |_| {
        if locating.get_untracked() == Some(None) {
            return;
        }
        locating.set(Some(None));
        spawn_local(async move {
            use crate::device_location::{ask, LocationError};
            let msg = match ask(state).await {
                Ok(_) => None,
                Err(LocationError::Denied) => Some("Location permission denied".to_string()),
                Err(LocationError::Unavailable(m)) => Some(format!("Location unavailable: {m}")),
            };
            locating.set(msg.map(Some));
        });
    };
    // Beside the checkbox, so turning it on or off changes this one value
    // and moves nothing: blank when off or before there are results, then
    // "Checking..." until the location list is worked out, then how many
    // listed species are flagged.
    // Without a place there is no location list to wait for.
    let has_place = place.is_some();
    let marked = move || {
        if !state.call_detection.mark_unexpected().get() || !has_results.get() || !has_place {
            return String::new();
        }
        let checking = state
            .classifier
            .occurrence()
            .with(|m| m.get(&file).is_none_or(|o| o.scores.is_none()));
        if checking {
            "Checking\u{2026}".to_string()
        } else {
            count(
                unlikely_species_count(state, file, method),
                "species",
                "species",
            )
        }
    };
    let mark_row = view! {
        <div class="setting-row detection-annotate-row">
            <label class="setting-label">
                <input
                    type="checkbox"
                    prop:checked=move || state.call_detection.mark_unexpected().get()
                    on:change=move |ev| state.call_detection.mark_unexpected().set(event_target_checked(&ev))
                />
                " Mark birds unlikely here"
                {unlikely_flag()}
            </label>
            <span class="setting-value detection-marked-count">{marked}</span>
        </div>
    };
    let Some(place) = place else {
        return Some(
            view! {
                <div class="setting-row">
                    <span class="setting-label">"Place"</span>
                    <span class="setting-value">"No location data"</span>
                </div>
                <div class="setting-row">
                    <button
                        class="setting-button"
                        disabled=move || locating.get() == Some(None)
                        on:click=on_locate
                    >
                        {move || if locating.get() == Some(None) {
                            "Getting location\u{2026}"
                        } else {
                            "Use my location"
                        }}
                    </button>
                </div>
                {error}
                {mark_row}
            }
            .into_any(),
        );
    };
    let from_device = own.is_none();
    let month = month_of(place.week).unwrap_or("any month");
    let value = format!("{:.2}, {:.2} \u{b7} {month}", place.lat, place.lon);
    Some(
        view! {
            <div
                class="setting-row"
                title=if from_device { PLACE_HELP_DEVICE } else { PLACE_HELP_OWN }
            >
                <span class="setting-label">"Place"</span>
                <span class="setting-value">
                    {from_device.then(|| view! {
                        <span class="place-source">
                            "Your location "
                            <button
                                class="detection-link-btn"
                                title="Get this device's location again"
                                disabled=move || locating.get() == Some(None)
                                on:click=on_locate
                            >
                                {move || if locating.get() == Some(None) {
                                    "Getting location\u{2026}"
                                } else {
                                    "Update"
                                }}
                            </button>
                        </span>
                    })}
                    {value}
                </span>
            </div>
            {error}
            {mark_row}
        }
        .into_any(),
    )
}

const PLACE_HELP_OWN: &str = "From the recording's metadata. Birds the BirdNET Geomodel scores below 0.01 at this place and month are marked unlikely here. The place stays on this device.";
const PLACE_HELP_DEVICE: &str = "Your location, from this device, used for recordings without location data. Birds the BirdNET Geomodel scores below 0.01 at this place and month are marked unlikely here. Your location stays on this device and is not saved.";

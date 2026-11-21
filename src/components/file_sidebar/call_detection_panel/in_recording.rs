//! The "In this recording" section: one list of the species found, the
//! sound clusters and the picked components, each with a pin; the search
//! that adds a row; the selected window's top 3; and notes on the windows
//! left out.

use super::bird_boxes;
use super::classifier_section::RunInfo;
use super::count;
use super::parts::{pin_button, section_head};
use super::rows::{
    class_name, component_picked, flag_cell, flag_header, save_strips, show_window, species_pin,
    species_swatch, toggle_component,
};
use super::species_search::strip_search;
use crate::classifier::{self, location, summary, Method, MethodFacts};
use crate::components::icons::Icon;
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;

/// "16 windows", or when windows overlap (so a count would count the same
/// audio more than once) the time of their cells: "14 s of audio".
fn windows_or_time(s: &summary::Summary, windows: usize, secs: f64) -> String {
    if s.overlapping {
        format!("{} of audio", summary::time_amount(secs))
    } else {
        count(windows, "window", "windows")
    }
}

/// One species row of the "In this recording" list.
#[derive(Clone, PartialEq)]
struct SpeciesListRow {
    class: u16,
    name: String,
    scientific: String,
    label: Option<classifier::SpeciesLabel>,
    slot: Option<usize>,
    /// Windows, or the time when windows overlap.
    amount: String,
    best: f32,
    /// The best window, which a press on the row shows.
    start: f64,
    end: f64,
}

impl SpeciesListRow {
    /// Everything the row shows, so a row is drawn again only when it
    /// changes.
    fn key(&self) -> (u16, Option<usize>, String, u32, u64) {
        (
            self.class,
            self.slot,
            self.amount.clone(),
            self.best.to_bits(),
            self.start.to_bits(),
        )
    }
}

/// A species picked for a row above the spectrogram that is not among this
/// recording's species at or above the minimum confidence.
#[derive(Clone, PartialEq)]
struct MissingRow {
    scientific: String,
    name: String,
    label: Option<classifier::SpeciesLabel>,
    /// The model has the species among its classes.
    in_model: bool,
}

/// The "In this recording" section: the header with what was found, notes
/// on the results, the list (species, sound clusters, components) with a pin
/// on each row, the search that adds a row, the selected window's top 3, and
/// the counts of windows left out.
#[allow(clippy::too_many_arguments)]
pub(super) fn in_recording_section(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
    summary: Memo<Option<summary::Summary>>,
    results_info: Memo<Option<RunInfo>>,
    has_results: Memo<bool>,
    bat_scope: Memo<classifier::BatScope>,
    step_pick: RwSignal<summary::StepPick>,
) -> impl IntoView {
    let min_conf = move || classifier::min_confidence(state, method);
    // "2 species, 1 sound, 16 windows", or "No species".
    let head_fact = move || {
        let s = summary.get()?;
        if s.species.is_empty() {
            return Some("No species".to_string());
        }
        // Named sounds (Thunder, Dog) are listed with the species but
        // counted apart, so "3 species" never includes Thunder.
        let sounds = state.classifier.results().with(|m| {
            current_file
                .get()
                .and_then(|f| m.get(&(f, method)))
                .map_or(0, |r| {
                    s.species
                        .iter()
                        .filter(|row| r.labels.get(row.class as usize).is_some_and(|l| l.is_sound))
                        .count()
                })
        });
        let animals = s.species.len() - sounds;
        let mut parts = Vec::new();
        if animals > 0 {
            parts.push(count(animals, "species", "species"));
        }
        if sounds > 0 {
            parts.push(count(sounds, "sound", "sounds"));
        }
        parts.push(windows_or_time(&s, s.species_windows, s.species_secs));
        Some(parts.join(", "))
    };
    let head_title = move || format!("At confidence {:.2} or above", min_conf());

    // Notes on what the results are: for another BattyBirdNET region (a
    // relabel is going or failed), or made with another step than the one
    // picked now.
    let results_notes = move || {
        let r = results_info.get()?;
        if r.analysed == 0 {
            return None;
        }
        if let Some(region) = r.region.filter(|g| *g != bat_scope.get()) {
            return Some(format!("Results for {}", region.label()));
        }
        if method == Method::BirdNetV3 {
            if let Some(note) = bird_boxes::analyse_again_note(state, current_file.get()) {
                return Some(note.to_string());
            }
        }
        let picked = step_pick.get().secs(method);
        let differs = !summary::step_choices(method).is_empty()
            && (r.spacing_secs - picked).abs() > 1e-6
            && r.complete;
        differs.then(|| {
            format!(
                "Results use a {} step. Analyse again for a {} step.",
                summary::step_text(r.spacing_secs),
                summary::step_text(picked)
            )
        })
    };

    let flag_col = location::applies_to(method);
    // Columns before the pin: name, the flag, time (or windows), best.
    let cols = 3 + usize::from(flag_col);

    // Species rows, worked out in a memo and drawn by key, so readings that
    // change no row (several a second while a file plays) redraw nothing.
    let species_rows = Memo::new(move |_| {
        let min = min_conf();
        state.classifier.results().with(|m| {
            let Some((file, run)) = current_file
                .get()
                .and_then(|f| m.get(&(f, method)).map(|r| (f, r)))
            else {
                return (false, Vec::new());
            };
            let order = classifier::kept_order::colour_order(file, method, run, min);
            let s = summary::summarise(run, min);
            let rows = s
                .species
                .iter()
                .map(|row| {
                    let label = run.labels.get(row.class as usize).cloned();
                    let w = &run.windows[row.best_window];
                    SpeciesListRow {
                        class: row.class,
                        name: class_name(label.as_ref()),
                        scientific: label
                            .as_ref()
                            .map(|l| l.scientific.clone())
                            .unwrap_or_default(),
                        label,
                        slot: order.iter().position(|&c| c == row.class),
                        // Windows that overlap would count the same audio twice.
                        amount: if s.overlapping {
                            summary::time_amount(row.secs)
                        } else {
                            row.windows.to_string()
                        },
                        best: row.best,
                        start: w.start_s,
                        end: w.end_s,
                    }
                })
                .collect::<Vec<_>>();
            (s.overlapping, rows)
        })
    });
    let overlapping = move || species_rows.with(|r| r.0);
    let species_row = move |r: SpeciesListRow| {
        let (start, end) = (r.start, r.end);
        let class = move || {
            if state.call_detection.selected_window().get() == Some(start) {
                "detection-row selected"
            } else {
                "detection-row"
            }
        };
        let flag = flag_cell(
            state,
            method,
            current_file.get_untracked(),
            r.label.as_ref(),
        );
        view! {
            <tr class=class title=r.scientific.clone() on:click=move |_| show_window(state, start, end)>
                <td class="detection-col-kind">
                    <span class="cd-name-line">{species_swatch(r.slot)}<span>{r.name.clone()}</span></span>
                </td>
                {flag}
                <td class="detection-col-num">{r.amount.clone()}</td>
                <td class="detection-col-num">{format!("{:.2}", r.best)}</td>
                {species_pin(state, method, r.scientific.clone(), r.name.clone())}
            </tr>
        }
    };

    // Picked species with no row above: still listed, so they can be
    // unpinned.
    let missing_rows = Memo::new(move |_| {
        let picked: Vec<String> = state.call_detection.strip_species().with(|v| {
            v.iter()
                .filter(|s| s.method == method)
                .map(|s| s.scientific.clone())
                .collect()
        });
        if picked.is_empty() {
            return Vec::new();
        }
        let listed: Vec<String> =
            species_rows.with(|r| r.1.iter().map(|r| r.scientific.clone()).collect());
        state.classifier.results().with(|m| {
            let run = current_file.get().and_then(|f| m.get(&(f, method)));
            picked
                .into_iter()
                .filter(|s| !listed.contains(s))
                .map(|scientific| {
                    let label = run.and_then(|run| {
                        classifier::strips::class_of(run, &scientific)
                            .and_then(|c| run.labels.get(c as usize).cloned())
                    });
                    MissingRow {
                        name: label
                            .as_ref()
                            .map_or_else(|| scientific.clone(), classifier::strips::display_name),
                        // Without results the model's list is not known here.
                        in_model: label.is_some() || run.is_none(),
                        label,
                        scientific,
                    }
                })
                .collect()
        })
    });
    let missing_row = move |r: MissingRow| {
        let flag = flag_cell(
            state,
            method,
            current_file.get_untracked(),
            r.label.as_ref(),
        );
        let cells = if r.in_model {
            view! {
                <td class="detection-col-num cd-missing">"\u{2013}"</td>
                <td class="detection-col-num cd-missing" title="Highest confidence in this recording is below the minimum">
                    {move || format!("< {:.2}", min_conf())}
                </td>
            }
            .into_any()
        } else {
            view! {
                <td class="cd-missing" colspan="2" title="This model's list of classes does not have this species">
                    "Not in this model"
                </td>
            }
            .into_any()
        };
        view! {
            <tr class="detection-row-other" title=r.scientific.clone()>
                <td class="detection-col-kind">
                    <span class="cd-name-line">{species_swatch(None)}<span>{r.name.clone()}</span></span>
                </td>
                {flag}
                {cells}
                {species_pin(state, method, r.scientific.clone(), r.name.clone())}
            </tr>
        }
    };

    // The model's components picked for a row above the spectrogram.
    let component_rows = move || {
        if !method.info().component_rows {
            return Vec::new();
        }
        let picked = state.call_detection.strip_components().get();
        let shares = crate::components::score_strips::components_for(method)
            .map(|p| p.variance.clone())
            .unwrap_or_default();
        picked
            .into_iter()
            .map(|k| {
                let name = format!("PC{}", k + 1);
                let help = shares.get(k).map_or_else(String::new, |s| {
                    format!("{name}: {:.1}% of the variation between the model's classes", s * 100.0)
                });
                view! {
                    <tr title=help>
                        <td class="detection-col-kind" colspan=cols.to_string()>
                            <span class="cd-name-line">
                                <span class="map-key cd-swatch cd-swatch-pc"></span>
                                <span>{name.clone()}</span>
                            </span>
                        </td>
                        <td class="cd-col-pin">
                            {pin_button(name, move || component_picked(state, k), move || toggle_component(state, k))}
                        </td>
                    </tr>
                }
            })
            .collect::<Vec<_>>()
    };

    let clusters_pin = pin_button(
        "Sound clusters".to_string(),
        move || state.call_detection.sound_groups().get(),
        move || {
            state.call_detection.sound_groups().update(|v| *v = !*v);
            save_strips(state);
        },
    )
    .into_any();

    // The windows left out of the list, and why nothing is listed.
    let notes = move || {
        let s = summary.get()?;
        let min = min_conf();
        let mut notes = Vec::new();
        if s.species.is_empty() {
            notes.push(match s.highest {
                Some((class, conf)) => {
                    let name = state.classifier.results().with(|m| {
                        current_file
                            .get()
                            .and_then(|f| m.get(&(f, method)))
                            .map(|r| class_name(r.labels.get(class as usize)))
                            .unwrap_or_default()
                    });
                    format!(
                        "No species at confidence {min:.2} or above. Highest: {name}, {conf:.2}"
                    )
                }
                None => format!("No species at confidence {min:.2} or above."),
            });
        }
        if s.below_windows > 0 {
            notes.push(format!(
                "Below {min:.2}: {}",
                windows_or_time(&s, s.below_windows, s.below_secs)
            ));
        }
        if s.not_species_windows > 0 {
            notes.push(format!(
                "Noise and other sounds: {}",
                windows_or_time(&s, s.not_species_windows, s.not_species_secs)
            ));
        }
        (!notes.is_empty()).then(|| {
            view! {
                <div class="cd-notes">
                    {notes.into_iter().map(|t| view! { <div class="detection-status-note">{t}</div> }).collect::<Vec<_>>()}
                </div>
            }
        })
    };

    view! {
        <div class="cd-section cd-recording">
            {section_head("In this recording", move || head_fact(), head_title)}
            {move || results_notes().map(|t| view! { <div class="cd-notes"><div class="detection-status-note">{t}</div></div> })}
            {move || (!has_results.get()).then(|| view! {
                <div class="cd-notes"><div class="detection-status-note">"Analyse to list the species found in this recording."</div></div>
            })}
            // Hidden, not removed, before there are results: a row above the
            // spectrogram needs a run, so there is nothing to pin yet.
            <table class="detection-table cd-list" style:display=move || if has_results.get() { "" } else { "none" }>
                <thead>
                    <tr>
                        <th></th>
                        {flag_header(method)}
                        <th class="detection-col-num">{move || if overlapping() { "Time" } else { "Windows" }}</th>
                        <th class="detection-col-num" title="Best confidence">"Best"</th>
                        <th class="cd-col-pin" title="Shown above the spectrogram">
                            <span class="cd-pin-head" aria-label="Shown above the spectrogram"><Icon kind=Icon::Pin /></span>
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {(method == Method::BirdNetV3).then(|| bird_boxes::list_row(state, current_file, cols))}
                    <For each=move || species_rows.with(|r| r.1.clone()) key=SpeciesListRow::key children=species_row />
                    <For each=move || missing_rows.get() key=|r: &MissingRow| (r.scientific.clone(), r.in_model) children=missing_row />
                    {crate::components::sound_groups::list_rows(state, method, current_file, save_strips, cols, clusters_pin)}
                    {component_rows}
                </tbody>
            </table>
            {strip_search(state, method, current_file)}
            {top_three(state, method, current_file)}
            {notes}
        </div>
    }
}

/// The selected window's top 3 classes, as the data they are drawn from, so
/// the table is drawn again only when it changes (not on every reading).
#[derive(Clone, PartialEq)]
struct TopThree {
    /// "Top 3 in window 4 (9.0s to 12.0s)"
    title: String,
    rows: Vec<(Option<classifier::SpeciesLabel>, f32)>,
}

/// The selected window's top 3 classes, under the list.
fn top_three(state: AppState, method: Method, current_file: Memo<Option<u64>>) -> impl IntoView {
    let data = Memo::new(move |_| {
        let start = state.call_detection.selected_window().get()?;
        state.classifier.results().with(|m| {
            let run = current_file.get().and_then(|f| m.get(&(f, method)))?;
            let i = run.window_at_start(start)?;
            let w = run.windows.get(i)?;
            let (a, b) = (
                crate::format_time::format_time_display(w.start_s, 1),
                crate::format_time::format_time_display(w.end_s, 1),
            );
            // With overlapping windows the numbers change as a run adds
            // windows between others, so only the times are given.
            let title = if run.overlapping() {
                format!("Top 3 in window {a} to {b}")
            } else {
                format!("Top 3 in window {} ({a} to {b})", i + 1)
            };
            Some(TopThree {
                title,
                rows: w
                    .top
                    .iter()
                    .take(3)
                    .map(|s| (run.label(s).cloned(), s.confidence))
                    .collect(),
            })
        })
    });
    move || {
        let t = data.get()?;
        let rows = t
            .rows
            .into_iter()
            .map(|(label, conf)| {
                let scientific = label
                    .as_ref()
                    .filter(|l| l.is_species && l.scientific != l.common)
                    .map(|l| l.scientific.clone());
                let flag = flag_cell(state, method, current_file.get_untracked(), label.as_ref());
                // BattyBirdNET's "Noise" and the like get no row above.
                let pin = match label.as_ref().filter(|l| l.is_species) {
                    Some(l) => {
                        species_pin(state, method, l.scientific.clone(), class_name(Some(l)))
                            .into_any()
                    }
                    None => view! { <td class="cd-col-pin"></td> }.into_any(),
                };
                view! {
                    <tr>
                        <td class="detection-col-kind">
                            {class_name(label.as_ref())}
                            {scientific.map(|s| view! { <div class="detection-scientific">{s}</div> })}
                        </td>
                        {flag}
                        <td class="detection-col-num">{format!("{conf:.2}")}</td>
                        {pin}
                    </tr>
                }
            })
            .collect::<Vec<_>>();
        Some(view! {
            <div class="detection-top3">
                <div class="detection-status-note">{t.title}</div>
                <table class="detection-table">
                    <tbody>{rows}</tbody>
                </table>
            </div>
        })
    }
}

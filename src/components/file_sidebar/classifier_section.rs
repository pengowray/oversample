//! The BirdNET and BattyBirdNET part of the Call detection tab: download the
//! model, analyse the current file, list the species found, and add them as
//! annotations. The calls it makes are in `crate::classifier`.
//!
//! Order on screen: the action (download, analyse, progress), the results
//! (minimum confidence, summary, list, the selected window's top 3), adding
//! annotations, then the stored model and the attribution BirdNET's licence
//! asks for.

use super::call_detection_panel::{count, event_target_checked, event_target_value, undo_keys};
use crate::annotations::{now_iso8601, AnnotationId, AnnotationSet};
use crate::call_annotations::remove_annotations_where;
use crate::classifier::summary::{self, WindowKind};
use crate::classifier::{
    self, catalog, ClassifierError, DownloadStatus, Method, ModelChoice, RunStatus,
};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListView {
    BySpecies,
    ByWindow,
}

const MB: f64 = 1_000_000.0;

/// `67 MB`, or `0.1 MB` for a small file.
fn mb(bytes: u64) -> String {
    let m = bytes as f64 / MB;
    if m >= 10.0 {
        format!("{m:.0} MB")
    } else {
        format!("{:.1} MB", m.max(0.1))
    }
}

/// `1,200`
fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `3 s`, `0.56 s`
fn window_len(secs: f64) -> String {
    if secs >= 1.0 {
        format!("{secs:.0} s")
    } else {
        format!("{secs:.2} s")
    }
}

fn method_name(method: Method) -> &'static str {
    match method {
        Method::BirdNet => "BirdNET",
        Method::Bats => "BattyBirdNET",
    }
}

/// A class name as shown: BattyBirdNET's lower-case classes get a capital.
fn class_name(label: Option<&classifier::SpeciesLabel>) -> String {
    let Some(l) = label else {
        return "Unknown".to_string();
    };
    let mut c = l.common.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Scroll the spectrogram so window `i`'s middle is in the centre, and pick it.
fn show_window(state: AppState, start_s: f64, end_s: f64, i: usize) {
    state.call_detection.selected_window().set(Some(i));
    let mid = 0.5 * (start_s + end_s);
    let files = state.library.files().get_untracked();
    let idx = state.library.current_index().get_untracked();
    if let Some(file) = idx.and_then(|i| files.get(i)) {
        let canvas_w = state.viewmode.spectrogram_canvas_width().get_untracked();
        let zoom = state.view.zoom_level().get_untracked();
        let visible_time = (canvas_w / zoom) * file.spectrogram.time_resolution;
        state
            .view
            .scroll_offset()
            .set((mid - visible_time / 2.0).max(0.0));
    }
}

/// Download what `choice` still needs (a region's classifier is about 0.1 MB),
/// then analyse the current file.
fn analyse(state: AppState, file_id: u64, choice: ModelChoice) {
    state.call_detection.overlay_enabled().set(true);
    state.call_detection.selected_window().set(None);
    if classifier::is_ready(state, choice) {
        classifier::run(state, file_id, choice);
        return;
    }
    let method = choice.method();
    state
        .classifier
        .run()
        .set(RunStatus::LoadingModel { file_id, method });
    spawn_local(async move {
        match classifier::ensure_downloaded(state, choice).await {
            Ok(()) => classifier::run(state, file_id, choice),
            Err(error) => state.classifier.run().set(RunStatus::Failed {
                file_id,
                method,
                error,
            }),
        }
    });
}

/// Add the windows at or above the minimum confidence as annotations,
/// replacing the ones this method (and region) added before.
fn add_annotations(state: AppState, file_id: u64, method: Method, merge: bool) {
    let min = state.call_detection.min_confidence().get_untracked();
    let now = now_iso8601();
    let built = state.classifier.results().with_untracked(|m| {
        m.get(&(file_id, method)).map(|run| {
            let spans = summary::spans(run, min, merge);
            (
                summary::build_annotations(run, &spans, &now, AnnotationId::new),
                summary::auto_tag_value(run),
            )
        })
    });
    let Some((annotations, tag)) = built else {
        return;
    };
    if annotations.is_empty() {
        return;
    }
    let Some(idx) = state.library.current_index().get_untracked() else {
        return;
    };
    let new_set = state.library.files().with_untracked(|files| {
        files.get(idx).map(|f| {
            let id = f.identity.clone().unwrap_or_else(|| {
                crate::file_identity::identity_layer1(&f.name, f.audio.metadata.file_size as u64)
            });
            AnnotationSet::new_with_metadata(id, &f.audio, f.cached_peak_db, f.cached_full_peak_db)
        })
    });
    let Some(new_set) = new_set else {
        return;
    };
    let added = annotations.len();
    state.snapshot_annotations();
    let mut removed = 0;
    let mut remaining_ids: Vec<AnnotationId> = Vec::new();
    state.annotations.store().update(|store| {
        let set = store.entry_or_insert_with(file_id, || new_set);
        removed =
            remove_annotations_where(&mut set.annotations, |a| summary::has_auto_tag(a, &tag));
        set.annotations.extend(annotations);
        remaining_ids = set.annotations.iter().map(|a| a.id.clone()).collect();
    });
    if removed > 0 {
        state
            .annotations
            .selected_ids()
            .update(|ids| ids.retain(|id| remaining_ids.contains(id)));
    }
    state.annotations.dirty().set(true);
    state.annotations.visible().set(true);
    let mut toast = format!("{} added", count(added, "annotation", "annotations"));
    if removed > 0 {
        toast.push_str(&format!(
            ". {} removed.",
            count(removed, "previous annotation", "previous annotations")
        ));
    }
    state.show_info_toast(toast);
}

#[component]
pub(super) fn ClassifierSection(bats: bool) -> impl IntoView {
    let state = expect_context::<AppState>();
    let method = if bats { Method::Bats } else { Method::BirdNet };
    let choice = move || {
        if bats {
            ModelChoice::Bats(state.call_detection.bat_region().get())
        } else {
            ModelChoice::BirdNet
        }
    };

    // Which model files are on this device; checked once per opening.
    classifier::refresh_downloads(state);

    let list_view = RwSignal::new(ListView::BySpecies);
    // Merging neighbouring windows suits bats: one pass is many 0.56 s windows.
    let merge = RwSignal::new(bats);
    let show_credits = RwSignal::new(false);

    let current_file = Memo::new(move |_| state.current_file_id_tracked());
    let source_rate = Memo::new(move |_| {
        let idx = state.library.current_index().get();
        state.library.files().with(|files| {
            idx.and_then(|i| files.get(i))
                .map(|f| (f.audio.sample_rate, f.audio.duration_secs))
        })
    });
    let backbone = Memo::new(move |_| {
        state
            .classifier
            .downloads()
            .with(|m| m.get(catalog::BACKBONE.id).cloned().unwrap_or_default())
    });
    // This file's results for this method: (windows, complete, region, window length).
    let results_info = Memo::new(move |_| {
        let file = current_file.get()?;
        state.classifier.results().with(|m| {
            m.get(&(file, method))
                .map(|r| (r.windows.len(), r.complete, r.region, r.window_secs))
        })
    });
    let min_conf = move || state.call_detection.min_confidence().get();
    let summary = Memo::new(move |_| {
        let file = current_file.get()?;
        let min = min_conf();
        state
            .classifier
            .results()
            .with(|m| m.get(&(file, method)).map(|r| summary::summarise(r, min)))
    });
    let run_status = move || state.classifier.run().get();
    let busy_here = move || {
        let file = current_file.get();
        match run_status() {
            RunStatus::LoadingModel { file_id, method: m }
            | RunStatus::Running {
                file_id, method: m, ..
            } => Some(file_id) == file && m == method,
            RunStatus::Rescoring { file_id, .. } => Some(file_id) == file && bats,
            _ => false,
        }
    };

    // A new region relabels this file's bat results from the stored
    // embeddings; the backbone does not run again.
    if bats {
        Effect::new(move |_| {
            let region = state.call_detection.bat_region().get();
            let Some(file) = current_file.get() else {
                return;
            };
            let Some((n, _, Some(have), _)) = results_info.get() else {
                return;
            };
            if n > 0 && have != region && !state.classifier.run().with_untracked(|r| r.is_busy()) {
                state.call_detection.selected_window().set(None);
                if let Err(error) = classifier::rescore(state, file, region) {
                    state.classifier.run().set(RunStatus::Failed {
                        file_id: file,
                        method,
                        error,
                    });
                }
            }
        });
    }

    // Download, analyse, or progress.
    let action = move || {
        let Some(file_id) = current_file.get() else {
            return view! { <div class="sidebar-panel-empty">"No file selected"</div> }.into_any();
        };
        let total_size: u64 = choice().files().iter().map(|f| f.size).sum();
        match backbone.get() {
            DownloadStatus::Unknown => {
                return view! { <div class="detection-status"></div> }.into_any();
            }
            DownloadStatus::NotDownloaded => {
                return view! {
                    <div class="detection-status">
                        <button
                            class="sidebar-btn detection-primary-btn"
                            on:click=move |_| classifier::download(state, choice())
                        >{format!("Download model ({})", mb(total_size))}</button>
                        <div class="detection-status-note">"Runs on this device. Recordings are not uploaded."</div>
                    </div>
                }
                .into_any();
            }
            DownloadStatus::Downloading { fraction } => {
                let done = (fraction * catalog::BACKBONE.size as f64 / MB).round();
                let pct = fraction * 100.0;
                return view! {
                    <div class="detection-status">
                        <div>{format!(
                            "Downloading model: {done:.0} of {:.0} MB",
                            catalog::BACKBONE.size as f64 / MB
                        )}</div>
                        <div class="detection-progress">
                            <div class="detection-progress-fill" style=format!("width:{pct:.1}%")></div>
                        </div>
                    </div>
                }
                .into_any();
            }
            DownloadStatus::Failed(e) => {
                return view! {
                    <div class="detection-status">
                        <div class="detection-error">{format!("Download failed. {e}")}</div>
                        <button
                            class="sidebar-btn detection-primary-btn"
                            on:click=move |_| classifier::download(state, choice())
                        >{format!("Retry download ({})", mb(total_size))}</button>
                    </div>
                }
                .into_any();
            }
            DownloadStatus::Downloaded => {}
        }

        // Bats: the sample rate must reach 96 kHz.
        if let (Some(min), Some((rate, _))) = (method.min_source_rate(), source_rate.get()) {
            if rate < min {
                return view! {
                    <div class="detection-status">
                        <div class="detection-error">{format!(
                            "Sample rate too low: {} kHz. BattyBirdNET needs a sample rate of {} kHz or more.",
                            rate as f64 / 1000.0,
                            min / 1000
                        )}</div>
                    </div>
                }
                .into_any();
            }
        }

        let stop = move || {
            view! {
                <button class="psd-btn detection-stop-btn" on:click=move |_| classifier::cancel(state)>"Stop"</button>
            }
        };
        if busy_here() {
            let (text, pct) = match run_status() {
                RunStatus::LoadingModel { .. } => ("Loading the model\u{2026}".to_string(), None),
                RunStatus::Rescoring { region, .. } => {
                    (format!("Relabelling for {}\u{2026}", region.label()), None)
                }
                RunStatus::Running { fraction, .. } => {
                    let (done, len) = results_info
                        .get()
                        .map(|(n, _, _, len)| (n, len))
                        .unwrap_or((0, method.window_secs()));
                    let total = source_rate
                        .get()
                        .map(|(_, dur)| (dur / len).ceil() as usize)
                        .unwrap_or(0)
                        .max(done);
                    (
                        format!(
                            "Analysing {} windows: {} of {}",
                            window_len(len),
                            thousands(done),
                            thousands(total)
                        ),
                        Some(fraction * 100.0),
                    )
                }
                _ => (String::new(), None),
            };
            return view! {
                <div class="detection-status">
                    <div class="detection-busy-row">
                        <span>{text}</span>
                        {stop()}
                    </div>
                    <div class="detection-progress">
                        {pct.map(|p| view! {
                            <div class="detection-progress-fill" style=format!("width:{p:.1}%")></div>
                        })}
                    </div>
                </div>
            }
            .into_any();
        }

        let label = match choice() {
            ModelChoice::BirdNet => "Analyse with BirdNET".to_string(),
            ModelChoice::Bats(r) => format!("Analyse with BattyBirdNET ({})", r.label()),
        };
        let error = match run_status() {
            RunStatus::Failed {
                file_id: f,
                method: m,
                error,
            } if f == file_id && m == method => Some(error),
            _ => None,
        };
        let partial = results_info.get().and_then(|(n, complete, _, _)| {
            (!complete && n > 0).then(|| {
                format!(
                    "Stopped after {}. Analyse again for the whole recording.",
                    count(n, "window", "windows")
                )
            })
        });
        view! {
            <div class="detection-status">
                {error.map(|e: ClassifierError| view! { <div class="detection-error">{e.to_string()}</div> })}
                {partial.map(|p| view! { <div class="detection-status-note">{p}</div> })}
                <button
                    class="sidebar-btn detection-primary-btn"
                    on:click=move |_| analyse(state, file_id, choice())
                >{label}</button>
            </div>
        }
        .into_any()
    };

    // Minimum confidence, summary, overlay, list, and the selected window.
    let results = move || {
        let s = summary.get()?;
        let (n, _, region, _) = results_info.get()?;
        if n == 0 {
            return None;
        }
        let min = min_conf();
        let title = match region {
            Some(r) => format!("Results: BattyBirdNET, {}", r.label()),
            None => format!("Results: {}", method_name(method)),
        };
        let headline = if s.species.is_empty() {
            match s.highest {
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
            }
        } else {
            format!(
                "{min:.2} or above: {}, {}",
                count(s.species.len(), "species", "species"),
                count(s.species_windows, "window", "windows")
            )
        };
        let mut notes = vec![format!(
            "Below {min:.2}: {}",
            count(s.below_windows, "window", "windows")
        )];
        if s.not_species_windows > 0 {
            notes.push(format!(
                "Noise and other sounds: {}",
                count(s.not_species_windows, "window", "windows")
            ));
        }
        let view_button = move |value: ListView, label: &'static str| {
            view! {
                <button
                    class=move || if list_view.get() == value { "psd-btn psd-btn-active" } else { "psd-btn" }
                    on:click=move |_| list_view.set(value)
                >{label}</button>
            }
        };
        Some(view! {
            <div class="setting-group">
                <div class="setting-group-title">{title}</div>
                <div class="setting-row" title="Shows only windows whose top species has at least this confidence. Applies to the summary, the list, the annotations and the overlay.">
                    <span class="setting-label">"Minimum confidence"</span>
                    <span class="setting-value">{move || format!("{:.2}", min_conf())}</span>
                </div>
                <div class="setting-row">
                    <input
                        type="range"
                        class="setting-range"
                        min="0.05"
                        max="0.95"
                        step="0.05"
                        prop:value=move || min_conf().to_string()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                state.call_detection.min_confidence().set(v);
                            }
                        }
                    />
                </div>
                <div class="detection-status">
                    <div class="detection-summary">{headline}</div>
                    {notes.into_iter().map(|t| view! { <div class="detection-status-note">{t}</div> }).collect::<Vec<_>>()}
                </div>
                <div class="setting-row detection-annotate-row">
                    <label class="setting-label">
                        <input
                            type="checkbox"
                            prop:checked=move || state.call_detection.overlay_enabled().get()
                            on:change=move |ev| state.call_detection.overlay_enabled().set(event_target_checked(&ev))
                        />
                        " Show overlay"
                    </label>
                </div>
                <div class="detection-annotate-row">
                    <div class="psd-btn-group">
                        {view_button(ListView::BySpecies, "By species")}
                        {view_button(ListView::ByWindow, "By window")}
                    </div>
                </div>
                {move || match list_view.get() {
                    ListView::BySpecies => species_table(state, method, current_file).into_any(),
                    ListView::ByWindow => window_table(state, method, current_file).into_any(),
                }}
                {move || top_three(state, method, current_file)}
            </div>
        })
    };

    // Add as annotations.
    let annotate = move || {
        let file_id = current_file.get()?;
        let (n, _, _, _) = results_info.get()?;
        if n == 0 || busy_here() {
            return None;
        }
        let min = min_conf();
        let m = merge.get();
        let (to_add, tag) = state.classifier.results().with(|r| {
            r.get(&(file_id, method))
                .map(|run| {
                    (
                        summary::spans(run, min, m).len(),
                        summary::auto_tag_value(run),
                    )
                })
                .unwrap_or_default()
        });
        let existing = state.annotations.store().with(|store| {
            store
                .get(file_id)
                .map(|set| {
                    set.annotations
                        .iter()
                        .filter(|a| summary::has_auto_tag(a, &tag))
                        .count()
                })
                .unwrap_or(0)
        });
        let note = (existing > 0 && to_add > 0).then(|| {
            format!(
                "Replaces {} tagged #auto={tag}, including any you edited. Undo with {}.",
                count(existing, "annotation", "annotations"),
                undo_keys()
            )
        });
        let option = move |value: bool, label: &'static str| {
            view! {
                <label class="setting-label detection-radio">
                    <input
                        type="radio"
                        name="classifier-annotate-merge"
                        prop:checked=move || merge.get() == value
                        on:change=move |_| merge.set(value)
                    />
                    {label}
                </label>
            }
        };
        Some(view! {
            <div class="setting-group">
                <div class="setting-group-title">"Add as annotations"</div>
                <div class="detection-annotate-row detection-radio-group">
                    {option(false, "One per window")}
                    {option(true, "Merge neighbouring windows with the same species")}
                </div>
                {note.map(|t| view! { <div class="detection-status-note detection-annotate-row">{t}</div> })}
                <button
                    class="sidebar-btn detection-add-btn"
                    disabled=to_add == 0
                    on:click=move |_| add_annotations(state, file_id, method, merge.get_untracked())
                >{if to_add == 0 {
                    "Add annotations".to_string()
                } else {
                    format!("Add {}", count(to_add, "annotation", "annotations"))
                }}</button>
            </div>
        })
    };

    // The stored model, and the attribution.
    let model = move || {
        let downloaded = backbone.get() == DownloadStatus::Downloaded;
        let a = catalog::attribution(method);
        let byline = if bats {
            "BattyBirdNET by rdz-oss, powered by BirdNET."
        } else {
            "Powered by BirdNET."
        };
        view! {
            <div class="setting-group">
                <div class="setting-group-title">"Model"</div>
                {downloaded.then(|| view! {
                    <div class="detection-status">
                        <div class="detection-status-note">{format!(
                            "Downloaded: {}. Used by BirdNET and BattyBirdNET.",
                            mb(catalog::BACKBONE.size)
                        )}</div>
                    </div>
                    <button
                        class="psd-btn detection-delete-btn"
                        on:click=move |_| {
                            let ok = web_sys::window()
                                .and_then(|w| {
                                    w.confirm_with_message(&format!(
                                        "Delete the model ({}) from this device? BirdNET and BattyBirdNET can't run until it is downloaded again. Results already shown stay.",
                                        mb(catalog::BACKBONE.size)
                                    ))
                                    .ok()
                                })
                                .unwrap_or(false);
                            if ok {
                                classifier::cancel(state);
                                classifier::delete_files(state, catalog::all_files().collect());
                            }
                        }
                    >{format!("Delete model ({})", mb(catalog::BACKBONE.size))}</button>
                })}
                <div class="detection-status detection-credit">
                    {byline}" "
                    <a href=a.licence_url target="_blank" rel="noopener">{a.licence}</a>
                    ". "
                    <button
                        class="detection-link-btn"
                        aria-expanded=move || show_credits.get().to_string()
                        on:click=move |_| show_credits.update(|v| *v = !*v)
                    >"Credits"</button>
                    {move || show_credits.get().then(|| view! {
                        <div class="detection-credits">
                            {a.credits.iter().map(|c| view! { <p>{*c}</p> }).collect::<Vec<_>>()}
                            <p>{a.citation}</p>
                            <p>
                                <a href=a.powered_by_url target="_blank" rel="noopener">"birdnet.cornell.edu"</a>
                            </p>
                        </div>
                    })}
                </div>
            </div>
        }
    };

    view! {
        <div>
            {action}
            {results}
            {annotate}
            {model}
        </div>
    }
}

/// The "By species" list: one row per species, most windows first. A click
/// shows the species' best window.
fn species_table(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
) -> impl IntoView {
    let min = state.call_detection.min_confidence().get();
    let rows = state.classifier.results().with(|m| {
        let Some(run) = current_file.get().and_then(|f| m.get(&(f, method))) else {
            return Vec::new();
        };
        summary::summarise(run, min)
            .species
            .iter()
            .map(|row| {
                let label = run.labels.get(row.class as usize);
                let name = class_name(label);
                let scientific = label.map(|l| l.scientific.clone()).unwrap_or_default();
                let w = &run.windows[row.best_window];
                let (start, end, i) = (w.start_s, w.end_s, row.best_window);
                let class = move || {
                    if state.call_detection.selected_window().get() == Some(i) {
                        "detection-row selected"
                    } else {
                        "detection-row"
                    }
                };
                view! {
                    <tr class=class title=scientific on:click=move |_| show_window(state, start, end, i)>
                        <td class="detection-col-kind">{name}</td>
                        <td class="detection-col-num">{row.windows}</td>
                        <td class="detection-col-num">{format!("{:.2}", row.best)}</td>
                    </tr>
                }
            })
            .collect::<Vec<_>>()
    });
    view! {
        <div class="detection-list">
            <table class="detection-table">
                <thead>
                    <tr>
                        <th>"Species"</th>
                        <th class="detection-col-num">"Windows"</th>
                        <th class="detection-col-num" title="Best confidence">"Best"</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    }
}

/// The "By window" list: windows at or above the minimum, in time order.
fn window_table(state: AppState, method: Method, current_file: Memo<Option<u64>>) -> impl IntoView {
    let min = state.call_detection.min_confidence().get();
    let rows = state.classifier.results().with(|m| {
        let Some(run) = current_file.get().and_then(|f| m.get(&(f, method))) else {
            return Vec::new();
        };
        run.windows
            .iter()
            .enumerate()
            .filter_map(|(i, w)| {
                let kind = summary::window_kind(run, i, min);
                if kind == WindowKind::Below {
                    return None;
                }
                let top = w.top[0];
                let label = run.label(&top);
                let name = class_name(label);
                let scientific = label.map(|l| l.scientific.clone()).unwrap_or_default();
                let (start, end) = (w.start_s, w.end_s);
                let other = kind == WindowKind::NotSpecies;
                let class = move || {
                    let mut c = String::from("detection-row");
                    if other {
                        c.push_str(" detection-row-other");
                    }
                    if state.call_detection.selected_window().get() == Some(i) {
                        c.push_str(" selected");
                    }
                    c
                };
                Some(view! {
                    <tr class=class title=scientific on:click=move |_| show_window(state, start, end, i)>
                        <td class="detection-col-num">{i + 1}</td>
                        <td>{crate::format_time::format_time_display(start, 1)}</td>
                        <td class="detection-col-kind">{name}</td>
                        <td class="detection-col-num">{format!("{:.2}", top.confidence)}</td>
                    </tr>
                })
            })
            .collect::<Vec<_>>()
    });
    view! {
        <div class="detection-list">
            <table class="detection-table">
                <thead>
                    <tr>
                        <th class="detection-col-num">"#"</th>
                        <th>"Start"</th>
                        <th>"Species"</th>
                        <th class="detection-col-num" title="Confidence">"Conf."</th>
                    </tr>
                </thead>
                <tbody>{rows}</tbody>
            </table>
        </div>
    }
}

/// The selected window's top 3 classes.
fn top_three(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
) -> Option<impl IntoView> {
    let i = state.call_detection.selected_window().get()?;
    state.classifier.results().with(|m| {
        let run = current_file.get().and_then(|f| m.get(&(f, method)))?;
        let w = run.windows.get(i)?;
        let rows = w
            .top
            .iter()
            .take(3)
            .map(|s| {
                let label = run.label(s);
                let scientific = label
                    .filter(|l| l.is_species && l.scientific != l.common)
                    .map(|l| l.scientific.clone());
                view! {
                    <tr>
                        <td class="detection-col-kind">
                            {class_name(label)}
                            {scientific.map(|s| view! { <div class="detection-scientific">{s}</div> })}
                        </td>
                        <td class="detection-col-num">{format!("{:.2}", s.confidence)}</td>
                    </tr>
                }
            })
            .collect::<Vec<_>>();
        Some(view! {
            <div class="detection-top3">
                <div class="detection-status-note">{format!(
                    "Top 3 in window {} ({} to {})",
                    i + 1,
                    crate::format_time::format_time_display(w.start_s, 1),
                    crate::format_time::format_time_display(w.end_s, 1)
                )}</div>
                <table class="detection-table">
                    <tbody>{rows}</tbody>
                </table>
            </div>
        })
    })
}

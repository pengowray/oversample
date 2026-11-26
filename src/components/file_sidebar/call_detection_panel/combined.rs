//! The combined scan's panel (`crate::classifier::combined`): one action
//! (Download, Analyse, or progress through its models), the settings in
//! force, one list of what was found grouped as Birds, Bats, Other animals
//! and Noise, and the models' credits.
//!
//! The list reads each model's own results, so picking that model in the
//! method drop-down afterwards shows the same results in full (rows above
//! the spectrogram, every window, annotations).

use super::classifier_section::{combined_status, mb, model_files, AUTO_ANALYSE_MAX_SECS};
use super::model_line::model_line;
use super::parts::{in_force_line, section_head, settings_button};
use super::{count, event_target_checked, search_when_open};
use crate::call_annotations::kind_label;
use crate::canvas::box_class::{call_class, BoxClass};
use crate::classifier::{
    self, boxes, combined, summary, DownloadStatus, Method, MethodFacts, RunStatus,
};
use crate::dsp::call_detect::CallKind;
use oversample_core::dsp::digital_signals::DigitalKind;
use oversample_core::dsp::noise_events::NoiseKind;
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;

thread_local! {
    /// The files the combined scan has started on by itself this session: a
    /// scan the user stopped, or that failed, is not started again.
    static AUTO_STARTED: std::cell::RefCell<std::collections::HashSet<u64>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// The groups of the list, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Group {
    Birds,
    Bats,
    OtherAnimals,
    Noise,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Group::Birds => "Birds",
            Group::Bats => "Bats",
            Group::OtherAnimals => "Other animals",
            Group::Noise => "Noise",
        }
    }
}

/// How a row's swatch is drawn: as its boxes are on the spectrogram.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Swatch {
    Animal,
    /// A species colour slot (`overlays::SPECIES_COLOURS`).
    Species(usize),
    Noise,
}

/// What a press on a row shows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Show {
    /// A stretch of time, from start to end in seconds.
    Time(f64, f64),
    /// A bird box: start and end in seconds, and lowest frequency.
    Box(f64, f64, f64),
}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    group: Group,
    name: String,
    /// A named species, counted in the heading.
    species: bool,
    /// "12 boxes", "3 windows".
    found: String,
    /// For ordering: boxes, windows or detections.
    n: usize,
    best: Option<f32>,
    swatch: Swatch,
    show: Option<Show>,
    title: String,
}

/// The rows for file `file`, from every model's results. Reactive.
fn rows(state: AppState, file: u64) -> Vec<Row> {
    let mut out = Vec::new();
    // V3.0's birds are listed by their boxes once boxes were looked for;
    // before that (an Analyse with BirdNET+ V3.0 alone), by their windows.
    let boxes_apply = boxes::found_for(state, file);

    // Birds: the bird boxes, one row per name.
    let drawn = boxes::drawn(state);
    let mut birds: Vec<Row> = Vec::new();
    for b in drawn.iter() {
        let name = b.name.clone().unwrap_or_else(|| boxes::UNNAMED.to_string());
        // The named bird's own score; an unnamed box has none.
        let score = b
            .name
            .as_ref()
            .and_then(|n| b.species.iter().find(|(s, _)| s == n))
            .map(|(_, s)| *s);
        match birds.iter_mut().find(|r| r.name == name) {
            Some(r) => {
                r.n += 1;
                if score > r.best {
                    r.best = score;
                    r.show = Some(Show::Box(b.t0, b.t1, b.f0));
                }
            }
            None => birds.push(Row {
                group: Group::Birds,
                species: b.name.is_some(),
                name,
                found: String::new(),
                n: 1,
                best: score,
                swatch: Swatch::Animal,
                show: Some(Show::Box(b.t0, b.t1, b.f0)),
                title: String::new(),
            }),
        }
    }
    for r in &mut birds {
        r.found = count(r.n, "box", "boxes");
        r.title = if r.best.is_some() {
            "Go to the box with this bird's highest confidence".to_string()
        } else {
            "Go to the first box".to_string()
        };
    }
    out.extend(birds);

    // The call detector: bat calls, and noise by kind.
    let scanned = state
        .call_detection
        .scanned()
        .with(|s| s.as_ref().map(|(id, _)| *id));
    if scanned == Some(file) {
        state.call_detection.detected().with(|calls| {
            let mut kinds: Vec<(CallKind, usize, f64, f64)> = Vec::new();
            for c in calls {
                match kinds.iter_mut().find(|k| k.0 == c.kind) {
                    Some(k) => k.1 += 1,
                    None => kinds.push((c.kind, 1, c.start_s, c.end_s)),
                }
            }
            for (kind, n, t0, t1) in kinds {
                let animal = call_class(kind) == BoxClass::Animal;
                let (group, swatch, found) = if animal {
                    (Group::Bats, Swatch::Animal, count(n, "call", "calls"))
                } else {
                    (Group::Noise, Swatch::Noise, count(n, "detection", "detections"))
                };
                let name = kind_label(kind).to_string();
                out.push(Row {
                    group,
                    title: if animal {
                        "Go to the first call".to_string()
                    } else {
                        "Go to the first detection".to_string()
                    },
                    name,
                    species: false,
                    found,
                    n,
                    best: None,
                    swatch,
                    show: Some(Show::Time(t0, t1)),
                });
            }
        });
    }

    // Wind and handling noise.
    if let Some(rate) = state.library.files().with(|fs| {
        fs.iter().find(|f| f.id == file).map(|f| f.audio.sample_rate)
    }) {
        let events = classifier::noise_scan::events(file, rate).unwrap_or_default();
        for kind in [NoiseKind::Wind, NoiseKind::Handling] {
            let of_kind: Vec<_> = events.iter().filter(|e| e.kind == kind).collect();
            let Some(first) = of_kind.first() else {
                continue;
            };
            let n = of_kind.len();
            let found = match kind {
                NoiseKind::Wind => count(n, "stretch", "stretches"),
                NoiseKind::Handling => count(n, "detection", "detections"),
            };
            out.push(Row {
                group: Group::Noise,
                title: match kind {
                    NoiseKind::Wind => "Go to the first stretch".to_string(),
                    NoiseKind::Handling => "Go to the first detection".to_string(),
                },
                name: classifier::noise_scan::kind_name(kind).to_string(),
                species: false,
                found,
                n,
                best: None,
                swatch: Swatch::Noise,
                show: Some(Show::Time(first.start_s, first.end_s)),
            });
        }
    }

    // Square waves and modulated carriers from electronics.
    if let Some(rate) = state.library.files().with(|fs| {
        fs.iter().find(|f| f.id == file).map(|f| f.audio.sample_rate)
    }) {
        let events = classifier::noise_scan::digital(file, rate).unwrap_or_default();
        for kind in [DigitalKind::SquareWave, DigitalKind::Modulated] {
            let of_kind: Vec<_> = events.iter().filter(|e| e.kind == kind).collect();
            let Some(first) = of_kind.first() else {
                continue;
            };
            let n = of_kind.len();
            out.push(Row {
                group: Group::Noise,
                title: "Go to the first detection".to_string(),
                name: classifier::noise_scan::digital_kind_name(kind).to_string(),
                species: false,
                found: count(n, "detection", "detections"),
                n,
                best: None,
                swatch: Swatch::Noise,
                show: Some(Show::Time(first.start_s, first.end_s)),
            });
        }
    }

    // BattyBirdNET's bats, and BirdNET+ V3.0's other animals and sounds
    // (its birds have the boxes, while they apply).
    for method in combined::methods(state, file) {
        let min = classifier::min_confidence(state, method);
        state.classifier.results().with(|m| {
            let Some(run) = m.get(&(file, method)) else {
                return;
            };
            let s = summary::summarise(run, min);
            let order = classifier::kept_order::colour_order(file, method, run, min);
            let is_bird = boxes::bird_test(&run.labels);
            let by = method.info().name;
            let window = summary::step_text(run.window_secs);
            for sp in &s.species {
                let Some(label) = run.labels.get(sp.class as usize) else {
                    continue;
                };
                let bird = method == Method::BirdNetV3 && is_bird(sp.class);
                if bird && boxes_apply {
                    continue;
                }
                let (group, swatch) = if label.is_sound {
                    (Group::Noise, Swatch::Noise)
                } else if method == Method::Bats {
                    let slot = order.iter().position(|&c| c == sp.class);
                    (Group::Bats, slot.map_or(Swatch::Animal, Swatch::Species))
                } else if bird {
                    (Group::Birds, Swatch::Animal)
                } else {
                    (Group::OtherAnimals, Swatch::Animal)
                };
                let found = count(sp.windows, "window", "windows");
                let name = classifier::strips::display_name(label);
                let w = &run.windows[sp.best_window];
                out.push(Row {
                    group,
                    title: format!(
                        "Go to the window with the highest confidence. Found counts the {window} {by} windows where this species is the top match at confidence {min:.2} or above."
                    ),
                    name,
                    species: !label.is_sound,
                    found,
                    n: sp.windows,
                    best: Some(sp.best),
                    swatch,
                    show: Some(Show::Time(w.start_s, w.end_s)),
                });
            }
            if s.not_species_windows > 0 {
                let found = count(s.not_species_windows, "window", "windows");
                out.push(Row {
                    group: Group::Noise,
                    title: format!(
                        "The {window} {by} windows where the top match is noise or another sound that is not a species, at confidence {min:.2} or above"
                    ),
                    name: "Noise and other sounds".to_string(),
                    species: false,
                    found,
                    n: s.not_species_windows,
                    best: None,
                    swatch: Swatch::Noise,
                    show: None,
                });
            }
        });
    }
    out.sort_by(|a, b| a.group.cmp(&b.group).then(b.n.cmp(&a.n)));
    out
}

fn swatch_view(s: Swatch) -> impl IntoView {
    use crate::canvas::spectrogram_renderer::species_colour;
    match s {
        Swatch::Animal => view! { <span class="map-key cd-swatch cd-swatch-box"></span> }.into_any(),
        Swatch::Noise => view! { <span class="map-key cd-swatch cd-swatch-noise"></span> }.into_any(),
        Swatch::Species(slot) => view! {
            <span class="map-key cd-swatch" style=format!("background:{}", species_colour(slot).0)></span>
        }
        .into_any(),
    }
}

fn show(state: AppState, s: Show) {
    match s {
        Show::Time(t0, t1) => {
            super::rows::show_window(state, t0, t1);
            state.call_detection.selected_window().set(None);
        }
        Show::Box(t0, t1, f0) => {
            super::rows::show_window(state, t0, t1);
            state.call_detection.selected_window().set(None);
            state.call_detection.picked_box().set(Some((t0, f0)));
        }
    }
}

#[component]
pub(super) fn CombinedSection() -> impl IntoView {
    let state = expect_context::<AppState>();
    search_when_open(state);
    classifier::noise_scan::scan_when_shown(state);
    classifier::sound_boxes::measure_when_shown(state);
    classifier::refresh_downloads(state);
    let settings_open = RwSignal::new(false);
    let current_file = Memo::new(move |_| state.current_file_id_tracked());
    let duration = Memo::new(move |_| {
        let idx = state.library.current_index().get();
        state.library.files().with(|files| {
            idx.and_then(|i| files.get(i))
                .map(|f| f.audio.duration_secs)
        })
    });
    let files = Memo::new(move |_| {
        current_file
            .get()
            .map(|f| combined::files(state, f))
            .unwrap_or_default()
    });
    let status = Memo::new(move |_| {
        let f = files.get();
        state
            .classifier
            .downloads()
            .with(|m| combined_status(m, &f))
    });
    let busy = move || current_file.get().is_some_and(|f| combined::busy(state, f));

    // A new BattyBirdNET region relabels this file's bat results from the
    // stored embeddings, as for BattyBirdNET on its own.
    let bat_scope = Memo::new(move |_| classifier::bat_pick::bat_scope(state));
    Effect::new(move |_| {
        let region = bat_scope.get();
        let Some(file) = current_file.get() else {
            return;
        };
        let have = state.classifier.results().with(|m| {
            m.get(&(file, Method::Bats))
                .filter(|r| !r.windows.is_empty())
                .and_then(|r| r.region)
        });
        if have.is_some_and(|h| h != region)
            && !state.classifier.run().with_untracked(|r| r.is_busy())
        {
            if let Err(error) = classifier::rescore(state, file, region) {
                state.classifier.run().set(RunStatus::Failed {
                    file_id: file,
                    method: Method::Bats,
                    error,
                });
            }
        }
    });
    let analysed = Memo::new(move |_| {
        current_file
            .get()
            .is_some_and(|f| combined::analysed(state, f))
    });

    // Analyse runs by itself once per file when the models are on this
    // device, as for each model on its own; long recordings wait for the
    // button.
    Effect::new(move |_| {
        let Some(file) = current_file.get() else {
            return;
        };
        if status.get() != DownloadStatus::Downloaded
            || analysed.get()
            || state.classifier.run().with(|r| r.is_busy())
            || duration.get().is_none_or(|d| d > AUTO_ANALYSE_MAX_SECS)
        {
            return;
        }
        if AUTO_STARTED.with(|a| a.borrow_mut().insert(file)) {
            combined::run(state, file);
        }
    });

    let primary = move || {
        let Some(file) = current_file.get() else {
            return view! { <div class="cd-no-file">"No file selected"</div> }.into_any();
        };
        if busy() {
            let stage = state.classifier.combined_stage().get();
            let run = state.classifier.run().get();
            let downloading = match status.get() {
                DownloadStatus::Downloading { fraction } => Some(fraction),
                _ => None,
            };
            let total: u64 = files.get().iter().map(|f| f.size).sum();
            let (text, pct) = match (downloading, &run, stage) {
                (Some(fr), _, _) => (
                    format!(
                        "Downloading models: {:.0} of {}",
                        fr * total as f64 / 1e6,
                        mb(total)
                    ),
                    Some(fr * 100.0),
                ),
                (
                    None,
                    RunStatus::Running {
                        method,
                        fraction,
                        naming_boxes,
                        ..
                    },
                    Some((_, i, n)),
                ) => {
                    let what = if *naming_boxes {
                        "Naming bird sound boxes".to_string()
                    } else {
                        method.info().name.to_string()
                    };
                    (
                        format!(
                            "Step {} of {n}, {what}: {:.0}%",
                            i + 1,
                            (fraction * 100.0).floor()
                        ),
                        Some(fraction * 100.0),
                    )
                }
                (None, RunStatus::LoadingModel { method, .. }, Some((_, i, n))) => (
                    format!(
                        "Step {} of {n}, loading {}\u{2026}",
                        i + 1,
                        method.info().name
                    ),
                    None,
                ),
                _ => ("Loading\u{2026}".to_string(), None),
            };
            return view! {
                <div class="detection-busy-row">
                    <span>{text}</span>
                    <button class="psd-btn detection-stop-btn" on:click=move |_| classifier::cancel(state)>"Stop"</button>
                </div>
                <div class="detection-progress">
                    {pct.map(|p| view! {
                        <div class="detection-progress-fill" style=format!("width:{p:.1}%")></div>
                    })}
                </div>
            }
            .into_any();
        }
        let missing: u64 = state.classifier.downloads().with(|m| {
            files
                .get()
                .iter()
                .filter(|f| m.get(f.id) != Some(&DownloadStatus::Downloaded))
                .map(|f| f.size)
                .sum()
        });
        match status.get() {
            DownloadStatus::Unknown => view! { <div class="detection-status"></div> }.into_any(),
            DownloadStatus::NotDownloaded | DownloadStatus::Failed(_) => view! {
                <button
                    class="sidebar-btn detection-primary-btn"
                    on:click=move |_| combined::run(state, file)
                >{format!("Download models ({})", mb(missing))}</button>
                <div class="detection-status-note">"Runs on this device. Recordings are not uploaded."</div>
            }
            .into_any(),
            _ => {
                let first = !analysed.get();
                view! {
                    <button
                        class=if first { "sidebar-btn detection-primary-btn" } else { "psd-btn cd-again-btn" }
                        on:click=move |_| combined::run(state, file)
                    >{if first { "Analyse" } else { "Analyse again" }}</button>
                }
                .into_any()
            }
        }
    };

    let error = move || {
        let file = current_file.get()?;
        if busy() {
            return None;
        }
        match state.classifier.run().get() {
            RunStatus::Failed {
                file_id,
                method,
                error,
            } if file_id == file => Some(view! {
                <div class="cd-notes">
                    <div class="detection-error">{format!("{}: {error}", method.info().name)}</div>
                </div>
            }),
            _ => None,
        }
    };

    // The models it uses on this file.
    let facts = move || {
        let mut f = vec!["Bat call detector".to_string()];
        if let Some(file) = current_file.get() {
            for s in combined::stages(state, file) {
                let name = s.choice.method().info().name;
                f.push(if s.boxes {
                    format!("{name} with BirdBox")
                } else {
                    name.to_string()
                });
            }
        }
        f
    };

    let settings = move || {
        settings_open.get().then(|| {
            view! {
                <div class="cd-settings">
                    <div class="setting-row detection-annotate-row">
                        <label class="setting-label">
                            <input
                                type="checkbox"
                                prop:checked=move || state.call_detection.classifier_boxes().get()
                                on:change=move |ev| state.call_detection.classifier_boxes().set(event_target_checked(&ev))
                            />
                            " Boxes on the spectrogram"
                        </label>
                    </div>
                </div>
            }
        })
    };

    let all_rows = Memo::new(move |_| {
        current_file
            .get()
            .map(|f| rows(state, f))
            .unwrap_or_default()
    });
    let head_fact = move || {
        let r = all_rows.get();
        // Species are named only by an Analyse; before one, the detector's
        // calls are listed without a count of species.
        let species = r.iter().filter(|r| r.species).count();
        if r.is_empty() || (species == 0 && !analysed.get()) {
            String::new()
        } else {
            count(species, "species", "species")
        }
    };
    let list = move || {
        let rows = all_rows.get();
        if rows.is_empty() {
            return view! {
                <div class="cd-notes"><div class="detection-status-note">
                    {move || if analysed.get() { "No birds, bats or other sounds found in this recording." } else { "Analyse to list the species found in this recording." }}
                </div></div>
            }
            .into_any();
        }
        let mut body = Vec::new();
        let mut last = None;
        for r in rows {
            if last != Some(r.group) {
                last = Some(r.group);
                body.push(
                    view! {
                        <tr class="cd-group-row"><th colspan="3">{r.group.title()}</th></tr>
                    }
                    .into_any(),
                );
            }
            let target = r.show;
            body.push(view! {
                <tr
                    class="detection-row"
                    title=r.title.clone()
                    on:click=move |_| if let Some(s) = target { show(state, s) }
                >
                    <td class="detection-col-kind">
                        <span class="cd-name-line">{swatch_view(r.swatch)}<span>{r.name.clone()}</span></span>
                    </td>
                    <td class="detection-col-num">{r.found.clone()}</td>
                    <td class="detection-col-num">{r.best.map_or_else(|| "\u{2013}".to_string(), |b| format!("{b:.2}"))}</td>
                </tr>
            }.into_any());
        }
        view! {
            <table class="detection-table cd-list">
                <thead>
                    <tr>
                        <th></th>
                        <th class="detection-col-num">"Found"</th>
                        <th class="detection-col-num" title="Best confidence">"Best"</th>
                    </tr>
                </thead>
                <tbody>{body}</tbody>
            </table>
        }
        .into_any()
    };

    // The credits each model's licence asks for.
    let show_credits_v3 = RwSignal::new(false);
    let show_credits_bats = RwSignal::new(false);
    let model_status = move |m: Method| {
        Memo::new(move |_| {
            state
                .classifier
                .downloads()
                .with(|d| combined_status(d, &model_files(m)))
        })
    };
    let v3_status = model_status(Method::BirdNetV3);
    let bats_status = model_status(Method::Bats);
    let size = |m: Method| model_files(m).iter().map(|f| f.size).sum::<u64>();
    let models = move || {
        let methods = current_file
            .get()
            .map(|f| combined::methods(state, f))
            .unwrap_or_default();
        methods
            .into_iter()
            .map(|m| {
                if m == Method::Bats {
                    model_line(state, m, size(m), bats_status, show_credits_bats).into_any()
                } else {
                    model_line(state, m, size(m), v3_status, show_credits_v3).into_any()
                }
            })
            .collect::<Vec<_>>()
    };

    view! {
        <div class="cd-classifier">
            <div class="cd-action-row">
                <div class="cd-action">{primary}</div>
                {settings_button(settings_open)}
            </div>
            {move || current_file.get().is_some().then(|| view! {
                <div class="cd-action-row">
                    <div class="cd-action">{in_force_line(settings_open, facts)}</div>
                </div>
            })}
            {error}
            {settings}
            <div class="cd-section cd-recording">
                {section_head("In this recording", head_fact, || "Named species".to_string())}
                {list}
            </div>
            {super::combined_annotate::annotate_fold(state, current_file)}
            {models}
        </div>
    }
}

//! The combined scan, the Call detection tab's default method: every model
//! that suits the recording, one after another, so one Analyse finds bird
//! boxes, bat calls and the species of both.
//!
//! - The in-built call detector runs on its own, as for its own method
//!   (`call_detection_panel::search_when_open`); it finds bat calls, and
//!   tones, clicks and other noise, in recordings of 32 kHz or more.
//! - BirdNET+ V3.0 names birds, insects, frogs and mammals, and Just-Bird
//!   finds each bird sound as a box (`super::boxes`), whatever "Find bird
//!   boxes" is set to for BirdNET+ V3.0 on its own.
//! - BattyBirdNET names the bats, for recordings of 96 kHz or more, with the
//!   region picked for it (Auto by default).
//!
//! Each model's results are kept under its own method, as if it had been
//! picked on its own, so picking that method afterwards shows them in full.
//! One generation covers the whole scan, so Stop or a new run ends every
//! stage, and the status never says Done between stages.

use super::{
    bat_pick, cancel, ensure_files, finds_boxes_for, is_latest, next_generation, run_task,
    ClassifierError, Method, ModelChoice, ModelFile, RunStatus,
};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_ipc::classifier::BATS_MIN_SOURCE_RATE;

/// One model the combined scan runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub choice: ModelChoice,
    /// BirdNET+ V3.0 also finds bird boxes.
    pub boxes: bool,
}

/// The models the combined scan runs on file `file_id`, in order: BirdNET+
/// V3.0 (with bird boxes unless the file is a heterodyne recording), then
/// BattyBirdNET when the sample rate is 96 kHz or more. Reactive.
pub fn stages(state: AppState, file_id: u64) -> Vec<Stage> {
    let rate = state.library.files().with(|files| {
        files
            .iter()
            .find(|f| f.id == file_id)
            .map(|f| f.audio.sample_rate)
    });
    let Some(rate) = rate else {
        return Vec::new();
    };
    let mut out = vec![Stage {
        choice: ModelChoice::BirdNetV3,
        boxes: finds_boxes_for(state, file_id),
    }];
    if rate >= BATS_MIN_SOURCE_RATE {
        out.push(Stage {
            choice: ModelChoice::Bats(bat_pick::bat_scope(state)),
            boxes: false,
        });
    }
    out
}

/// Every model file the combined scan of `file_id` needs. Reactive.
pub fn files(state: AppState, file_id: u64) -> Vec<&'static ModelFile> {
    let mut out: Vec<&'static ModelFile> = Vec::new();
    for s in stages(state, file_id) {
        let mut f = s.choice.files();
        if s.boxes {
            f.push(&super::catalog::JUST_BIRD);
        }
        for f in f {
            if !out.iter().any(|o| o.id == f.id) {
                out.push(f);
            }
        }
    }
    out
}

/// The methods whose results the combined scan of `file_id` makes. Reactive.
pub fn methods(state: AppState, file_id: u64) -> Vec<Method> {
    stages(state, file_id)
        .iter()
        .map(|s| s.choice.method())
        .collect()
}

/// True when every method of the combined scan has results for file
/// `file_id` from an Analyse, and the bird boxes have been looked for where
/// they apply. Reactive.
pub fn analysed(state: AppState, file_id: u64) -> bool {
    let stages = stages(state, file_id);
    if stages
        .iter()
        .any(|s| s.boxes && !super::boxes::found_for(state, file_id))
    {
        return false;
    }
    state.classifier.results().with(|m| {
        stages.iter().all(|s| {
            m.get(&(file_id, s.choice.method()))
                .is_some_and(|r| r.analysed_windows() > 0)
        })
    })
}

/// Run the combined scan on file `file_id` (the current file): download what
/// is missing, then each stage in turn. Cancels any other run.
pub fn run(state: AppState, file_id: u64) {
    cancel(state);
    let generation = next_generation(state);
    let stages = untrack(|| stages(state, file_id));
    let files = untrack(|| files(state, file_id));
    let Some(first) = stages.first().copied() else {
        return;
    };
    let n = stages.len();
    state.classifier.combined_stage().set(Some((file_id, 0, n)));
    state.classifier.run().set(RunStatus::LoadingModel {
        file_id,
        method: first.choice.method(),
    });
    spawn_local(async move {
        let fail = |method: Method, error: ClassifierError| {
            state.classifier.combined_stage().set(None);
            state.classifier.run().set(RunStatus::Failed {
                file_id,
                method,
                error,
            });
        };
        if let Err(error) = ensure_files(state, files).await {
            if is_latest(state, generation) {
                fail(first.choice.method(), error);
            }
            return;
        }
        for (i, stage) in stages.iter().enumerate() {
            if !is_latest(state, generation) {
                return;
            }
            let method = stage.choice.method();
            state.classifier.combined_stage().set(Some((file_id, i, n)));
            state
                .classifier
                .run()
                .set(RunStatus::LoadingModel { file_id, method });
            // Whole windows: BirdNET+ V3.0 adds the 1.5 s grid that names the
            // bird boxes by itself, and BattyBirdNET has no step.
            let step = method.window_secs();
            let res = run_task(state, generation, file_id, stage.choice, step, stage.boxes).await;
            if !is_latest(state, generation) {
                return;
            }
            if let Err(stopped) = res {
                let res: Result<(), ClassifierError> = stopped.into();
                match res {
                    Ok(()) => state.classifier.combined_stage().set(None),
                    Err(error) => fail(method, error),
                }
                return;
            }
        }
        state.classifier.combined_stage().set(None);
        let last = stages
            .last()
            .map_or(first.choice.method(), |s| s.choice.method());
        state.classifier.run().set(RunStatus::Done {
            file_id,
            method: last,
        });
    });
}

/// True while the combined scan of `file_id` is running. Reactive.
pub fn busy(state: AppState, file_id: u64) -> bool {
    state
        .classifier
        .combined_stage()
        .with(|s| s.is_some_and(|(f, _, _)| f == file_id))
        && state.classifier.run().with(|r| r.is_busy())
}

/// One box the combined scan found, as it is added as an annotation.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundBox {
    pub t0: f64,
    pub t1: f64,
    pub f0: f64,
    pub f1: f64,
    /// "European Robin 0.60", "Bat calls", "Tone".
    pub label: String,
    /// The scientific name, when the box is named after a species.
    pub scientific: Option<String>,
    pub class: crate::canvas::box_class::BoxClass,
}

/// The boxes the combined scan found on file `file`, in time order: the
/// bird boxes (as drawn), the call detector's bat call trains (named after
/// the BattyBirdNET species whose windows cover the train's middle) and bat
/// calls in no train, and the detector's other detections (noise). Reactive.
pub fn found_boxes(state: AppState, file: u64) -> Vec<FoundBox> {
    use crate::canvas::box_class::{call_class, BoxClass};
    let mut out = Vec::new();
    for b in super::boxes::drawn(state).iter() {
        let score = b
            .name
            .as_ref()
            .and_then(|n| b.species.iter().find(|(s, _)| s == n))
            .map(|(_, s)| *s);
        out.push(FoundBox {
            t0: b.t0,
            t1: b.t1,
            f0: b.f0,
            f1: b.f1,
            label: match (&b.name, score) {
                (Some(n), Some(s)) => format!("{n} {s:.2}"),
                (Some(n), None) => n.clone(),
                (None, _) => super::boxes::UNNAMED.to_string(),
            },
            scientific: None,
            class: BoxClass::Animal,
        });
    }
    let scanned = state
        .call_detection
        .scanned()
        .with(|s| s.as_ref().map(|(id, _)| *id));
    if scanned == Some(file) {
        // BattyBirdNET's species stretches, to name the trains.
        let min = super::min_confidence(state, Method::Bats);
        let named: Vec<(f64, f64, String, Option<String>)> = state.classifier.results().with(|m| {
            m.get(&(file, Method::Bats))
                .map(|run| {
                    super::summary::spans(run, min, true)
                        .iter()
                        .filter_map(|s| {
                            let l = run.labels.get(s.class as usize)?;
                            Some((
                                s.start_s,
                                s.end_s,
                                format!("{} {:.2}", super::strips::display_name(l), s.best),
                                (l.scientific != l.common).then(|| l.scientific.clone()),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default()
        });
        state.call_detection.detected().with(|calls| {
            state.call_detection.trains().with(|trains| {
                let mut in_train = vec![false; calls.len()];
                for t in trains {
                    for &i in &t.calls {
                        if let Some(x) = in_train.get_mut(i) {
                            *x = true;
                        }
                    }
                    let mid = 0.5 * (t.start_s + t.end_s);
                    let species = named.iter().find(|n| n.0 <= mid && mid < n.1);
                    out.push(FoundBox {
                        t0: t.start_s,
                        t1: t.end_s,
                        f0: t.min_freq_hz as f64,
                        f1: t.max_freq_hz as f64,
                        label: species.map_or_else(|| "Bat calls".to_string(), |s| s.2.clone()),
                        scientific: species.and_then(|s| s.3.clone()),
                        class: BoxClass::Animal,
                    });
                }
                for (i, c) in calls.iter().enumerate() {
                    if in_train[i] {
                        continue;
                    }
                    out.push(FoundBox {
                        t0: c.start_s,
                        t1: c.end_s,
                        f0: c.event.min_freq_hz as f64,
                        f1: c.event.max_freq_hz as f64,
                        label: crate::call_annotations::kind_label(c.kind).to_string(),
                        scientific: None,
                        class: call_class(c.kind),
                    });
                }
            })
        });
    }
    for b in super::noise_scan::boxes(state) {
        out.push(FoundBox {
            t0: b.t0,
            t1: b.t1,
            f0: b.f0,
            f1: b.f1,
            label: b.label,
            scientific: None,
            class: b.class,
        });
    }
    out.sort_by(|a, b| a.t0.total_cmp(&b.t0));
    out
}

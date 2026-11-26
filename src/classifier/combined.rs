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

/// True when every method of the combined scan has results for the whole of
/// file `file_id` from an Analyse. Reactive.
pub fn analysed(state: AppState, file_id: u64) -> bool {
    let methods = methods(state, file_id);
    state.classifier.results().with(|m| {
        methods.iter().all(|&method| {
            m.get(&(file_id, method))
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

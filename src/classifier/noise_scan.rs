//! Wind and handling noise in the current file, for Birds and bats
//! (`super::combined`): `oversample_core::dsp::noise_events` run over the
//! whole file a piece at a time, with the page drawing in between, while the
//! Call detection tab shows Birds and bats. Results are kept per file for
//! this session.

use crate::canvas::overlays::LabelledBox;
use crate::state::store_fields::*;
use crate::state::{AppState, DetectionMethod, RightSidebarTab};
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_core::audio::source::ChannelView;
use oversample_core::dsp::noise_events::{NoiseEvent, NoiseKind, NoiseScanner};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Samples read per piece.
const PIECE: u64 = 1 << 19;

thread_local! {
    /// Per file id: the sample rate the file was scanned at, and the events.
    static FOUND: RefCell<HashMap<u64, (u32, Vec<NoiseEvent>)>> = RefCell::new(HashMap::new());
    /// Files being scanned now.
    static SCANNING: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
    /// Bumped when a scan finishes, so readers draw again.
    static VERSION: ArcRwSignal<u32> = ArcRwSignal::new(0);
}

fn version() -> ArcRwSignal<u32> {
    VERSION.with(|v| v.clone())
}

/// The name a kind has on screen.
pub fn kind_name(kind: NoiseKind) -> &'static str {
    match kind {
        NoiseKind::Wind => "Wind",
        NoiseKind::Handling => "Handling noise",
    }
}

/// The events found in file `file` at sample rate `rate`, once its scan has
/// finished. Reactive.
pub fn events(file: u64, rate: u32) -> Option<Vec<NoiseEvent>> {
    version().track();
    FOUND.with(|f| {
        f.borrow()
            .get(&file)
            .filter(|(r, _)| *r == rate)
            .map(|(_, e)| e.clone())
    })
}

/// The events of the current file as boxes. Reactive.
pub fn boxes(state: AppState) -> Vec<LabelledBox> {
    let Some(file) = state.current_file_id_tracked() else {
        return Vec::new();
    };
    let Some(rate) = file_rate(state, file) else {
        return Vec::new();
    };
    events(file, rate)
        .unwrap_or_default()
        .iter()
        .map(|e| LabelledBox {
            t0: e.start_s,
            t1: e.end_s,
            f0: e.lo_hz as f64,
            f1: e.hi_hz as f64,
            label: kind_name(e.kind).to_string(),
            class: crate::canvas::box_class::BoxClass::Noise,
        })
        .collect()
}

fn file_rate(state: AppState, file: u64) -> Option<u32> {
    state.library.files().with(|files| {
        files
            .iter()
            .find(|f| f.id == file)
            .map(|f| f.audio.sample_rate)
    })
}

/// Scan the current file while the Call detection tab shows Birds and bats,
/// unless it has been scanned at its present sample rate.
pub fn scan_when_shown(state: AppState) {
    Effect::new(move || {
        if state.panels.right_tab().get() != RightSidebarTab::CallDetection
            || state.call_detection.method().get() != DetectionMethod::Combined
        {
            return;
        }
        let Some(file) = state.current_file_id_tracked() else {
            return;
        };
        let Some(rate) = file_rate(state, file) else {
            return;
        };
        if untrack(|| events(file, rate)).is_some() || SCANNING.with(|s| s.borrow().contains(&file)) {
            return;
        }
        scan(state, file);
    });
}

fn scan(state: AppState, file: u64) {
    let found = state.library.files().with_untracked(|files| {
        files
            .iter()
            .find(|f| f.id == file)
            .map(|f| (f.audio.source.clone(), f.audio.sample_rate))
    });
    let Some((source, rate)) = found else {
        return;
    };
    SCANNING.with(|s| s.borrow_mut().insert(file));
    spawn_local(async move {
        crate::web_util::yield_now().await;
        let mut scanner = NoiseScanner::new(rate);
        let total = source.total_samples();
        let mut at = 0u64;
        let mut left = false;
        while at < total {
            let len = PIECE.min(total - at) as usize;
            let read = crate::audio::streaming_source::read_region_ready(
                &*source,
                ChannelView::MonoMix,
                at,
                len,
            )
            .await;
            if state.current_file_id() != Some(file) {
                left = true;
                break;
            }
            match read {
                None => scanner.gap(len as u64),
                Some(samples) => {
                    scanner.push(&samples);
                    if samples.len() < len {
                        if source.length_is_estimated() {
                            break;
                        }
                        scanner.gap((len - samples.len()) as u64);
                    }
                }
            }
            at += len as u64;
            crate::web_util::yield_now().await;
        }
        SCANNING.with(|s| s.borrow_mut().remove(&file));
        // A file left part-way is scanned again when it is shown again.
        if left {
            return;
        }
        let events = scanner.finish();
        FOUND.with(|f| f.borrow_mut().insert(file, (rate, events)));
        version().update(|v| *v = v.wrapping_add(1));
    });
}

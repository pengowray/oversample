//! Boxes round the insects, frogs, mammals and named sounds BirdNET+ V3.0
//! finds, for Birds and bats (`super::combined`). BirdNET+ V3.0 names a 3 s
//! window but gives no frequencies; `oversample_core::dsp::sound_box`
//! finds the most prominent sound in each stretch of neighbouring windows
//! with the same top class and gives its times and frequency range.
//!
//! Stretches are measured one at a time, with the page drawing in between,
//! and kept per file for this session. A stretch longer than [`PIECE_S`] is
//! measured in pieces of that length, each with a box of its own.

use super::{boxes, summary, Method};
use crate::canvas::box_class::BoxClass;
use crate::canvas::overlays::LabelledBox;
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields, DetectionMethod};
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_core::audio::source::ChannelView;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// Longest stretch measured at once, in seconds.
const PIECE_S: f64 = 30.0;

/// One stretch to measure: file, top class, start and end in seconds (as
/// bits, to be a key).
type Key = (u64, u16, u64, u64);

/// A stretch BirdNET+ V3.0 named, with what the box shows.
#[derive(Clone, Debug, PartialEq)]
struct Stretch {
    key: Key,
    start_s: f64,
    end_s: f64,
    label: String,
    class: BoxClass,
}

thread_local! {
    /// Per stretch, its box (`None`: nothing stood out).
    static FOUND: RefCell<HashMap<Key, Option<LabelledBox>>> = RefCell::new(HashMap::new());
    /// A measuring task is running.
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// Bumped as boxes are found, so readers draw again.
    static VERSION: ArcRwSignal<u32> = ArcRwSignal::new(0);
}

fn version() -> ArcRwSignal<u32> {
    VERSION.with(|v| v.clone())
}

/// The stretches of file `file` BirdNET+ V3.0 named with a class that is not
/// a bird, split into pieces of at most [`PIECE_S`]. Reactive.
fn stretches(state: AppState, file: u64) -> Vec<Stretch> {
    let min = super::min_confidence(state, Method::BirdNetV3);
    state.classifier.results().with(|m| {
        // Only a finished run: while windows arrive, a stretch grows and
        // would be measured again each time.
        let Some(run) = m.get(&(file, Method::BirdNetV3)).filter(|r| r.complete) else {
            return Vec::new();
        };
        let is_bird = boxes::bird_test(&run.labels);
        let mut out = Vec::new();
        for s in summary::spans(run, min, true) {
            let Some(l) = run.labels.get(s.class as usize) else {
                continue;
            };
            if is_bird(s.class) {
                continue;
            }
            let label = format!("{} {:.2}", super::strips::display_name(l), s.best);
            let class = if l.is_sound {
                BoxClass::Noise
            } else {
                BoxClass::Animal
            };
            let mut t = s.start_s;
            while t < s.end_s - 1e-6 {
                let end = (t + PIECE_S).min(s.end_s);
                out.push(Stretch {
                    key: (file, s.class, t.to_bits(), end.to_bits()),
                    start_s: t,
                    end_s: end,
                    label: label.clone(),
                    class,
                });
                t = end;
            }
        }
        out
    })
}

/// The boxes found so far on the current file. Reactive.
pub fn boxes(state: AppState) -> Vec<LabelledBox> {
    let Some(file) = state.current_file_id_tracked() else {
        return Vec::new();
    };
    version().track();
    let want = stretches(state, file);
    FOUND.with(|f| {
        let f = f.borrow();
        want.iter()
            .filter_map(|s| f.get(&s.key).cloned().flatten())
            .collect()
    })
}

/// The stretches of file `file` that have a box, as top class, start and
/// end in seconds: the label strip leaves their windows out, as the box
/// names them. Reactive.
pub fn boxed(state: AppState, file: u64) -> Vec<(u16, f64, f64)> {
    version().track();
    let want = stretches(state, file);
    FOUND.with(|f| {
        let f = f.borrow();
        want.iter()
            .filter(|s| f.get(&s.key).is_some_and(|b| b.is_some()))
            .map(|s| (s.key.1, s.start_s, s.end_s))
            .collect()
    })
}

/// Measure the current file's stretches that have no box yet, while the
/// Call detection tab shows Birds and bats.
pub fn measure_when_shown(state: AppState) {
    Effect::new(move || {
        version().track();
        if state.call_detection.method().get() != DetectionMethod::Combined {
            return;
        }
        let Some(file) = state.current_file_id_tracked() else {
            return;
        };
        if state.classifier.run().with(|r| r.is_busy()) {
            return;
        }
        let todo: Vec<Stretch> = stretches(state, file)
            .into_iter()
            .filter(|s| FOUND.with(|f| !f.borrow().contains_key(&s.key)))
            .collect();
        if todo.is_empty() || BUSY.with(|b| b.get()) {
            return;
        }
        measure(state, file, todo);
    });
}

fn measure(state: AppState, file: u64, todo: Vec<Stretch>) {
    let found = state.library.files().with_untracked(|files| {
        files
            .iter()
            .find(|f| f.id == file)
            .map(|f| (f.audio.source.clone(), f.audio.sample_rate))
    });
    let Some((source, rate)) = found else {
        return;
    };
    let band = super::quiet::model_band(Method::BirdNetV3);
    BUSY.with(|b| b.set(true));
    spawn_local(async move {
        for s in todo {
            crate::web_util::yield_now().await;
            if state.current_file_id() != Some(file) {
                break;
            }
            let at = (s.start_s * rate as f64).round() as u64;
            let len = ((s.end_s - s.start_s) * rate as f64).round() as usize;
            let read = crate::audio::streaming_source::read_region_ready(
                &*source,
                ChannelView::MonoMix,
                at,
                len,
            )
            .await;
            let b = read.and_then(|samples| {
                oversample_core::dsp::sound_box::estimate(&samples, rate, band)
            });
            let labelled = b.map(|b| LabelledBox {
                t0: s.start_s + b.start_s,
                t1: s.start_s + b.end_s,
                f0: b.lo_hz as f64,
                f1: b.hi_hz as f64,
                label: s.label.clone(),
                class: s.class,
            });
            FOUND.with(|f| f.borrow_mut().insert(s.key, labelled));
            version().update(|v| *v = v.wrapping_add(1));
        }
        BUSY.with(|b| b.set(false));
        // Stretches added while this ran are measured on the next change;
        // bump once more so the Effect looks again.
        version().update(|v| *v = v.wrapping_add(1));
    });
}

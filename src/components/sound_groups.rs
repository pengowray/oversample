//! The "Sound groups" row above the spectrogram and its controls in the
//! Call detection tab: the windows of the current file's classifier run,
//! grouped by how alike their embeddings are (`crate::classifier::
//! sound_groups`).
//!
//! The grouping is worked out off the reactive path and cached:
//!
//! - per (file, method), the reduced embeddings (the slow step, about
//!   `2 * windows * 1,024 * 10` multiply-adds per round) and the number of
//!   windows they cover;
//! - per number of groups, the k-means result on those (fast), so switching
//!   between 2, 3 and 4 groups answers at once.
//!
//! While a run is going, the reduction is redone at most once a second, and
//! no more often than every ten times its last duration, so a long file does
//! not hold up the page. When the run ends it is redone straight away.
//!
//! The group hints ("mostly Gentoo Penguin") are not cached: a BattyBirdNET
//! rescore changes the top classes but not the embeddings.

use crate::classifier::sound_groups::{
    self as sg, GroupTop, Grouping, Reduced, GROUP_CHOICES, GROUP_COLOURS,
};
use crate::classifier::strips::{display_name, ScoreRow, ScoreRowKind};
use crate::classifier::{ClassifierRun, Method, RunStatus};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

type Key = (u64, Method);

/// The cached grouping work for one file and method.
struct Entry {
    /// Windows the reduction covers.
    windows: usize,
    /// [`fingerprint`] of the embeddings it was made from.
    first: u64,
    reduced: Rc<Reduced>,
    /// Per number of groups: the grouping, `None` with too few windows.
    groupings: HashMap<usize, Option<Rc<Grouping>>>,
    /// How long the reduction took, in ms.
    took_ms: f64,
    /// When it finished (`Date.now()`, ms).
    done_at: f64,
}

thread_local! {
    static CACHE: RefCell<HashMap<Key, Entry>> = RefCell::new(HashMap::new());
    /// Notified when a grouping is added to the cache.
    static UPDATED: ArcTrigger = ArcTrigger::new();
    /// A reduction or grouping is being worked out.
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// A delayed reduction is waiting (during a run).
    static WAITING: Cell<bool> = const { Cell::new(false) };
}

/// Shortest gap between reductions while a run is going, in ms.
const RUN_GAP_MS: f64 = 1000.0;

/// A cheap check that the embeddings are the ones a cache entry was made
/// from: a hash of the first window's values. A rerun on the same file with
/// the same method gives the same embeddings, so this only changes when the
/// audio or the model does.
fn fingerprint(embeddings: &[f32], len: usize) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in embeddings.iter().take(len) {
        h ^= u64::from(v.to_bits());
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// Windows with embeddings for `key`, and their fingerprint. Reactive.
fn embedded(state: AppState, key: Key) -> (usize, u64) {
    let len = key.1.embedding_len();
    state.classifier.embeddings().with(|m| {
        m.get(&key)
            .map_or((0, 0), |e| (e.len() / len, fingerprint(e, len)))
    })
}

/// The current grouping of `file`'s windows for `method`, with the picked
/// number of groups. `None` until worked out, with too few windows, or when
/// the row is off. Reactive. While a run adds windows, the last grouping is
/// returned until the next one is ready; windows after it have no group.
pub fn grouping(state: AppState, file: u64, method: Method) -> Option<Rc<Grouping>> {
    UPDATED.with(|t| t.track());
    if !state.call_detection.sound_groups().get() {
        return None;
    }
    let k = state.call_detection.sound_group_count().get();
    let (windows, first) = embedded(state, (file, method));
    CACHE.with(|c| {
        let c = c.borrow();
        let e = c.get(&(file, method))?;
        // Made from other embeddings (a new run that is shorter so far).
        if e.first != first || e.windows > windows {
            return None;
        }
        e.groupings.get(&k).cloned().flatten()
    })
}

/// A class's name as shown, from the run's labels.
fn class_name(run: &ClassifierRun, class: u16) -> String {
    run.labels
        .get(class as usize)
        .map_or_else(|| "Unknown".to_string(), display_name)
}

fn hint(run: &ClassifierRun, top: &GroupTop) -> String {
    sg::hint(top, |c| class_name(run, c))
}

/// The "Sound groups" row for `run`, and each group's hover name ("Group 2:
/// mostly Gentoo Penguin"). Reactive. `None` when there is no grouping.
pub fn row(
    state: AppState,
    file: u64,
    method: Method,
    run: &ClassifierRun,
) -> Option<(ScoreRow, Vec<String>)> {
    let g = grouping(state, file, method)?;
    let names = sg::group_tops(&g, run)
        .iter()
        .enumerate()
        .map(|(i, top)| format!("Group {}: {}", i + 1, hint(run, top)))
        .collect();
    let cells = (0..run.windows.len())
        .map(|i| g.of_window.get(i).map(|&x| f32::from(x)))
        .collect();
    Some((
        ScoreRow {
            kind: ScoreRowKind::Groups,
            label: "Sound groups".to_string(),
            scientific: String::new(),
            unlikely: false,
            cells,
        },
        names,
    ))
}

/// Work out the reduction and grouping for `key` with `k` groups, after
/// letting the page draw. `reduce` false reuses the cached reduction.
fn compute(state: AppState, key: Key, k: usize, reduce: bool, done: ArcTrigger) {
    BUSY.with(|b| b.set(true));
    spawn_local(async move {
        crate::web_util::yield_now().await;
        if reduce {
            let len = key.1.embedding_len();
            let started = state.classifier.embeddings().with_untracked(|m| {
                m.get(&key).map(|e| {
                    let w = e.len() / len;
                    (w, fingerprint(e, len), sg::Reducer::new(&e[..w * len], len))
                })
            });
            if let Some((windows, first, mut reducer)) = started {
                // One round at a time (about 20 ms each for an hour of
                // BirdNET, natively), letting the page draw in between.
                let mut took_ms = 0.0;
                loop {
                    let t0 = now_ms();
                    let done = reducer.step();
                    took_ms += now_ms() - t0;
                    if done {
                        break;
                    }
                    crate::web_util::yield_now().await;
                }
                let reduced = reducer.finish();
                CACHE.with(|c| {
                    c.borrow_mut().insert(
                        key,
                        Entry {
                            windows,
                            first,
                            reduced: Rc::new(reduced),
                            groupings: HashMap::new(),
                            took_ms,
                            done_at: now_ms(),
                        },
                    )
                });
                crate::web_util::yield_now().await;
            }
        }
        let reduced = CACHE.with(|c| c.borrow().get(&key).map(|e| e.reduced.clone()));
        if let Some(reduced) = reduced {
            let g = sg::group(&reduced, k).map(Rc::new);
            CACHE.with(|c| {
                if let Some(e) = c.borrow_mut().get_mut(&key) {
                    e.groupings.insert(k, g);
                }
            });
        }
        BUSY.with(|b| b.set(false));
        UPDATED.with(|t| t.notify());
        done.notify();
    });
}

/// Keep the grouping of the current file up to date while the row is on.
/// Call once, from the component that draws the rows.
pub fn drive(state: AppState) {
    let tick = ArcTrigger::new();
    Effect::new(move |_| {
        tick.track();
        if !state.call_detection.sound_groups().get() {
            return;
        }
        let k = state.call_detection.sound_group_count().get();
        let Some(method) = state.call_detection.method().get().classifier() else {
            return;
        };
        let Some(file) = state.current_file_id_tracked() else {
            return;
        };
        let key = (file, method);
        let (windows, first) = embedded(state, key);
        if windows == 0 || BUSY.with(Cell::get) || WAITING.with(Cell::get) {
            return;
        }
        let running = state.classifier.run().with(|r| match r {
            RunStatus::Running {
                file_id, method: m, ..
            }
            | RunStatus::LoadingModel { file_id, method: m } => *file_id == file && *m == method,
            _ => false,
        });
        let cached = CACHE.with(|c| {
            c.borrow().get(&key).map(|e| {
                (
                    e.windows == windows && e.first == first,
                    e.groupings.contains_key(&k),
                    e.done_at + RUN_GAP_MS.max(10.0 * e.took_ms),
                )
            })
        });
        match cached {
            Some((true, true, _)) => {}
            Some((true, false, _)) => compute(state, key, k, false, tick.clone()),
            Some((false, _, due)) if running && due > now_ms() => {
                WAITING.with(|w| w.set(true));
                let tick = tick.clone();
                let wait = (due - now_ms()).ceil() as i32;
                spawn_local(async move {
                    crate::web_util::sleep_ms(wait).await;
                    WAITING.with(|w| w.set(false));
                    tick.notify();
                });
            }
            _ => compute(state, key, k, true, tick.clone()),
        }
    });
}

/// `1 window`, `41 windows`.
fn windows_text(n: usize) -> String {
    if n == 1 {
        "1 window".to_string()
    } else {
        format!("{n} windows")
    }
}

/// The legend under the controls: one line per group, or why there is none.
fn legend(state: AppState, file: u64, method: Method) -> Option<AnyView> {
    let k = state.call_detection.sound_group_count().get();
    let g = grouping(state, file, method);
    state.classifier.results().with(|m| {
        let run = m.get(&(file, method))?;
        let Some(g) = g else {
            // A run that will never have enough windows says so; one still
            // going may get them.
            let (windows, _) = embedded(state, (file, method));
            let short = run.complete && windows > 0 && windows < k + 1;
            return short.then(|| {
                view! {
                    <div class="detection-status-note sound-groups-note">{format!(
                        "{k} groups need at least {} windows. This file has {}.",
                        k + 1,
                        windows
                    )}</div>
                }
                .into_any()
            });
        };
        let rows = sg::group_tops(&g, run)
            .iter()
            .enumerate()
            .map(|(i, top)| {
                let detail = top.top.map(|(class, count)| {
                    format!(
                        "Top class in {count} of {} windows: {}",
                        top.size,
                        class_name(run, class)
                    )
                });
                view! {
                    <tr title=detail>
                        <td class="detection-col-kind">
                            <span
                                class="map-key sound-groups-swatch"
                                style:background=GROUP_COLOURS[i.min(GROUP_COLOURS.len() - 1)]
                            ></span>
                            {format!("Group {}", i + 1)}
                            <div class="sound-groups-hint">{hint(run, top)}</div>
                        </td>
                        <td class="detection-col-num">{windows_text(top.size)}</td>
                    </tr>
                }
            })
            .collect::<Vec<_>>();
        Some(
            view! {
                <table class="detection-table sound-groups-legend">
                    <tbody>{rows}</tbody>
                </table>
            }
            .into_any(),
        )
    })
}

/// The "Sound groups" toggle, the number of groups and the legend, for the
/// "Rows above the spectrogram" group. `save` saves the row picks.
pub fn controls(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
    save: fn(AppState),
) -> impl IntoView {
    let on = move || state.call_detection.sound_groups().get();
    let count_buttons = move || {
        on().then(|| {
            let buttons = GROUP_CHOICES
                .iter()
                .map(|&n| {
                    let picked = move || state.call_detection.sound_group_count().get() == n;
                    view! {
                        <button
                            class=move || if picked() { "psd-btn psd-btn-active" } else { "psd-btn" }
                            aria-pressed=move || picked().to_string()
                            aria-label=format!("{n} groups")
                            title=format!("{n} groups")
                            on:click=move |_| {
                                state.call_detection.sound_group_count().set(n);
                                save(state);
                            }
                        >{n.to_string()}</button>
                    }
                })
                .collect::<Vec<_>>();
            view! { <div class="psd-btn-group">{buttons}</div> }
        })
    };
    view! {
        <div class="detection-annotate-row sound-groups-controls">
            <button
                class=move || if on() { "psd-btn psd-btn-active" } else { "psd-btn" }
                aria-pressed=move || on().to_string()
                title="Windows that sound alike get the same colour"
                on:click=move |_| {
                    state.call_detection.sound_groups().update(|v| *v = !*v);
                    save(state);
                }
            >"Sound groups"</button>
            {count_buttons}
        </div>
        {move || {
            if !on() {
                return None;
            }
            legend(state, current_file.get()?, method)
        }}
    }
}

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
//! Quiet windows (`crate::classifier::quiet`) are left out of the grouping.
//! Their check needs the file's frame levels, measured once per file and
//! method by reading the whole file in pieces (streamed files too) after the
//! run has stopped, and cached. During a run the quiet check is used only if
//! the levels are already cached (a rerun); otherwise every window is grouped
//! until the run stops.
//!
//! The group hints ("mostly Gentoo Penguin") are not cached: a BattyBirdNET
//! rescore changes the top classes but not the embeddings.

use crate::classifier::quiet::{self, FrameLevels, LevelMeter, FRAMES_PER_WINDOW};
use crate::classifier::sound_groups::{
    self as sg, GroupTop, Grouping, Reduced, GROUP_CHOICES, GROUP_COLOURS,
};
use crate::classifier::strips::{display_name, ScoreRow, ScoreRowKind};
use crate::classifier::{ClassifierRun, Method, RunStatus};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_core::audio::source::ChannelView;
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
    /// Per window (`windows` of them), whether it is quiet and left out.
    quiet: Rc<Vec<bool>>,
    /// The quiet check was made (or could not be: the file's audio was
    /// gone). False while a run goes on with no levels cached yet.
    quiet_checked: bool,
    reduced: Rc<Reduced>,
    /// Per number of groups: the grouping, `None` with too few windows.
    groupings: HashMap<usize, Option<Rc<Grouping>>>,
    /// How long the reduction took, in ms.
    took_ms: f64,
    /// When it finished (`Date.now()`, ms).
    done_at: f64,
}

/// A file's frame levels for one method, and the file length they cover.
struct Levels {
    total_samples: u64,
    /// `None` when the file could not be found.
    levels: Option<Rc<FrameLevels>>,
}

thread_local! {
    static CACHE: RefCell<HashMap<Key, Entry>> = RefCell::new(HashMap::new());
    static LEVELS: RefCell<HashMap<Key, Levels>> = RefCell::new(HashMap::new());
    /// Notified when a grouping is added to the cache.
    static UPDATED: ArcTrigger = ArcTrigger::new();
    /// A reduction or grouping is being worked out.
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// The file and method a delayed reduction is waiting for (during a run).
    static WAITING: Cell<Option<Key>> = const { Cell::new(None) };
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

/// The current state of the sound groups of one file and method.
pub struct Current {
    /// The grouping with the picked number of groups; `None` with too few
    /// windows that are not quiet.
    pub grouping: Option<Rc<Grouping>>,
    /// Per window, whether it is quiet and has no group.
    pub quiet: Rc<Vec<bool>>,
    /// Windows that are not quiet (the ones grouped).
    pub kept: usize,
}

/// The current grouping of `file`'s windows for `method`, with the picked
/// number of groups, and which windows are quiet. `None` until worked out or
/// when the row is off. Reactive. While a run adds windows, the last
/// grouping is returned until the next one is ready; windows after it have
/// no group.
pub fn current(state: AppState, file: u64, method: Method) -> Option<Current> {
    UPDATED.with(|t| t.track());
    if !state.call_detection.sound_groups().get() {
        return None;
    }
    let k = state.call_detection.sound_group_count().get();
    let (windows, first) = embedded(state, (file, method));
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let e = c.get_mut(&(file, method))?;
        // Made from other embeddings (a new run that is shorter so far).
        if e.first != first || e.windows > windows {
            return None;
        }
        Some(Current {
            grouping: group_now(e, k),
            quiet: e.quiet.clone(),
            kept: e.reduced.n,
        })
    })
}

/// `e`'s grouping into `k` groups, worked out now if it is not cached
/// (k-means on the reduced windows: a few ms), so a new number of groups
/// shows at once.
fn group_now(e: &mut Entry, k: usize) -> Option<Rc<Grouping>> {
    e.groupings
        .entry(k)
        .or_insert_with(|| sg::group(&e.reduced, k).map(Rc::new))
        .clone()
}

/// A class's name as shown, from the run's labels.
fn class_name(run: &ClassifierRun, class: u16) -> String {
    run.labels
        .get(class as usize)
        .map_or_else(|| "Unknown".to_string(), display_name)
}

fn hints(run: &ClassifierRun, tops: &[GroupTop]) -> Vec<String> {
    sg::hints(tops, |c| class_name(run, c))
}

/// The "Sound groups" row for `run`, each group's hover name ("Group 2:
/// mostly Gentoo Penguin"), and per window whether it is quiet (empty cell,
/// no group). Reactive. `None` when there is no grouping.
pub fn row(
    state: AppState,
    file: u64,
    method: Method,
    run: &ClassifierRun,
) -> Option<(ScoreRow, Vec<String>, Vec<bool>)> {
    let Current {
        grouping, quiet, ..
    } = current(state, file, method)?;
    let g = grouping?;
    let names = hints(run, &sg::group_tops(&g, run))
        .iter()
        .enumerate()
        .map(|(i, hint)| format!("Group {}: {hint}", i + 1))
        .collect();
    let cells = (0..run.windows.len())
        .map(|i| g.of_window.get(i).copied().flatten().map(f32::from))
        .collect();
    Some((
        ScoreRow {
            kind: ScoreRowKind::Groups,
            label: "Sound groups".to_string(),
            scientific: String::new(),
            unlikely: false,
            colour: None,
            cells,
        },
        names,
        quiet.to_vec(),
    ))
}

/// The band the quiet check measures in, in Hz: the part of the spectrum
/// the model looks at, less rumble under 250 Hz. BirdNET looks at 0 to
/// 15 kHz of its 48 kHz input, and BirdNET+ V3.0's mel bands also stop near
/// 15 kHz; BattyBirdNET is BirdNET on audio slowed from
/// 256 kHz, so its band is 256 / 48 times higher.
fn band(method: Method) -> (f32, f32) {
    crate::classifier::quiet::model_band(method)
}

/// Samples read per piece when measuring levels.
const LEVEL_PIECE: u64 = 1 << 20;

/// What [`measure`] found.
enum Measured {
    Levels(Option<Rc<FrameLevels>>),
    /// The file was left before the measuring finished.
    Abandoned,
}

/// `file`'s frame levels for `method`, from the cache or by reading the
/// whole file in pieces with the page drawing in between. A piece that
/// cannot be read counts as silence, as it does in the run.
async fn measure(state: AppState, key: Key) -> Measured {
    let (file, method) = key;
    let found = state.library.files().with_untracked(|files| {
        files
            .iter()
            .find(|f| f.id == file)
            .map(|f| (f.audio.source.clone(), f.audio.sample_rate))
    });
    let Some((source, rate)) = found else {
        return Measured::Levels(None);
    };
    let total = source.total_samples();
    let cached = LEVELS.with(|l| {
        l.borrow()
            .get(&key)
            .filter(|l| l.total_samples == total)
            .map(|l| l.levels.clone())
    });
    if let Some(levels) = cached {
        return Measured::Levels(levels);
    }
    let mut meter = LevelMeter::new(
        rate,
        method.window_secs() / FRAMES_PER_WINDOW as f64,
        band(method),
    );
    let mut at = 0u64;
    while at < total {
        let len = LEVEL_PIECE.min(total - at) as usize;
        let read = crate::audio::streaming_source::read_region_ready(
            &*source,
            ChannelView::MonoMix,
            at,
            len,
        )
        .await;
        if state.current_file_id() != Some(file) {
            return Measured::Abandoned;
        }
        let samples = read.unwrap_or_else(|| vec![0.0; len]);
        meter.push(&samples);
        if samples.len() < len {
            if source.length_is_estimated() {
                break;
            }
            meter.push(&vec![0.0; len - samples.len()]);
        }
        at += len as u64;
        crate::web_util::yield_now().await;
    }
    let levels = Some(Rc::new(meter.finish()));
    LEVELS.with(|l| {
        l.borrow_mut().insert(
            key,
            Levels {
                total_samples: total,
                levels: levels.clone(),
            },
        )
    });
    Measured::Levels(levels)
}

/// The cached levels for `key`, if measured: `Some(None)` when the file
/// could not be found.
fn cached_levels(key: Key) -> Option<Option<Rc<FrameLevels>>> {
    LEVELS.with(|l| l.borrow().get(&key).map(|l| l.levels.clone()))
}

/// Work out the quiet windows, the reduction for `key` and its grouping into
/// `k` groups, one round at a time with the page drawing in between. With
/// `measure_levels`, the file's levels are measured if not cached; without,
/// cached levels are used if there are any. The cache entry is replaced only
/// once all is ready, so the row keeps showing the last grouping until then.
fn compute(state: AppState, key: Key, k: usize, measure_levels: bool, done: ArcTrigger) {
    BUSY.with(|b| b.set(true));
    spawn_local(async move {
        crate::web_util::yield_now().await;
        let levels = if measure_levels {
            match measure(state, key).await {
                Measured::Levels(l) => Some(l),
                Measured::Abandoned => {
                    BUSY.with(|b| b.set(false));
                    done.notify();
                    return;
                }
            }
        } else {
            cached_levels(key)
        };
        let quiet_checked = levels.is_some();
        let times: Vec<(f64, f64)> = state.classifier.results().with_untracked(|m| {
            m.get(&key).map_or_else(Vec::new, |r| {
                r.windows.iter().map(|w| (w.start_s, w.end_s)).collect()
            })
        });
        let len = key.1.embedding_len();
        let started = state.classifier.embeddings().with_untracked(|m| {
            m.get(&key).map(|e| {
                let w = e.len() / len;
                let mut quiet = match levels.as_ref().and_then(Option::as_ref) {
                    Some(l) => quiet::quiet_windows(l, &times[..w.min(times.len())]),
                    None => Vec::new(),
                };
                quiet.resize(w, false);
                let reducer = sg::Reducer::new(&e[..w * len], len, &quiet);
                (w, fingerprint(e, len), quiet, reducer)
            })
        });
        if let Some((windows, first, quiet, mut reducer)) = started {
            // About 20 ms a round for an hour of BirdNET, natively.
            let mut took_ms = 0.0;
            loop {
                let t0 = now_ms();
                let finished = reducer.step();
                took_ms += now_ms() - t0;
                if finished {
                    break;
                }
                crate::web_util::yield_now().await;
            }
            let mut entry = Entry {
                windows,
                first,
                quiet: Rc::new(quiet),
                quiet_checked,
                reduced: Rc::new(reducer.finish()),
                groupings: HashMap::new(),
                took_ms,
                done_at: now_ms(),
            };
            group_now(&mut entry, k);
            CACHE.with(|c| c.borrow_mut().insert(key, entry));
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
        if windows == 0 || BUSY.with(Cell::get) || WAITING.with(|w| w.get() == Some(key)) {
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
                    // Made from these windows, and with the quiet check
                    // once the run has stopped.
                    e.windows == windows && e.first == first && (running || e.quiet_checked),
                    e.done_at + RUN_GAP_MS.max(10.0 * e.took_ms),
                )
            })
        });
        match cached {
            // Up to date. A missing number of groups is worked out by
            // `current` when the row asks for it.
            Some((true, _)) => {}
            Some((false, due)) if running && due > now_ms() => {
                WAITING.with(|w| w.set(Some(key)));
                let tick = tick.clone();
                let wait = (due - now_ms()).ceil() as i32;
                spawn_local(async move {
                    crate::web_util::sleep_ms(wait).await;
                    WAITING.with(|w| {
                        if w.get() == Some(key) {
                            w.set(None);
                        }
                    });
                    tick.notify();
                });
            }
            _ => compute(state, key, k, !running, tick.clone()),
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

/// "9 quiet windows are not grouped".
fn quiet_text(n: usize) -> String {
    if n == 1 {
        "1 quiet window is not grouped".to_string()
    } else {
        format!("{n} quiet windows are not grouped")
    }
}

/// The legend under the controls: one line per group, or why there is none,
/// and how many quiet windows were left out.
fn legend(state: AppState, file: u64, method: Method) -> Option<AnyView> {
    let k = state.call_detection.sound_group_count().get();
    let cur = current(state, file, method);
    let quiet = cur
        .as_ref()
        .map_or(0, |c| c.quiet.iter().filter(|&&q| q).count());
    let quiet_note = (quiet > 0).then(|| {
        view! { <div class="detection-status-note sound-groups-note">{quiet_text(quiet)}</div> }
    });
    state.classifier.results().with(|m| {
        let run = m.get(&(file, method))?;
        let Some(g) = cur.as_ref().and_then(|c| c.grouping.clone()) else {
            // A run that will never have enough windows says so; one still
            // going may get them.
            let (windows, _) = embedded(state, (file, method));
            let kept = cur.as_ref().map_or(windows, |c| c.kept);
            let short = run.complete && windows > 0 && kept < k + 1;
            let text = if quiet > 0 {
                format!(
                    "{k} groups need at least {} windows with sound. This file has {kept}.",
                    k + 1
                )
            } else {
                format!(
                    "{k} groups need at least {} windows. This file has {}.",
                    k + 1,
                    windows_text(windows)
                )
            };
            return short.then(|| {
                view! {
                    <div class="detection-status-note sound-groups-note">{text}</div>
                    {quiet_note}
                }
                .into_any()
            });
        };
        let tops = sg::group_tops(&g, run);
        let rows = tops
            .iter()
            .zip(hints(run, &tops))
            .enumerate()
            .map(|(i, (top, hint))| {
                let detail = top.top.map(|(class, count)| {
                    let second = top.second.map_or_else(String::new, |c| {
                        format!("\nMost often second: {}", class_name(run, c))
                    });
                    format!(
                        "Top class in {count} of {} windows: {}\nAverage confidence: {:.2}{second}",
                        top.size,
                        class_name(run, class),
                        top.mean
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
                            <div class="sound-groups-hint">{hint}</div>
                        </td>
                        <td class="detection-col-num">{windows_text(top.size)}</td>
                    </tr>
                }
            })
            .collect::<Vec<_>>();
        let shared_note = sg::shared_class(&tops).map(|class| {
            let text = sg::shared_note(tops.len(), &class_name(run, class));
            view! { <div class="detection-status-note sound-groups-note">{text}</div> }
        });
        Some(
            view! {
                <table class="detection-table sound-groups-legend">
                    <tbody>{rows}</tbody>
                </table>
                {shared_note}
                {quiet_note}
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

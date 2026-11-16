//! Experimental live BirdNET view: BirdNET V2.4 run on the last 3 s of audio a
//! few times a second, while a file plays or the microphone is listening.
//!
//! It has a worker of its own (the same `birdnet-worker` WASM as the
//! classifier, started a second time), so a classifier run, a Perch run or the
//! denoiser never wait for it and it never waits for them. The worker holds
//! the BirdNET backbone (about 300 MB); turning the view off stops it.
//!
//! One request is in flight at a time: the next goes out after the answer and
//! at least [`MIN_GAP_MS`] after the last one, so a slow device updates less
//! often instead of falling behind.
//!
//! Each answer goes straight to the chart (`live-embedding.js`) through a
//! registered element; only the status line (loading, the update interval, an
//! error) goes through the store, so nothing reactive changes per reading.

use crate::classifier::{self, catalog, ClassifierError, Method, SpeciesLabel};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use futures_channel::oneshot;
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_core::audio::source::ChannelView;
use oversample_ipc::classifier::{attach, FromWorker, ModelPart, ToWorker, EMBEDDING_LEN};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker};

const WORKER_URL: &str = "birdnet-worker_loader.js";
/// Shortest gap between two requests. Measured on a desktop (Ryzen, Chrome),
/// one reading takes about 120 to 200 ms, so 4 a second fits with room left
/// for the classifier and the spectrogram. Slower devices are held back by
/// the one-request-in-flight rule instead.
const MIN_GAP_MS: f64 = 250.0;
/// How often the loop checks whether a request can go out.
const TICK_MS: u32 = 50;
/// Seconds of readings kept for the trail and the strip.
pub const HISTORY_S: f64 = 20.0;
/// Seconds of trail on the map.
const TRAIL_S: f64 = 15.0;
/// Score a class needs somewhere in the history to get a row in the strip.
const ROW_MIN: f32 = 0.1;
/// Most rows in the strip; the ones with the lowest peak drop out first.
const MAX_ROWS: usize = 8;
/// Placement rule of the species map (`classifier::species_map`): the top 3
/// classes pull the marker, but only those within the map's radius of the
/// top class, so the marker never sits between unrelated groups.
const PLACE_TOP: usize = 3;
const PLACE_RADIUS: f32 = classifier::species_map::BIRDNET_MAP.radius;
/// Recordings at or above this rate are usually bats or other ultrasound,
/// which BirdNET (0 to 15 kHz) does not hear.
pub const ULTRASONIC_RATE: u32 = 96_000;

/// What the live view is doing, for its status line.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum LiveStatus {
    #[default]
    Off,
    /// The BirdNET model is not on this device yet.
    NeedsDownload,
    Loading,
    /// Running; `interval_ms` is the measured time between readings.
    Ready {
        interval_ms: f64,
        model_ms: f64,
    },
    Failed(String),
}

/// One reading: when it was made, and every class's score.
struct Reading {
    t: f64,
    scores: Vec<f32>,
    embedding: Vec<f32>,
    /// Map position of the marker, if a map is loaded.
    at: Option<(f32, f32)>,
    /// Cosine distance from the previous reading's embedding, 0 to 2.
    change: Option<f32>,
}

struct Link {
    worker: Worker,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
    /// Waiting for the model to be built.
    loaded: Option<oneshot::Sender<Result<(), String>>>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.worker.terminate();
    }
}

thread_local! {
    static LINK: RefCell<Option<Link>> = const { RefCell::new(None) };
    /// Goes up with each start and stop; an older loop stops when it changes.
    static GENERATION: Cell<u32> = const { Cell::new(0) };
    static HISTORY: RefCell<VecDeque<Reading>> = const { RefCell::new(VecDeque::new()) };
    /// The request in flight: its number and when it went out.
    static IN_FLIGHT: Cell<Option<(u32, f64)>> = const { Cell::new(None) };
    static LAST_SENT: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
    static NEXT_REQ: Cell<u32> = const { Cell::new(1) };
    /// The chart element, while the view is shown.
    static CHART: RefCell<Option<web_sys::HtmlElement>> = const { RefCell::new(None) };
    static LABELS: RefCell<Option<Arc<Vec<SpeciesLabel>>>> = const { RefCell::new(None) };
    static MAP: RefCell<Option<Arc<Vec<Option<(f32, f32)>>>>> = const { RefCell::new(None) };
    /// The strip's rows, in the order they first appeared.
    static ROWS: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
    /// Interval between the last two readings, smoothed.
    static INTERVAL: Cell<f64> = const { Cell::new(0.0) };
    static LAST_READING: Cell<f64> = const { Cell::new(0.0) };
    /// Sample rate of the audio last sent.
    static RATE: Cell<u32> = const { Cell::new(0) };
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

fn generation() -> u32 {
    GENERATION.with(|g| g.get())
}

/// Turn the view on or off. Off stops the worker and clears the readings.
pub fn set_enabled(state: AppState, on: bool) {
    GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    LINK.with(|l| *l.borrow_mut() = None);
    IN_FLIGHT.with(|f| f.set(None));
    HISTORY.with(|h| h.borrow_mut().clear());
    ROWS.with(|r| r.borrow_mut().clear());
    INTERVAL.with(|i| i.set(0.0));
    state.classifier.live_on().set(on);
    if !on {
        state.classifier.live().set(LiveStatus::Off);
        return;
    }
    let gen = generation();
    spawn_local(async move {
        if let Err(e) = start(state, gen).await {
            if generation() == gen {
                LINK.with(|l| *l.borrow_mut() = None);
                state.classifier.live().set(e);
            }
            return;
        }
        run_loop(state, gen).await;
    });
}

/// Register the element the readings are drawn into (`None` when it goes).
pub fn set_chart(el: Option<web_sys::HtmlElement>) {
    CHART.with(|c| *c.borrow_mut() = el);
}

async fn start(state: AppState, gen: u32) -> Result<(), LiveStatus> {
    if !classifier::store::is_downloaded(&catalog::BACKBONE).await
        || !classifier::store::is_downloaded(&catalog::BIRDNET_LABELS).await
    {
        return Err(LiveStatus::NeedsDownload);
    }
    state.classifier.live().set(LiveStatus::Loading);
    let fail = |e: String| LiveStatus::Failed(e);
    let labels = classifier::birdnet_labels(state)
        .await
        .map_err(|e| fail(e.to_string()))?;
    LABELS.with(|l| *l.borrow_mut() = Some(labels));
    if MAP.with(|m| m.borrow().is_none()) {
        // The map is optional: without it the strip still works.
        if let Ok(bytes) = crate::components::file_sidebar::fetch_bytes(classifier::species_map::BIRDNET_MAP.file).await {
            let m = classifier::species_map::parse_map(&bytes);
            MAP.with(|c| *c.borrow_mut() = Some(Arc::new(m)));
        }
    }
    let bytes = classifier::store::read_js(&catalog::BACKBONE)
        .await
        .map_err(|e: ClassifierError| fail(e.to_string()))?;
    if generation() != gen {
        return Err(LiveStatus::Off);
    }
    let (tx, rx) = oneshot::channel();
    start_worker(state, gen, tx).map_err(fail)?;
    // The worker drops messages sent before its WASM starts, so wait for Ready
    // (the first `loaded` answer) before sending the model.
    match rx.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(fail(e)),
        Err(_) => return Err(LiveStatus::Off),
    }
    let (tx, rx) = oneshot::channel();
    LINK.with(|l| {
        if let Some(link) = l.borrow_mut().as_mut() {
            link.loaded = Some(tx);
            let buf = bytes.buffer();
            post(
                link,
                &ToWorker::Load {
                    key: catalog::BACKBONE.id.to_string(),
                    part: ModelPart::Backbone,
                },
                Some((attach::BYTES, &buf)),
            );
        }
    });
    match rx.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(fail(e)),
        Err(_) => return Err(LiveStatus::Off),
    }
    state.classifier.live().set(LiveStatus::Ready {
        interval_ms: 0.0,
        model_ms: 0.0,
    });
    Ok(())
}

fn post(link: &Link, msg: &ToWorker, extra: Option<(&str, &JsValue)>) {
    let Ok(inner) = serde_wasm_bindgen::to_value(msg) else {
        return;
    };
    let obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&obj, &attach::MSG.into(), &inner);
    let _ = match extra {
        Some((name, value)) => {
            let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(name), value);
            let transfer = if value.is_instance_of::<js_sys::ArrayBuffer>() {
                value.clone()
            } else {
                value
                    .unchecked_ref::<js_sys::Float32Array>()
                    .buffer()
                    .into()
            };
            link.worker
                .post_message_with_transfer(&obj, &js_sys::Array::of1(&transfer))
        }
        None => link.worker.post_message(&obj),
    };
}

fn start_worker(
    state: AppState,
    gen: u32,
    ready: oneshot::Sender<Result<(), String>>,
) -> Result<(), String> {
    let worker = Worker::new(WORKER_URL).map_err(|e| format!("{e:?}"))?;
    let on_message =
        Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| on_message(state, gen, ev));
    let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
        let message = ev
            .dyn_ref::<web_sys::ErrorEvent>()
            .map(|e| e.message())
            .unwrap_or_else(|| "the worker stopped".to_string());
        if generation() == gen {
            LINK.with(|l| *l.borrow_mut() = None);
            state.classifier.live().set(LiveStatus::Failed(message));
        }
    });
    worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    LINK.with(|l| {
        *l.borrow_mut() = Some(Link {
            worker,
            _on_message: on_message,
            _on_error: on_error,
            loaded: Some(ready),
        })
    });
    Ok(())
}

fn on_message(state: AppState, gen: u32, ev: MessageEvent) {
    if generation() != gen {
        return;
    }
    let data = ev.data();
    let inner = js_sys::Reflect::get(&data, &attach::MSG.into()).unwrap_or(JsValue::UNDEFINED);
    let Ok(msg) = serde_wasm_bindgen::from_value::<FromWorker>(inner) else {
        return;
    };
    let resolve = |r: Result<(), String>| {
        if let Some(tx) = LINK.with(|l| l.borrow_mut().as_mut().and_then(|l| l.loaded.take())) {
            let _ = tx.send(r);
        }
    };
    match msg {
        FromWorker::Ready | FromWorker::Loaded { .. } => resolve(Ok(())),
        FromWorker::Live { req, ms } => {
            let payload = js_sys::Reflect::get(&data, &attach::EMBEDDING.into())
                .ok()
                .and_then(|v| v.dyn_into::<js_sys::Float32Array>().ok())
                .map(|a| a.to_vec())
                .unwrap_or_default();
            if IN_FLIGHT.with(|f| f.get().map(|(r, _)| r)) == Some(req) {
                IN_FLIGHT.with(|f| f.set(None));
            }
            if payload.len() > EMBEDDING_LEN {
                add_reading(state, payload, ms);
            }
        }
        FromWorker::Error { error, run } => {
            IN_FLIGHT.with(|f| f.set(None));
            if run.is_none() {
                resolve(Err(error.to_string()));
            } else {
                state
                    .classifier
                    .live()
                    .set(LiveStatus::Failed(error.to_string()));
            }
        }
        _ => {}
    }
}

/// Where the audio comes from now: the microphone while it listens or
/// records, else the playing file. Returns the samples (mono, the last 3 s and
/// a little more) and their rate.
async fn current_audio(state: AppState) -> Option<(Vec<f32>, u32)> {
    let mic = state.mic.listening().get_untracked() || state.mic.recording().get_untracked();
    if mic {
        let rate = state.mic.sample_rate().get_untracked();
        if rate == 0 {
            return None;
        }
        let want = (rate as f64 * 3.1) as usize;
        let s = crate::audio::mic_backend::with_live_samples(state.is_tauri, |s| {
            s[s.len().saturating_sub(want)..].to_vec()
        });
        return (!s.is_empty()).then_some((s, rate));
    }
    if !state.playback.is_playing().get_untracked() {
        return None;
    }
    let idx = state.library.current_index().get_untracked()?;
    let (source, rate) = state.library.files().with_untracked(|f| {
        f.get(idx)
            .map(|f| (f.audio.source.clone(), f.audio.sample_rate))
    })?;
    if rate == 0 {
        return None;
    }
    let end = (state.playback.playhead_time().get_untracked().max(0.0) * rate as f64) as u64;
    let end = end.min(source.total_samples());
    let start = end.saturating_sub((rate as f64 * 3.0) as u64);
    let len = (end - start) as usize;
    if len == 0 {
        return None;
    }
    let s = crate::audio::streaming_source::read_region_ready(
        &*source,
        ChannelView::MonoMix,
        start,
        len,
    )
    .await?;
    Some((s, rate))
}

async fn run_loop(state: AppState, gen: u32) {
    loop {
        crate::web_util::sleep_ms(TICK_MS as i32).await;
        if generation() != gen {
            return;
        }
        let now = now_ms();
        // A request lost (the worker answered with something else) is given up
        // after 5 s, so the view never stalls for good.
        let busy = IN_FLIGHT.with(|f| f.get().is_some_and(|(_, t)| now - t < 5_000.0));
        if busy || now - LAST_SENT.with(|l| l.get()) < MIN_GAP_MS {
            continue;
        }
        let Some((samples, rate)) = current_audio(state).await else {
            continue;
        };
        if generation() != gen {
            return;
        }
        let req = NEXT_REQ.with(|n| {
            let r = n.get();
            n.set(r.wrapping_add(1));
            r
        });
        let sent = LINK.with(|l| {
            let l = l.borrow();
            let Some(link) = l.as_ref() else {
                return false;
            };
            let arr = js_sys::Float32Array::from(samples.as_slice());
            post(
                link,
                &ToWorker::Live {
                    req,
                    source_rate: rate,
                },
                Some((attach::SAMPLES, &arr)),
            );
            true
        });
        if !sent {
            return;
        }
        IN_FLIGHT.with(|f| f.set(Some((req, now))));
        RATE.with(|r| r.set(rate));
        LAST_SENT.with(|l| l.set(now));
    }
}

fn add_reading(state: AppState, payload: Vec<f32>, model_ms: f64) {
    let t = now_ms();
    let (embedding, scores) = payload.split_at(EMBEDDING_LEN);
    let scores = scores.to_vec();
    let embedding = embedding.to_vec();
    let at = MAP.with(|m| m.borrow().as_ref().and_then(|m| place(&scores, m)));
    let change = HISTORY.with(|h| {
        h.borrow()
            .back()
            .map(|prev| cosine_distance(&prev.embedding, &embedding))
    });
    let last = LAST_READING.with(|l| l.replace(t));
    if last > 0.0 && t - last < 5_000.0 {
        INTERVAL.with(|i| {
            let prev = i.get();
            i.set(if prev == 0.0 {
                t - last
            } else {
                prev * 0.8 + (t - last) * 0.2
            })
        });
    }
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        h.push_back(Reading {
            t,
            scores,
            embedding,
            at,
            change,
        });
        while h.front().is_some_and(|r| t - r.t > HISTORY_S * 1000.0) {
            h.pop_front();
        }
    });
    let interval_ms = INTERVAL.with(|i| i.get());
    // Round so the status line changes rarely.
    let interval_ms = (interval_ms / 50.0).round() * 50.0;
    let model_ms = (model_ms / 10.0).round() * 10.0;
    let status = LiveStatus::Ready {
        interval_ms,
        model_ms,
    };
    if state.classifier.live().with_untracked(|s| *s != status) {
        state.classifier.live().set(status);
    }
    draw(state);
}

/// Cosine distance between two embeddings: 0 for the same direction, 1 for
/// unrelated, up to 2 for opposite.
pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 1.0;
    }
    1.0 - dot / (na.sqrt() * nb.sqrt())
}

/// The marker's map position: the score-weighted mean of the top classes near
/// the top class, as the species map places a window.
pub fn place(scores: &[f32], map: &[Option<(f32, f32)>]) -> Option<(f32, f32)> {
    let top = top_classes(scores, PLACE_TOP);
    let &(c1, _) = top.first()?;
    let (x1, y1) = (*map.get(c1)?)?;
    let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
    for &(c, s) in &top {
        let Some(&Some((x, y))) = map.get(c) else {
            continue;
        };
        if ((x - x1).powi(2) + (y - y1).powi(2)).sqrt() > PLACE_RADIUS {
            continue;
        }
        sx += x * s;
        sy += y * s;
        sw += s;
    }
    Some(if sw > 0.0 {
        (sx / sw, sy / sw)
    } else {
        (x1, y1)
    })
}

/// The `n` highest scores, highest first.
pub fn top_classes(scores: &[f32], n: usize) -> Vec<(usize, f32)> {
    let mut top: Vec<(usize, f32)> = Vec::with_capacity(n + 1);
    for (i, &s) in scores.iter().enumerate() {
        if top.len() < n || s > top[top.len() - 1].1 {
            let at = top.partition_point(|&(_, t)| t >= s);
            top.insert(at, (i, s));
            top.truncate(n);
        }
    }
    top
}

/// Update the strip's rows from the history: a class gets a row once it
/// scores [`ROW_MIN`] or more, keeps its place while it is in the history, and
/// the rows with the lowest peaks go first when there are too many.
fn update_rows(history: &VecDeque<Reading>) -> Vec<u16> {
    let n = history.front().map_or(0, |r| r.scores.len());
    let mut peak = vec![0.0f32; n];
    for r in history {
        for (p, &s) in peak.iter_mut().zip(&r.scores) {
            if s > *p {
                *p = s;
            }
        }
    }
    ROWS.with(|rows| {
        let mut rows = rows.borrow_mut();
        rows.retain(|&c| peak.get(c as usize).is_some_and(|&p| p >= ROW_MIN));
        // New rows, in the order they first reached the threshold.
        for r in history {
            for (c, &s) in r.scores.iter().enumerate() {
                if s >= ROW_MIN && !rows.contains(&(c as u16)) {
                    rows.push(c as u16);
                }
            }
        }
        while rows.len() > MAX_ROWS {
            let (drop, _) = rows
                .iter()
                .enumerate()
                .min_by(|a, b| peak[*a.1 as usize].total_cmp(&peak[*b.1 as usize]))
                .expect("non-empty");
            rows.remove(drop);
        }
        rows.clone()
    })
}

fn draw(state: AppState) {
    let Some(el) = CHART.with(|c| c.borrow().clone()) else {
        return;
    };
    let min = classifier::min_confidence_untracked(state, Method::BirdNet);
    let labels = LABELS.with(|l| l.borrow().clone());
    let name = |c: usize| -> String {
        labels
            .as_ref()
            .and_then(|l| l.get(c))
            .map_or_else(|| format!("Class {c}"), |l| l.common.clone())
    };
    let json = HISTORY.with(|h| {
        let h = h.borrow();
        let now = h.back()?.t;
        let rows = update_rows(&h);
        let latest = h.back()?;
        let top: Vec<_> = top_classes(&latest.scores, 3)
            .into_iter()
            .enumerate()
            .filter(|&(i, (_, s))| i == 0 || s >= 0.05)
            .map(|(_, t)| t)
            .map(|(c, s)| (name(c), round3(s)))
            .collect();
        let ago = |r: &Reading| round3(((r.t - now) / 1000.0) as f32);
        let trail: Vec<_> = h
            .iter()
            .filter(|r| now - r.t <= TRAIL_S * 1000.0)
            .filter_map(|r| {
                let (x, y) = r.at?;
                let s = top_classes(&r.scores, 1).first().map_or(0.0, |t| t.1);
                Some((round3(x), round3(y), ago(r), round3(s)))
            })
            .collect();
        let strip: Vec<_> = h
            .iter()
            .flat_map(|r| {
                rows.iter().enumerate().map(move |(row, &c)| {
                    (
                        ago(r),
                        row,
                        round3(r.scores.get(c as usize).copied().unwrap_or(0.0)),
                    )
                })
            })
            .collect();
        let change: Vec<_> = h
            .iter()
            .filter_map(|r| Some((ago(r), round3(r.change?))))
            .collect();
        let row_names: Vec<_> = rows.iter().map(|&c| name(c as usize)).collect();
        Some(serde_json::json!({
            "min": min,
            "top": top,
            "trail": trail,
            "rows": row_names,
            "strip": strip,
            "change": change,
            "history": HISTORY_S,
            "ultrasonic": RATE.with(|r| r.get()) >= ULTRASONIC_RATE,
        }))
    });
    let Some(json) = json else { return };
    let species = MAP.with(|m| m.borrow().clone()).filter(|_| {
        !js_sys::Reflect::get(&el, &"__liveSpecies".into())
            .map(|v| v.is_truthy())
            .unwrap_or(false)
    });
    let species_json = species.map(|m| {
        let s: Vec<_> = m
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.map(|(x, y)| (round3(x), round3(y), name(i))))
            .collect();
        serde_json::json!(s).to_string()
    });
    call(
        "update",
        &[
            &el,
            &JsValue::from_str(&json.to_string()),
            &species_json.map_or(JsValue::NULL, |s| JsValue::from_str(&s)),
        ],
    );
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// Call `oversampleLiveEmbedding.<name>(args...)`.
pub fn call(name: &str, args: &[&JsValue]) -> Option<JsValue> {
    let api = js_sys::Reflect::get(&js_sys::global(), &"oversampleLiveEmbedding".into()).ok()?;
    let f: js_sys::Function = js_sys::Reflect::get(&api, &name.into())
        .ok()?
        .dyn_into()
        .ok()?;
    let a = js_sys::Array::new();
    for v in args {
        a.push(v);
    }
    f.apply(&api, &a).ok()
}

/// Labels for the view's text, shared so the status line and the chart agree.
pub fn labels() -> Option<Arc<Vec<SpeciesLabel>>> {
    LABELS.with(|l| l.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_classes_in_order() {
        let t = top_classes(&[0.1, 0.9, 0.5, 0.7], 3);
        assert_eq!(t, vec![(1, 0.9), (3, 0.7), (2, 0.5)]);
    }

    #[test]
    fn place_ignores_far_classes() {
        let map = [Some((0.0, 0.0)), Some((0.1, 0.0)), Some((0.9, 0.9))];
        let (x, y) = place(&[0.8, 0.8, 0.8], &map).unwrap();
        assert!((x - 0.05).abs() < 1e-6 && y == 0.0, "{x} {y}");
    }

    #[test]
    fn cosine_distance_basics() {
        assert!(cosine_distance(&[1.0, 0.0], &[2.0, 0.0]).abs() < 1e-6);
        assert!((cosine_distance(&[1.0, 0.0], &[0.0, 1.0]) - 1.0).abs() < 1e-6);
    }
}

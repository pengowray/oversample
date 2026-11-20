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

use crate::classifier::{self, catalog, location, ClassifierError, Method, SpeciesLabel};
use crate::format_time::format_time_display;
use crate::recording_place::{file_place, file_week, Place};
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
    /// Map position of the marker on the t-SNE layout, if a map is loaded.
    at: Option<(f32, f32)>,
    /// The same on the UMAP layout.
    at_umap: Option<(f32, f32)>,
    /// Percentile (0 to 1) on each component.
    pc: Vec<f32>,
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
    /// Principal components of the class rows, for the component maps.
    static PCS: RefCell<Option<Arc<Components>>> = const { RefCell::new(None) };
    /// UMAP places of the classes: the layout the maps start on.
    static PC_UMAP: RefCell<Option<Arc<Vec<Option<(f32, f32)>>>>> = const { RefCell::new(None) };
    /// The strip's rows, in the order they first appeared.
    static ROWS: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
    /// Interval between the last two readings, smoothed.
    static INTERVAL: Cell<f64> = const { Cell::new(0.0) };
    static LAST_READING: Cell<f64> = const { Cell::new(0.0) };
    /// Sample rate of the audio last sent.
    static RATE: Cell<u32> = const { Cell::new(0) };
    /// Which views want readings ([`LIVE_VIEW`], [`EXPLORER`]). The worker
    /// runs while this is not 0.
    static USERS: Cell<u8> = const { Cell::new(0) };
    /// Readings of this session for the Live explorer: embedding and top
    /// classes, the newest last, at most [`SESSION_MAX`].
    static SESSION: RefCell<VecDeque<SessionPoint>> = const { RefCell::new(VecDeque::new()) };
    /// The explorer's chart element, while its tab is shown.
    static EXPLORER_EL: RefCell<Option<web_sys::HtmlElement>> = const { RefCell::new(None) };
    /// Power spectrum of the last 3 s, for the explorer.
    static SPECTRUM: RefCell<Option<Spectrum>> = const { RefCell::new(None) };
    static LAST_SPECTRUM: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
    /// The paused reading: the 3 s Play would start with, while nothing plays.
    static PREVIEW: RefCell<Option<Preview>> = const { RefCell::new(None) };
    /// The paused reading's request: its number, what it reads, and when it
    /// went out. Kept after the answer, so a reading that failed is sent again
    /// only after [`PREVIEW_RETRY_MS`].
    static PREVIEW_REQ: Cell<Option<(u32, PreviewKey, f64)>> = const { Cell::new(None) };
    /// The play-start window last seen, and since when (for the debounce).
    static PREVIEW_WANT: Cell<Option<(PreviewKey, f64)>> = const { Cell::new(None) };
    /// What the chart shows for the pause now.
    static PAUSED_DRAWN: Cell<PausedView> = const { Cell::new(None) };
    /// Whether the chart's flags were drawn with a location list.
    static FLAGS_DRAWN: Cell<bool> = const { Cell::new(false) };
}

/// What the chart shows while paused: `None` when something plays (or no
/// file is open, or the live view is off), `Some(None)` while the paused
/// reading for the current play start is not in yet, `Some(Some(key))` the
/// paused reading for `key`.
type PausedView = Option<Option<PreviewKey>>;

/// Wait before a paused reading that did not arrive is asked for again.
const PREVIEW_RETRY_MS: f64 = 10_000.0;

/// Which audio a paused reading is of: the file's id, and the start and end
/// of the window in ms.
type PreviewKey = (u64, i64, i64);

/// A reading of the 3 s that Play would start with (see [`preview_window`]).
struct Preview {
    key: PreviewKey,
    scores: Vec<f32>,
    at: Option<(f32, f32)>,
    at_umap: Option<(f32, f32)>,
}

/// Wait after the play-start position stops changing before reading it.
const PREVIEW_DEBOUNCE_MS: f64 = 300.0;
/// Length of a reading, in seconds (BirdNET's window).
const WINDOW_S: f64 = 3.0;

/// The window a paused reading covers: the 3 s starting at `start`, moved
/// back to end at the end of the file when fewer than 3 s are left, and
/// shorter only when the file is. Seconds.
fn preview_window(start: f64, duration: f64) -> (f64, f64) {
    let duration = duration.max(0.0);
    let from = start.min(duration - WINDOW_S).max(0.0);
    (from, (from + WINDOW_S).min(duration))
}

/// The live view in the Call detection tab.
pub const LIVE_VIEW: u8 = 1;
/// The Live explorer tab.
pub const EXPLORER: u8 = 2;
/// Readings kept for the explorer: 5 minutes at 4 a second.
pub const SESSION_MAX: usize = 1200;
/// Shortest gap between two spectra.
const SPECTRUM_GAP_MS: f64 = 500.0;

/// One reading kept for the session.
pub struct SessionPoint {
    /// `performance.now()` in ms.
    pub t: f64,
    pub embedding: Vec<f32>,
    /// The top 5 classes and their scores.
    pub top: Vec<(usize, f32)>,
    /// All scores of the newest reading only (kept for the explorer's bars).
    pub scores: Option<Vec<f32>>,
}

/// The component maps' data with `?pc=rec` in the address: PCA of
/// recordings' embeddings (8 components), made by
/// `research/live-embedding/pc_rec.py`.
const PC_FILE: &str = "birdnet-pc.bin";
/// The default: PCA of BirdNET's class rows (12 components), made by
/// `pc_bands.py`. The count comes from the file's header.
pub const PC_FILE_ROWS: &str = "birdnet-pc-rows.bin";
/// Readings averaged for the component graphs: 4 at 0.25 s apart, so a line
/// settles within about a second.
const PC_SMOOTH: usize = 4;

/// UMAP layout of the classes (`species_umap.py`), same format as the
/// species map file.
const PC_UMAP_FILE: &str = "birdnet-umap.bin";

/// Classes sit closer together on the UMAP layout (the 10th nearest
/// neighbour is at 0.023 map units, against 0.031 on t-SNE), so the radius
/// within which the top classes pull the marker is smaller by the same share.
const PLACE_RADIUS_UMAP: f32 = PLACE_RADIUS * 0.73;

/// True for the components of BirdNET's class rows, the default. False when
/// the address asks for the components of recordings' embeddings
/// (`?pc=rec`).
fn pc_rows() -> bool {
    !web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|s| s.contains("pc=rec"))
}

/// The first principal components of BirdNET's class rows (see
/// `pc_bands.py`): each axis, where real recordings fall on it, and each
/// class's percentile rank on it.
pub struct Components {
    /// Share of the class rows' variance, per component.
    pub variance: Vec<f32>,
    axes: Vec<Vec<f32>>,
    /// Reference recordings' projections at 0, 5, ..., 100 %.
    quantiles: Vec<Vec<f32>>,
    /// Per component, each class's rank, 0 (lowest score) up. In the
    /// recordings file ranks run to 254 and 255 means no place; in the
    /// class-row file they run to 255.
    pub ranks: Vec<Vec<u8>>,
}

impl Components {
    pub fn parse(b: &[u8]) -> Option<Components> {
        if b.get(..4)? != b"OPC1" {
            return None;
        }
        let u16_at = |i: usize| Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize);
        let (n, d, c, q) = (u16_at(4)?, u16_at(6)?, u16_at(8)?, u16_at(10)?);
        if d != EMBEDDING_LEN || b.len() != 12 + 4 * (n + n * d + n * q) + n * c {
            return None;
        }
        let mut at = 12;
        let mut floats = |len: usize| -> Vec<f32> {
            let v = b[at..at + 4 * len]
                .chunks_exact(4)
                .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                .collect();
            at += 4 * len;
            v
        };
        let variance = floats(n);
        let axes = (0..n).map(|_| floats(d)).collect();
        let quantiles = (0..n).map(|_| floats(q)).collect();
        let ranks = b[at..].chunks_exact(c).map(<[u8]>::to_vec).collect();
        Some(Components {
            variance,
            axes,
            quantiles,
            ranks,
        })
    }

    /// Where `embedding` falls on each component, as a percentile (0 to 1)
    /// among the reference recordings.
    pub fn percentiles(&self, embedding: &[f32]) -> Vec<f32> {
        let norm = embedding.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        self.axes
            .iter()
            .zip(&self.quantiles)
            .map(|(axis, q)| {
                let v = axis.iter().zip(embedding).map(|(a, e)| a * e).sum::<f32>() / norm;
                interp_quantile(q, v)
            })
            .collect()
    }
}

/// The percentile of `v` among values whose evenly spaced quantiles are `q`.
fn interp_quantile(q: &[f32], v: f32) -> f32 {
    let steps = (q.len() - 1) as f32;
    let i = q.partition_point(|&x| x <= v);
    if i == 0 {
        return 0.0;
    }
    if i >= q.len() {
        return 1.0;
    }
    let (a, b) = (q[i - 1], q[i]);
    let f = if b > a { (v - a) / (b - a) } else { 0.5 };
    ((i - 1) as f32 + f) / steps
}

/// Power spectrum of the last 3 s.
pub struct Spectrum {
    pub rate: u32,
    pub psd: oversample_core::dsp::psd::PsdResult,
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

fn generation() -> u32 {
    GENERATION.with(|g| g.get())
}

/// The live view's switch: remembers it and starts or stops the readings.
pub fn set_enabled(state: AppState, on: bool) {
    state.classifier.live_on().set(on);
    set_user(state, LIVE_VIEW, on);
}

/// Say whether view `who` ([`LIVE_VIEW`] or [`EXPLORER`]) wants readings. The
/// worker starts with the first view and stops with the last; readings are
/// cleared when it stops.
pub fn set_user(state: AppState, who: u8, on: bool) {
    let before = USERS.with(|u| u.get());
    let after = if on { before | who } else { before & !who };
    USERS.with(|u| u.set(after));
    if (before == 0) != (after == 0) {
        restart(state);
    }
}

/// Stop, and start again if any view wants readings (for example after the
/// model was downloaded).
pub fn restart(state: AppState) {
    GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    LINK.with(|l| *l.borrow_mut() = None);
    IN_FLIGHT.with(|f| f.set(None));
    HISTORY.with(|h| h.borrow_mut().clear());
    SESSION.with(|h| h.borrow_mut().clear());
    SPECTRUM.with(|h| *h.borrow_mut() = None);
    crate::live_explorer::reset();
    PREVIEW.with(|p| *p.borrow_mut() = None);
    PREVIEW_REQ.with(|p| p.set(None));
    PREVIEW_WANT.with(|p| p.set(None));
    PAUSED_DRAWN.with(|p| p.set(None));
    ROWS.with(|r| r.borrow_mut().clear());
    INTERVAL.with(|i| i.set(0.0));
    if USERS.with(|u| u.get()) == 0 {
        state.classifier.live().set(LiveStatus::Off);
        return;
    }
    let gen = generation();
    spawn_local(async move {
        // Without the model the loop still runs, for the explorer's spectrum.
        if let Err(e) = start(state, gen).await {
            if generation() != gen {
                return;
            }
            LINK.with(|l| *l.borrow_mut() = None);
            state.classifier.live().set(e);
        }
        run_loop(state, gen).await;
    });
}

/// True while the worker has the model and takes requests.
fn worker_ready() -> bool {
    LINK.with(|l| l.borrow().as_ref().is_some_and(|l| l.loaded.is_none()))
}

/// Register the explorer's element (`None` when its tab closes).
pub fn set_explorer(el: Option<web_sys::HtmlElement>) {
    EXPLORER_EL.with(|c| *c.borrow_mut() = el);
}

/// The explorer's element, while its tab is shown.
pub fn explorer_el() -> Option<web_sys::HtmlElement> {
    EXPLORER_EL.with(|c| c.borrow().clone())
}

/// Read this session's readings.
pub fn with_session<R>(f: impl FnOnce(&VecDeque<SessionPoint>) -> R) -> R {
    SESSION.with(|s| f(&s.borrow()))
}

/// Read the last spectrum.
pub fn with_spectrum<R>(f: impl FnOnce(Option<&Spectrum>) -> R) -> R {
    SPECTRUM.with(|s| f(s.borrow().as_ref()))
}

/// The name of BirdNET class `c`.
pub fn class_name(c: usize) -> String {
    LABELS
        .with(|l| {
            l.borrow()
                .as_ref()
                .and_then(|l| l.get(c))
                .map(|l| l.common.clone())
        })
        .unwrap_or_else(|| format!("Class {c}"))
}

/// Register the element the readings are drawn into (`None` when it goes).
pub fn set_chart(el: Option<web_sys::HtmlElement>) {
    CHART.with(|c| *c.borrow_mut() = el);
    // A new element shows nothing yet, so the paused reading is drawn again.
    PAUSED_DRAWN.with(|p| p.set(None));
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
        if let Ok(bytes) =
            crate::components::file_sidebar::fetch_bytes(classifier::species_map::BIRDNET_MAP.file)
                .await
        {
            let m = classifier::species_map::parse_map(&bytes);
            MAP.with(|c| *c.borrow_mut() = Some(Arc::new(m)));
        }
    }
    if PCS.with(|m| m.borrow().is_none()) {
        // Optional too: without it the component maps are not shown.
        let file = if pc_rows() { PC_FILE_ROWS } else { PC_FILE };
        if let Ok(bytes) = crate::components::file_sidebar::fetch_bytes(file).await {
            if let Some(c) = Components::parse(&bytes) {
                PCS.with(|p| *p.borrow_mut() = Some(Arc::new(c)));
            }
        }
        if let Ok(bytes) = crate::components::file_sidebar::fetch_bytes(PC_UMAP_FILE).await {
            let m = classifier::species_map::parse_map(&bytes);
            PC_UMAP.with(|u| *u.borrow_mut() = Some(Arc::new(m)));
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
            let preview = PREVIEW_REQ
                .with(|p| p.get())
                .filter(|&(r, ..)| r == req)
                .map(|(_, key, _)| key);
            if let Some(key) = preview {
                if sound_on(state) {
                    // Playback or the microphone started meanwhile: the live
                    // readings take over, and the next pause asks again.
                    PREVIEW_REQ.with(|p| p.set(None));
                } else if payload.len() > EMBEDDING_LEN {
                    add_preview(state, key, &payload);
                }
            } else if payload.len() > EMBEDDING_LEN {
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

/// True while a file plays or the microphone listens or records.
fn sound_on(state: AppState) -> bool {
    state.playback.is_playing().get_untracked()
        || state.mic.listening().get_untracked()
        || state.mic.recording().get_untracked()
}

/// The window a paused reading would cover now: the current file and the 3 s
/// Play would start with. `None` with no file or a timeline open.
fn preview_key(state: AppState) -> Option<PreviewKey> {
    let start = crate::audio::playback::play_start_time(&state)?;
    let idx = state.library.current_index().get_untracked()?;
    let (id, duration) = state
        .library
        .files()
        .with_untracked(|f| f.get(idx).map(|f| (f.id, f.audio.duration_secs)))?;
    let (from, to) = preview_window(start, duration);
    let ms = |s: f64| (s * 1000.0).round() as i64;
    (to > from).then(|| (id, ms(from), ms(to)))
}

/// What the chart should show for the pause now (see [`PausedView`]).
/// `key` is [`preview_key`], asked for only while paused.
fn paused_view(state: AppState, key: impl FnOnce() -> Option<PreviewKey>) -> PausedView {
    if sound_on(state) || USERS.with(|u| u.get()) & LIVE_VIEW == 0 {
        return None;
    }
    let key = key()?;
    Some(
        PREVIEW
            .with(|p| p.borrow().as_ref().is_some_and(|p| p.key == key))
            .then_some(key),
    )
}

/// While nothing plays: redraw when the paused reading to show changes, and
/// read the play-start window once it has stayed put for
/// [`PREVIEW_DEBOUNCE_MS`].
async fn preview_tick(state: AppState, gen: u32, now: f64) {
    let key = preview_key(state);
    let view = paused_view(state, || key);
    // Redraw when the pause shows something else, or when the location list
    // for the flags has come in (it arrives after the reading).
    let flags_changed =
        view.is_some() && flag_scores(state).is_some() != FLAGS_DRAWN.with(|f| f.get());
    if PAUSED_DRAWN.with(|p| p.get()) != view || flags_changed {
        draw_view(state, view);
    }
    if USERS.with(|u| u.get()) & LIVE_VIEW == 0 {
        return;
    }
    let Some(key) = key else {
        PREVIEW_WANT.with(|p| p.set(None));
        return;
    };
    let have = matches!(view, Some(Some(_)));
    let asked = PREVIEW_REQ
        .with(|p| p.get())
        .is_some_and(|(_, k, sent)| k == key && now - sent < PREVIEW_RETRY_MS);
    if have || asked {
        return;
    }
    match PREVIEW_WANT.with(|p| p.get()) {
        Some((k, since)) if k == key => {
            if now - since < PREVIEW_DEBOUNCE_MS {
                return;
            }
        }
        _ => {
            PREVIEW_WANT.with(|p| p.set(Some((key, now))));
            return;
        }
    }
    let busy = IN_FLIGHT.with(|f| f.get().is_some_and(|(_, t)| now - t < 5_000.0));
    if !worker_ready() || busy || now - LAST_SENT.with(|l| l.get()) < MIN_GAP_MS {
        return;
    }
    let Some(idx) = state.library.current_index().get_untracked() else {
        return;
    };
    let Some((source, rate)) = state.library.files().with_untracked(|f| {
        f.get(idx)
            .filter(|f| f.id == key.0)
            .map(|f| (f.audio.source.clone(), f.audio.sample_rate))
    }) else {
        return;
    };
    if rate == 0 {
        return;
    }
    let total = source.total_samples();
    let at = |ms: i64| ((ms as f64 / 1000.0 * rate as f64) as u64).min(total);
    let (start, end) = (at(key.1), at(key.2));
    if end <= start {
        return;
    }
    // Not decoded yet (a long file still streaming in): tried again next tick.
    let Some(samples) = crate::audio::streaming_source::read_region_ready(
        &*source,
        ChannelView::MonoMix,
        start,
        (end - start) as usize,
    )
    .await
    else {
        return;
    };
    if generation() != gen || sound_on(state) || preview_key(state) != Some(key) {
        return;
    }
    if let Some(req) = send(&samples, rate, now) {
        PREVIEW_REQ.with(|p| p.set(Some((req, key, now))));
    }
}

/// Send `samples` for a reading. Returns the request's number, or `None`
/// when the worker is gone.
fn send(samples: &[f32], rate: u32, now: f64) -> Option<u32> {
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
        let arr = js_sys::Float32Array::from(samples);
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
        return None;
    }
    IN_FLIGHT.with(|f| f.set(Some((req, now))));
    RATE.with(|r| r.set(rate));
    LAST_SENT.with(|l| l.set(now));
    Some(req)
}

async fn run_loop(state: AppState, gen: u32) {
    loop {
        crate::web_util::sleep_ms(TICK_MS as i32).await;
        if generation() != gen {
            return;
        }
        let now = now_ms();
        if !sound_on(state) {
            // Nothing to read live: the paused reading instead.
            preview_tick(state, gen, now).await;
            continue;
        }
        if PAUSED_DRAWN.with(|p| p.get()).is_some() {
            // Playback started: take the paused reading off the chart now,
            // before the first live reading.
            draw(state);
        }
        // A request lost (the worker answered with something else) is given up
        // after 5 s, so the view never stalls for good.
        let busy = IN_FLIGHT.with(|f| f.get().is_some_and(|(_, t)| now - t < 5_000.0));
        let want_reading =
            worker_ready() && !busy && now - LAST_SENT.with(|l| l.get()) >= MIN_GAP_MS;
        let want_spectrum = USERS.with(|u| u.get()) & EXPLORER != 0
            && now - LAST_SPECTRUM.with(|l| l.get()) >= SPECTRUM_GAP_MS;
        if !want_reading && !want_spectrum {
            continue;
        }
        let Some((samples, rate)) = current_audio(state).await else {
            continue;
        };
        if generation() != gen {
            return;
        }
        if want_spectrum {
            LAST_SPECTRUM.with(|l| l.set(now));
            let tail = &samples[samples.len().saturating_sub((rate as f64 * 3.0) as usize)..];
            // About 25 Hz per bin for birds, coarser for bats, within 512 to 8192.
            let nfft = ((rate as f64 / 25.0) as usize)
                .next_power_of_two()
                .clamp(512, 8192);
            let psd = oversample_core::dsp::psd::compute_psd(tail, rate, nfft, None);
            SPECTRUM.with(|s| *s.borrow_mut() = Some(Spectrum { rate, psd }));
            if !want_reading {
                crate::live_explorer::draw(state);
                continue;
            }
        }
        if send(&samples, rate, now).is_none() {
            return;
        }
    }
}

/// Keep the paused reading and draw it.
fn add_preview(state: AppState, key: PreviewKey, payload: &[f32]) {
    let scores = payload[EMBEDDING_LEN..].to_vec();
    let at = MAP.with(|m| {
        m.borrow()
            .as_ref()
            .and_then(|m| place(&scores, m, PLACE_RADIUS))
    });
    let at_umap = PC_UMAP.with(|m| {
        m.borrow()
            .as_ref()
            .and_then(|m| place(&scores, m, PLACE_RADIUS_UMAP))
    });
    PREVIEW.with(|p| {
        *p.borrow_mut() = Some(Preview {
            key,
            scores,
            at,
            at_umap,
        })
    });
    draw(state);
}

/// Where the "unlikely here" flags are judged: the microphone's place is this
/// device's position (if the user gave it this session) in this week; a
/// file's is its own place, or else the device's in the file's week. The
/// same order as [`current_audio`].
fn flag_place(state: AppState) -> Option<Place> {
    let device = state.classifier.device_position().get_untracked();
    if state.mic.listening().get_untracked() || state.mic.recording().get_untracked() {
        let (lat, lon) = device?;
        let today = js_sys::Date::new_0();
        let week = oversample_ipc::classifier::week_of(today.get_month() + 1, today.get_date());
        return Some(Place { lat, lon, week });
    }
    let idx = state.library.current_index().get_untracked()?;
    state.library.files().with_untracked(|f| {
        let file = f.get(idx)?;
        file_place(file).or_else(|| {
            device.map(|(lat, lon)| Place {
                lat,
                lon,
                week: file_week(file),
            })
        })
    })
}

/// The location list for the flags, when "Mark birds unlikely here" is on
/// and there is a place. The first call for a place starts working it out.
fn flag_scores(state: AppState) -> Option<Arc<Vec<f32>>> {
    if !state.call_detection.mark_unexpected().get_untracked() {
        return None;
    }
    location::scores_at(state, flag_place(state)?)
}

fn add_reading(state: AppState, payload: Vec<f32>, model_ms: f64) {
    let t = now_ms();
    let (embedding, scores) = payload.split_at(EMBEDDING_LEN);
    let scores = scores.to_vec();
    let embedding = embedding.to_vec();
    let at = MAP.with(|m| {
        m.borrow()
            .as_ref()
            .and_then(|m| place(&scores, m, PLACE_RADIUS))
    });
    let at_umap = PC_UMAP.with(|m| {
        m.borrow()
            .as_ref()
            .and_then(|m| place(&scores, m, PLACE_RADIUS_UMAP))
    });
    let pc = PCS.with(|p| {
        p.borrow()
            .as_ref()
            .map_or_else(Vec::new, |p| p.percentiles(&embedding))
    });
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
    if USERS.with(|u| u.get()) & EXPLORER != 0 {
        SESSION.with(|ss| {
            let mut ss = ss.borrow_mut();
            if let Some(last) = ss.back_mut() {
                last.scores = None;
            }
            ss.push_back(SessionPoint {
                t,
                embedding: embedding.clone(),
                top: top_classes(&scores, 5),
                scores: Some(scores.clone()),
            });
            while ss.len() > SESSION_MAX {
                ss.pop_front();
            }
        });
    }
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        h.push_back(Reading {
            t,
            scores,
            embedding,
            at,
            at_umap,
            pc,
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
    crate::live_explorer::draw(state);
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
pub fn place(scores: &[f32], map: &[Option<(f32, f32)>], radius: f32) -> Option<(f32, f32)> {
    let top = top_classes(scores, PLACE_TOP);
    let &(c1, _) = top.first()?;
    let (x1, y1) = (*map.get(c1)?)?;
    let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
    for &(c, s) in &top {
        let Some(&Some((x, y))) = map.get(c) else {
            continue;
        };
        if ((x - x1).powi(2) + (y - y1).powi(2)).sqrt() > radius {
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
    draw_view(state, paused_view(state, || preview_key(state)));
}

/// Draw, with `view` the pause's part of the chart (see [`PausedView`]).
fn draw_view(state: AppState, view: PausedView) {
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
    // "Unlikely here" flags, by class.
    let flags = flag_scores(state);
    let unlikely = |c: usize| -> bool {
        flags
            .as_ref()
            .zip(labels.as_ref().and_then(|l| l.get(c)))
            .and_then(|(s, l)| location::unlikely_in(s, l))
            .unwrap_or(false)
    };
    FLAGS_DRAWN.with(|f| f.set(flags.is_some()));
    let shown = view.flatten();
    // Paused, with the paused reading not in yet: the live reading from
    // before the pause is hidden, so it is not taken for the play start.
    let pending = matches!(view, Some(None));
    let was_shown = PAUSED_DRAWN.with(|p| p.replace(view)).flatten();
    // The paused reading: its top classes and its place on both layouts.
    let preview = shown.and_then(|key| {
        PREVIEW.with(|p| {
            let p = p.borrow();
            let p = p.as_ref()?;
            let (x, y) = p.at.unwrap_or((0.0, 0.0));
            let (ux, uy) = p.at_umap.or(p.at).unwrap_or((0.0, 0.0));
            let s = top_classes(&p.scores, 1).first().map_or(0.0, |t| t.1);
            Some((
                p.scores.clone(),
                // No place on the map (the map file did not load): text only.
                p.at.map(|_| {
                    serde_json::json!({
                        "x": round3(x), "y": round3(y), "ux": round3(ux), "uy": round3(uy),
                        "s": round3(s),
                    })
                }),
                format!(
                    "{} to {}",
                    format_time_display(key.1 as f64 / 1000.0, 1),
                    format_time_display(key.2 as f64 / 1000.0, 1)
                ),
            ))
        })
    });
    let json = HISTORY.with(|h| {
        let h = h.borrow();
        // Nothing heard and no paused reading: nothing to draw, unless a
        // paused reading is on the chart and has to go.
        if h.is_empty() && preview.is_none() && was_shown.is_none() && !pending {
            return None;
        }
        let now = h.back().map_or(0.0, |r| r.t);
        let rows = update_rows(&h);
        let top_scores = match &preview {
            Some(p) => p.0.as_slice(),
            None if pending => &[],
            None => h.back().map_or(&[][..], |r| r.scores.as_slice()),
        };
        let top: Vec<_> = top_classes(top_scores, 3)
            .into_iter()
            .enumerate()
            .filter(|&(i, (_, s))| i == 0 || s >= 0.05)
            .map(|(_, t)| t)
            .map(|(c, s)| (name(c), round3(s), unlikely(c)))
            .collect();
        let ago = |r: &Reading| round3(((r.t - now) / 1000.0) as f32);
        let trail: Vec<_> = h
            .iter()
            .filter(|r| now - r.t <= TRAIL_S * 1000.0)
            .filter_map(|r| {
                let (x, y) = r.at?;
                let s = top_classes(&r.scores, 1).first().map_or(0.0, |t| t.1);
                // The UMAP place follows; the t-SNE place stands in for it
                // when the UMAP file did not load.
                let (ux, uy) = r.at_umap.unwrap_or((x, y));
                Some((round3(x), round3(y), ago(r), round3(s), round3(ux), round3(uy)))
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
        let rows_unlikely: Vec<bool> = rows.iter().map(|&c| unlikely(c as usize)).collect();
        // Each reading's percentile on each component, as the mean of the
        // last few readings so a line does not jump from one reading to the
        // next: [secondsAgo, p1, p2, ...].
        let readings: Vec<&Reading> = h.iter().collect();
        let pc_hist: Vec<Vec<f32>> = (0..readings.len())
            .filter(|&i| !readings[i].pc.is_empty())
            .map(|i| {
                let from = (i + 1).saturating_sub(PC_SMOOTH);
                let group: Vec<&Reading> = readings[from..=i]
                    .iter()
                    .copied()
                    .filter(|r| r.pc.len() == readings[i].pc.len())
                    .collect();
                let mut row = vec![ago(readings[i])];
                for k in 0..readings[i].pc.len() {
                    let sum: f32 = group.iter().map(|r| r.pc[k]).sum();
                    row.push(round3(sum / group.len() as f32));
                }
                row
            })
            .collect();
        let pc: Vec<f32> = pc_hist.last().map_or_else(Vec::new, |r| r[1..].to_vec());
        Some(serde_json::json!({
            "pc": pc,
            "pcHist": pc_hist,
            "min": min,
            "top": top,
            "trail": trail,
            "rows": row_names,
            "rowsUnlikely": rows_unlikely,
            "heard": !h.is_empty(),
            "preview": preview.as_ref().map(|p| serde_json::json!({
                "at": p.1,
                "range": p.2,
            })),
            "pending": pending,
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
    let pcs = PCS.with(|p| p.borrow().clone());
    let mut pc_json = None;
    let species_json = species.map(|m| {
        let kept: Vec<usize> = (0..m.len()).filter(|&i| m[i].is_some()).collect();
        let s: Vec<_> = kept
            .iter()
            .map(|&i| {
                let (x, y) = m[i].expect("kept");
                let sci = labels
                    .as_ref()
                    .and_then(|l| l.get(i))
                    .map_or_else(String::new, |l| l.scientific.clone());
                (round3(x), round3(y), name(i), sci)
            })
            .collect();
        // Each component's ranks, in the same order as the points.
        pc_json = pcs.as_ref().map(|p| {
            let ranks: Vec<Vec<u8>> = p
                .ranks
                .iter()
                .map(|r| kept.iter().map(|&i| r.get(i).copied().unwrap_or(0)).collect())
                .collect();
            let basis = if pc_rows() { "rows" } else { "recordings" };
            let umap: Option<Vec<(f32, f32)>> = PC_UMAP.with(|u| {
                u.borrow().as_ref().map(|u| {
                    kept.iter()
                        .map(|&i| {
                            let (x, y) = u.get(i).copied().flatten().unwrap_or((0.0, 0.0));
                            (round3(x), round3(y))
                        })
                        .collect()
                })
            });
            serde_json::json!({
                "variance": p.variance,
                "ranks": ranks,
                "basis": basis,
                "umap": umap,
            })
            .to_string()
        });
        serde_json::json!(s).to_string()
    });
    let str_or_null = |s: Option<String>| s.map_or(JsValue::NULL, |s| JsValue::from_str(&s));
    call(
        "update",
        &[
            &el,
            &JsValue::from_str(&json.to_string()),
            &str_or_null(species_json),
            &str_or_null(pc_json),
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
        let (x, y) = place(&[0.8, 0.8, 0.8], &map, PLACE_RADIUS).unwrap();
        assert!((x - 0.05).abs() < 1e-6 && y == 0.0, "{x} {y}");
    }

    #[test]
    fn quantile_interpolation() {
        let q = [0.0, 1.0, 2.0];
        assert_eq!(interp_quantile(&q, -1.0), 0.0);
        assert_eq!(interp_quantile(&q, 3.0), 1.0);
        assert!((interp_quantile(&q, 0.5) - 0.25).abs() < 1e-6);
        assert!((interp_quantile(&q, 1.5) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn preview_window_starts_at_play_start() {
        assert_eq!(preview_window(12.4, 60.0), (12.4, 15.4));
        assert_eq!(preview_window(0.0, 60.0), (0.0, 3.0));
    }

    #[test]
    fn preview_window_near_the_end_is_the_last_3_s() {
        assert_eq!(preview_window(59.0, 60.0), (57.0, 60.0));
        assert_eq!(preview_window(60.0, 60.0), (57.0, 60.0));
    }

    #[test]
    fn preview_window_of_a_short_file_is_the_whole_file() {
        assert_eq!(preview_window(1.0, 2.0), (0.0, 2.0));
        assert_eq!(preview_window(0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn component_files_parse() {
        let rows = Components::parse(include_bytes!(
            "../research/live-embedding/birdnet-pc-rows.bin"
        ))
        .expect("class-row components");
        assert_eq!(rows.variance.len(), 12);
        assert_eq!(rows.ranks.len(), 12);
        assert!(rows.ranks.iter().all(|r| r.len() == 6522));
        let rec = Components::parse(include_bytes!("../research/live-embedding/birdnet-pc.bin"))
            .expect("recording components");
        assert_eq!(rec.variance.len(), 8);
    }

    #[test]
    fn cosine_distance_basics() {
        assert!(cosine_distance(&[1.0, 0.0], &[2.0, 0.0]).abs() < 1e-6);
        assert!((cosine_distance(&[1.0, 0.0], &[0.0, 1.0]) - 1.0).abs() < 1e-6);
    }
}

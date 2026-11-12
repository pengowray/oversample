//! The app's side of the BirdNET worker: starting it, sending models and audio,
//! and turning its answers into state.
//!
//! One worker at a time, kept in a thread-local (a `Worker` handle is not
//! `Send`, so it can't live in the store). Requests that expect an answer
//! register a waiter; dropping the waiters (a new run, a cancel, a worker crash)
//! wakes every waiting task with an error, so nothing hangs.

use super::types::WindowScores;
use super::ClassifierError;
use crate::state::{AppState, ClassifierStateStoreFields};
use futures_channel::oneshot;
use leptos::prelude::*;
use oversample_ipc::classifier::{attach, FromWorker, Method, ModelSlot, Score, ToWorker};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker};

/// The loader Trunk writes next to `index.html` (`data-loader-shim`). Resolved
/// against the page's URL, so it works under any `--public-url`.
const WORKER_URL: &str = "birdnet-worker_loader.js";
/// Shortest gap between two pushes of new windows into the store.
const FLUSH_MS: f64 = 100.0;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Wait {
    Ready,
    Loaded(String),
    ChunkDone(u32, u32),
    Done(u32),
    Rescored(u32),
    Occurrence(u32),
}

impl Wait {
    fn run(&self) -> Option<u32> {
        match self {
            Wait::ChunkDone(r, _) | Wait::Done(r) | Wait::Rescored(r) => Some(*r),
            _ => None,
        }
    }
}

/// What a waiter gets back.
#[derive(Clone)]
pub enum Reply {
    Ok,
    Rescored(Vec<Vec<Score>>),
    /// The Geomodel's score for each of its species.
    Occurrence(Vec<f32>),
}

pub type Waiter = oneshot::Receiver<Result<Reply, ClassifierError>>;

/// Where the windows of the run in progress go.
struct Active {
    run: u32,
    key: (u64, Method),
    windows: Vec<WindowScores>,
    embeddings: Vec<f32>,
    last_flush: f64,
}

struct Client {
    state: AppState,
    worker: Worker,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
    ready: bool,
    /// The model in each slot, by file id.
    audio: Option<String>,
    head: Option<String>,
    /// True once a Perch model has been loaded, false once a BirdNET one has.
    /// The worker's memory never shrinks, so changing family restarts it.
    perch: Option<bool>,
    waiters: Vec<(Wait, oneshot::Sender<Result<Reply, ClassifierError>>)>,
    active: Option<Active>,
}

thread_local! {
    static CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) };
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

fn worker_err(message: impl Into<String>) -> ClassifierError {
    ClassifierError::Worker {
        message: message.into(),
    }
}

/// Start the worker if it isn't running, and wait until it takes messages.
pub async fn ensure_started(state: AppState) -> Result<(), Stopped> {
    let waiter = CLIENT.with(|c| -> Result<Option<Waiter>, ClassifierError> {
        let mut c = c.borrow_mut();
        if let Some(client) = c.as_mut() {
            if client.ready {
                return Ok(None);
            }
            return Ok(Some(client.wait(Wait::Ready)));
        }
        let worker =
            Worker::new(WORKER_URL).map_err(|e| worker_err(format!("could not start: {e:?}")))?;
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(on_message);
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(on_error);
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        let mut client = Client {
            state,
            worker,
            _on_message: on_message,
            _on_error: on_error,
            ready: false,
            audio: None,
            head: None,
            perch: None,
            waiters: Vec::new(),
            active: None,
        };
        let w = client.wait(Wait::Ready);
        *c = Some(client);
        Ok(Some(w))
    })?;
    match waiter {
        Some(w) => wait(w).await.map(|_| ()),
        None => Ok(()),
    }
}

/// Await a waiter. A dropped waiter (superseded or stopped) is
/// [`Stopped::Superseded`].
pub async fn wait(w: Waiter) -> Result<Reply, Stopped> {
    match w.await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => Err(Stopped::Failed(e)),
        Err(_) => Err(Stopped::Superseded),
    }
}

/// Why a task stopped early.
#[derive(Debug)]
pub enum Stopped {
    /// A newer run, a cancel or `unload` took over; leave the state alone.
    Superseded,
    Failed(ClassifierError),
}

impl From<ClassifierError> for Stopped {
    fn from(e: ClassifierError) -> Self {
        Stopped::Failed(e)
    }
}

impl From<Stopped> for Result<(), ClassifierError> {
    fn from(s: Stopped) -> Self {
        match s {
            Stopped::Superseded => Ok(()),
            Stopped::Failed(e) => Err(e),
        }
    }
}

impl Client {
    fn wait(&mut self, what: Wait) -> Waiter {
        let (tx, rx) = oneshot::channel();
        self.waiters.push((what, tx));
        rx
    }

    /// Answer every waiter for `what` (two tasks can wait for the same model).
    fn resolve(&mut self, what: &Wait, reply: Result<Reply, ClassifierError>) {
        let (hit, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|(w, _)| w == what);
        self.waiters = keep;
        for (_, tx) in hit {
            let _ = tx.send(reply.clone());
        }
    }

    fn post(&self, msg: &ToWorker, extra: Option<(&str, &JsValue)>, transfer: Option<&JsValue>) {
        let Ok(inner) = serde_wasm_bindgen::to_value(msg) else {
            return;
        };
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &attach::MSG.into(), &inner);
        if let Some((name, value)) = extra {
            let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(name), value);
        }
        let _ = match transfer {
            Some(t) => self
                .worker
                .post_message_with_transfer(&obj, &js_sys::Array::of1(t)),
            None => self.worker.post_message(&obj),
        };
    }

    /// Move pending windows into the store.
    fn take_flush(&mut self) -> Option<((u64, Method), Vec<WindowScores>, Vec<f32>)> {
        let a = self.active.as_mut()?;
        if a.windows.is_empty() {
            return None;
        }
        a.last_flush = now_ms();
        Some((
            a.key,
            std::mem::take(&mut a.windows),
            std::mem::take(&mut a.embeddings),
        ))
    }
}

fn flush_into_state(state: AppState, flush: Option<((u64, Method), Vec<WindowScores>, Vec<f32>)>) {
    let Some((key, windows, embeddings)) = flush else {
        return;
    };
    state.classifier.results().update(|m| {
        if let Some(r) = m.get_mut(&key) {
            r.windows.extend(windows);
        }
    });
    state.classifier.embeddings().update(|m| {
        m.entry(key).or_default().extend(embeddings);
    });
}

fn on_message(ev: MessageEvent) {
    let data = ev.data();
    let inner = js_sys::Reflect::get(&data, &attach::MSG.into()).unwrap_or(JsValue::UNDEFINED);
    let msg: FromWorker = match serde_wasm_bindgen::from_value(inner) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("birdnet worker: unreadable message: {e}");
            return;
        }
    };
    // Decide under the borrow; touch the store after it, so a store update can
    // never re-enter the client.
    let (state, flush) = CLIENT.with(|c| {
        let mut c = c.borrow_mut();
        let Some(client) = c.as_mut() else {
            return (None, None);
        };
        let state = client.state;
        let mut flush = None;
        match msg {
            FromWorker::Ready => {
                client.ready = true;
                client.resolve(&Wait::Ready, Ok(Reply::Ok));
            }
            FromWorker::Loaded { key, ms } => {
                log::info!("birdnet worker: built {key} in {ms:.0} ms");
                client.resolve(&Wait::Loaded(key), Ok(Reply::Ok));
            }
            FromWorker::Window {
                run,
                start_s,
                end_s,
                top,
                ..
            } => {
                if let Some(a) = client.active.as_mut().filter(|a| a.run == run) {
                    a.windows.push(WindowScores {
                        start_s,
                        end_s,
                        top,
                    });
                    if let Ok(e) = js_sys::Reflect::get(&data, &attach::EMBEDDING.into()) {
                        if let Some(arr) = e.dyn_ref::<js_sys::Float32Array>() {
                            a.embeddings.extend(arr.to_vec());
                        }
                    }
                    if now_ms() - a.last_flush >= FLUSH_MS {
                        flush = client.take_flush();
                    }
                }
            }
            FromWorker::ChunkDone { run, seq } => {
                flush = client.take_flush();
                client.resolve(&Wait::ChunkDone(run, seq), Ok(Reply::Ok));
            }
            FromWorker::Done { run, .. } => {
                flush = client.take_flush();
                client.resolve(&Wait::Done(run), Ok(Reply::Ok));
            }
            FromWorker::Rescored { run, top } => {
                client.resolve(&Wait::Rescored(run), Ok(Reply::Rescored(top)));
            }
            FromWorker::Occurrence { req, scores } => {
                client.resolve(&Wait::Occurrence(req), Ok(Reply::Occurrence(scores)));
            }
            FromWorker::Error { run, error } => {
                flush = client.take_flush();
                // Fail every waiter the error concerns: the run's, or (for a
                // model that would not load) the load's.
                let (hit, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut client.waiters)
                    .into_iter()
                    .partition(|(w, _)| match run {
                        Some(r) => w.run() == Some(r),
                        None => matches!(w, Wait::Loaded(_) | Wait::Occurrence(_)),
                    });
                client.waiters = keep;
                if run.is_none() {
                    // The worker dropped the model it was replacing.
                    client.audio = None;
                    client.head = None;
                }
                for (_, tx) in hit {
                    let _ = tx.send(Err(error.clone()));
                }
            }
        }
        (Some(state), flush)
    });
    if let Some(state) = state {
        flush_into_state(state, flush);
    }
}

fn on_error(ev: web_sys::Event) {
    let message = ev
        .dyn_ref::<web_sys::ErrorEvent>()
        .map(|e| e.message())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "the worker could not load or crashed".into());
    log::error!("birdnet worker: {message}");
    // Drop the client: the next request starts a fresh worker.
    let waiters = CLIENT.with(|c| {
        let mut c = c.borrow_mut();
        let client = c.take()?;
        client.worker.terminate();
        Some(client.waiters)
    });
    for (_, tx) in waiters.unwrap_or_default() {
        let _ = tx.send(Err(worker_err(message.clone())));
    }
}

/// Stop the worker if it holds models of the other family (BirdNET or Perch),
/// so loading this family starts from fresh memory. Call before
/// [`ensure_started`].
pub fn ensure_family(perch: bool) {
    let other = CLIENT.with(|c| {
        c.borrow()
            .as_ref()
            .is_some_and(|cl| cl.perch.is_some_and(|p| p != perch))
    });
    if other {
        terminate();
    }
}

/// Make sure the worker holds model `file` in its slot, reading and sending it
/// if not.
pub async fn ensure_model(file: &'static super::catalog::ModelFile) -> Result<(), Stopped> {
    let part = file
        .part()
        .ok_or_else(|| worker_err(format!("{} is not a model", file.id)))?;
    let key = file.id.to_string();
    let loaded = CLIENT.with(|c| {
        c.borrow().as_ref().is_some_and(|cl| match part.slot() {
            ModelSlot::Audio => cl.audio.as_deref() == Some(&key),
            ModelSlot::Head => cl.head.as_deref() == Some(&key),
        })
    });
    if loaded {
        return Ok(());
    }
    let bytes = super::store::read_js(file).await?;
    let waiter = CLIENT.with(|c| {
        let mut c = c.borrow_mut();
        let client = c.as_mut()?;
        match part.slot() {
            ModelSlot::Audio => client.audio = None,
            ModelSlot::Head => client.head = None,
        }
        client.perch = Some(part.is_perch());
        let buf = bytes.buffer();
        client.post(
            &ToWorker::Load {
                key: key.clone(),
                part,
            },
            Some((attach::BYTES, &buf)),
            Some(&buf),
        );
        Some(client.wait(Wait::Loaded(key.clone())))
    });
    let Some(waiter) = waiter else {
        return Err(Stopped::Superseded);
    };
    wait(waiter).await?;
    CLIENT.with(|c| {
        if let Some(cl) = c.borrow_mut().as_mut() {
            match part.slot() {
                ModelSlot::Audio => cl.audio = Some(key),
                ModelSlot::Head => cl.head = Some(key),
            }
        }
    });
    Ok(())
}

/// Start run `run`; windows go to the results under `key`.
pub fn start(run: u32, key: (u64, Method), source_rate: u32) -> Result<(), Stopped> {
    CLIENT.with(|c| {
        let mut c = c.borrow_mut();
        let client = c.as_mut().ok_or(Stopped::Superseded)?;
        client.active = Some(Active {
            run,
            key,
            windows: Vec::new(),
            embeddings: Vec::new(),
            last_flush: now_ms(),
        });
        client.post(
            &ToWorker::Start {
                run,
                method: key.1,
                source_rate,
            },
            None,
            None,
        );
        Ok(())
    })
}

/// Send a piece of audio; the waiter resolves when the worker has used it.
pub fn send_chunk(run: u32, seq: u32, samples: &[f32]) -> Result<Waiter, Stopped> {
    CLIENT.with(|c| {
        let mut c = c.borrow_mut();
        let client = c.as_mut().ok_or(Stopped::Superseded)?;
        let arr = js_sys::Float32Array::from(samples);
        let buf = arr.buffer();
        client.post(
            &ToWorker::Chunk { run, seq },
            Some((attach::SAMPLES, &arr)),
            Some(&buf),
        );
        Ok(client.wait(Wait::ChunkDone(run, seq)))
    })
}

/// No more audio; the waiter resolves when the last window is out.
pub fn finish(run: u32) -> Result<Waiter, Stopped> {
    CLIENT.with(|c| {
        let mut c = c.borrow_mut();
        let client = c.as_mut().ok_or(Stopped::Superseded)?;
        client.post(&ToWorker::Finish { run }, None, None);
        Ok(client.wait(Wait::Done(run)))
    })
}

/// Score `windows` embeddings with the loaded head.
pub fn rescore(run: u32, windows: u32, embeddings: &[f32]) -> Result<Waiter, Stopped> {
    CLIENT.with(|c| {
        let mut c = c.borrow_mut();
        let client = c.as_mut().ok_or(Stopped::Superseded)?;
        let arr = js_sys::Float32Array::from(embeddings);
        let buf = arr.buffer();
        client.post(
            &ToWorker::Rescore { run, windows },
            Some((attach::EMBEDDING, &arr)),
            Some(&buf),
        );
        Ok(client.wait(Wait::Rescored(run)))
    })
}

/// Score a place and week with the Geomodel (`bytes`, its file), which the
/// worker builds and drops for this one query. `week` 0 is the whole year.
pub fn occurrence(
    req: u32,
    lat: f32,
    lon: f32,
    week: u32,
    bytes: &js_sys::Uint8Array,
) -> Result<Waiter, Stopped> {
    CLIENT.with(|c| {
        let mut c = c.borrow_mut();
        let client = c.as_mut().ok_or(Stopped::Superseded)?;
        let buf = bytes.buffer();
        client.post(
            &ToWorker::Occurrence {
                req,
                lat,
                lon,
                week,
            },
            Some((attach::BYTES, &buf)),
            Some(&buf),
        );
        Ok(client.wait(Wait::Occurrence(req)))
    })
}

/// Stop any run in the worker and wake its waiters as superseded. The models
/// stay loaded.
pub fn cancel_active() {
    let state_flush = CLIENT.with(|c| {
        let mut c = c.borrow_mut();
        let client = c.as_mut()?;
        let flush = client.take_flush();
        if let Some(a) = client.active.take() {
            client.post(&ToWorker::Cancel { run: a.run }, None, None);
        }
        // Dropping the senders wakes their tasks with "superseded".
        client
            .waiters
            .retain(|(w, _)| matches!(w, Wait::Ready | Wait::Loaded(_)));
        Some((client.state, flush))
    });
    if let Some((state, flush)) = state_flush {
        flush_into_state(state, flush);
    }
}

/// Stop the worker and free its memory (about 300 MB with the model loaded).
pub fn terminate() {
    let client = CLIENT.with(|c| c.borrow_mut().take());
    if let Some(client) = client {
        client.worker.terminate();
        // Dropping `client` drops its waiters, waking their tasks.
    }
}

/// True while a worker exists.
pub fn is_running() -> bool {
    CLIENT.with(|c| c.borrow().is_some())
}

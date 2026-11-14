//! The learned bat-call denoiser, from the app's side.
//!
//! The network runs in a Web Worker of its own: the same WASM as the
//! classifier worker (`birdnet-worker`), started a second time, so a denoise
//! and a classifier run never wait for each other and the classifier's worker
//! restarts don't touch it. The worker is stopped after every recording, which
//! gives its memory back.
//!
//! One recording at a time. The result is added to the file list as a new
//! recording, so playback, the spectrogram, export and the call detector all
//! work on it without knowing about the denoiser.

use crate::audio::source::ChannelView;
use crate::classifier::{catalog, store, ClassifierError};
use crate::state::{AppState, DenoiseStateStoreFields, DenoiseStatus};
use futures_channel::oneshot;
use leptos::prelude::*;
use oversample_ipc::classifier::{attach, FromWorker, ModelPart, ToWorker};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker};

/// The loader Trunk writes for the model worker.
const WORKER_URL: &str = "birdnet-worker_loader.js";

/// Lowest sample rate the denoiser takes. It was trained on recordings from
/// 250 to 500 kHz, used sample for sample.
pub const MIN_RATE: u32 = 250_000;
/// Longest recording, in samples (62 s at 384 kHz). The worker holds the
/// recording four times over while it works.
pub const MAX_SAMPLES: usize = 24_000_000;

/// The menu's strings, in one place. Drafted from a brief by a writer that
/// had not seen the code (2026-11-14).
pub mod text {
    pub const TITLE: &str = "Bat Call Model";
    pub const BUTTON: &str = "Make Denoised Copy";
    pub const CANCEL: &str = "Cancel";
    pub const STARTING: &str = "Loading recording and model\u{2026}";
    pub const STEREO: &str = "The denoised copy will be mono.";
    pub const ADDED: &str = "Denoised copy added to the file list";
    pub const TOOLTIP: &str = "Keeps 10% of the original sound, so noise is lowered, not removed.";

    pub fn help(model_stored: bool, size: &str) -> String {
        if model_stored {
            "Adds a new recording and keeps the original.".to_string()
        } else {
            format!("Adds a new recording. Downloads a {size} model first.")
        }
    }
    pub fn downloading(percent: u32) -> String {
        format!("Downloading model: {percent}%")
    }
    pub fn running(percent: u32) -> String {
        format!("Denoising: {percent}%")
    }
    pub fn download_failed(error: &str) -> String {
        format!("Model download failed: {error}")
    }
    pub fn failed(error: &str) -> String {
        format!("Denoising failed: {error}")
    }
    pub fn rate_too_low(min_khz: u32, khz: u32) -> String {
        format!("Needs a sample rate of {min_khz} kHz or more. This recording: {khz} kHz.")
    }
    pub fn too_long(seconds: f64, max_seconds: f64) -> String {
        format!(
            "Recording is too long: {seconds:.0} s. Maximum at this sample rate: {max_seconds:.0} s."
        )
    }
}

/// What the worker sent: a message and the samples that came with it.
type Incoming = Result<(FromWorker, Option<js_sys::Float32Array>), String>;

/// Messages from the worker, in order, for the one task that reads them.
#[derive(Default)]
struct Mailbox {
    queue: VecDeque<Incoming>,
    waiting: Option<oneshot::Sender<Incoming>>,
}

impl Mailbox {
    fn push(&mut self, item: Incoming) {
        match self.waiting.take() {
            Some(tx) => {
                let _ = tx.send(item);
            }
            None => self.queue.push_back(item),
        }
    }
}

struct Link {
    worker: Worker,
    mailbox: Rc<RefCell<Mailbox>>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.worker.terminate();
    }
}

thread_local! {
    /// The worker of the recording in progress. Dropping it stops the worker.
    static LINK: RefCell<Option<Link>> = const { RefCell::new(None) };
    /// Goes up with every start and cancel. A task that finds it changed stops.
    static GENERATION: Cell<u32> = const { Cell::new(0) };
}

fn start_worker() -> Result<(), String> {
    let worker = Worker::new(WORKER_URL).map_err(|e| format!("{e:?}"))?;
    let mailbox = Rc::new(RefCell::new(Mailbox::default()));
    let inbox = mailbox.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
        let data = ev.data();
        let inner = js_sys::Reflect::get(&data, &attach::MSG.into()).unwrap_or(JsValue::UNDEFINED);
        let item = match serde_wasm_bindgen::from_value::<FromWorker>(inner) {
            Ok(msg) => {
                let samples = js_sys::Reflect::get(&data, &attach::SAMPLES.into())
                    .ok()
                    .filter(|v| !v.is_undefined() && !v.is_null())
                    .map(|v| v.unchecked_into::<js_sys::Float32Array>());
                Ok((msg, samples))
            }
            Err(e) => Err(format!("unreadable message: {e}")),
        };
        inbox.borrow_mut().push(item);
    });
    let inbox = mailbox.clone();
    let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
        let message = ev
            .dyn_ref::<web_sys::ErrorEvent>()
            .map(|e| e.message())
            .unwrap_or_else(|| "the worker stopped".to_string());
        inbox.borrow_mut().push(Err(message));
    });
    worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    LINK.with(|l| {
        *l.borrow_mut() = Some(Link {
            worker,
            mailbox,
            _on_message: on_message,
            _on_error: on_error,
        })
    });
    Ok(())
}

fn post(msg: &ToWorker, extra: Option<(&str, &JsValue)>) {
    LINK.with(|l| {
        let l = l.borrow();
        let Some(link) = l.as_ref() else {
            return;
        };
        let Ok(inner) = serde_wasm_bindgen::to_value(msg) else {
            return;
        };
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &attach::MSG.into(), &inner);
        let _ = match extra {
            Some((name, value)) => {
                let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(name), value);
                link.worker
                    .post_message_with_transfer(&obj, &js_sys::Array::of1(value))
            }
            None => link.worker.post_message(&obj),
        };
    });
}

/// The worker's next message. `None` when the worker is gone (a cancel).
async fn next() -> Option<Incoming> {
    let rx = LINK.with(|l| {
        let l = l.borrow();
        let link = l.as_ref()?;
        let mut mailbox = link.mailbox.borrow_mut();
        if let Some(item) = mailbox.queue.pop_front() {
            return Some(Ok(item));
        }
        let (tx, rx) = oneshot::channel();
        mailbox.waiting = Some(tx);
        Some(Err(rx))
    })?;
    match rx {
        Ok(item) => Some(item),
        Err(rx) => rx.await.ok(),
    }
}

fn stop_worker() {
    LINK.with(|l| *l.borrow_mut() = None);
}

/// Look up whether the model file is on this device, for the menu's help line.
pub fn check_model_stored(state: AppState) {
    wasm_bindgen_futures::spawn_local(async move {
        let stored = store::is_downloaded(&catalog::DENOISER).await;
        state.denoise.model_stored().set(Some(stored));
    });
}

/// The model file's size, as the menu shows it: "0.3 MB".
pub fn model_size_text() -> String {
    format!("{:.1} MB", catalog::DENOISER.size as f64 / 1e6)
}

/// True while a recording is being denoised (or its model downloaded).
pub fn is_busy(state: AppState) -> bool {
    !matches!(
        state.denoise.status().get(),
        DenoiseStatus::Idle | DenoiseStatus::Failed(_)
    )
}

/// Why the open recording can't be denoised, or `None` when it can.
pub fn refusal(sample_rate: u32, samples: usize) -> Option<String> {
    if sample_rate < MIN_RATE {
        return Some(text::rate_too_low(MIN_RATE / 1000, sample_rate / 1000));
    }
    if samples > MAX_SAMPLES {
        return Some(text::too_long(
            samples as f64 / sample_rate as f64,
            MAX_SAMPLES as f64 / sample_rate as f64,
        ));
    }
    None
}

/// Stop the recording in progress, if any.
pub fn cancel(state: AppState) {
    GENERATION.with(|g| g.set(g.get().wrapping_add(1)));
    stop_worker();
    state.denoise.status().set(DenoiseStatus::Idle);
}

/// Denoise the open recording and add the result to the file list.
pub fn start(state: AppState) {
    if is_busy(state) {
        return;
    }
    let generation = GENERATION.with(|g| {
        g.set(g.get().wrapping_add(1));
        g.get()
    });
    wasm_bindgen_futures::spawn_local(async move {
        let result = run(state, generation).await;
        stop_worker();
        if GENERATION.with(|g| g.get()) != generation {
            // Cancelled, or another recording started: its status stands.
            return;
        }
        match result {
            Ok(()) => state.denoise.status().set(DenoiseStatus::Idle),
            Err(message) => state.denoise.status().set(DenoiseStatus::Failed(message)),
        }
    });
}

/// `Err(String::new())` means the task was cancelled.
async fn run(state: AppState, generation: u32) -> Result<(), String> {
    let live = || GENERATION.with(|g| g.get()) == generation;
    let cancelled = String::new;

    let file = state
        .current_file()
        .ok_or_else(|| text::failed("no recording is open"))?;
    let sample_rate = file.audio.sample_rate;
    let total = file.audio.source.total_samples() as usize;
    if let Some(why) = refusal(sample_rate, total) {
        return Err(why);
    }

    let model = &catalog::DENOISER;
    if !store::is_downloaded(model).await {
        state
            .denoise
            .status()
            .set(DenoiseStatus::Downloading(0.0));
        store::download(model, |fraction| {
            if live() {
                state
                    .denoise
                    .status()
                    .set(DenoiseStatus::Downloading(fraction));
            }
        })
        .await
        .map_err(|e: ClassifierError| text::download_failed(&e.to_string()))?;
        state.denoise.model_stored().set(Some(true));
    }
    if !live() {
        return Err(cancelled());
    }

    state.denoise.status().set(DenoiseStatus::Starting);
    let bytes = store::read_js(model)
        .await
        .map_err(|e| text::failed(&e.to_string()))?;
    let samples = crate::audio::streaming_source::read_region_ready(
        &*file.audio.source,
        ChannelView::MonoMix,
        0,
        total,
    )
    .await
    .ok_or_else(|| text::failed("the recording could not be read"))?;
    if !live() {
        return Err(cancelled());
    }

    start_worker().map_err(|e| text::failed(&format!("the worker could not start ({e})")))?;
    let key = model.id.to_string();
    let run_id = generation;
    let mut sent_model = false;
    let mut sent_audio = false;
    let denoised = loop {
        let Some(item) = next().await else {
            return Err(cancelled());
        };
        let (msg, attached) = item.map_err(|e| text::failed(&e))?;
        match msg {
            FromWorker::Ready if !sent_model => {
                sent_model = true;
                let buffer = bytes.buffer();
                post(
                    &ToWorker::Load {
                        key: key.clone(),
                        part: ModelPart::Denoiser,
                    },
                    Some((attach::BYTES, &buffer)),
                );
            }
            FromWorker::Loaded { .. } if !sent_audio => {
                sent_audio = true;
                let array = js_sys::Float32Array::from(samples.as_slice());
                // Posted by its buffer so the samples move instead of copying.
                let obj_value: JsValue = array.buffer().into();
                post_samples(run_id, &array, &obj_value);
            }
            FromWorker::DenoiseProgress { run, done, total } if run == run_id => {
                state
                    .denoise
                    .status()
                    .set(DenoiseStatus::Running(done as f64 / total.max(1) as f64));
                post(&ToWorker::DenoiseStep { run: run_id }, None);
            }
            FromWorker::Denoised { run } if run == run_id => {
                break attached
                    .ok_or_else(|| text::failed("the worker sent no audio"))?
                    .to_vec();
            }
            FromWorker::Error { error, .. } => return Err(text::failed(&error.to_string())),
            _ => {}
        }
    };
    stop_worker();
    if !live() {
        return Err(cancelled());
    }

    let name = denoised_name(&file.name);
    let wav = crate::audio::wav_encoder::encode_wav_at(&denoised, 1, sample_rate, 32, true);
    let load_id = state.loading_start(&name);
    let loaded = crate::components::file_sidebar::load_named_bytes(
        name, &wav, None, None, None, state, load_id, false,
    )
    .await;
    state.loading_done(load_id);
    loaded.map_err(|e| text::failed(&format!("the copy could not be opened ({e})")))?;
    state.show_info_toast(text::ADDED);
    Ok(())
}

/// `DenoiseStart` with the recording's samples, transferred.
fn post_samples(run: u32, array: &js_sys::Float32Array, buffer: &JsValue) {
    LINK.with(|l| {
        let l = l.borrow();
        let Some(link) = l.as_ref() else {
            return;
        };
        let Ok(inner) = serde_wasm_bindgen::to_value(&ToWorker::DenoiseStart { run }) else {
            return;
        };
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&obj, &attach::MSG.into(), &inner);
        let _ = js_sys::Reflect::set(&obj, &attach::SAMPLES.into(), array);
        let _ = link
            .worker
            .post_message_with_transfer(&obj, &js_sys::Array::of1(buffer));
    });
}

/// "bat.wav" becomes "bat (denoised).wav".
pub fn denoised_name(name: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    format!("{stem} (denoised).wav")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_is_named_after_the_recording() {
        assert_eq!(denoised_name("XC1 - Bat.flac"), "XC1 - Bat (denoised).wav");
        assert_eq!(denoised_name("take"), "take (denoised).wav");
    }

    #[test]
    fn low_rates_and_long_recordings_are_refused() {
        assert!(refusal(384_000, 1_000_000).is_none());
        assert!(refusal(192_000, 1_000_000).unwrap().contains("250 kHz"));
        assert!(refusal(384_000, MAX_SAMPLES + 1).unwrap().contains("62 s"));
    }
}

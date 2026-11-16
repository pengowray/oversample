//! The learned denoiser, from the app's side.
//!
//! The network runs in a Web Worker of its own: the same WASM as the
//! classifier worker (`birdnet-worker`), started a second time, so a denoise
//! and a classifier run never wait for each other and the classifier's worker
//! restarts don't touch it. The worker is stopped after every recording, which
//! gives its memory back.
//!
//! A recording of up to [`IN_MEMORY_MAX_SAMPLES`] samples goes to the worker
//! whole and comes back whole. A longer one goes a piece at a time, twice
//! (once for its noise level, once to be denoised), and the copy is written
//! to a file as it comes back (`crate::denoise_copy`), so memory does not
//! grow with the recording's length. Both ways give the same samples.
//!
//! One recording at a time. A stereo recording gives a stereo copy: the
//! worker works out each channel's gains and gives both channels the larger
//! one, so the level difference between left and right stays as it was. A
//! recording with more than two channels is mixed to mono first. The result
//! is added to the file list as a new recording, so playback, the spectrogram, export and the call detector all
//! work on it without knowing about the denoiser.

use crate::audio::source::ChannelView;
use crate::classifier::{catalog, store, ClassifierError};
use crate::state::{AppState, DenoiseStateStoreFields, DenoiseStatus};
use futures_channel::oneshot;
use leptos::prelude::*;
use oversample_ipc::classifier::{attach, DenoiseGain, FromWorker, ModelPart, ToWorker};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, Worker};

/// The loader Trunk writes for the model worker.
const WORKER_URL: &str = "birdnet-worker_loader.js";

/// A denoising model the menu can use: its file, and what the menu says and
/// checks for it.
pub struct Model {
    pub file: &'static catalog::ModelFile,
    /// What the model was trained on, for the menu's "Model:" line.
    pub trained_on: &'static str,
    /// Lowest sample rate the model is run at, in Hz.
    pub min_rate: u32,
}

/// The model for bat and bird recordings (v2). It was trained on 203 bat
/// recordings at 250 to 500 kHz and 461 bird recordings at 22 to 96 kHz
/// (nearly all at 44.1 or 48 kHz), each used sample for sample at its own
/// rate (the model card in batchi-noisier2noise's `hf/oversample-denoise`).
/// It works on spectrogram bins and frames, not on hertz. The app does not
/// run it below 22.05 kHz.
pub const DENOISE_MODEL: Model = Model {
    file: &catalog::DENOISER,
    trained_on: "bat and bird recordings",
    min_rate: 22_050,
};

/// The model "Make denoised copy" uses, for every recording.
pub fn model() -> &'static Model {
    &DENOISE_MODEL
}

/// What the worker does to the network's gain: the smoothed chain, which
/// leaves fewer patches of noise than the plain gain.
const GAIN: DenoiseGain = DenoiseGain::Smoothed;

/// Frames in the smoothed gain's median (5 bins by this many frames of 128
/// samples) for a recording read at `sample_rate` Hz, after any
/// time-expansion correction. Over 96 kHz it is 3 frames (1 ms at 384 kHz):
/// 9 frames are longer than a short bat call and take about 5 dB off pulses
/// under 2 ms. At 96 kHz and below it is 9.
pub fn smooth_frames(sample_rate: u32) -> u32 {
    if sample_rate > 96_000 {
        3
    } else {
        oversample_ipc::classifier::DEFAULT_SMOOTH_FRAMES
    }
}

/// Longest recording that is denoised in memory, in samples over all the
/// channels of the copy: 62 s at 384 kHz and 9 min 4 s at 44.1 kHz in mono,
/// half that in stereo. In memory the app and the worker hold the recording
/// several times over (about 25 bytes for each sample). A longer recording is
/// denoised a piece at a time and written to a file. Where the device cannot
/// write such a file, this is the longest recording it can denoise.
pub const IN_MEMORY_MAX_SAMPLES: usize = 24_000_000;

/// Samples of one channel in each piece of a long recording: one block of
/// the worker's denoiser, so the progress moves with every piece.
const PIECE_FRAMES: usize = 1024 * 128;

/// [`IN_MEMORY_MAX_SAMPLES`], or the number in
/// `window.__oversample_denoise_in_memory_max` when a test has set one, so a
/// short recording can go through the long recording's code.
fn in_memory_max() -> usize {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"__oversample_denoise_in_memory_max".into()).ok())
        .and_then(|v| v.as_f64())
        .map_or(IN_MEMORY_MAX_SAMPLES, |n| n as usize)
}

/// The most samples a recording can have here, over all the channels of the
/// copy. `None` on a device that can write a long copy to a file, where the
/// length has no limit of its own.
pub fn sample_limit(state: AppState) -> Option<usize> {
    (!crate::denoise_copy::supported(state)).then(in_memory_max)
}

/// Channels in the copy of a recording with `channels` channels: stereo
/// stays stereo, and anything else is one channel (a mono mix).
pub fn copy_channels(channels: u32) -> u32 {
    if channels == 2 {
        2
    } else {
        1
    }
}

/// The menu's strings, in one place. Drafted from a brief by a writer that
/// had not seen the code (2026-11-14, and again 2026-11-15 when the section
/// began to take recordings at bird sample rates).
pub mod text {
    use crate::format_units::{khz, sample_rate_khz};

    pub const TITLE: &str = "Denoised copy";
    pub const BUTTON: &str = "Make denoised copy";
    pub const CANCEL: &str = "Cancel";
    pub const STARTING: &str = "Loading recording and model\u{2026}";
    pub const ADDED: &str = "Denoised copy added to the file list";
    pub const TOOLTIP: &str = "Lowers steady background noise such as hiss, keeping at least 10% of the original sound at every frequency.";

    /// The status area's first line: "Model: trained on bat and bird
    /// recordings".
    pub fn model_line(trained_on: &str) -> String {
        format!("Model: trained on {trained_on}")
    }
    /// What the button does. A recording with more than two channels gives a
    /// mono copy.
    pub fn result_line(mono_mix: bool) -> &'static str {
        if mono_mix {
            "Adds a mono copy and keeps the original."
        } else {
            "Adds a copy and keeps the original."
        }
    }
    /// Shown until the model file is on this device.
    pub fn download_line(size: &str) -> String {
        format!("Downloads the model first: {size}, once.")
    }
    pub fn downloading(percent: u32) -> String {
        format!("Downloading model: {percent}%")
    }
    /// The line under the progress bar. `seconds_left` is an estimate, and
    /// `None` until enough of the recording is done to make one.
    pub fn running(percent: u32, seconds_left: Option<u64>) -> String {
        match seconds_left {
            Some(seconds) => format!("Denoising: {percent}%, {}", time_left(seconds)),
            None => format!("Denoising: {percent}%"),
        }
    }
    /// "about 3 min left", rounded up to the minute; "about 1 h 5 min left".
    fn time_left(seconds: u64) -> String {
        let minutes = seconds.div_ceil(60).max(1);
        if minutes < 60 {
            format!("about {minutes} min left")
        } else {
            format!("about {} h {} min left", minutes / 60, minutes % 60)
        }
    }
    pub fn download_failed(error: &str) -> String {
        format!("Model download failed: {error}")
    }
    pub fn failed(error: &str) -> String {
        format!("Denoising failed: {error}")
    }
    /// Two lines. `min_rate` keeps two decimals, so 22,050 Hz reads
    /// "22.05 kHz" and not the "22.1 kHz" of the app's sample rate text.
    pub fn rate_too_low(rate: u32, min_rate: u32) -> String {
        format!(
            "Sample rate too low\nNeeds {} or more. This recording: {}.",
            khz(i64::from(min_rate), 2),
            sample_rate_khz(rate)
        )
    }
    /// For a browser that cannot write the copy to a file as it is made, so
    /// the copy has to fit in memory. A headline, then the numbers. `frames` and `max_frames`
    /// are samples in one channel. The recording's length is rounded up and
    /// the maximum down, so the two never read the same. A stereo
    /// recording's maximum is half a mono one's, so its line says "stereo".
    pub fn too_long(frames: u64, max_frames: u64, rate: u32, stereo: bool) -> String {
        let rate_f = f64::from(rate.max(1));
        format!(
            "Recording too long for this browser\nMaximum at {}{}: {}. This recording: {}.",
            sample_rate_khz(rate),
            if stereo { " stereo" } else { "" },
            minutes_seconds((max_frames as f64 / rate_f).floor() as u64),
            minutes_seconds((frames as f64 / rate_f).ceil() as u64)
        )
    }
    /// For a browser whose storage has too little room for the copy's file.
    /// A headline, then the numbers. A long copy that is open in the file
    /// list has a file in that storage, which closing it deletes.
    pub fn no_space(needed_bytes: u64, free_bytes: u64) -> String {
        format!(
            "Not enough browser storage\nCopy needs {}. Free: {}. Closing an earlier denoised copy can free storage.",
            megabytes(needed_bytes, true),
            megabytes(free_bytes, false)
        )
    }
    /// "921 MB" or "1.2 GB". What is needed rounds up and what is free rounds
    /// down, so the two never read the same.
    fn megabytes(bytes: u64, round_up: bool) -> String {
        let mb = if round_up {
            bytes.div_ceil(1_000_000)
        } else {
            bytes / 1_000_000
        };
        if mb < 1000 {
            format!("{mb} MB")
        } else if round_up {
            format!("{:.1} GB", (mb as f64 / 100.0).ceil() / 10.0)
        } else {
            format!("{:.1} GB", (mb as f64 / 100.0).floor() / 10.0)
        }
    }
    /// "62 s" or "9 min 4 s": seconds alone under two minutes.
    fn minutes_seconds(seconds: u64) -> String {
        if seconds < 120 {
            format!("{seconds} s")
        } else {
            format!("{} min {} s", seconds / 60, seconds % 60)
        }
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
/// The first time in a session it also deletes the old model files that
/// earlier versions downloaded ([`store::delete_retired`]).
pub fn check_model_stored(state: AppState) {
    thread_local! {
        static RETIRED_DELETED: Cell<bool> = const { Cell::new(false) };
    }
    let delete_retired = !RETIRED_DELETED.with(|d| d.replace(true));
    wasm_bindgen_futures::spawn_local(async move {
        if delete_retired {
            store::delete_retired().await;
        }
        let stored = store::is_downloaded(model().file).await;
        state.denoise.model_stored().set(Some(stored));
    });
}

/// The model file's size, as the menu shows it: "0.3 MB".
pub fn model_size_text() -> String {
    format!("{:.1} MB", model().file.size as f64 / 1e6)
}

/// True while a recording is being denoised (or its model downloaded).
pub fn is_busy(state: AppState) -> bool {
    !matches!(
        state.denoise.status().get(),
        DenoiseStatus::Idle | DenoiseStatus::Failed(_)
    )
}

/// Why the open recording can't be denoised, or `None` when it can. `frames`
/// is the recording's length in samples of one channel, and `limit` is
/// [`sample_limit`]'s answer for this device.
pub fn refusal(
    sample_rate: u32,
    frames: u64,
    channels: u32,
    limit: Option<usize>,
) -> Option<String> {
    let min_rate = model().min_rate;
    if sample_rate < min_rate {
        return Some(text::rate_too_low(sample_rate, min_rate));
    }
    let copy_channels = u64::from(copy_channels(channels));
    let max_frames = limit? as u64 / copy_channels;
    (frames > max_frames)
        .then(|| text::too_long(frames, max_frames, sample_rate, copy_channels == 2))
}

/// Seconds the rest of a run should take, when `fraction` of it took
/// `elapsed_s` seconds. `None` in the first seconds, when the rate is not
/// yet steady enough to say.
fn seconds_left(fraction: f64, elapsed_s: f64) -> Option<u64> {
    (elapsed_s >= 5.0 && fraction > 0.0 && fraction <= 1.0)
        .then(|| (elapsed_s * (1.0 - fraction) / fraction).ceil() as u64)
}

/// The status for `done` of `total` blocks, with the time left as worked out
/// from `started_ms`, the time (`Date.now()`) the first block began.
fn running_status(done: u32, total: u32, started_ms: f64) -> DenoiseStatus {
    let fraction = f64::from(done) / f64::from(total.max(1));
    DenoiseStatus::Running {
        fraction,
        seconds_left: seconds_left(fraction, (js_sys::Date::now() - started_ms) / 1000.0),
    }
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
        if GENERATION.with(|g| g.get()) != generation {
            // Cancelled, or another recording started: its status stands,
            // and the worker that is running now is the other recording's.
            return;
        }
        stop_worker();
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
    let frames = file.audio.source.total_samples();
    if let Some(why) = refusal(
        sample_rate,
        frames,
        file.audio.channels,
        sample_limit(state),
    ) {
        return Err(why);
    }
    let channels = copy_channels(file.audio.channels);
    let in_pieces = frames.saturating_mul(u64::from(channels)) > in_memory_max() as u64;
    // In memory, the recording is short enough to count in a `usize`.
    let total = frames as usize;

    let model = model().file;
    if !store::is_downloaded(model).await {
        state.denoise.status().set(DenoiseStatus::Downloading(0.0));
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
    // The channels the copy will have, read from the source the app already
    // holds, and interleaved for the worker.
    let views: Vec<ChannelView> = if channels == 1 {
        vec![ChannelView::MonoMix]
    } else {
        (0..channels).map(ChannelView::Channel).collect()
    };
    if in_pieces {
        return run_in_pieces(state, generation, &file, bytes, &views).await;
    }
    let read =
        crate::audio::streaming_source::read_views_ready(&*file.audio.source, &views, 0, total)
            .await
            .views
            // A read that runs past the end of the file gives shorter channels.
            .filter(|v| v.len() == views.len() && v.iter().all(|c| c.len() == v[0].len()))
            .ok_or_else(|| text::failed("the recording could not be read"))?;
    let mut samples = Some(interleave(read));
    if !live() {
        return Err(cancelled());
    }

    start_worker().map_err(|e| text::failed(&format!("the worker could not start ({e})")))?;
    let key = model.id.to_string();
    let run_id = generation;
    let mut sent_model = false;
    let mut sent_audio = false;
    let mut started_ms = None;
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
                // The worker has the samples from here on, so this copy goes.
                let samples = samples.take().unwrap_or_default();
                let array = js_sys::Float32Array::from(samples.as_slice());
                drop(samples);
                // Posted by its buffer so the samples move instead of copying.
                let obj_value: JsValue = array.buffer().into();
                let smooth_frames = smooth_frames(sample_rate);
                let start = if channels == 1 {
                    ToWorker::DenoiseStart {
                        run: run_id,
                        gain: GAIN,
                        smooth_frames,
                    }
                } else {
                    ToWorker::DenoiseStartChannels {
                        run: run_id,
                        channels,
                        gain: GAIN,
                        smooth_frames,
                    }
                };
                post_samples(&start, &array, &obj_value);
            }
            FromWorker::DenoiseProgress { run, done, total } if run == run_id => {
                let started_ms = *started_ms.get_or_insert_with(js_sys::Date::now);
                state
                    .denoise
                    .status()
                    .set(running_status(done, total, started_ms));
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
    let wav =
        crate::audio::wav_encoder::encode_wav_at(&denoised, channels as u16, sample_rate, 32, true);
    drop(denoised);
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

/// [`run`] for a recording too long to hold several times over: the worker
/// gets it a piece at a time, and the copy is written to a file as the
/// worker gives it back. `bytes` is the model and `views` are the channels
/// the copy has.
async fn run_in_pieces(
    state: AppState,
    generation: u32,
    file: &crate::state::LoadedFile,
    bytes: js_sys::Uint8Array,
    views: &[ChannelView],
) -> Result<(), String> {
    use crate::denoise_copy::{CopyFile, CreateError};
    let live = || GENERATION.with(|g| g.get()) == generation;
    let cancelled = String::new;

    let name = denoised_name(&file.name);
    let total = file.audio.source.total_samples();
    let mut copy = CopyFile::create(
        state,
        &name,
        total,
        views.len() as u32,
        file.audio.sample_rate,
        generation,
    )
    .await
    .map_err(|e| match e {
        CreateError::NoSpace { needed, free } => text::no_space(needed, free),
        CreateError::Other(e) => text::failed(&format!("the copy could not be started ({e})")),
    })?;
    // A cancel, or a failure, deletes what was written.
    let denoised = if live() {
        denoise_into(state, generation, file, bytes, views, &mut copy).await
    } else {
        Err(cancelled())
    };
    let denoised = denoised.and_then(|()| if live() { Ok(()) } else { Err(cancelled()) });
    if let Err(e) = denoised {
        copy.discard().await;
        return Err(e);
    }
    stop_worker();
    let written = copy
        .finish()
        .await
        .map_err(|e| text::failed(&format!("the copy could not be written ({e})")))?;
    crate::denoise_copy::open(state, written, &name)
        .await
        .map_err(|e| text::failed(&format!("the copy could not be opened ({e})")))?;
    state.show_info_toast(text::ADDED);
    Ok(())
}

/// Sends the recording to a new worker a piece at a time, first for its
/// noise level and then to be denoised, and writes the denoised samples to
/// `copy` as they come back. One piece is with the worker at a time.
async fn denoise_into(
    state: AppState,
    generation: u32,
    file: &crate::state::LoadedFile,
    bytes: js_sys::Uint8Array,
    views: &[ChannelView],
    copy: &mut crate::denoise_copy::CopyFile,
) -> Result<(), String> {
    let live = || GENERATION.with(|g| g.get()) == generation;
    let cancelled = String::new;
    let source = &*file.audio.source;
    let total = source.total_samples();
    let channels = views.len() as u32;
    // Samples the copy has, over all its channels.
    let all = total * u64::from(channels);

    start_worker().map_err(|e| text::failed(&format!("the worker could not start ({e})")))?;
    let key = model().file.id.to_string();
    let run_id = generation;
    // Reads the piece that starts at sample `at` and posts it with `msg`.
    // Returns the samples of one channel that it sent.
    let send = |msg: ToWorker, at: u64| async move {
        let len = (total - at).min(PIECE_FRAMES as u64) as usize;
        let read = crate::audio::streaming_source::read_views_ready(source, views, at, len)
            .await
            .views
            // A recording that turns out shorter than its file said gives a
            // short piece.
            .filter(|v| v.len() == views.len() && v.iter().all(|c| c.len() == len))
            .ok_or_else(|| text::failed("the recording could not be read"))?;
        if !live() {
            return Err(cancelled());
        }
        let array = js_sys::Float32Array::from(interleave(read).as_slice());
        post_samples(&msg, &array, &array.buffer().into());
        Ok(len as u64)
    };

    let mut sent_model = false;
    let mut sent_start = false;
    // Samples of one channel sent to be measured, and sent to be denoised.
    let mut measured = 0u64;
    let mut sent = 0u64;
    // Samples written to the copy, over all its channels.
    let mut written = 0u64;
    let mut started_ms = 0.0;
    loop {
        let Some(item) = next().await else {
            return Err(cancelled());
        };
        let (msg, attached) = item.map_err(|e| text::failed(&e))?;
        match msg {
            FromWorker::Ready if !sent_model => {
                sent_model = true;
                post(
                    &ToWorker::Load {
                        key: key.clone(),
                        part: ModelPart::Denoiser,
                    },
                    Some((attach::BYTES, &bytes.buffer())),
                );
            }
            FromWorker::Loaded { .. } if !sent_start => {
                sent_start = true;
                post(
                    &ToWorker::DenoiseStreamStart {
                        run: run_id,
                        channels,
                        frames: total as f64,
                        gain: GAIN,
                        smooth_frames: smooth_frames(file.audio.sample_rate),
                    },
                    None,
                );
            }
            FromWorker::DenoiseProgress { run, done: 0, .. } if run == run_id && measured == 0 => {
                measured += send(ToWorker::DenoiseMeasure { run: run_id }, 0).await?;
            }
            FromWorker::DenoiseMeasured { run } if run == run_id && sent == 0 => {
                if measured < total {
                    measured += send(ToWorker::DenoiseMeasure { run: run_id }, measured).await?;
                } else {
                    started_ms = js_sys::Date::now();
                    sent += send(ToWorker::DenoisePiece { run: run_id }, 0).await?;
                }
            }
            FromWorker::DenoisedPiece {
                run,
                done,
                total: blocks,
            } if run == run_id => {
                let samples = attached.ok_or_else(|| text::failed("the worker sent no audio"))?;
                if samples.length() > 0 {
                    copy.write(&samples).await.map_err(|e| {
                        text::failed(&format!("the copy could not be written ({e})"))
                    })?;
                    if !live() {
                        return Err(cancelled());
                    }
                    written += u64::from(samples.length());
                }
                state
                    .denoise
                    .status()
                    .set(running_status(done, blocks, started_ms));
                if written == all {
                    return Ok(());
                }
                if sent >= total {
                    return Err(text::failed("the worker sent too little audio"));
                }
                sent += send(ToWorker::DenoisePiece { run: run_id }, sent).await?;
            }
            FromWorker::Error { error, .. } => return Err(text::failed(&error.to_string())),
            _ => {}
        }
    }
}

/// The channels' samples as one list: the first sample of every channel, then
/// the second of every channel, and so on. One channel is returned as it is.
fn interleave(mut channels: Vec<Vec<f32>>) -> Vec<f32> {
    if channels.len() == 1 {
        return channels.pop().unwrap_or_default();
    }
    let frames = channels.first().map_or(0, Vec::len);
    let mut out = Vec::with_capacity(frames * channels.len());
    for i in 0..frames {
        out.extend(channels.iter().map(|c| c[i]));
    }
    out
}

/// `start` (a message that takes samples) with the samples in `array`,
/// transferred.
fn post_samples(start: &ToWorker, array: &js_sys::Float32Array, buffer: &JsValue) {
    LINK.with(|l| {
        let l = l.borrow();
        let Some(link) = l.as_ref() else {
            return;
        };
        let Ok(inner) = serde_wasm_bindgen::to_value(start) else {
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
    // The suffix pairs the copy with its original in the file list.
    let suffix = crate::components::file_sidebar::file_groups::DENOISED_SUFFIX;
    format!("{stem}{suffix}.wav")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordings_over_96_khz_get_a_median_of_3_frames() {
        assert_eq!(smooth_frames(22_050), 9);
        assert_eq!(smooth_frames(48_000), 9);
        assert_eq!(smooth_frames(96_000), 9);
        assert_eq!(smooth_frames(96_001), 3);
        assert_eq!(smooth_frames(384_000), 3);
        // A ×10 time-expanded file read at its header's 38.4 kHz, and the
        // same file after "Correct for ×10 time expansion" (384 kHz).
        assert_eq!(smooth_frames(38_400), 9);
        assert_eq!(smooth_frames(384_000), 3);
    }

    #[test]
    fn every_recording_uses_the_bat_and_bird_model() {
        assert_eq!(model().file.id, "oversample-denoise-v2.onnx");
        assert_eq!(
            text::model_line(model().trained_on),
            "Model: trained on bat and bird recordings"
        );
        assert_eq!(model().min_rate, 22_050);
    }

    #[test]
    fn the_result_is_named_after_the_recording() {
        assert_eq!(denoised_name("XC1 - Bat.flac"), "XC1 - Bat (denoised).wav");
        assert_eq!(denoised_name("take"), "take (denoised).wav");
    }

    #[test]
    fn low_rates_and_long_recordings_are_refused() {
        // A device that can write a long copy to a file has no limit, and
        // one that cannot has the most that fits in memory.
        let (none, max) = (None, Some(IN_MEMORY_MAX_SAMPLES));
        let refusal = |rate, frames, channels| refusal(rate, frames, channels, max);
        assert!(refusal(384_000, 1_000_000, 1).is_none());
        assert!(refusal(44_100, 1_000_000, 1).is_none());
        assert!(refusal(22_050, 1_000_000, 2).is_none());
        assert!(super::refusal(384_000, 1_000_000_000, 2, none).is_none());
        assert!(super::refusal(16_000, 1_000_000, 1, none).is_some());
        assert_eq!(
            refusal(16_000, 1_000_000, 1).unwrap(),
            "Sample rate too low\nNeeds 22.05\u{a0}kHz or more. This recording: 16\u{a0}kHz."
        );
        // 22,000 Hz is under the limit, and the two rates must not read alike.
        assert_eq!(
            refusal(22_000, 1_000_000, 1).unwrap(),
            "Sample rate too low\nNeeds 22.05\u{a0}kHz or more. This recording: 22\u{a0}kHz."
        );
        assert_eq!(
            refusal(384_000, IN_MEMORY_MAX_SAMPLES as u64 + 1, 1).unwrap(),
            "Recording too long for this browser\nMaximum at 384\u{a0}kHz: 62 s. This recording: 63 s."
        );
        assert_eq!(
            refusal(44_100, 44_100 * 750, 1).unwrap(),
            "Recording too long for this browser\nMaximum at 44.1\u{a0}kHz: 9 min 4 s. This recording: 12 min 30 s."
        );
        // The limit counts the samples of both channels of a stereo copy.
        assert!(refusal(44_100, IN_MEMORY_MAX_SAMPLES as u64 / 2, 2).is_none());
        assert_eq!(
            refusal(44_100, 44_100 * 300, 2).unwrap(),
            "Recording too long for this browser\nMaximum at 44.1\u{a0}kHz stereo: 4 min 32 s. This recording: 5 min 0 s."
        );
        // More than two channels are mixed to one, so the mono limit applies.
        assert!(refusal(44_100, 44_100 * 300, 4).is_none());
    }

    #[test]
    fn the_progress_line_gets_the_time_left_once_there_is_an_estimate() {
        assert_eq!(seconds_left(0.5, 4.0), None);
        assert_eq!(seconds_left(0.0, 30.0), None);
        assert_eq!(seconds_left(0.25, 30.0), Some(90));
        assert_eq!(seconds_left(1.0, 30.0), Some(0));
        assert_eq!(text::running(3, None), "Denoising: 3%");
        assert_eq!(
            text::running(25, Some(90)),
            "Denoising: 25%, about 2 min left"
        );
        assert_eq!(
            text::running(99, Some(4)),
            "Denoising: 99%, about 1 min left"
        );
        assert_eq!(
            text::running(10, Some(3900)),
            "Denoising: 10%, about 1 h 5 min left"
        );
    }

    #[test]
    fn the_storage_refusal_gives_what_is_needed_and_what_is_free() {
        assert_eq!(
            text::no_space(921_600_080, 310_999_999),
            "Not enough browser storage\nCopy needs 922 MB. Free: 310 MB. Closing an earlier denoised copy can free storage."
        );
        assert_eq!(
            text::no_space(2_540_160_080, 1_999_000_000),
            "Not enough browser storage\nCopy needs 2.6 GB. Free: 1.9 GB. Closing an earlier denoised copy can free storage."
        );
    }

    #[test]
    fn channels_are_interleaved_for_the_worker() {
        assert_eq!(interleave(vec![vec![1.0, 2.0]]), [1.0, 2.0]);
        assert_eq!(
            interleave(vec![vec![1.0, 2.0], vec![-1.0, -2.0]]),
            [1.0, -1.0, 2.0, -2.0]
        );
    }
}

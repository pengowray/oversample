//! The learned denoiser, from the app's side.
//!
//! The network runs in a Web Worker of its own: the same WASM as the
//! classifier worker (`birdnet-worker`), started a second time, so a denoise
//! and a classifier run never wait for each other and the classifier's worker
//! restarts don't touch it. The worker is stopped after every recording, which
//! gives its memory back.
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
use oversample_ipc::classifier::{attach, FromWorker, ModelPart, ToWorker};
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

/// The bat model. It was trained on bat recordings from 250 to 500 kHz, used
/// sample for sample, and checked by eye at 192, 256, 384 and 500 kHz
/// (2026-11-14). It works on spectrogram bins and frames, not on hertz, and in
/// a trial on bird recordings at 22.05, 32, 44.1 and 48 kHz it took about
/// 12 dB off the hiss and kept loud song (2026-11-15, the "Bird pilot" section
/// of batchi-noisier2noise's README). Not tried below 22.05 kHz.
pub const BAT_MODEL: Model = Model {
    file: &catalog::DENOISER,
    trained_on: "bat recordings",
    min_rate: 22_050,
};

/// The model "Make Denoised Copy" uses. There is one model today; a choice
/// between models would be made here.
pub fn model() -> &'static Model {
    &BAT_MODEL
}

/// Longest recording, in samples over all the channels that are denoised:
/// 62 s at 384 kHz and 9 min 4 s at 44.1 kHz in mono, half that in stereo.
/// It is a memory limit, and memory goes by the number of samples, whichever
/// channels they are in. The worker holds the recording four times over
/// while it works.
pub const MAX_SAMPLES: usize = 24_000_000;

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

    pub const TITLE: &str = "Denoised Copy";
    pub const BUTTON: &str = "Make Denoised Copy";
    pub const CANCEL: &str = "Cancel";
    pub const STARTING: &str = "Loading recording and model\u{2026}";
    pub const ADDED: &str = "Denoised copy added to the file list";
    pub const TOOLTIP: &str = "Lowers steady background noise such as hiss, keeping at least 10% of the original sound at every frequency.";

    /// The status area's first line: "Model: trained on bat recordings".
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
    pub fn running(percent: u32) -> String {
        format!("Denoising: {percent}%")
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
    /// Two lines. `frames` and `max_frames` are samples in one channel. The
    /// recording's length is rounded up and the maximum down, so the two
    /// never read the same. A stereo recording's maximum is half a mono
    /// one's, so its line says "stereo".
    pub fn too_long(frames: usize, max_frames: usize, rate: u32, stereo: bool) -> String {
        let rate_f = f64::from(rate.max(1));
        format!(
            "Recording too long: {}\nMaximum at {}{}: {}",
            minutes_seconds((frames as f64 / rate_f).ceil() as u64),
            sample_rate_khz(rate),
            if stereo { " stereo" } else { "" },
            minutes_seconds((max_frames as f64 / rate_f).floor() as u64)
        )
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
pub fn check_model_stored(state: AppState) {
    wasm_bindgen_futures::spawn_local(async move {
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
/// is the recording's length in samples of one channel.
pub fn refusal(sample_rate: u32, frames: usize, channels: u32) -> Option<String> {
    let min_rate = model().min_rate;
    if sample_rate < min_rate {
        return Some(text::rate_too_low(sample_rate, min_rate));
    }
    let copy_channels = copy_channels(channels) as usize;
    let max_frames = MAX_SAMPLES / copy_channels;
    if frames > max_frames {
        return Some(text::too_long(
            frames,
            max_frames,
            sample_rate,
            copy_channels == 2,
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
    if let Some(why) = refusal(sample_rate, total, file.audio.channels) {
        return Err(why);
    }
    let channels = copy_channels(file.audio.channels);

    let model = model().file;
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
    // The channels the copy will have, read from the source the app already
    // holds, and interleaved for the worker.
    let views: Vec<ChannelView> = if channels == 1 {
        vec![ChannelView::MonoMix]
    } else {
        (0..channels).map(ChannelView::Channel).collect()
    };
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
                let start = if channels == 1 {
                    ToWorker::DenoiseStart { run: run_id }
                } else {
                    ToWorker::DenoiseStartChannels {
                        run: run_id,
                        channels,
                    }
                };
                post_samples(&start, &array, &obj_value);
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

/// `start` (a `DenoiseStart` or `DenoiseStartChannels`) with the recording's
/// samples, transferred.
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
    fn the_result_is_named_after_the_recording() {
        assert_eq!(denoised_name("XC1 - Bat.flac"), "XC1 - Bat (denoised).wav");
        assert_eq!(denoised_name("take"), "take (denoised).wav");
    }

    #[test]
    fn low_rates_and_long_recordings_are_refused() {
        assert!(refusal(384_000, 1_000_000, 1).is_none());
        assert!(refusal(44_100, 1_000_000, 1).is_none());
        assert!(refusal(22_050, 1_000_000, 2).is_none());
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
            refusal(384_000, MAX_SAMPLES + 1, 1).unwrap(),
            "Recording too long: 63 s\nMaximum at 384\u{a0}kHz: 62 s"
        );
        assert_eq!(
            refusal(44_100, 44_100 * 750, 1).unwrap(),
            "Recording too long: 12 min 30 s\nMaximum at 44.1\u{a0}kHz: 9 min 4 s"
        );
        // The limit counts the samples of both channels of a stereo copy.
        assert!(refusal(44_100, MAX_SAMPLES / 2, 2).is_none());
        assert_eq!(
            refusal(44_100, 44_100 * 300, 2).unwrap(),
            "Recording too long: 5 min 0 s\nMaximum at 44.1\u{a0}kHz stereo: 4 min 32 s"
        );
        // More than two channels are mixed to one, so the mono limit applies.
        assert!(refusal(44_100, 44_100 * 300, 4).is_none());
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

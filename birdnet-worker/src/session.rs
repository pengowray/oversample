//! The worker's state and message handling, without any browser types, so the
//! native tests drive the same code the Web Worker runs.

use crate::denoise::stream::StreamDenoiser;
use crate::denoise::{check_smooth_frames, Denoiser, TractMask};
use crate::engine::{sigmoid, top_confidences, top_scores, Backbone, Head, MergedHeads};
use crate::perch;
use crate::resample::StreamResampler;
use crate::v3;
use crate::windows::{Window, Windower};
use oversample_ipc::classifier::{
    ClassifierError, FromWorker, Method, ModelPart, ModelSlot, ToWorker,
};

/// Arrays that arrive next to a [`ToWorker`] message.
#[derive(Default)]
pub struct Attached {
    pub bytes: Option<Vec<u8>>,
    pub samples: Option<Vec<f32>>,
    pub embedding: Option<Vec<f32>>,
}

/// Where the worker's answers go: the message, and for `Window` its embedding
/// or for `Denoised` its samples.
pub trait Outbox {
    fn send(&mut self, msg: FromWorker, embedding: Option<&[f32]>);
}

impl<F: FnMut(FromWorker, Option<&[f32]>)> Outbox for F {
    fn send(&mut self, msg: FromWorker, embedding: Option<&[f32]>) {
        self(msg, embedding)
    }
}

struct Run {
    id: u32,
    method: Method,
    windower: Windower,
}

/// The model that takes the audio.
enum AudioModel {
    BirdNet(Backbone),
    Perch(perch::Embedding),
    BirdNetV3(v3::Model),
}

/// The model that scores the audio model's output.
enum HeadModel {
    Bat(Head),
    /// Every regional head, scored as one.
    BatAll(MergedHeads),
    Perch(perch::Classifier),
}

impl HeadModel {
    /// The bat logits for one BirdNET embedding; `None` for Perch's classifier.
    fn bat_logits(&self, embedding: &[f32]) -> Option<tract_onnx::prelude::TractResult<Vec<f32>>> {
        match self {
            HeadModel::Bat(h) => Some(h.run(embedding)),
            HeadModel::BatAll(h) => Some(h.run(embedding)),
            HeadModel::Perch(_) => None,
        }
    }

    fn is_bat(&self) -> bool {
        matches!(self, HeadModel::Bat(_) | HeadModel::BatAll(_))
    }
}

#[derive(Default)]
pub struct Session {
    audio: Option<(String, AudioModel)>,
    head: Option<(String, HeadModel)>,
    run: Option<Run>,
    /// The denoiser's model, and the recording it is working through.
    denoiser: Option<(String, TractMask)>,
    denoising: Option<(u32, Denoiser)>,
    /// The recording it is working through that arrives in pieces.
    streaming: Option<(u32, StreamDenoiser)>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle one message. `now_ms` times model loading.
    pub fn handle(
        &mut self,
        msg: ToWorker,
        att: Attached,
        out: &mut dyn Outbox,
        now_ms: &dyn Fn() -> f64,
    ) {
        match msg {
            ToWorker::Load { key, part } => {
                // Free the old model first: two at once may not fit on a phone.
                match part.slot() {
                    ModelSlot::Audio => self.audio = None,
                    ModelSlot::Head => self.head = None,
                    ModelSlot::Denoise => {
                        self.denoiser = None;
                        self.denoising = None;
                        self.streaming = None;
                    }
                }
                let t = now_ms();
                let bytes = att.bytes.unwrap_or_default();
                let built = match part {
                    ModelPart::Backbone => Backbone::load(&bytes)
                        .map(|m| self.audio = Some((key.clone(), AudioModel::BirdNet(m))))
                        .map_err(|e| format!("{e:#}")),
                    ModelPart::PerchEmbedding => perch::Embedding::load(&bytes)
                        .map(|m| self.audio = Some((key.clone(), AudioModel::Perch(m))))
                        .map_err(|e| format!("{e:#}")),
                    ModelPart::BirdNetV3 => v3::Model::load(&bytes)
                        .map(|m| self.audio = Some((key.clone(), AudioModel::BirdNetV3(m))))
                        .map_err(|e| format!("{e:#}")),
                    ModelPart::BatHead => Head::load(&bytes)
                        .map(|m| self.head = Some((key.clone(), HeadModel::Bat(m))))
                        .map_err(|e| format!("{e:#}")),
                    ModelPart::PerchClassifier => perch::Classifier::from_bytes(bytes)
                        .map(|m| self.head = Some((key.clone(), HeadModel::Perch(m)))),
                    ModelPart::Denoiser => TractMask::load(bytes)
                        .map(|m| self.denoiser = Some((key.clone(), m)))
                        .map_err(|e| format!("{e:#}")),
                };
                match built {
                    Ok(()) => out.send(
                        FromWorker::Loaded {
                            key,
                            ms: now_ms() - t,
                        },
                        None,
                    ),
                    Err(message) => out.send(load_error(message), None),
                }
            }
            ToWorker::LoadBatHeads {
                key,
                heads,
                species,
                classes,
            } => {
                self.head = None;
                let t = now_ms();
                let bytes = att.bytes.unwrap_or_default();
                match MergedHeads::load(&bytes, &heads, species as usize, classes as usize) {
                    Ok(m) => {
                        self.head = Some((key.clone(), HeadModel::BatAll(m)));
                        out.send(
                            FromWorker::Loaded {
                                key,
                                ms: now_ms() - t,
                            },
                            None,
                        );
                    }
                    Err(e) => out.send(load_error(format!("{e:#}")), None),
                }
            }
            ToWorker::Start {
                run,
                method,
                source_rate,
            } => {
                self.run = None;
                if let Some(min) = method.min_source_rate() {
                    if source_rate < min {
                        out.send(
                            FromWorker::Error {
                                run: Some(run),
                                error: ClassifierError::SampleRateTooLow {
                                    rate: source_rate,
                                    min,
                                },
                            },
                            None,
                        );
                        return;
                    }
                }
                if source_rate == 0 {
                    out.send(
                        inference_error(run, "the file has no sample rate".into()),
                        None,
                    );
                    return;
                }
                self.run = Some(Run {
                    id: run,
                    method,
                    windower: Windower::new(method, source_rate),
                });
            }
            ToWorker::Chunk { run, seq } => {
                let Some(r) = self.run.as_mut().filter(|r| r.id == run) else {
                    return;
                };
                let windows = r.windower.push(att.samples.as_deref().unwrap_or_default());
                if self.score_windows(run, windows, out) {
                    out.send(FromWorker::ChunkDone { run, seq }, None);
                }
            }
            ToWorker::Finish { run } => {
                let Some(r) = self.run.as_mut().filter(|r| r.id == run) else {
                    return;
                };
                let windows = r.windower.finish();
                if self.score_windows(run, windows, out) {
                    let count = self.run.as_ref().map_or(0, |r| r.windower.windows());
                    self.run = None;
                    out.send(
                        FromWorker::Done {
                            run,
                            windows: count,
                        },
                        None,
                    );
                }
            }
            ToWorker::Cancel { run } => {
                if self.run.as_ref().is_some_and(|r| r.id == run) {
                    self.run = None;
                }
                if self.denoising.as_ref().is_some_and(|(id, _)| *id == run) {
                    self.denoising = None;
                }
                if self.streaming.as_ref().is_some_and(|(id, _)| *id == run) {
                    self.streaming = None;
                }
            }
            ToWorker::DenoiseStart {
                run,
                gain,
                smooth_frames,
            }
            | ToWorker::DenoiseStartChannels {
                run,
                gain,
                smooth_frames,
                ..
            } => {
                let channels = match msg {
                    ToWorker::DenoiseStartChannels { channels, .. } => channels as usize,
                    _ => 1,
                };
                self.denoising = None;
                self.streaming = None;
                if self.denoiser.is_none() {
                    out.send(inference_error(run, "no denoiser loaded".into()), None);
                    return;
                }
                let samples = att.samples.as_deref().unwrap_or_default();
                let started = check_smooth_frames(smooth_frames).and_then(|smooth_frames| {
                    Denoiser::interleaved(samples, channels)
                        .map(|d| d.with_gain(gain).with_smooth_frames(smooth_frames))
                });
                match started {
                    Ok(d) => {
                        let total = d.blocks() as u32;
                        self.denoising = Some((run, d));
                        out.send(
                            FromWorker::DenoiseProgress {
                                run,
                                done: 0,
                                total,
                            },
                            None,
                        );
                    }
                    Err(message) => out.send(inference_error(run, message), None),
                }
            }
            ToWorker::DenoiseStep { run } => {
                let Some((_, model)) = self.denoiser.as_mut() else {
                    return;
                };
                let Some((_, d)) = self.denoising.as_mut().filter(|(id, _)| *id == run) else {
                    return;
                };
                match d.step(model) {
                    Ok(true) => out.send(
                        FromWorker::DenoiseProgress {
                            run,
                            done: d.blocks_done() as u32,
                            total: d.blocks() as u32,
                        },
                        None,
                    ),
                    Ok(false) => {
                        if let Some((_, d)) = self.denoising.take() {
                            out.send(FromWorker::Denoised { run }, Some(&d.finish()));
                        }
                    }
                    Err(message) => {
                        self.denoising = None;
                        out.send(inference_error(run, message), None);
                    }
                }
            }
            ToWorker::DenoiseStreamStart {
                run,
                channels,
                frames,
                gain,
                smooth_frames,
            } => {
                self.denoising = None;
                self.streaming = None;
                if self.denoiser.is_none() {
                    out.send(inference_error(run, "no denoiser loaded".into()), None);
                    return;
                }
                let whole = frames.is_finite() && frames >= 0.0 && frames.fract() == 0.0;
                let started = if whole {
                    check_smooth_frames(smooth_frames).and_then(|smooth_frames| {
                        StreamDenoiser::new(frames as u64, channels as usize)
                            .map(|d| d.with_gain(gain).with_smooth_frames(smooth_frames))
                    })
                } else {
                    Err(format!("{frames} is not a number of samples"))
                };
                match started {
                    Ok(d) => {
                        let total = progress_count(d.blocks());
                        self.streaming = Some((run, d));
                        out.send(
                            FromWorker::DenoiseProgress {
                                run,
                                done: 0,
                                total,
                            },
                            None,
                        );
                    }
                    Err(message) => out.send(inference_error(run, message), None),
                }
            }
            ToWorker::DenoiseMeasure { run } => {
                let Some((_, d)) = self.streaming.as_mut().filter(|(id, _)| *id == run) else {
                    return;
                };
                match d.measure(att.samples.as_deref().unwrap_or_default()) {
                    Ok(_) => out.send(FromWorker::DenoiseMeasured { run }, None),
                    Err(message) => {
                        self.streaming = None;
                        out.send(inference_error(run, message), None);
                    }
                }
            }
            ToWorker::DenoisePiece { run } => {
                let Some((_, model)) = self.denoiser.as_mut() else {
                    return;
                };
                let Some((_, d)) = self.streaming.as_mut().filter(|(id, _)| *id == run) else {
                    return;
                };
                match d.push(att.samples.as_deref().unwrap_or_default(), model) {
                    Ok(samples) => {
                        let msg = FromWorker::DenoisedPiece {
                            run,
                            done: progress_count(d.blocks_done()),
                            total: progress_count(d.blocks()),
                        };
                        if d.finished() {
                            self.streaming = None;
                        }
                        out.send(msg, Some(&samples));
                    }
                    Err(message) => {
                        self.streaming = None;
                        out.send(inference_error(run, message), None);
                    }
                }
            }
            ToWorker::Rescore { run, windows } => {
                let Some(head) = self.head.as_ref().map(|(_, h)| h).filter(|h| h.is_bat()) else {
                    out.send(
                        inference_error(run, "no regional model loaded".into()),
                        None,
                    );
                    return;
                };
                let len = Method::Bats.embedding_len();
                let emb = att.embedding.unwrap_or_default();
                if emb.len() != windows as usize * len {
                    out.send(
                        inference_error(run, "embeddings do not match the window count".into()),
                        None,
                    );
                    return;
                }
                let mut top = Vec::with_capacity(windows as usize);
                for e in emb.chunks_exact(len) {
                    match head.bat_logits(e) {
                        Some(Ok(logits)) => top.push(top_scores(&logits)),
                        Some(Err(err)) => {
                            out.send(inference_error(run, format!("{err:#}")), None);
                            return;
                        }
                        None => return,
                    }
                }
                out.send(FromWorker::Rescored { run, top }, None);
            }
            ToWorker::Live { req, source_rate } => {
                let t = now_ms();
                if source_rate == 0 {
                    out.send(inference_error(req, "no sample rate".into()), None);
                    return;
                }
                let samples = att.samples.as_deref().unwrap_or_default();
                // The model in the audio slot says which: BirdNET's backbone,
                // BirdNET+ V3.0, or Perch's embedding model with its classifier.
                let payload = match (
                    self.audio.as_ref().map(|(_, m)| m),
                    self.head.as_ref().map(|(_, h)| h),
                ) {
                    (Some(AudioModel::BirdNet(backbone)), _) => {
                        let window = live_window(samples, source_rate, Method::BirdNet);
                        backbone.run(&window).map(|b| {
                            let mut payload = b.embedding;
                            payload.extend(b.logits.iter().map(|&l| sigmoid(l)));
                            payload
                        })
                    }
                    (Some(AudioModel::BirdNetV3(model)), _) => {
                        let window = live_window(samples, source_rate, Method::BirdNetV3);
                        model.run(&window).map(|o| {
                            let mut payload = o.embedding;
                            payload.extend(o.confidences);
                            payload
                        })
                    }
                    (Some(AudioModel::Perch(embed)), Some(HeadModel::Perch(classifier))) => {
                        let window = live_window(samples, source_rate, Method::Perch);
                        embed.run(&window).map(|e| {
                            let logits = classifier.scores(&e.spatial);
                            let mut payload = e.embedding;
                            payload.extend(perch::softmax(&logits));
                            payload
                        })
                    }
                    (Some(AudioModel::Perch(_)), _) => {
                        out.send(
                            inference_error(req, "Perch's classifier is not loaded".into()),
                            None,
                        );
                        return;
                    }
                    (None, _) => {
                        out.send(inference_error(req, "no model is loaded".into()), None);
                        return;
                    }
                };
                match payload {
                    Ok(payload) => out.send(
                        FromWorker::Live {
                            req,
                            ms: now_ms() - t,
                        },
                        Some(&payload),
                    ),
                    Err(e) => out.send(inference_error(req, format!("{e:#}")), None),
                }
            }
            ToWorker::Occurrence {
                req,
                lat,
                lon,
                week,
            } => {
                let res = crate::geomodel::Geomodel::load(att.bytes.as_deref().unwrap_or_default())
                    .and_then(|g| g.scores(lat, lon, week));
                match res {
                    Ok(scores) => out.send(FromWorker::Occurrence { req, scores }, None),
                    Err(e) => out.send(load_error(format!("{e:#}")), None),
                }
            }
        }
    }

    /// Run the models on `windows` and send each result. Returns false (and ends
    /// the run) after an error.
    fn score_windows(&mut self, run: u32, windows: Vec<Window>, out: &mut dyn Outbox) -> bool {
        if windows.is_empty() {
            return true;
        }
        let method = self
            .run
            .as_ref()
            .map(|r| r.method)
            .unwrap_or(Method::BirdNet);
        let fail = |this: &mut Self, out: &mut dyn Outbox, message: String| {
            this.run = None;
            out.send(inference_error(run, message), None);
            false
        };
        let audio = self.audio.as_ref().map(|(_, m)| m);
        let head = self.head.as_ref().map(|(_, h)| h);
        // Check the models once, not per window.
        let ready = match (method, audio, head) {
            (Method::BirdNet, Some(AudioModel::BirdNet(_)), _) => Ok(()),
            (Method::Bats, Some(AudioModel::BirdNet(_)), Some(h)) if h.is_bat() => Ok(()),
            (Method::Bats, Some(AudioModel::BirdNet(_)), _) => Err("no regional model loaded"),
            (Method::Perch, Some(AudioModel::Perch(_)), Some(HeadModel::Perch(_))) => Ok(()),
            (Method::Perch, Some(AudioModel::Perch(_)), _) => {
                Err("Perch's classifier is not loaded")
            }
            (Method::BirdNetV3, Some(AudioModel::BirdNetV3(_)), _) => Ok(()),
            _ => Err("no model loaded"),
        };
        if let Err(message) = ready {
            return fail(self, out, message.into());
        }
        for w in windows {
            let res = match (audio, head) {
                (Some(AudioModel::BirdNet(backbone)), head) => {
                    backbone.run(&w.samples).and_then(|b| {
                        let bat = match method {
                            Method::Bats => head.and_then(|h| h.bat_logits(&b.embedding)),
                            _ => None,
                        };
                        let logits = match bat {
                            Some(l) => l?,
                            None => b.logits,
                        };
                        Ok((top_scores(&logits), b.embedding))
                    })
                }
                (Some(AudioModel::Perch(embed)), Some(HeadModel::Perch(classifier))) => {
                    embed.run(&w.samples).map(|e| {
                        let logits = classifier.scores(&e.spatial);
                        (perch::top_scores(&logits), e.embedding)
                    })
                }
                (Some(AudioModel::BirdNetV3(model)), _) => v3_window(model, &w.samples),
                _ => unreachable!("checked above"),
            };
            match res {
                Ok((top, embedding)) => out.send(
                    FromWorker::Window {
                        run,
                        index: w.index,
                        start_s: w.start_s,
                        end_s: w.end_s,
                        top,
                    },
                    Some(&embedding),
                ),
                Err(e) => return fail(self, out, format!("{e:#}")),
            }
        }
        true
    }
}

/// BirdNET+ V3.0's scores and embedding for one window. A window of digital
/// silence is not run: it gets no scores and an embedding of zeros, so later
/// windows keep their places.
fn v3_window(
    model: &v3::Model,
    samples: &[f32],
) -> tract_onnx::prelude::TractResult<(Vec<oversample_ipc::classifier::Score>, Vec<f32>)> {
    if v3::is_digital_silence(samples) {
        return Ok((Vec::new(), vec![0.0; Method::BirdNetV3.embedding_len()]));
    }
    // The model's outputs went through its own sigmoid already.
    model
        .run(samples)
        .map(|o| (top_confidences(&o.confidences), o.embedding))
}

/// The last window of `samples` (3 s for BirdNET and BirdNET+ V3.0, 5 s for
/// Perch) at the
/// model's rate: resampled from `source_rate`, then cut or zero-padded at the
/// front to one window.
pub fn live_window(samples: &[f32], source_rate: u32, method: Method) -> Vec<f32> {
    let (rate, len) = (method.model_rate(), method.window_samples());
    let want_in = (len as u64 * source_rate as u64).div_ceil(rate as u64) as usize;
    let tail = &samples[samples.len().saturating_sub(want_in)..];
    let mut buf = Vec::with_capacity(len + 1024);
    let mut r = StreamResampler::new(source_rate, rate);
    r.push(tail, &mut buf);
    r.finish(&mut buf);
    if buf.len() >= len {
        buf.drain(..buf.len() - len);
        buf
    } else {
        let mut w = vec![0.0; len - buf.len()];
        w.extend(buf);
        w
    }
}

/// A count of blocks for a progress message. A recording has fewer than
/// `u32::MAX` blocks unless it is over 17 years long at 1 MHz.
fn progress_count(blocks: u64) -> u32 {
    blocks.min(u64::from(u32::MAX)) as u32
}

fn load_error(message: String) -> FromWorker {
    FromWorker::Error {
        run: None,
        error: ClassifierError::ModelLoad { message },
    }
}

fn inference_error(run: u32, message: String) -> FromWorker {
    FromWorker::Error {
        run: Some(run),
        error: ClassifierError::Inference { message },
    }
}

#[cfg(test)]
mod tests {
    use super::live_window;
    use oversample_ipc::classifier::{Method, PERCH_WINDOW_SAMPLES, WINDOW_SAMPLES};

    #[test]
    fn live_window_pads_short_audio_at_the_front() {
        let w = live_window(&vec![0.5; 48_000], 48_000, Method::BirdNet);
        assert_eq!(w.len(), WINDOW_SAMPLES);
        assert_eq!(w[0], 0.0);
        assert_eq!(w[WINDOW_SAMPLES - 1], 0.5);
    }

    #[test]
    fn live_window_keeps_the_last_three_seconds() {
        let mut s = vec![1.0; 48_000];
        s.extend(vec![0.25; WINDOW_SAMPLES]);
        let w = live_window(&s, 48_000, Method::BirdNet);
        assert_eq!(w.len(), WINDOW_SAMPLES);
        assert!(w.iter().all(|&v| v == 0.25));
        // Other rates come out one window long too.
        assert_eq!(
            live_window(&vec![0.1; 400_000], 256_000, Method::BirdNet).len(),
            WINDOW_SAMPLES
        );
        assert_eq!(
            live_window(&vec![0.1; 30_000], 22_050, Method::BirdNet).len(),
            WINDOW_SAMPLES
        );
    }

    #[test]
    fn perch_live_window_is_five_seconds_at_32_khz() {
        let mut s = vec![1.0; 32_000];
        s.extend(vec![0.25; PERCH_WINDOW_SAMPLES]);
        let w = live_window(&s, 32_000, Method::Perch);
        assert_eq!(w.len(), PERCH_WINDOW_SAMPLES);
        assert!(w.iter().all(|&v| v == 0.25));
        let w = live_window(&vec![0.5; 48_000], 48_000, Method::Perch);
        assert_eq!(w.len(), PERCH_WINDOW_SAMPLES);
        assert_eq!(w[0], 0.0);
    }
}

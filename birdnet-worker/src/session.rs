//! The worker's state and message handling, without any browser types, so the
//! native tests drive the same code the Web Worker runs.

use crate::denoise::{Denoiser, TractMask};
use crate::engine::{top_scores, Backbone, Head};
use crate::perch;
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
}

/// The model that scores the audio model's output.
enum HeadModel {
    Bat(Head),
    Perch(perch::Classifier),
}

#[derive(Default)]
pub struct Session {
    audio: Option<(String, AudioModel)>,
    head: Option<(String, HeadModel)>,
    run: Option<Run>,
    /// The denoiser's model, and the recording it is working through.
    denoiser: Option<(String, TractMask)>,
    denoising: Option<(u32, Denoiser)>,
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
            }
            ToWorker::DenoiseStart { run } => {
                self.denoising = None;
                if self.denoiser.is_none() {
                    out.send(inference_error(run, "no denoiser loaded".into()), None);
                    return;
                }
                match Denoiser::new(att.samples.as_deref().unwrap_or_default()) {
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
            ToWorker::Rescore { run, windows } => {
                let Some((_, HeadModel::Bat(head))) = self.head.as_ref() else {
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
                    match head.run(e) {
                        Ok(logits) => top.push(top_scores(&logits)),
                        Err(err) => {
                            out.send(inference_error(run, format!("{err:#}")), None);
                            return;
                        }
                    }
                }
                out.send(FromWorker::Rescored { run, top }, None);
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
            (Method::Bats, Some(AudioModel::BirdNet(_)), Some(HeadModel::Bat(_))) => Ok(()),
            (Method::Bats, Some(AudioModel::BirdNet(_)), _) => Err("no regional model loaded"),
            (Method::Perch, Some(AudioModel::Perch(_)), Some(HeadModel::Perch(_))) => Ok(()),
            (Method::Perch, Some(AudioModel::Perch(_)), _) => {
                Err("Perch's classifier is not loaded")
            }
            _ => Err("no model loaded"),
        };
        if let Err(message) = ready {
            return fail(self, out, message.into());
        }
        for w in windows {
            let res = match (audio, head) {
                (Some(AudioModel::BirdNet(backbone)), head) => {
                    backbone.run(&w.samples).and_then(|b| {
                        let logits = match (method, head) {
                            (Method::Bats, Some(HeadModel::Bat(h))) => h.run(&b.embedding)?,
                            _ => b.logits,
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

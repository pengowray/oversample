//! The worker's state and message handling, without any browser types, so the
//! native tests drive the same code the Web Worker runs.

use crate::engine::{top_scores, Backbone, Head};
use crate::windows::{Window, Windower};
use oversample_ipc::classifier::{ClassifierError, FromWorker, Method, ToWorker, EMBEDDING_LEN};

/// Arrays that arrive next to a [`ToWorker`] message.
#[derive(Default)]
pub struct Attached {
    pub bytes: Option<Vec<u8>>,
    pub samples: Option<Vec<f32>>,
    pub embedding: Option<Vec<f32>>,
}

/// Where the worker's answers go: the message, and for `Window` its embedding.
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

#[derive(Default)]
pub struct Session {
    backbone: Option<(String, Backbone)>,
    head: Option<(String, Head)>,
    run: Option<Run>,
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
            ToWorker::LoadBackbone { key } => {
                // Free the old model first: two at once may not fit on a phone.
                self.backbone = None;
                let t = now_ms();
                match Backbone::load(att.bytes.as_deref().unwrap_or_default()) {
                    Ok(b) => {
                        self.backbone = Some((key.clone(), b));
                        out.send(
                            FromWorker::Loaded {
                                key,
                                ms: now_ms() - t,
                            },
                            None,
                        );
                    }
                    Err(e) => out.send(load_error(None, e), None),
                }
            }
            ToWorker::LoadHead { key } => {
                self.head = None;
                let t = now_ms();
                match Head::load(att.bytes.as_deref().unwrap_or_default()) {
                    Ok(h) => {
                        self.head = Some((key.clone(), h));
                        out.send(
                            FromWorker::Loaded {
                                key,
                                ms: now_ms() - t,
                            },
                            None,
                        );
                    }
                    Err(e) => out.send(load_error(None, e), None),
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
            }
            ToWorker::Rescore { run, windows } => {
                let Some((_, head)) = self.head.as_ref() else {
                    out.send(
                        inference_error(run, "no regional model loaded".into()),
                        None,
                    );
                    return;
                };
                let emb = att.embedding.unwrap_or_default();
                if emb.len() != windows as usize * EMBEDDING_LEN {
                    out.send(
                        inference_error(run, "embeddings do not match the window count".into()),
                        None,
                    );
                    return;
                }
                let mut top = Vec::with_capacity(windows as usize);
                for e in emb.chunks_exact(EMBEDDING_LEN) {
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
        let Some((_, backbone)) = self.backbone.as_ref() else {
            return fail(self, out, "no model loaded".into());
        };
        let head = match method {
            Method::BirdNet => None,
            Method::Bats => match self.head.as_ref() {
                Some((_, h)) => Some(h),
                None => return fail(self, out, "no regional model loaded".into()),
            },
        };
        for w in windows {
            let res = backbone.run(&w.samples).and_then(|b| {
                let logits = match head {
                    Some(h) => h.run(&b.embedding)?,
                    None => b.logits,
                };
                Ok((top_scores(&logits), b.embedding))
            });
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

fn load_error(run: Option<u32>, e: tract_onnx::prelude::TractError) -> FromWorker {
    FromWorker::Error {
        run,
        error: ClassifierError::ModelLoad {
            message: format!("{e:#}"),
        },
    }
}

fn inference_error(run: u32, message: String) -> FromWorker {
    FromWorker::Error {
        run: Some(run),
        error: ClassifierError::Inference { message },
    }
}

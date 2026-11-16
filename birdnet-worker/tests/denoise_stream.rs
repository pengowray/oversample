//! A recording sent to the worker in pieces comes out with the samples of the
//! same recording sent whole, bit for bit, with the denoiser's real network.
//!
//! Uses the parity folder's `model.onnx` and `input.f32` (see
//! `denoise_parity.rs`), and is skipped when the folder is missing.

use birdnet_worker::session::{Attached, Session};
use oversample_ipc::classifier::{DenoiseGain, FromWorker, ModelPart, ToWorker};
use std::path::PathBuf;

fn parity_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("DENOISE_PARITY_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("projects/batchi-noisier2noise/runs/parity"))
        })?;
    dir.join("model.onnx").exists().then_some(dir)
}

fn read_f32(path: PathBuf) -> Vec<f32> {
    std::fs::read(path)
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// A session with the model loaded, and a way to send it one message and
/// get its one answer. Recordings are started with `gain`.
struct Worker {
    session: Session,
    gain: DenoiseGain,
}

impl Worker {
    fn new(model: Vec<u8>) -> Worker {
        let mut worker = Worker {
            session: Session::new(),
            gain: DenoiseGain::Plain,
        };
        let loaded = worker.send(
            ToWorker::Load {
                key: "denoiser".into(),
                part: ModelPart::Denoiser,
            },
            Attached {
                bytes: Some(model),
                ..Attached::default()
            },
        );
        assert!(
            matches!(loaded, (FromWorker::Loaded { .. }, _)),
            "{loaded:?}"
        );
        worker
    }

    fn send(&mut self, msg: ToWorker, att: Attached) -> (FromWorker, Option<Vec<f32>>) {
        let mut answers = Vec::new();
        let mut out = |m: FromWorker, a: Option<&[f32]>| answers.push((m, a.map(<[f32]>::to_vec)));
        self.session.handle(msg, att, &mut out, &|| 0.0);
        assert_eq!(answers.len(), 1, "one answer for each message");
        answers.pop().unwrap()
    }

    fn samples(samples: &[f32]) -> Attached {
        Attached {
            samples: Some(samples.to_vec()),
            ..Attached::default()
        }
    }

    /// The recording sent whole, as the app sends a short one.
    fn whole(&mut self, input: &[f32], channels: u32) -> Vec<f32> {
        let gain = self.gain;
        let start = if channels == 1 {
            ToWorker::DenoiseStart { run: 7, gain }
        } else {
            ToWorker::DenoiseStartChannels {
                run: 7,
                channels,
                gain,
            }
        };
        let started = self.send(start, Worker::samples(input));
        assert!(
            matches!(started, (FromWorker::DenoiseProgress { done: 0, .. }, _)),
            "{started:?}"
        );
        loop {
            match self.send(ToWorker::DenoiseStep { run: 7 }, Attached::default()) {
                (FromWorker::DenoiseProgress { .. }, _) => {}
                (FromWorker::Denoised { run: 7 }, Some(samples)) => break samples,
                other => panic!("{other:?}"),
            }
        }
    }

    /// The recording sent in pieces of `piece` samples of each channel, as
    /// the app sends a long one.
    fn in_pieces(&mut self, input: &[f32], channels: u32, piece: usize) -> Vec<f32> {
        let frames = input.len() / channels as usize;
        let started = self.send(
            ToWorker::DenoiseStreamStart {
                run: 9,
                channels,
                frames: frames as f64,
                gain: self.gain,
            },
            Attached::default(),
        );
        let (FromWorker::DenoiseProgress { done: 0, total, .. }, _) = started else {
            panic!("{started:?}");
        };
        for p in input.chunks(piece * channels as usize) {
            let measured = self.send(ToWorker::DenoiseMeasure { run: 9 }, Worker::samples(p));
            assert!(
                matches!(measured, (FromWorker::DenoiseMeasured { run: 9 }, None)),
                "{measured:?}"
            );
        }
        let mut output = Vec::new();
        let mut last = 0;
        for p in input.chunks(piece * channels as usize) {
            match self.send(ToWorker::DenoisePiece { run: 9 }, Worker::samples(p)) {
                (
                    FromWorker::DenoisedPiece {
                        run: 9,
                        done,
                        total: of,
                    },
                    Some(samples),
                ) => {
                    assert!(done >= last && of == total);
                    last = done;
                    output.extend(samples);
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(last, total);
        output
    }
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|s| s.to_bits()).collect()
}

#[test]
fn pieces_give_the_samples_of_the_whole_recording() {
    let Some(dir) = parity_dir() else {
        eprintln!("SKIP: no parity folder (see batchi-noisier2noise/reference.py)");
        return;
    };
    let input = read_f32(dir.join("input.f32"));
    let mut worker = Worker::new(std::fs::read(dir.join("model.onnx")).unwrap());

    for gain in [
        DenoiseGain::Plain,
        DenoiseGain::Smoothed,
        DenoiseGain::Heavy,
    ] {
        worker.gain = gain;
        // One block of the denoiser is 131,072 samples: pieces of that size,
        // and of a size that does not line up with the blocks.
        let mono = worker.whole(&input, 1);
        for piece in [131_072, 100_000] {
            let out = worker.in_pieces(&input, 1, piece);
            assert_eq!(out.len(), mono.len());
            assert!(
                bits(&out) == bits(&mono),
                "{gain:?} mono, pieces of {piece}"
            );
        }

        // Two channels: the recording, and the recording from a little later
        // at a lower level.
        let n = input.len() - 40_000;
        let stereo: Vec<f32> = (0..n)
            .flat_map(|i| [input[i], input[i + 40_000] * 0.5])
            .collect();
        let whole = worker.whole(&stereo, 2);
        let out = worker.in_pieces(&stereo, 2, 100_000);
        assert_eq!(out.len(), whole.len());
        assert!(bits(&out) == bits(&whole), "{gain:?} stereo");

        // A recording shorter than one block.
        let short = &input[..50_000];
        let whole = worker.whole(short, 1);
        assert!(
            bits(&worker.in_pieces(short, 1, 20_000)) == bits(&whole),
            "{gain:?} short"
        );
    }
}

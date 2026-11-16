//! The smoothed and heavy gain chains against their Python reference.
//!
//! `reference.py --chain smoothed` (or `heavy`) in the `batchi-noisier2noise`
//! project writes a folder as for `denoise_parity.rs`, plus `chain.txt`. By
//! default the folders are `runs/parity-smoothed` and `runs/parity-heavy`
//! there; `DENOISE_PARITY_SMOOTHED_DIR` and `DENOISE_PARITY_HEAVY_DIR` name
//! others. Each test runs the input through the worker's session whole and in
//! pieces, as the app does, checks that the two give the same samples, and
//! compares them with the reference. It is skipped when its folder is
//! missing.

use birdnet_worker::session::{Attached, Session};
use oversample_ipc::classifier::{DenoiseGain, FromWorker, ModelPart, ToWorker};
use std::path::PathBuf;

fn parity_dir(chain: &str) -> Option<PathBuf> {
    let var = format!("DENOISE_PARITY_{}_DIR", chain.to_uppercase());
    let dir = std::env::var_os(var).map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h).join(format!("projects/batchi-noisier2noise/runs/parity-{chain}"))
        })
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

/// Sends one message and returns its one answer.
fn send(session: &mut Session, msg: ToWorker, att: Attached) -> (FromWorker, Option<Vec<f32>>) {
    let mut answers = Vec::new();
    let mut out = |m: FromWorker, a: Option<&[f32]>| answers.push((m, a.map(<[f32]>::to_vec)));
    session.handle(msg, att, &mut out, &|| 0.0);
    assert_eq!(answers.len(), 1, "{answers:?}");
    answers.pop().unwrap()
}

fn samples(s: &[f32]) -> Attached {
    Attached {
        samples: Some(s.to_vec()),
        ..Attached::default()
    }
}

/// The recording sent whole, then in pieces of `piece` samples.
fn denoise(
    session: &mut Session,
    input: &[f32],
    gain: DenoiseGain,
    piece: usize,
) -> (Vec<f32>, Vec<f32>) {
    let started = send(
        session,
        ToWorker::DenoiseStart { run: 7, gain },
        samples(input),
    );
    assert!(
        matches!(started, (FromWorker::DenoiseProgress { done: 0, .. }, _)),
        "{started:?}"
    );
    let whole = loop {
        match send(
            session,
            ToWorker::DenoiseStep { run: 7 },
            Attached::default(),
        ) {
            (FromWorker::DenoiseProgress { .. }, _) => {}
            (FromWorker::Denoised { run: 7 }, Some(samples)) => break samples,
            other => panic!("{other:?}"),
        }
    };

    let start = ToWorker::DenoiseStreamStart {
        run: 9,
        channels: 1,
        frames: input.len() as f64,
        gain,
    };
    send(session, start, Attached::default());
    for p in input.chunks(piece) {
        send(session, ToWorker::DenoiseMeasure { run: 9 }, samples(p));
    }
    let mut pieces = Vec::new();
    for p in input.chunks(piece) {
        match send(session, ToWorker::DenoisePiece { run: 9 }, samples(p)) {
            (FromWorker::DenoisedPiece { run: 9, .. }, Some(s)) => pieces.extend(s),
            other => panic!("{other:?}"),
        }
    }
    (whole, pieces)
}

fn matches_the_reference(chain: &str, gain: DenoiseGain) {
    let Some(dir) = parity_dir(chain) else {
        eprintln!("SKIP: no parity-{chain} folder (see batchi-noisier2noise/reference.py --chain)");
        return;
    };
    let written = std::fs::read_to_string(dir.join("chain.txt")).unwrap_or_default();
    assert_eq!(
        written.trim(),
        chain,
        "{} holds another chain",
        dir.display()
    );
    let input = read_f32(dir.join("input.f32"));
    let expected = read_f32(dir.join("expected.f32"));

    let mut session = Session::new();
    let model = Attached {
        bytes: Some(std::fs::read(dir.join("model.onnx")).unwrap()),
        ..Attached::default()
    };
    let load = ToWorker::Load {
        key: "denoiser".into(),
        part: ModelPart::Denoiser,
    };
    let loaded = send(&mut session, load, model);
    assert!(
        matches!(loaded, (FromWorker::Loaded { .. }, _)),
        "{loaded:?}"
    );

    let (output, pieces) = denoise(&mut session, &input, gain, 100_000);
    assert_eq!(output.len(), expected.len());
    let bits = |v: &[f32]| v.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
    assert!(
        bits(&pieces) == bits(&output),
        "pieces differ from the whole"
    );

    let rms =
        |v: &[f32]| (v.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
    let diff: Vec<f32> = output.iter().zip(&expected).map(|(a, b)| a - b).collect();
    let relative = rms(&diff) / rms(&expected);
    let largest = diff.iter().fold(0.0f32, |a, d| a.max(d.abs()));
    eprintln!(
        "{chain}: {} samples: {:.4}% of the reference's RMS apart, largest difference {largest:.3e}; \
         input RMS {:.6}, output RMS {:.6}",
        output.len(),
        relative * 100.0,
        rms(&input),
        rms(&output)
    );
    // tract and onnxruntime round differently, which leaves about 0.0001%.
    // Taking the gains round each block's edges from its own network run
    // instead of the next block's would leave 0.04% to 1.5%
    // (`chain_check.py`), so the limit is well under that.
    assert!(
        relative < 1e-4,
        "{chain} differs from the reference by {:.4}% of its RMS",
        relative * 100.0
    );
}

#[test]
fn the_smoothed_chain_matches_the_python_reference() {
    matches_the_reference("smoothed", DenoiseGain::Smoothed);
}

#[test]
fn the_heavy_chain_matches_the_python_reference() {
    matches_the_reference("heavy", DenoiseGain::Heavy);
}

//! The denoiser against its Python reference.
//!
//! `reference.py` in the `batchi-noisier2noise` project writes a folder with
//! `model.onnx`, `input.f32` (mono samples) and `expected.f32` (what its own
//! denoise step gives). This test runs the same input through the worker's
//! session, as the app does, and compares. It is skipped when the folder is
//! missing. `DENOISE_PARITY_DIR` names another folder.

use birdnet_worker::session::{Attached, Session};
use oversample_ipc::classifier::{FromWorker, ModelPart, ToWorker};
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

#[test]
fn the_worker_matches_the_python_reference() {
    let Some(dir) = parity_dir() else {
        eprintln!("SKIP: no parity folder (see batchi-noisier2noise/reference.py)");
        return;
    };
    let input = read_f32(dir.join("input.f32"));
    let expected = read_f32(dir.join("expected.f32"));

    let mut session = Session::new();
    let mut answers: Vec<(FromWorker, Option<Vec<f32>>)> = Vec::new();
    let mut send = |session: &mut Session, msg: ToWorker, att: Attached| {
        let mut out = |m: FromWorker, a: Option<&[f32]>| answers.push((m, a.map(<[f32]>::to_vec)));
        session.handle(msg, att, &mut out, &|| 0.0);
        answers.pop()
    };

    let loaded = send(
        &mut session,
        ToWorker::Load {
            key: "denoiser".into(),
            part: ModelPart::Denoiser,
        },
        Attached {
            bytes: Some(std::fs::read(dir.join("model.onnx")).unwrap()),
            ..Attached::default()
        },
    );
    assert!(
        matches!(loaded, Some((FromWorker::Loaded { .. }, _))),
        "{loaded:?}"
    );

    let started = send(
        &mut session,
        ToWorker::DenoiseStart { run: 7 },
        Attached {
            samples: Some(input.clone()),
            ..Attached::default()
        },
    );
    let Some((FromWorker::DenoiseProgress { total, done: 0, .. }, _)) = started else {
        panic!("{started:?}");
    };

    let mut steps = 0;
    let output = loop {
        steps += 1;
        match send(
            &mut session,
            ToWorker::DenoiseStep { run: 7 },
            Attached::default(),
        ) {
            Some((FromWorker::DenoiseProgress { done, .. }, _)) => assert_eq!(done, steps),
            Some((FromWorker::Denoised { run: 7 }, Some(samples))) => break samples,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(steps, total);
    assert_eq!(output.len(), expected.len());

    let rms = |v: &[f32]| (v.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
    let diff: Vec<f32> = output.iter().zip(&expected).map(|(a, b)| a - b).collect();
    let relative = rms(&diff) / rms(&expected);
    // tract and onnxruntime round differently; the audio paths are the same.
    assert!(
        relative < 0.01,
        "differs from the reference by {:.2}% of its RMS",
        relative * 100.0
    );
    eprintln!(
        "{} samples, {total} blocks: {:.4}% of the reference's RMS apart; input RMS {:.6}, output RMS {:.6}",
        output.len(),
        relative * 100.0,
        rms(&input),
        rms(&output)
    );
}

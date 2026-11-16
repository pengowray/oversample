//! The denoiser's real network on recordings the parity test does not cover:
//! long digital silence, and two channels.
//!
//! Uses the parity folder's `model.onnx` and `input.f32` (see
//! `denoise_parity.rs`), and is skipped when the folder is missing.

use birdnet_worker::session::{Attached, Session};
use oversample_ipc::classifier::{DenoiseGain, FromWorker, ModelPart, ToWorker};
use std::path::{Path, PathBuf};

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

/// `input` (interleaved when `channels` is over 1) through the worker's
/// session with the model in `dir`, as the app sends it.
fn denoise(dir: &Path, input: &[f32], channels: u32) -> Vec<f32> {
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

    let gain = DenoiseGain::Plain;
    let start = if channels == 1 {
        ToWorker::DenoiseStart { run: 7, gain }
    } else {
        ToWorker::DenoiseStartChannels {
            run: 7,
            channels,
            gain,
        }
    };
    let started = send(
        &mut session,
        start,
        Attached {
            samples: Some(input.to_vec()),
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
    assert_eq!(output.len(), input.len());
    output
}

fn rms(v: &[f32]) -> f64 {
    (v.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

fn channel(interleaved: &[f32], c: usize) -> Vec<f32> {
    interleaved.iter().skip(c).step_by(2).copied().collect()
}

/// A recording with a long stretch of digital silence: no NaN or infinite
/// samples come out, the silence stays silent, and the part with sound is
/// denoised about as it is without the silence.
#[test]
fn a_recording_that_is_mostly_digital_silence_is_denoised() {
    let Some(dir) = parity_dir() else {
        eprintln!("SKIP: no parity folder (see batchi-noisier2noise/reference.py)");
        return;
    };
    let whole = read_f32(dir.join("input.f32"));
    // Sound in the first 30%, zeros after it.
    let cut = whole.len() * 3 / 10;
    let mut input = whole.clone();
    input[cut..].fill(0.0);

    let output = denoise(&dir, &input, 1);
    assert!(output.iter().all(|s| s.is_finite()));
    // A window past the last sample with sound, the output is silent.
    assert!(output[cut + 1024..].iter().all(|s| *s == 0.0));

    // The sounding part, away from the cut, against the same samples
    // denoised on their own.
    let alone = denoise(&dir, &whole[..cut], 1);
    let keep = cut - 16_384;
    let diff: Vec<f32> = output[..keep]
        .iter()
        .zip(&alone[..keep])
        .map(|(a, b)| a - b)
        .collect();
    let relative = rms(&diff) / rms(&alone[..keep]);
    assert!(
        relative < 0.05,
        "the sounding part differs by {:.2}% of its RMS",
        relative * 100.0
    );
    eprintln!(
        "sounding part: {:.4}% apart; input RMS {:.6}, output RMS {:.6}",
        relative * 100.0,
        rms(&input[..keep]),
        rms(&output[..keep])
    );
}

/// Two channels: the same recording in both gives the mono result in both,
/// and a quieter right channel keeps its level against the left.
#[test]
fn two_channels_share_one_gain() {
    let Some(dir) = parity_dir() else {
        eprintln!("SKIP: no parity folder (see batchi-noisier2noise/reference.py)");
        return;
    };
    let mono = read_f32(dir.join("input.f32"));
    let expected = denoise(&dir, &mono, 1);

    let same: Vec<f32> = mono.iter().flat_map(|s| [*s, *s]).collect();
    let output = denoise(&dir, &same, 2);
    assert!(channel(&output, 0) == expected);
    assert!(channel(&output, 1) == expected);

    // The right channel at half the level: the features take off each bin's
    // median, so its gains are close to the left's, and with one gain for
    // both the copy's right channel is half its left one.
    let panned: Vec<f32> = mono.iter().flat_map(|s| [*s, *s * 0.5]).collect();
    let output = denoise(&dir, &panned, 2);
    assert!(output.iter().all(|s| s.is_finite()));
    let (left, right) = (channel(&output, 0), channel(&output, 1));
    let ratio = rms(&right) / rms(&left);
    assert!((ratio - 0.5).abs() < 1e-4, "right over left is {ratio}");
    assert!(rms(&left) < rms(&mono), "the left channel was not lowered");
}

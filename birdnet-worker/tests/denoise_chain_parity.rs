//! The smoothed and heavy gain chains against their Python reference.
//!
//! `reference.py --chain smoothed` (or `heavy`) in the `batchi-noisier2noise`
//! project writes a folder as for `denoise_parity.rs`, plus `chain.txt`: the
//! chain's name, then the number of frames in the median when it is not 9
//! (`smoothed 3`). By default the folders are `runs/parity-smoothed`,
//! `runs/parity-heavy` and `runs/parity-smoothed-3` there (the last from
//! `reference.py --chain smoothed --frames 3` on 2.5 s of a Daubenton's bat
//! recording at 384 kHz); `DENOISE_PARITY_SMOOTHED_DIR`,
//! `DENOISE_PARITY_HEAVY_DIR` and `DENOISE_PARITY_SMOOTHED_3_DIR` name others.
//! Each test runs the input through the worker's session whole and in
//! pieces, as the app does, checks that the two give the same samples, and
//! compares them with the reference. It is skipped when its folder is
//! missing.

use birdnet_worker::session::{Attached, Session};
use oversample_ipc::classifier::{DenoiseGain, FromWorker, ModelPart, ToWorker};
use std::path::PathBuf;

/// One parity folder: its name after `parity-`, and the chain and number of
/// frames in the median that it was written with.
struct Case {
    folder: &'static str,
    chain: &'static str,
    gain: DenoiseGain,
    smooth_frames: u32,
}

const SMOOTHED_3: Case = Case {
    folder: "smoothed-3",
    chain: "smoothed",
    gain: DenoiseGain::Smoothed,
    smooth_frames: 3,
};

fn parity_dir(folder: &str) -> Option<PathBuf> {
    let var = format!(
        "DENOISE_PARITY_{}_DIR",
        folder.to_uppercase().replace('-', "_")
    );
    let dir = std::env::var_os(var).map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h).join(format!(
                "projects/batchi-noisier2noise/runs/parity-{folder}"
            ))
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
    smooth_frames: u32,
    piece: usize,
) -> (Vec<f32>, Vec<f32>) {
    let started = send(
        session,
        ToWorker::DenoiseStart {
            run: 7,
            gain,
            smooth_frames,
        },
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
        smooth_frames,
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

/// The folder's input, the worker's output for it with `smooth_frames`, and
/// the folder's expected output. Checks that the folder was written for
/// `case` and that the pieces give the same samples as the whole. `None`
/// when the folder is missing.
fn outputs(case: &Case, smooth_frames: u32) -> Option<(Vec<f32>, Vec<f32>, Vec<f32>)> {
    let Some(dir) = parity_dir(case.folder) else {
        eprintln!(
            "SKIP: no parity-{} folder (see batchi-noisier2noise/reference.py --chain)",
            case.folder
        );
        return None;
    };
    let written = std::fs::read_to_string(dir.join("chain.txt")).unwrap_or_default();
    let mut words = written.split_whitespace();
    let chain = words.next().unwrap_or_default();
    let frames: u32 = words.next().map_or(9, |n| n.parse().unwrap());
    assert_eq!(
        (chain, frames),
        (case.chain, case.smooth_frames),
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

    let (output, pieces) = denoise(&mut session, &input, case.gain, smooth_frames, 100_000);
    assert_eq!(output.len(), expected.len());
    let bits = |v: &[f32]| v.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
    assert!(
        bits(&pieces) == bits(&output),
        "pieces differ from the whole"
    );
    Some((input, output, expected))
}

fn rms(v: &[f32]) -> f64 {
    (v.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

/// The RMS of the difference, as a share of the reference's RMS.
fn relative_difference(output: &[f32], expected: &[f32]) -> f64 {
    let diff: Vec<f32> = output.iter().zip(expected).map(|(a, b)| a - b).collect();
    rms(&diff) / rms(expected)
}

fn matches_the_reference(case: Case) {
    let Some((input, output, expected)) = outputs(&case, case.smooth_frames) else {
        return;
    };
    let relative = relative_difference(&output, &expected);
    let largest = output
        .iter()
        .zip(&expected)
        .fold(0.0f32, |a, (o, e)| a.max((o - e).abs()));
    eprintln!(
        "{}: {} samples: {:.4}% of the reference's RMS apart, largest difference {largest:.3e}; \
         input RMS {:.6}, output RMS {:.6}",
        case.folder,
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
        "{} differs from the reference by {:.4}% of its RMS",
        case.folder,
        relative * 100.0
    );
}

#[test]
fn the_smoothed_chain_matches_the_python_reference() {
    matches_the_reference(Case {
        folder: "smoothed",
        chain: "smoothed",
        gain: DenoiseGain::Smoothed,
        smooth_frames: 9,
    });
}

#[test]
fn the_heavy_chain_matches_the_python_reference() {
    matches_the_reference(Case {
        folder: "heavy",
        chain: "heavy",
        gain: DenoiseGain::Heavy,
        smooth_frames: 9,
    });
}

#[test]
fn the_smoothed_chain_with_3_frames_matches_the_python_reference() {
    matches_the_reference(SMOOTHED_3);
}

/// The 3-frame folder denoised with 9 frames must be far outside the limit
/// above, so that test would catch a worker that ignored the number of
/// frames.
#[test]
fn the_3_frame_reference_differs_from_9_frames() {
    let Some((_, output, expected)) = outputs(&SMOOTHED_3, 9) else {
        return;
    };
    let relative = relative_difference(&output, &expected);
    eprintln!(
        "smoothed-3 denoised with 9 frames: {:.4}% of the reference's RMS apart",
        relative * 100.0
    );
    assert!(relative > 1e-3, "{:.4}%", relative * 100.0);
}

//! Runs the real models on real recordings. Skipped unless these are set:
//!
//! - `BIRDNET_MODELS_DIR`: a folder with `birdnet-v2.4-embeddings-fp32.onnx`,
//!   `labels.txt`, `fp32/BattyBirdNET-{EU,UK}-256kHz_fp32.onnx` and
//!   `labels/BattyBirdNET-{EU,UK}-256kHz_Labels.txt`.
//! - `BIRDNET_SOUNDS_DIR`: the folder holding `no-bat-sounds/sounds` and
//!   `bat-demo-sounds/sounds` (the batchi-extras checkout). Defaults to
//!   `../../batchi-extras` from this crate.
//!
//! Debug builds of tract are very slow; run with
//! `BIRDNET_MODELS_DIR=... cargo test -p birdnet-worker --release --test real_models`.

use birdnet_worker::session::{Attached, Session};
use oversample_ipc::classifier::{FromWorker, Method, Score, ToWorker, EMBEDDING_LEN};
use std::path::{Path, PathBuf};

fn models_dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("BIRDNET_MODELS_DIR")?);
    d.is_dir().then_some(d)
}

fn sounds_dir() -> PathBuf {
    std::env::var_os("BIRDNET_SOUNDS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../batchi-extras"))
}

fn labels(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| l.split('_').next().unwrap_or(l).to_string())
        .collect()
}

/// Load a recording's mono mix, at most `max_s` seconds of it.
fn load(path: &Path, max_s: f64) -> (Vec<f32>, u32) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let audio = oversample_core::audio::loader::load_audio(&bytes).unwrap();
    let n = ((max_s * audio.sample_rate as f64) as usize).min(audio.samples.len());
    (audio.samples[..n].to_vec(), audio.sample_rate)
}

struct Out {
    windows: Vec<(f64, f64, Vec<Score>)>,
    embeddings: Vec<f32>,
    rescored: Option<Vec<Vec<Score>>>,
    errors: Vec<String>,
}

fn send(s: &mut Session, msg: ToWorker, att: Attached, out: &mut Out) {
    let mut sink = |m: FromWorker, emb: Option<&[f32]>| match m {
        FromWorker::Window {
            start_s,
            end_s,
            top,
            ..
        } => {
            out.windows.push((start_s, end_s, top));
            out.embeddings.extend_from_slice(emb.unwrap());
        }
        FromWorker::Rescored { top, .. } => out.rescored = Some(top),
        FromWorker::Error { error, .. } => out.errors.push(error.to_string()),
        _ => {}
    };
    s.handle(msg, att, &mut sink, &|| 0.0);
}

fn load_model(s: &mut Session, head: bool, path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    let key = path.display().to_string();
    let msg = if head {
        ToWorker::LoadHead { key }
    } else {
        ToWorker::LoadBackbone { key }
    };
    let mut out = new_out();
    send(
        s,
        msg,
        Attached {
            bytes: Some(bytes),
            ..Default::default()
        },
        &mut out,
    );
    assert!(out.errors.is_empty(), "{:?}", out.errors);
}

fn new_out() -> Out {
    Out {
        windows: Vec::new(),
        embeddings: Vec::new(),
        rescored: None,
        errors: Vec::new(),
    }
}

/// Run a whole recording through the session in 1 s pieces.
fn analyse(s: &mut Session, method: Method, samples: &[f32], rate: u32) -> Out {
    let mut out = new_out();
    send(
        s,
        ToWorker::Start {
            run: 1,
            method,
            source_rate: rate,
        },
        Attached::default(),
        &mut out,
    );
    for (seq, piece) in samples.chunks(rate as usize).enumerate() {
        send(
            s,
            ToWorker::Chunk {
                run: 1,
                seq: seq as u32,
            },
            Attached {
                samples: Some(piece.to_vec()),
                ..Default::default()
            },
            &mut out,
        );
    }
    send(
        s,
        ToWorker::Finish { run: 1 },
        Attached::default(),
        &mut out,
    );
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    out
}

/// The class with the highest confidence in any window.
fn best(windows: &[(f64, f64, Vec<Score>)]) -> Score {
    windows
        .iter()
        .filter_map(|w| w.2.first().copied())
        .max_by(|a, b| a.confidence.total_cmp(&b.confidence))
        .unwrap()
}

#[test]
fn birdnet_finds_pauraque() {
    let Some(models) = models_dir() else {
        eprintln!("BIRDNET_MODELS_DIR not set; skipped");
        return;
    };
    let names = labels(&models.join("labels.txt"));
    let mut s = Session::new();
    load_model(
        &mut s,
        false,
        &models.join("birdnet-v2.4-embeddings-fp32.onnx"),
    );
    let (samples, rate) = load(
        &sounds_dir()
            .join("no-bat-sounds/sounds/XC1083752 - Pauraque - Nyctidromus albicollis.wav"),
        30.0,
    );
    let out = analyse(&mut s, Method::BirdNet, &samples, rate);
    assert_eq!(out.embeddings.len(), out.windows.len() * EMBEDDING_LEN);
    let top = best(&out.windows);
    eprintln!(
        "{} windows at {rate} Hz; best {} {:.3}",
        out.windows.len(),
        names[top.class as usize],
        top.confidence
    );
    assert_eq!(names[top.class as usize], "Nyctidromus albicollis");
    assert!(top.confidence > 0.9);
}

#[test]
fn bats_eu_finds_common_pipistrelle_and_rescore_matches_uk_run() {
    let Some(models) = models_dir() else {
        eprintln!("BIRDNET_MODELS_DIR not set; skipped");
        return;
    };
    let eu = labels(&models.join("labels/BattyBirdNET-EU-256kHz_Labels.txt"));
    let mut s = Session::new();
    load_model(
        &mut s,
        false,
        &models.join("birdnet-v2.4-embeddings-fp32.onnx"),
    );
    load_model(
        &mut s,
        true,
        &models.join("fp32/BattyBirdNET-EU-256kHz_fp32.onnx"),
    );
    let (samples, rate) = load(
        &sounds_dir().join(
            "bat-demo-sounds/sounds/XC1062064 - Common Pipistrelle - Pipistrellus pipistrellus.wav",
        ),
        6.0,
    );
    let out = analyse(&mut s, Method::Bats, &samples, rate);
    let top = best(&out.windows);
    eprintln!(
        "{} windows at {rate} Hz; best {} {:.3}",
        out.windows.len(),
        eu[top.class as usize],
        top.confidence
    );
    assert_eq!(eu[top.class as usize], "Pipistrellus pipistrellus");

    // Scoring the stored embeddings with the UK head gives what a fresh UK run
    // gives.
    load_model(
        &mut s,
        true,
        &models.join("fp32/BattyBirdNET-UK-256kHz_fp32.onnx"),
    );
    let mut re = new_out();
    send(
        &mut s,
        ToWorker::Rescore {
            run: 2,
            windows: out.windows.len() as u32,
        },
        Attached {
            embedding: Some(out.embeddings.clone()),
            ..Default::default()
        },
        &mut re,
    );
    assert!(re.errors.is_empty(), "{:?}", re.errors);
    let rescored = re.rescored.unwrap();
    let fresh = analyse(&mut s, Method::Bats, &samples, rate);
    assert_eq!(rescored.len(), fresh.windows.len());
    for (a, b) in rescored.iter().zip(&fresh.windows) {
        assert_eq!(a, &b.2);
    }
}

#[test]
fn bats_refuse_low_sample_rate() {
    let mut s = Session::new();
    let mut out = new_out();
    send(
        &mut s,
        ToWorker::Start {
            run: 1,
            method: Method::Bats,
            source_rate: 48_000,
        },
        Attached::default(),
        &mut out,
    );
    assert_eq!(
        out.errors,
        vec!["Bat analysis needs a sample rate of at least 96 kHz. This file is 48 kHz."]
    );
}

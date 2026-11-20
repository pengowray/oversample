//! Runs the real models on real recordings. Skipped unless these are set:
//!
//! - `BIRDNET_MODELS_DIR`: a folder with `birdnet-v2.4-embeddings-fp32.onnx`,
//!   `labels.txt`, `fp32/BattyBirdNET-{EU,UK}-256kHz_fp32.onnx` and
//!   `labels/BattyBirdNET-{EU,UK}-256kHz_Labels.txt`.
//! - `BIRDNET_SOUNDS_DIR`: the folder holding `no-bat-sounds/sounds` and
//!   `bat-demo-sounds/sounds` (the batchi-extras checkout). Defaults to
//!   `../../batchi-extras` from this crate.
//!
//! - `BIRDNET_V3_MODEL`: `BirdNET+_V3.0_Global_12K_FP16_pruned.onnx` from
//!   Zenodo record 24362980, for the BirdNET+ V3.0 test. Its labels are the
//!   app's trimmed copy in `research/birdnet-v3-bench`.
//!
//! Debug builds of tract are very slow; run with
//! `BIRDNET_MODELS_DIR=... cargo test -p birdnet-worker --release --test real_models`.

use birdnet_worker::session::{Attached, Session};
use oversample_ipc::classifier::{
    FromWorker, Method, ModelPart, Score, ToWorker, EMBEDDING_LEN, PERCH_CLASSES,
    PERCH_EMBEDDING_LEN, V3_CLASSES, V3_EMBEDDING_LEN,
};
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
    let part = if head {
        ModelPart::BatHead
    } else {
        ModelPart::Backbone
    };
    load_part(s, part, path);
}

fn load_part(s: &mut Session, part: ModelPart, path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    let key = path.display().to_string();
    let msg = ToWorker::Load { key, part };
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
    analyse_pass(s, method, samples, rate, Vec::new())
}

/// One pass of a run (`refine_passes`) over a whole recording, in 1 s pieces.
fn analyse_pass(
    s: &mut Session,
    method: Method,
    samples: &[f32],
    rate: u32,
    offsets: Vec<u32>,
) -> Out {
    let mut out = new_out();
    send(
        s,
        ToWorker::Start {
            run: 1,
            method,
            source_rate: rate,
            offsets,
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

/// A run with a 1 s step on the Gentoo Penguin recording: its first pass
/// gives exactly what a run without a step gives, and the second fills in the
/// starts between, 3 times as many windows in all, each scored as if the
/// whole run had gone front to back.
#[test]
fn birdnet_step_1s_on_gentoo_penguin() {
    use oversample_ipc::classifier::refine_passes;
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
            .join("bat-demo-sounds/sounds/XC540767 - Gentoo Penguin - Pygoscelis papua.mp3"),
        18.0,
    );
    let duration = samples.len() as f64 / rate as f64;
    let plain = analyse(&mut s, Method::BirdNet, &samples, rate);
    let method = Method::BirdNet;
    let passes = refine_passes(
        method.window_samples() as u32,
        method.step_samples(1.0).unwrap(),
    );
    assert_eq!(passes.len(), 2);
    let coarse = analyse_pass(&mut s, method, &samples, rate, passes[0].offsets.clone());
    let fine = analyse_pass(&mut s, method, &samples, rate, passes[1].offsets.clone());
    // Step 3 s: the same windows and scores as before.
    assert_eq!(coarse.windows, plain.windows);
    assert_eq!(coarse.embeddings, plain.embeddings);
    let mut all: Vec<(f64, f64, Vec<Score>)> = coarse.windows.clone();
    all.extend(fine.windows.iter().cloned());
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    eprintln!(
        "{duration:.2} s at {rate} Hz: {} windows end to end, {} with a 1 s step",
        plain.windows.len(),
        all.len()
    );
    // Starts every 1 s, no start twice, every window inside the recording.
    for (k, w) in all.iter().enumerate() {
        assert_eq!(w.0, k as f64);
        assert!(w.1 <= duration + 1e-9);
    }
    assert_eq!(all.len(), 3 * plain.windows.len() - 2);
    for w in &all {
        let top = w.2[0];
        eprintln!(
            "{:>4.1} to {:>4.1} s: {} {:.3}",
            w.0, w.1, names[top.class as usize], top.confidence
        );
    }
    // Every window hears penguins, as the 3 s ones do.
    let penguin = all
        .iter()
        .filter(|w| names[w.2[0].class as usize] == "Pygoscelis papua")
        .count();
    assert!(penguin * 10 >= all.len() * 8, "{penguin} of {}", all.len());
}

/// `PERCH_MODELS_DIR`: a folder with `perch_v2_embed.tract.onnx`,
/// `perch_v2_head.int8.bin` and `labels.csv` (huggingface.co/pengowray/perch-v2-tract
/// and cgeorgiaw/Perch `assets/labels.csv`).
fn perch_dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("PERCH_MODELS_DIR")?);
    d.is_dir().then_some(d)
}

/// Perch through the worker's own resampling and windowing names a bird and a
/// katydid that it named in `research/perch.md`'s tests (at 0.86 and 0.88 there).
#[test]
fn perch_finds_pauraque_and_a_katydid() {
    let Some(models) = perch_dir() else {
        eprintln!("PERCH_MODELS_DIR not set; skipped");
        return;
    };
    // labels.csv: a header line, then one name per class.
    let names: Vec<String> = std::fs::read_to_string(models.join("labels.csv"))
        .unwrap()
        .lines()
        .skip(1)
        .map(str::to_string)
        .collect();
    assert_eq!(names.len(), PERCH_CLASSES);
    let mut s = Session::new();
    load_part(
        &mut s,
        ModelPart::PerchEmbedding,
        &models.join("perch_v2_embed.tract.onnx"),
    );
    load_part(
        &mut s,
        ModelPart::PerchClassifier,
        &models.join("perch_v2_head.int8.bin"),
    );
    for (file, species) in [
        (
            "XC1083752 - Pauraque - Nyctidromus albicollis.wav",
            "Nyctidromus albicollis",
        ),
        ("XC875004 -  - Mecopoda elongata.wav", "Mecopoda elongata"),
    ] {
        let (samples, rate) = load(&sounds_dir().join("no-bat-sounds/sounds").join(file), 30.0);
        let t = std::time::Instant::now();
        let out = analyse(&mut s, Method::Perch, &samples, rate);
        assert_eq!(
            out.embeddings.len(),
            out.windows.len() * PERCH_EMBEDDING_LEN
        );
        assert_eq!(out.windows[0].1, 5.0);
        for w in &out.windows {
            let total: f32 = w.2.iter().map(|s| s.confidence).sum();
            assert!(total <= 1.0001, "softmax shares add up to {total}");
        }
        let top = best(&out.windows);
        eprintln!(
            "{file}: {} windows at {rate} Hz in {:.1} s; best {} {:.3}",
            out.windows.len(),
            t.elapsed().as_secs_f64(),
            names[top.class as usize],
            top.confidence
        );
        assert_eq!(names[top.class as usize], species);
        assert!(top.confidence > 0.6);
    }
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

    // Scoring stored embeddings with the UK head gives what a fresh UK run
    // gives, on a recording long enough for many windows.
    let (samples, rate) = load(
        &sounds_dir().join(
            "bat-demo-sounds/sounds/XC1060750 - Soprano Pipistrelle - Pipistrellus pygmaeus.wav",
        ),
        12.0,
    );
    let out = analyse(&mut s, Method::Bats, &samples, rate);
    let top = best(&out.windows);
    eprintln!(
        "{} windows at {rate} Hz; best {} {:.3}",
        out.windows.len(),
        eu[top.class as usize],
        top.confidence
    );
    assert!(out.windows.len() > 10);
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
            offsets: Vec::new(),
        },
        Attached::default(),
        &mut out,
    );
    assert_eq!(
        out.errors,
        vec!["Bat analysis needs a sample rate of at least 96 kHz. This file is 48 kHz."]
    );
}

/// Every regional head in the catalog that is in `BIRDNET_MODELS_DIR` gives one
/// output per line of its label file.
#[test]
fn heads_match_their_labels() {
    use birdnet_worker::engine::Head;
    use oversample_ipc::classifier::{catalog, BatRegion};
    let Some(models) = models_dir() else {
        eprintln!("BIRDNET_MODELS_DIR not set; skipped");
        return;
    };
    let mut checked = 0;
    for r in BatRegion::ALL {
        let head = models.join("fp32").join(catalog::bat_head(r).id);
        let lab = models.join("labels").join(catalog::bat_labels(r).id);
        if !head.exists() || !lab.exists() {
            eprintln!("{r:?}: not in BIRDNET_MODELS_DIR; skipped");
            continue;
        }
        let h = Head::load(&std::fs::read(&head).unwrap()).unwrap();
        let n = h.run(&[0.0; EMBEDDING_LEN]).unwrap().len();
        let names = labels(&lab);
        assert_eq!(n, names.len(), "{r:?}");
        checked += 1;
    }
    eprintln!("{checked} heads checked");
}

/// "All regions": the 11 heads loaded as one give each species the highest
/// confidence any single head gives it. Scored on made-up embeddings, so it
/// needs the heads and label lists but not the backbone.
#[test]
fn all_regions_take_each_species_highest_score() {
    use oversample_ipc::classifier::{catalog, merge_bat_labels, BatHeadPart, BatRegion, TOP_K};
    let Some(models) = models_dir() else {
        eprintln!("BIRDNET_MODELS_DIR not set; skipped");
        return;
    };
    let head_path = |r| models.join("fp32").join(catalog::bat_head(r).id);
    let label_path = |r| models.join("labels").join(catalog::bat_labels(r).id);
    if BatRegion::ALL
        .iter()
        .any(|&r| !head_path(r).exists() || !label_path(r).exists())
    {
        eprintln!("not every region is in BIRDNET_MODELS_DIR; skipped");
        return;
    }
    let texts: Vec<String> = BatRegion::ALL
        .iter()
        .map(|&r| std::fs::read_to_string(label_path(r)).unwrap())
        .collect();
    let merged = merge_bat_labels(&texts.iter().map(String::as_str).collect::<Vec<_>>());
    eprintln!(
        "{} merged classes, {} of them species",
        merged.lines.len(),
        merged.species
    );

    // Embeddings like the backbone's: most values 0, the rest positive.
    let windows = 40usize;
    let mut seed = 0x2545_f491_u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    let emb: Vec<f32> = (0..windows * EMBEDDING_LEN)
        .map(|_| (next() * 4.0 - 2.5).max(0.0))
        .collect();
    let rescore = |s: &mut Session| {
        let mut out = new_out();
        send(
            s,
            ToWorker::Rescore {
                run: 1,
                windows: windows as u32,
            },
            Attached {
                embedding: Some(emb.clone()),
                ..Default::default()
            },
            &mut out,
        );
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        out.rescored.unwrap()
    };

    // Each head on its own: the best confidence per merged species.
    let mut best = vec![vec![0f32; merged.species]; windows];
    for (r, map) in BatRegion::ALL.iter().zip(&merged.maps) {
        let mut s = Session::new();
        load_model(&mut s, true, &head_path(*r));
        for (w, top) in rescore(&mut s).iter().enumerate() {
            for score in top {
                let class = map[score.class as usize] as usize;
                if class < merged.species {
                    best[w][class] = best[w][class].max(score.confidence);
                }
            }
        }
    }

    let mut bytes = Vec::new();
    let mut heads = Vec::new();
    for (r, map) in BatRegion::ALL.iter().zip(&merged.maps) {
        let file = std::fs::read(head_path(*r)).unwrap();
        heads.push(BatHeadPart {
            len: file.len() as u32,
            classes: map.clone(),
        });
        bytes.extend(file);
    }
    let (all_bytes, all_heads) = (bytes.clone(), heads.clone());
    let mut s = Session::new();
    let mut out = new_out();
    send(
        &mut s,
        ToWorker::LoadBatHeads {
            key: "all".into(),
            heads,
            species: merged.species as u16,
            classes: merged.lines.len() as u16,
        },
        Attached {
            bytes: Some(bytes),
            ..Default::default()
        },
        &mut out,
    );
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let all = rescore(&mut s);
    assert_eq!(all.len(), windows);
    let mut species_on_top = 0;
    for (w, top) in all.iter().enumerate() {
        assert_eq!(top.len(), TOP_K);
        assert!(top.windows(2).all(|p| p[0].confidence >= p[1].confidence));
        species_on_top += usize::from((top[0].class as usize) < merged.species);
        for score in top {
            let class = score.class as usize;
            // A head's own top 10 can leave a species out, so only a species
            // some head listed can be compared exactly.
            if class < merged.species && best[w][class] > 0.0 {
                assert!(
                    (score.confidence - best[w][class]).abs() < 1e-5,
                    "window {w}, {}: {} merged, {} best single",
                    merged.lines[class],
                    score.confidence,
                    best[w][class]
                );
            }
        }
        // The best species of any single head is the best merged species.
        let single = best[w].iter().copied().fold(0f32, f32::max);
        let merged_best = top
            .iter()
            .filter(|s| (s.class as usize) < merged.species)
            .map(|s| s.confidence)
            .fold(0f32, f32::max);
        assert!((single - merged_best).abs() < 1e-5, "window {w}");
    }
    eprintln!("a species is the top class in {species_on_top} of {windows} windows");
    // "noise" and the like still get a score: one head has no such class,
    // and it must not set them all to 0. They seldom reach a top 10, so read
    // every merged logit.
    let merged_heads = birdnet_worker::engine::MergedHeads::load(
        &all_bytes,
        &all_heads,
        merged.species,
        merged.lines.len(),
    )
    .unwrap();
    let logits = merged_heads.run(&emb[..EMBEDDING_LEN]).unwrap();
    assert_eq!(logits.len(), merged.lines.len());
    let scored = logits[merged.species..]
        .iter()
        .filter(|l| l.is_finite())
        .count();
    eprintln!(
        "{scored} of {} non-species classes have a score",
        merged.lines.len() - merged.species
    );
    assert_eq!(scored, merged.lines.len() - merged.species);
}

/// The live view's `Live` request on the 3 s ending at a window's end gives the
/// same top class and nearly the same score as the run's window, so the live
/// view and the classifier agree. Also prints the time per reading.
#[test]
fn live_matches_the_run() {
    let Some(models) = models_dir() else {
        eprintln!("BIRDNET_MODELS_DIR not set; skipped");
        return;
    };
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
    for (k, (start, end, top)) in out.windows.iter().enumerate().take(4) {
        let a = (start * rate as f64) as usize;
        let b = ((end * rate as f64) as usize).min(samples.len());
        if b - a < 3 * rate as usize {
            continue;
        }
        let mut live: Option<Vec<f32>> = None;
        let mut sink = |m: FromWorker, emb: Option<&[f32]>| {
            if let FromWorker::Live { .. } = m {
                live = emb.map(<[f32]>::to_vec);
            }
        };
        let t = std::time::Instant::now();
        s.handle(
            ToWorker::Live {
                req: k as u32,
                source_rate: rate,
                at: None,
            },
            Attached {
                samples: Some(samples[a..b].to_vec()),
                ..Default::default()
            },
            &mut sink,
            &|| 0.0,
        );
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let live = live.expect("a Live answer");
        let scores = &live[EMBEDDING_LEN..];
        let (best, score) = scores
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.total_cmp(y.1))
            .unwrap();
        eprintln!(
            "window {k}: run {} {:.3}, live {best} {score:.3}, {ms:.0} ms",
            top[0].class, top[0].confidence
        );
        assert_eq!(best, top[0].class as usize);
        assert!((score - top[0].confidence).abs() < 0.05);
    }
}

/// The live view's reading of a file window (`Live` with `at`, on a cut made
/// by `live_cut`) is the run's window with the same start: the same top 10
/// classes and confidences and the same embedding, to float rounding. For
/// the windows end to end and those a 1 s step adds between them.
fn kept_reading_matches(s: &mut Session, method: Method, samples: &[f32], rate: u32) -> usize {
    use oversample_ipc::classifier::{live_cut, refine_passes};
    let win = method.window_samples() as u32;
    let mut run = new_out();
    for pass in refine_passes(win, method.step_samples(1.0).unwrap()) {
        let out = analyse_pass(s, method, samples, rate, pass.offsets);
        run.windows.extend(out.windows);
        run.embeddings.extend(out.embeddings);
    }
    let len = method.embedding_len();
    let mut checked = 0;
    for (k, (start, end, top)) in run.windows.iter().enumerate() {
        if end - start < method.window_secs() - 1e-9 {
            continue;
        }
        let cut = live_cut(method, rate, *start, samples.len() as u64).expect("inside");
        assert_eq!(cut.start_s, *start);
        let from = cut.from as usize;
        let mut live = None;
        let mut sink = |m: FromWorker, emb: Option<&[f32]>| {
            if let FromWorker::Live { top, .. } = m {
                live = Some((top, emb.unwrap().to_vec()));
            }
        };
        s.handle(
            ToWorker::Live {
                req: k as u32,
                source_rate: rate,
                at: Some(cut.skip),
            },
            Attached {
                samples: Some(samples[from..from + cut.len as usize].to_vec()),
                ..Default::default()
            },
            &mut sink,
            &|| 0.0,
        );
        let (live_top, payload) = live.expect("a Live answer");
        let run_emb = &run.embeddings[k * len..(k + 1) * len];
        let emb_diff = payload[..len]
            .iter()
            .zip(run_emb)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        let conf_diff = live_top
            .iter()
            .zip(top)
            .map(|(a, b)| (a.confidence - b.confidence).abs())
            .fold(0.0f32, f32::max);
        eprintln!(
            "{method:?} {start:>5.2} s: top {} {:.4}, largest difference: embedding {emb_diff:.1e}, confidence {conf_diff:.1e}",
            top[0].class, top[0].confidence
        );
        let classes = |t: &[Score]| t.iter().map(|s| s.class).collect::<Vec<_>>();
        assert_eq!(classes(&live_top), classes(top), "{method:?} at {start} s");
        assert!(conf_diff < 1e-4, "{method:?} at {start} s: {conf_diff}");
        assert!(emb_diff < 1e-3, "{method:?} at {start} s: {emb_diff}");
        // The full scores sent with it agree with the top 10.
        for t in &live_top {
            assert!((payload[len + t.class as usize] - t.confidence).abs() < 1e-6);
        }
        checked += 1;
    }
    checked
}

#[test]
fn birdnet_kept_reading_is_the_run_window() {
    let Some(models) = models_dir() else {
        eprintln!("BIRDNET_MODELS_DIR not set; skipped");
        return;
    };
    let mut s = Session::new();
    load_model(
        &mut s,
        false,
        &models.join("birdnet-v2.4-embeddings-fp32.onnx"),
    );
    let (samples, rate) = load(
        &sounds_dir()
            .join("no-bat-sounds/sounds/XC1083752 - Pauraque - Nyctidromus albicollis.wav"),
        12.0,
    );
    let checked = kept_reading_matches(&mut s, Method::BirdNet, &samples, rate);
    eprintln!("{checked} windows at {rate} Hz");
    assert!(checked >= 8);
}

#[test]
fn perch_kept_reading_is_the_run_window() {
    let Some(models) = perch_dir() else {
        eprintln!("PERCH_MODELS_DIR not set; skipped");
        return;
    };
    let mut s = Session::new();
    load_part(
        &mut s,
        ModelPart::PerchEmbedding,
        &models.join("perch_v2_embed.tract.onnx"),
    );
    load_part(
        &mut s,
        ModelPart::PerchClassifier,
        &models.join("perch_v2_head.int8.bin"),
    );
    let (samples, rate) = load(
        &sounds_dir()
            .join("bat-demo-sounds/sounds/XC540767 - Gentoo Penguin - Pygoscelis papua.mp3"),
        9.0,
    );
    let checked = kept_reading_matches(&mut s, Method::Perch, &samples, rate);
    eprintln!("{checked} windows at {rate} Hz");
    assert!(checked >= 4);
}

/// Perch's `Live` request on the 5 s of a run's window gives the same top
/// class and nearly the same softmax share as the run, and the same embedding.
/// Also prints the time per reading.
#[test]
fn perch_live_matches_the_run() {
    let Some(models) = perch_dir() else {
        eprintln!("PERCH_MODELS_DIR not set; skipped");
        return;
    };
    let mut s = Session::new();
    load_part(
        &mut s,
        ModelPart::PerchEmbedding,
        &models.join("perch_v2_embed.tract.onnx"),
    );
    load_part(
        &mut s,
        ModelPart::PerchClassifier,
        &models.join("perch_v2_head.int8.bin"),
    );
    let (samples, rate) = load(
        &sounds_dir()
            .join("no-bat-sounds/sounds/XC1083752 - Pauraque - Nyctidromus albicollis.wav"),
        30.0,
    );
    let out = analyse(&mut s, Method::Perch, &samples, rate);
    let mut checked = 0;
    for (k, (start, end, top)) in out.windows.iter().enumerate().take(4) {
        let a = (start * rate as f64) as usize;
        let b = ((end * rate as f64) as usize).min(samples.len());
        if b - a < 5 * rate as usize {
            continue;
        }
        let mut live: Option<Vec<f32>> = None;
        let mut sink = |m: FromWorker, emb: Option<&[f32]>| {
            if let FromWorker::Live { .. } = m {
                live = emb.map(<[f32]>::to_vec);
            }
        };
        let t = std::time::Instant::now();
        s.handle(
            ToWorker::Live {
                req: k as u32,
                source_rate: rate,
                at: None,
            },
            Attached {
                samples: Some(samples[a..b].to_vec()),
                ..Default::default()
            },
            &mut sink,
            &|| 0.0,
        );
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let live = live.expect("a Live answer");
        assert_eq!(live.len(), PERCH_EMBEDDING_LEN + PERCH_CLASSES);
        let (embedding, scores) = live.split_at(PERCH_EMBEDDING_LEN);
        let total: f32 = scores.iter().sum();
        assert!(
            (total - 1.0).abs() < 1e-3,
            "softmax shares add up to {total}"
        );
        let (best, score) = scores
            .iter()
            .enumerate()
            .max_by(|x, y| x.1.total_cmp(y.1))
            .unwrap();
        let run_emb = &out.embeddings[k * PERCH_EMBEDDING_LEN..(k + 1) * PERCH_EMBEDDING_LEN];
        let dot: f32 = embedding.iter().zip(run_emb).map(|(x, y)| x * y).sum();
        let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
        let cos = dot / (norm(embedding) * norm(run_emb));
        eprintln!(
            "window {k}: run {} {:.3}, live {best} {score:.3}, embedding cosine {cos:.5}, {ms:.0} ms",
            top[0].class, top[0].confidence
        );
        assert_eq!(best, top[0].class as usize);
        assert!((score - top[0].confidence).abs() < 0.05);
        assert!(cos > 0.99);
        checked += 1;
    }
    assert!(checked > 0, "no whole 5 s window to check");
}

/// BirdNET+ V3.0 (the official pruned float16 file, patched to float32 as it
/// loads) names the Gentoo Penguin in each of the 6 windows that
/// `research/birdnet-v3-bench/results/ref-pruned.txt` lists for onnxruntime,
/// at about the same confidence (the reference used Python's decoder and
/// resampler, so the scores differ a little). The confidences are the model's
/// own: a second sigmoid would turn 0.90 into 0.71. Windows of digital silence
/// get no scores and an embedding of zeros.
#[test]
fn birdnet_v3_finds_gentoo_penguin_and_skips_digital_silence() {
    let Some(model) = std::env::var_os("BIRDNET_V3_MODEL").map(PathBuf::from) else {
        eprintln!("BIRDNET_V3_MODEL not set; skipped");
        return;
    };
    let names: Vec<String> = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../research/birdnet-v3-bench/birdnet-v3-labels.tsv"),
    )
    .unwrap()
    .lines()
    .map(|l| l.split('\t').nth(1).unwrap().to_string())
    .collect();
    assert_eq!(names.len(), V3_CLASSES);
    let mut s = Session::new();
    let t = std::time::Instant::now();
    load_part(&mut s, ModelPart::BirdNetV3, &model);
    eprintln!("loaded in {:.1} s", t.elapsed().as_secs_f64());
    let (mut samples, rate) = load(
        &sounds_dir()
            .join("bat-demo-sounds/sounds/XC540767 - Gentoo Penguin - Pygoscelis papua.mp3"),
        18.0,
    );
    // 9 s of digital silence after the 6 windows of the recording.
    samples.resize(samples.len() + 9 * rate as usize, 0.0);
    let t = std::time::Instant::now();
    let out = analyse(&mut s, Method::BirdNetV3, &samples, rate);
    eprintln!(
        "{} windows at {rate} Hz in {:.1} s",
        out.windows.len(),
        t.elapsed().as_secs_f64()
    );
    assert_eq!(out.windows.len(), 9);
    assert_eq!(out.embeddings.len(), 9 * V3_EMBEDDING_LEN);
    assert_eq!(out.windows[0].1, 3.0);
    // onnxruntime, ref-pruned.txt windows 0 to 5.
    let reference = [0.725, 0.871, 0.902, 0.772, 0.461, 0.869];
    for (i, (w, want)) in out.windows.iter().zip(reference).enumerate() {
        let top = w.2[0];
        eprintln!(
            "window {i}: {} {:.3} (reference {want:.3})",
            names[top.class as usize], top.confidence
        );
        assert_eq!(names[top.class as usize], "Pygoscelis papua");
        assert!(
            (top.confidence - want).abs() < 0.15,
            "window {i}: {:.3}, reference {want:.3}",
            top.confidence
        );
        assert!(w.2.windows(2).all(|p| p[0].confidence >= p[1].confidence));
    }
    // Window 6 starts where the silence does; the resampler's filter may
    // leave the recording's last echo in its first samples.
    if let Some(top) = out.windows[6].2.first() {
        eprintln!(
            "window 6 (starts at the silence): {} {:.3}",
            names[top.class as usize], top.confidence
        );
    }
    for i in 7..9 {
        assert!(out.windows[i].2.is_empty(), "window {i} was scored");
        let e = &out.embeddings[i * V3_EMBEDDING_LEN..(i + 1) * V3_EMBEDDING_LEN];
        assert!(e.iter().all(|&v| v == 0.0));
    }
    assert!(out.embeddings[..V3_EMBEDDING_LEN].iter().any(|&v| v != 0.0));
}

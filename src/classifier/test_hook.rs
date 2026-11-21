//! `window.__oversample_classifier`, for end-to-end tests and for trying the
//! engine before its panel exists. Every call goes through the public API in
//! [`super`], so it starts nothing the panel couldn't.
//!
//! - `refresh()`, `download(choice)`, `deleteAll()`
//! - `run(choice)`: analyse the current file
//! - `seed(json)`: made-up results for the current file (see [`seed`])
//! - `rescore(region)`, `cancel()`, `unload()`
//! - `status()`: a JSON string with downloads, the run status and a summary of
//!   each result (window count, completeness, best class) and of each file's
//!   bird sound boxes
//! - `seedBoxes(json)`: made-up bird box windows for the current file (see
//!   [`seed_boxes`]); `boxes()`: the boxes the spectrogram draws, as JSON
//!
//! `choice` is `"birdnet"`, `"perch"` or `"bats:<region>"`, and `region` is
//! `"all"` or a file-name part such as `"EU"`, `"UK"` or `"USA-EAST"` (see
//! `BatScope::stem`).

use super::*;
use wasm_bindgen::prelude::*;

fn parse_region(s: &str) -> Option<BatScope> {
    std::iter::once(BatScope::All)
        .chain(BatRegion::ALL.into_iter().map(BatScope::Region))
        .find(|r| r.stem().eq_ignore_ascii_case(s))
}

fn parse_choice(s: &str) -> Option<ModelChoice> {
    if s.eq_ignore_ascii_case("birdnet") {
        return Some(ModelChoice::BirdNet);
    }
    if s.eq_ignore_ascii_case("perch") {
        return Some(ModelChoice::Perch);
    }
    if s.eq_ignore_ascii_case("birdnet-v3") {
        return Some(ModelChoice::BirdNetV3);
    }
    let r = s.strip_prefix("bats:")?;
    parse_region(r).map(ModelChoice::Bats)
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::BirdNet => "BirdNet",
        Method::Bats => "Bats",
        Method::Perch => "Perch",
        Method::BirdNetV3 => "BirdNetV3",
    }
}

fn download_text(s: &DownloadStatus) -> String {
    match s {
        DownloadStatus::Unknown => "Unknown".into(),
        DownloadStatus::NotDownloaded => "NotDownloaded".into(),
        DownloadStatus::Downloading { fraction } => format!("Downloading {fraction:.3}"),
        DownloadStatus::Downloaded => "Downloaded".into(),
        DownloadStatus::Failed(e) => format!("Failed: {e}"),
    }
}

/// Plain text, not `Debug`, so the hook pulls little formatting code into the
/// app's WASM.
fn run_text(s: &RunStatus) -> String {
    match s {
        RunStatus::Idle => "Idle".into(),
        RunStatus::LoadingModel { file_id, method } => {
            format!("LoadingModel {file_id} {}", method_name(*method))
        }
        RunStatus::Running {
            file_id,
            method,
            fraction,
            refining,
            ..
        } => {
            let pass = refining.map_or_else(String::new, |s| format!(" refining {s}"));
            format!(
                "Running {file_id} {} {fraction:.3}{pass}",
                method_name(*method)
            )
        }
        RunStatus::Rescoring { file_id, scope } => {
            format!("Rescoring {file_id} {}", scope.stem())
        }
        RunStatus::Done { file_id, method } => format!("Done {file_id} {}", method_name(*method)),
        RunStatus::Failed {
            file_id,
            method,
            error,
        } => format!("Failed {file_id} {}: {error}", method_name(*method)),
    }
}

fn status_json(state: AppState) -> String {
    let mut downloads: Vec<String> = state.classifier.downloads().with_untracked(|m| {
        m.iter()
            .map(|(id, s)| format!("{id}: {}", download_text(s)))
            .collect()
    });
    downloads.sort();
    let run = run_text(&state.classifier.run().get_untracked());
    let results: Vec<serde_json::Value> = state.classifier.results().with_untracked(|m| {
        m.iter()
            .map(|((file_id, method), r)| {
                let best = r
                    .windows
                    .iter()
                    .filter_map(|w| w.top.first().map(|s| (w.start_s, *s)))
                    .max_by(|a, b| a.1.confidence.total_cmp(&b.1.confidence))
                    .map(|(t, s)| {
                        format!(
                            "{} {:.3} at {t} s",
                            r.label(&s).map_or("?", |l| l.scientific.as_str()),
                            s.confidence
                        )
                    });
                let emb = state
                    .classifier
                    .embeddings()
                    .with_untracked(|e| e.get(&(*file_id, *method)).map_or(0, Vec::len));
                serde_json::json!({
                    "file": file_id,
                    "method": method_name(*method),
                    "region": r.region.map(|g| g.stem()),
                    "windows": r.windows.len(),
                    "embeddingFloats": emb,
                    "complete": r.complete,
                    "stepSecs": r.step_secs,
                    "spacingSecs": r.spacing_secs,
                    "best": best,
                })
            })
            .collect()
    });
    let boxes: Vec<serde_json::Value> = state.classifier.boxes().with_untracked(|m| {
        m.iter()
            .map(|(file_id, b)| {
                serde_json::json!({
                    "file": file_id,
                    "windows": b.windows,
                    "windowBoxes": b.raw.len(),
                    "samples": b.samples,
                })
            })
            .collect()
    });
    serde_json::json!({ "downloads": downloads, "run": run, "results": results, "boxes": boxes })
        .to_string()
}

/// Put made-up results in place of a run on the current file, for tests of
/// the Call detection tab without a model:
/// `{"method":"birdnet","labels":["Turdus merula_Common Blackbird",...],
/// "windows":[[0,3,[[0,0.91],[1,0.2]]],...]}` (each window: start and end in
/// seconds, then its top classes as [class, confidence]). Embeddings are
/// zeros. Returns "" or what was wrong.
fn seed(state: AppState, json: &str) -> String {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => return e.to_string(),
    };
    let method = match v["method"].as_str() {
        Some("birdnet") => Method::BirdNet,
        Some("perch") => Method::Perch,
        Some("birdnet-v3") => Method::BirdNetV3,
        Some("bats") => Method::Bats,
        _ => return "unknown method".into(),
    };
    let Some(file_id) = current_file_id(state) else {
        return "no file".into();
    };
    let labels: Vec<SpeciesLabel> = v["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str().map(SpeciesLabel::parse))
        .collect();
    let windows: Vec<WindowScores> = v["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, w)| {
            let top = w[2]
                .as_array()?
                .iter()
                .filter_map(|s| {
                    Some(Score {
                        class: s[0].as_u64()? as u16,
                        confidence: s[1].as_f64()? as f32,
                    })
                })
                .collect();
            Some(WindowScores {
                start_s: w[0].as_f64()?,
                end_s: w[1].as_f64()?,
                top,
                slot: i as u32,
                live: false,
            })
        })
        .collect();
    let window = method.window_secs();
    let n = windows.len();
    let run = ClassifierRun {
        method,
        region: (method == Method::Bats).then_some(BatScope::All),
        backbone: match method {
            Method::Perch => catalog::PERCH_FILES[0],
            Method::BirdNetV3 => &catalog::BIRDNET_V3,
            _ => &catalog::BACKBONE,
        },
        head: None,
        source_rate: 48_000,
        model_rate: 48_000,
        window_secs: window,
        step_secs: window,
        spacing_secs: window,
        labels: Arc::new(labels),
        windows,
        whole_file: true,
        complete: true,
    };
    state
        .classifier
        .embeddings()
        .update(|m| drop(m.insert((file_id, method), vec![0.0; n * method.embedding_len()])));
    state
        .classifier
        .results()
        .update(|m| drop(m.insert((file_id, method), run)));
    String::new()
}

/// Put made-up bird box windows in place of a run on the current file:
/// `{"raw":[[clip_start, t0, t1, f0, f1, conf], ...], "samples": 384000}`
/// (`samples`: 32 kHz samples in the recording; leave it out for a run still
/// going). Returns "" or what was wrong.
fn seed_boxes(state: AppState, json: &str) -> String {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => return e.to_string(),
    };
    let Some(file_id) = current_file_id(state) else {
        return "no file".into();
    };
    let rate = state.library.files().with_untracked(|fs| {
        fs.iter()
            .find(|f| f.id == file_id)
            .map_or(0, |f| f.audio.sample_rate)
    });
    let raw: Vec<oversample_ipc::classifier::boxes::RawBox> = v["raw"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| {
            let n = |i: usize| b[i].as_f64();
            Some(oversample_ipc::classifier::boxes::RawBox {
                clip_start: n(0)?,
                t0: n(1)?,
                t1: n(2)?,
                f0: n(3)?,
                f1: n(4)?,
                conf: n(5)?,
            })
        })
        .collect();
    let run = boxes::BoxRun {
        source_rate: rate,
        windows: raw.len() as u32,
        raw,
        samples: v["samples"].as_u64(),
    };
    state.classifier.boxes().update(|m| drop(m.insert(file_id, run)));
    String::new()
}

/// The boxes the spectrogram draws on the current file, as JSON.
fn drawn_json(state: AppState) -> String {
    let drawn = untrack(|| boxes::drawn(state));
    serde_json::Value::Array(
        drawn
            .iter()
            .map(|b| {
                serde_json::json!({
                    "t0": b.t0, "t1": b.t1, "f0": b.f0, "f1": b.f1,
                    "confidence": b.confidence,
                    "name": b.name,
                    "species": b.species,
                })
            })
            .collect(),
    )
    .to_string()
}

pub fn install(state: AppState) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let o = js_sys::Object::new();
    let set = |name: &str, f: JsValue| {
        let _ = js_sys::Reflect::set(&o, &JsValue::from_str(name), &f);
    };
    set(
        "refresh",
        Closure::<dyn Fn()>::new(move || refresh_downloads(state)).into_js_value(),
    );
    set(
        "download",
        Closure::<dyn Fn(String) -> bool>::new(move |c: String| match parse_choice(&c) {
            Some(c) => {
                download(state, c);
                true
            }
            None => false,
        })
        .into_js_value(),
    );
    set(
        "deleteAll",
        Closure::<dyn Fn()>::new(move || delete_files(state, catalog::all_files().collect()))
            .into_js_value(),
    );
    set(
        "run",
        // "BirdNet", or "BirdNet@1" for a 1 s step between window starts.
        Closure::<dyn Fn(String) -> bool>::new(move |c: String| {
            let (c, step) = c
                .split_once('@')
                .map_or((c.as_str(), 0.0), |(c, s)| (c, s.parse().unwrap_or(0.0)));
            let Some(choice) = parse_choice(c) else {
                return false;
            };
            let Some(file_id) = current_file_id(state) else {
                return false;
            };
            run(state, file_id, choice, step);
            true
        })
        .into_js_value(),
    );
    set(
        "rescore",
        Closure::<dyn Fn(String) -> String>::new(move |r: String| {
            let Some(region) = parse_region(&r) else {
                return "unknown region".into();
            };
            let Some(file_id) = current_file_id(state) else {
                return "no file".into();
            };
            match rescore(state, file_id, region) {
                Ok(()) => String::new(),
                Err(e) => e.to_string(),
            }
        })
        .into_js_value(),
    );
    set(
        "seed",
        Closure::<dyn Fn(String) -> String>::new(move |j: String| seed(state, &j)).into_js_value(),
    );
    set(
        "seedBoxes",
        Closure::<dyn Fn(String) -> String>::new(move |j: String| seed_boxes(state, &j))
            .into_js_value(),
    );
    set(
        "boxes",
        Closure::<dyn Fn() -> String>::new(move || drawn_json(state)).into_js_value(),
    );
    set(
        "cancel",
        Closure::<dyn Fn()>::new(move || cancel(state)).into_js_value(),
    );
    set(
        "unload",
        Closure::<dyn Fn()>::new(move || unload(state)).into_js_value(),
    );
    set(
        "status",
        Closure::<dyn Fn() -> String>::new(move || status_json(state)).into_js_value(),
    );
    let _ = js_sys::Reflect::set(&window, &"__oversample_classifier".into(), &o);
}

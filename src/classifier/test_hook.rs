//! `window.__oversample_classifier`, for end-to-end tests and for trying the
//! engine before its panel exists. Every call goes through the public API in
//! [`super`], so it starts nothing the panel couldn't.
//!
//! - `refresh()`, `download(choice)`, `deleteAll()`
//! - `run(choice)`: analyse the current file
//! - `rescore(region)`, `cancel()`, `unload()`
//! - `status()`: a JSON string with downloads, the run status and a summary of
//!   each result (window count, completeness, best class)
//!
//! `choice` is `"birdnet"`, `"perch"` or `"bats:<region>"`, and `region` a
//! file-name part such as `"EU"`, `"UK"` or `"USA-EAST"` (see
//! `BatRegion::file_stem`).

use super::*;
use wasm_bindgen::prelude::*;

fn parse_region(s: &str) -> Option<BatRegion> {
    BatRegion::ALL
        .into_iter()
        .find(|r| r.file_stem().eq_ignore_ascii_case(s))
}

fn parse_choice(s: &str) -> Option<ModelChoice> {
    if s.eq_ignore_ascii_case("birdnet") {
        return Some(ModelChoice::BirdNet);
    }
    if s.eq_ignore_ascii_case("perch") {
        return Some(ModelChoice::Perch);
    }
    let r = s.strip_prefix("bats:")?;
    parse_region(r).map(ModelChoice::Bats)
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::BirdNet => "BirdNet",
        Method::Bats => "Bats",
        Method::Perch => "Perch",
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
        } => format!("Running {file_id} {} {fraction:.3}", method_name(*method)),
        RunStatus::Rescoring { file_id, region } => {
            format!("Rescoring {file_id} {}", region.file_stem())
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
                    "region": r.region.map(|g| g.file_stem()),
                    "windows": r.windows.len(),
                    "embeddingFloats": emb,
                    "complete": r.complete,
                    "best": best,
                })
            })
            .collect()
    });
    serde_json::json!({ "downloads": downloads, "run": run, "results": results }).to_string()
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
        Closure::<dyn Fn(String) -> bool>::new(move |c: String| {
            let Some(choice) = parse_choice(&c) else {
                return false;
            };
            let Some(file_id) = current_file_id(state) else {
                return false;
            };
            run(state, file_id, choice);
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

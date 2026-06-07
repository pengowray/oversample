//! Automated live-render benchmark. Drives the synthetic mic (see
//! `synthetic_mic`) through a matrix of {sample rate × view × signal}, measures
//! the browser frame cadence (rAF intervals) + column throughput for each combo
//! with no user intervention, and emits a copy/downloadable report tagged with
//! the app version — so runs can be compared across code versions and devices.
//!
//! What actually varies the workload during *live* capture: the sample rate
//! (FFTs/s), and the view's column generation (Resonators runs a stateful EMA;
//! Spectrogram/Flow share the plain STFT path). Signal content does not change
//! cost (FFT cost is content-independent) but is swept for visual coverage and
//! to catch combos that fail to render.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::store_fields::*;
use crate::audio::synthetic_mic::{self, SynthSignal};
use crate::state::{AppState, MainView};
use crate::web_util::sleep_ms;

// ── Benchmark matrix (tunable) ───────────────────────────────────────────────
const RATES: [u32; 2] = [192_000, 384_000];
const VIEWS: [(&str, MainView); 3] = [
    ("Spectrogram", MainView::Spectrogram),
    ("Flow", MainView::Flow),
    ("Resonators", MainView::Resonators),
];
const SIGNALS: [SynthSignal; 3] = [SynthSignal::Noise, SynthSignal::Chirp, SynthSignal::Pulses];
/// Settle time before measuring (lets the waterfall start scrolling).
const SETTLE_MS: f64 = 600.0;
/// Frame-cadence measurement window per combo.
const MEASURE_MS: f64 = 1_800.0;

thread_local! {
    static BENCH_GEN: Cell<u64> = const { Cell::new(0) };
    static BENCH_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// Whether a benchmark run is in progress.
pub fn is_running() -> bool {
    BENCH_ACTIVE.with(|a| a.get())
}

/// Cancel a running benchmark (e.g. the user pressed Stop).
pub fn cancel() {
    BENCH_GEN.with(|g| g.set(g.get().wrapping_add(1)));
    BENCH_ACTIVE.with(|a| a.set(false));
}

struct ComboResult {
    rate: u32,
    view: &'static str,
    signal: &'static str,
    avg_fps: f64,
    min_fps: f64,
    p95_ms: f64,
    frames: usize,
    cols_per_s: f64,
}

/// Run the full benchmark matrix. Restores the original view + stops the synth
/// when done, then emits the report. No-op if a run is already in progress.
pub fn run(state: AppState) {
    if BENCH_ACTIVE.with(|a| a.get()) {
        return;
    }
    let gen = BENCH_GEN.with(|g| {
        let next = g.get().wrapping_add(1);
        g.set(next);
        next
    });
    BENCH_ACTIVE.with(|a| a.set(true));

    let orig_view = state.viewmode.main_view().get_untracked();
    let total = RATES.len() * VIEWS.len() * SIGNALS.len();
    state.show_info_toast(format!(
        "Benchmark started: {} combos (~{}s)",
        total,
        ((total as f64) * (SETTLE_MS + MEASURE_MS) / 1000.0).round() as i64,
    ));

    wasm_bindgen_futures::spawn_local(async move {
        let mut results: Vec<ComboResult> = Vec::new();
        let mut idx = 0usize;
        'outer: for &rate in RATES.iter() {
            for &(view_label, view) in VIEWS.iter() {
                for &signal in SIGNALS.iter() {
                    if BENCH_GEN.with(|g| g.get()) != gen {
                        break 'outer; // cancelled
                    }
                    idx += 1;
                    state.log_debug(
                        "info",
                        format!("Bench {}/{}: {} / {} @ {} kHz",
                            idx, total, view_label, signal.label(), rate / 1000),
                    );

                    // Set the view, then (re)start the synth for this combo.
                    state.viewmode.main_view().set(view);
                    synthetic_mic::start(state, signal, rate);

                    sleep_ms(SETTLE_MS as i32).await;
                    if BENCH_GEN.with(|g| g.get()) != gen {
                        break 'outer;
                    }

                    let cols0 = crate::canvas::live_waterfall::total_columns();
                    let frame_ts = measure_frames(MEASURE_MS, gen).await;
                    let cols1 = crate::canvas::live_waterfall::total_columns();
                    // Stop now if cancelled during the measurement window — don't
                    // record a partial combo or start the next one.
                    if BENCH_GEN.with(|g| g.get()) != gen {
                        break 'outer;
                    }

                    let (avg_fps, min_fps, p95_ms, frames) = frame_stats(&frame_ts);
                    let cols_per_s = (cols1.saturating_sub(cols0)) as f64 / (MEASURE_MS / 1000.0);

                    results.push(ComboResult {
                        rate,
                        view: view_label,
                        signal: signal.label(),
                        avg_fps,
                        min_fps,
                        p95_ms,
                        frames,
                        cols_per_s,
                    });
                }
            }
        }

        let cancelled = BENCH_GEN.with(|g| g.get()) != gen;
        synthetic_mic::stop(&state);
        state.viewmode.main_view().set(orig_view);
        BENCH_ACTIVE.with(|a| a.set(false));

        if results.is_empty() {
            state.show_info_toast("Benchmark cancelled");
            return;
        }
        let report = build_report(&results, cancelled);
        emit_report(&state, report, cancelled);
    });
}

/// Collect requestAnimationFrame timestamps for `window_ms`, returning the raw
/// per-frame DOMHighResTimeStamps (ms). The rAF cadence reflects how much main-
/// thread time the live render is taking: a saturated frame budget → fewer,
/// further-spaced frames → lower measured fps.
///
/// Always resolves, even if rAF stalls (a backgrounded tab suspends rAF): a
/// `setTimeout` watchdog resolves the Promise after the window regardless, so
/// the benchmark can never hang. The rAF chain also stops early if the run is
/// cancelled (gen bump) so Stop is responsive and the closure cycle is freed.
async fn measure_frames(window_ms: f64, gen: u64) -> Vec<f64> {
    let times: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(Vec::new()));
    let times_out = times.clone();

    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let Some(win) = web_sys::window() else {
            let _ = resolve.call0(&JsValue::NULL);
            return;
        };

        // Watchdog: resolve after the window even if rAF never fires again.
        {
            let resolve_w = resolve.clone();
            let watchdog = Closure::once_into_js(move || {
                let _ = resolve_w.call0(&JsValue::NULL);
            });
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                watchdog.as_ref().unchecked_ref(),
                (window_ms as i32) + 300,
            );
        }

        let cb: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
        let cb_inner = cb.clone();
        let times = times.clone();
        let start = Rc::new(Cell::new(f64::NAN));

        *cb.borrow_mut() = Some(Closure::wrap(Box::new(move |ts: f64| {
            if start.get().is_nan() {
                start.set(ts);
            }
            times.borrow_mut().push(ts);
            let done = ts - start.get() >= window_ms
                || BENCH_GEN.with(|g| g.get()) != gen; // cancelled → stop early
            if done {
                let _ = resolve.call0(&JsValue::NULL);
                // Break the self-reference cycle so the closure chain frees.
                let _ = cb_inner.borrow_mut().take();
            } else if let Some(w) = web_sys::window() {
                if let Some(ref c) = *cb_inner.borrow() {
                    let _ = w.request_animation_frame(c.as_ref().unchecked_ref());
                }
            }
        }) as Box<dyn FnMut(f64)>));

        // Schedule the first frame (scope the borrow so the `Ref` drops before
        // `cb` does at the end of this executor closure).
        {
            let b = cb.borrow();
            if let Some(c) = b.as_ref() {
                let _ = win.request_animation_frame(c.as_ref().unchecked_ref());
            }
        }
        // The closure captures `cb_inner` (a clone of `cb`), so the Rc cycle keeps
        // it alive across frames; it's freed by the `take()` above on completion.
    });

    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    let v = times_out.borrow().clone();
    v
}

/// (avg_fps, min_fps, p95_frame_ms, frame_count) from rAF timestamps.
fn frame_stats(times: &[f64]) -> (f64, f64, f64, usize) {
    let n = times.len();
    if n < 2 {
        return (0.0, 0.0, 0.0, n);
    }
    let mut deltas: Vec<f64> = times.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).collect();
    if deltas.is_empty() {
        return (0.0, 0.0, 0.0, n);
    }
    let mean = deltas.iter().sum::<f64>() / deltas.len() as f64;
    let avg_fps = if mean > 0.0 { 1000.0 / mean } else { 0.0 };
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let max_delta = *deltas.last().unwrap();
    let min_fps = if max_delta > 0.0 { 1000.0 / max_delta } else { 0.0 };
    let p95_idx = (((deltas.len() as f64) * 0.95).floor() as usize).min(deltas.len() - 1);
    let p95_ms = deltas[p95_idx];
    (avg_fps, min_fps, p95_ms, n)
}

fn build_report(results: &[ComboResult], cancelled: bool) -> String {
    let mut s = String::new();
    s.push_str("# Oversample live-render benchmark\n\n");
    s.push_str(&format!("- Version: {}\n", env!("CARGO_PKG_VERSION")));
    let date = js_sys::Date::new_0().to_iso_string();
    s.push_str(&format!("- Date: {}\n", String::from(date)));
    if let Some(nav) = web_sys::window().map(|w| w.navigator()) {
        if let Ok(ua) = nav.user_agent() {
            s.push_str(&format!("- User agent: {}\n", ua));
        }
        let cores = nav.hardware_concurrency();
        s.push_str(&format!("- Logical cores: {}\n", cores as i64));
    }
    if let Some(win) = web_sys::window() {
        let dpr = win.device_pixel_ratio();
        s.push_str(&format!("- devicePixelRatio: {:.2}\n", dpr));
    }
    s.push_str(&format!("- Per combo: {:.0}ms settle + {:.0}ms measure\n", SETTLE_MS, MEASURE_MS));
    if cancelled {
        s.push_str("- NOTE: run was cancelled early (partial results)\n");
    }
    s.push('\n');

    s.push_str("| Rate | View | Signal | avg fps | min fps | p95 ms | cols/s | frames |\n");
    s.push_str("|------|------|--------|--------:|--------:|-------:|-------:|-------:|\n");
    for r in results {
        s.push_str(&format!(
            "| {}k | {} | {} | {:.1} | {:.1} | {:.1} | {:.0} | {} |\n",
            r.rate / 1000, r.view, r.signal, r.avg_fps, r.min_fps, r.p95_ms, r.cols_per_s, r.frames,
        ));
    }

    // Summary: overall average + worst combo by avg fps.
    if !results.is_empty() {
        let mean_fps = results.iter().map(|r| r.avg_fps).sum::<f64>() / results.len() as f64;
        let worst = results.iter().min_by(|a, b| {
            a.avg_fps.partial_cmp(&b.avg_fps).unwrap_or(std::cmp::Ordering::Equal)
        });
        s.push('\n');
        s.push_str(&format!("Overall avg fps: {:.1}\n", mean_fps));
        if let Some(w) = worst {
            s.push_str(&format!(
                "Worst combo: {} / {} @ {}k — {:.1} avg fps ({:.1} min)\n",
                w.view, w.signal, w.rate / 1000, w.avg_fps, w.min_fps,
            ));
        }
    }
    s
}

fn emit_report(state: &AppState, report: String, cancelled: bool) {
    // Echo the report into the debug log in ONE batched update (auto-scrolls;
    // Copy All includes it) rather than a reactive update per line.
    let ts = js_sys::Date::now();
    state.status.debug_log().update(|entries| {
        for line in report.lines() {
            entries.push((ts, "info".to_string(), line.to_string()));
        }
        if entries.len() > 500 {
            entries.drain(0..entries.len() - 500);
        }
    });
    let version = env!("CARGO_PKG_VERSION");
    download_text(&format!("oversample-bench-{}.md", version), &report);
    copy_to_clipboard(&report);
    state.show_info_toast(if cancelled {
        "Benchmark cancelled — partial report downloaded".to_string()
    } else {
        "Benchmark complete — report downloaded & copied".to_string()
    });
}

/// Download `content` as a file via a data-URL anchor (no Blob features needed).
/// The anchor is attached to the DOM before clicking and removed after, which
/// some browsers require for a programmatic download to fire.
fn download_text(filename: &str, content: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let encoded = String::from(js_sys::encode_uri_component(content));
    let href = format!("data:text/markdown;charset=utf-8,{}", encoded);
    let Ok(a) = doc.create_element("a") else { return };
    let _ = a.set_attribute("href", &href);
    let _ = a.set_attribute("download", filename);
    let _ = a.set_attribute("style", "display:none");
    if let Some(body) = doc.body() {
        let _ = body.append_child(&a);
    }
    if let Some(html) = a.dyn_ref::<web_sys::HtmlElement>() {
        html.click();
    }
    let _ = a.remove();
}

/// Best-effort copy to the clipboard (same pattern the debug-log "Copy All" uses).
fn copy_to_clipboard(text: &str) {
    let Some(window) = web_sys::window() else { return };
    if let Ok(nav) = js_sys::Reflect::get(&window, &JsValue::from_str("navigator")) {
        if let Ok(clip) = js_sys::Reflect::get(&nav, &JsValue::from_str("clipboard")) {
            if let Some(func) = js_sys::Reflect::get(&clip, &JsValue::from_str("writeText"))
                .ok()
                .and_then(|f| f.dyn_ref::<js_sys::Function>().cloned())
            {
                let _ = func.call1(&clip, &JsValue::from_str(text));
            }
        }
    }
}

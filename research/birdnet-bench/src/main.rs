//! Native timing of BirdNET V2.4 ONNX in tract (and onnxruntime via `ort` with
//! `--features ort`).
//!
//!   native tract MODEL.onnx windows.f32 [batch] [logits_out.f32]
//!   native ort   MODEL.onnx windows.f32 [threads] [logits_out.f32]

use std::time::Instant;

use birdnet_bench::{load_model, run_windows, CLASSES, WIN};

fn peak_rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

fn read_windows(path: &str) -> Vec<f32> {
    let b = std::fs::read(path).expect("read windows");
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn report(name: &str, load_ms: f64, per_batch: &[f64], batch: usize, windows: usize) {
    let mut t: Vec<f64> = per_batch.iter().skip(1).copied().collect();
    if t.is_empty() {
        t = per_batch.to_vec();
    }
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = t[t.len() / 2] / batch as f64;
    let total: f64 = per_batch.iter().sum();
    println!(
        "{name}: load {load_ms:.0} ms, per window median {med:.1} ms, {windows} windows in {:.2} s ({:.0}x real time), peak RSS {:.0} MB",
        total / 1000.0,
        windows as f64 * 3.0 / (total / 1000.0),
        peak_rss_mb()
    );
}

fn write_logits(path: Option<&String>, logits: &[f32]) {
    if let Some(p) = path {
        let bytes: Vec<u8> = logits.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(p, bytes).expect("write logits");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (engine, model, windows_path) = (&args[1], &args[2], &args[3]);
    let samples = read_windows(windows_path);
    let n = samples.len() / WIN;
    match engine.as_str() {
        "tract" => {
            let batch: usize = args.get(4).map_or(1, |s| s.parse().unwrap());
            let t0 = Instant::now();
            let bytes = std::fs::read(model).unwrap();
            let plan = load_model(&bytes, batch).expect("load model");
            drop(bytes);
            let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let mut times = vec![];
            let mut logits = Vec::with_capacity(n * CLASSES);
            let mut chunk = vec![0f32; batch * WIN];
            for start in (0..n).step_by(batch) {
                let k = batch.min(n - start);
                chunk.fill(0.0);
                chunk[..k * WIN].copy_from_slice(&samples[start * WIN..(start + k) * WIN]);
                let t = Instant::now();
                let out = run_windows(&plan, batch, &chunk).expect("run");
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                logits.extend_from_slice(&out[..k * CLASSES]);
            }
            report(&format!("tract batch={batch}"), load_ms, &times, batch, n);
            write_logits(args.get(5), &logits);
        }
        #[cfg(feature = "ort")]
        "ort" => {
            use ort::session::Session;
            use ort::value::TensorRef;
            let threads: usize = args.get(4).map_or(1, |s| s.parse().unwrap());
            let t0 = Instant::now();
            let env = ort::environment::init().build().unwrap();
            let mut session = Session::builder(&env)
                .unwrap()
                .with_intra_threads(threads)
                .unwrap()
                .commit_from_file(model)
                .expect("load model");
            let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let mut times = vec![];
            let mut logits = Vec::with_capacity(n * CLASSES);
            for w in samples.chunks_exact(WIN) {
                let t = Instant::now();
                let input = TensorRef::from_array_view(([1usize, WIN], w)).unwrap();
                let out = session.run(ort::inputs!["input" => input]).unwrap();
                let (_, data) = out[0].try_extract_tensor::<f32>().unwrap();
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                logits.extend_from_slice(data);
            }
            report(&format!("ort threads={threads}"), load_ms, &times, 1, n);
            write_logits(args.get(5), &logits);
        }
        other => panic!("unknown engine {other}"),
    }
}

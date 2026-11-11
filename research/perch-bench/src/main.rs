//! Native timing of Perch v2 ONNX in tract.
//!
//!   native MODEL.onnx windows.f32 [batch] [out_prefix]
//!
//! Writes <out_prefix>.emb.f32 (N x 1536) and, when the model has a label output,
//! <out_prefix>.label.f32 (N x classes).

use std::time::Instant;

use perch_bench::{load_model, run_windows, EMB, WIN};

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

fn write_f32(path: &str, v: &[f32]) {
    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(path, bytes).expect("write");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (model, windows_path) = (&args[1], &args[2]);
    let batch: usize = args.get(3).map_or(1, |s| s.parse().unwrap());
    let b = std::fs::read(windows_path).expect("read windows");
    let samples: Vec<f32> = b
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let n = samples.len() / WIN;

    let t0 = Instant::now();
    let bytes = std::fs::read(model).unwrap();
    let m = load_model(&bytes, batch).expect("load model");
    drop(bytes);
    let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let per = m.out_len();

    let mut times = vec![];
    let mut all = Vec::with_capacity(n * per);
    let mut chunk = vec![0f32; batch * WIN];
    for start in (0..n).step_by(batch) {
        let k = batch.min(n - start);
        chunk.fill(0.0);
        chunk[..k * WIN].copy_from_slice(&samples[start * WIN..(start + k) * WIN]);
        let t = Instant::now();
        let out = run_windows(&m, batch, &chunk).expect("run");
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        all.extend_from_slice(&out[..k * per]);
    }

    let mut t: Vec<f64> = times.iter().skip(1).copied().collect();
    if t.is_empty() {
        t = times.clone();
    }
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = t[t.len() / 2] / batch as f64;
    let total: f64 = times.iter().sum();
    println!(
        "tract native batch={batch} classes={}: load {load_ms:.0} ms, per 5 s window median {med:.1} ms, min {:.1} ms, \
         {n} windows in {:.2} s ({:.0}x real time), peak RSS {:.0} MB",
        m.classes,
        t[0] / batch as f64,
        total / 1000.0,
        n as f64 * 5.0 / (total / 1000.0),
        peak_rss_mb()
    );

    if let Some(prefix) = args.get(4) {
        let emb: Vec<f32> = all.chunks(per).flat_map(|r| r[..EMB].iter().copied()).collect();
        write_f32(&format!("{prefix}.emb.f32"), &emb);
        if m.classes > 0 {
            let lab: Vec<f32> = all.chunks(per).flat_map(|r| r[EMB..].iter().copied()).collect();
            write_f32(&format!("{prefix}.label.f32"), &lab);
        }
    }
}

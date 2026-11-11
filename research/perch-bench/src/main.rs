//! Native timing of Perch v2 ONNX in tract.
//!
//!   native MODEL.onnx windows.f32 [batch] [out_prefix] [HEAD.int8.bin]
//!
//! Writes <out_prefix>.emb.f32 (N x 1536) and, when the model has a label output or a
//! classifier file is given, <out_prefix>.label.f32 (N x classes). With a classifier
//! file (export_head.py), MODEL should be the embedding model; the label logits then come
//! from head.rs, timed separately.

use std::time::Instant;

use perch_bench::{head::Head, load_model_with, run_windows, EMB, WIN};

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

fn median(times: &[f64]) -> (f64, f64) {
    let mut t: Vec<f64> = times.iter().skip(1).copied().collect();
    if t.is_empty() {
        t = times.to_vec();
    }
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (t[t.len() / 2], t[0])
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

    let head = args.get(5).map(|p| {
        let t = Instant::now();
        let h =
            Head::from_bytes(std::fs::read(p).expect("read classifier")).expect("load classifier");
        println!(
            "classifier: {} classes, load {:.0} ms",
            h.classes,
            t.elapsed().as_secs_f64() * 1000.0
        );
        h
    });

    let t0 = Instant::now();
    let bytes = std::fs::read(model).unwrap();
    let m = load_model_with(&bytes, batch, head.is_some()).expect("load model");
    drop(bytes);
    let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let per = m.out_len();

    let mut times = vec![];
    let mut head_times = vec![];
    let (mut emb, mut lab) = (vec![], vec![]);
    let mut chunk = vec![0f32; batch * WIN];
    for start in (0..n).step_by(batch) {
        let k = batch.min(n - start);
        chunk.fill(0.0);
        chunk[..k * WIN].copy_from_slice(&samples[start * WIN..(start + k) * WIN]);
        let t = Instant::now();
        let out = run_windows(&m, batch, &chunk).expect("run");
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        for r in out.chunks(per).take(k) {
            emb.extend_from_slice(&r[..EMB]);
            match &head {
                Some(h) => {
                    let t = Instant::now();
                    lab.extend(h.scores(&r[EMB..EMB + m.spatial]));
                    head_times.push(t.elapsed().as_secs_f64() * 1000.0);
                }
                None => lab.extend_from_slice(&r[EMB + m.spatial..]),
            }
        }
    }

    let (med, min) = median(&times);
    let total: f64 = times.iter().sum::<f64>() + head_times.iter().sum::<f64>();
    let classes = head.as_ref().map_or(m.classes, |h| h.classes);
    println!(
        "tract native batch={batch} classes={classes}: load {load_ms:.0} ms, model per 5 s window median {:.1} ms, min {:.1} ms",
        med / batch as f64,
        min / batch as f64,
    );
    if !head_times.is_empty() {
        let (hmed, hmin) = median(&head_times);
        println!("classifier (head.rs) per window: median {hmed:.1} ms, min {hmin:.1} ms");
    }
    println!(
        "{n} windows in {:.2} s ({:.0}x real time), peak RSS {:.0} MB",
        total / 1000.0,
        n as f64 * 5.0 / (total / 1000.0),
        peak_rss_mb()
    );

    if let Some(prefix) = args.get(4) {
        write_f32(&format!("{prefix}.emb.f32"), &emb);
        if !lab.is_empty() {
            write_f32(&format!("{prefix}.label.f32"), &lab);
        }
    }
}

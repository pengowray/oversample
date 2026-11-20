//! Native timing of BirdNET+ V3.0 ONNX in tract.
//!
//!   native MODEL.onnx windows.f32 [out_prefix]
//!   native geo GEOMODEL.onnx LAT LON WEEK out.f32
//!   native plain MODEL.onnx windows.f32 WIN     (timing only, e.g. BirdNET V2.4)
//!
//! Writes <out_prefix>.emb.f32 (N x 1280) and <out_prefix>.score.f32 (N x classes;
//! empty for the backbone cut, which has no scores).

use std::time::Instant;

use birdnet_v3_bench::{load_model, run_window, EMB, WIN};

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
    if args[1] == "geo" {
        // native geo GEOMODEL.onnx LAT LON WEEK OUT.f32
        let f = |i: usize| args[i].parse::<f32>().unwrap();
        let t = Instant::now();
        let s = birdnet_v3_bench::geomodel_scores(&std::fs::read(&args[2]).unwrap(), f(3), f(4), f(5))
            .expect("geomodel");
        println!(
            "geomodel: {} scores, {} at 0.03 or more, load and run {:.0} ms",
            s.len(),
            s.iter().filter(|&&v| v >= 0.03).count(),
            t.elapsed().as_secs_f64() * 1000.0
        );
        write_f32(&args[6], &s);
        return;
    }
    if args[1] == "plain" {
        // native plain MODEL.onnx windows.f32 WIN   (timing only, e.g. BirdNET V2.4)
        let win: usize = args[4].parse().unwrap();
        let b = std::fs::read(&args[3]).expect("read windows");
        let s: Vec<f32> = b
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let t0 = Instant::now();
        let plan = birdnet_v3_bench::load_plain(&std::fs::read(&args[2]).unwrap(), win)
            .expect("load");
        let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut times: Vec<f64> = s
            .chunks_exact(win)
            .map(|w| {
                let t = Instant::now();
                birdnet_v3_bench::run_plain(&plan, w).expect("run");
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        let n = times.len();
        times.remove(0);
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "tract native plain: load {load_ms:.0} ms, {n} windows, median {:.1} ms, min {:.1} ms, peak RSS {:.0} MB",
            times[times.len() / 2],
            times[0],
            peak_rss_mb()
        );
        return;
    }
    let (model, windows_path) = (&args[1], &args[2]);
    let b = std::fs::read(windows_path).expect("read windows");
    let samples: Vec<f32> = b
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let n = samples.len() / WIN;

    let t0 = Instant::now();
    let bytes = std::fs::read(model).unwrap();
    let m = match load_model(&bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("load failed: {e:?}");
            std::process::exit(1);
        }
    };
    drop(bytes);
    let load_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let mut times = vec![];
    let (mut emb, mut sc) = (vec![], vec![]);
    for i in 0..n {
        let t = Instant::now();
        let out = match run_window(&m, &samples[i * WIN..(i + 1) * WIN]) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("run failed on window {i}: {e:?}");
                std::process::exit(1);
            }
        };
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        emb.extend_from_slice(&out[..EMB]);
        sc.extend_from_slice(&out[EMB..]);
    }
    let mut t: Vec<f64> = times.iter().skip(1).copied().collect();
    if t.is_empty() {
        t = times.clone();
    }
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let total: f64 = times.iter().sum();
    println!(
        "tract native, {} classes: load {load_ms:.0} ms, per 3 s window median {:.1} ms, min {:.1} ms, first {:.1} ms",
        m.classes,
        t[t.len() / 2],
        t[0],
        times[0]
    );
    println!(
        "{n} windows in {:.2} s ({:.0}x real time), peak RSS {:.0} MB",
        total / 1000.0,
        n as f64 * 3.0 / (total / 1000.0),
        peak_rss_mb()
    );
    if let Some(prefix) = args.get(3) {
        write_f32(&format!("{prefix}.emb.f32"), &emb);
        write_f32(&format!("{prefix}.score.f32"), &sc);
    }
}

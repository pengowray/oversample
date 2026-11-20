//! Native driver.
//!
//!   native detect MODEL.onnx AUDIO.wav OUT_PREFIX [--conf 0.15] [--gap 0.05] [--images]
//!       The whole pipeline on a 32 kHz WAV. Writes OUT_PREFIX.raw.tsv (per-window boxes:
//!       clip_start, t0, t1, f0, f1, conf), OUT_PREFIX.boxes.tsv (after window fusion:
//!       t0, t1, f0, f1, avg_conf, max_conf, merged), OUT_PREFIX.head.f32 (windows x 5 x
//!       1344) and, with --images, OUT_PREFIX.images.f32 (windows x 3 x 256 x 256). Prints
//!       the time per step.
//!   native net MODEL.onnx IMAGES.f32 OUT.head.f32
//!       The network alone on given images (BirdBox's own PNGs, from ref_images.py).
//!   native decode HEAD.f32 AUDIO.wav OUT_PREFIX [--conf 0.15] [--gap 0.05]
//!       NMS and fusion alone, on given heads (onnxruntime's, from ref_images.py).
//!   native fuse RAW.tsv AUDIO.wav OUT_PREFIX [--gap 0.05]
//!       Fusion alone, on given per-window boxes (BirdBox's own, from cli_tsv.py).

use birdbox_bench::{frontend, network, post};
use std::io::Write;
use std::time::Instant;

fn read_f32(path: &str) -> Vec<f32> {
    std::fs::read(path)
        .unwrap()
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn write_f32(path: &str, v: &[f32]) {
    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(path, bytes).unwrap();
}

fn read_wav(path: &str) -> Vec<f32> {
    let mut r = hound::WavReader::open(path).unwrap();
    let spec = r.spec();
    assert_eq!(spec.sample_rate, 32_000, "expects 32 kHz (prep.py)");
    assert_eq!(spec.channels, 1);
    match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => {
            let s = (1u64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().map(|v| v.unwrap() as f32 / s).collect()
        }
    }
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).map(|l| l.to_string()))
        .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok()))
        .map_or(0.0, |kb| kb / 1024.0)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn write_boxes(out: &str, raw: &[post::RawBox], merged: &[post::Merged]) {
    let mut f = std::fs::File::create(format!("{out}.raw.tsv")).unwrap();
    for r in raw {
        writeln!(f, "{:.6}\t{:.6}\t{:.6}\t{:.6}\t{:.6}\t{:.6}", r.clip_start, r.t0, r.t1, r.f0, r.f1, r.conf).unwrap();
    }
    let mut f = std::fs::File::create(format!("{out}.boxes.tsv")).unwrap();
    for m in merged {
        writeln!(
            f,
            "{:.6}\t{:.6}\t{:.6}\t{:.6}\t{:.6}\t{:.6}\t{:.6}",
            m.t0, m.t1, m.f0, m.f1, m.avg_conf, m.max_conf, m.merged
        )
        .unwrap();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str, default: f64| -> f64 {
        args.iter().position(|a| a == name).map_or(default, |i| args[i + 1].parse().unwrap())
    };
    match args.get(1).map(String::as_str) {
        Some("decode") | Some("fuse") => {
            let (input, wav, out) = (&args[2], &args[3], &args[4]);
            let conf = flag("--conf", post::DEFAULT_CONF as f64) as f32;
            let gap = flag("--gap", post::DEFAULT_PHRASE_GAP);
            let n = read_wav(wav).len();
            let padded = frontend::padded_len(n);
            let (content_end, file_end) = (n as f64 / 32000.0, padded as f64 / 32000.0);
            let raw = if args[1] == "decode" {
                let heads = read_f32(input);
                let mut raw = vec![];
                for (c, head) in heads.chunks_exact(network::HEAD).enumerate() {
                    raw.extend(post::decode(head, frontend::FileFrames::clip_start_s(c), conf));
                }
                post::limit_to_content(raw, content_end, file_end)
            } else {
                std::fs::read_to_string(input)
                    .unwrap()
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| {
                        let v: Vec<f64> = l.split('\t').map(|x| x.parse().unwrap()).collect();
                        post::RawBox { clip_start: v[0], t0: v[1], t1: v[2], f0: v[3], f1: v[4], conf: v[5] }
                    })
                    .collect()
            };
            let merged = post::fuse(&raw, file_end, gap);
            write_boxes(out, &raw, &merged);
            println!("{input}: {} window boxes, {} merged boxes", raw.len(), merged.len());
        }
        Some("net") => {
            let t = Instant::now();
            let net = network::load(&std::fs::read(&args[2]).unwrap()).unwrap();
            let load = ms(t);
            let images = read_f32(&args[3]);
            let per = 3 * network::IMAGE * network::IMAGE;
            let mut out = vec![];
            let mut times = vec![];
            for img in images.chunks_exact(per) {
                let t = Instant::now();
                out.extend(network::run(&net, img).unwrap());
                times.push(ms(t));
            }
            write_f32(&args[4], &out);
            let n = times.len();
            println!(
                "network: load {load:.0} ms ({:?}), {n} images, median {:.1} ms, peak RSS {:.0} MB",
                net.patched,
                median(&mut times),
                rss_mb()
            );
        }
        Some("detect") => {
            let (model, wav, out) = (&args[2], &args[3], &args[4]);
            let conf = flag("--conf", post::DEFAULT_CONF as f64) as f32;
            let gap = flag("--gap", post::DEFAULT_PHRASE_GAP);
            let t = Instant::now();
            let net = network::load(&std::fs::read(model).unwrap()).unwrap();
            let load = ms(t);
            let audio = read_wav(wav);
            let fe = frontend::Frontend::new();
            let t = Instant::now();
            let ff = frontend::file_frames(&fe, &audio);
            let spec = ms(t);
            let (mut t_floor, mut t_img, mut t_net, mut t_dec) = (vec![], vec![], vec![], vec![]);
            let mut heads = vec![];
            let mut images = vec![];
            let mut raw = vec![];
            for c in 0..ff.n_clips {
                let t = Instant::now();
                let floor = frontend::rolling_floor(&ff, c);
                t_floor.push(ms(t));
                let t = Instant::now();
                let img = frontend::clip_image(&ff, c, &floor);
                t_img.push(ms(t));
                let t = Instant::now();
                let head = network::run(&net, &img).unwrap();
                t_net.push(ms(t));
                let t = Instant::now();
                raw.extend(post::decode(&head, frontend::FileFrames::clip_start_s(c), conf));
                t_dec.push(ms(t));
                heads.extend(head);
                if args.iter().any(|a| a == "--images") {
                    images.extend(img);
                }
            }
            let t = Instant::now();
            let raw = post::limit_to_content(raw, ff.content_end_s(), ff.file_end_s());
            let merged = post::fuse(&raw, ff.file_end_s(), gap);
            let t_fuse = ms(t);
            write_f32(&format!("{out}.head.f32"), &heads);
            if !images.is_empty() {
                write_f32(&format!("{out}.images.f32"), &images);
            }
            write_boxes(out, &raw, &merged);
            let n = ff.n_clips;
            let total: f64 = spec + t_floor.iter().sum::<f64>() + t_img.iter().sum::<f64>()
                + t_net.iter().sum::<f64>() + t_dec.iter().sum::<f64>() + t_fuse;
            println!(
                "{wav}: {n} windows, {} window boxes, {} merged boxes; load {load:.0} ms; per window: \
                 spectrogram {:.2} ms (whole file {spec:.0} ms / {n}), floor median {:.2} ms, image median {:.2} ms, \
                 network median {:.1} ms, NMS median {:.3} ms; fusion {t_fuse:.2} ms for the file; \
                 total {:.0} ms for {:.1} s of audio ({:.0}x real time); peak RSS {:.0} MB",
                raw.len(),
                merged.len(),
                spec / n as f64,
                median(&mut t_floor),
                median(&mut t_img),
                median(&mut t_net),
                median(&mut t_dec),
                total,
                ff.content_end_s(),
                ff.content_end_s() * 1000.0 / total,
                rss_mb()
            );
        }
        _ => eprintln!("usage: see the top of src/main.rs"),
    }
}

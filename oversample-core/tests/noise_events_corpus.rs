//! Wind and handling noise found in the recording corpora, for checking the
//! detector by eye. Nothing is asserted: it prints, per recording, how many
//! Wind and Handling events were found and their total seconds, then the
//! recordings with the most of each.
//!
//! Reads bat-demo-sounds, no-bat-sounds and sound-event-sounds (see
//! `common`); a missing folder is skipped. Run in release:
//!
//! ```text
//! cargo test -p oversample-core --release --test noise_events_corpus -- --ignored --nocapture
//! ```
//!
//! Set `NOISE_EVENTS_LIST=1` to print every event as well.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use oversample_core::audio::loader::load_audio;
use oversample_core::dsp::noise_events::{NoiseEvent, NoiseKind, NoiseScanner};

mod common;

/// Audio files in `dir`, named `label/<file name>`, sorted.
fn audio_files(dir: &Path, label: &str) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        eprintln!("skipping {label}: {} not found", dir.display());
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|e| {
                let e = e.to_ascii_lowercase();
                ["wav", "flac", "ogg", "oga", "mp3"].iter().any(|x| e == *x)
            })
        })
        .map(|p| {
            (
                format!("{label}/{}", p.file_name().unwrap().to_string_lossy()),
                p,
            )
        })
        .collect();
    files.sort();
    files
}

struct FileResult {
    name: String,
    rate: u32,
    secs: f64,
    events: Vec<NoiseEvent>,
    scan_ms: f64,
}

impl FileResult {
    fn count(&self, kind: NoiseKind) -> (usize, f64) {
        self.events
            .iter()
            .filter(|e| e.kind == kind)
            .fold((0, 0.0), |(n, s), e| (n + 1, s + e.end_s - e.start_s))
    }
}

fn scan_file(name: &str, path: &Path) -> Result<FileResult, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let audio = load_audio(&bytes)?;
    let start = Instant::now();
    // Fed in pieces, as the app reads long files.
    let mut scanner = NoiseScanner::new(audio.sample_rate);
    for piece in audio.samples.chunks(65_536) {
        scanner.push(piece);
    }
    let events = scanner.finish();
    Ok(FileResult {
        name: name.to_string(),
        rate: audio.sample_rate,
        secs: audio.samples.len() as f64 / audio.sample_rate as f64,
        events,
        scan_ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}

#[test]
#[ignore = "reads the recording corpora; run in release with --ignored --nocapture"]
fn noise_events_corpus() {
    let mut files = audio_files(&common::bat_demo_sounds_dir(), "bat-demo");
    files.extend(audio_files(&common::no_bat_sounds_dir(), "no-bat"));
    files.extend(audio_files(
        &common::sound_event_sounds_dir(),
        "sound-event",
    ));
    if files.is_empty() {
        eprintln!("no corpus found; nothing to do");
        return;
    }
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8);
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((name, path)) = files.get(i) else {
                    break;
                };
                match scan_file(name, path) {
                    Ok(r) => results.lock().unwrap().push(r),
                    Err(e) => eprintln!("{name}: {e}"),
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.name.cmp(&b.name));

    let list = std::env::var("NOISE_EVENTS_LIST").is_ok_and(|v| v == "1");
    println!("wind\twind_s\thand\thand_s\tsecs\trate\tscan_ms\tfile");
    for r in &results {
        let (wn, ws) = r.count(NoiseKind::Wind);
        let (hn, hs) = r.count(NoiseKind::Handling);
        println!(
            "{wn}\t{ws:.1}\t{hn}\t{hs:.2}\t{:.1}\t{}\t{:.0}\t{}",
            r.secs, r.rate, r.scan_ms, r.name
        );
        if list {
            for e in &r.events {
                println!(
                    "    {:?}\t{:.3}-{:.3}\t{:.0}-{:.0} Hz\t{:.1} dB",
                    e.kind, e.start_s, e.end_s, e.lo_hz, e.hi_hz, e.level_db
                );
            }
        }
    }

    let audio_s: f64 = results.iter().map(|r| r.secs).sum();
    let scan_s: f64 = results.iter().map(|r| r.scan_ms).sum::<f64>() / 1000.0;
    let with = |kind| results.iter().filter(|r| r.count(kind).0 > 0).count();
    println!(
        "\n{} files, {:.0} s of audio, scanned in {:.1} s; {} with wind, {} with handling",
        results.len(),
        audio_s,
        scan_s,
        with(NoiseKind::Wind),
        with(NoiseKind::Handling)
    );
    for kind in [NoiseKind::Wind, NoiseKind::Handling] {
        let mut most: Vec<&FileResult> = results.iter().filter(|r| r.count(kind).0 > 0).collect();
        most.sort_by(|a, b| b.count(kind).1.total_cmp(&a.count(kind).1));
        println!("\nMost {kind:?} (events, seconds, share of file):");
        for r in most.iter().take(15) {
            let (n, s) = r.count(kind);
            println!("  {n}\t{s:.2}\t{:.0}%\t{}", 100.0 * s / r.secs, r.name);
        }
    }
}

//! Square waves and modulated carriers found in the recording corpora, for
//! checking the detector by eye. Nothing is asserted: it prints, per
//! recording with events, each event, then totals and scan speed.
//!
//! Reads bat-demo-sounds, no-bat-sounds and sound-event-sounds (see
//! `common`); a missing folder is skipped. Run in release:
//!
//! ```text
//! cargo test -p oversample-core --release --test digital_signals_corpus -- --ignored --nocapture
//! ```
//!
//! Set `DIGITAL_LIST_ALL=1` to list recordings without events too,
//! `DIGITAL_THREADS=1` for timings not slowed by other threads, and
//! `DIGITAL_TRACE=<part of a file name>` to print the scanner's trace for
//! matching recordings.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use oversample_core::audio::loader::load_audio;
use oversample_core::dsp::digital_signals::{DigitalEvent, DigitalKind, DigitalScanner};

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
    events: Vec<DigitalEvent>,
    scan_ms: f64,
    trace: Vec<String>,
}

fn scan_file(name: &str, path: &Path, trace: bool) -> Result<FileResult, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let audio = load_audio(&bytes)?;
    let start = Instant::now();
    // Fed in pieces, as the app reads long files.
    let mut scanner = DigitalScanner::new(audio.sample_rate);
    scanner.set_trace(trace);
    for piece in audio.samples.chunks(65_536) {
        scanner.push(piece);
    }
    let trace = scanner.take_trace();
    let events = scanner.finish();
    Ok(FileResult {
        name: name.to_string(),
        rate: audio.sample_rate,
        secs: audio.samples.len() as f64 / audio.sample_rate as f64,
        events,
        scan_ms: start.elapsed().as_secs_f64() * 1000.0,
        trace,
    })
}

fn describe(e: &DigitalEvent) -> String {
    let what = match e.kind {
        DigitalKind::SquareWave => format!("f0 {:.1} Hz", e.fundamental_hz.unwrap_or(0.0)),
        DigitalKind::Modulated => format!(
            "carrier {:.0} Hz, {} Bd, {}",
            e.carrier_hz.unwrap_or(0.0),
            e.symbol_rate_hz
                .map_or("?".to_string(), |r| format!("{r:.1}")),
            e.modulation.unwrap_or("?")
        ),
    };
    format!(
        "{:?}\t{:.2}-{:.2} s\t{:.0}-{:.0} Hz\t{what}",
        e.kind, e.start_s, e.end_s, e.lo_hz, e.hi_hz
    )
}

#[test]
#[ignore = "reads the recording corpora; run in release with --ignored --nocapture"]
fn digital_signals_corpus() {
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
    let trace_pat = std::env::var("DIGITAL_TRACE").ok();
    // DIGITAL_THREADS=1 for clean timings.
    let threads = std::env::var("DIGITAL_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(4, |n| n.get())
                .min(8)
        });
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((name, path)) = files.get(i) else {
                    break;
                };
                let trace = trace_pat.as_deref().is_some_and(|p| name.contains(p));
                match scan_file(name, path, trace) {
                    Ok(r) => results.lock().unwrap().push(r),
                    Err(e) => eprintln!("{name}: {e}"),
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by(|a, b| a.name.cmp(&b.name));

    let all = std::env::var("DIGITAL_LIST_ALL").is_ok_and(|v| v == "1");
    for r in &results {
        if r.events.is_empty() && !all && r.trace.is_empty() {
            continue;
        }
        println!(
            "{}\t{} Hz\t{:.1} s\t{} events",
            r.name,
            r.rate,
            r.secs,
            r.events.len()
        );
        for e in &r.events {
            println!("    {}", describe(e));
        }
        for t in &r.trace {
            println!("    | {t}");
        }
    }

    let audio_s: f64 = results.iter().map(|r| r.secs).sum();
    let scan_s: f64 = results.iter().map(|r| r.scan_ms).sum::<f64>() / 1000.0;
    let with = |kind| {
        results
            .iter()
            .filter(|r| r.events.iter().any(|e| e.kind == kind))
            .count()
    };
    println!(
        "\n{} files, {:.0} s of audio, scanned in {:.1} s ({:.0}x real time on one thread); {} with square waves, {} with modulated carriers",
        results.len(),
        audio_s,
        scan_s,
        audio_s / scan_s.max(1e-9),
        with(DigitalKind::SquareWave),
        with(DigitalKind::Modulated)
    );
    // Slowest recordings at least 5 s long.
    let mut slow: Vec<&FileResult> = results.iter().filter(|r| r.secs >= 5.0).collect();
    slow.sort_by(|a, b| (a.secs / a.scan_ms).total_cmp(&(b.secs / b.scan_ms)));
    println!("Slowest:");
    for r in slow.iter().take(6) {
        println!(
            "  {:.0}x real time\t{} Hz\t{}",
            1000.0 * r.secs / r.scan_ms,
            r.rate,
            r.name
        );
    }
    // Speed by sample rate.
    let mut rates: Vec<u32> = results.iter().map(|r| r.rate).collect();
    rates.sort_unstable();
    rates.dedup();
    for rate in rates {
        let (a, s) = results
            .iter()
            .filter(|r| r.rate == rate)
            .fold((0.0, 0.0), |(a, s), r| (a + r.secs, s + r.scan_ms / 1000.0));
        if a > 30.0 {
            println!("  {rate} Hz: {a:.0} s of audio, {:.0}x real time", a / s);
        }
    }
}

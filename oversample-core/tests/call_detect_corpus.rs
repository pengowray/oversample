//! Regression test for the call detector on the recording corpora.
//!
//! Scans every WAV in bat-demo-sounds, and in the no-bat set when it is
//! present (see `common`), and compares each recording's detections with a
//! committed snapshot, `tests/golden/call_detect_corpus.<os>.snapshot` (one
//! per OS, as float results can differ in the last bit between platforms).
//!
//! The snapshot records the `DETECTOR_ID` it was made with. Annotations the
//! detector makes are tagged `#auto=<DETECTOR_ID>`, so its results must not
//! change while the id stays the same; that is the failure this test exists
//! to catch. After an intended change, give the detector a new id, then
//! accept the new results with `UPDATE_CALL_DETECT_SNAPSHOT=1`.
//!
//! Run in release; a debug build takes many minutes:
//!
//! ```text
//! cargo test -p oversample-core --release --test call_detect_corpus -- --ignored
//! ```
//!
//! This only says which recordings changed. To see which calls changed and
//! why (CF and FM bats, false bat calls by animal group), use the harness in
//! `batchi-extras/call-detect-regress`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use oversample_core::audio::loader::load_audio;
use oversample_core::dsp::call_detect::{scan_calls, CallKind, CallScanParams, DETECTOR_ID};

mod common;

const UPDATE_ENV: &str = "UPDATE_CALL_DETECT_SNAPSHOT";
const MAX_LISTED: usize = 40;

fn snapshot_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/golden/call_detect_corpus.{}.snapshot",
        std::env::consts::OS
    ))
}

/// The WAV files in `dir`, named `label/<file name>`, sorted.
fn wav_files(dir: &Path, label: &str) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")))
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

/// One recording's result as a snapshot value: detections, bat calls, and a
/// hash of every detection's times, frequency range and kind.
fn scan_file(path: &Path) -> String {
    let scanned = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| load_audio(&bytes))
        .map(|audio| {
            scan_calls(
                &audio.samples,
                audio.sample_rate,
                &CallScanParams::default(),
            )
        });
    let result = match scanned {
        Ok(r) => r,
        Err(e) => return format!("ERR\t{e}"),
    };
    let mut h = common::Fnv::new();
    for c in &result.calls {
        h.bytes(&c.event.start_sample.to_le_bytes());
        h.bytes(&c.event.end_sample.to_le_bytes());
        h.bytes(&(c.event.min_freq_hz.round() as u32).to_le_bytes());
        h.bytes(&(c.event.max_freq_hz.round() as u32).to_le_bytes());
        h.bytes(format!("{:?};", c.kind).as_bytes());
    }
    let bats = result
        .calls
        .iter()
        .filter(|c| c.kind == CallKind::Bat)
        .count();
    format!("{}\t{}\t{:016x}", result.calls.len(), bats, h.0)
}

/// Scan every file, a few at a time.
fn scan_all(files: &[(String, PathBuf)]) -> BTreeMap<String, String> {
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8);
    let next = AtomicUsize::new(0);
    let out = Mutex::new(BTreeMap::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((name, path)) = files.get(i) else {
                    break;
                };
                let value = scan_file(path);
                out.lock().unwrap().insert(name.clone(), value);
            });
        }
    });
    out.into_inner().unwrap()
}

fn write_snapshot(path: &Path, results: &BTreeMap<String, String>) {
    let mut text = String::from(
        "# Call detector results per recording: detections, bat calls, hash.\n\
         # Made by oversample-core/tests/call_detect_corpus.rs; see there before editing.\n",
    );
    text.push_str(&format!("detector\t{DETECTOR_ID}\n"));
    for (name, value) in results {
        text.push_str(&format!("{name}\t{value}\n"));
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The snapshot's detector id and its per-recording values.
fn read_snapshot(text: &str) -> (String, BTreeMap<String, String>) {
    let mut id = String::new();
    let mut values = BTreeMap::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let (key, value) = line.split_once('\t').unwrap_or((line, ""));
        if key == "detector" {
            id = value.to_string();
        } else {
            values.insert(key.to_string(), value.to_string());
        }
    }
    (id, values)
}

/// "12 detections, 5 bat calls" from a snapshot value.
fn counts(value: &str) -> String {
    let mut parts = value.split('\t');
    match (parts.next(), parts.next()) {
        (Some("ERR"), msg) => format!("not read ({})", msg.unwrap_or("")),
        (Some(d), Some(b)) => format!("{d} detections, {b} bat calls"),
        _ => value.to_string(),
    }
}

#[test]
#[ignore = "scans every corpus recording; run in release: \
            cargo test -p oversample-core --release --test call_detect_corpus -- --ignored"]
fn call_detector_results_match_snapshot() {
    let mut files = wav_files(&common::bat_demo_sounds_dir(), "bat-demo-sounds");
    if files.is_empty() {
        eprintln!(
            "SKIP call_detector_results_match_snapshot: no recordings at {:?}",
            common::bat_demo_sounds_dir()
        );
        return;
    }
    files.extend(wav_files(&common::no_bat_sounds_dir(), "no-bat-sounds"));
    let results = scan_all(&files);

    let path = snapshot_path();
    let old = std::fs::read_to_string(&path).ok();
    if std::env::var_os(UPDATE_ENV).is_some() || old.is_none() {
        write_snapshot(&path, &results);
        eprintln!("Wrote {} recordings to {path:?}", results.len());
        return;
    }
    let (old_id, old_values) = read_snapshot(&old.unwrap());

    let mut changed = Vec::new();
    let mut not_in_snapshot = 0;
    for (name, now) in &results {
        match old_values.get(name) {
            Some(before) if before == now => {}
            Some(before) if counts(before) == counts(now) => changed.push(format!(
                "  {name}: {}, but different times, ranges or kinds",
                counts(now)
            )),
            Some(before) => {
                changed.push(format!("  {name}: {} -> {}", counts(before), counts(now)))
            }
            None => not_in_snapshot += 1,
        }
    }
    if not_in_snapshot > 0 {
        eprintln!(
            "{not_in_snapshot} recordings are not in the snapshot yet; {UPDATE_ENV}=1 adds them."
        );
    }
    if changed.is_empty() && old_id == DETECTOR_ID {
        return;
    }

    let problem = if changed.is_empty() {
        format!(
            "DETECTOR_ID is now {DETECTOR_ID}, but the snapshot was made with {old_id} and no results \
             changed. Run with {UPDATE_ENV}=1 to record the new id."
        )
    } else if old_id == DETECTOR_ID {
        format!(
            "Call detector results changed in {} recordings, but DETECTOR_ID is still {DETECTOR_ID}, so \
             #auto={DETECTOR_ID} tags would mean two different things. If the change is intended, give \
             the detector a new DETECTOR_ID (oversample-core/src/dsp/call_detect/mod.rs), then run with \
             {UPDATE_ENV}=1.",
            changed.len()
        )
    } else {
        format!(
            "DETECTOR_ID changed from {old_id} to {DETECTOR_ID}, and results changed in {} recordings. \
             If these changes are intended, run with {UPDATE_ENV}=1 to accept them.",
            changed.len()
        )
    };
    let more = changed.len().saturating_sub(MAX_LISTED);
    let mut list = changed
        .into_iter()
        .take(MAX_LISTED)
        .collect::<Vec<_>>()
        .join("\n");
    if more > 0 {
        list.push_str(&format!("\n  and {more} more"));
    }
    panic!("{problem}\n{list}");
}

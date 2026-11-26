//! Sanity run of the sound box estimator on real recordings: the xeno-canto
//! files in the no-bat set (insects, frogs, birds, mammals). Prints the box
//! for the first 3 s of each beside the species name from the file name, to
//! check by eye. It asserts nothing.
//!
//! ```text
//! cargo test -p oversample-core --release --test sound_box_corpus -- --ignored --nocapture
//! ```

use oversample_core::audio::loader::load_audio;
use oversample_core::dsp::sound_box::estimate;

mod common;

const BAND: (f32, f32) = (250.0, 15_000.0);
/// Seconds from the start of each file; `SOUND_BOX_SECONDS` overrides it
/// (the app measures stretches of up to 30 s).
fn seconds() -> f64 {
    std::env::var("SOUND_BOX_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0)
}

#[test]
#[ignore]
fn sound_box_corpus() {
    let dir = common::no_bat_sounds_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("SKIP: no recordings at {}", dir.display());
        return;
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy();
            name.starts_with("XC") && name.to_ascii_lowercase().ends_with(".wav")
        })
        .collect();
    files.sort();

    println!(
        "{:<9} {:<45} {:>6} {:>12} {:>16}",
        "file", "species", "kHz", "time s", "box kHz"
    );
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let mut parts = name.splitn(2, " - ");
        let id = parts.next().unwrap_or("").to_string();
        let species = parts.next().unwrap_or("").trim_start_matches("- ").trim();
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let audio = match load_audio(&bytes) {
            Ok(a) => a,
            Err(e) => {
                println!("{id:<9} {species:<45} decode error: {e}");
                continue;
            }
        };
        let sr = audio.sample_rate;
        let n = ((seconds() * sr as f64) as usize).min(audio.samples.len());
        let t = std::time::Instant::now();
        let b = estimate(&audio.samples[..n], sr, BAND);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let species: String = species.chars().take(45).collect();
        match b {
            Some(b) => println!(
                "{id:<9} {species:<45} {:>6.1} {:>5.2}-{:<5.2} {:>6.2}-{:<6.2}{}  {ms:.1} ms",
                sr as f64 / 1000.0,
                b.start_s,
                b.end_s,
                b.lo_hz / 1000.0,
                b.hi_hz / 1000.0,
                if b.hi_hz >= BAND.1 - 100.0 {
                    " clipped"
                } else {
                    ""
                },
            ),
            None => println!(
                "{id:<9} {species:<45} {:>6.1} {:>12} {:>16}  {ms:.1} ms",
                sr as f64 / 1000.0,
                "-",
                "none"
            ),
        }
    }
}

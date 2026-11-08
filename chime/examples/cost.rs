//! Times one chime decoder on this machine, stage by stage, in ms of CPU per second of
//! 192 kHz audio. The phone remote runs one decoder per input, so the X8 pays this once or
//! twice.
//!
//! RUSTFLAGS="-C no-vectorize-loops -C no-vectorize-slp" cargo run --release -p chime --features profile --example cost
//!
//! The X8's processor has no SIMD, so the figures without auto-vectorisation are closer to
//! its shares. The X8 is far slower and its memory is a different shape, so these are not X8
//! times.
//!
//! Each stage needs the ones before it, so the decoder runs once up to each stage and a
//! stage's cost is the difference from the run before. The differences are small next to
//! the front end, so they can come out a little below zero. "Detection" is the detector and
//! the timing search, which runs when the detector asks for it but starts no walk. During a
//! chime it searches again and again where the full decoder would walk, so "walk" there is
//! the walk less those searches.

use std::hint::black_box;
use std::time::Instant;

use chime::{Decoder, Event, Stage};

#[path = "../tests/support/mod.rs"]
mod support;

use support::channel;
use support::generator;

const RATE: u32 = 192_000;
const RUNS: usize = 15;
/// The app feeds the decoders in chunks of this many samples.
const CHUNK: usize = 1024;

/// A signal and the parts of it that each row times.
struct Signal {
    name: &'static str,
    samples: Vec<f32>,
    /// (start, end, is a chime).
    parts: Vec<(usize, usize, bool)>,
}

fn command_chime() -> Vec<f64> {
    let mut bytes = generator::time_payload(1_793_000_000, 600);
    bytes.extend_from_slice(b"BATXREC\x01");
    generator::render(RATE, &bytes, generator::tune_for_payload(bytes.len())).mix()
}

fn noise_only(name: &'static str, rms: f64, seed: u64) -> Signal {
    let n = 10 * RATE as usize;
    let samples = channel::to_f32(&channel::add_white_noise_rms(vec![0.0; n], rms, seed));
    Signal { name, samples, parts: vec![(0, n, false)] }
}

/// Eight command chimes with their melody at the page's level less `gain_db`, 0.5 s apart, in
/// white noise.
fn chimes_in_noise(name: &'static str, gain_db: f64, noise_rms: f64) -> Signal {
    let chime = channel::gain_db(command_chime(), gain_db);
    let gap = RATE as usize / 2;
    let mut stream = Vec::new();
    let mut parts = Vec::new();
    for _ in 0..8 {
        parts.push((stream.len(), stream.len() + gap, false));
        stream.extend(std::iter::repeat_n(0.0, gap));
        parts.push((stream.len(), stream.len() + chime.len(), true));
        stream.extend_from_slice(&chime);
    }
    parts.push((stream.len(), stream.len() + gap, false));
    stream.extend(std::iter::repeat_n(0.0, gap));
    let samples = channel::to_f32(&channel::add_white_noise_rms(stream, noise_rms, 5));
    Signal { name, samples, parts }
}

/// Seconds of CPU spent in the chime parts and in the others, in one run, and the number of
/// chimes read.
fn time(signal: &Signal, stage: Stage, per_sample: bool) -> (f64, f64, usize) {
    let mut decoder = Decoder::new(RATE).unwrap();
    decoder.set_last_stage(stage);
    let (mut in_chimes, mut between, mut chimes) = (0.0, 0.0, 0);
    for &(start, end, is_chime) in &signal.parts {
        let started = Instant::now();
        for chunk in signal.samples[start..end].chunks(CHUNK) {
            if per_sample {
                for &x in black_box(chunk) {
                    if let Some(Event::Chime(_)) = decoder.push(x) {
                        chimes += 1;
                    }
                }
            } else {
                decoder.push_block(black_box(chunk), |_, event| chimes += matches!(event, Event::Chime(_)) as usize);
            }
        }
        let t = started.elapsed().as_secs_f64();
        if is_chime {
            in_chimes += t;
        } else {
            between += t;
        }
    }
    (in_chimes, between, chimes)
}

/// For each stage, the smallest times of `RUNS` runs. The stages take turns within each run,
/// so that a change in the machine's clock speed falls on all of them.
fn time_stages(signal: &Signal, stages: &[Stage], per_sample: bool) -> Vec<(f64, f64, usize)> {
    let mut best = vec![(f64::MAX, f64::MAX, 0); stages.len()];
    for _ in 0..RUNS {
        for (best, &stage) in best.iter_mut().zip(stages) {
            let (in_chimes, between, chimes) = time(signal, stage, per_sample);
            *best = (best.0.min(in_chimes), best.1.min(between), chimes);
        }
    }
    best
}

fn main() {
    let signals = [
        noise_only("Noise at -70 dBFS RMS", 10f64.powf(-70.0 / 20.0), 1),
        noise_only("Noise at -40 dBFS RMS", 0.01, 2),
        chimes_in_noise("Chimes 30 dB down, noise -60 dBFS", -30.0, 0.001),
    ];
    let stages = [Stage::FrontEnd, Stage::Search, Stage::Walk];
    for (per_sample, heading) in [
        (false, "with push_block, in chunks of 1024 samples, as the app feeds it"),
        (true, "with push, one sample at a time"),
    ] {
        println!("One decoder {heading}, ms of CPU per second of 192 kHz audio, smallest of {RUNS} runs:");
        println!();
        println!("{:<42}{:>11}{:>11}{:>9}{:>9}{:>7}", "", "front end", "detection", "walk", "total", "read");
        for signal in &signals {
            print_rows(signal, &time_stages(signal, &stages, per_sample));
        }
        println!();
    }
}

fn print_rows(signal: &Signal, times: &[(f64, f64, usize)]) {
    let samples = |chime: bool| -> usize { signal.parts.iter().filter(|p| p.2 == chime).map(|p| p.1 - p.0).sum() };
    for chime in [false, true] {
        let n = samples(chime);
        if n == 0 {
            continue;
        }
        let seconds = n as f64 / RATE as f64;
        let at = |i: usize| if chime { times[i].0 } else { times[i].1 } * 1000.0 / seconds;
        let label = match (chime, signal.parts.len()) {
            (_, 1) => signal.name.to_string(),
            (false, _) => format!("{}: gaps", signal.name),
            (true, _) => format!("{}: chimes", signal.name),
        };
        let read = if chime { format!("{}/8", times[2].2) } else { String::new() };
        println!("{label:<42}{:>11.2}{:>11.2}{:>9.2}{:>9.2}{read:>7}", at(0), at(1) - at(0), at(2) - at(1), at(2));
    }
}

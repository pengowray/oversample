//! Writes test files of the phone remote: what the phone plays for a start command (the
//! page's tone, a 1 s wait, then the chime with its melody), in white noise, at several levels.
//! The files are 192 kHz stereo 32-bit float, with different noise on each channel, so that
//! `decode_wav` and bat-app's `remote_check` can be run on them.
//!
//! cargo run --release -p chime --example chime_wavs -- OUT_DIR
//!
//! Each file is named for its carrier's peak level and the noise's RMS level, in dBFS. The
//! page plays the carrier at -12 dBFS peak.

use std::f64::consts::TAU;
use std::path::PathBuf;

#[path = "../tests/support/mod.rs"]
mod support;

use support::channel;
use support::generator;

const RATE: u32 = 192_000;
const CARRIER_PEAK_DBFS: [f64; 5] = [-40.0, -60.0, -75.0, -85.0, -90.0];
const NOISE_RMS_DBFS: [f64; 2] = [-90.0, -70.0];
const PAGE_CARRIER_PEAK: f64 = 0.25;

/// The page's tone: the carrier in pairs of 10 ms and 5 ms symbols, with 0.5 ms sin^2 ramps and
/// the polarity turned over at each symbol.
fn page_tone(seconds: f64) -> Vec<f64> {
    let total = (seconds * RATE as f64) as usize;
    let ramp = RATE as usize / 2000;
    let mut out = Vec::with_capacity(total);
    let mut polarity = 1.0;
    for ms in [10, 5].into_iter().cycle() {
        if out.len() >= total {
            break;
        }
        let n = ms * RATE as usize / 1000;
        for i in 0..n {
            let edge = (i.min(n - i) as f64 / ramp as f64).min(1.0);
            let envelope = (edge * TAU / 4.0).sin().powi(2);
            let t = out.len() as f64 / RATE as f64;
            out.push(PAGE_CARRIER_PEAK * polarity * envelope * (TAU * 18_000.0 * t).sin());
        }
        polarity = -polarity;
    }
    out
}

fn press(gain: f64) -> Vec<f64> {
    let mut bytes = generator::time_payload(1_793_000_000, 600);
    bytes.extend_from_slice(&chime::format::RemoteCommand::Start.deployment_id());
    let chime = generator::render(RATE, &bytes, generator::tune_for_payload(bytes.len())).mix();
    let second = RATE as usize;
    let mut x = vec![0.0; second];
    x.extend(page_tone(0.5));
    x.extend(std::iter::repeat_n(0.0, second));
    x.extend(chime);
    x.extend(std::iter::repeat_n(0.0, second));
    x.iter().map(|v| v * gain).collect()
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: chime_wavs OUT_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    for carrier in CARRIER_PEAK_DBFS {
        let clean = press(10f64.powf(carrier / 20.0) / PAGE_CARRIER_PEAK);
        for noise in NOISE_RMS_DBFS {
            let rms = 10f64.powf(noise / 20.0);
            let left = channel::add_white_noise_rms(clean.clone(), rms, 1);
            let right = channel::add_white_noise_rms(clean.clone(), rms, 2);
            let path = dir.join(format!("press_carrier{carrier:.0}_noise{noise:.0}.wav"));
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            for (l, r) in left.iter().zip(&right) {
                writer.write_sample(*l as f32).unwrap();
                writer.write_sample(*r as f32).unwrap();
            }
            writer.finalize().unwrap();
            println!("{}", path.display());
        }
    }
}

//! Loudness of the hearing modes against each other at the same gain.
//!
//! Auto gain is worked out from the input peak (`-3 dBFS - peak`), so every
//! mode has to give an output whose level follows the input level. ZC used to
//! give clicks at about -40 dBFS whatever the input, which auto gain on a
//! quiet recording pushed above full scale.

use oversample_core::audio::loader::load_audio;
use oversample_core::dsp::heterodyne::heterodyne_mix;
use oversample_core::dsp::pitch_shift::{pitch_shift_realtime, PitchFactor};
use oversample_core::dsp::zc_divide::zc_divide;

mod common;

const CHUNK: usize = 96_000;

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}
fn stats(name: &str, s: &[f32], gain_db: f64) {
    let g = 10f64.powf(gain_db / 20.0);
    let peak = s.iter().fold(0.0f64, |m, &x| m.max((x as f64).abs())) * g;
    let rms = (s.iter().map(|&x| (x as f64 * g).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
    let clipped = s.iter().filter(|&&x| (x as f64 * g).abs() > 1.0).count();
    println!(
        "{name:8} peak {:7.1} dBFS  rms {:7.1} dBFS  samples over full scale {:.2}%",
        db(peak),
        db(rms),
        100.0 * clipped as f64 / s.len() as f64
    );
}

fn peak_db(s: &[f32]) -> f64 {
    db(s.iter().fold(0.0f64, |m, &x| m.max((x as f64).abs())))
}
fn rms_db(s: &[f32]) -> f64 {
    db((s.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt())
}

/// One second at 192 kHz: a 5 ms 40 kHz call every 100 ms, with noise 40 dB
/// below it throughout. The call peaks at `amp`.
fn calls_with_noise(amp: f32) -> Vec<f32> {
    let sr = 192_000usize;
    let mut state = 12_345u32;
    (0..sr)
        .map(|i| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * 0.01;
            let in_call = i % (sr / 10) < sr / 200;
            let call = if in_call {
                (2.0 * std::f64::consts::PI * 40_000.0 * i as f64 / sr as f64).sin() as f32
            } else {
                0.0
            };
            (call * 0.99 + noise) * amp
        })
        .collect()
}

/// At auto gain, ZC is no louder than pitch shift, not more than 15 dB
/// quieter, and does not pass full scale. Checked for a quiet and a loud
/// recording of the same sound: the old fixed click level passed at
/// neither (too loud when quiet, too quiet when loud).
#[test]
fn zc_is_about_as_loud_as_pitch_shift_at_auto_gain() {
    let sr = 192_000;
    for input_peak_db in [-45.0, -6.0] {
        let x = calls_with_noise(10f32.powf(input_peak_db / 20.0));
        let auto_gain = -3.0 - peak_db(&x);
        let ps = pitch_shift_realtime(&x, PitchFactor::down(10.0));
        let zc = zc_divide(&x, sr, 8, false, 20_000.0);
        let (ps_rms, zc_rms) = (rms_db(&ps) + auto_gain, rms_db(&zc) + auto_gain);
        let zc_peak = peak_db(&zc) + auto_gain;
        println!(
            "input peak {input_peak_db} dBFS, gain {auto_gain:+.1} dB: PS rms {ps_rms:.1}, ZC rms {zc_rms:.1}, ZC peak {zc_peak:.1} dBFS"
        );
        assert!(
            zc_peak <= -3.0,
            "input at {input_peak_db} dBFS: ZC peaks at {zc_peak:.1} dBFS, above the input's -3 dBFS"
        );
        assert!(
            (-15.0..=0.0).contains(&(zc_rms - ps_rms)),
            "input at {input_peak_db} dBFS: ZC rms {zc_rms:.1} dBFS against PS {ps_rms:.1} dBFS"
        );
    }
}

/// Prints the level of each mode at auto gain for the recording the fault
/// was reported on. Run with `--ignored --nocapture`; skips without the
/// bat-demo-sounds folder.
#[test]
#[ignore]
fn measure_xc1058561() {
    let path = common::bat_demo_sounds_dir()
        .join("XC1058561 - Long-fingered Myotis - Myotis capaccinii.wav");
    if !path.exists() {
        println!("SKIP: {} not found", path.display());
        return;
    }
    let audio = load_audio(&std::fs::read(path).unwrap()).unwrap();
    let sr = audio.sample_rate;
    let n = audio.samples.len().min(sr as usize * 30);
    let x = &audio.samples[..n];
    let peak = x.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    let auto = (-3.0 - db(peak as f64)).min(60.0);
    println!("rate {sr}, first-30s peak {:.1} dBFS, auto gain {auto:+.1} dB", db(peak as f64));
    let per_chunk = |f: &dyn Fn(&[f32]) -> Vec<f32>| -> Vec<f32> {
        x.chunks(CHUNK).flat_map(|c| f(c)).collect()
    };
    stats("1:1/TE", x, auto);
    stats("HET", &per_chunk(&|c| heterodyne_mix(c, sr, 45_000.0, 15_000.0)), auto);
    stats("PS", &per_chunk(&|c| pitch_shift_realtime(c, PitchFactor::down(10.0))), auto);
    let zc = per_chunk(&|c| zc_divide(c, sr, 8, false, 20_000.0));
    stats("ZC", &zc, auto);
    stats("ZC 0dB", &zc, 0.0);
    // fraction of time with clicks
    let active = zc.iter().filter(|v| v.abs() > 1e-4).count();
    println!("ZC non-silent {:.1}% of samples", 100.0 * active as f64 / zc.len() as f64);
}

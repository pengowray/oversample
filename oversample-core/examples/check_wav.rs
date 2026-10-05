//! Quick CLI to run the new LSB-autocorrelation and pipistrelle-signature
//! detectors against a WAV file. Useful for spot-checking detection on real
//! recordings without spinning up the full app.
//!
//! Run: `cargo run -p oversample-core --example check_wav -- <path/to/file.wav>`

use oversample_core::audio::loader::load_audio;
use oversample_core::dsp::{lsb_autocorr, pipistrelle};

fn main() {
    let path = std::env::args().nth(1).expect("usage: check_wav <path>");
    let bytes = std::fs::read(&path).expect("read file");
    let audio = load_audio(&bytes).expect("decode");
    let meta = &audio.metadata;
    println!(
        "File: {}\n  channels={} sample_rate={} bits={} float={}",
        path, audio.channels, audio.sample_rate, meta.bits_per_sample, meta.is_float
    );
    if let Some(wav) = &meta.wav {
        for note in &wav.notes {
            println!("  note: {note}");
        }
    }

    // `samples` is already mixed to mono.
    let mono: Vec<f32> = audio.samples.to_vec();
    println!(
        "  {} mono samples ({:.2} s)\n",
        mono.len(),
        mono.len() as f64 / audio.sample_rate as f64
    );

    struct Spec {
        sample_rate: u32,
        bits_per_sample: u16,
    }
    let spec = Spec {
        sample_rate: audio.sample_rate,
        bits_per_sample: meta.bits_per_sample,
    };
    let is_float = meta.is_float;

    println!("=== LSB autocorrelation ===");
    let lsb = lsb_autocorr::analyze_lsb_autocorr(&mono, spec.bits_per_sample, is_float);
    println!("  Verdict: {:?}", lsb.verdict);
    println!(
        "  Quietest window: idx={}  stdev={:.2}  nonzero_frac={:.3}",
        lsb.quietest_window_idx, lsb.quietest_window_stdev, lsb.quiet_lsb_nonzero_frac
    );
    println!(
        "  chi2={:.1}  lag1_acf={:+.4}  lag256_acf={:+.4}",
        lsb.quiet_lsb_chi2, lsb.quiet_lsb_lag1_acf, lsb.quiet_lsb_lag256_acf
    );
    println!("  GCD nonzero: {}", lsb.gcd_nonzero);
    println!("  {}\n", lsb.explanation);

    println!("=== Pipistrelle firmware signature ===");
    let pip = pipistrelle::detect(&mono, spec.sample_rate, spec.bits_per_sample, is_float);
    println!("  Verdict: {:?}", pip.verdict);
    println!("  Best dBcut: {:?}", pip.best_db_cut);
    println!(
        "  Best normalized residual: {:.4} ({:.2}%)",
        pip.best_normalized_residual,
        pip.best_normalized_residual * 100.0
    );
    println!("  Best in-range fraction: {:.3}", pip.best_in_range_frac);
    println!(
        "  Windows used: {}  samples analyzed: {}",
        pip.windows_used, pip.samples_analyzed
    );
    println!("  Per preset:");
    for s in &pip.per_preset {
        println!(
            "    dBcut={:>2}  residual={:.4}  in_range={:.3}",
            s.db_cut, s.normalized_residual, s.in_range_frac
        );
    }
    println!("  {}", pip.explanation);
}

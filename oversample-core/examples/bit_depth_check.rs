//! Spot-check `bit_depth_certainty::compose` on a single WAV file.
//! Usage: `cargo run -p oversample-core --release --example bit_depth_check -- <path>`

use oversample_core::audio::loader;
use oversample_core::audio::source::ChannelView;
use oversample_core::bit_depth_certainty::{self, CertaintyLevel};
use oversample_core::dsp::{audiomoth, bit_analysis, float_grid, lsb_autocorr, pipistrelle};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: bit_depth_check <path>");
    let bytes = std::fs::read(&path).expect("read file");
    let audio = loader::load_audio(&bytes).expect("decode");

    let bits = audio.metadata.bits_per_sample;
    let is_float = audio.metadata.is_float;
    let dur = audio.samples.len() as f64 / audio.sample_rate as f64;

    let bit = bit_analysis::analyze_bits(&audio.samples, bits, is_float, dur);
    let lsb = lsb_autocorr::analyze_lsb_autocorr(&audio.samples, bits, is_float);
    let pip = pipistrelle::detect(&audio.samples, audio.sample_rate, bits, is_float);
    let lsb_zero_padded = matches!(lsb.verdict, lsb_autocorr::LsbVerdict::ZeroPaddedNBit { .. });
    let am = audiomoth::detect(
        &audio.samples,
        audio.sample_rate,
        bits,
        is_float,
        lsb_zero_padded,
    );

    let total = audio.source.total_samples() as usize;
    let grid: Vec<_> = (0..audio.channels)
        .map(|channel| {
            let view = if audio.channels == 1 {
                ChannelView::MonoMix
            } else {
                ChannelView::Channel(channel)
            };
            let samples = audio.source.read_region(view, 0, total);
            float_grid::detect(&samples, bits, is_float)
        })
        .collect();

    let bdc = bit_depth_certainty::compose(
        bits,
        is_float,
        audio.metadata.format,
        &bit,
        &lsb,
        Some(&pip),
        Some(&am),
        &grid,
    );

    println!("File: {}", path);
    println!(
        "  Format: {}-bit{}, {} Hz, {} samples",
        bits,
        if is_float { " float" } else { "" },
        audio.sample_rate,
        audio.samples.len()
    );
    println!();
    println!("HEADLINE: {}", bdc.headline);
    println!("({})", bdc.certainty.label());
    println!();
    for fact in &bdc.facts {
        let prefix = match fact.certainty {
            CertaintyLevel::Certain => "✓",
            CertaintyLevel::HighConfidence => "●",
            CertaintyLevel::Suggestive => "○",
        };
        println!("  {} {}", prefix, fact.statement);
    }
}

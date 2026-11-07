//! Run the float grid detector on every channel of each WAV file given.
//! Usage: `cargo run -p oversample-core --release --example float_grid_check -- <paths>`

use oversample_core::audio::loader;
use oversample_core::audio::source::ChannelView;
use oversample_core::dsp::float_grid::{self, FloatGridResult};

fn main() {
    println!("file\tchannel\tverdict\tfine\tfine_fit\tcoarse\tcoarse_fit\tratio_db\tceiling\tceiling_samples\tpath_bits\tneg_zeros\tsamples");
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("{path}: could not read");
            continue;
        };
        let Ok(audio) = loader::load_audio(&bytes) else {
            eprintln!("{path}: could not decode");
            continue;
        };
        let meta = &audio.metadata;
        let total = audio.source.total_samples() as usize;
        for channel in 0..audio.channels {
            let view = if audio.channels == 1 {
                ChannelView::MonoMix
            } else {
                ChannelView::Channel(channel)
            };
            let samples = audio.source.read_region(view, 0, total);
            let r = float_grid::detect(&samples, meta.bits_per_sample, meta.is_float);
            print_row(&path, channel, &r);
        }
    }
}

fn print_row(path: &str, channel: u32, r: &FloatGridResult) {
    let step = |s: Option<float_grid::GridStep>| {
        s.map_or("-\t-".to_string(), |s| {
            format!("{:.6e}\t{:.3}", s.step, s.fit)
        })
    };
    let (ceiling, ceiling_samples) = r.flat_ceiling.map_or(("-".to_string(), 0), |c| {
        (format!("{:.5}", c.value), c.samples)
    });
    println!(
        "{path}\t{channel}\t{:?}\t{}\t{}\t{}\t{ceiling}\t{ceiling_samples}\t{}\t{}\t{}",
        r.verdict,
        step(r.fine),
        step(r.coarse),
        r.step_ratio_db()
            .map_or("-".to_string(), |d| format!("{d:.3}")),
        r.path_bits.map_or("-".to_string(), |b| b.to_string()),
        r.negative_zeros,
        r.samples_checked,
    );
}

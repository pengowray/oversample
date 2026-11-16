//! How long each gain chain takes to denoise a recording natively. Ignored by
//! default; run in release:
//!
//! ```text
//! DENOISE_TIMING_INPUT=bench.f32 DENOISE_TIMING_MODEL=both_hp_s24.onnx \
//!   cargo test -p birdnet-worker --release --test denoise_timing -- --ignored --nocapture
//! ```
//!
//! `DENOISE_TIMING_INPUT` is raw little-endian float32 mono samples (60 s at
//! 384 kHz is 23,040,000 of them). The second test times the chains with a
//! stand-in for the network whose gains are spread from 0 to 1, so that
//! almost no cell is low enough to skip the median: the chains' worst case.

use birdnet_worker::denoise::{DenoiseGain, Denoiser, MaskModel, TractMask};
use std::time::Instant;

const CHAINS: [DenoiseGain; 3] = [
    DenoiseGain::Plain,
    DenoiseGain::Smoothed,
    DenoiseGain::Heavy,
];

fn input() -> Option<Vec<f32>> {
    let path = std::env::var_os("DENOISE_TIMING_INPUT")?;
    Some(
        std::fs::read(path)
            .unwrap()
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
    )
}

/// Seconds to denoise `x` with `gain`.
fn time(x: &[f32], gain: DenoiseGain, model: &mut dyn MaskModel) -> f64 {
    let start = Instant::now();
    let mut d = Denoiser::new(x).unwrap().with_gain(gain);
    while d.step(model).unwrap() {}
    std::hint::black_box(d.finish());
    start.elapsed().as_secs_f64()
}

#[test]
#[ignore]
fn each_chain_with_the_network() {
    let (Some(x), Some(model)) = (input(), std::env::var_os("DENOISE_TIMING_MODEL")) else {
        eprintln!("SKIP: set DENOISE_TIMING_INPUT and DENOISE_TIMING_MODEL");
        return;
    };
    let mut model = TractMask::load(std::fs::read(model).unwrap()).unwrap();
    for gain in CHAINS {
        let seconds = time(&x, gain, &mut model);
        eprintln!("{gain:?}: {seconds:.2} s for {} samples", x.len());
    }
}

/// Gains spread from 0 to 1 whatever the input.
struct Spread(u64);
impl MaskModel for Spread {
    fn logits(&mut self, feat: &[f32], _frames: usize) -> Result<Vec<f32>, String> {
        Ok(feat
            .iter()
            .map(|_| {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                (self.0 >> 40) as f32 / (1u64 << 24) as f32 * 6.0
            })
            .collect())
    }
}

#[test]
#[ignore]
fn each_chain_at_its_slowest() {
    let Some(x) = input() else {
        eprintln!("SKIP: set DENOISE_TIMING_INPUT");
        return;
    };
    for gain in CHAINS {
        let seconds = time(&x, gain, &mut Spread(0x2545_f491_4f6c_dd1d));
        eprintln!(
            "{gain:?} with no network: {seconds:.2} s for {} samples",
            x.len()
        );
    }
}

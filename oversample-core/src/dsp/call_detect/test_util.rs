//! Test signals, as in `bat-dsp`'s `test_util`, so seeds give the same noise.

pub fn sine(fs: f32, freq: f32, amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (std::f64::consts::TAU * freq as f64 * i as f64 / fs as f64).sin() as f32)
        .collect()
}

/// Linear chirp from `f0` to `f1` over `n` samples.
pub fn chirp(fs: f32, f0: f32, f1: f32, amp: f32, n: usize) -> Vec<f32> {
    let dur = n as f64 / fs as f64;
    let k = (f1 as f64 - f0 as f64) / dur;
    (0..n)
        .map(|i| {
            let t = i as f64 / fs as f64;
            amp * (std::f64::consts::TAU * (f0 as f64 * t + 0.5 * k * t * t)).sin() as f32
        })
        .collect()
}

/// Deterministic white noise in [-amp, amp] (xorshift), so tests repeat.
pub fn noise(amp: f32, n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed.max(1);
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            amp * ((s as f32 / u32::MAX as f32) * 2.0 - 1.0)
        })
        .collect()
}

/// Add `s` into `x` from sample `at`.
pub fn add(x: &mut [f32], at: usize, s: &[f32]) {
    for (a, b) in x[at..].iter_mut().zip(s) {
        *a += b;
    }
}

/// Butterworth filters, for shaping test noise.
pub use crate::dsp::filters::Butterworth;

/// A call from `f0` to `f1` Hz with a Hann envelope, `n` samples long, as a
/// bat's calls fade in and out. An abrupt start splatters into the audible
/// band, which marks the call as an audible sound.
pub fn faded_chirp(fs: f32, f0: f32, f1: f32, amp: f32, n: usize) -> Vec<f32> {
    let mut x = chirp(fs, f0, f1, amp, n);
    for (i, v) in x.iter_mut().enumerate() {
        let w = (std::f64::consts::PI * i as f64 / n as f64).sin();
        *v *= (w * w) as f32;
    }
    x
}

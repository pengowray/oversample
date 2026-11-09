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

/// One RBJ biquad section in transposed direct form II.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn new(sample_rate: f32, freq_hz: f32, q: f32, highpass: bool) -> Self {
        let w = std::f32::consts::TAU * freq_hz / sample_rate;
        let (cos_w, alpha) = (w.cos(), w.sin() / (2.0 * q));
        let (b0, b1, b2) = if highpass {
            let b1 = -(1.0 + cos_w);
            (-b1 / 2.0, b1, -b1 / 2.0)
        } else {
            let b1 = 1.0 - cos_w;
            (b1 / 2.0, b1, b1 / 2.0)
        };
        let a0 = 1.0 + alpha;
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos_w / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// 4th-order Butterworth filter, for shaping test noise.
pub struct Butterworth4([Biquad; 2]);

impl Butterworth4 {
    const Q: [f32; 2] = [0.541_196_1, 1.306_563];

    pub fn lowpass(sample_rate: f32, cutoff_hz: f32) -> Self {
        Self(Self::Q.map(|q| Biquad::new(sample_rate, cutoff_hz, q, false)))
    }

    pub fn highpass(sample_rate: f32, cutoff_hz: f32) -> Self {
        Self(Self::Q.map(|q| Biquad::new(sample_rate, cutoff_hz, q, true)))
    }

    pub fn process_in_place(&mut self, buf: &mut [f32]) {
        for x in buf {
            let y = self.0[0].process(*x);
            *x = self.0[1].process(y);
        }
    }
}

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

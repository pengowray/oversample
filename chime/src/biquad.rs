#![allow(dead_code)] // A copy of bat-dsp's filters, of which the decoder uses Butterworth4 only.

//! Second-order IIR sections (biquads) and Butterworth cascades.
//!
//! Coefficients follow the RBJ Audio EQ Cookbook. Sections run in transposed
//! direct form II, which behaves well in f32.

use core::f32::consts::PI;

/// Q of the two sections of a 4th-order Butterworth filter.
const BUTTERWORTH4_Q: [f32; 2] = [0.541_196_1, 1.306_563];

#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    /// Pass-through section.
    pub const fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn lowpass(sample_rate: f32, cutoff_hz: f32, q: f32) -> Self {
        let (cos_w, alpha) = Self::prewarp(sample_rate, cutoff_hz, q);
        let b1 = 1.0 - cos_w;
        Self::normalised(
            b1 / 2.0,
            b1,
            b1 / 2.0,
            1.0 + alpha,
            -2.0 * cos_w,
            1.0 - alpha,
        )
    }

    pub fn highpass(sample_rate: f32, cutoff_hz: f32, q: f32) -> Self {
        let (cos_w, alpha) = Self::prewarp(sample_rate, cutoff_hz, q);
        let b1 = -(1.0 + cos_w);
        Self::normalised(
            -b1 / 2.0,
            b1,
            -b1 / 2.0,
            1.0 + alpha,
            -2.0 * cos_w,
            1.0 - alpha,
        )
    }

    /// Band-stop around `centre_hz`, for removing a steady tone.
    pub fn notch(sample_rate: f32, centre_hz: f32, q: f32) -> Self {
        let (cos_w, alpha) = Self::prewarp(sample_rate, centre_hz, q);
        Self::normalised(
            1.0,
            -2.0 * cos_w,
            1.0,
            1.0 + alpha,
            -2.0 * cos_w,
            1.0 - alpha,
        )
    }

    fn prewarp(sample_rate: f32, freq_hz: f32, q: f32) -> (f32, f32) {
        let f = freq_hz.clamp(1.0, 0.499 * sample_rate);
        let w = 2.0 * PI * f / sample_rate;
        (libm::cosf(w), libm::sinf(w) / (2.0 * q))
    }

    fn normalised(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Take another section's coefficients and keep this one's state, so a
    /// filter can be retuned while audio runs through it.
    pub fn set_coefficients(&mut self, other: &Biquad) {
        self.b0 = other.b0;
        self.b1 = other.b1;
        self.b2 = other.b2;
        self.a1 = other.a1;
        self.a2 = other.a2;
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    pub fn process_sample(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(input.len(), output.len());
        for (o, &x) in output.iter_mut().zip(input) {
            *o = self.process_sample(x);
        }
    }

    pub fn process_in_place(&mut self, buf: &mut [f32]) {
        for x in buf {
            *x = self.process_sample(*x);
        }
    }
}

/// 4th-order Butterworth filter: two biquads in series.
#[derive(Clone, Copy, Debug)]
pub struct Butterworth4 {
    sections: [Biquad; 2],
}

impl Butterworth4 {
    /// Pass-through filter, for building a processor in a `static` before its
    /// sample rate is known.
    pub const fn identity() -> Self {
        Self {
            sections: [Biquad::identity(); 2],
        }
    }

    pub fn lowpass(sample_rate: f32, cutoff_hz: f32) -> Self {
        Self {
            sections: BUTTERWORTH4_Q.map(|q| Biquad::lowpass(sample_rate, cutoff_hz, q)),
        }
    }

    pub fn highpass(sample_rate: f32, cutoff_hz: f32) -> Self {
        Self {
            sections: BUTTERWORTH4_Q.map(|q| Biquad::highpass(sample_rate, cutoff_hz, q)),
        }
    }

    /// Retune to another filter's coefficients, keeping the state.
    pub fn set_coefficients(&mut self, other: &Butterworth4) {
        for (s, o) in self.sections.iter_mut().zip(&other.sections) {
            s.set_coefficients(o);
        }
    }

    pub fn reset(&mut self) {
        for s in &mut self.sections {
            s.reset();
        }
    }

    #[inline]
    pub fn process_sample(&mut self, x: f32) -> f32 {
        let y = self.sections[0].process_sample(x);
        self.sections[1].process_sample(y)
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(input.len(), output.len());
        for (o, &x) in output.iter_mut().zip(input) {
            *o = self.process_sample(x);
        }
    }

    pub fn process_in_place(&mut self, buf: &mut [f32]) {
        for x in buf {
            *x = self.process_sample(*x);
        }
    }
}

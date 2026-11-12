//! A complex number of two `f32`s, for baseband samples.

use core::ops::{Add, AddAssign, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Cplx {
    pub(super) re: f32,
    pub(super) im: f32,
}

impl Cplx {
    pub(super) const ZERO: Cplx = Cplx { re: 0.0, im: 0.0 };
    pub(super) const ONE: Cplx = Cplx { re: 1.0, im: 0.0 };

    pub(super) fn power(self) -> f32 {
        self.re * self.re + self.im * self.im
    }

    /// The real part of self * conj(other).
    pub(super) fn dot(self, other: Cplx) -> f32 {
        self.re * other.re + self.im * other.im
    }

    /// self * conj(other).
    pub(super) fn mul_conj(self, other: Cplx) -> Cplx {
        Cplx {
            re: self.re * other.re + self.im * other.im,
            im: self.im * other.re - self.re * other.im,
        }
    }

    /// self * other.
    pub(super) fn mul(self, other: Cplx) -> Cplx {
        Cplx {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    /// This number scaled down, if necessary, so that its power is at most `limit`.
    pub(super) fn limited(self, limit: f32) -> Cplx {
        let power = self.power();
        if power > limit {
            self * libm::sqrtf(limit / power)
        } else {
            self
        }
    }
}

impl Add for Cplx {
    type Output = Cplx;
    fn add(self, other: Cplx) -> Cplx {
        Cplx {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }
}

impl AddAssign for Cplx {
    fn add_assign(&mut self, other: Cplx) {
        self.re += other.re;
        self.im += other.im;
    }
}

impl Sub for Cplx {
    type Output = Cplx;
    fn sub(self, other: Cplx) -> Cplx {
        Cplx {
            re: self.re - other.re,
            im: self.im - other.im,
        }
    }
}

impl Neg for Cplx {
    type Output = Cplx;
    fn neg(self) -> Cplx {
        Cplx {
            re: -self.re,
            im: -self.im,
        }
    }
}

impl Mul<f32> for Cplx {
    type Output = Cplx;
    fn mul(self, gain: f32) -> Cplx {
        Cplx {
            re: self.re * gain,
            im: self.im * gain,
        }
    }
}

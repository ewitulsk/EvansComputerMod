//! Single-precision complex numbers (`C32`), the sample type for IQ streams.

use core::iter::Sum;
use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// A complex number with `f32` parts. Layout-compatible with interleaved `cf32` IQ.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct C32 {
    pub re: f32,
    pub im: f32,
}

impl C32 {
    pub const ZERO: C32 = C32 { re: 0.0, im: 0.0 };
    pub const ONE: C32 = C32 { re: 1.0, im: 0.0 };
    pub const I: C32 = C32 { re: 0.0, im: 1.0 };

    #[inline]
    pub const fn new(re: f32, im: f32) -> C32 {
        C32 { re, im }
    }

    /// `r * e^{j theta}`.
    #[inline]
    pub fn from_polar(r: f32, theta: f32) -> C32 {
        let (s, c) = theta.sin_cos();
        C32::new(r * c, r * s)
    }

    /// `e^{j theta}` (unit phasor).
    #[inline]
    pub fn expj(theta: f32) -> C32 {
        C32::from_polar(1.0, theta)
    }

    /// `e^{j theta}` computed in double precision (for long phase accumulators).
    #[inline]
    pub fn expj64(theta: f64) -> C32 {
        let (s, c) = theta.sin_cos();
        C32::new(c as f32, s as f32)
    }

    #[inline]
    pub fn conj(self) -> C32 {
        C32::new(self.re, -self.im)
    }

    /// `|z|^2`.
    #[inline]
    pub fn norm_sqr(self) -> f32 {
        self.re * self.re + self.im * self.im
    }

    /// `|z|`.
    #[inline]
    pub fn abs(self) -> f32 {
        self.re.hypot(self.im)
    }

    /// Phase angle in `(-pi, pi]`.
    #[inline]
    pub fn arg(self) -> f32 {
        self.im.atan2(self.re)
    }

    #[inline]
    pub fn scale(self, k: f32) -> C32 {
        C32::new(self.re * k, self.im * k)
    }

    /// Multiply by `conj(other)`.
    #[inline]
    pub fn mul_conj(self, other: C32) -> C32 {
        C32::new(
            self.re * other.re + self.im * other.im,
            self.im * other.re - self.re * other.im,
        )
    }

    /// Unit-magnitude version (or zero when the input is zero).
    #[inline]
    pub fn normalize(self) -> C32 {
        let m = self.abs();
        if m > 0.0 {
            self.scale(1.0 / m)
        } else {
            C32::ZERO
        }
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
}

impl From<f32> for C32 {
    #[inline]
    fn from(re: f32) -> C32 {
        C32::new(re, 0.0)
    }
}

impl From<(f32, f32)> for C32 {
    #[inline]
    fn from(v: (f32, f32)) -> C32 {
        C32::new(v.0, v.1)
    }
}

impl Add for C32 {
    type Output = C32;
    #[inline]
    fn add(self, o: C32) -> C32 {
        C32::new(self.re + o.re, self.im + o.im)
    }
}
impl Sub for C32 {
    type Output = C32;
    #[inline]
    fn sub(self, o: C32) -> C32 {
        C32::new(self.re - o.re, self.im - o.im)
    }
}
impl Mul for C32 {
    type Output = C32;
    #[inline]
    fn mul(self, o: C32) -> C32 {
        C32::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}
impl Div for C32 {
    type Output = C32;
    #[inline]
    fn div(self, o: C32) -> C32 {
        let d = o.norm_sqr();
        let n = self.mul_conj(o);
        C32::new(n.re / d, n.im / d)
    }
}
impl Mul<f32> for C32 {
    type Output = C32;
    #[inline]
    fn mul(self, k: f32) -> C32 {
        self.scale(k)
    }
}
impl Mul<C32> for f32 {
    type Output = C32;
    #[inline]
    fn mul(self, z: C32) -> C32 {
        z.scale(self)
    }
}
impl Div<f32> for C32 {
    type Output = C32;
    #[inline]
    fn div(self, k: f32) -> C32 {
        C32::new(self.re / k, self.im / k)
    }
}
impl Neg for C32 {
    type Output = C32;
    #[inline]
    fn neg(self) -> C32 {
        C32::new(-self.re, -self.im)
    }
}
impl AddAssign for C32 {
    #[inline]
    fn add_assign(&mut self, o: C32) {
        self.re += o.re;
        self.im += o.im;
    }
}
impl SubAssign for C32 {
    #[inline]
    fn sub_assign(&mut self, o: C32) {
        self.re -= o.re;
        self.im -= o.im;
    }
}
impl MulAssign for C32 {
    #[inline]
    fn mul_assign(&mut self, o: C32) {
        *self = *self * o;
    }
}
impl MulAssign<f32> for C32 {
    #[inline]
    fn mul_assign(&mut self, k: f32) {
        self.re *= k;
        self.im *= k;
    }
}
impl DivAssign<f32> for C32 {
    #[inline]
    fn div_assign(&mut self, k: f32) {
        self.re /= k;
        self.im /= k;
    }
}
impl Sum for C32 {
    fn sum<I: Iterator<Item = C32>>(iter: I) -> C32 {
        iter.fold(C32::ZERO, |a, b| a + b)
    }
}
impl<'a> Sum<&'a C32> for C32 {
    fn sum<I: Iterator<Item = &'a C32>>(iter: I) -> C32 {
        iter.fold(C32::ZERO, |a, b| a + *b)
    }
}

/// Mean power `E|x|^2` of a block (0 for an empty block).
pub fn mean_power(x: &[C32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|z| z.norm_sqr() as f64).sum::<f64>() / x.len() as f64) as f32
}

/// Mean power `E[x^2]` of a real block.
pub fn mean_power_real(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / x.len() as f64) as f32
}

/// Linear power ratio to decibels (floored at -300 dB).
#[inline]
pub fn to_db(p: f32) -> f32 {
    10.0 * p.max(1e-30).log10()
}

/// Decibels to a linear power ratio.
#[inline]
pub fn from_db(db: f32) -> f32 {
    10f32.powf(db / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let a = C32::new(1.0, 2.0);
        let b = C32::new(3.0, -1.0);
        assert_eq!(a + b, C32::new(4.0, 1.0));
        assert_eq!(a * b, C32::new(5.0, 5.0));
        let q = (a * b) / b;
        assert!((q - a).abs() < 1e-6);
        assert_eq!(a.mul_conj(b), a * b.conj());
        assert!((C32::expj(1.0).abs() - 1.0).abs() < 1e-6);
    }
}

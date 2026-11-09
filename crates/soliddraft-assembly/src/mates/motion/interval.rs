//! Outward-rounded evaluation of residuals at stored placements.

use super::super::solver::{exact, nearest};
use num_rational::BigRational;
use std::ops::{Add, Div, Mul, Neg, Sub};

pub(super) trait Number:
    Copy + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> + Div<Output = Self> + Neg<Output = Self>
{
    fn f(v: f64) -> Self;
    fn q(v: &BigRational) -> Self;
    fn bounds(v: &soliddraft_kernel_api::SupportBounds) -> Self;
    fn sqrt(self) -> Self;
    fn atan2(self, x: Self, near: f64) -> Self;
    fn middle(self) -> f64;
    fn sin_cos(self) -> (Self, Self);
    fn pi() -> Self;
    fn positive(self) -> Self;
}
impl Number for f64 {
    fn f(v: f64) -> Self {
        v
    }
    fn pi() -> Self {
        std::f64::consts::PI
    }
    fn positive(self) -> Self {
        if self.is_nan() { self } else { self.max(0.0) }
    }
    fn q(v: &BigRational) -> Self {
        nearest(v)
    }
    fn bounds(v: &soliddraft_kernel_api::SupportBounds) -> Self {
        let (Some(lo), Some(hi)) = (super::super::solver::rational(&v.lo), super::super::solver::rational(&v.hi))
        else {
            return f64::NAN;
        };
        if lo > hi {
            return f64::NAN;
        }
        nearest(&lo) * 0.5 + nearest(&hi) * 0.5
    }
    fn sqrt(self) -> Self {
        self.sqrt()
    }
    fn atan2(self, x: Self, near: f64) -> Self {
        let a = libm::atan2(self, x);
        let turns = libm::round((near - a) / (2.0 * std::f64::consts::PI));
        if turns == 0.0 { a } else { a + turns * (2.0 * std::f64::consts::PI) }
    }
    fn middle(self) -> f64 {
        self
    }
    fn sin_cos(self) -> (Self, Self) {
        (libm::sin(self), libm::cos(self))
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Interval {
    pub lo: f64,
    pub hi: f64,
}
impl Interval {
    fn new(lo: f64, hi: f64) -> Self {
        Self { lo, hi }
    }
    pub fn finite(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }
    pub fn abs_max(self) -> f64 {
        self.lo.abs().max(self.hi.abs())
    }
    fn whole() -> Self {
        Self::new(f64::NEG_INFINITY, f64::INFINITY)
    }
    fn atan(self) -> Self {
        if !self.finite() {
            return Self::whole();
        }
        if self.lo != self.hi {
            return Self::new(Self::f(self.lo).atan().lo, Self::f(self.hi).atan().hi);
        }
        // Two half-angle reductions put |x| below 0.2 even at infinity.
        let one = Self::f(1.0);
        let negative = self.hi < 0.0;
        if self.lo < 0.0 && self.hi > 0.0 {
            return Self::new(Self::f(self.lo).atan().lo, Self::f(self.hi).atan().hi);
        }
        let v = if negative { -self } else { self };
        let reciprocal = v.lo > 1.0;
        let mut x = if reciprocal { one / v } else { v };
        for _ in 0..2 {
            x = x / (one + (one + x * x).sqrt());
        }
        if x.abs_max() > 0.21 {
            return Self::whole();
        }
        let xx = x * x;
        let mut power = x;
        let mut sum = x;
        for k in 1..32 {
            power = -power * xx;
            sum = sum + power / Self::f((2 * k + 1) as f64);
        }
        // Alternating-series remainder, using the largest possible argument.
        let error: f64 = 1e-43;
        sum = sum * Self::f(4.0) + Self::new(-error.next_up(), error.next_up());
        if reciprocal {
            sum = pi() / Self::f(2.0) - sum;
        }
        if negative { -sum } else { sum }
    }
}
impl Interval {
    fn principal_atan2(self, x: Self) -> Self {
        if x.lo > 0.0 {
            return (self / x).atan();
        }
        if x.hi < 0.0 {
            if self.lo >= 0.0 {
                return (self / x).atan() + pi();
            }
            if self.hi <= 0.0 {
                return (self / x).atan() - pi();
            }
        }
        if self.lo > 0.0 {
            return pi() / Self::f(2.0) - (x / self).atan();
        }
        if self.hi < 0.0 {
            return -pi() / Self::f(2.0) - (x / self).atan();
        }
        Self::whole()
    }
}
fn pi() -> Interval {
    Interval::new(std::f64::consts::PI, std::f64::consts::PI.next_up())
}
impl Add for Interval {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self::new((self.lo + b.lo).next_down(), (self.hi + b.hi).next_up())
    }
}
impl Neg for Interval {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.hi, -self.lo)
    }
}
impl Sub for Interval {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        self + -b
    }
}
impl Mul for Interval {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        let v = [self.lo * b.lo, self.lo * b.hi, self.hi * b.lo, self.hi * b.hi];
        if v.iter().any(|x| x.is_nan()) {
            return Self::whole();
        }
        Self::new(
            v.into_iter().fold(f64::INFINITY, f64::min).next_down(),
            v.into_iter().fold(f64::NEG_INFINITY, f64::max).next_up(),
        )
    }
}
impl Div for Interval {
    type Output = Self;
    fn div(self, b: Self) -> Self {
        if b.lo <= 0.0 && b.hi >= 0.0 {
            return Self::whole();
        }
        self * Self::new((1.0 / b.hi).next_down(), (1.0 / b.lo).next_up())
    }
}
impl Number for Interval {
    fn f(v: f64) -> Self {
        Self::new(v, v)
    }
    fn pi() -> Self {
        pi()
    }
    fn positive(self) -> Self {
        if !self.finite() { Self::whole() } else { Self::new(self.lo.max(0.0), self.hi.max(0.0)) }
    }
    fn q(v: &BigRational) -> Self {
        let f = nearest(v);
        if !f.is_finite() {
            return Self::whole();
        }
        let (mut lo, mut hi) = (f, f);
        for _ in 0..4 {
            if exact(lo).is_some_and(|q| q > *v) {
                lo = lo.next_down();
            }
            if exact(hi).is_some_and(|q| q < *v) {
                hi = hi.next_up();
            }
        }
        if exact(lo).is_some_and(|q| q <= *v) && exact(hi).is_some_and(|q| q >= *v) {
            Self::new(lo, hi)
        } else {
            Self::whole()
        }
    }
    fn bounds(v: &soliddraft_kernel_api::SupportBounds) -> Self {
        let (Some(lo), Some(hi)) = (super::super::solver::rational(&v.lo), super::super::solver::rational(&v.hi))
        else {
            return Self::whole();
        };
        if lo > hi {
            return Self::whole();
        }
        Self::new(Self::q(&lo).lo, Self::q(&hi).hi)
    }
    fn sqrt(self) -> Self {
        if self.hi < 0.0 {
            return Self::whole();
        }
        Self::new(self.lo.max(0.0).sqrt().next_down().max(0.0), self.hi.sqrt().next_up())
    }
    fn middle(self) -> f64 {
        self.lo * 0.5 + self.hi * 0.5
    }
    fn atan2(self, x: Self, near: f64) -> Self {
        let result = if x.hi < 0.0 && self.lo <= 0.0 && self.hi >= 0.0 {
            (self / x).atan() + pi()
        } else {
            self.principal_atan2(x)
        };
        let turns = libm::round((near - result.middle()) / (2.0 * std::f64::consts::PI));
        if turns == 0.0 { result } else { result + pi() * Self::f(2.0 * turns) }
    }
    fn sin_cos(self) -> (Self, Self) {
        let square = self * self;
        let (mut sine, mut cosine) = (self, Self::f(1.0));
        let (mut s, mut c) = (sine, cosine);
        for k in 1..32 {
            s = -s * square / Self::f(((2 * k) * (2 * k + 1)) as f64);
            c = -c * square / Self::f(((2 * k - 1) * (2 * k)) as f64);
            sine = sine + s;
            cosine = cosine + c;
        }
        // Inputs are signed degrees in [-180,180], so |x| < 4. The 64th
        // Taylor remainder is below 4^64/64! < 1e-49 for both functions.
        let error = Self::new(-1e-49, 1e-49);
        (sine + error, cosine + error)
    }
}
pub(super) fn radians<N: Number>(degrees: f64) -> N {
    N::f(degrees) * N::pi() / N::f(180.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trigonometric_enclosures_cover_all_signed_degrees() {
        for degrees in -180..=180 {
            let angle = radians::<Interval>(degrees as f64);
            let (s, c) = angle.sin_cos();
            let reference = (degrees as f64).to_radians();
            assert!(s.lo <= libm::sin(reference) && s.hi >= libm::sin(reference), "sin {degrees}: {s:?}");
            assert!(c.lo <= libm::cos(reference) && c.hi >= libm::cos(reference), "cos {degrees}: {c:?}");
            let near = if degrees < 0 { -1.0 } else { 1.0 };
            let measured = s.atan2(c, near);
            assert!(
                measured.lo <= reference + 1e-14 && measured.hi >= reference - 1e-14,
                "atan {degrees}: {measured:?}"
            );
        }
    }
    #[test]
    fn invalid_intervals_stay_invalid_through_limit_clamping() {
        assert!(!Interval::f(f64::NAN).positive().finite());
        assert!(!Interval::whole().positive().finite());
        assert!(f64::NAN.positive().is_nan());
    }
}

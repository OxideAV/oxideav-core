//! Rational number used for time bases and frame rates.

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

/// An exact fraction `num/den`.
///
/// # Equality vs. value comparison
///
/// The derived [`PartialEq`] / [`Eq`] / [`Hash`] are **structural**:
/// `1/2` and `2/4` compare *unequal* because their fields differ. This
/// is deliberate — it keeps `Hash` cheap and lets callers preserve the
/// exact on-wire fraction (e.g. a `30000/1001` frame rate must not be
/// silently folded into `30/1`). When you want to compare by *value*
/// — "do these two fractions denote the same number?" — use
/// [`Rational::equals_value`] or [`Rational::cmp_value`], which reduce
/// the comparison to an overflow-safe `i128` cross-product. Because the
/// derived `Eq` is structural and value-`cmp` is not, `Rational`
/// deliberately does **not** implement [`Ord`] / [`PartialOrd`] (a
/// value-based ordering would violate the `Ord`/`Eq` consistency
/// contract against the structural `Eq`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const fn new(num: i64, den: i64) -> Self {
        Self { num, den }
    }

    pub const fn zero() -> Self {
        Self { num: 0, den: 1 }
    }

    pub fn is_zero(&self) -> bool {
        self.num == 0
    }

    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Reduce the fraction to lowest terms. Sign is normalized onto the numerator.
    pub fn reduced(mut self) -> Self {
        if self.den < 0 {
            self.num = -self.num;
            self.den = -self.den;
        }
        let g = gcd(self.num.unsigned_abs(), self.den.unsigned_abs()) as i64;
        if g > 1 {
            self.num /= g;
            self.den /= g;
        }
        self
    }

    /// Invert the fraction (num/den → den/num).
    pub fn invert(self) -> Self {
        Self {
            num: self.den,
            den: self.num,
        }
    }

    /// Compare two fractions by the *number they denote*, not by their
    /// stored fields. Uses a 128-bit cross-product so `30000/1001`
    /// and `30/1` order correctly without reducing or losing precision,
    /// and handles negative denominators by folding the sign onto the
    /// numerator first.
    ///
    /// A zero denominator is treated as a signed "infinity": `+n/0`
    /// sorts above every finite fraction, `-n/0` below every finite
    /// fraction, and `0/0` ties with a finite zero. All `+∞` compare
    /// equal to each other (likewise all `-∞`). This is a defensive
    /// total order, not a claim that such a fraction is meaningful.
    pub fn cmp_value(&self, other: &Self) -> Ordering {
        // Normalize sign onto the numerator so the cross-product
        // comparison is monotonic regardless of denominator sign.
        let (an, ad) = sign_normalized(self.num, self.den);
        let (bn, bd) = sign_normalized(other.num, other.den);
        if ad != 0 && bd != 0 {
            // Both finite: exact i128 cross-product.
            return (an as i128 * bd as i128).cmp(&(bn as i128 * ad as i128));
        }
        // At least one side is zero-denominator. Map each value to an
        // "extended sign rank" on the line −∞ … 0 … +∞:
        //   +∞ (n>0, d=0) → +2,  −∞ (n<0, d=0) → −2,  0/0 → 0,
        //   finite > 0 → +1,  finite < 0 → −1,  finite == 0 → 0.
        // Comparing ranks yields a defensive total order: all +∞ tie,
        // all −∞ tie, ±∞ bracket every finite value, and 0/0 ties with
        // a finite zero. (Two finite operands never reach this branch;
        // they are compared exactly above.)
        fn rank(n: i64, d: i64) -> i64 {
            if d == 0 {
                n.signum() * 2
            } else {
                n.signum()
            }
        }
        rank(an, ad).cmp(&rank(bn, bd))
    }

    /// Whether two fractions denote the same number (value equality),
    /// e.g. `Rational::new(2, 4).equals_value(&Rational::new(1, 2))`.
    /// Contrast with `==`, which is structural (field-by-field).
    pub fn equals_value(&self, other: &Self) -> bool {
        self.cmp_value(other) == Ordering::Equal
    }

    /// The sign of the fraction: `-1`, `0`, or `1`. A zero denominator
    /// reports the sign of the numerator.
    pub fn signum(&self) -> i64 {
        let (n, _) = sign_normalized(self.num, self.den);
        n.signum()
    }

    /// The absolute value of the fraction (sign stripped from both
    /// terms; a negative denominator is normalized onto the numerator
    /// first).
    pub fn abs(self) -> Self {
        let (n, d) = sign_normalized(self.num, self.den);
        Self {
            num: n.abs(),
            den: d.abs(),
        }
    }
}

/// Move any sign on the denominator onto the numerator, leaving the
/// denominator non-negative. A zero denominator is left as-is.
#[inline]
const fn sign_normalized(num: i64, den: i64) -> (i64, i64) {
    if den < 0 {
        (-num, -den)
    } else {
        (num, den)
    }
}

/// Reduce a 128-bit `num/den` pair to lowest terms (sign on the
/// numerator) and narrow back to `i64`. Used by the arithmetic
/// operators so intermediate products that overflow `i64` but reduce
/// back into range still yield the right answer.
fn reduce_i128(mut num: i128, mut den: i128) -> Rational {
    if den < 0 {
        num = -num;
        den = -den;
    }
    let g = gcd_i128(num.unsigned_abs(), den.unsigned_abs()) as i128;
    if g > 1 {
        num /= g;
        den /= g;
    }
    Rational {
        num: num as i64,
        den: den as i64,
    }
}

impl Add for Rational {
    type Output = Rational;
    /// Add two fractions, returning the result in lowest terms.
    fn add(self, rhs: Self) -> Self {
        let num = self.num as i128 * rhs.den as i128 + rhs.num as i128 * self.den as i128;
        let den = self.den as i128 * rhs.den as i128;
        reduce_i128(num, den)
    }
}

impl Sub for Rational {
    type Output = Rational;
    /// Subtract `rhs` from `self`, returning the result in lowest terms.
    fn sub(self, rhs: Self) -> Self {
        let num = self.num as i128 * rhs.den as i128 - rhs.num as i128 * self.den as i128;
        let den = self.den as i128 * rhs.den as i128;
        reduce_i128(num, den)
    }
}

impl Mul for Rational {
    type Output = Rational;
    /// Multiply two fractions, returning the result in lowest terms.
    fn mul(self, rhs: Self) -> Self {
        let num = self.num as i128 * rhs.num as i128;
        let den = self.den as i128 * rhs.den as i128;
        reduce_i128(num, den)
    }
}

impl Div for Rational {
    type Output = Rational;
    /// Divide `self` by `rhs` (multiply by the reciprocal), returning
    /// the result in lowest terms.
    fn div(self, rhs: Self) -> Self {
        let num = self.num as i128 * rhs.den as i128;
        let den = self.den as i128 * rhs.num as i128;
        reduce_i128(num, den)
    }
}

impl Neg for Rational {
    type Output = Rational;
    /// Negate the fraction (sign applied to the numerator).
    fn neg(self) -> Self {
        Self {
            num: -self.num,
            den: self.den,
        }
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

fn gcd_i128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce() {
        assert_eq!(Rational::new(10, 20).reduced(), Rational::new(1, 2));
        assert_eq!(Rational::new(-6, 9).reduced(), Rational::new(-2, 3));
        assert_eq!(Rational::new(6, -9).reduced(), Rational::new(-2, 3));
    }

    #[test]
    fn invert() {
        assert_eq!(Rational::new(1, 2).invert(), Rational::new(2, 1));
    }

    #[test]
    fn cmp_value_orders_by_number_not_fields() {
        // Structurally unequal, value-equal.
        assert!(Rational::new(1, 2).equals_value(&Rational::new(2, 4)));
        assert_ne!(Rational::new(1, 2), Rational::new(2, 4));

        assert_eq!(
            Rational::new(1, 3).cmp_value(&Rational::new(1, 2)),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            Rational::new(30_000, 1001).cmp_value(&Rational::new(30, 1)),
            std::cmp::Ordering::Less
        );
        // Negative denominator folds onto numerator for comparison.
        assert!(Rational::new(1, -2).equals_value(&Rational::new(-1, 2)));
        assert_eq!(
            Rational::new(-1, 2).cmp_value(&Rational::new(1, 2)),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn cmp_value_zero_denominator_is_total() {
        let pos_inf = Rational::new(1, 0);
        let neg_inf = Rational::new(-1, 0);
        let finite = Rational::new(1_000_000, 1);
        assert_eq!(pos_inf.cmp_value(&finite), std::cmp::Ordering::Greater);
        assert_eq!(finite.cmp_value(&pos_inf), std::cmp::Ordering::Less);
        assert_eq!(neg_inf.cmp_value(&finite), std::cmp::Ordering::Less);
        assert_eq!(pos_inf.cmp_value(&neg_inf), std::cmp::Ordering::Greater);
        assert!(pos_inf.equals_value(&Rational::new(7, 0)));
    }

    #[test]
    fn signum_and_abs() {
        assert_eq!(Rational::new(3, 4).signum(), 1);
        assert_eq!(Rational::new(-3, 4).signum(), -1);
        assert_eq!(Rational::new(3, -4).signum(), -1);
        assert_eq!(Rational::new(0, 4).signum(), 0);
        assert_eq!(Rational::new(-3, 4).abs(), Rational::new(3, 4));
        assert_eq!(Rational::new(3, -4).abs(), Rational::new(3, 4));
    }

    #[test]
    fn arithmetic_reduces() {
        assert_eq!(
            Rational::new(1, 2) + Rational::new(1, 3),
            Rational::new(5, 6)
        );
        assert_eq!(
            Rational::new(1, 2) - Rational::new(1, 3),
            Rational::new(1, 6)
        );
        // 2/4 * 3/9 = 6/36 = 1/6
        assert_eq!(
            Rational::new(2, 4) * Rational::new(3, 9),
            Rational::new(1, 6)
        );
        // (1/2) / (3/4) = 4/6 = 2/3
        assert_eq!(
            Rational::new(1, 2) / Rational::new(3, 4),
            Rational::new(2, 3)
        );
        assert_eq!(-Rational::new(1, 2), Rational::new(-1, 2));
    }

    #[test]
    fn arithmetic_uses_128bit_intermediates() {
        // Products that overflow i64 but reduce back into range must
        // still yield the correct reduced fraction.
        let big = Rational::new(i64::MAX / 2, 3);
        let r = big * Rational::new(3, 1);
        assert_eq!(r, Rational::new(i64::MAX / 2, 1));
        // num/den both large, equal → reduces to 1/1.
        let a = Rational::new(1_000_000_000, 1);
        let sum = a + a; // 2_000_000_000 / 1
        assert_eq!(sum, Rational::new(2_000_000_000, 1));
    }
}

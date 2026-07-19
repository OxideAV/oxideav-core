//! Property tests for the numeric/time foundations: edge-biased random
//! inputs hammered through `Rational` arithmetic, the rescale kernel,
//! and the bit readers/writers, checked against independent oracles.
//!
//! Deterministic: a fixed-seed LCG drives every case, so failures
//! reproduce exactly.

use oxideav_core::bits::{BitReader, BitReaderLsb, BitWriter, BitWriterLsb};
use oxideav_core::rational::Rational;
use oxideav_core::time::{rescale, rescale_checked, rescale_rnd, Rounding, TimeBase, Timestamp};

/// Minimal deterministic PRNG (64-bit LCG, high-bits output).
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        // Mix the high bits down; plain LCG low bits are weak.
        (self.0 >> 32) ^ self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 16
    }
    /// An i64 heavily biased toward the overflow-prone edges.
    fn edge_i64(&mut self) -> i64 {
        match self.next_u64() % 8 {
            0 => i64::MIN,
            1 => i64::MAX,
            2 => 0,
            3 => 1,
            4 => -1,
            5 => (self.next_u64() % 1000) as i64 - 500,
            6 => i64::MIN + (self.next_u64() % 1000) as i64,
            _ => self.next_u64() as i64,
        }
    }
    /// A small positive i64 (1..=bound).
    fn small_pos(&mut self, bound: u64) -> i64 {
        (self.next_u64() % bound) as i64 + 1
    }
}

// ==================== Rational ====================

/// Independent oracle: compare a `Rational` result against an exact
/// `num/den` pair held in i128 (den > 0), by cross-product in i128 —
/// valid because both sides' terms fit in 64/65 bits.
fn rational_equals_i128(r: Rational, num: i128, den: i128) -> bool {
    // r.num/r.den == num/den  <=>  r.num * den == num * r.den
    // |r.num| ≤ 2^63, |den| ≤ 2^126 → the product needs > i128. Restrict
    // callers to |den| ≤ 2^63 so the cross-product stays in range.
    debug_assert!(den > 0 && den <= 1 << 63);
    r.num as i128 * den == num * r.den as i128
}

#[test]
fn rational_ops_never_panic_and_normalize_sign() {
    let mut rng = Lcg::new(1);
    for _ in 0..20_000 {
        let a = Rational::new(rng.edge_i64(), rng.edge_i64());
        let b = Rational::new(rng.edge_i64(), rng.edge_i64());
        // Every entry point must be total.
        let results = [a + b, a - b, a * b, a / b, -a, a.reduced(), a.abs()];
        let _ = a.cmp_value(&b);
        let _ = a.equals_value(&b);
        let _ = a.signum();
        let _ = a.invert();
        let _ = a.checked_add(b);
        let _ = a.checked_sub(b);
        let _ = a.checked_mul(b);
        let _ = a.checked_div(b);
        // The operators and reduced() normalize any sign onto the
        // numerator: a negative denominator never escapes.
        for (i, r) in results.iter().enumerate().take(4) {
            assert!(
                r.den >= 0,
                "op {i} produced negative den: {a} vs {b} -> {r}"
            );
        }
        assert!(results[5].den >= 0, "reduced() negative den for {a}");
        let abs = results[6];
        assert!(
            abs.num >= 0 && abs.den >= 0,
            "abs() kept a sign: {a} -> {abs}"
        );
    }
}

#[test]
fn rational_checked_ops_match_exact_i128_oracle() {
    let mut rng = Lcg::new(2);
    let mut exact = 0u32;
    for _ in 0..20_000 {
        // Bound the fields so the oracle cross-product below fits i128.
        let a = Rational::new(rng.edge_i64() >> 33, rng.small_pos(1 << 30));
        let b = Rational::new(rng.edge_i64() >> 33, rng.small_pos(1 << 30));
        // Oracle in exact i128 (positive denominators by construction).
        let num = a.num as i128 * b.den as i128 + b.num as i128 * a.den as i128;
        let den = a.den as i128 * b.den as i128;
        let sum = a.checked_add(b).expect("in-range checked_add must succeed");
        assert!(
            rational_equals_i128(sum, num, den),
            "checked_add mismatch: {a} + {b} -> {sum}, exact {num}/{den}"
        );
        // The operator must agree with the checked path when it's exact.
        assert_eq!(a + b, sum);
        exact += 1;
    }
    assert_eq!(exact, 20_000);
}

#[test]
fn rational_neg_and_abs_preserve_value() {
    let mut rng = Lcg::new(3);
    for _ in 0..20_000 {
        let a = Rational::new(rng.edge_i64(), rng.edge_i64());
        if a.den == 0 {
            continue; // defensive-infinity values don't negate exactly
        }
        // Double negation is value-identity (representation may differ).
        assert!((-(-a)).equals_value(&a), "-(-{a}) = {} != {a}", -(-a));
        // |a| never compares below zero…
        let abs = a.abs();
        assert!(abs.signum() >= 0, "abs({a}) = {abs} has negative sign");
        // …and equals ±a exactly, except when an i64::MIN term forces
        // the documented closest-representable approximation (|MIN|
        // doesn't fit i64; reduction may not bring it back in range).
        if a.num != i64::MIN && a.den != i64::MIN {
            assert!(
                abs.equals_value(&a) || abs.equals_value(&(-a)),
                "abs({a}) = {abs} matches neither ±input"
            );
        }
    }
}

#[test]
fn rational_cmp_value_is_consistent_total_order() {
    let mut rng = Lcg::new(4);
    for _ in 0..10_000 {
        let a = Rational::new(rng.edge_i64(), rng.edge_i64());
        let b = Rational::new(rng.edge_i64(), rng.edge_i64());
        let c = Rational::new(rng.edge_i64(), rng.edge_i64());
        // Antisymmetry.
        assert_eq!(a.cmp_value(&b), b.cmp_value(&a).reverse());
        // Reflexivity.
        assert_eq!(a.cmp_value(&a), std::cmp::Ordering::Equal);
        // Transitivity of ≤ (spot form).
        if a.cmp_value(&b) != std::cmp::Ordering::Greater
            && b.cmp_value(&c) != std::cmp::Ordering::Greater
        {
            assert_ne!(
                a.cmp_value(&c),
                std::cmp::Ordering::Greater,
                "transitivity violated: {a} <= {b} <= {c}"
            );
        }
        // reduced() preserves value ordering — exactly representable
        // inputs only: an i64::MIN term can force reduced() into its
        // documented closest-representable approximation (e.g.
        // (i64::MIN)/-1 = 2^63 saturates to i64::MAX), which is allowed
        // to collapse an ordering against a nearby value.
        let has_min = [a.num, a.den, b.num, b.den].contains(&i64::MIN);
        if !has_min {
            assert_eq!(
                a.reduced().cmp_value(&b.reduced()),
                a.cmp_value(&b),
                "reduced() changed ordering of {a} vs {b}"
            );
        }
    }
}

// ==================== rescale ====================

/// Exact oracle for small-domain rescale: all arithmetic fits i128
/// comfortably (|value| ≤ 2^40, terms ≤ 2^20). Half-away-from-zero.
fn rescale_oracle(value: i64, from: Rational, to: Rational) -> i64 {
    let num = from.num as i128 * to.den as i128;
    let den = from.den as i128 * to.num as i128;
    assert!(den > 0);
    let p = value as i128 * num;
    let q = p / den;
    let r = (p % den).abs();
    let bump = if r * 2 >= den { p.signum() } else { 0 };
    (q + bump) as i64
}

#[test]
fn rescale_matches_small_domain_oracle() {
    let mut rng = Lcg::new(5);
    for _ in 0..20_000 {
        let value = (rng.next_u64() % (1 << 41)) as i64 - (1 << 40);
        let from = Rational::new(rng.small_pos(1 << 20), rng.small_pos(1 << 20));
        let to = Rational::new(rng.small_pos(1 << 20), rng.small_pos(1 << 20));
        let want = rescale_oracle(value, from, to);
        assert_eq!(
            rescale(value, from, to),
            want,
            "rescale({value}, {from}, {to})"
        );
        assert_eq!(
            rescale_checked(value, from, to),
            Some(want),
            "rescale_checked({value}, {from}, {to})"
        );
    }
}

#[test]
fn rescale_never_panics_and_checked_agrees() {
    let mut rng = Lcg::new(6);
    for _ in 0..20_000 {
        let value = rng.edge_i64();
        let from = Rational::new(rng.edge_i64(), rng.edge_i64());
        let to = Rational::new(rng.edge_i64(), rng.edge_i64());
        let plain = rescale(value, from, to);
        match rescale_checked(value, from, to) {
            // When checked succeeds, the plain path must return the
            // same (unsaturated) result.
            Some(v) => assert_eq!(plain, v, "rescale({value}, {from}, {to})"),
            // When checked fails, the plain path returns its documented
            // fallback: 0 for an undefined factor, a saturated bound
            // otherwise.
            None => assert!(
                plain == 0 || plain == i64::MAX || plain == i64::MIN,
                "rescale({value}, {from}, {to}) = {plain} but checked = None"
            ),
        }
        // All rounding modes are total too.
        for mode in [
            Rounding::NearestAway,
            Rounding::Floor,
            Rounding::Ceil,
            Rounding::TowardZero,
        ] {
            let _ = rescale_rnd(value, from, to, mode);
        }
    }
}

#[test]
fn rescale_rounding_modes_bracket_correctly() {
    let mut rng = Lcg::new(7);
    for _ in 0..20_000 {
        let value = (rng.next_u64() % (1 << 41)) as i64 - (1 << 40);
        let from = Rational::new(rng.small_pos(1 << 20), rng.small_pos(1 << 20));
        let to = Rational::new(rng.small_pos(1 << 20), rng.small_pos(1 << 20));
        let floor = rescale_rnd(value, from, to, Rounding::Floor);
        let ceil = rescale_rnd(value, from, to, Rounding::Ceil);
        let near = rescale_rnd(value, from, to, Rounding::NearestAway);
        let zero = rescale_rnd(value, from, to, Rounding::TowardZero);
        assert!(ceil - floor <= 1, "floor/ceil differ by >1 tick");
        assert!(
            (floor..=ceil).contains(&near),
            "nearest outside [floor, ceil]"
        );
        assert!(
            (floor..=ceil).contains(&zero),
            "toward-zero outside [floor, ceil]"
        );
        // Toward-zero picks floor for positives, ceil for negatives.
        if value >= 0 {
            assert_eq!(zero, floor);
        } else {
            assert_eq!(zero, ceil);
        }
    }
}

#[test]
fn rescale_is_monotonic_in_value() {
    let mut rng = Lcg::new(8);
    let from = TimeBase::MPEG_TS;
    let to = TimeBase::MILLIS;
    for _ in 0..20_000 {
        let v1 = rng.edge_i64();
        let v2 = rng.edge_i64();
        let (lo, hi) = if v1 <= v2 { (v1, v2) } else { (v2, v1) };
        assert!(
            from.rescale(lo, to) <= from.rescale(hi, to),
            "monotonicity violated at {lo}..{hi}"
        );
    }
}

#[test]
fn timestamp_checked_ops_are_total() {
    let mut rng = Lcg::new(9);
    for _ in 0..10_000 {
        let ts = Timestamp::new(
            rng.edge_i64(),
            TimeBase::new(rng.edge_i64(), rng.edge_i64()),
        );
        let other = Timestamp::new(
            rng.edge_i64(),
            TimeBase::new(rng.edge_i64(), rng.edge_i64()),
        );
        let _ = ts.checked_add_ticks(rng.edge_i64());
        let _ = ts.checked_sub_ticks(rng.edge_i64());
        let _ = ts.checked_diff(other);
        let _ = ts.checked_rescale(other.base);
        let _ = ts.rescale(other.base);
        let _ = ts.seconds();
    }
}

// ==================== bits ====================

#[test]
fn bits_msb_random_write_read_roundtrip() {
    let mut rng = Lcg::new(10);
    for _ in 0..500 {
        let mut writes: Vec<(u32, u32)> = Vec::new();
        let mut w = BitWriter::new();
        for _ in 0..rng.next_u64() % 200 {
            let n = (rng.next_u64() % 33) as u32; // 0..=32
            let v = rng.next_u64() as u32;
            w.write_u32(v, n);
            writes.push((v, n));
        }
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        for &(v, n) in &writes {
            let mask = if n == 0 {
                0
            } else if n == 32 {
                u32::MAX
            } else {
                (1u32 << n) - 1
            };
            assert_eq!(r.read_u32(n).unwrap(), v & mask, "width {n}");
        }
    }
}

#[test]
fn bits_lsb_random_write_read_roundtrip() {
    let mut rng = Lcg::new(11);
    for _ in 0..500 {
        let mut writes: Vec<(u32, u32)> = Vec::new();
        let mut w = BitWriterLsb::new();
        for _ in 0..rng.next_u64() % 200 {
            let n = (rng.next_u64() % 33) as u32;
            let v = rng.next_u64() as u32;
            w.write_u32(v, n);
            writes.push((v, n));
        }
        let bytes = w.finish();
        let mut r = BitReaderLsb::new(&bytes);
        for &(v, n) in &writes {
            let mask = if n == 0 {
                0
            } else if n == 32 {
                u32::MAX
            } else {
                (1u32 << n) - 1
            };
            assert_eq!(r.read_u32(n).unwrap(), v & mask, "width {n}");
        }
    }
}

#[test]
fn bits_msb_reader_never_panics_on_random_ops() {
    let mut rng = Lcg::new(12);
    for _ in 0..500 {
        let len = (rng.next_u64() % 64) as usize;
        let data: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
        let mut r = BitReader::new(&data);
        for _ in 0..100 {
            match rng.next_u64() % 8 {
                0 => {
                    let _ = r.read_u32((rng.next_u64() % 33) as u32);
                }
                1 => {
                    let _ = r.read_u64((rng.next_u64() % 65) as u32);
                }
                2 => {
                    let _ = r.read_i32((rng.next_u64() % 33) as u32);
                }
                3 => {
                    let _ = r.peek_u32((rng.next_u64() % 33) as u32);
                }
                4 => {
                    let _ = r.skip((rng.next_u64() % 100) as u32);
                }
                5 => {
                    let _ = r.read_unary();
                }
                6 => r.align_to_byte(),
                _ => {
                    let _ = r.read_bytes((rng.next_u64() % 16) as usize);
                }
            }
            // Position invariants hold whatever happened above.
            assert!(r.bit_position() <= data.len() as u64 * 8);
            assert_eq!(r.bits_remaining(), data.len() as u64 * 8 - r.bit_position());
        }
    }
}

// ==================== VideoFrame side-channels ====================

/// Model-based property run for the `VideoFrame` side-channels: random
/// sequences of palette / significant-bits set/take operations checked
/// against a trivially-correct model (two `Option<Vec<u8>>`s plus the
/// frozen image planes). The two records must never interfere with each
/// other or with the image planes, whatever order operations arrive in.
#[test]
fn video_frame_side_channels_match_two_option_model() {
    use oxideav_core::{VideoFrame, VideoPlane};

    let mut rng = Lcg::new(0xF3A7);
    for _ in 0..500 {
        // Random image geometry: 0..=4 planes of random shape.
        let plane_count = (rng.next_u64() % 5) as usize;
        let planes: Vec<VideoPlane> = (0..plane_count)
            .map(|_| {
                let stride = (rng.next_u64() % 16 + 1) as usize;
                let rows = (rng.next_u64() % 8) as usize;
                VideoPlane {
                    stride,
                    data: (0..stride * rows).map(|_| rng.next_u64() as u8).collect(),
                }
            })
            .collect();
        let frozen: Vec<(usize, Vec<u8>)> =
            planes.iter().map(|p| (p.stride, p.data.clone())).collect();

        let mut frame = VideoFrame { pts: None, planes };
        let mut model_palette: Option<Vec<u8>> = None;
        let mut model_bits: Option<Vec<u8>> = None;

        for _ in 0..40 {
            match rng.next_u64() % 6 {
                0 => {
                    // Attach/replace/clear the palette (empty clears).
                    let n = (rng.next_u64() % 4) as usize * 3;
                    let pal: Vec<u8> = (0..n).map(|_| rng.next_u64() as u8).collect();
                    model_palette = if pal.is_empty() {
                        None
                    } else {
                        Some(pal.clone())
                    };
                    frame.set_palette(pal);
                }
                1 => {
                    // Attach/replace/clear the depth record.
                    let n = (rng.next_u64() % 6) as usize;
                    let bits: Vec<u8> = (0..n).map(|_| (rng.next_u64() % 16 + 1) as u8).collect();
                    model_bits = if bits.is_empty() {
                        None
                    } else {
                        Some(bits.clone())
                    };
                    frame.set_significant_bits(bits);
                }
                2 => {
                    assert_eq!(frame.take_palette(), model_palette.take());
                }
                3 => {
                    assert_eq!(frame.take_significant_bits(), model_bits.take());
                }
                4 => {
                    // Per-entry sugar agrees with the raw record.
                    let idx = rng.next_u64() as usize % 8;
                    assert_eq!(
                        frame.plane_significant_bits(idx),
                        model_bits.as_deref().and_then(|b| b.get(idx).copied())
                    );
                }
                _ => {
                    let entry = rng.next_u64() as u8;
                    let expect = model_palette.as_deref().and_then(|p| {
                        let at = usize::from(entry) * 3;
                        p.get(at..at + 3).map(|e| [e[0], e[1], e[2]])
                    });
                    assert_eq!(frame.palette_rgb(entry), expect);
                }
            }

            // Invariants after EVERY operation:
            assert_eq!(frame.palette(), model_palette.as_deref());
            assert_eq!(frame.significant_bits(), model_bits.as_deref());
            // Image planes are never touched by side-channel traffic.
            assert_eq!(frame.image_plane_count(), frozen.len());
            for (plane, (stride, data)) in frame.image_planes().iter().zip(&frozen) {
                assert_eq!(plane.stride, *stride);
                assert_eq!(&plane.data, data);
            }
            // Raw plane vector = image planes + one entry per record.
            let records = usize::from(model_palette.is_some()) + usize::from(model_bits.is_some());
            assert_eq!(frame.planes.len(), frozen.len() + records);
        }
    }
}

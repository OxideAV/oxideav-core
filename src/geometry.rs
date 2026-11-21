//! Video stream geometry: pixel aspect ratio, clean aperture, field
//! order and display orientation.
//!
//! A decoded picture is a grid of `width` × `height` samples. Four more
//! facts decide how that grid reaches a screen, and none of them is
//! visible in the samples themselves:
//!
//! * the **pixel aspect ratio** — whether one sample is square or
//!   wider / taller than it is high ([`VideoGeometry::pixel_aspect`]);
//! * the **clean aperture** — the sub-rectangle of the grid that is
//!   meant to be shown, positioned with sub-pixel precision
//!   ([`CleanAperture`]);
//! * the **field order** — whether the picture is progressive or two
//!   interleaved fields, and which field comes first ([`FieldOrder`]);
//! * the **display matrix** — a 3×3 transformation from the decoded
//!   frame to the rendering surface, in practice almost always a
//!   quarter-turn rotation and / or a mirror ([`DisplayMatrix`],
//!   [`DisplayOrientation`]).
//!
//! [`VideoGeometry`] groups the four. It is a **stream-level** record:
//! it lives in
//! [`CodecParameters::geometry`](crate::CodecParameters::geometry),
//! never on individual frames, and a mid-stream change travels through
//! the same in-band [`Frame::StreamParams`](crate::Frame::StreamParams)
//! event as a size or pixel-format change. Every part is optional and
//! defaults to "unspecified", so streams that say nothing are exactly
//! what they always were.
//!
//! The definitions follow ISO/IEC 14496-12: §12.1.4 (the `pasp` and
//! `clap` sample-entry boxes) for the pixel aspect ratio and the clean
//! aperture, and §6.2.2 (the matrix carried by the track and movie
//! header boxes) for the display matrix. The field-order vocabulary is
//! that of the QuickTime `fiel` sample-description extension.

use crate::rational::Rational;

/// One of the two fields of an interlaced picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// The field that contains the topmost scan line of the picture
    /// (lines 0, 2, 4, …).
    Top,
    /// The other field (lines 1, 3, 5, …).
    Bottom,
}

impl Field {
    /// The other field.
    pub const fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }
}

/// Scan structure of a video stream: progressive, or interlaced with a
/// given temporal and storage order of the two fields.
///
/// Two independent orders exist for interlaced material: which field is
/// **displayed** earliest (what a deinterlacer needs) and which field
/// is **stored** first in each coded sample (what a codec or muxer
/// needs). They usually agree; the two cross variants cover the
/// streams where they do not.
///
/// `#[non_exhaustive]`: match with a wildcard arm, or use the
/// predicates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FieldOrder {
    /// Nothing is known about the scan structure.
    #[default]
    Unspecified,
    /// Every picture is one progressive frame.
    Progressive,
    /// Interlaced; the top field is displayed earliest and stored
    /// first.
    TopFirst,
    /// Interlaced; the bottom field is displayed earliest and stored
    /// first.
    BottomFirst,
    /// Interlaced; the top field is displayed earliest but the bottom
    /// field is stored first.
    TopDisplayedBottomStored,
    /// Interlaced; the bottom field is displayed earliest but the top
    /// field is stored first.
    BottomDisplayedTopStored,
    /// Interlaced, but the producer does not know the field order.
    Interlaced,
}

impl FieldOrder {
    /// `true` for [`Unspecified`](Self::Unspecified).
    pub const fn is_unspecified(self) -> bool {
        matches!(self, Self::Unspecified)
    }

    /// `true` for [`Progressive`](Self::Progressive).
    pub const fn is_progressive(self) -> bool {
        matches!(self, Self::Progressive)
    }

    /// `true` for every interlaced variant, including
    /// [`Interlaced`](Self::Interlaced) (order unknown).
    pub const fn is_interlaced(self) -> bool {
        matches!(
            self,
            Self::TopFirst
                | Self::BottomFirst
                | Self::TopDisplayedBottomStored
                | Self::BottomDisplayedTopStored
                | Self::Interlaced
        )
    }

    /// The field displayed earliest, when the stream is interlaced and
    /// the order is known.
    pub const fn first_displayed(self) -> Option<Field> {
        match self {
            Self::TopFirst | Self::TopDisplayedBottomStored => Some(Field::Top),
            Self::BottomFirst | Self::BottomDisplayedTopStored => Some(Field::Bottom),
            _ => None,
        }
    }

    /// The field stored first in each coded sample, when the stream is
    /// interlaced and the order is known.
    pub const fn first_stored(self) -> Option<Field> {
        match self {
            Self::TopFirst | Self::BottomDisplayedTopStored => Some(Field::Top),
            Self::BottomFirst | Self::TopDisplayedBottomStored => Some(Field::Bottom),
            _ => None,
        }
    }

    /// Build an interlaced order from its two component orders.
    pub const fn from_fields(first_displayed: Field, first_stored: Field) -> Self {
        match (first_displayed, first_stored) {
            (Field::Top, Field::Top) => Self::TopFirst,
            (Field::Bottom, Field::Bottom) => Self::BottomFirst,
            (Field::Top, Field::Bottom) => Self::TopDisplayedBottomStored,
            (Field::Bottom, Field::Top) => Self::BottomDisplayedTopStored,
        }
    }

    /// Interpret the two bytes of a QuickTime `fiel` sample-description
    /// extension: `count` is the field count (1 = progressive, 2 =
    /// interlaced) and `ordering` the field-ordering code (1, 6, 9 or
    /// 14 when `count` is 2). Any other combination yields
    /// [`Interlaced`](Self::Interlaced) for a count of 2 and
    /// [`Unspecified`](Self::Unspecified) otherwise.
    ///
    /// ```
    /// # use oxideav_core::geometry::FieldOrder;
    /// assert_eq!(FieldOrder::from_field_atom(1, 0), FieldOrder::Progressive);
    /// assert_eq!(FieldOrder::from_field_atom(2, 6), FieldOrder::BottomFirst);
    /// assert_eq!(FieldOrder::BottomFirst.to_field_atom(), Some((2, 6)));
    /// ```
    pub const fn from_field_atom(count: u8, ordering: u8) -> Self {
        match (count, ordering) {
            (1, _) => Self::Progressive,
            (2, 1) => Self::TopFirst,
            (2, 6) => Self::BottomFirst,
            (2, 9) => Self::BottomDisplayedTopStored,
            (2, 14) => Self::TopDisplayedBottomStored,
            (2, _) => Self::Interlaced,
            _ => Self::Unspecified,
        }
    }

    /// The `(count, ordering)` byte pair of a QuickTime `fiel`
    /// extension describing this order, or `None` for
    /// [`Unspecified`](Self::Unspecified) (write no extension).
    /// [`Interlaced`](Self::Interlaced) maps to `(2, 0)`.
    pub const fn to_field_atom(self) -> Option<(u8, u8)> {
        match self {
            Self::Unspecified => None,
            Self::Progressive => Some((1, 0)),
            Self::TopFirst => Some((2, 1)),
            Self::BottomFirst => Some((2, 6)),
            Self::BottomDisplayedTopStored => Some((2, 9)),
            Self::TopDisplayedBottomStored => Some((2, 14)),
            Self::Interlaced => Some((2, 0)),
        }
    }
}

/// The region of the coded picture that is meant to be displayed,
/// expressed as in the ISO/IEC 14496-12 §12.1.4 `clap` box: an exact
/// (possibly fractional) width and height, and the offset of the
/// region's centre from the centre of the coded picture.
///
/// All four values are fractions and need not be in reduced terms.
/// With a coded picture of `W` × `H` samples, the picture centre is at
/// `pcX = horiz_off + (W - 1) / 2`, `pcY = vert_off + (H - 1) / 2`, and
/// the outermost sample centres of the aperture fall at
/// `pcX ± (width - 1) / 2` and `pcY ± (height - 1) / 2`. Equivalently
/// the aperture's left edge lies `horiz_off + (W - width) / 2` samples
/// from the left edge of the coded picture — see
/// [`rect_in`](Self::rect_in).
///
/// The dimensions the offsets refer to are the `width` / `height` of
/// the [`CodecParameters`](crate::CodecParameters) the record travels
/// in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct CleanAperture {
    /// Exact aperture width in counted samples. Positive.
    pub width: Rational,
    /// Exact aperture height in counted samples. Positive.
    pub height: Rational,
    /// Horizontal offset of the aperture centre from the coded picture
    /// centre, in samples; positive moves the aperture right.
    pub horiz_off: Rational,
    /// Vertical offset of the aperture centre from the coded picture
    /// centre, in samples; positive moves the aperture down.
    pub vert_off: Rational,
}

/// A clean aperture resolved against concrete coded dimensions: the
/// top-left corner and size of the displayed region, in samples of the
/// coded picture, with sub-pixel precision.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ApertureRect {
    /// Distance of the region's left edge from the picture's left edge.
    pub left: f64,
    /// Distance of the region's top edge from the picture's top edge.
    pub top: f64,
    /// Region width in samples.
    pub width: f64,
    /// Region height in samples.
    pub height: f64,
}

impl ApertureRect {
    /// The rectangle as whole samples `(left, top, width, height)` when
    /// all four values are non-negative integers, `None` when any of
    /// them is fractional or negative (the consumer then resamples, or
    /// rounds by its own policy).
    pub fn to_integer(&self) -> Option<(u32, u32, u32, u32)> {
        fn whole(v: f64) -> Option<u32> {
            (v >= 0.0 && v <= f64::from(u32::MAX) && v.fract() == 0.0).then_some(v as u32)
        }
        Some((
            whole(self.left)?,
            whole(self.top)?,
            whole(self.width)?,
            whole(self.height)?,
        ))
    }
}

impl CleanAperture {
    /// Build a clean aperture from its four fractions.
    pub const fn new(
        width: Rational,
        height: Rational,
        horiz_off: Rational,
        vert_off: Rational,
    ) -> Self {
        Self {
            width,
            height,
            horiz_off,
            vert_off,
        }
    }

    /// A `width` × `height` aperture centred on the coded picture
    /// (both offsets zero).
    pub const fn centered(width: u32, height: u32) -> Self {
        Self {
            width: Rational::new(width as i64, 1),
            height: Rational::new(height as i64, 1),
            horiz_off: Rational::new(0, 1),
            vert_off: Rational::new(0, 1),
        }
    }

    /// The aperture equivalent to an integer crop rectangle: `width` ×
    /// `height` samples whose top-left corner is `left` / `top` samples
    /// into a coded picture of `coded_width` × `coded_height`.
    ///
    /// ```
    /// # use oxideav_core::geometry::CleanAperture;
    /// // 1920×1080 shown out of a 1920×1088 coded picture, top-aligned.
    /// let clap = CleanAperture::from_crop(1920, 1088, 0, 0, 1920, 1080);
    /// assert_eq!(clap.vert_off.as_f64(), -4.0);
    /// let r = clap.rect_in(1920, 1088).unwrap();
    /// assert_eq!(r.to_integer(), Some((0, 0, 1920, 1080)));
    /// ```
    pub const fn from_crop(
        coded_width: u32,
        coded_height: u32,
        left: u32,
        top: u32,
        width: u32,
        height: u32,
    ) -> Self {
        // left = horiz_off + (W - width) / 2
        //   ⇒ horiz_off = (2·left + width − W) / 2, likewise vertically.
        let h2 = 2 * left as i64 + width as i64 - coded_width as i64;
        let v2 = 2 * top as i64 + height as i64 - coded_height as i64;
        Self {
            width: Rational::new(width as i64, 1),
            height: Rational::new(height as i64, 1),
            horiz_off: if h2 % 2 == 0 {
                Rational::new(h2 / 2, 1)
            } else {
                Rational::new(h2, 2)
            },
            vert_off: if v2 % 2 == 0 {
                Rational::new(v2 / 2, 1)
            } else {
                Rational::new(v2, 2)
            },
        }
    }

    /// `true` when every denominator is non-zero and the width and
    /// height are positive — the conditions §12.1.4 places on the box.
    pub fn is_valid(&self) -> bool {
        self.width.den != 0
            && self.height.den != 0
            && self.horiz_off.den != 0
            && self.vert_off.den != 0
            && self.width.signum() > 0
            && self.height.signum() > 0
    }

    /// Resolve the aperture against a coded picture of `coded_width` ×
    /// `coded_height` samples. `None` when the record is not
    /// [valid](Self::is_valid). The result is not clamped to the
    /// picture; a consumer that crops intersects it with the picture
    /// bounds.
    pub fn rect_in(&self, coded_width: u32, coded_height: u32) -> Option<ApertureRect> {
        if !self.is_valid() {
            return None;
        }
        let width = self.width.as_f64();
        let height = self.height.as_f64();
        Some(ApertureRect {
            left: self.horiz_off.as_f64() + (f64::from(coded_width) - width) / 2.0,
            top: self.vert_off.as_f64() + (f64::from(coded_height) - height) / 2.0,
            width,
            height,
        })
    }
}

/// A display orientation: an optional horizontal mirror followed by a
/// clockwise rotation of 0, 90, 180 or 270 degrees.
///
/// The mirror is applied **first** (left and right of the decoded
/// picture swap), then the rotation, clockwise as seen by the viewer.
/// These eight combinations are every way of placing a rectangle
/// upright on a screen; a vertical flip is `mirror` plus a half turn.
///
/// ```
/// # use oxideav_core::geometry::DisplayOrientation;
/// let o = DisplayOrientation::rotated_cw(1);
/// assert_eq!(o.degrees_cw(), 90);
/// assert_eq!(o.output_size(1920, 1080), (1080, 1920));
/// assert_eq!(o.then(o.inverse()), DisplayOrientation::IDENTITY);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DisplayOrientation {
    quarter_turns_cw: u8,
    mirror: bool,
}

impl DisplayOrientation {
    /// No mirror, no rotation.
    pub const IDENTITY: Self = Self {
        quarter_turns_cw: 0,
        mirror: false,
    };

    /// Build an orientation from a clockwise quarter-turn count (taken
    /// modulo 4) and a mirror flag (applied before the rotation).
    pub const fn new(quarter_turns_cw: u8, mirror: bool) -> Self {
        Self {
            quarter_turns_cw: quarter_turns_cw % 4,
            mirror,
        }
    }

    /// A pure clockwise rotation by `quarter_turns` × 90 degrees.
    pub const fn rotated_cw(quarter_turns: u8) -> Self {
        Self::new(quarter_turns, false)
    }

    /// A pure rotation by `degrees` clockwise, when `degrees` is a
    /// multiple of 90 (negative values rotate counter-clockwise).
    /// `None` for any other angle.
    pub const fn from_degrees_cw(degrees: i32) -> Option<Self> {
        if degrees % 90 != 0 {
            return None;
        }
        Some(Self::new((degrees / 90).rem_euclid(4) as u8, false))
    }

    /// A horizontal mirror (left ↔ right) with no rotation.
    pub const fn mirrored_horizontally() -> Self {
        Self::new(0, true)
    }

    /// A vertical flip (top ↔ bottom): a horizontal mirror followed by
    /// a half turn.
    pub const fn flipped_vertically() -> Self {
        Self::new(2, true)
    }

    /// Clockwise quarter turns applied after the mirror, `0..=3`.
    pub const fn quarter_turns_cw(self) -> u8 {
        self.quarter_turns_cw
    }

    /// The clockwise rotation in degrees: 0, 90, 180 or 270.
    pub const fn degrees_cw(self) -> u16 {
        self.quarter_turns_cw as u16 * 90
    }

    /// Whether the picture is mirrored horizontally before the
    /// rotation.
    pub const fn is_mirrored(self) -> bool {
        self.mirror
    }

    /// `true` for the identity orientation.
    pub const fn is_identity(self) -> bool {
        self.quarter_turns_cw == 0 && !self.mirror
    }

    /// `true` when the displayed picture's width and height are the
    /// decoded picture's height and width (a 90 or 270 degree turn).
    pub const fn swaps_dimensions(self) -> bool {
        self.quarter_turns_cw % 2 == 1
    }

    /// The displayed size of a `width` × `height` decoded picture.
    pub const fn output_size(self, width: u32, height: u32) -> (u32, u32) {
        if self.swaps_dimensions() {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// The orientation equivalent to applying `self` and then `next`.
    pub const fn then(self, next: Self) -> Self {
        // With R a clockwise quarter turn and M the mirror, M·R = R⁻¹·M,
        // so Rᵇ·Mⁿ · Rᵃ·Mᵐ = R^(b ± a) · M^(n xor m), minus when n is set.
        let a = self.quarter_turns_cw;
        let turns = if next.mirror {
            next.quarter_turns_cw + 4 - a
        } else {
            next.quarter_turns_cw + a
        };
        Self::new(turns, self.mirror != next.mirror)
    }

    /// The orientation that undoes this one.
    pub const fn inverse(self) -> Self {
        // (Rᵃ·M)⁻¹ = M·R⁻ᵃ = Rᵃ·M; a pure rotation inverts its angle.
        if self.mirror {
            self
        } else {
            Self::new(4 - self.quarter_turns_cw, false)
        }
    }

    /// The 2×2 integer matrix `[a, b, c, d]` of this orientation in the
    /// [`DisplayMatrix`] convention (`p' = a·p + c·q`,
    /// `q' = b·p + d·q`, Y growing downward).
    const fn linear(self) -> [i32; 4] {
        // Columns of the rotation: image of (1,0) and of (0,1).
        let (rx, ry, sx, sy) = match self.quarter_turns_cw {
            0 => (1, 0, 0, 1),
            1 => (0, 1, -1, 0),
            2 => (-1, 0, 0, -1),
            _ => (0, -1, 1, 0),
        };
        // The mirror negates p first, i.e. negates the first column.
        if self.mirror {
            [-rx, -ry, sx, sy]
        } else {
            [rx, ry, sx, sy]
        }
    }
}

/// The 3×3 transformation from the decoded frame to the rendering
/// surface, as carried by the ISO/IEC 14496-12 track and movie headers
/// (§6.2.2).
///
/// The nine values are held in the stored order `{a, b, u, c, d, v, x,
/// y, w}` and in the stored fixed-point formats — 16.16 for `a b c d x
/// y`, 2.30 for `u v w` — so a demux → mux round trip is exact. A point
/// `(p, q)` of the decoded frame maps to `(p', q')` with
///
/// ```text
/// m = a·p + c·q + x      n = b·p + d·q + y      z = u·p + v·q + w
/// p' = m / z             q' = n / z
/// ```
///
/// The origin is the upper-left corner, X grows to the right and Y
/// downward.
///
/// In practice the matrix is a rotation by a multiple of 90 degrees,
/// sometimes with a mirror, plus the translation that brings the
/// rotated picture back into the positive quadrant;
/// [`orientation`](Self::orientation) recognises exactly those and
/// [`from_orientation`](Self::from_orientation) builds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DisplayMatrix {
    m: [i32; 9],
}

impl Default for DisplayMatrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl DisplayMatrix {
    /// `1.0` in 16.16 fixed point.
    const ONE_16: i32 = 1 << 16;
    /// `1.0` in 2.30 fixed point.
    const ONE_30: i32 = 1 << 30;

    /// The identity (unity) matrix.
    pub const IDENTITY: Self = Self {
        m: [Self::ONE_16, 0, 0, 0, Self::ONE_16, 0, 0, 0, Self::ONE_30],
    };

    /// Wrap the nine stored values, in the order `{a, b, u, c, d, v, x,
    /// y, w}`: 16.16 fixed point except `u`, `v`, `w` (2.30).
    pub const fn from_fixed(values: [i32; 9]) -> Self {
        Self { m: values }
    }

    /// The nine stored values, in the order `{a, b, u, c, d, v, x, y,
    /// w}`, ready to be written back.
    pub const fn to_fixed(self) -> [i32; 9] {
        self.m
    }

    /// The matrix for `orientation` with no translation: the picture is
    /// rotated / mirrored about the origin. Use
    /// [`from_orientation_in`](Self::from_orientation_in) when the
    /// result must stay in the positive quadrant.
    pub const fn from_orientation(orientation: DisplayOrientation) -> Self {
        let [a, b, c, d] = orientation.linear();
        Self {
            m: [
                a * Self::ONE_16,
                b * Self::ONE_16,
                0,
                c * Self::ONE_16,
                d * Self::ONE_16,
                0,
                0,
                0,
                Self::ONE_30,
            ],
        }
    }

    /// The matrix for `orientation` applied to a `width` × `height`
    /// picture, with the translation that puts the upper-left corner of
    /// the transformed picture back at the origin. Dimensions above
    /// 32767 saturate the 16.16 translation.
    ///
    /// ```
    /// # use oxideav_core::geometry::{DisplayMatrix, DisplayOrientation};
    /// let m = DisplayMatrix::from_orientation_in(DisplayOrientation::rotated_cw(1), 1920, 1080);
    /// // The decoded top-left corner lands at the top-right of a 1080×1920 surface.
    /// assert_eq!(m.apply(0.0, 0.0), (1080.0, 0.0));
    /// assert_eq!(m.apply(0.0, 1080.0), (0.0, 0.0));
    /// assert_eq!(m.orientation(), Some(DisplayOrientation::rotated_cw(1)));
    /// ```
    pub const fn from_orientation_in(
        orientation: DisplayOrientation,
        width: u32,
        height: u32,
    ) -> Self {
        const fn fixed(pixels: u32) -> i32 {
            if pixels > 32767 {
                i32::MAX
            } else {
                (pixels as i32) << 16
            }
        }
        let [a, b, c, d] = orientation.linear();
        // Each output axis receives exactly one negative or positive
        // unit term; a negative one needs a shift by that extent.
        let x = if a < 0 {
            fixed(width)
        } else if c < 0 {
            fixed(height)
        } else {
            0
        };
        let y = if b < 0 {
            fixed(width)
        } else if d < 0 {
            fixed(height)
        } else {
            0
        };
        let mut out = Self::from_orientation(orientation);
        out.m[6] = x;
        out.m[7] = y;
        out
    }

    /// `true` when every value equals the identity matrix's.
    pub const fn is_identity(self) -> bool {
        let i = Self::IDENTITY.m;
        let mut k = 0;
        while k < 9 {
            if self.m[k] != i[k] {
                return false;
            }
            k += 1;
        }
        true
    }

    /// `true` when the projective row is `(0, 0, 1)` — the restriction
    /// the track and movie headers place on video matrices.
    pub const fn is_affine(self) -> bool {
        self.m[2] == 0 && self.m[5] == 0 && self.m[8] == Self::ONE_30
    }

    /// The linear part `[a, b, c, d]` as real numbers.
    pub fn linear(self) -> [f64; 4] {
        let s = f64::from(Self::ONE_16);
        [
            f64::from(self.m[0]) / s,
            f64::from(self.m[1]) / s,
            f64::from(self.m[3]) / s,
            f64::from(self.m[4]) / s,
        ]
    }

    /// The translation `(x, y)` in pixels.
    pub fn translation(self) -> (f64, f64) {
        let s = f64::from(Self::ONE_16);
        (f64::from(self.m[6]) / s, f64::from(self.m[7]) / s)
    }

    /// Map the point `(p, q)` of the decoded frame to the rendering
    /// surface. A degenerate projective term (`z == 0`) yields
    /// non-finite coordinates.
    pub fn apply(self, p: f64, q: f64) -> (f64, f64) {
        let [a, b, c, d] = self.linear();
        let (x, y) = self.translation();
        let s = f64::from(Self::ONE_30);
        let z =
            f64::from(self.m[2]) / s * p + f64::from(self.m[5]) / s * q + f64::from(self.m[8]) / s;
        ((a * p + c * q + x) / z, (b * p + d * q + y) / z)
    }

    /// The clockwise rotation angle of the X axis in degrees, in
    /// `(-180, 180]`: `atan2(b, a)`. Meaningful for any matrix whose
    /// linear part is a rotation, with or without scaling; use
    /// [`orientation`](Self::orientation) for the exact quarter-turn
    /// classification (which also detects mirrors).
    pub fn rotation_degrees_cw(self) -> f64 {
        let [a, b, _, _] = self.linear();
        b.atan2(a).to_degrees()
    }

    /// The [`DisplayOrientation`] this matrix reduces to: `Some` when
    /// it is affine and its linear part is exactly one of the eight
    /// unit quarter-turn / mirror matrices (the translation is
    /// ignored), `None` for anything else (scaling, shear, arbitrary
    /// angles, projective terms).
    pub fn orientation(self) -> Option<DisplayOrientation> {
        if !self.is_affine() {
            return None;
        }
        let unit = |v: i32| match v {
            0 => Some(0),
            v if v == Self::ONE_16 => Some(1),
            v if v == -Self::ONE_16 => Some(-1),
            _ => None,
        };
        let got = [
            unit(self.m[0])?,
            unit(self.m[1])?,
            unit(self.m[3])?,
            unit(self.m[4])?,
        ];
        (0..8u8)
            .map(|k| DisplayOrientation::new(k % 4, k >= 4))
            .find(|o| o.linear() == got)
    }
}

impl From<DisplayOrientation> for DisplayMatrix {
    fn from(o: DisplayOrientation) -> Self {
        Self::from_orientation(o)
    }
}

/// How a video stream's sample grid maps to a display: pixel aspect
/// ratio, clean aperture, field order and display matrix.
///
/// Every part is optional. `Default` / [`unspecified`](Self::unspecified)
/// leaves all four open, which is what
/// [`CodecParameters`](crate::CodecParameters) starts with.
///
/// `#[non_exhaustive]`: build it with [`new`](Self::new) and the
/// `with_*` builders; read the public fields directly.
///
/// ```
/// # use oxideav_core::Rational;
/// # use oxideav_core::geometry::{CleanAperture, DisplayOrientation, FieldOrder, VideoGeometry};
/// // Anamorphic 720×576 interlaced video, shown as 1024×576.
/// let g = VideoGeometry::new()
///     .with_pixel_aspect(Rational::new(64, 45))
///     .with_field_order(FieldOrder::TopFirst);
/// assert_eq!(g.display_size(720, 576), Some((1024, 576)));
///
/// // A phone clip recorded sideways.
/// let g = VideoGeometry::new().with_orientation(DisplayOrientation::rotated_cw(1));
/// assert_eq!(g.display_size(1920, 1080), Some((1080, 1920)));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct VideoGeometry {
    /// Pixel (sample) aspect ratio: the width of one sample divided by
    /// its height, as `hSpacing / vSpacing` of §12.1.4. `1/1` is a
    /// square pixel; `None` is unspecified (consumers assume square).
    /// Both terms are positive and need not be reduced.
    pub pixel_aspect: Option<Rational>,
    /// The displayed sub-rectangle of the coded picture. `None` means
    /// the whole picture.
    pub clean_aperture: Option<CleanAperture>,
    /// Scan structure. [`FieldOrder::Unspecified`] when unknown.
    pub field_order: FieldOrder,
    /// Transformation from the decoded frame to the rendering surface.
    /// `None` is unspecified (consumers treat it as the identity).
    pub display_matrix: Option<DisplayMatrix>,
}

impl VideoGeometry {
    /// A geometry with every part unspecified — same as `Default`.
    pub const fn new() -> Self {
        Self {
            pixel_aspect: None,
            clean_aperture: None,
            field_order: FieldOrder::Unspecified,
            display_matrix: None,
        }
    }

    /// A geometry with every part unspecified — same as
    /// [`new`](Self::new).
    pub const fn unspecified() -> Self {
        Self::new()
    }

    /// `true` when no part is specified.
    pub const fn is_unspecified(&self) -> bool {
        self.pixel_aspect.is_none()
            && self.clean_aperture.is_none()
            && self.field_order.is_unspecified()
            && self.display_matrix.is_none()
    }

    /// Builder: set the pixel aspect ratio (sample width / height).
    pub const fn with_pixel_aspect(mut self, ratio: Rational) -> Self {
        self.pixel_aspect = Some(ratio);
        self
    }

    /// Builder: set the clean aperture.
    pub const fn with_clean_aperture(mut self, aperture: CleanAperture) -> Self {
        self.clean_aperture = Some(aperture);
        self
    }

    /// Builder: set the field order.
    pub const fn with_field_order(mut self, order: FieldOrder) -> Self {
        self.field_order = order;
        self
    }

    /// Builder: set the display matrix.
    pub const fn with_display_matrix(mut self, matrix: DisplayMatrix) -> Self {
        self.display_matrix = Some(matrix);
        self
    }

    /// Builder: set the display matrix to the untranslated matrix of
    /// `orientation` ([`DisplayMatrix::from_orientation`]).
    pub const fn with_orientation(mut self, orientation: DisplayOrientation) -> Self {
        self.display_matrix = Some(DisplayMatrix::from_orientation(orientation));
        self
    }

    /// Field-wise merge: each part of `self` that is specified wins,
    /// each part that is not is taken from `fallback`.
    pub fn or(self, fallback: Self) -> Self {
        Self {
            pixel_aspect: self.pixel_aspect.or(fallback.pixel_aspect),
            clean_aperture: self.clean_aperture.or(fallback.clean_aperture),
            field_order: if self.field_order.is_unspecified() {
                fallback.field_order
            } else {
                self.field_order
            },
            display_matrix: self.display_matrix.or(fallback.display_matrix),
        }
    }

    /// The pixel aspect ratio when it is specified with two positive
    /// terms; `None` when unspecified or malformed.
    pub fn valid_pixel_aspect(&self) -> Option<Rational> {
        self.pixel_aspect.filter(|r| r.num > 0 && r.den > 0)
    }

    /// `true` when the pixel aspect ratio is unspecified, malformed or
    /// equal to `1/1` in value.
    pub fn has_square_pixels(&self) -> bool {
        self.valid_pixel_aspect().is_none_or(|r| r.num == r.den)
    }

    /// The display orientation: the identity when no matrix is set,
    /// the matrix's [`orientation`](DisplayMatrix::orientation) when it
    /// reduces to one, `None` when the matrix is something else.
    pub fn orientation(&self) -> Option<DisplayOrientation> {
        match self.display_matrix {
            None => Some(DisplayOrientation::IDENTITY),
            Some(m) => m.orientation(),
        }
    }

    /// The size, in square display pixels, at which a coded picture of
    /// `coded_width` × `coded_height` samples is shown: the clean
    /// aperture is taken out, the width is scaled by the pixel aspect
    /// ratio (the horizontal dimension is the one adjusted), and the
    /// result is turned by the display orientation. Fractional sizes
    /// round to the nearest pixel.
    ///
    /// `None` when the clean aperture is malformed, the display matrix
    /// does not reduce to a quarter-turn orientation, or the result
    /// does not fit a `u32`.
    pub fn display_size(&self, coded_width: u32, coded_height: u32) -> Option<(u32, u32)> {
        let (mut w, h) = match self.clean_aperture {
            Some(clap) => {
                let r = clap.rect_in(coded_width, coded_height)?;
                (r.width, r.height)
            }
            None => (f64::from(coded_width), f64::from(coded_height)),
        };
        if let Some(par) = self.valid_pixel_aspect() {
            w *= par.as_f64();
        }
        let orientation = self.orientation()?;
        let fit = |v: f64| {
            let r = v.round();
            (r >= 0.0 && r <= f64::from(u32::MAX)).then_some(r as u32)
        };
        Some(orientation.output_size(fit(w)?, fit(h)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_unspecified() {
        let g = VideoGeometry::default();
        assert!(g.is_unspecified());
        assert_eq!(g, VideoGeometry::new());
        assert_eq!(g, VideoGeometry::unspecified());
        assert_eq!(g.field_order, FieldOrder::Unspecified);
        assert!(g.has_square_pixels());
        assert_eq!(g.orientation(), Some(DisplayOrientation::IDENTITY));
        assert_eq!(g.display_size(640, 480), Some((640, 480)));
        assert_eq!(DisplayMatrix::default(), DisplayMatrix::IDENTITY);
        assert!(DisplayMatrix::IDENTITY.is_identity());
        assert!(DisplayOrientation::default().is_identity());
    }

    #[test]
    fn builders_set_each_part_and_clear_unspecified() {
        let clap = CleanAperture::centered(704, 480);
        let g = VideoGeometry::new()
            .with_pixel_aspect(Rational::new(10, 11))
            .with_clean_aperture(clap)
            .with_field_order(FieldOrder::BottomFirst)
            .with_display_matrix(DisplayMatrix::IDENTITY);
        assert!(!g.is_unspecified());
        assert_eq!(g.pixel_aspect, Some(Rational::new(10, 11)));
        assert_eq!(g.clean_aperture, Some(clap));
        assert_eq!(g.field_order, FieldOrder::BottomFirst);
        assert_eq!(g.display_matrix, Some(DisplayMatrix::IDENTITY));
        assert!(!g.has_square_pixels());
        // 704 * 10/11 = 640.
        assert_eq!(g.display_size(720, 480), Some((640, 480)));
        for part in [
            VideoGeometry::new().with_pixel_aspect(Rational::new(1, 1)),
            VideoGeometry::new().with_clean_aperture(clap),
            VideoGeometry::new().with_field_order(FieldOrder::Progressive),
            VideoGeometry::new().with_orientation(DisplayOrientation::IDENTITY),
        ] {
            assert!(!part.is_unspecified());
        }
    }

    #[test]
    fn or_merges_field_wise() {
        let container = VideoGeometry::new()
            .with_pixel_aspect(Rational::new(4, 3))
            .with_orientation(DisplayOrientation::rotated_cw(1))
            .with_field_order(FieldOrder::TopFirst);
        let decoder = VideoGeometry::new()
            .with_pixel_aspect(Rational::new(16, 11))
            .with_clean_aperture(CleanAperture::centered(8, 8));
        let merged = decoder.or(container);
        assert_eq!(merged.pixel_aspect, Some(Rational::new(16, 11)));
        assert_eq!(merged.clean_aperture, Some(CleanAperture::centered(8, 8)));
        assert_eq!(merged.field_order, FieldOrder::TopFirst);
        assert_eq!(
            merged.orientation(),
            Some(DisplayOrientation::rotated_cw(1))
        );
        assert_eq!(VideoGeometry::new().or(container), container);
        assert_eq!(container.or(VideoGeometry::new()), container);
    }

    #[test]
    fn malformed_pixel_aspect_is_ignored() {
        for bad in [
            Rational::new(0, 1),
            Rational::new(1, 0),
            Rational::new(-4, 3),
        ] {
            let g = VideoGeometry::new().with_pixel_aspect(bad);
            assert_eq!(g.valid_pixel_aspect(), None);
            assert!(g.has_square_pixels());
            assert_eq!(g.display_size(100, 50), Some((100, 50)));
        }
        // Unreduced but square.
        assert!(
            VideoGeometry::new()
                .with_pixel_aspect(Rational::new(3, 3))
                .has_square_pixels()
        );
    }

    #[test]
    fn field_order_predicates_and_components() {
        use FieldOrder::*;
        assert!(Unspecified.is_unspecified());
        assert!(!Unspecified.is_interlaced() && !Unspecified.is_progressive());
        assert!(Progressive.is_progressive() && !Progressive.is_interlaced());
        for o in [
            TopFirst,
            BottomFirst,
            TopDisplayedBottomStored,
            BottomDisplayedTopStored,
            Interlaced,
        ] {
            assert!(o.is_interlaced() && !o.is_progressive() && !o.is_unspecified());
        }
        assert_eq!(Interlaced.first_displayed(), None);
        assert_eq!(Progressive.first_stored(), None);
        for d in [Field::Top, Field::Bottom] {
            for s in [Field::Top, Field::Bottom] {
                let o = FieldOrder::from_fields(d, s);
                assert_eq!(o.first_displayed(), Some(d));
                assert_eq!(o.first_stored(), Some(s));
            }
        }
        assert_eq!(Field::Top.opposite(), Field::Bottom);
        assert_eq!(Field::Bottom.opposite(), Field::Top);
    }

    #[test]
    fn field_atom_codes_round_trip() {
        use FieldOrder::*;
        // QuickTime `fiel`: 1 = T displayed + stored first, 6 = B / B,
        // 9 = B displayed, T stored, 14 = T displayed, B stored.
        assert_eq!(FieldOrder::from_field_atom(2, 1), TopFirst);
        assert_eq!(FieldOrder::from_field_atom(2, 6), BottomFirst);
        assert_eq!(FieldOrder::from_field_atom(2, 9), BottomDisplayedTopStored);
        assert_eq!(FieldOrder::from_field_atom(2, 14), TopDisplayedBottomStored);
        assert_eq!(FieldOrder::from_field_atom(2, 0), Interlaced);
        assert_eq!(FieldOrder::from_field_atom(2, 77), Interlaced);
        assert_eq!(FieldOrder::from_field_atom(1, 0), Progressive);
        assert_eq!(FieldOrder::from_field_atom(0, 0), Unspecified);
        assert_eq!(FieldOrder::from_field_atom(3, 1), Unspecified);
        assert_eq!(Unspecified.to_field_atom(), None);
        for o in [
            Progressive,
            TopFirst,
            BottomFirst,
            TopDisplayedBottomStored,
            BottomDisplayedTopStored,
            Interlaced,
        ] {
            let (count, ordering) = o.to_field_atom().unwrap();
            assert_eq!(FieldOrder::from_field_atom(count, ordering), o);
        }
    }

    #[test]
    fn clean_aperture_centre_offset_arithmetic() {
        // §12.1.4: pcX = horizOff + (W-1)/2; edges at pcX ± (cw-1)/2.
        let clap = CleanAperture::new(
            Rational::new(1440, 2),
            Rational::new(480, 1),
            Rational::new(-3, 2),
            Rational::new(2, 1),
        );
        assert!(clap.is_valid());
        let r = clap.rect_in(728, 486).unwrap();
        assert_eq!(r.width, 720.0);
        assert_eq!(r.height, 480.0);
        // left = -1.5 + (728 - 720)/2 = 2.5; top = 2 + (486-480)/2 = 5.
        assert_eq!(r.left, 2.5);
        assert_eq!(r.top, 5.0);
        assert_eq!(r.to_integer(), None);
        // Leftmost sample centre: pcX - (cw-1)/2 = left.
        let pc_x = clap.horiz_off.as_f64() + (728.0 - 1.0) / 2.0;
        assert_eq!(pc_x - (r.width - 1.0) / 2.0, r.left);
    }

    #[test]
    fn clean_aperture_from_crop_round_trips() {
        for (cw, ch, l, t, w, h) in [
            (1920u32, 1088u32, 0u32, 0u32, 1920u32, 1080u32),
            (720, 486, 8, 3, 704, 480),
            (65, 33, 1, 0, 63, 32),
            (16, 16, 0, 0, 16, 16),
            (100, 100, 99, 99, 1, 1),
        ] {
            let clap = CleanAperture::from_crop(cw, ch, l, t, w, h);
            assert!(clap.is_valid());
            let r = clap.rect_in(cw, ch).unwrap();
            assert_eq!(r.to_integer(), Some((l, t, w, h)), "{cw}x{ch}");
        }
        let full = CleanAperture::from_crop(16, 16, 0, 0, 16, 16);
        assert_eq!(full, CleanAperture::centered(16, 16));
    }

    #[test]
    fn invalid_clean_aperture_resolves_to_none() {
        let zero = Rational::new(0, 1);
        let one = Rational::new(1, 1);
        for bad in [
            CleanAperture::new(Rational::new(0, 1), one, zero, zero),
            CleanAperture::new(one, Rational::new(-2, 1), zero, zero),
            CleanAperture::new(Rational::new(1, 0), one, zero, zero),
            CleanAperture::new(one, one, Rational::new(1, 0), zero),
            CleanAperture::new(one, one, zero, Rational::new(1, 0)),
        ] {
            assert!(!bad.is_valid());
            assert!(bad.rect_in(10, 10).is_none());
            assert!(
                VideoGeometry::new()
                    .with_clean_aperture(bad)
                    .display_size(10, 10)
                    .is_none()
            );
        }
    }

    fn all_orientations() -> Vec<DisplayOrientation> {
        (0..8u8)
            .map(|k| DisplayOrientation::new(k % 4, k >= 4))
            .collect()
    }

    /// 2×2 product in the `[a, b, c, d]` layout: `x` applied after `y`.
    fn mul(x: [i32; 4], y: [i32; 4]) -> [i32; 4] {
        [
            x[0] * y[0] + x[2] * y[1],
            x[1] * y[0] + x[3] * y[1],
            x[0] * y[2] + x[2] * y[3],
            x[1] * y[2] + x[3] * y[3],
        ]
    }

    #[test]
    fn orientation_group_laws_match_matrices() {
        let all = all_orientations();
        for &a in &all {
            assert_eq!(a.then(a.inverse()), DisplayOrientation::IDENTITY);
            assert_eq!(a.inverse().then(a), DisplayOrientation::IDENTITY);
            for &b in &all {
                assert_eq!(
                    a.then(b).linear(),
                    mul(b.linear(), a.linear()),
                    "{a:?} then {b:?}"
                );
            }
        }
        // All eight are distinct.
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.linear(), b.linear());
            }
        }
    }

    #[test]
    fn orientation_constructors_and_accessors() {
        assert_eq!(
            DisplayOrientation::new(5, false),
            DisplayOrientation::rotated_cw(1)
        );
        assert_eq!(
            DisplayOrientation::from_degrees_cw(270),
            Some(DisplayOrientation::rotated_cw(3))
        );
        assert_eq!(
            DisplayOrientation::from_degrees_cw(-90),
            Some(DisplayOrientation::rotated_cw(3))
        );
        assert_eq!(
            DisplayOrientation::from_degrees_cw(360),
            Some(DisplayOrientation::IDENTITY)
        );
        assert_eq!(DisplayOrientation::from_degrees_cw(45), None);
        let o = DisplayOrientation::new(3, true);
        assert_eq!(o.quarter_turns_cw(), 3);
        assert_eq!(o.degrees_cw(), 270);
        assert!(o.is_mirrored());
        assert!(o.swaps_dimensions());
        assert_eq!(o.output_size(4, 3), (3, 4));
        assert_eq!(DisplayOrientation::rotated_cw(2).output_size(4, 3), (4, 3));
        // A vertical flip maps (p, q) to (p, -q).
        assert_eq!(
            DisplayOrientation::flipped_vertically().linear(),
            [1, 0, 0, -1]
        );
        assert_eq!(
            DisplayOrientation::mirrored_horizontally().linear(),
            [-1, 0, 0, 1]
        );
        // Mirror then half turn == vertical flip.
        assert_eq!(
            DisplayOrientation::mirrored_horizontally().then(DisplayOrientation::rotated_cw(2)),
            DisplayOrientation::flipped_vertically()
        );
    }

    #[test]
    fn matrix_orientation_round_trips_for_all_eight() {
        for o in all_orientations() {
            let m = DisplayMatrix::from_orientation(o);
            assert!(m.is_affine());
            assert_eq!(m.orientation(), Some(o));
            assert_eq!(DisplayMatrix::from(o), m);
            assert_eq!(DisplayMatrix::from_fixed(m.to_fixed()), m);
            // The translated form classifies the same and keeps the
            // whole picture in the positive quadrant.
            let (w, h) = (640u32, 360u32);
            let t = DisplayMatrix::from_orientation_in(o, w, h);
            assert_eq!(t.orientation(), Some(o));
            let (ow, oh) = o.output_size(w, h);
            let corners = [
                t.apply(0.0, 0.0),
                t.apply(f64::from(w), 0.0),
                t.apply(0.0, f64::from(h)),
                t.apply(f64::from(w), f64::from(h)),
            ];
            let min_x = corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
            let min_y = corners.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
            let max_x = corners
                .iter()
                .map(|c| c.0)
                .fold(f64::NEG_INFINITY, f64::max);
            let max_y = corners
                .iter()
                .map(|c| c.1)
                .fold(f64::NEG_INFINITY, f64::max);
            assert_eq!((min_x, min_y), (0.0, 0.0), "{o:?}");
            assert_eq!((max_x, max_y), (f64::from(ow), f64::from(oh)), "{o:?}");
        }
        assert_eq!(
            DisplayMatrix::IDENTITY.orientation(),
            Some(DisplayOrientation::IDENTITY)
        );
        assert_eq!(
            DisplayMatrix::from_orientation(DisplayOrientation::IDENTITY),
            DisplayMatrix::IDENTITY
        );
    }

    #[test]
    fn quarter_turn_matrix_matches_the_stored_layout() {
        // Clockwise 90° with Y down sends (p, q) to (-q, p): a = 0,
        // b = 1, c = -1, d = 0, stored as {a, b, u, c, d, v, x, y, w}.
        let m = DisplayMatrix::from_orientation(DisplayOrientation::rotated_cw(1));
        assert_eq!(
            m.to_fixed(),
            [0, 0x1_0000, 0, -0x1_0000, 0, 0, 0, 0, 0x4000_0000]
        );
        assert_eq!(m.apply(1.0, 0.0), (0.0, 1.0));
        assert_eq!(m.apply(0.0, 1.0), (-1.0, 0.0));
        assert_eq!(m.rotation_degrees_cw(), 90.0);
        assert_eq!(DisplayMatrix::IDENTITY.rotation_degrees_cw(), 0.0);
        assert_eq!(
            DisplayMatrix::from_orientation(DisplayOrientation::rotated_cw(2))
                .rotation_degrees_cw(),
            180.0
        );
        assert_eq!(
            DisplayMatrix::from_orientation(DisplayOrientation::rotated_cw(3))
                .rotation_degrees_cw(),
            -90.0
        );
    }

    #[test]
    fn general_matrices_do_not_reduce() {
        // The standard's own example: {2,0,0, 0,2,0, 0,0,1} doubles the
        // pixel dimensions.
        let double = DisplayMatrix::from_fixed([0x2_0000, 0, 0, 0, 0x2_0000, 0, 0, 0, 0x4000_0000]);
        assert_eq!(double.apply(3.0, 5.0), (6.0, 10.0));
        assert_eq!(double.orientation(), None);
        assert!(!double.is_identity());
        assert_eq!(double.linear(), [2.0, 0.0, 0.0, 2.0]);
        // Translation only: still the identity orientation.
        let shifted = DisplayMatrix::from_fixed([
            0x1_0000,
            0,
            0,
            0,
            0x1_0000,
            0,
            20 << 16,
            30 << 16,
            0x4000_0000,
        ]);
        assert_eq!(shifted.translation(), (20.0, 30.0));
        assert_eq!(shifted.apply(1.0, 1.0), (21.0, 31.0));
        assert_eq!(shifted.orientation(), Some(DisplayOrientation::IDENTITY));
        assert!(!shifted.is_identity());
        // A projective term disqualifies it.
        let mut p = DisplayMatrix::IDENTITY.to_fixed();
        p[2] = 1;
        assert!(!DisplayMatrix::from_fixed(p).is_affine());
        assert_eq!(DisplayMatrix::from_fixed(p).orientation(), None);
        // A singular linear part is not an orientation either.
        let singular = DisplayMatrix::from_fixed([0x1_0000, 0, 0, 0x1_0000, 0, 0, 0, 0, 1 << 30]);
        assert_eq!(singular.orientation(), None);
        let g = VideoGeometry::new().with_display_matrix(double);
        assert_eq!(g.orientation(), None);
        assert_eq!(g.display_size(10, 10), None);
    }

    #[test]
    fn oversized_translation_saturates() {
        let m =
            DisplayMatrix::from_orientation_in(DisplayOrientation::rotated_cw(2), 40_000, 20_000);
        let v = m.to_fixed();
        assert_eq!(v[6], i32::MAX);
        assert_eq!(v[7], 20_000 << 16);
    }

    #[test]
    fn display_size_combines_aperture_aspect_and_turn() {
        let g = VideoGeometry::new()
            .with_clean_aperture(CleanAperture::from_crop(720, 576, 8, 0, 704, 576))
            .with_pixel_aspect(Rational::new(16, 11))
            .with_orientation(DisplayOrientation::rotated_cw(3));
        // 704 * 16/11 = 1024, then a quarter turn swaps.
        assert_eq!(g.display_size(720, 576), Some((576, 1024)));
        // Rounds to nearest: 720 * 10/11 = 654.54…
        let g = VideoGeometry::new().with_pixel_aspect(Rational::new(10, 11));
        assert_eq!(g.display_size(720, 480), Some((655, 480)));
    }
}

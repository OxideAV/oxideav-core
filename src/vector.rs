//! Vector graphics frame and primitive types.
//!
//! This module models a resolution-independent, scene-graph-style vector
//! frame so the same [`VectorFrame`] can round-trip through both SVG 1.1
//! and PDF 1.4 without lossy conversion. The primitive set is the
//! intersection of what those two formats represent natively:
//!
//! * paths built from move / line / quadratic / cubic / elliptic-arc / close
//!   commands,
//! * solid + linear-gradient + radial-gradient paints,
//! * stroke style (width, cap, join, miter limit, dash),
//! * even-odd / non-zero fill rules,
//! * 2D affine transforms,
//! * group nodes (transform, opacity, optional clip),
//! * embedded raster passthrough via [`ImageRef`] (carries a child
//!   [`VideoFrame`](crate::VideoFrame) — the rasterizer paints the image
//!   into vector space).
//!
//! Text nodes are intentionally **deferred to round 2** — text needs
//! font handling and tight scribe coupling that will land alongside the
//! `oxideav-svg` parser (#349). Round 1 is shape-only.
//!
//! No rasterizer / SVG parser / PDF writer lives in `oxideav-core`; those
//! are downstream tasks (#349 / #350 / #351). This module ships only the
//! data types every consumer of the vector pipeline needs to agree on.

use crate::time::TimeBase;

/// A decoded vector-graphics frame.
///
/// The `width` / `height` define the natural rendering canvas size in
/// user units. `view_box` lets a producer separate the user-coordinate
/// system from the canvas (an SVG `viewBox` attribute, or the PDF
/// `MediaBox` vs. `CropBox`); when `None`, callers should treat it as
/// `(0, 0, width, height)`.
#[derive(Clone, Debug)]
pub struct VectorFrame {
    /// Viewport width in user units.
    pub width: f32,
    /// Viewport height in user units.
    pub height: f32,
    /// Optional view box. `None` defaults to `(0, 0, width, height)`.
    pub view_box: Option<ViewBox>,
    /// Root group of the scene.
    pub root: Group,
    /// Presentation timestamp in `time_base` units, or `None` if unknown.
    pub pts: Option<i64>,
    /// Time base for `pts`. Consumers that don't care about timing
    /// (e.g. a one-shot SVG render) can use `TimeBase::new(1, 1)`.
    pub time_base: TimeBase,
}

/// User-coordinate system rectangle. Mirrors the SVG `viewBox` attribute
/// and the PDF `MediaBox` / `CropBox` rectangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewBox {
    pub min_x: f32,
    pub min_y: f32,
    pub width: f32,
    pub height: f32,
}

/// One node in the scene tree.
///
/// Marked `#[non_exhaustive]` so future variants (text, filters) can
/// be added without breaking downstream `match` arms.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Node {
    Path(PathNode),
    Group(Group),
    /// An embedded raster image painted into vector space.
    Image(ImageRef),
    /// A soft-mask composite. The `mask` subtree is rasterised and
    /// converted to a per-pixel alpha multiplier (luminance or alpha,
    /// per [`MaskKind`]), then applied to the rasterised `content`
    /// subtree. Mirrors SVG `<mask>` and PDF `SMask` (subtype `Luminosity`
    /// vs. `Alpha`).
    SoftMask {
        /// Subtree rasterised to produce the per-pixel opacity
        /// modulator.
        mask: Box<Node>,
        /// How to convert the rasterised mask to a coverage value.
        mask_kind: MaskKind,
        /// Subtree whose pixels are modulated by the mask.
        content: Box<Node>,
    },
}

/// How to interpret a soft mask's rasterised pixels as a coverage
/// modulator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaskKind {
    /// Convert the mask's RGB to luminance (ITU-R BT.709 coefficients
    /// — Y = 0.2126·R + 0.7152·G + 0.0722·B) and use Y as the
    /// per-pixel alpha multiplier. Matches SVG `<mask>` default
    /// (`mask-type="luminance"`) and PDF `SMask` `/Luminosity`.
    #[default]
    Luminance,
    /// Use the mask's own alpha channel as the multiplier. Matches
    /// SVG `<mask mask-type="alpha">` and PDF `SMask` `/Alpha`.
    Alpha,
}

/// A grouping node — applies a transform / opacity / optional clip path
/// to all descendants. Mirrors SVG `<g>` and PDF `q ... Q` graphic-state
/// blocks.
#[derive(Clone, Debug)]
pub struct Group {
    /// Coordinate transform applied to children. Identity by default.
    pub transform: Transform2D,
    /// Group opacity in `0.0..=1.0`. `1.0` is fully opaque.
    pub opacity: f32,
    /// Optional clip path. Children are clipped to this path's interior
    /// (using the path's own fill rule). `None` means "no clip".
    pub clip: Option<Path>,
    pub children: Vec<Node>,
    /// Opaque cache key. When `Some(k)`, a downstream rasterizer is free
    /// to memoise the rendered bitmap of this group's content (after
    /// `transform` is applied) under key `k`, so re-rendering the same
    /// group at the same effective resolution returns the cached bitmap.
    ///
    /// Producers that emit cacheable content (e.g. scribe shaping a
    /// glyph at `(face_id, glyph_id, size_q8, subpixel_x)`) compute a
    /// deterministic hash of their identity tuple and put it here. The
    /// rasterizer treats it as a black box — `oxideav-core` never
    /// inspects the value, so each producer's namespace stays private.
    ///
    /// `None` (the default) means "do not cache; render fresh every
    /// time". Most synthesised vector content (a one-off rectangle, a
    /// gradient panel) leaves this `None`.
    pub cache_key: Option<u64>,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            transform: Transform2D::identity(),
            opacity: 1.0,
            clip: None,
            children: Vec::new(),
            cache_key: None,
        }
    }
}

/// A drawn path with optional fill and stroke.
///
/// SVG `<path>` and PDF path-painting operators (`f`, `S`, `B`, `f*`,
/// `B*`) both express "one path, optional fill, optional stroke", so a
/// single struct covers both formats. At least one of `fill` / `stroke`
/// would normally be `Some` to produce visible output.
#[derive(Clone, Debug)]
pub struct PathNode {
    pub path: Path,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
    pub fill_rule: FillRule,
}

/// A geometric path expressed as a sequence of drawing commands.
///
/// All coordinates are in the local user space of the enclosing group.
#[derive(Clone, Debug, Default)]
pub struct Path {
    pub commands: Vec<PathCommand>,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(&mut self, p: Point) -> &mut Self {
        self.commands.push(PathCommand::MoveTo(p));
        self
    }

    pub fn line_to(&mut self, p: Point) -> &mut Self {
        self.commands.push(PathCommand::LineTo(p));
        self
    }

    pub fn quad_to(&mut self, control: Point, end: Point) -> &mut Self {
        self.commands
            .push(PathCommand::QuadCurveTo { control, end });
        self
    }

    pub fn cubic_to(&mut self, c1: Point, c2: Point, end: Point) -> &mut Self {
        self.commands
            .push(PathCommand::CubicCurveTo { c1, c2, end });
        self
    }

    pub fn close(&mut self) -> &mut Self {
        self.commands.push(PathCommand::Close);
        self
    }
}

/// A single path-construction command.
///
/// Marked `#[non_exhaustive]` so smooth-curve / Bezier-shorthand
/// variants can be added later without breaking match arms.
///
/// Note on `ArcTo`: SVG and PDF both accept elliptic-arc segments in
/// their path syntax (SVG `A` command, PDF via cubic approximation in
/// the writer). We keep the variant in the round-1 IR — converting an
/// arc to its spec-correct cubic-Bezier flattening is a pure function
/// of the arc parameters that downstream rasterizers / writers can do
/// independently.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PathCommand {
    MoveTo(Point),
    LineTo(Point),
    QuadCurveTo {
        control: Point,
        end: Point,
    },
    CubicCurveTo {
        c1: Point,
        c2: Point,
        end: Point,
    },
    /// SVG `A`-style elliptic arc segment. `x_axis_rot` is in radians
    /// (consistent with `Transform2D::rotate`); `large_arc` / `sweep`
    /// match the SVG flag semantics.
    ArcTo {
        rx: f32,
        ry: f32,
        x_axis_rot: f32,
        large_arc: bool,
        sweep: bool,
        end: Point,
    },
    Close,
}

/// 2D point in user-space coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A paint server — what fills the inside of a path or strokes its
/// outline. The variant set is the SVG/PDF intersection.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Paint {
    Solid(Rgba),
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
}

/// 32-bit straight (non-premultiplied) RGBA color.
///
/// Matches SVG's `rgb()` + `opacity` model and PDF's `RGB` + `CA`/`ca`
/// graphic-state model. Premultiplication is a rasterizer concern; this
/// IR carries straight alpha to avoid lossy round-trips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Fully-opaque color with the given RGB triple.
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

/// A linear gradient: color stops sweep along the line `start` → `end`.
#[derive(Clone, Debug)]
pub struct LinearGradient {
    pub start: Point,
    pub end: Point,
    pub stops: Vec<GradientStop>,
    pub spread: SpreadMethod,
}

/// A radial gradient: color stops sweep from `focal` outward to a
/// circle of radius `radius` centered on `center`. When `focal` is
/// `None`, it defaults to `center` (the common case).
#[derive(Clone, Debug)]
pub struct RadialGradient {
    pub center: Point,
    pub radius: f32,
    pub focal: Option<Point>,
    pub stops: Vec<GradientStop>,
    pub spread: SpreadMethod,
}

/// One color stop along a gradient. `offset` is in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    /// Position of the stop along the gradient axis. `0.0` is the
    /// start, `1.0` is the end.
    pub offset: f32,
    pub color: Rgba,
}

impl GradientStop {
    pub const fn new(offset: f32, color: Rgba) -> Self {
        Self { offset, color }
    }
}

/// What happens past the gradient endpoints. Mirrors SVG
/// `spreadMethod="pad|reflect|repeat"` and PDF gradient `Extend` arrays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpreadMethod {
    /// Final stop colors extend forever. SVG default.
    #[default]
    Pad,
    /// Gradient mirrors at each boundary.
    Reflect,
    /// Gradient repeats periodically.
    Repeat,
}

/// Stroke style for a path's outline.
#[derive(Clone, Debug)]
pub struct Stroke {
    pub width: f32,
    pub paint: Paint,
    pub cap: LineCap,
    pub join: LineJoin,
    /// Miter limit ratio. SVG / PDF default is `4.0`.
    pub miter_limit: f32,
    pub dash: Option<DashPattern>,
}

impl Stroke {
    /// Build a default solid-paint stroke with width `width`.
    pub fn solid(width: f32, color: Rgba) -> Self {
        Self {
            width,
            paint: Paint::Solid(color),
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.0,
            dash: None,
        }
    }
}

/// How an open path's endpoints are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// How two stroke segments meet at a corner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Dash pattern for a stroke. `array` is an alternating
/// dash-on / dash-off length list (in user units); `offset` is the
/// phase offset from the path start.
#[derive(Clone, Debug, Default)]
pub struct DashPattern {
    pub array: Vec<f32>,
    pub offset: f32,
}

/// Fill rule for self-intersecting and compound paths. Matches SVG's
/// `fill-rule` attribute and PDF's `f` (non-zero) vs. `f*` (even-odd)
/// painting operators.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// A 2D affine transform stored as the column-major matrix
///
/// ```text
/// | a c e |   | x |
/// | b d f | * | y |
/// | 0 0 1 |   | 1 |
/// ```
///
/// — i.e. `(x', y') = (a*x + c*y + e, b*x + d*y + f)`. The layout
/// matches SVG's `matrix(a, b, c, d, e, f)` and PDF's `cm` operator
/// argument order, so emitters can serialize fields directly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform2D {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Transform2D {
    /// The identity transform. `compose(identity, x) == x`.
    pub const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Build a translation by `(tx, ty)`.
    pub const fn translate(tx: f32, ty: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    /// Build a non-uniform scale by `(sx, sy)` about the origin.
    pub const fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Build a rotation by `angle_radians` about the origin
    /// (counter-clockwise in a Y-up system, clockwise visually under
    /// the SVG / PDF Y-down convention — this matches both formats).
    pub fn rotate(angle_radians: f32) -> Self {
        let (s, c) = angle_radians.sin_cos();
        Self {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Build a horizontal skew (shear along X) by `angle_radians`.
    pub fn skew_x(angle_radians: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: angle_radians.tan(),
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Build a vertical skew (shear along Y) by `angle_radians`.
    pub fn skew_y(angle_radians: f32) -> Self {
        Self {
            a: 1.0,
            b: angle_radians.tan(),
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Compose `self ∘ other` — the resulting transform applies
    /// `other` first, then `self`, to a point. Equivalent to
    /// `self.matrix() * other.matrix()` in column-vector form.
    pub fn compose(&self, other: &Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    /// Apply this transform to a point.
    pub fn apply(&self, p: Point) -> Point {
        Point {
            x: self.a * p.x + self.c * p.y + self.e,
            y: self.b * p.x + self.d * p.y + self.f,
        }
    }

    /// `true` when this transform is bit-identical to the identity.
    /// Useful for emitters that want to skip a no-op `matrix(...)` /
    /// `cm` write.
    pub fn is_identity(&self) -> bool {
        *self == Self::identity()
    }
}

impl Default for Transform2D {
    fn default() -> Self {
        Self::identity()
    }
}

/// An embedded raster image painted into vector space.
///
/// `bounds` is the axis-aligned rectangle (in the local user space,
/// before `transform`) that the image is painted into; SVG `<image>`
/// `x/y/width/height` and PDF `Do` with a matrix-pre-positioned
/// `Image` XObject both reduce to this shape.
#[derive(Clone, Debug)]
pub struct ImageRef {
    /// Embedded raster payload. Boxed so a `Node::Image` variant
    /// doesn't bloat every other [`Node`] case.
    pub frame: Box<crate::VideoFrame>,
    pub bounds: Rect,
    pub transform: Transform2D,
}

/// Axis-aligned rectangle in user-space coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::TimeBase;

    fn approx_point(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5
    }

    #[test]
    fn path_builder_produces_command_sequence() {
        let mut p = Path::new();
        p.move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(10.0, 0.0))
            .quad_to(Point::new(15.0, 5.0), Point::new(10.0, 10.0))
            .cubic_to(
                Point::new(5.0, 15.0),
                Point::new(0.0, 10.0),
                Point::new(0.0, 0.0),
            )
            .close();
        assert_eq!(p.commands.len(), 5);
        assert_eq!(p.commands[0], PathCommand::MoveTo(Point::new(0.0, 0.0)));
        assert_eq!(p.commands[4], PathCommand::Close);
    }

    #[test]
    fn transform_identity_round_trips() {
        let id = Transform2D::identity();
        assert!(id.is_identity());
        let p = Point::new(3.5, -2.25);
        assert_eq!(id.apply(p), p);
    }

    #[test]
    fn transform_translate_round_trip() {
        let t = Transform2D::translate(10.0, -5.0);
        assert_eq!(t.apply(Point::new(0.0, 0.0)), Point::new(10.0, -5.0));
        assert_eq!(t.apply(Point::new(1.0, 1.0)), Point::new(11.0, -4.0));
    }

    #[test]
    fn transform_scale_round_trip() {
        let s = Transform2D::scale(2.0, 3.0);
        assert_eq!(s.apply(Point::new(1.0, 1.0)), Point::new(2.0, 3.0));
        assert_eq!(s.apply(Point::new(0.0, 0.0)), Point::new(0.0, 0.0));
    }

    #[test]
    fn transform_rotate_quarter_turn() {
        let r = Transform2D::rotate(std::f32::consts::FRAC_PI_2);
        // Under SVG/PDF Y-down with matrix(c,s,-s,c,0,0):
        // (1, 0) rotates to (cos, sin) = (0, 1).
        assert!(approx_point(
            r.apply(Point::new(1.0, 0.0)),
            Point::new(0.0, 1.0)
        ));
        // (0, 1) rotates to (-sin, cos) = (-1, 0).
        assert!(approx_point(
            r.apply(Point::new(0.0, 1.0)),
            Point::new(-1.0, 0.0)
        ));
    }

    #[test]
    fn transform_compose_identity_is_left_and_right_unit() {
        let t = Transform2D::translate(7.0, 11.0);
        let id = Transform2D::identity();
        assert_eq!(id.compose(&t), t);
        assert_eq!(t.compose(&id), t);
    }

    #[test]
    fn transform_compose_translate_then_scale() {
        // Apply translate(2,3) first, then scale(10,10):
        //   p -> p + (2,3) -> 10*(p+(2,3)) = 10p + (20,30).
        let scale = Transform2D::scale(10.0, 10.0);
        let translate = Transform2D::translate(2.0, 3.0);
        let composed = scale.compose(&translate);
        let result = composed.apply(Point::new(1.0, 1.0));
        assert!(approx_point(result, Point::new(30.0, 40.0)));
    }

    #[test]
    fn transform_compose_matches_sequential_apply() {
        // Composition equivalence: composed.apply(p) == a.apply(b.apply(p)).
        let a = Transform2D::rotate(0.5);
        let b = Transform2D::translate(3.0, -1.0);
        let composed = a.compose(&b);
        let p = Point::new(2.0, 5.0);
        let direct = composed.apply(p);
        let stepwise = a.apply(b.apply(p));
        assert!(approx_point(direct, stepwise));
    }

    #[test]
    fn group_default_is_identity_opacity_one_no_clip() {
        let g = Group::default();
        assert!(g.transform.is_identity());
        assert_eq!(g.opacity, 1.0);
        assert!(g.clip.is_none());
        assert!(g.children.is_empty());
    }

    #[test]
    fn group_nesting_with_transforms() {
        // Outer group translates by (10, 10); inner group scales by 2.
        // A point (1, 1) drawn at the inner level should land at
        // (12, 12) after the outer transform is also applied — but the
        // tree itself only stores the local transforms. This test
        // pins down that the nested data is preserved verbatim, since
        // composing transforms is a rasterizer responsibility.
        let inner = Group {
            transform: Transform2D::scale(2.0, 2.0),
            children: vec![Node::Path(PathNode {
                path: {
                    let mut p = Path::new();
                    p.move_to(Point::new(1.0, 1.0));
                    p
                },
                fill: Some(Paint::Solid(Rgba::opaque(255, 0, 0))),
                stroke: None,
                fill_rule: FillRule::NonZero,
            })],
            ..Group::default()
        };
        let outer = Group {
            transform: Transform2D::translate(10.0, 10.0),
            children: vec![Node::Group(inner)],
            ..Group::default()
        };
        match &outer.children[0] {
            Node::Group(g) => {
                assert_eq!(g.transform, Transform2D::scale(2.0, 2.0));
                assert_eq!(g.children.len(), 1);
            }
            _ => panic!("expected a Group child"),
        }
        assert_eq!(outer.transform, Transform2D::translate(10.0, 10.0));
    }

    #[test]
    fn vector_frame_construction() {
        let frame = VectorFrame {
            width: 100.0,
            height: 50.0,
            view_box: Some(ViewBox {
                min_x: 0.0,
                min_y: 0.0,
                width: 100.0,
                height: 50.0,
            }),
            root: Group::default(),
            pts: Some(0),
            time_base: TimeBase::new(1, 1000),
        };
        assert_eq!(frame.width, 100.0);
        assert_eq!(frame.height, 50.0);
        assert!(frame.view_box.is_some());
        assert_eq!(frame.pts, Some(0));
    }

    #[test]
    fn rgba_constructors() {
        let c = Rgba::opaque(10, 20, 30);
        assert_eq!(c.a, 255);
        let c2 = Rgba::new(10, 20, 30, 128);
        assert_eq!(c2.a, 128);
    }

    #[test]
    fn gradient_stop_round_trips() {
        let s = GradientStop::new(0.5, Rgba::opaque(255, 0, 0));
        assert_eq!(s.offset, 0.5);
        let s2 = GradientStop::new(0.5, Rgba::opaque(255, 0, 0));
        assert_eq!(s, s2);
    }

    #[test]
    fn stroke_solid_defaults() {
        let s = Stroke::solid(2.0, Rgba::opaque(0, 0, 0));
        assert_eq!(s.width, 2.0);
        assert_eq!(s.cap, LineCap::Butt);
        assert_eq!(s.join, LineJoin::Miter);
        assert_eq!(s.miter_limit, 4.0);
        assert!(s.dash.is_none());
    }

    #[test]
    fn soft_mask_construction_and_inspection() {
        // Wrap a path in a SoftMask node with a luminance mask. Round-
        // trips both children verbatim through clone + match.
        fn rect_path() -> PathNode {
            let mut p = Path::new();
            p.move_to(Point::new(0.0, 0.0))
                .line_to(Point::new(10.0, 0.0))
                .line_to(Point::new(10.0, 10.0))
                .line_to(Point::new(0.0, 10.0))
                .close();
            PathNode {
                path: p,
                fill: Some(Paint::Solid(Rgba::opaque(255, 255, 255))),
                stroke: None,
                fill_rule: FillRule::NonZero,
            }
        }
        let n = Node::SoftMask {
            mask: Box::new(Node::Path(rect_path())),
            mask_kind: MaskKind::Luminance,
            content: Box::new(Node::Path(rect_path())),
        };
        match &n {
            Node::SoftMask {
                mask_kind, content, ..
            } => {
                assert_eq!(*mask_kind, MaskKind::Luminance);
                match content.as_ref() {
                    Node::Path(_) => {}
                    _ => panic!("expected Path content"),
                }
            }
            _ => panic!("expected SoftMask"),
        }
    }

    #[test]
    fn mask_kind_default_is_luminance() {
        assert_eq!(MaskKind::default(), MaskKind::Luminance);
    }
}

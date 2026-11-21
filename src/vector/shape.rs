//! Shapes with stroke-related extras: non-scaling strokes, vertex
//! markers and paint order.
//!
//! [`PathNode`] is the plain "one path, optional
//! fill, optional stroke" shape. [`ShapeNode`] — the payload of
//! [`Node::Shape`](super::Node::Shape) — wraps one and adds what the
//! plain record cannot say:
//!
//! * a **non-scaling stroke** flag: the stroke outline is computed in
//!   the host (device) coordinate space, so its width does not follow
//!   the transforms the shape is drawn under;
//! * **markers**: symbols (arrowheads, dots, …) drawn at the first,
//!   middle and last vertices of the path, described once as a
//!   [`Marker`] and instantiated by the rasteriser at each vertex;
//! * the **paint order** of fill, stroke and markers.
//!
//! The definitions follow SVG 2 chapter 13 (*Painting*): §13.6 for the
//! vector effect, §13.7 for markers, §13.8 for the paint order.

use std::sync::Arc;

use super::{Node, PathNode, Point, PreserveAspectRatio, Rect, Transform2D, ViewBox};

/// The coordinate system a marker's size and contents are expressed
/// in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MarkerUnits {
    /// One unit is the painted stroke width of the shape that carries
    /// the marker. The initial value.
    #[default]
    StrokeWidth,
    /// The user coordinate system of the shape that carries the
    /// marker.
    UserSpaceOnUse,
}

/// How a marker is rotated when it is placed on a vertex.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum MarkerOrient {
    /// A fixed angle in **degrees** between the shape's positive X
    /// axis and the marker's positive X axis. The initial value is
    /// `Angle(0.0)`.
    Angle(f32),
    /// The marker's positive X axis points along the path at the
    /// vertex.
    Auto,
    /// As [`Auto`](Self::Auto), except that a marker placed at the
    /// start of the path is turned a further 180°, so one arrowhead
    /// definition points outward at both ends.
    AutoStartReverse,
}

impl Default for MarkerOrient {
    fn default() -> Self {
        Self::Angle(0.0)
    }
}

/// Which vertices of a path a marker is drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkerPosition {
    /// The first vertex of the path.
    Start,
    /// Every vertex other than the first and the last.
    Mid,
    /// The last vertex of the path.
    End,
}

/// A marker definition: a small drawing with a reference point, a
/// viewport and an orientation rule.
///
/// `content` is drawn in the marker's own coordinate system. When
/// `view_box` is set it is fitted into the `width` × `height` marker
/// viewport by `preserve_aspect`; `ref_point`, expressed in the
/// content's coordinates, is the point that lands exactly on the
/// vertex. [`placement`](Self::placement) composes the whole chain.
///
/// Paints inside `content` are final: a producer that supports paints
/// taken from the referencing shape (its stroke or fill) resolves them
/// when it builds the marker for that shape.
///
/// `#[non_exhaustive]`: build it with [`new`](Self::new) and the
/// `with_*` builders.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Marker {
    /// The marker's drawing.
    pub content: Node,
    /// Coordinate rectangle of `content` that is fitted into the
    /// marker viewport. `None`: content coordinates are marker units.
    pub view_box: Option<ViewBox>,
    /// How `view_box` is fitted into the viewport.
    pub preserve_aspect: PreserveAspectRatio,
    /// The point of the content that is placed on the vertex, in
    /// content coordinates.
    pub ref_point: Point,
    /// Marker viewport width, in [`units`](Self::units). `3.0`
    /// initially; zero draws nothing.
    pub width: f32,
    /// Marker viewport height, in [`units`](Self::units). `3.0`
    /// initially; zero draws nothing.
    pub height: f32,
    /// Coordinate system of the viewport and the content.
    pub units: MarkerUnits,
    /// Rotation rule.
    pub orient: MarkerOrient,
    /// Whether the content is clipped to the marker viewport. `true`
    /// initially.
    pub clip: bool,
}

impl Marker {
    /// A marker drawing `content`, with the initial attribute values:
    /// a 3 × 3 viewport in stroke-width units, reference point at the
    /// origin, a fixed orientation of 0° and clipping on.
    pub fn new(content: Node) -> Self {
        Self {
            content,
            view_box: None,
            preserve_aspect: PreserveAspectRatio::default(),
            ref_point: Point::new(0.0, 0.0),
            width: 3.0,
            height: 3.0,
            units: MarkerUnits::StrokeWidth,
            orient: MarkerOrient::default(),
            clip: true,
        }
    }

    /// Builder: set the view box fitted into the marker viewport.
    pub fn with_view_box(mut self, view_box: ViewBox) -> Self {
        self.view_box = Some(view_box);
        self
    }

    /// Builder: set how the view box is fitted into the viewport.
    pub fn with_preserve_aspect(mut self, preserve_aspect: PreserveAspectRatio) -> Self {
        self.preserve_aspect = preserve_aspect;
        self
    }

    /// Builder: set the reference point (content coordinates).
    pub fn with_ref_point(mut self, ref_point: Point) -> Self {
        self.ref_point = ref_point;
        self
    }

    /// Builder: set the marker viewport size.
    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Builder: set the coordinate system of the viewport and content.
    pub fn with_units(mut self, units: MarkerUnits) -> Self {
        self.units = units;
        self
    }

    /// Builder: set the rotation rule.
    pub fn with_orient(mut self, orient: MarkerOrient) -> Self {
        self.orient = orient;
        self
    }

    /// Builder: set whether the content is clipped to the viewport.
    pub fn with_clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }

    /// `true` when the marker draws something: both viewport
    /// dimensions are positive.
    pub fn is_renderable(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }

    /// Content coordinates → viewport coordinates (the view-box fit;
    /// identity without a view box).
    fn fit(&self) -> Transform2D {
        match self.view_box {
            Some(vb) => self
                .preserve_aspect
                .viewport_transform(vb, Rect::new(0.0, 0.0, self.width, self.height)),
            None => Transform2D::identity(),
        }
    }

    /// The transform from content coordinates to **marker units
    /// centred on the vertex**: the view-box fit followed by the
    /// translation that brings [`ref_point`](Self::ref_point) to the
    /// origin.
    pub fn content_transform(&self) -> Transform2D {
        let fit = self.fit();
        let anchor = fit.apply(self.ref_point);
        Transform2D::translate(-anchor.x, -anchor.y).compose(&fit)
    }

    /// The marker viewport — the clip rectangle when
    /// [`clip`](Self::clip) is set — in the same vertex-centred marker
    /// units as the output of
    /// [`content_transform`](Self::content_transform).
    pub fn viewport_rect(&self) -> Rect {
        let anchor = self.fit().apply(self.ref_point);
        Rect::new(-anchor.x, -anchor.y, self.width, self.height)
    }

    /// The rotation applied at a vertex, in radians, for a marker at
    /// `position` on a path whose direction there is
    /// `path_direction` (radians, in the [`Transform2D::rotate`]
    /// sense).
    pub fn angle_at(&self, position: MarkerPosition, path_direction: f32) -> f32 {
        match self.orient {
            MarkerOrient::Angle(degrees) => degrees.to_radians(),
            MarkerOrient::AutoStartReverse if position == MarkerPosition::Start => {
                path_direction + std::f32::consts::PI
            }
            MarkerOrient::Auto | MarkerOrient::AutoStartReverse => path_direction,
        }
    }

    /// The transform from marker **content** coordinates to the user
    /// space of the shape, for one instance of the marker.
    ///
    /// `vertex` is the path vertex the marker sits on,
    /// `path_direction` the direction of the path there (radians), and
    /// `stroke_width` the painted stroke width of the shape. The chain
    /// is the one of SVG 2 §13.7.4: translate to the vertex, rotate by
    /// the orientation, scale by the stroke width when the units are
    /// [`MarkerUnits::StrokeWidth`], then
    /// [`content_transform`](Self::content_transform).
    ///
    /// ```
    /// # use oxideav_core::vector::{Marker, MarkerOrient, MarkerPosition};
    /// # use oxideav_core::{Group, Node, Point, ViewBox};
    /// // viewBox 0 0 10 10 in a 4×3 viewport, tip of the arrow at (10, 5).
    /// let arrow = Marker::new(Node::Group(Group::new()))
    ///     .with_view_box(ViewBox::new(0.0, 0.0, 10.0, 10.0))
    ///     .with_size(4.0, 3.0)
    ///     .with_ref_point(Point::new(10.0, 5.0))
    ///     .with_orient(MarkerOrient::Auto);
    /// let t = arrow.placement(MarkerPosition::End, Point::new(50.0, 20.0), 0.0, 2.0);
    /// // The reference point lands on the vertex …
    /// assert_eq!(t.apply(Point::new(10.0, 5.0)), Point::new(50.0, 20.0));
    /// // … and one content unit is 0.3 marker units × stroke width 2.
    /// assert_eq!(t.apply(Point::new(0.0, 5.0)), Point::new(44.0, 20.0));
    /// ```
    pub fn placement(
        &self,
        position: MarkerPosition,
        vertex: Point,
        path_direction: f32,
        stroke_width: f32,
    ) -> Transform2D {
        let scale = match self.units {
            MarkerUnits::StrokeWidth => stroke_width,
            MarkerUnits::UserSpaceOnUse => 1.0,
        };
        Transform2D::translate(vertex.x, vertex.y)
            .compose(&Transform2D::rotate(
                self.angle_at(position, path_direction),
            ))
            .compose(&Transform2D::scale(scale, scale))
            .compose(&self.content_transform())
    }
}

/// The markers of one shape: an optional definition for the first
/// vertex, for the middle vertices and for the last vertex.
/// Definitions are shared behind [`Arc`], since one marker typically
/// decorates many shapes.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Markers {
    /// Marker drawn on the first vertex of the path.
    pub start: Option<Arc<Marker>>,
    /// Marker drawn on every vertex other than the first and last.
    pub mid: Option<Arc<Marker>>,
    /// Marker drawn on the last vertex of the path.
    pub end: Option<Arc<Marker>>,
}

impl Markers {
    /// No markers.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same marker at every position.
    pub fn all(marker: impl Into<Arc<Marker>>) -> Self {
        let marker = marker.into();
        Self {
            start: Some(marker.clone()),
            mid: Some(marker.clone()),
            end: Some(marker),
        }
    }

    /// Builder: set the start marker.
    pub fn with_start(mut self, marker: impl Into<Arc<Marker>>) -> Self {
        self.start = Some(marker.into());
        self
    }

    /// Builder: set the mid marker.
    pub fn with_mid(mut self, marker: impl Into<Arc<Marker>>) -> Self {
        self.mid = Some(marker.into());
        self
    }

    /// Builder: set the end marker.
    pub fn with_end(mut self, marker: impl Into<Arc<Marker>>) -> Self {
        self.end = Some(marker.into());
        self
    }

    /// The marker for `position`, if any.
    pub fn get(&self, position: MarkerPosition) -> Option<&Arc<Marker>> {
        match position {
            MarkerPosition::Start => self.start.as_ref(),
            MarkerPosition::Mid => self.mid.as_ref(),
            MarkerPosition::End => self.end.as_ref(),
        }
    }

    /// `true` when no position has a marker.
    pub fn is_empty(&self) -> bool {
        self.start.is_none() && self.mid.is_none() && self.end.is_none()
    }
}

/// One of the three paint operations of a shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaintLayer {
    /// The interior fill.
    Fill,
    /// The stroked outline.
    Stroke,
    /// The vertex markers.
    Markers,
}

/// The order in which a shape's fill, stroke and markers are painted,
/// first to last. `Default` is the normal order: fill, stroke,
/// markers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaintOrder([PaintLayer; 3]);

impl Default for PaintOrder {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl PaintOrder {
    /// Fill, then stroke, then markers.
    pub const NORMAL: Self = Self([PaintLayer::Fill, PaintLayer::Stroke, PaintLayer::Markers]);

    /// The stroke beneath the fill: stroke, fill, markers.
    pub const STROKE_FIRST: Self =
        Self([PaintLayer::Stroke, PaintLayer::Fill, PaintLayer::Markers]);

    /// Build an order from the layers named first; layers not named
    /// follow in their normal relative order, and a layer named twice
    /// counts once.
    ///
    /// ```
    /// # use oxideav_core::vector::{PaintLayer, PaintOrder};
    /// use PaintLayer::*;
    /// assert_eq!(PaintOrder::new(&[Stroke]).layers(), [Stroke, Fill, Markers]);
    /// assert_eq!(PaintOrder::new(&[Markers]).layers(), [Markers, Fill, Stroke]);
    /// assert_eq!(PaintOrder::new(&[]), PaintOrder::NORMAL);
    /// ```
    pub fn new(first: &[PaintLayer]) -> Self {
        let mut out = Vec::with_capacity(3);
        for &layer in first.iter().chain(&Self::NORMAL.0) {
            if !out.contains(&layer) {
                out.push(layer);
            }
        }
        Self([out[0], out[1], out[2]])
    }

    /// The three layers, first painted to last painted.
    pub const fn layers(self) -> [PaintLayer; 3] {
        self.0
    }

    /// `true` for the normal order.
    pub fn is_normal(self) -> bool {
        self == Self::NORMAL
    }
}

/// A path shape with stroke-related extras: the payload of
/// [`Node::Shape`](super::Node::Shape).
///
/// A consumer that implements none of the extras renders
/// [`path`](Self::path) exactly as it would a [`Node::Path`]: the
/// result is the same shape with a scaling stroke, no markers and the
/// normal paint order. A producer with nothing extra to say keeps
/// emitting [`Node::Path`] ([`into_node`](Self::into_node) does that
/// automatically).
///
/// `#[non_exhaustive]`: build it with [`new`](Self::new) and the
/// `with_*` builders.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ShapeNode {
    /// Geometry, fill, stroke and fill rule.
    pub path: PathNode,
    /// When `true`, the stroke outline is computed in the host
    /// (device) coordinate space: the path is transformed there, the
    /// outline is built with the stroke's width, dashes, caps and
    /// joins taken as host-space lengths, and the outline is
    /// transformed back to be filled with the stroke paint in user
    /// space. The visible stroke width is then independent of the
    /// transforms — including non-uniform scales and shears — the
    /// shape is drawn under.
    ///
    /// Markers in [`MarkerUnits::StrokeWidth`] units follow: they are
    /// sized by the stroke width as painted, so they do not scale
    /// either.
    pub non_scaling_stroke: bool,
    /// Vertex markers.
    pub markers: Markers,
    /// Order of the fill, stroke and marker paint operations.
    pub paint_order: PaintOrder,
}

impl ShapeNode {
    /// A shape with no extras: a scaling stroke, no markers, normal
    /// paint order.
    pub fn new(path: PathNode) -> Self {
        Self {
            path,
            non_scaling_stroke: false,
            markers: Markers::new(),
            paint_order: PaintOrder::NORMAL,
        }
    }

    /// Builder: set the non-scaling stroke flag.
    pub fn with_non_scaling_stroke(mut self, non_scaling: bool) -> Self {
        self.non_scaling_stroke = non_scaling;
        self
    }

    /// Builder: set the markers.
    pub fn with_markers(mut self, markers: Markers) -> Self {
        self.markers = markers;
        self
    }

    /// Builder: set the paint order.
    pub fn with_paint_order(mut self, paint_order: PaintOrder) -> Self {
        self.paint_order = paint_order;
        self
    }

    /// `true` when the shape says nothing a plain [`PathNode`] cannot:
    /// a scaling stroke, no markers and the normal paint order.
    pub fn is_plain(&self) -> bool {
        !self.non_scaling_stroke && self.markers.is_empty() && self.paint_order.is_normal()
    }

    /// The scene node for this shape: [`Node::Path`] when it
    /// [is plain](Self::is_plain) — so consumers that predate
    /// [`Node::Shape`](super::Node::Shape) keep rendering it — and
    /// `Node::Shape` otherwise.
    pub fn into_node(self) -> Node {
        if self.is_plain() {
            Node::Path(self.path)
        } else {
            Node::Shape(self)
        }
    }
}

impl From<PathNode> for ShapeNode {
    fn from(path: PathNode) -> Self {
        Self::new(path)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Group, Path, Rgba, Stroke};
    use super::*;

    fn approx(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4
    }

    fn empty() -> Node {
        Node::Group(Group::new())
    }

    fn line() -> PathNode {
        let mut p = Path::new();
        p.move_to(Point::new(0.0, 0.0))
            .line_to(Point::new(10.0, 0.0));
        PathNode::new(p).with_stroke(Stroke::solid(2.0, Rgba::opaque(0, 0, 0)))
    }

    #[test]
    fn marker_defaults_are_the_initial_values() {
        let m = Marker::new(empty());
        assert!(m.view_box.is_none());
        assert_eq!(m.preserve_aspect, PreserveAspectRatio::default());
        assert_eq!(m.ref_point, Point::new(0.0, 0.0));
        assert_eq!((m.width, m.height), (3.0, 3.0));
        assert_eq!(m.units, MarkerUnits::StrokeWidth);
        assert_eq!(m.orient, MarkerOrient::Angle(0.0));
        assert!(m.clip);
        assert!(m.is_renderable());
        assert!(m.content_transform().is_identity());
        assert_eq!(m.viewport_rect(), Rect::new(0.0, 0.0, 3.0, 3.0));
        assert_eq!(MarkerUnits::default(), MarkerUnits::StrokeWidth);
        assert_eq!(MarkerOrient::default(), MarkerOrient::Angle(0.0));
    }

    #[test]
    fn marker_builders_set_each_field() {
        let m = Marker::new(empty())
            .with_view_box(ViewBox::new(0.0, 0.0, 10.0, 10.0))
            .with_preserve_aspect(PreserveAspectRatio::none())
            .with_ref_point(Point::new(1.0, 5.0))
            .with_size(4.0, 0.0)
            .with_units(MarkerUnits::UserSpaceOnUse)
            .with_orient(MarkerOrient::AutoStartReverse)
            .with_clip(false);
        assert_eq!(m.view_box, Some(ViewBox::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(m.preserve_aspect, PreserveAspectRatio::none());
        assert_eq!(m.ref_point, Point::new(1.0, 5.0));
        assert_eq!((m.width, m.height), (4.0, 0.0));
        assert_eq!(m.units, MarkerUnits::UserSpaceOnUse);
        assert_eq!(m.orient, MarkerOrient::AutoStartReverse);
        assert!(!m.clip);
        assert!(!m.is_renderable());
    }

    /// The worked example of SVG 2 §13.7.4: viewBox 0 0 10 10,
    /// refX 0 refY 5, a 4×3 viewport in stroke-width units, auto
    /// orientation, on the end of a 100-wide stroke heading 45°.
    #[test]
    fn placement_follows_the_marker_rendering_chain() {
        let m = Marker::new(empty())
            .with_view_box(ViewBox::new(0.0, 0.0, 10.0, 10.0))
            .with_ref_point(Point::new(0.0, 5.0))
            .with_size(4.0, 3.0)
            .with_orient(MarkerOrient::Auto);
        let vertex = Point::new(2500.0, 1250.0);
        let dir = std::f32::consts::FRAC_PI_4;
        let t = m.placement(MarkerPosition::End, vertex, dir, 100.0);
        // The reference point lands on the vertex.
        assert!(approx(t.apply(Point::new(0.0, 5.0)), vertex));
        // Uniform fit scale 3/10; ten content units along X are
        // 0.3 × 10 × 100 = 300 user units along the 45° direction.
        let tip = t.apply(Point::new(10.0, 5.0));
        let d = 300.0 * std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            (tip.x - (vertex.x + d)).abs() < 0.05 && (tip.y - (vertex.y + d)).abs() < 0.05,
            "{tip:?}"
        );
        // The viewport, in vertex-centred marker units: the view box
        // is centred horizontally in the 4-wide viewport (0.5 slack).
        assert_eq!(m.viewport_rect(), Rect::new(-0.5, -1.5, 4.0, 3.0));
    }

    #[test]
    fn user_space_markers_ignore_the_stroke_width() {
        let m = Marker::new(empty())
            .with_units(MarkerUnits::UserSpaceOnUse)
            .with_ref_point(Point::new(1.0, 1.0));
        let v = Point::new(7.0, 9.0);
        for sw in [0.5, 1.0, 40.0] {
            let t = m.placement(MarkerPosition::Mid, v, 1.0, sw);
            assert!(approx(t.apply(Point::new(1.0, 1.0)), v));
            assert!(approx(t.apply(Point::new(2.0, 1.0)), Point::new(8.0, 9.0)));
        }
        // Stroke-width units scale with it.
        let m = Marker::new(empty());
        let t = m.placement(MarkerPosition::Mid, v, 0.0, 5.0);
        assert!(approx(t.apply(Point::new(1.0, 0.0)), Point::new(12.0, 9.0)));
    }

    #[test]
    fn orientation_rules() {
        use std::f32::consts::PI;
        let fixed = Marker::new(empty()).with_orient(MarkerOrient::Angle(90.0));
        let auto = Marker::new(empty()).with_orient(MarkerOrient::Auto);
        let rev = Marker::new(empty()).with_orient(MarkerOrient::AutoStartReverse);
        for pos in [
            MarkerPosition::Start,
            MarkerPosition::Mid,
            MarkerPosition::End,
        ] {
            assert!((fixed.angle_at(pos, 0.3) - PI / 2.0).abs() < 1e-6);
            assert_eq!(auto.angle_at(pos, 0.3), 0.3);
        }
        assert_eq!(rev.angle_at(MarkerPosition::Start, 0.3), 0.3 + PI);
        assert_eq!(rev.angle_at(MarkerPosition::Mid, 0.3), 0.3);
        assert_eq!(rev.angle_at(MarkerPosition::End, 0.3), 0.3);
        // A start arrow with auto-start-reverse points backwards.
        let t = rev.placement(MarkerPosition::Start, Point::new(0.0, 0.0), 0.0, 1.0);
        assert!(approx(t.apply(Point::new(1.0, 0.0)), Point::new(-1.0, 0.0)));
    }

    #[test]
    fn markers_set_lookup_and_sharing() {
        let none = Markers::new();
        assert!(none.is_empty());
        assert!(none.get(MarkerPosition::Start).is_none());
        let dot = Arc::new(Marker::new(empty()));
        let all = Markers::all(dot.clone());
        assert!(!all.is_empty());
        for pos in [
            MarkerPosition::Start,
            MarkerPosition::Mid,
            MarkerPosition::End,
        ] {
            assert!(Arc::ptr_eq(all.get(pos).unwrap(), &dot));
        }
        let ends = Markers::new()
            .with_start(dot.clone())
            .with_end(Marker::new(empty()).with_size(1.0, 1.0));
        assert!(ends.get(MarkerPosition::Start).is_some());
        assert!(ends.get(MarkerPosition::Mid).is_none());
        assert_eq!(ends.get(MarkerPosition::End).unwrap().width, 1.0);
        assert!(!Markers::new().with_mid(dot).is_empty());
    }

    #[test]
    fn paint_order_fills_in_omitted_layers() {
        use PaintLayer::*;
        assert_eq!(PaintOrder::default(), PaintOrder::NORMAL);
        assert!(PaintOrder::NORMAL.is_normal());
        assert_eq!(PaintOrder::NORMAL.layers(), [Fill, Stroke, Markers]);
        assert_eq!(PaintOrder::new(&[Stroke]), PaintOrder::STROKE_FIRST);
        assert!(!PaintOrder::STROKE_FIRST.is_normal());
        assert_eq!(
            PaintOrder::new(&[Markers, Stroke]).layers(),
            [Markers, Stroke, Fill]
        );
        assert_eq!(
            PaintOrder::new(&[Stroke, Stroke, Fill, Markers, Fill]).layers(),
            [Stroke, Fill, Markers]
        );
        assert_eq!(PaintOrder::new(&[Fill]), PaintOrder::NORMAL);
    }

    #[test]
    fn shape_defaults_builders_and_plain_downgrade() {
        let s = ShapeNode::new(line());
        assert!(!s.non_scaling_stroke);
        assert!(s.markers.is_empty());
        assert!(s.paint_order.is_normal());
        assert!(s.is_plain());
        assert!(matches!(s.into_node(), Node::Path(_)));
        let from: ShapeNode = line().into();
        assert!(from.is_plain());

        for shape in [
            ShapeNode::new(line()).with_non_scaling_stroke(true),
            ShapeNode::new(line()).with_markers(Markers::all(Marker::new(empty()))),
            ShapeNode::new(line()).with_paint_order(PaintOrder::STROKE_FIRST),
        ] {
            assert!(!shape.is_plain());
            match shape.clone().into_node() {
                Node::Shape(n) => {
                    // The wrapped path is intact for consumers that
                    // render it without the extras.
                    assert_eq!(n.path.path.commands.len(), 2);
                    assert_eq!(n.path.stroke.as_ref().map(|s| s.width), Some(2.0));
                }
                _ => panic!("expected Node::Shape"),
            }
        }
    }
}

//! Filter effects: a typed description of a filter primitive graph
//! attached to a subtree of the scene.
//!
//! A filter takes the rendering of a subtree (the *source graphic*),
//! runs it through a list of image-processing *primitives* — blur,
//! offset, colour matrix, composite, lighting, … — and paints the last
//! primitive's result in place of the subtree. Each primitive reads
//! its inputs from the source graphic, from one of the other standard
//! inputs, or from the result of an earlier primitive, so the list
//! forms a directed acyclic graph evaluated in order.
//!
//! This module carries that graph as plain data, following the model
//! and the attribute defaults of the W3C *Filter Effects Module Level
//! 1* (the `filter` element and its `fe*` children): [`FilterEffect`]
//! is the filter, [`FilterPrimitive`] one node, [`FilterPrimitiveKind`]
//! the operation with its parameters, [`FilterInput`] an edge.
//! [`FilterNode`] — the payload of
//! [`Node::Filter`](super::Node::Filter) — binds an effect to the
//! subtree it filters.
//!
//! Nothing here renders. A parser fills the records in; a rasteriser
//! walks them. Lengths are numbers in the coordinate system named by
//! the effect's [`primitive_units`](FilterEffect::primitive_units);
//! angles of light sources and hue rotation are in **degrees**, as the
//! specification states them.

use std::sync::Arc;

use super::{Node, Paint, PreserveAspectRatio, Rect, Rgba};
use crate::error::{Error, Result};

/// The coordinate system a filter's lengths are expressed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FilterUnits {
    /// Fractions of the bounding box of the filtered content: `0.0` is
    /// the box's left / top edge, `1.0` its width / height.
    ObjectBoundingBox,
    /// The user coordinate system in place where the filter is
    /// applied.
    UserSpaceOnUse,
}

/// The colour space a filter primitive computes in
/// (`color-interpolation-filters`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColorInterpolation {
    /// Gamma-encoded sRGB component values, used as they are.
    SRgb,
    /// Linear-light RGB: inputs are converted from sRGB before the
    /// operation and the result converted back. The property's initial
    /// value, hence what [`FilterPrimitive::new`] selects.
    LinearRgb,
}

/// Where a filter primitive reads one of its inputs from.
///
/// `#[non_exhaustive]`: match with a wildcard arm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FilterInput {
    /// No explicit input: the result of the preceding primitive, or
    /// the source graphic for the first primitive.
    /// [`FilterEffect::resolve_input`] makes that substitution.
    #[default]
    Previous,
    /// The rendering of the filtered content.
    SourceGraphic,
    /// The alpha channel of the source graphic, with black colour
    /// channels.
    SourceAlpha,
    /// The backdrop behind the filter region at the time the filter is
    /// invoked.
    BackgroundImage,
    /// The alpha channel of the background image.
    BackgroundAlpha,
    /// The fill paint of the filtered element, with conceptually
    /// infinite extent ([`FilterEffect::fill_paint`]).
    FillPaint,
    /// The stroke paint of the filtered element, with conceptually
    /// infinite extent ([`FilterEffect::stroke_paint`]).
    StrokePaint,
    /// The result of the primitive at this index of
    /// [`FilterEffect::primitives`]. Must be lower than the index of
    /// the primitive that reads it (no forward references).
    Result(usize),
}

/// The subregion that restricts the calculation and rendering of one
/// primitive: `x`, `y`, `width`, `height` in the effect's
/// [`primitive_units`](FilterEffect::primitive_units).
///
/// A `None` component takes the default: the corresponding component
/// of the union of the subregions of the referenced primitives, or of
/// the filter region when the primitive has no input, reads a standard
/// input, or is a tile. A zero or negative width / height disables the
/// primitive (its result is transparent black).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FilterSubregion {
    /// Minimum X of the subregion.
    pub x: Option<f32>,
    /// Minimum Y of the subregion.
    pub y: Option<f32>,
    /// Width of the subregion.
    pub width: Option<f32>,
    /// Height of the subregion.
    pub height: Option<f32>,
}

impl FilterSubregion {
    /// A subregion with every component defaulted.
    pub const fn new() -> Self {
        Self {
            x: None,
            y: None,
            width: None,
            height: None,
        }
    }

    /// A subregion with all four components given.
    pub const fn from_rect(rect: Rect) -> Self {
        Self {
            x: Some(rect.x),
            y: Some(rect.y),
            width: Some(rect.width),
            height: Some(rect.height),
        }
    }

    /// Builder: set the minimum X.
    pub const fn with_x(mut self, x: f32) -> Self {
        self.x = Some(x);
        self
    }

    /// Builder: set the minimum Y.
    pub const fn with_y(mut self, y: f32) -> Self {
        self.y = Some(y);
        self
    }

    /// Builder: set the width.
    pub const fn with_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Builder: set the height.
    pub const fn with_height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// `true` when every component is defaulted.
    pub const fn is_default(&self) -> bool {
        self.x.is_none() && self.y.is_none() && self.width.is_none() && self.height.is_none()
    }

    /// The concrete rectangle: each given component, and the matching
    /// component of `default` for the others.
    pub fn resolve(&self, default: Rect) -> Rect {
        Rect {
            x: self.x.unwrap_or(default.x),
            y: self.y.unwrap_or(default.y),
            width: self.width.unwrap_or(default.width),
            height: self.height.unwrap_or(default.height),
        }
    }
}

/// A blend mode: how a source colour mixes with the backdrop colour
/// beneath it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[allow(missing_docs)] // The variants are the sixteen named modes.
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

/// The matrix applied by a colour-matrix primitive to the
/// non-premultiplied RGBA value of every pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ColorMatrixKind {
    /// A full 5×4 matrix in row order `a00 a01 a02 a03 a04 a10 … a34`:
    /// row *i* gives output component *i* (R, G, B, A) as a weighted
    /// sum of the input R, G, B, A and the constant `1`.
    Matrix([f32; 20]),
    /// Saturation adjustment by a single factor (`1.0` leaves the
    /// colour unchanged, `0.0` removes all saturation).
    Saturate(f32),
    /// Hue rotation by an angle in degrees.
    HueRotate(f32),
    /// Luminance moved to the alpha channel; colour channels cleared.
    LuminanceToAlpha,
}

impl ColorMatrixKind {
    /// The identity 5×4 matrix — the effect of a matrix-type primitive
    /// with no values.
    pub const IDENTITY: Self = Self::Matrix([
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        1.0, 0.0,
    ]);
}

/// A per-component transfer function `C → C'` on the closed interval
/// `[0, 1]`, as used by the component-transfer primitive.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum TransferFunction {
    /// `C' = C`.
    #[default]
    Identity,
    /// Piecewise-linear interpolation through `n + 1` values `v0 … vn`
    /// that bound `n` equal regions: for `k/n <= C < (k+1)/n`,
    /// `C' = vk + (C − k/n)·n·(vk+1 − vk)`; `C = 1` gives `vn`. An
    /// empty list is the identity.
    Table(Vec<f32>),
    /// A step function of `n` values `v0 … vn-1`: for
    /// `k/n <= C < (k+1)/n`, `C' = vk`; `C = 1` gives `vn-1`. An empty
    /// list is the identity.
    Discrete(Vec<f32>),
    /// `C' = slope·C + intercept`.
    Linear {
        /// Multiplier of the input component.
        slope: f32,
        /// Constant added to the product.
        intercept: f32,
    },
    /// `C' = amplitude·C^exponent + offset`.
    Gamma {
        /// Multiplier of the power term.
        amplitude: f32,
        /// Exponent applied to the input component.
        exponent: f32,
        /// Constant added to the product.
        offset: f32,
    },
}

impl TransferFunction {
    /// Evaluate the function at `c`. The input is clamped to `[0, 1]`
    /// first; the output is returned unclamped (a consumer clamps when
    /// it stores the component).
    ///
    /// ```
    /// # use oxideav_core::vector::TransferFunction;
    /// let t = TransferFunction::Table(vec![0.0, 1.0, 0.0]);
    /// assert_eq!(t.apply(0.25), 0.5);
    /// assert_eq!(t.apply(0.5), 1.0);
    /// let d = TransferFunction::Discrete(vec![0.2, 0.8]);
    /// assert_eq!(d.apply(0.49), 0.2);
    /// assert_eq!(d.apply(1.0), 0.8);
    /// ```
    pub fn apply(&self, c: f32) -> f32 {
        let c = c.clamp(0.0, 1.0);
        match self {
            Self::Identity => c,
            Self::Table(v) => match v.len() {
                0 => c,
                1 => v[0],
                len => {
                    let n = len - 1;
                    let scaled = c * n as f32;
                    let k = (scaled.floor() as usize).min(n - 1);
                    v[k] + (scaled - k as f32) * (v[k + 1] - v[k])
                }
            },
            Self::Discrete(v) => match v.len() {
                0 => c,
                n => v[((c * n as f32).floor() as usize).min(n - 1)],
            },
            Self::Linear { slope, intercept } => slope * c + intercept,
            Self::Gamma {
                amplitude,
                exponent,
                offset,
            } => amplitude * c.powf(*exponent) + offset,
        }
    }
}

/// The operator of a composite primitive, combining input 0 (the
/// source) with input 1 (the destination).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum CompositeOperator {
    /// Source over destination.
    #[default]
    Over,
    /// The part of the source inside the destination.
    In,
    /// The part of the source outside the destination.
    Out,
    /// The source where it overlaps the destination, over the
    /// destination.
    Atop,
    /// The non-overlapping parts of both.
    Xor,
    /// The sum of both.
    Lighter,
    /// Per component, on premultiplied values:
    /// `result = k1·i1·i2 + k2·i1 + k3·i2 + k4`.
    Arithmetic {
        /// Weight of the product of both inputs.
        k1: f32,
        /// Weight of input 0.
        k2: f32,
        /// Weight of input 1.
        k3: f32,
        /// Constant term.
        k4: f32,
    },
}

/// How an input image is extended past its edges for a neighbourhood
/// operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EdgeMode {
    /// Extend by repeating the colour values at the nearest edge.
    Duplicate,
    /// Extend by taking the colour values from the opposite edge.
    Wrap,
    /// Extend with transparent black.
    None,
}

/// One colour channel of an RGBA pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum ColorChannel {
    R,
    G,
    B,
    A,
}

/// The operator of a morphology primitive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MorphologyOperator {
    /// Thin the image: per-channel minimum over the neighbourhood.
    #[default]
    Erode,
    /// Fatten the image: per-channel maximum over the neighbourhood.
    Dilate,
}

/// The noise function of a turbulence primitive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TurbulenceKind {
    /// The sum of signed noise octaves.
    FractalNoise,
    /// The sum of absolute noise octaves.
    #[default]
    Turbulence,
}

/// The light source of a lighting primitive. Positions are in the
/// effect's primitive units, with Z measured out of the image plane
/// toward the viewer; angles are in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LightSource {
    /// A light infinitely far away.
    Distant {
        /// Direction angle of the light in the image plane, clockwise
        /// from the X axis.
        azimuth: f32,
        /// Direction angle of the light from the image plane toward
        /// the Z axis.
        elevation: f32,
    },
    /// A light at a point, shining in all directions.
    Point {
        /// X location of the light.
        x: f32,
        /// Y location of the light.
        y: f32,
        /// Z location of the light.
        z: f32,
    },
    /// A light at a point, shining toward another point within a cone.
    Spot {
        /// X location of the light.
        x: f32,
        /// Y location of the light.
        y: f32,
        /// Z location of the light.
        z: f32,
        /// X of the point the light is aimed at.
        points_at_x: f32,
        /// Y of the point the light is aimed at.
        points_at_y: f32,
        /// Z of the point the light is aimed at.
        points_at_z: f32,
        /// Exponent controlling the focus of the light (`1.0` by
        /// default).
        specular_exponent: f32,
        /// Half-angle of the cone outside which no light is projected.
        /// `None` applies no limiting cone.
        limiting_cone_angle: Option<f32>,
    },
}

/// The operation performed by one filter primitive, with its
/// parameters.
///
/// Each variant documents how many inputs it reads
/// ([`input_count`](Self::input_count)); the inputs themselves live in
/// [`FilterPrimitive::inputs`]. Pairs written `(x, y)` hold the
/// horizontal and vertical value of an attribute that accepts one or
/// two numbers.
///
/// `#[non_exhaustive]`: match with a wildcard arm — a rasteriser that
/// meets a kind it does not implement passes the first input through.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum FilterPrimitiveKind {
    /// Blend input 0 (the source) onto input 1 (the backdrop). Two
    /// inputs.
    Blend {
        /// The blend mode.
        mode: BlendMode,
        /// When `true` the blend is not combined with source-over
        /// compositing.
        no_composite: bool,
    },
    /// Apply a colour matrix to every pixel. One input.
    ColorMatrix(ColorMatrixKind),
    /// Remap each channel through its own transfer function. One input.
    ComponentTransfer {
        /// Function for the red channel.
        r: TransferFunction,
        /// Function for the green channel.
        g: TransferFunction,
        /// Function for the blue channel.
        b: TransferFunction,
        /// Function for the alpha channel.
        a: TransferFunction,
    },
    /// Combine input 0 with input 1 through a compositing operator.
    /// Two inputs.
    Composite(CompositeOperator),
    /// Convolve the image with a kernel. One input.
    ConvolveMatrix {
        /// Kernel size as `(columns, rows)`; each at least 1.
        order: (u32, u32),
        /// The `columns × rows` kernel values, in row order.
        kernel: Vec<f32>,
        /// Divisor applied to the weighted sum. `None` takes the sum of
        /// the kernel values, or `1` when that sum is zero.
        divisor: Option<f32>,
        /// Constant added after the division.
        bias: f32,
        /// Position `(x, y)` of the kernel cell aligned with the
        /// target pixel. `None` takes `floor(order / 2)` on each axis.
        target: Option<(u32, u32)>,
        /// How the input is extended past its edges.
        edge_mode: EdgeMode,
        /// Intended distance between kernel columns / rows, in
        /// primitive units. `None` means one device pixel.
        kernel_unit_length: Option<(f32, f32)>,
        /// When `true` only the colour channels are convolved and the
        /// alpha channel is copied through.
        preserve_alpha: bool,
    },
    /// Light the alpha channel of the input as a bump map, diffuse
    /// component only. One input.
    DiffuseLighting {
        /// Height of the surface for alpha `1.0`.
        surface_scale: f32,
        /// The diffuse reflection constant *kd*; non-negative.
        diffuse_constant: f32,
        /// Intended distance between surface-normal samples, in
        /// primitive units. `None` means one device pixel.
        kernel_unit_length: Option<(f32, f32)>,
        /// Colour of the light (`lighting-color`).
        color: Rgba,
        /// The light source.
        light: LightSource,
    },
    /// Displace the pixels of input 0 by the channel values of input 1.
    /// Two inputs.
    DisplacementMap {
        /// Displacement scale, in primitive units.
        scale: f32,
        /// Channel of input 1 that displaces along X.
        x_channel: ColorChannel,
        /// Channel of input 1 that displaces along Y.
        y_channel: ColorChannel,
    },
    /// Draw a blurred, offset, flood-coloured copy of the input's
    /// alpha beneath the input. One input.
    DropShadow {
        /// Horizontal offset of the shadow.
        dx: f32,
        /// Vertical offset of the shadow.
        dy: f32,
        /// Blur standard deviation `(x, y)` of the shadow.
        std_deviation: (f32, f32),
        /// Shadow colour: the flood colour with the flood opacity
        /// folded into its alpha.
        color: Rgba,
    },
    /// Fill the subregion with one colour. No input.
    Flood {
        /// The flood colour with the flood opacity folded into its
        /// alpha.
        color: Rgba,
    },
    /// Gaussian blur. One input.
    GaussianBlur {
        /// Standard deviation `(x, y)`. Zero on one axis blurs along
        /// the other only; zero on both passes the input through.
        std_deviation: (f32, f32),
        /// How the input is extended past its edges.
        edge_mode: EdgeMode,
    },
    /// Render an external image or a scene subtree into the subregion.
    /// No input.
    Image {
        /// What to render: typically a [`Node::Image`] or the subtree
        /// of a referenced element.
        content: Box<Node>,
        /// How an image is fitted into the subregion.
        preserve_aspect: PreserveAspectRatio,
    },
    /// Composite all inputs over one another, first input at the
    /// bottom. Any number of inputs.
    Merge,
    /// Erode or dilate. One input.
    Morphology {
        /// The operator.
        operator: MorphologyOperator,
        /// Neighbourhood radius `(x, y)`.
        radius: (f32, f32),
    },
    /// Shift the input. One input.
    Offset {
        /// Horizontal shift.
        dx: f32,
        /// Vertical shift.
        dy: f32,
    },
    /// Light the alpha channel of the input as a bump map, specular
    /// component only. One input.
    SpecularLighting {
        /// Height of the surface for alpha `1.0`.
        surface_scale: f32,
        /// The specular reflection constant *ks*; non-negative.
        specular_constant: f32,
        /// Exponent of the specular term; larger is shinier.
        specular_exponent: f32,
        /// Intended distance between surface-normal samples, in
        /// primitive units. `None` means one device pixel.
        kernel_unit_length: Option<(f32, f32)>,
        /// Colour of the light (`lighting-color`).
        color: Rgba,
        /// The light source.
        light: LightSource,
    },
    /// Fill the subregion with repeated copies of the input's
    /// subregion. One input.
    Tile,
    /// Synthesise a noise image. No input.
    Turbulence {
        /// Base frequency `(x, y)` of the noise function.
        base_frequency: (f32, f32),
        /// Number of noise octaves summed.
        octaves: u32,
        /// Seed of the pseudo-random generator.
        seed: f32,
        /// When `true` the frequencies are adjusted so the tile edges
        /// join smoothly.
        stitch_tiles: bool,
        /// The noise function.
        kind: TurbulenceKind,
    },
}

impl FilterPrimitiveKind {
    /// A Gaussian blur with the same deviation on both axes and no
    /// edge extension (the attribute's initial value).
    pub const fn gaussian_blur(std_deviation: f32) -> Self {
        Self::GaussianBlur {
            std_deviation: (std_deviation, std_deviation),
            edge_mode: EdgeMode::None,
        }
    }

    /// An offset by `(dx, dy)`.
    pub const fn offset(dx: f32, dy: f32) -> Self {
        Self::Offset { dx, dy }
    }

    /// A flood of `color`.
    pub const fn flood(color: Rgba) -> Self {
        Self::Flood { color }
    }

    /// A blend with the given mode, combined with source-over
    /// compositing.
    pub const fn blend(mode: BlendMode) -> Self {
        Self::Blend {
            mode,
            no_composite: false,
        }
    }

    /// A drop shadow with the same blur deviation on both axes.
    pub const fn drop_shadow(dx: f32, dy: f32, std_deviation: f32, color: Rgba) -> Self {
        Self::DropShadow {
            dx,
            dy,
            std_deviation: (std_deviation, std_deviation),
            color,
        }
    }

    /// A component transfer applying `f` to the three colour channels
    /// and leaving alpha untouched.
    pub fn component_transfer_rgb(f: TransferFunction) -> Self {
        Self::ComponentTransfer {
            r: f.clone(),
            g: f.clone(),
            b: f,
            a: TransferFunction::Identity,
        }
    }

    /// A convolution with a `columns × rows` kernel and every other
    /// attribute at its initial value (divisor = kernel sum, bias 0,
    /// centred target, duplicated edges, alpha convolved).
    pub fn convolve_matrix(columns: u32, rows: u32, kernel: Vec<f32>) -> Self {
        Self::ConvolveMatrix {
            order: (columns, rows),
            kernel,
            divisor: None,
            bias: 0.0,
            target: None,
            edge_mode: EdgeMode::Duplicate,
            kernel_unit_length: None,
            preserve_alpha: false,
        }
    }

    /// A noise primitive with one octave, seed `0`, no stitching.
    pub const fn turbulence(kind: TurbulenceKind, base_frequency: f32) -> Self {
        Self::Turbulence {
            base_frequency: (base_frequency, base_frequency),
            octaves: 1,
            seed: 0.0,
            stitch_tiles: false,
            kind,
        }
    }

    /// The number of inputs this kind reads: `Some(0)`, `Some(1)` or
    /// `Some(2)`, or `None` for [`Merge`](Self::Merge), which takes
    /// any number.
    pub const fn input_count(&self) -> Option<usize> {
        match self {
            Self::Flood { .. } | Self::Image { .. } | Self::Turbulence { .. } => Some(0),
            Self::Blend { .. } | Self::Composite(_) | Self::DisplacementMap { .. } => Some(2),
            Self::Merge => None,
            _ => Some(1),
        }
    }

    /// The element name of this kind in the Filter Effects vocabulary
    /// (`"feGaussianBlur"`, …), for diagnostics and writers.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Blend { .. } => "feBlend",
            Self::ColorMatrix(_) => "feColorMatrix",
            Self::ComponentTransfer { .. } => "feComponentTransfer",
            Self::Composite(_) => "feComposite",
            Self::ConvolveMatrix { .. } => "feConvolveMatrix",
            Self::DiffuseLighting { .. } => "feDiffuseLighting",
            Self::DisplacementMap { .. } => "feDisplacementMap",
            Self::DropShadow { .. } => "feDropShadow",
            Self::Flood { .. } => "feFlood",
            Self::GaussianBlur { .. } => "feGaussianBlur",
            Self::Image { .. } => "feImage",
            Self::Merge => "feMerge",
            Self::Morphology { .. } => "feMorphology",
            Self::Offset { .. } => "feOffset",
            Self::SpecularLighting { .. } => "feSpecularLighting",
            Self::Tile => "feTile",
            Self::Turbulence { .. } => "feTurbulence",
        }
    }
}

/// One node of a filter graph: an operation, where it reads its
/// inputs, the subregion it is confined to and the colour space it
/// computes in.
///
/// `#[non_exhaustive]`: build it with [`new`](Self::new) and the
/// `with_*` builders.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FilterPrimitive {
    /// The operation and its parameters.
    pub kind: FilterPrimitiveKind,
    /// The inputs, in the order the kind defines (`in`, then `in2`;
    /// for a merge, bottom layer first). Its length should equal
    /// [`FilterPrimitiveKind::input_count`] for kinds with a fixed
    /// count.
    pub inputs: Vec<FilterInput>,
    /// The primitive subregion.
    pub subregion: FilterSubregion,
    /// The colour space the operation is computed in.
    pub color_interpolation: ColorInterpolation,
}

impl FilterPrimitive {
    /// A primitive of `kind` with every input left implicit
    /// ([`FilterInput::Previous`]; none for a merge), a default
    /// subregion and linear-light RGB computation.
    pub fn new(kind: FilterPrimitiveKind) -> Self {
        let inputs = vec![FilterInput::Previous; kind.input_count().unwrap_or(0)];
        Self {
            kind,
            inputs,
            subregion: FilterSubregion::new(),
            color_interpolation: ColorInterpolation::LinearRgb,
        }
    }

    /// Builder: set input `slot` (growing the list with implicit
    /// inputs if needed).
    pub fn with_input(mut self, slot: usize, input: FilterInput) -> Self {
        if self.inputs.len() <= slot {
            self.inputs.resize(slot + 1, FilterInput::Previous);
        }
        self.inputs[slot] = input;
        self
    }

    /// Builder: replace the whole input list.
    pub fn with_inputs(mut self, inputs: impl Into<Vec<FilterInput>>) -> Self {
        self.inputs = inputs.into();
        self
    }

    /// Builder: set the primitive subregion.
    pub fn with_subregion(mut self, subregion: FilterSubregion) -> Self {
        self.subregion = subregion;
        self
    }

    /// Builder: set the colour space the operation is computed in.
    pub fn with_color_interpolation(mut self, space: ColorInterpolation) -> Self {
        self.color_interpolation = space;
        self
    }
}

/// A filter: a region and an ordered list of primitives whose last
/// result replaces the filtered content.
///
/// `new()` gives the initial values of the `filter` element: a region
/// of `-10 %, -10 %, 120 %, 120 %` of the content's bounding box and
/// primitive lengths in user space.
///
/// An effect with **no primitives** renders the filtered content as
/// transparent black — it does not pass the content through.
///
/// `#[non_exhaustive]`: build it with [`new`](Self::new) and the
/// `with_*` builders.
///
/// ```
/// # use oxideav_core::vector::{
/// #     CompositeOperator, FilterEffect, FilterInput, FilterPrimitive, FilterPrimitiveKind,
/// # };
/// # use oxideav_core::Rgba;
/// // Soft shadow: blur the alpha, offset it, tint it, put the
/// // original on top.
/// let effect = FilterEffect::new()
///     .with_primitive(
///         FilterPrimitive::new(FilterPrimitiveKind::gaussian_blur(4.0))
///             .with_input(0, FilterInput::SourceAlpha),
///     )
///     .with_primitive(FilterPrimitive::new(FilterPrimitiveKind::offset(3.0, 3.0)))
///     .with_primitive(FilterPrimitive::new(FilterPrimitiveKind::flood(Rgba::new(0, 0, 0, 128))))
///     .with_primitive(
///         FilterPrimitive::new(FilterPrimitiveKind::Composite(CompositeOperator::In))
///             .with_inputs([FilterInput::Previous, FilterInput::Result(1)]),
///     )
///     .with_primitive(
///         FilterPrimitive::new(FilterPrimitiveKind::Merge)
///             .with_inputs([FilterInput::Previous, FilterInput::SourceGraphic]),
///     );
/// assert!(effect.validate().is_ok());
/// // The implicit input of primitive 1 is the result of primitive 0.
/// assert_eq!(effect.resolve_input(1, 0), Some(FilterInput::Result(0)));
/// ```
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FilterEffect {
    /// The filter region: the rectangle the effect is confined to, in
    /// [`filter_units`](Self::filter_units). It clips every input and
    /// the final result.
    pub region: Rect,
    /// Coordinate system of [`region`](Self::region).
    pub filter_units: FilterUnits,
    /// Coordinate system of the primitives' subregions and length
    /// parameters.
    pub primitive_units: FilterUnits,
    /// The primitives, evaluated in order. The result of the last one
    /// is the output of the effect.
    pub primitives: Vec<FilterPrimitive>,
    /// The paint [`FilterInput::FillPaint`] stands for. `None` reads
    /// as transparent black.
    pub fill_paint: Option<Paint>,
    /// The paint [`FilterInput::StrokePaint`] stands for. `None` reads
    /// as transparent black.
    pub stroke_paint: Option<Paint>,
}

impl Default for FilterEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl FilterEffect {
    /// An effect with the default region and no primitives.
    pub fn new() -> Self {
        Self {
            region: Rect::new(-0.1, -0.1, 1.2, 1.2),
            filter_units: FilterUnits::ObjectBoundingBox,
            primitive_units: FilterUnits::UserSpaceOnUse,
            primitives: Vec::new(),
            fill_paint: None,
            stroke_paint: None,
        }
    }

    /// Builder: set the filter region and the units it is given in.
    pub fn with_region(mut self, region: Rect, units: FilterUnits) -> Self {
        self.region = region;
        self.filter_units = units;
        self
    }

    /// Builder: set the units of the primitives' subregions and length
    /// parameters.
    pub fn with_primitive_units(mut self, units: FilterUnits) -> Self {
        self.primitive_units = units;
        self
    }

    /// Builder: append a primitive.
    pub fn with_primitive(mut self, primitive: FilterPrimitive) -> Self {
        self.primitives.push(primitive);
        self
    }

    /// Builder: replace the primitive list.
    pub fn with_primitives(mut self, primitives: Vec<FilterPrimitive>) -> Self {
        self.primitives = primitives;
        self
    }

    /// Builder: set the paint behind [`FilterInput::FillPaint`].
    pub fn with_fill_paint(mut self, paint: Paint) -> Self {
        self.fill_paint = Some(paint);
        self
    }

    /// Builder: set the paint behind [`FilterInput::StrokePaint`].
    pub fn with_stroke_paint(mut self, paint: Paint) -> Self {
        self.stroke_paint = Some(paint);
        self
    }

    /// Append a primitive and return its index, for use in a later
    /// [`FilterInput::Result`].
    pub fn push(&mut self, primitive: FilterPrimitive) -> usize {
        self.primitives.push(primitive);
        self.primitives.len() - 1
    }

    /// The filter region as a concrete rectangle in user space, given
    /// the bounding box of the filtered content. With
    /// [`FilterUnits::UserSpaceOnUse`] that is [`region`](Self::region)
    /// itself; with [`FilterUnits::ObjectBoundingBox`] the region's
    /// fractions are applied to `bounds`.
    pub fn region_in(&self, bounds: Rect) -> Rect {
        match self.filter_units {
            FilterUnits::UserSpaceOnUse => self.region,
            FilterUnits::ObjectBoundingBox => Rect {
                x: bounds.x + self.region.x * bounds.width,
                y: bounds.y + self.region.y * bounds.height,
                width: self.region.width * bounds.width,
                height: self.region.height * bounds.height,
            },
        }
    }

    /// Input `slot` of primitive `primitive` with the implicit
    /// [`FilterInput::Previous`] replaced by what it stands for:
    /// [`FilterInput::SourceGraphic`] for the first primitive,
    /// [`FilterInput::Result`] of the preceding one otherwise. `None`
    /// when either index is out of range.
    pub fn resolve_input(&self, primitive: usize, slot: usize) -> Option<FilterInput> {
        let input = *self.primitives.get(primitive)?.inputs.get(slot)?;
        Some(match input {
            FilterInput::Previous if primitive == 0 => FilterInput::SourceGraphic,
            FilterInput::Previous => FilterInput::Result(primitive - 1),
            other => other,
        })
    }

    /// `true` when any primitive reads `input` (compared after
    /// [`resolve_input`](Self::resolve_input)). Lets a rasteriser skip
    /// preparing a standard input nobody uses — the backdrop in
    /// particular.
    pub fn reads(&self, input: FilterInput) -> bool {
        (0..self.primitives.len()).any(|p| {
            (0..self.primitives[p].inputs.len()).any(|s| self.resolve_input(p, s) == Some(input))
        })
    }

    /// Check the graph is well formed: every primitive has the number
    /// of inputs its kind requires, every [`FilterInput::Result`]
    /// refers to an earlier primitive, and a convolution kernel matches
    /// its declared order and target.
    ///
    /// Returns [`Error::InvalidData`] naming the first offending
    /// primitive. Parameter *values* (negative deviations, zero
    /// divisors, …) are not checked: the specification gives each of
    /// those a defined rendering.
    pub fn validate(&self) -> Result<()> {
        for (index, primitive) in self.primitives.iter().enumerate() {
            let name = primitive.kind.name();
            if let Some(want) = primitive.kind.input_count()
                && primitive.inputs.len() != want
            {
                return Err(Error::invalid(format!(
                    "filter primitive {index} ({name}) has {} inputs, expected {want}",
                    primitive.inputs.len()
                )));
            }
            for input in &primitive.inputs {
                if let FilterInput::Result(r) = *input
                    && r >= index
                {
                    return Err(Error::invalid(format!(
                        "filter primitive {index} ({name}) reads result {r}, which is not an earlier primitive"
                    )));
                }
            }
            if let FilterPrimitiveKind::ConvolveMatrix {
                order,
                kernel,
                target,
                ..
            } = &primitive.kind
            {
                let cells = u64::from(order.0) * u64::from(order.1);
                if cells == 0 || kernel.len() as u64 != cells {
                    return Err(Error::invalid(format!(
                        "filter primitive {index} ({name}) has a {}x{} kernel with {} values",
                        order.0,
                        order.1,
                        kernel.len()
                    )));
                }
                if let Some((tx, ty)) = *target
                    && (tx >= order.0 || ty >= order.1)
                {
                    return Err(Error::invalid(format!(
                        "filter primitive {index} ({name}) targets cell ({tx}, {ty}) outside its {}x{} kernel",
                        order.0, order.1
                    )));
                }
            }
        }
        Ok(())
    }
}

/// A filter effect applied to a subtree: the payload of
/// [`Node::Filter`](super::Node::Filter).
///
/// The subtree is rendered into an offscreen image (the source
/// graphic), the effect is evaluated, and its output is composited in
/// place of the subtree. The effect is held behind an [`Arc`] so the
/// many elements that reference one filter definition share a single
/// description. Chained filters are nested nodes: the inner node is
/// the `content` of the outer one.
///
/// `#[non_exhaustive]`: build it with [`new`](Self::new) and the
/// `with_*` builders.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FilterNode {
    /// The effect to apply.
    pub effect: Arc<FilterEffect>,
    /// The filtered subtree — normally a [`Node::Group`].
    pub content: Box<Node>,
    /// The bounding box of the content in the node's user space, as
    /// the producer's format defines it, for effects that use
    /// [`FilterUnits::ObjectBoundingBox`]. `None` leaves the
    /// rasteriser to compute the geometric bounds of `content` itself.
    pub bounds: Option<Rect>,
}

impl FilterNode {
    /// Apply `effect` to `content`.
    pub fn new(effect: impl Into<Arc<FilterEffect>>, content: Node) -> Self {
        Self {
            effect: effect.into(),
            content: Box::new(content),
            bounds: None,
        }
    }

    /// Builder: state the bounding box of the content.
    pub fn with_bounds(mut self, bounds: Rect) -> Self {
        self.bounds = Some(bounds);
        self
    }
}

impl From<FilterNode> for Node {
    fn from(node: FilterNode) -> Self {
        Node::Filter(node)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Group, ViewBox};
    use super::*;

    fn blur(sd: f32) -> FilterPrimitive {
        FilterPrimitive::new(FilterPrimitiveKind::gaussian_blur(sd))
    }

    #[test]
    fn effect_defaults_match_the_filter_element() {
        let e = FilterEffect::new();
        assert_eq!(e.region, Rect::new(-0.1, -0.1, 1.2, 1.2));
        assert_eq!(e.filter_units, FilterUnits::ObjectBoundingBox);
        assert_eq!(e.primitive_units, FilterUnits::UserSpaceOnUse);
        assert!(e.primitives.is_empty());
        assert!(e.fill_paint.is_none() && e.stroke_paint.is_none());
        assert!(e.validate().is_ok());
        let d = FilterEffect::default();
        assert_eq!(d.region, e.region);
        assert_eq!(d.filter_units, e.filter_units);
    }

    #[test]
    fn primitive_defaults_and_builders() {
        let p = blur(2.0);
        assert_eq!(p.inputs, [FilterInput::Previous]);
        assert!(p.subregion.is_default());
        assert_eq!(p.color_interpolation, ColorInterpolation::LinearRgb);
        assert_eq!(FilterInput::default(), FilterInput::Previous);

        let p = p
            .with_input(0, FilterInput::SourceAlpha)
            .with_subregion(FilterSubregion::new().with_x(1.0).with_width(10.0))
            .with_color_interpolation(ColorInterpolation::SRgb);
        assert_eq!(p.inputs, [FilterInput::SourceAlpha]);
        assert_eq!(p.subregion.x, Some(1.0));
        assert_eq!(p.subregion.y, None);
        assert_eq!(p.color_interpolation, ColorInterpolation::SRgb);
        assert_eq!(
            p.subregion.resolve(Rect::new(5.0, 6.0, 7.0, 8.0)),
            Rect::new(1.0, 6.0, 10.0, 8.0)
        );

        // with_input grows the list with implicit inputs.
        let m = FilterPrimitive::new(FilterPrimitiveKind::Merge);
        assert!(m.inputs.is_empty());
        let m = m.with_input(2, FilterInput::SourceGraphic);
        assert_eq!(
            m.inputs,
            [
                FilterInput::Previous,
                FilterInput::Previous,
                FilterInput::SourceGraphic
            ]
        );
        let full = FilterSubregion::from_rect(Rect::new(1.0, 2.0, 3.0, 4.0))
            .with_y(9.0)
            .with_height(5.0);
        assert!(!full.is_default());
        assert_eq!(full.resolve(Rect::default()), Rect::new(1.0, 9.0, 3.0, 5.0));
        assert_eq!(FilterSubregion::default(), FilterSubregion::new());
    }

    #[test]
    fn input_counts_and_names_cover_every_kind() {
        use FilterPrimitiveKind as K;
        let light = LightSource::Distant {
            azimuth: 0.0,
            elevation: 0.0,
        };
        let kinds: Vec<(K, Option<usize>, &str)> = vec![
            (K::blend(BlendMode::Multiply), Some(2), "feBlend"),
            (
                K::ColorMatrix(ColorMatrixKind::IDENTITY),
                Some(1),
                "feColorMatrix",
            ),
            (
                K::component_transfer_rgb(TransferFunction::Identity),
                Some(1),
                "feComponentTransfer",
            ),
            (
                K::Composite(CompositeOperator::default()),
                Some(2),
                "feComposite",
            ),
            (
                K::convolve_matrix(3, 3, vec![0.0; 9]),
                Some(1),
                "feConvolveMatrix",
            ),
            (
                K::DiffuseLighting {
                    surface_scale: 1.0,
                    diffuse_constant: 1.0,
                    kernel_unit_length: None,
                    color: Rgba::opaque(255, 255, 255),
                    light,
                },
                Some(1),
                "feDiffuseLighting",
            ),
            (
                K::DisplacementMap {
                    scale: 0.0,
                    x_channel: ColorChannel::A,
                    y_channel: ColorChannel::A,
                },
                Some(2),
                "feDisplacementMap",
            ),
            (
                K::drop_shadow(2.0, 2.0, 2.0, Rgba::opaque(0, 0, 0)),
                Some(1),
                "feDropShadow",
            ),
            (K::flood(Rgba::default()), Some(0), "feFlood"),
            (K::gaussian_blur(1.0), Some(1), "feGaussianBlur"),
            (
                K::Image {
                    content: Box::new(Node::Group(Group::new())),
                    preserve_aspect: PreserveAspectRatio::default(),
                },
                Some(0),
                "feImage",
            ),
            (K::Merge, None, "feMerge"),
            (
                K::Morphology {
                    operator: MorphologyOperator::default(),
                    radius: (0.0, 0.0),
                },
                Some(1),
                "feMorphology",
            ),
            (K::offset(0.0, 0.0), Some(1), "feOffset"),
            (
                K::SpecularLighting {
                    surface_scale: 1.0,
                    specular_constant: 1.0,
                    specular_exponent: 1.0,
                    kernel_unit_length: None,
                    color: Rgba::opaque(255, 255, 255),
                    light,
                },
                Some(1),
                "feSpecularLighting",
            ),
            (K::Tile, Some(1), "feTile"),
            (
                K::turbulence(TurbulenceKind::default(), 0.0),
                Some(0),
                "feTurbulence",
            ),
        ];
        assert_eq!(kinds.len(), 17);
        for (kind, count, name) in kinds {
            assert_eq!(kind.input_count(), count, "{name}");
            assert_eq!(kind.name(), name);
            // new() provides exactly the inputs the kind needs.
            let p = FilterPrimitive::new(kind);
            assert_eq!(p.inputs.len(), count.unwrap_or(0));
            assert!(FilterEffect::new().with_primitive(p).validate().is_ok());
        }
    }

    #[test]
    fn enum_defaults_are_the_initial_attribute_values() {
        assert_eq!(BlendMode::default(), BlendMode::Normal);
        assert_eq!(CompositeOperator::default(), CompositeOperator::Over);
        assert_eq!(MorphologyOperator::default(), MorphologyOperator::Erode);
        assert_eq!(TurbulenceKind::default(), TurbulenceKind::Turbulence);
        assert_eq!(TransferFunction::default(), TransferFunction::Identity);
        match FilterPrimitiveKind::gaussian_blur(0.0) {
            FilterPrimitiveKind::GaussianBlur { edge_mode, .. } => {
                assert_eq!(edge_mode, EdgeMode::None)
            }
            _ => unreachable!(),
        }
        match FilterPrimitiveKind::convolve_matrix(3, 3, vec![1.0; 9]) {
            FilterPrimitiveKind::ConvolveMatrix {
                divisor,
                bias,
                target,
                edge_mode,
                preserve_alpha,
                ..
            } => {
                assert_eq!(divisor, None);
                assert_eq!(bias, 0.0);
                assert_eq!(target, None);
                assert_eq!(edge_mode, EdgeMode::Duplicate);
                assert!(!preserve_alpha);
            }
            _ => unreachable!(),
        }
        match FilterPrimitiveKind::turbulence(TurbulenceKind::FractalNoise, 0.05) {
            FilterPrimitiveKind::Turbulence {
                base_frequency,
                octaves,
                seed,
                stitch_tiles,
                kind,
            } => {
                assert_eq!(base_frequency, (0.05, 0.05));
                assert_eq!(octaves, 1);
                assert_eq!(seed, 0.0);
                assert!(!stitch_tiles);
                assert_eq!(kind, TurbulenceKind::FractalNoise);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn previous_resolves_to_source_or_preceding_result() {
        let e = FilterEffect::new()
            .with_primitive(blur(1.0))
            .with_primitive(blur(2.0))
            .with_primitive(
                FilterPrimitive::new(FilterPrimitiveKind::blend(BlendMode::Screen))
                    .with_inputs([FilterInput::Result(0), FilterInput::BackgroundImage]),
            );
        assert_eq!(e.resolve_input(0, 0), Some(FilterInput::SourceGraphic));
        assert_eq!(e.resolve_input(1, 0), Some(FilterInput::Result(0)));
        assert_eq!(e.resolve_input(2, 0), Some(FilterInput::Result(0)));
        assert_eq!(e.resolve_input(2, 1), Some(FilterInput::BackgroundImage));
        assert_eq!(e.resolve_input(2, 2), None);
        assert_eq!(e.resolve_input(3, 0), None);
        assert!(e.reads(FilterInput::SourceGraphic));
        assert!(e.reads(FilterInput::BackgroundImage));
        assert!(e.reads(FilterInput::Result(0)));
        assert!(!e.reads(FilterInput::Result(1)));
        assert!(!e.reads(FilterInput::SourceAlpha));
        assert!(!e.reads(FilterInput::Previous));
        assert!(e.validate().is_ok());
    }

    #[test]
    fn validate_rejects_malformed_graphs() {
        // Wrong arity.
        let e = FilterEffect::new().with_primitive(
            FilterPrimitive::new(FilterPrimitiveKind::blend(BlendMode::Normal))
                .with_inputs([FilterInput::SourceGraphic]),
        );
        let err = e.validate().unwrap_err();
        assert!(matches!(err, Error::InvalidData(_)), "{err:?}");
        assert!(err.to_string().contains("feBlend"), "{err}");
        // Self and forward references.
        for bad in [0usize, 1, 5] {
            let e = FilterEffect::new()
                .with_primitive(blur(1.0).with_input(0, FilterInput::Result(bad)));
            assert!(e.validate().is_err(), "result {bad}");
        }
        // Kernel size mismatch, empty kernel, target outside.
        for kind in [
            FilterPrimitiveKind::convolve_matrix(3, 3, vec![0.0; 8]),
            FilterPrimitiveKind::convolve_matrix(0, 3, vec![]),
            FilterPrimitiveKind::ConvolveMatrix {
                order: (3, 3),
                kernel: vec![0.0; 9],
                divisor: None,
                bias: 0.0,
                target: Some((3, 0)),
                edge_mode: EdgeMode::Wrap,
                kernel_unit_length: None,
                preserve_alpha: true,
            },
        ] {
            let e = FilterEffect::new().with_primitive(FilterPrimitive::new(kind));
            assert!(e.validate().is_err());
        }
        // A merge accepts any number of inputs, including none.
        let e =
            FilterEffect::new().with_primitive(FilterPrimitive::new(FilterPrimitiveKind::Merge));
        assert!(e.validate().is_ok());
    }

    #[test]
    fn push_returns_indices_usable_as_results() {
        let mut e = FilterEffect::new();
        let a = e.push(blur(3.0).with_input(0, FilterInput::SourceAlpha));
        let b = e.push(FilterPrimitive::new(FilterPrimitiveKind::offset(2.0, 2.0)));
        let c = e.push(
            FilterPrimitive::new(FilterPrimitiveKind::Merge)
                .with_inputs([FilterInput::Result(b), FilterInput::SourceGraphic]),
        );
        assert_eq!((a, b, c), (0, 1, 2));
        assert!(e.validate().is_ok());
        let replaced = e.clone().with_primitives(vec![blur(1.0)]);
        assert_eq!(replaced.primitives.len(), 1);
    }

    #[test]
    fn region_resolves_against_the_bounding_box() {
        let bounds = Rect::new(100.0, 50.0, 200.0, 80.0);
        let e = FilterEffect::new();
        let r = e.region_in(bounds);
        assert!((r.x - 80.0).abs() < 1e-4 && (r.y - 42.0).abs() < 1e-4);
        assert!((r.width - 240.0).abs() < 1e-4 && (r.height - 96.0).abs() < 1e-4);
        let user = FilterEffect::new()
            .with_region(Rect::new(1.0, 2.0, 3.0, 4.0), FilterUnits::UserSpaceOnUse)
            .with_primitive_units(FilterUnits::ObjectBoundingBox);
        assert_eq!(user.region_in(bounds), Rect::new(1.0, 2.0, 3.0, 4.0));
        assert_eq!(user.primitive_units, FilterUnits::ObjectBoundingBox);
    }

    #[test]
    fn context_paints_are_carried() {
        let e = FilterEffect::new()
            .with_fill_paint(Paint::Solid(Rgba::opaque(1, 2, 3)))
            .with_stroke_paint(Paint::Solid(Rgba::opaque(4, 5, 6)));
        assert!(matches!(e.fill_paint, Some(Paint::Solid(c)) if c == Rgba::opaque(1, 2, 3)));
        assert!(matches!(e.stroke_paint, Some(Paint::Solid(c)) if c == Rgba::opaque(4, 5, 6)));
    }

    #[test]
    fn transfer_functions_follow_their_formulas() {
        let id = TransferFunction::Identity;
        assert_eq!(id.apply(0.3), 0.3);
        assert_eq!(id.apply(-1.0), 0.0);
        assert_eq!(id.apply(2.0), 1.0);
        // table: n = 3 regions over v0..v3.
        let t = TransferFunction::Table(vec![0.0, 0.3, 0.9, 1.0]);
        assert_eq!(t.apply(0.0), 0.0);
        assert!((t.apply(1.0 / 3.0) - 0.3).abs() < 1e-6);
        assert!((t.apply(0.5) - 0.6).abs() < 1e-6);
        assert_eq!(t.apply(1.0), 1.0);
        assert_eq!(TransferFunction::Table(vec![]).apply(0.4), 0.4);
        assert_eq!(TransferFunction::Table(vec![0.7]).apply(0.4), 0.7);
        // discrete: n = 4 steps.
        let d = TransferFunction::Discrete(vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(d.apply(0.0), 0.1);
        assert_eq!(d.apply(0.24), 0.1);
        assert_eq!(d.apply(0.25), 0.2);
        assert_eq!(d.apply(0.99), 0.4);
        assert_eq!(d.apply(1.0), 0.4);
        assert_eq!(TransferFunction::Discrete(vec![]).apply(0.4), 0.4);
        let l = TransferFunction::Linear {
            slope: 2.0,
            intercept: -0.25,
        };
        assert_eq!(l.apply(0.5), 0.75);
        let g = TransferFunction::Gamma {
            amplitude: 2.0,
            exponent: 2.0,
            offset: 0.5,
        };
        assert_eq!(g.apply(0.5), 1.0);
    }

    #[test]
    fn filter_node_wraps_content_and_shares_the_effect() {
        let effect = Arc::new(FilterEffect::new().with_primitive(blur(2.0)));
        let a = FilterNode::new(effect.clone(), Node::Group(Group::new()));
        let b = FilterNode::new(effect.clone(), Node::Group(Group::new().with_opacity(0.5)))
            .with_bounds(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(Arc::ptr_eq(&a.effect, &b.effect));
        assert!(a.bounds.is_none());
        assert_eq!(b.bounds, Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
        // An owned effect converts too, and the node nests for chains.
        let inner: Node = FilterNode::new(FilterEffect::new(), Node::Group(Group::new())).into();
        let outer: Node = FilterNode::new(effect, inner).into();
        match &outer {
            Node::Filter(f) => {
                assert_eq!(f.effect.primitives.len(), 1);
                assert!(matches!(f.content.as_ref(), Node::Filter(_)));
            }
            _ => panic!("expected Node::Filter"),
        }
        let cloned = outer.clone();
        assert!(matches!(cloned, Node::Filter(_)));
    }

    #[test]
    fn image_primitive_carries_a_subtree_and_fit() {
        let kind = FilterPrimitiveKind::Image {
            content: Box::new(Node::Group(Group::new())),
            preserve_aspect: PreserveAspectRatio::default(),
        };
        let p = FilterPrimitive::new(kind)
            .with_subregion(FilterSubregion::from_rect(Rect::new(0.0, 0.0, 20.0, 10.0)));
        match &p.kind {
            FilterPrimitiveKind::Image {
                preserve_aspect, ..
            } => {
                // A 10×10 image fitted "xMidYMid meet" into 20×10.
                let t = preserve_aspect.viewport_transform(
                    ViewBox::new(0.0, 0.0, 10.0, 10.0),
                    p.subregion.resolve(Rect::default()),
                );
                assert_eq!((t.a, t.d, t.e, t.f), (1.0, 1.0, 5.0, 0.0));
            }
            _ => unreachable!(),
        }
    }
}

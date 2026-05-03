//! Core types and registries for the oxideav framework.
//!
//! This crate is the dependency-light foundation: primitive types
//! (timestamps, packets, frames, media formats) plus the registries
//! every sibling crate registers itself into. The aggregate
//! [`RuntimeContext`] bundles all four registries (codec / container /
//! source / filter) into a single value that consumers pass around.

pub mod arena;
pub mod bits;
pub mod capabilities;
pub mod error;
pub mod execution;
pub mod filter;
pub mod format;
pub mod frame;
pub mod limits;
pub mod options;
pub mod packet;
pub mod picture;
pub mod rational;
pub mod registry;
pub mod stream;
pub mod subtitle;
pub mod time;
pub mod vector;

pub use capabilities::{CodecCapabilities, CodecPreferences, DEFAULT_PRIORITY};
pub use error::{Error, Result};
pub use execution::ExecutionContext;
pub use filter::{FilterContext, PortParams, PortSpec, StreamFilter};
pub use format::{
    ChannelLayout, ChannelPosition, MediaType, ParseChannelLayoutError, PixelFormat, SampleFormat,
};
pub use frame::{AudioFrame, Frame, VideoFrame, VideoPlane};
pub use limits::DecoderLimits;
pub use options::{
    parse_options, CodecOptions, CodecOptionsStruct, OptionField, OptionKind, OptionValue,
};
pub use packet::Packet;
pub use picture::{AttachedPicture, PictureType};
pub use rational::Rational;
pub use registry::{
    BytesSource, CodecImplementation, CodecInfo, CodecRegistry, ContainerProbeFn,
    ContainerRegistry, Decoder, DecoderFactory, Demuxer, Encoder, EncoderFactory, FilterFactory,
    FilterRegistry, FrameSource, Muxer, OpenBytesFn, OpenDemuxerFn, OpenFramesFn, OpenMuxerFn,
    OpenPacketsFn, PacketSource, ProbeData, ProbeScore, ReadSeek, RuntimeContext, SourceOutput,
    SourceRegistry, WriteSeek, MAX_PROBE_SCORE, PROBE_SCORE_EXTENSION,
};
pub use stream::{
    CodecId, CodecParameters, CodecResolver, CodecTag, Confidence, NullCodecResolver, ProbeContext,
    ProbeFn, StreamInfo,
};
pub use subtitle::{CuePosition, Segment, SubtitleCue, SubtitleStyle, TextAlign};
pub use time::{TimeBase, Timestamp};
pub use vector::{
    DashPattern, FillRule, GradientStop, Group, ImageRef, LineCap, LineJoin, LinearGradient, Node,
    Paint, Path, PathCommand, PathNode, Point, RadialGradient, Rect, Rgba, SpreadMethod, Stroke,
    Transform2D, VectorFrame, ViewBox,
};

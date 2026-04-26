//! Uncompressed audio and video frames.

use crate::subtitle::SubtitleCue;

/// A decoded chunk of uncompressed data: either audio samples, a video
/// picture, or (for subtitle streams) a single styled cue.
///
/// Marked `#[non_exhaustive]` — consumers that match on variants must
/// include a wildcard arm. This lets the crate add new frame kinds (data
/// tracks, hap rops, …) without breaking downstream code.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Frame {
    Audio(AudioFrame),
    Video(VideoFrame),
    /// A single subtitle cue. Timing is carried inside the cue itself
    /// (`start_us`/`end_us`) so it's independent of container time bases,
    /// but the enclosing pipeline/muxer can still rescale via `pts` at
    /// the packet layer.
    Subtitle(SubtitleCue),
}

impl Frame {
    pub fn pts(&self) -> Option<i64> {
        match self {
            Self::Audio(a) => a.pts,
            Self::Video(v) => v.pts,
            Self::Subtitle(s) => Some(s.start_us),
        }
    }
}

/// Uncompressed audio frame.
///
/// Stream-level properties (sample format, channel count, sample rate,
/// time base) are NOT carried per-frame — read them from the stream's
/// [`CodecParameters`](crate::CodecParameters). Frames stay lightweight
/// because real-time playback moves thousands per second per stream.
///
/// Sample layout is determined by the stream's `SampleFormat`:
/// - Interleaved formats: `data` has one plane; samples are stored as
///   `ch0 ch1 ... chN ch0 ch1 ... chN ...`.
/// - Planar formats: `data` has one plane per channel.
///
/// Use [`SampleFormat::plane_count`](crate::SampleFormat::plane_count)
/// with the stream's channel count to compute the expected `data.len()`.
#[derive(Clone, Debug)]
pub struct AudioFrame {
    /// Number of samples *per channel* in this frame. Variable per-frame
    /// for VBR codecs and on partial flushes.
    pub samples: u32,
    pub pts: Option<i64>,
    /// Raw sample bytes. Length matches `format.plane_count(channels)`
    /// from the stream's `CodecParameters`.
    pub data: Vec<Vec<u8>>,
}

/// Uncompressed video frame.
///
/// Stream-level properties (pixel format, width, height, time base) are
/// NOT carried per-frame — read them from the stream's
/// [`CodecParameters`](crate::CodecParameters). Frames stay lightweight
/// because real-time playback moves thousands per second per stream.
#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub pts: Option<i64>,
    /// One entry per plane (e.g., 3 for Yuv420P). Each entry is `(stride, bytes)`.
    pub planes: Vec<VideoPlane>,
}

#[derive(Clone, Debug)]
pub struct VideoPlane {
    /// Bytes per row in `data`.
    pub stride: usize,
    pub data: Vec<u8>,
}

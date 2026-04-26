//! Uncompressed audio and video frames.

use crate::format::{ChannelLayout, PixelFormat, SampleFormat};
use crate::subtitle::SubtitleCue;
use crate::time::TimeBase;

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

    pub fn time_base(&self) -> TimeBase {
        match self {
            Self::Audio(a) => a.time_base,
            Self::Video(v) => v.time_base,
            // Subtitle cues carry raw microseconds. Expose a 1/1_000_000
            // base so the value lines up with the pts() result above.
            Self::Subtitle(_) => TimeBase::new(1, 1_000_000),
        }
    }
}

/// Uncompressed audio frame.
///
/// Sample layout is determined by `format`:
/// - Interleaved formats: `data` has one plane; samples are stored as
///   `ch0 ch1 ... chN ch0 ch1 ... chN ...`.
/// - Planar formats: `data` has one plane per channel.
///
/// Speaker layout is *derived* from `channels` via
/// [`ChannelLayout::from_count`] in the [`Self::layout`] accessor. A
/// future revision can add an optional `channel_layout` field for codecs
/// that decode an explicit layout from their bitstream; until then, all
/// downstream codecs that produce an [`AudioFrame`] keep working
/// unchanged and downmix / device-routing filters can read
/// `frame.layout()` to get a `ChannelLayout` value.
#[derive(Clone, Debug)]
pub struct AudioFrame {
    pub format: SampleFormat,
    pub channels: u16,
    pub sample_rate: u32,
    /// Number of samples *per channel*.
    pub samples: u32,
    pub pts: Option<i64>,
    pub time_base: TimeBase,
    /// Raw sample bytes. `.len() == planes()` — i.e. one element per plane.
    pub data: Vec<Vec<u8>>,
}

impl AudioFrame {
    pub fn planes(&self) -> usize {
        if self.format.is_planar() {
            self.channels as usize
        } else {
            1
        }
    }

    /// Effective speaker layout, inferred from [`Self::channels`] via
    /// [`ChannelLayout::from_count`].
    ///
    /// All `AudioFrame` instances "know" their layout: counts in 1..=8
    /// resolve to a named layout (`Mono`, `Stereo`, `Surround51`, …),
    /// anything else falls through to `ChannelLayout::DiscreteN(n)`. This
    /// derived approach means existing codecs that build an `AudioFrame`
    /// via struct-literal syntax continue to compile unchanged — the
    /// layout machinery layered on top in [`crate::format`] supplies the
    /// missing structure on demand.
    pub fn layout(&self) -> ChannelLayout {
        ChannelLayout::from_count(self.channels)
    }
}

/// Uncompressed video frame.
#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub pts: Option<i64>,
    pub time_base: TimeBase,
    /// One entry per plane (e.g., 3 for Yuv420P). Each entry is `(stride, bytes)`.
    pub planes: Vec<VideoPlane>,
}

#[derive(Clone, Debug)]
pub struct VideoPlane {
    /// Bytes per row in `data`.
    pub stride: usize,
    pub data: Vec<u8>,
}

#[cfg(test)]
mod audio_frame_layout_tests {
    use super::*;

    fn af(channels: u16) -> AudioFrame {
        AudioFrame {
            format: SampleFormat::S16,
            channels,
            sample_rate: 48_000,
            samples: 1024,
            pts: None,
            time_base: TimeBase::new(1, 48_000),
            data: vec![Vec::new()],
        }
    }

    #[test]
    fn layout_infers_known_named_layouts_from_channel_count() {
        assert_eq!(af(1).layout(), ChannelLayout::Mono);
        assert_eq!(af(2).layout(), ChannelLayout::Stereo);
        assert_eq!(af(6).layout(), ChannelLayout::Surround51);
        assert_eq!(af(8).layout(), ChannelLayout::Surround71);
    }

    #[test]
    fn layout_falls_through_to_discrete_for_unusual_counts() {
        assert_eq!(af(13).layout(), ChannelLayout::DiscreteN(13));
        assert_eq!(af(0).layout(), ChannelLayout::DiscreteN(0));
    }
}

//! Uncompressed audio and video frames.

use crate::subtitle::SubtitleCue;
use crate::vector::VectorFrame;

/// A decoded chunk of uncompressed data: either audio samples, a video
/// picture, or (for subtitle streams) a single styled cue.
///
/// Marked `#[non_exhaustive]` — consumers that match on variants must
/// include a wildcard arm. This lets the crate add new frame kinds (data
/// tracks, hap rops, …) without breaking downstream code.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Frame {
    /// Uncompressed audio samples.
    Audio(AudioFrame),
    /// One uncompressed video picture.
    Video(VideoFrame),
    /// A single subtitle cue. Timing is carried inside the cue itself
    /// (`start_us`/`end_us`) so it's independent of container time bases,
    /// but the enclosing pipeline/muxer can still rescale via `pts` at
    /// the packet layer.
    Subtitle(SubtitleCue),
    /// A resolution-independent vector-graphics frame. Produced by
    /// vector-format decoders (`oxideav-svg`, the vector path of
    /// `oxideav-pdf`) and consumed by vector renderers / writers.
    /// See [`crate::vector`] for the full primitive set.
    Vector(VectorFrame),
}

impl Frame {
    /// Presentation timestamp of the frame in its stream's time base
    /// (a subtitle cue reports its `start_us`); `None` if unknown.
    pub fn pts(&self) -> Option<i64> {
        match self {
            Self::Audio(a) => a.pts,
            Self::Video(v) => v.pts,
            Self::Subtitle(s) => Some(s.start_us),
            Self::Vector(v) => v.pts,
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
    /// Presentation timestamp in the stream's time base; `None` if unknown.
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
///
/// # Palette side-channel
///
/// Frames in a palette-indexed format
/// ([`PixelFormat::Pal8`](crate::PixelFormat::Pal8)) can carry their
/// color table in-band as an optional side-channel: a trailing
/// [`VideoPlane`] whose `stride` is `0` and whose `data` is non-empty.
/// That shape is impossible for an image plane (an image plane's `data`
/// is `stride × rows` long, so a zero stride forces empty data), which
/// makes the sentinel unambiguous. Use [`palette`](Self::palette) /
/// [`set_palette`](Self::set_palette) to read and attach it, and
/// [`image_planes`](Self::image_planes) when iterating pixel data on a
/// frame that may carry one — frames without an attached palette are
/// byte-for-byte identical to what they always were.
#[derive(Clone, Debug)]
pub struct VideoFrame {
    /// Presentation timestamp in the stream's time base; `None` if unknown.
    pub pts: Option<i64>,
    /// One entry per plane (e.g., 3 for Yuv420P). Each entry is `(stride, bytes)`.
    ///
    /// May additionally end with a palette side-channel entry (`stride
    /// == 0`, non-empty `data`) — see the type-level docs. Code that
    /// wants only pixel planes should iterate
    /// [`image_planes`](Self::image_planes) instead of this field.
    pub planes: Vec<VideoPlane>,
}

impl VideoFrame {
    /// `true` when the trailing entry of `planes` is a palette
    /// side-channel (`stride == 0`, non-empty `data`) rather than an
    /// image plane.
    fn trailing_plane_is_palette(&self) -> bool {
        self.planes
            .last()
            .is_some_and(|p| p.stride == 0 && !p.data.is_empty())
    }

    /// The frame's attached palette, if any.
    ///
    /// Returns the raw bytes of the palette side-channel (see the
    /// type-level docs): packed 3-byte RGB entries, entry `i` at bytes
    /// `3*i .. 3*i + 3` in R, G, B order. A full
    /// [`Pal8`](crate::PixelFormat::Pal8) table is 256 entries
    /// (768 bytes), but producers may attach fewer when the source
    /// image declares a shorter table; indices at or beyond
    /// `len / 3` are undefined by this frame and up to the consumer's
    /// missing-entry policy (typically black).
    pub fn palette(&self) -> Option<&[u8]> {
        if self.trailing_plane_is_palette() {
            self.planes.last().map(|p| p.data.as_slice())
        } else {
            None
        }
    }

    /// The RGB triplet for palette entry `index`, or `None` when no
    /// palette is attached or the attached table is too short to cover
    /// `index`. Sugar over [`palette`](Self::palette) for per-pixel
    /// lookups.
    pub fn palette_rgb(&self, index: u8) -> Option<[u8; 3]> {
        let pal = self.palette()?;
        let at = usize::from(index) * 3;
        let entry = pal.get(at..at + 3)?;
        Some([entry[0], entry[1], entry[2]])
    }

    /// Attach (or replace) the frame's palette side-channel.
    ///
    /// `rgb` is packed 3-byte RGB entries — see
    /// [`palette`](Self::palette) for the exact layout; pass a length
    /// that is a multiple of 3 (up to 768 bytes for a full 256-entry
    /// [`Pal8`](crate::PixelFormat::Pal8) table). The bytes are stored
    /// verbatim. An empty `rgb` removes any attached palette instead
    /// (the sentinel requires non-empty data), leaving the frame
    /// exactly as it was before any palette was attached.
    pub fn set_palette(&mut self, rgb: Vec<u8>) {
        if self.trailing_plane_is_palette() {
            self.planes.pop();
        }
        if !rgb.is_empty() {
            self.planes.push(VideoPlane {
                stride: 0,
                data: rgb,
            });
        }
    }

    /// Builder-style counterpart to [`set_palette`](Self::set_palette)
    /// for construction chains:
    /// `VideoFrame { pts, planes }.with_palette(rgb)`.
    pub fn with_palette(mut self, rgb: Vec<u8>) -> Self {
        self.set_palette(rgb);
        self
    }

    /// Detach and return the frame's palette side-channel, if any.
    /// Afterwards the frame carries image planes only.
    pub fn take_palette(&mut self) -> Option<Vec<u8>> {
        if self.trailing_plane_is_palette() {
            self.planes.pop().map(|p| p.data)
        } else {
            None
        }
    }

    /// The frame's image planes — `planes` with the palette
    /// side-channel (if any) excluded. Prefer this over indexing
    /// `planes` directly in code that handles palette-capable formats.
    pub fn image_planes(&self) -> &[VideoPlane] {
        let n = self.image_plane_count();
        &self.planes[..n]
    }

    /// Number of image planes (excludes the palette side-channel).
    /// Matches the stream pixel format's
    /// [`plane_count`](crate::PixelFormat::plane_count) for well-formed
    /// frames.
    pub fn image_plane_count(&self) -> usize {
        self.planes.len() - usize::from(self.trailing_plane_is_palette())
    }
}

/// One plane of a [`VideoFrame`]: row-major sample bytes plus the
/// stride between rows.
///
/// An entry with `stride == 0` and non-empty `data` is not an image
/// plane: it is the palette side-channel sentinel described on
/// [`VideoFrame`] (only meaningful as the trailing entry of
/// `VideoFrame::planes`).
#[derive(Clone, Debug)]
pub struct VideoPlane {
    /// Bytes per row in `data`.
    pub stride: usize,
    /// Raw plane bytes, `stride × rows` long (rows may carry padding
    /// beyond the visible width).
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray_frame() -> VideoFrame {
        // 4×2 Gray8 image plane.
        VideoFrame {
            pts: Some(7),
            planes: vec![VideoPlane {
                stride: 4,
                data: vec![0u8; 8],
            }],
        }
    }

    /// A full 256-entry table where entry i is (i, !i, i^0x55).
    fn full_palette() -> Vec<u8> {
        (0u16..256)
            .flat_map(|i| {
                let i = i as u8;
                [i, !i, i ^ 0x55]
            })
            .collect()
    }

    #[test]
    fn frame_without_palette_reports_none_and_full_image_planes() {
        let f = gray_frame();
        assert_eq!(f.palette(), None);
        assert_eq!(f.palette_rgb(0), None);
        assert_eq!(f.image_plane_count(), 1);
        assert_eq!(f.image_planes().len(), 1);
        assert_eq!(f.image_planes()[0].stride, 4);
    }

    #[test]
    fn set_palette_round_trips_and_keeps_image_planes_intact() {
        let mut f = gray_frame();
        let pal = full_palette();
        f.set_palette(pal.clone());

        assert_eq!(f.palette(), Some(pal.as_slice()));
        // Image-plane view is unchanged by the side-channel.
        assert_eq!(f.image_plane_count(), 1);
        assert_eq!(f.image_planes()[0].data.len(), 8);
        // The raw field sees the sentinel entry at the tail.
        assert_eq!(f.planes.len(), 2);
        assert_eq!(f.planes[1].stride, 0);

        // Entry lookup: entry i is (i, !i, i ^ 0x55) by construction.
        assert_eq!(f.palette_rgb(0), Some([0x00, 0xFF, 0x55]));
        assert_eq!(f.palette_rgb(0xAB), Some([0xAB, 0x54, 0xFE]));
        assert_eq!(f.palette_rgb(255), Some([0xFF, 0x00, 0xAA]));
    }

    #[test]
    fn set_palette_replaces_existing_table() {
        let mut f = gray_frame();
        f.set_palette(vec![1, 2, 3]);
        f.set_palette(vec![9, 8, 7, 6, 5, 4]);
        // Replacement, not stacking: one image plane + one sentinel.
        assert_eq!(f.planes.len(), 2);
        assert_eq!(f.palette(), Some(&[9, 8, 7, 6, 5, 4][..]));
        assert_eq!(f.palette_rgb(1), Some([6, 5, 4]));
    }

    #[test]
    fn short_palette_covers_only_its_entries() {
        let f = gray_frame().with_palette(vec![10, 20, 30, 40, 50, 60]);
        assert_eq!(f.palette_rgb(0), Some([10, 20, 30]));
        assert_eq!(f.palette_rgb(1), Some([40, 50, 60]));
        // Beyond the table: undefined by the frame → None.
        assert_eq!(f.palette_rgb(2), None);
        assert_eq!(f.palette_rgb(255), None);
    }

    #[test]
    fn empty_palette_clears_and_take_palette_detaches() {
        let mut f = gray_frame();
        f.set_palette(vec![1, 2, 3]);
        assert!(f.palette().is_some());

        // Empty input removes the side-channel entirely.
        f.set_palette(Vec::new());
        assert_eq!(f.palette(), None);
        assert_eq!(f.planes.len(), 1);

        // take_palette detaches and returns the bytes.
        f.set_palette(vec![4, 5, 6]);
        assert_eq!(f.take_palette(), Some(vec![4, 5, 6]));
        assert_eq!(f.palette(), None);
        assert_eq!(f.take_palette(), None);
        assert_eq!(f.planes.len(), 1);
    }

    #[test]
    fn zero_stride_empty_plane_is_not_mistaken_for_a_palette() {
        // stride == 0 with EMPTY data is the degenerate (but
        // contract-consistent) empty image plane, not the sentinel.
        let f = VideoFrame {
            pts: None,
            planes: vec![
                VideoPlane {
                    stride: 4,
                    data: vec![0u8; 8],
                },
                VideoPlane {
                    stride: 0,
                    data: Vec::new(),
                },
            ],
        };
        assert_eq!(f.palette(), None);
        assert_eq!(f.image_plane_count(), 2);
    }

    #[test]
    fn palette_on_frame_without_image_planes() {
        // A palette can be attached before pixel planes exist (encoder
        // scaffolding); the image-plane view is then empty.
        let f = VideoFrame {
            pts: None,
            planes: Vec::new(),
        }
        .with_palette(vec![1, 2, 3]);
        assert_eq!(f.palette(), Some(&[1, 2, 3][..]));
        assert_eq!(f.image_plane_count(), 0);
        assert!(f.image_planes().is_empty());
    }

    #[test]
    fn palette_survives_clone_and_frame_wrapping() {
        let f = gray_frame().with_palette(full_palette());
        let cloned = f.clone();
        assert_eq!(cloned.palette(), f.palette());

        // Through the Frame enum, pts and palette both survive.
        let wrapped = Frame::Video(cloned);
        assert_eq!(wrapped.pts(), Some(7));
        if let Frame::Video(v) = wrapped {
            assert_eq!(v.palette().map(<[u8]>::len), Some(768));
        } else {
            unreachable!("wrapped as Video above");
        }
    }
}

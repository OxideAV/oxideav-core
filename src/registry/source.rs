//! Generic source registry.
//!
//! `SourceRegistry` maps URI schemes (`file`, `http`, `rtmp`, `generate`,
//! …) to opener functions and dispatches `open(uri)` to the right driver.
//! A driver opens a URI as one of three shapes:
//!
//! * [`BytesSource`] — a `Read + Seek` byte stream that downstream code
//!   then passes to a container demuxer (the historical shape, used by
//!   `file://` and `http(s)://`).
//! * [`PacketSource`] — a producer of already-demuxed [`Packet`]s. Used
//!   by transport-layer protocols that do their own demux (RTMP, future
//!   SRT / WebRTC). Skips the container layer entirely.
//! * [`FrameSource`] — a producer of already-decoded [`Frame`]s. Used by
//!   synthetic generators that emit frames natively, skipping both the
//!   container and decoder stages.
//!
//! The driver picks the variant when it registers; [`SourceRegistry::open`]
//! returns the corresponding [`SourceOutput`] enum so the pipeline
//! executor can branch on the source shape.

use std::collections::HashMap;
use std::io::{Read, Seek};

use crate::{CodecParameters, Error, Frame, Packet, Result, StreamInfo};

// ───────────────────────── traits ─────────────────────────

/// A seekable byte stream (`Read + Seek + Send`). Replaces the historical
/// `Box<dyn ReadSeek>` opener-return type with a name that mirrors the
/// other source-shape traits in this module. Blanket-implemented for
/// every type that satisfies the bounds, so existing readers (files,
/// `Cursor<Vec<u8>>`, HTTP-over-Range adapters) work unchanged.
pub trait BytesSource: Read + Seek + Send {}
impl<T: Read + Seek + Send> BytesSource for T {}

/// A producer of already-demuxed [`Packet`]s.
///
/// Used by transport-layer protocols that perform demux themselves
/// (RTMP, RTSP, …). The pipeline executor consumes packets directly,
/// skipping the container-demux stage that bytes-shape sources go
/// through.
pub trait PacketSource: Send {
    /// Streams advertised by this source. Stable across the lifetime of
    /// the source.
    fn streams(&self) -> &[StreamInfo];

    /// Read the next packet from any stream. Returns [`Error::Eof`] at
    /// end of stream.
    fn next_packet(&mut self) -> Result<Packet>;

    /// Source-level metadata as ordered (key, value) pairs. Default is
    /// empty.
    fn metadata(&self) -> &[(String, String)] {
        &[]
    }

    /// Source-level duration in microseconds, if known. Default is
    /// `None`. Live sources (RTMP push, etc.) typically return `None`.
    fn duration_micros(&self) -> Option<i64> {
        None
    }
}

/// A producer of already-decoded [`Frame`]s.
///
/// Used by synthetic generators (testsrc, sine sweep, gradient image,
/// …) that emit decoded frames natively. The pipeline executor consumes
/// frames directly, skipping both the container-demux and decode stages.
pub trait FrameSource: Send {
    /// Codec parameters describing the frames this source emits. Stable
    /// across the lifetime of the source. Even though the frames are
    /// already decoded, downstream filters and encoders need the
    /// parameter shape (sample rate / pixel format / channel layout /
    /// frame rate / …) to configure themselves.
    fn params(&self) -> &CodecParameters;

    /// Produce the next frame. Returns [`Error::Eof`] at end of stream.
    fn next_frame(&mut self) -> Result<Frame>;

    /// Source-level metadata as ordered (key, value) pairs. Default is
    /// empty.
    fn metadata(&self) -> &[(String, String)] {
        &[]
    }

    /// Source-level duration in microseconds, if known. Default is
    /// `None`.
    fn duration_micros(&self) -> Option<i64> {
        None
    }
}

/// What a [`SourceRegistry::open`] call returns. The variant is decided
/// at driver-registration time, so callers can match on the shape and
/// branch the pipeline accordingly.
pub enum SourceOutput {
    Bytes(Box<dyn BytesSource>),
    Packets(Box<dyn PacketSource>),
    Frames(Box<dyn FrameSource>),
}

// ───────────────────────── opener function aliases ─────────────────────────

/// Opener for a [`BytesSource`] driver.
pub type OpenBytesFn = fn(uri: &str) -> Result<Box<dyn BytesSource>>;

/// Opener for a [`PacketSource`] driver.
pub type OpenPacketsFn = fn(uri: &str) -> Result<Box<dyn PacketSource>>;

/// Opener for a [`FrameSource`] driver.
pub type OpenFramesFn = fn(uri: &str) -> Result<Box<dyn FrameSource>>;

/// Internal per-scheme entry: which opener kind is registered for this
/// scheme. Stored in a single map so [`SourceRegistry::open`] can
/// dispatch with a single lookup, then match the variant to wrap in the
/// returned [`SourceOutput`].
enum OpenerEntry {
    Bytes(OpenBytesFn),
    Packets(OpenPacketsFn),
    Frames(OpenFramesFn),
}

// ───────────────────────── SourceRegistry ─────────────────────────

/// Registry mapping URI schemes to opener functions. Each scheme picks
/// one of three opener kinds (bytes / packets / frames) at registration
/// time; callers see the choice via the [`SourceOutput`] variant
/// returned from [`open`](Self::open).
#[derive(Default)]
pub struct SourceRegistry {
    schemes: HashMap<String, OpenerEntry>,
}

impl SourceRegistry {
    /// Empty registry. Callers must register at least one driver before
    /// calling [`open`](Self::open). The conventional minimum is the
    /// `file` driver (provided by the `oxideav-source` crate).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a [`BytesSource`] opener for a scheme. Schemes are
    /// normalised to ASCII lowercase. Replaces any prior registration
    /// (including registrations of other opener kinds).
    pub fn register_bytes(&mut self, scheme: &str, opener: OpenBytesFn) {
        self.schemes
            .insert(scheme.to_ascii_lowercase(), OpenerEntry::Bytes(opener));
    }

    /// Register a [`PacketSource`] opener for a scheme. Schemes are
    /// normalised to ASCII lowercase. Replaces any prior registration
    /// (including registrations of other opener kinds).
    pub fn register_packets(&mut self, scheme: &str, opener: OpenPacketsFn) {
        self.schemes
            .insert(scheme.to_ascii_lowercase(), OpenerEntry::Packets(opener));
    }

    /// Register a [`FrameSource`] opener for a scheme. Schemes are
    /// normalised to ASCII lowercase. Replaces any prior registration
    /// (including registrations of other opener kinds).
    pub fn register_frames(&mut self, scheme: &str, opener: OpenFramesFn) {
        self.schemes
            .insert(scheme.to_ascii_lowercase(), OpenerEntry::Frames(opener));
    }

    /// Open a URI. The URI's scheme determines which opener runs; bare
    /// paths (no scheme) and unrecognised schemes both fall back to the
    /// `file` driver if it is registered.
    ///
    /// Returns a [`SourceOutput`] whose variant matches the registered
    /// opener kind: bytes-shape drivers return `SourceOutput::Bytes`,
    /// packet-shape drivers return `SourceOutput::Packets`, and so on.
    pub fn open(&self, uri_str: &str) -> Result<SourceOutput> {
        let (scheme, _) = split_scheme(uri_str);
        let scheme = scheme.to_ascii_lowercase();
        if let Some(entry) = self.schemes.get(&scheme) {
            return dispatch(entry, uri_str);
        }
        // Fall back to file driver for unknown schemes.
        if let Some(entry) = self.schemes.get("file") {
            return dispatch(entry, uri_str);
        }
        Err(Error::Unsupported(format!(
            "no source driver for scheme '{scheme}' (URI: {uri_str})"
        )))
    }

    /// Iterate the registered schemes (for diagnostics).
    pub fn schemes(&self) -> impl Iterator<Item = &str> {
        self.schemes.keys().map(|s| s.as_str())
    }
}

fn dispatch(entry: &OpenerEntry, uri_str: &str) -> Result<SourceOutput> {
    match entry {
        OpenerEntry::Bytes(open) => open(uri_str).map(SourceOutput::Bytes),
        OpenerEntry::Packets(open) => open(uri_str).map(SourceOutput::Packets),
        OpenerEntry::Frames(open) => open(uri_str).map(SourceOutput::Frames),
    }
}

/// Split a URI into `(scheme, rest)`. Bare paths (no scheme) report scheme
/// `"file"` and `rest = uri`. Path-like inputs that happen to start with
/// `c:` on Windows are treated as bare paths.
pub(crate) fn split_scheme(uri: &str) -> (&str, &str) {
    if let Some(idx) = uri.find(':') {
        let (scheme, rest) = uri.split_at(idx);
        let rest = &rest[1..]; // skip ':'

        // Reject single-letter scheme that looks like a Windows drive letter.
        if scheme.len() == 1 && scheme.chars().next().unwrap().is_ascii_alphabetic() {
            return ("file", uri);
        }

        // Scheme must be ASCII alphanumeric / `+` / `-` / `.`, starting with a letter.
        let valid = !scheme.is_empty()
            && scheme.chars().next().unwrap().is_ascii_alphabetic()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));

        if !valid {
            return ("file", uri);
        }

        // Strip leading `//` from rest if present.
        let rest = rest.strip_prefix("//").unwrap_or(rest);
        return (scheme, rest);
    }
    ("file", uri)
}

// ───────────────────────── tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{AudioFrame, Frame};
    use crate::packet::Packet;
    use crate::stream::{CodecId, CodecParameters, StreamInfo};
    use crate::time::TimeBase;
    use std::io::{Cursor, Read};

    // ---- mock BytesSource ----
    fn open_bytes_mock(_uri: &str) -> Result<Box<dyn BytesSource>> {
        Ok(Box::new(Cursor::new(b"hello world".to_vec())))
    }

    #[test]
    fn register_bytes_and_open_returns_bytes_variant() {
        let mut reg = SourceRegistry::new();
        reg.register_bytes("mockb", open_bytes_mock);
        let out = reg.open("mockb://anything").expect("open");
        match out {
            SourceOutput::Bytes(mut r) => {
                let mut buf = String::new();
                r.read_to_string(&mut buf).unwrap();
                assert_eq!(buf, "hello world");
            }
            _ => panic!("expected SourceOutput::Bytes"),
        }
    }

    // ---- mock PacketSource ----
    struct MockPacketSource {
        streams: Vec<StreamInfo>,
        emitted: bool,
    }

    impl MockPacketSource {
        fn new() -> Self {
            let params = CodecParameters::audio(CodecId::new("pcm_s16le"));
            let s = StreamInfo {
                index: 0,
                time_base: TimeBase::new(1, 1000),
                duration: None,
                start_time: None,
                params,
            };
            Self {
                streams: vec![s],
                emitted: false,
            }
        }
    }

    impl PacketSource for MockPacketSource {
        fn streams(&self) -> &[StreamInfo] {
            &self.streams
        }
        fn next_packet(&mut self) -> Result<Packet> {
            if self.emitted {
                return Err(Error::Eof);
            }
            self.emitted = true;
            Ok(Packet::new(0, TimeBase::new(1, 1000), vec![1, 2, 3, 4]))
        }
    }

    fn open_packets_mock(_uri: &str) -> Result<Box<dyn PacketSource>> {
        Ok(Box::new(MockPacketSource::new()))
    }

    #[test]
    fn register_packets_and_open_returns_packets_variant() {
        let mut reg = SourceRegistry::new();
        reg.register_packets("mockp", open_packets_mock);
        let out = reg.open("mockp://anything").expect("open");
        match out {
            SourceOutput::Packets(mut p) => {
                assert_eq!(p.streams().len(), 1);
                let pkt = p.next_packet().expect("first packet");
                assert_eq!(pkt.data, vec![1, 2, 3, 4]);
                assert!(matches!(p.next_packet(), Err(Error::Eof)));
            }
            _ => panic!("expected SourceOutput::Packets"),
        }
    }

    // ---- mock FrameSource ----
    struct MockFrameSource {
        params: CodecParameters,
        emitted: bool,
    }

    impl MockFrameSource {
        fn new() -> Self {
            Self {
                params: CodecParameters::audio(CodecId::new("pcm_s16le")),
                emitted: false,
            }
        }
    }

    impl FrameSource for MockFrameSource {
        fn params(&self) -> &CodecParameters {
            &self.params
        }
        fn next_frame(&mut self) -> Result<Frame> {
            if self.emitted {
                return Err(Error::Eof);
            }
            self.emitted = true;
            Ok(Frame::Audio(AudioFrame {
                samples: 1,
                pts: Some(0),
                data: vec![vec![0u8, 0u8]],
            }))
        }
    }

    fn open_frames_mock(_uri: &str) -> Result<Box<dyn FrameSource>> {
        Ok(Box::new(MockFrameSource::new()))
    }

    #[test]
    fn register_frames_and_open_returns_frames_variant() {
        let mut reg = SourceRegistry::new();
        reg.register_frames("mockf", open_frames_mock);
        let out = reg.open("mockf://anything").expect("open");
        match out {
            SourceOutput::Frames(mut f) => {
                assert_eq!(f.params().codec_id.as_str(), "pcm_s16le");
                let frame = f.next_frame().expect("first frame");
                match frame {
                    Frame::Audio(a) => assert_eq!(a.samples, 1),
                    _ => panic!("expected audio frame"),
                }
                assert!(matches!(f.next_frame(), Err(Error::Eof)));
            }
            _ => panic!("expected SourceOutput::Frames"),
        }
    }

    #[test]
    fn unknown_scheme_falls_back_to_file_when_registered() {
        let mut reg = SourceRegistry::new();
        reg.register_bytes("file", open_bytes_mock);
        // No `foo` driver — falls through to the `file` driver.
        let out = reg.open("foo://x").expect("fallback open");
        assert!(matches!(out, SourceOutput::Bytes(_)));
    }

    #[test]
    fn unknown_scheme_with_no_file_driver_errors() {
        let reg = SourceRegistry::new();
        let r = reg.open("nope://x");
        assert!(matches!(r, Err(Error::Unsupported(_))));
    }

    #[test]
    fn register_overrides_prior_kind() {
        // Registering `mock` first as bytes then as frames should leave
        // only the frames opener active (last write wins).
        let mut reg = SourceRegistry::new();
        reg.register_bytes("mock", open_bytes_mock);
        reg.register_frames("mock", open_frames_mock);
        let out = reg.open("mock://x").expect("open");
        assert!(matches!(out, SourceOutput::Frames(_)));
    }

    #[test]
    fn schemes_iterator_lists_registered() {
        let mut reg = SourceRegistry::new();
        reg.register_bytes("mockb", open_bytes_mock);
        reg.register_packets("mockp", open_packets_mock);
        reg.register_frames("mockf", open_frames_mock);
        let mut names: Vec<&str> = reg.schemes().collect();
        names.sort();
        assert_eq!(names, vec!["mockb", "mockf", "mockp"]);
    }
}

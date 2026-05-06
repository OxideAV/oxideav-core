//! Stream metadata shared between containers and codecs.

use crate::format::{ChannelLayout, MediaType, PixelFormat, SampleFormat};
use crate::limits::DecoderLimits;
use crate::options::CodecOptions;
use crate::rational::Rational;
use crate::time::TimeBase;

/// A stable identifier for a codec. Codec crates register a `CodecId` so the
/// codec registry can look them up by name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodecId(pub String);

impl CodecId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CodecId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl std::fmt::Display for CodecId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A codec identifier scoped to a container format — the thing a
/// demuxer reads out of the file to name a codec. Resolved to a
/// [`CodecId`] by the codec registry.
///
/// Centralising these in the registry (instead of each container
/// hand-rolling its own FourCC → CodecId table) lets:
///
/// * a codec crate declare its own tag claims in `register()`, keeping
///   ownership co-located with the decoder;
/// * multiple codecs claim the same tag with priority ordering;
/// * optional per-claim probes disambiguate the tag-collision cases
///   that happen everywhere in the wild (DIV3 that's actually MPEG-4
///   Part 2, XVID that's actually MS-MPEG4v3, audio wFormatTag=0x0055
///   that could be MP3 or — very rarely — something else, etc.).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CodecTag {
    /// Four-character code used by AVI's `bmih.biCompression`, MP4 /
    /// QuickTime sample-entry type, Matroska V_/A_ tags built around
    /// FourCC, and many others. Always stored with alphabetic bytes
    /// upper-cased so lookups are case-insensitive; non-alphabetic
    /// bytes are preserved as-is.
    Fourcc([u8; 4]),

    /// AVI / WAV `WAVEFORMATEX::wFormatTag` (e.g. 0x0001 = PCM,
    /// 0x0055 = MP3, 0x00FF = "raw" AAC, 0x1610 = AAC ADTS).
    WaveFormat(u16),

    /// MP4 ObjectTypeIndication (ISO/IEC 14496-1 Table 5 / the values
    /// in an MP4 `esds` `DecoderConfigDescriptor`). e.g. 0x40 = MPEG-4
    /// AAC, 0x20 = MPEG-4 Visual, 0x69 = MP3.
    Mp4ObjectType(u8),

    /// Matroska `CodecID` element (full string, e.g.
    /// `"V_MPEG4/ISO/AVC"`, `"A_AAC"`, `"A_VORBIS"`).
    Matroska(String),
}

impl CodecTag {
    /// Build a FourCC tag, upper-casing alphabetic bytes.
    pub fn fourcc(raw: &[u8; 4]) -> Self {
        let mut out = [0u8; 4];
        for i in 0..4 {
            out[i] = raw[i].to_ascii_uppercase();
        }
        Self::Fourcc(out)
    }

    pub fn wave_format(tag: u16) -> Self {
        Self::WaveFormat(tag)
    }

    pub fn mp4_object_type(oti: u8) -> Self {
        Self::Mp4ObjectType(oti)
    }

    pub fn matroska(id: impl Into<String>) -> Self {
        Self::Matroska(id.into())
    }
}

impl std::fmt::Display for CodecTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fourcc(fcc) => {
                // Print as bytes when ASCII-printable, else as hex.
                if fcc.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
                    write!(f, "fourcc({})", std::str::from_utf8(fcc).unwrap_or("????"))
                } else {
                    write!(
                        f,
                        "fourcc(0x{:02X}{:02X}{:02X}{:02X})",
                        fcc[0], fcc[1], fcc[2], fcc[3]
                    )
                }
            }
            Self::WaveFormat(t) => write!(f, "wFormatTag(0x{t:04X})"),
            Self::Mp4ObjectType(o) => write!(f, "mp4_oti(0x{o:02X})"),
            Self::Matroska(s) => write!(f, "matroska({s})"),
        }
    }
}

/// Context passed to a codec's probe function during tag resolution.
///
/// Built by the demuxer from whatever it has already parsed (stream
/// format block, a peek at the first packet, numeric hints like
/// `bits_per_sample`). Probes read fields directly; the struct is
/// `#[non_exhaustive]` so additional hints can be added later without
/// breaking codec crates that match on it.
///
/// The canonical construction pattern, for a demuxer:
///
/// ```
/// # use oxideav_core::{CodecTag, ProbeContext};
/// let tag = CodecTag::wave_format(0x0001);
/// let ctx = ProbeContext::new(&tag)
///     .bits(24)
///     .channels(2)
///     .sample_rate(48_000);
/// # let _ = ctx;
/// ```
///
/// Codec authors read fields like `ctx.bits_per_sample` / `ctx.tag`
/// directly — `#[non_exhaustive]` forbids struct-literal construction
/// from outside this crate but does not restrict field access.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct ProbeContext<'a> {
    /// The tag being resolved — always set.
    pub tag: &'a CodecTag,
    /// Raw container-level stream-format blob if available
    /// (e.g. WAVEFORMATEX, BITMAPINFOHEADER, MP4 sample-entry bytes,
    /// Matroska `CodecPrivate`). Format is container-specific.
    pub header: Option<&'a [u8]>,
    /// First packet bytes if the demuxer has already read one.
    /// Most demuxers resolve tags at stream-discovery time before any
    /// packet exists; this is `None` in that case.
    pub packet: Option<&'a [u8]>,
    /// Audio: bits per sample (from WAVEFORMATEX, MP4 sample entry,
    /// Matroska `BitDepth`, etc.).
    pub bits_per_sample: Option<u16>,
    pub channels: Option<u16>,
    pub sample_rate: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl<'a> ProbeContext<'a> {
    /// Start building a context for `tag` with every hint field empty.
    pub fn new(tag: &'a CodecTag) -> Self {
        Self {
            tag,
            header: None,
            packet: None,
            bits_per_sample: None,
            channels: None,
            sample_rate: None,
            width: None,
            height: None,
        }
    }

    pub fn header(mut self, h: &'a [u8]) -> Self {
        self.header = Some(h);
        self
    }

    pub fn packet(mut self, p: &'a [u8]) -> Self {
        self.packet = Some(p);
        self
    }

    pub fn bits(mut self, n: u16) -> Self {
        self.bits_per_sample = Some(n);
        self
    }

    pub fn channels(mut self, n: u16) -> Self {
        self.channels = Some(n);
        self
    }

    pub fn sample_rate(mut self, n: u32) -> Self {
        self.sample_rate = Some(n);
        self
    }

    pub fn width(mut self, n: u32) -> Self {
        self.width = Some(n);
        self
    }

    pub fn height(mut self, n: u32) -> Self {
        self.height = Some(n);
        self
    }
}

/// Confidence value returned by a probe. `1.0` means "certainly me",
/// `0.0` means "not me", values in between mean "partial evidence — if
/// no higher-confidence claim exists, this should win". The registry
/// picks the claim with the highest returned confidence and skips any
/// that return `0.0`.
pub type Confidence = f32;

/// A probe function a codec attaches to its registration to
/// disambiguate tag collisions. Called once per candidate
/// registration during `resolve_tag`.
pub type ProbeFn = fn(&ProbeContext) -> Confidence;

/// Resolve a [`CodecTag`] (FourCC / WAVEFORMATEX / Matroska id / …) to a
/// [`CodecId`]. The [`oxideav-codec`](https://crates.io/crates/oxideav-codec)
/// registry implements this, but defining the trait here lets
/// containers consume tag resolution via `&dyn CodecResolver` without
/// pulling in the codec crate as a direct dependency.
pub trait CodecResolver: Sync {
    /// Resolve the tag in `ctx.tag` to a codec id. Implementations walk
    /// every registration whose tag set contains the tag, call each
    /// probe (treating `None` as "always 1.0"), and return the id with
    /// the highest resulting confidence. Ties are broken by
    /// registration order.
    fn resolve_tag(&self, ctx: &ProbeContext) -> Option<CodecId>;
}

/// Null resolver that resolves nothing — useful as a default when a
/// caller doesn't have a real registry handy (e.g. unit tests, or
/// legacy callers of the tag-free `open()` APIs).
#[derive(Default, Clone, Copy)]
pub struct NullCodecResolver;

impl CodecResolver for NullCodecResolver {
    fn resolve_tag(&self, _ctx: &ProbeContext) -> Option<CodecId> {
        None
    }
}

/// Codec-level parameters shared between demuxer/muxer and en/decoder.
///
/// **Marked `#[non_exhaustive]`** — construction via struct-literal
/// syntax is not supported. Use the [`audio`](Self::audio) /
/// [`video`](Self::video) constructors (or functional-update
/// `CodecParameters { ..base }` syntax) so new fields can be added
/// without another semver break.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CodecParameters {
    pub codec_id: CodecId,
    pub media_type: MediaType,

    // Audio-specific
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub sample_format: Option<SampleFormat>,
    /// Speaker layout for the audio stream. **This is the canonical
    /// answer to "what layout does this stream have?"** — layout is a
    /// stream-level property and is intentionally *not* duplicated on
    /// individual [`AudioFrame`](crate::AudioFrame)s.
    ///
    /// Optional and additive alongside [`channels`](Self::channels): a
    /// codec/container that only knows the count can leave this `None`
    /// and consumers will fall back to [`ChannelLayout::from_count`]
    /// via [`Self::resolved_layout`]. When both are set, they must
    /// agree on channel count.
    pub channel_layout: Option<ChannelLayout>,

    // Video-specific
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub pixel_format: Option<PixelFormat>,
    pub frame_rate: Option<Rational>,

    /// Per-codec setup bytes (e.g., SPS/PPS, OpusHead). Format defined by codec.
    pub extradata: Vec<u8>,

    pub bit_rate: Option<u64>,

    /// Codec-specific tuning knobs (e.g. `{"interlace": "true"}` for PNG's
    /// Adam7 encode, `{"crf": "23"}` for h264). Empty by default. The shape
    /// is declared by each codec's options struct — see
    /// [`crate::options`]. Parsed once at encoder/decoder construction;
    /// the hot path never touches this.
    pub options: CodecOptions,

    /// DoS-protection caps threaded into every decoder constructed from
    /// these parameters. See [`DecoderLimits`] for the semantics of each
    /// field. Defaults are conservative-but-finite (32 k × 32 k pixels,
    /// 1 GiB per arena, etc.) — every existing real-world stream
    /// decodes unchanged. Tighten via [`Self::with_limits`] when the
    /// caller wants to harden the pipeline against untrusted input.
    pub limits: DecoderLimits,

    /// Optional 0-based device selector for hardware-accelerated codecs.
    /// `None` (the default) means "use the backend's default device";
    /// `Some(n)` requests device `n` from the backend's
    /// [`crate::engine::HwDeviceInfo`] enumeration order.
    ///
    /// Software codecs ignore this field. Hardware codecs read it as
    /// `params.device_index.unwrap_or(0)` to pick which physical engine
    /// to bind to. Indexing matches the order of devices reported by the
    /// codec entry's `engine_probe` function.
    pub device_index: Option<u32>,
}

impl CodecParameters {
    pub fn audio(codec_id: CodecId) -> Self {
        Self {
            codec_id,
            media_type: MediaType::Audio,
            sample_rate: None,
            channels: None,
            sample_format: None,
            channel_layout: None,
            width: None,
            height: None,
            pixel_format: None,
            frame_rate: None,
            extradata: Vec::new(),
            bit_rate: None,
            options: CodecOptions::default(),
            limits: DecoderLimits::default(),
            device_index: None,
        }
    }

    /// True when `self` and `other` have the same codec_id and core
    /// format parameters (sample_rate/channels/sample_format for audio,
    /// width/height/pixel_format for video). Extradata and bitrate
    /// differences are tolerated — many containers rewrite extradata
    /// losslessly during a copy operation. `channel_layout` is compared
    /// only via the channel count (through [`Self::resolved_layout`]) so
    /// a stream that surfaces an explicit layout still matches a
    /// count-only stream of the same width.
    pub fn matches_core(&self, other: &CodecParameters) -> bool {
        self.codec_id == other.codec_id
            && self.sample_rate == other.sample_rate
            && self.channels == other.channels
            && self.sample_format == other.sample_format
            && self.width == other.width
            && self.height == other.height
            && self.pixel_format == other.pixel_format
    }

    pub fn video(codec_id: CodecId) -> Self {
        Self {
            codec_id,
            media_type: MediaType::Video,
            sample_rate: None,
            channels: None,
            sample_format: None,
            channel_layout: None,
            width: None,
            height: None,
            pixel_format: None,
            frame_rate: None,
            extradata: Vec::new(),
            bit_rate: None,
            options: CodecOptions::default(),
            limits: DecoderLimits::default(),
            device_index: None,
        }
    }

    /// Construct subtitle codec parameters. No format-specific fields
    /// are populated — subtitle codecs typically only carry an opaque
    /// `extradata` blob (the format's header / style block) and the
    /// codec id.
    pub fn subtitle(codec_id: CodecId) -> Self {
        Self {
            codec_id,
            media_type: MediaType::Subtitle,
            sample_rate: None,
            channels: None,
            sample_format: None,
            channel_layout: None,
            width: None,
            height: None,
            pixel_format: None,
            frame_rate: None,
            extradata: Vec::new(),
            bit_rate: None,
            options: CodecOptions::default(),
            limits: DecoderLimits::default(),
            device_index: None,
        }
    }

    /// Construct generic data-stream codec parameters (timed metadata,
    /// chapters, etc.). Like [`Self::subtitle`], no format-specific
    /// fields are populated.
    pub fn data(codec_id: CodecId) -> Self {
        Self {
            codec_id,
            media_type: MediaType::Data,
            sample_rate: None,
            channels: None,
            sample_format: None,
            channel_layout: None,
            width: None,
            height: None,
            pixel_format: None,
            frame_rate: None,
            extradata: Vec::new(),
            bit_rate: None,
            options: CodecOptions::default(),
            limits: DecoderLimits::default(),
            device_index: None,
        }
    }

    /// Builder method: set the channel count.
    ///
    /// Pairs with [`Self::channel_layout`] for the layout. The two are
    /// kept as independent fields so a codec that only knows one or the
    /// other can populate just the field it has; [`Self::resolved_layout`]
    /// derives a layout from whatever is set.
    pub fn channels(mut self, n: u16) -> Self {
        self.channels = Some(n);
        self
    }

    /// Builder method: set the channel layout. Mirrors
    /// [`Self::channels`]; setting one does not auto-fill the other —
    /// use [`Self::resolved_layout`] / [`Self::resolved_channels`] at
    /// read time to bridge the two.
    pub fn channel_layout(mut self, layout: ChannelLayout) -> Self {
        self.channel_layout = Some(layout);
        self
    }

    /// Best-effort layout: prefers an explicit [`Self::channel_layout`]
    /// when set, otherwise infers one from [`Self::channels`] via
    /// [`ChannelLayout::from_count`]. Returns `None` only when neither
    /// field is populated (e.g. video / data streams, or audio params
    /// surfaced before the codec has been opened).
    ///
    /// This is the canonical call-site for resolving a stream's
    /// channel layout — frames do *not* carry layout, so audio
    /// consumers (downmix, device routing, channel-aware filters)
    /// should read it from the stream's `CodecParameters` once and
    /// pass it down with the frame.
    pub fn resolved_layout(&self) -> Option<ChannelLayout> {
        self.channel_layout
            .or_else(|| self.channels.map(ChannelLayout::from_count))
    }

    /// Best-effort channel count: prefers an explicit
    /// [`Self::channels`] when set, otherwise reads the count off
    /// [`Self::channel_layout`]. Returns `None` only when neither
    /// field is populated.
    pub fn resolved_channels(&self) -> Option<u16> {
        self.channels
            .or_else(|| self.channel_layout.map(|l| l.channel_count()))
    }

    /// Read-only access to the DoS-protection caps for any decoder
    /// constructed from these parameters. See [`DecoderLimits`].
    pub fn limits(&self) -> &DecoderLimits {
        &self.limits
    }

    /// Builder method: replace the [`DecoderLimits`] for these
    /// parameters. Use to tighten caps before passing parameters into
    /// `make_decoder` (e.g. when processing untrusted uploads on a
    /// shared server).
    ///
    /// ```
    /// # use oxideav_core::{CodecId, CodecParameters, DecoderLimits};
    /// let limits = DecoderLimits::default()
    ///     .with_max_pixels_per_frame(4096 * 4096)
    ///     .with_max_arenas_in_flight(2);
    /// let p = CodecParameters::video(CodecId::new("h263")).with_limits(limits);
    /// assert_eq!(p.limits().max_pixels_per_frame, 4096 * 4096);
    /// ```
    pub fn with_limits(mut self, limits: DecoderLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Bind subsequent decoder/encoder construction to a specific device.
    /// `index` matches the position in the `engine_probe` device list.
    ///
    /// Software codecs ignore this field. Hardware codecs read it as
    /// `params.device_index.unwrap_or(0)` to pick which physical engine
    /// to bind to.
    pub fn with_device_index(mut self, index: u32) -> Self {
        self.device_index = Some(index);
        self
    }
}

/// Description of a single stream inside a container.
#[derive(Clone, Debug)]
pub struct StreamInfo {
    pub index: u32,
    pub time_base: TimeBase,
    pub duration: Option<i64>,
    pub start_time: Option<i64>,
    pub params: CodecParameters,
}

#[cfg(test)]
mod codec_tag_tests {
    use super::*;

    #[test]
    fn fourcc_uppercases_on_construction() {
        let t = CodecTag::fourcc(b"div3");
        assert_eq!(t, CodecTag::Fourcc(*b"DIV3"));
        // Non-alphabetic bytes preserved unchanged.
        let t2 = CodecTag::fourcc(b"MP42");
        assert_eq!(t2, CodecTag::Fourcc(*b"MP42"));
        let t3 = CodecTag::fourcc(&[0xFF, b'a', 0x00, b'1']);
        assert_eq!(t3, CodecTag::Fourcc([0xFF, b'A', 0x00, b'1']));
    }

    #[test]
    fn fourcc_equality_case_insensitive_via_ctor() {
        assert_eq!(CodecTag::fourcc(b"xvid"), CodecTag::fourcc(b"XVID"));
        assert_eq!(CodecTag::fourcc(b"DiV3"), CodecTag::fourcc(b"div3"));
    }

    #[test]
    fn display_printable_fourcc() {
        assert_eq!(CodecTag::fourcc(b"XVID").to_string(), "fourcc(XVID)");
    }

    #[test]
    fn display_non_printable_fourcc_as_hex() {
        let t = CodecTag::Fourcc([0x00, 0x00, 0x00, 0x01]);
        assert_eq!(t.to_string(), "fourcc(0x00000001)");
    }

    #[test]
    fn display_wave_format() {
        assert_eq!(
            CodecTag::wave_format(0x0055).to_string(),
            "wFormatTag(0x0055)"
        );
    }

    #[test]
    fn display_mp4_oti() {
        assert_eq!(CodecTag::mp4_object_type(0x40).to_string(), "mp4_oti(0x40)");
    }

    #[test]
    fn display_matroska() {
        assert_eq!(
            CodecTag::matroska("V_MPEG4/ISO/AVC").to_string(),
            "matroska(V_MPEG4/ISO/AVC)",
        );
    }

    #[test]
    fn null_resolver_resolves_nothing() {
        let r = NullCodecResolver;
        let xvid = CodecTag::fourcc(b"XVID");
        assert!(r.resolve_tag(&ProbeContext::new(&xvid)).is_none());
        let wf = CodecTag::wave_format(0x0055);
        assert!(r.resolve_tag(&ProbeContext::new(&wf)).is_none());
    }

    #[test]
    fn probe_context_builder_fills_hints() {
        let tag = CodecTag::wave_format(0x0001);
        let ctx = ProbeContext::new(&tag)
            .bits(24)
            .channels(2)
            .sample_rate(48_000)
            .header(&[1, 2, 3])
            .packet(&[4, 5]);
        assert_eq!(ctx.bits_per_sample, Some(24));
        assert_eq!(ctx.channels, Some(2));
        assert_eq!(ctx.sample_rate, Some(48_000));
        assert_eq!(ctx.header.unwrap(), &[1, 2, 3]);
        assert_eq!(ctx.packet.unwrap(), &[4, 5]);
    }
}

#[cfg(test)]
mod channel_layout_plumbing_tests {
    use super::*;

    #[test]
    fn audio_params_default_to_no_layout() {
        let p = CodecParameters::audio(CodecId::new("pcm_s16le"));
        assert!(p.channel_layout.is_none());
        assert!(p.channels.is_none());
        assert!(p.resolved_layout().is_none());
        assert!(p.resolved_channels().is_none());
    }

    #[test]
    fn channels_only_infers_layout_via_from_count() {
        let p = CodecParameters::audio(CodecId::new("pcm_s16le")).channels(6);
        assert_eq!(p.channels, Some(6));
        assert!(p.channel_layout.is_none());
        assert_eq!(p.resolved_layout(), Some(ChannelLayout::Surround51));
        assert_eq!(p.resolved_channels(), Some(6));
    }

    #[test]
    fn explicit_layout_wins_over_count() {
        let p = CodecParameters::audio(CodecId::new("ac3"))
            .channels(6)
            .channel_layout(ChannelLayout::Surround60);
        // 6ch by-count would default to Surround51, but the explicit
        // layout overrides.
        assert_eq!(p.resolved_layout(), Some(ChannelLayout::Surround60));
        assert_eq!(p.resolved_channels(), Some(6));
    }

    #[test]
    fn layout_only_yields_count_via_resolved_channels() {
        let p =
            CodecParameters::audio(CodecId::new("ac3")).channel_layout(ChannelLayout::Surround71);
        assert!(p.channels.is_none());
        assert_eq!(p.resolved_channels(), Some(8));
        assert_eq!(p.resolved_layout(), Some(ChannelLayout::Surround71));
    }
}

#[cfg(test)]
mod codec_parameters_device_index_tests {
    use super::*;

    #[test]
    fn codec_parameters_device_index_defaults_to_none() {
        assert!(CodecParameters::audio(CodecId::new("pcm_s16le"))
            .device_index
            .is_none());
        assert!(CodecParameters::video(CodecId::new("h264"))
            .device_index
            .is_none());
        assert!(CodecParameters::subtitle(CodecId::new("srt"))
            .device_index
            .is_none());
        assert!(CodecParameters::data(CodecId::new("bin"))
            .device_index
            .is_none());
    }

    #[test]
    fn codec_parameters_with_device_index_sets_field() {
        let p = CodecParameters::video(CodecId::new("h264")).with_device_index(2);
        assert_eq!(p.device_index, Some(2));
    }
}

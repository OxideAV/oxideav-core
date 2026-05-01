//! In-process codec registry.
//!
//! Every codec crate declares itself with one [`CodecInfo`] value —
//! capabilities, factory functions, the container tags it claims, and
//! (optionally) a probe function used to disambiguate genuine tag
//! collisions. The registry stores those registrations and exposes
//! three orthogonal lookups:
//!
//! - **id-keyed** — `make_decoder(params)` / `make_encoder(params)` walk
//!   the implementations registered under `params.codec_id`, filter by
//!   capability restrictions, and try them in priority order with init-
//!   time fallback.
//! - **tag-keyed** — `resolve_tag(&ProbeContext)` walks every
//!   registration whose `tags` contains `ctx.tag`, calls each probe
//!   (treating `None` as "returns 1.0"), and returns the id with the
//!   highest resulting confidence. First-registered wins on ties.
//! - **diagnostic** — `all_implementations`, `all_tag_registrations`.
//!
//! The tag path explicitly DOES NOT short-circuit on "first claim with
//! no probe" — every claimant is asked, so a lower-priority probed
//! claim can out-rank a higher-priority unprobed one when the content
//! is actually ambiguous (DIV3 XVID-with-real-MSMPEG4 payload etc.).

use std::collections::HashMap;

use crate::arena;
use crate::{
    CodecCapabilities, CodecId, CodecOptionsStruct, CodecParameters, CodecPreferences,
    CodecResolver, CodecTag, Error, ExecutionContext, Frame, OptionField, Packet, PixelFormat,
    ProbeContext, ProbeFn, Result,
};

// ───────────────────────── codec traits ─────────────────────────

/// A packet-to-frame decoder.
pub trait Decoder: Send {
    fn codec_id(&self) -> &CodecId;

    /// Feed one compressed packet. May or may not produce a frame immediately —
    /// call `receive_frame` in a loop afterwards.
    fn send_packet(&mut self, packet: &Packet) -> Result<()>;

    /// Pull the next decoded frame, if any. Returns `Error::NeedMore` when the
    /// decoder needs another packet.
    fn receive_frame(&mut self) -> Result<Frame>;

    /// Pull the next decoded frame as an arena-backed [`arena::sync::Frame`].
    ///
    /// Decoders that build their output through an
    /// [`arena::sync::ArenaPool`] override this to return the pooled
    /// [`arena::sync::Frame`] **directly**, with no per-plane memcpy
    /// out — the caller gets true zero-copy plane access via
    /// [`arena::sync::FrameInner::plane`].
    ///
    /// The default implementation delegates to [`Self::receive_frame`]
    /// and copies the video planes into a freshly-leased one-shot
    /// `arena::sync::ArenaPool`. This makes the method an additive
    /// change for every existing [`Decoder`] impl: callers using the
    /// new API still work, but pay one memcpy per plane.
    ///
    /// **Audio / subtitle frames:** the [`arena::sync::Frame`] body is
    /// video-only (planes + [`arena::sync::FrameHeader`] with
    /// width/height/pixel format). The default implementation returns
    /// [`Error::Unsupported`] for non-video frames; an audio decoder
    /// that wants to expose `receive_arena_frame()` must override it
    /// with its own arena-backed audio-frame type once the framework
    /// gains one. Until then, audio decoders should keep using
    /// [`Self::receive_frame`].
    fn receive_arena_frame(&mut self) -> Result<arena::sync::Frame> {
        let frame = self.receive_frame()?;
        match frame {
            Frame::Video(v) => video_frame_to_arena_sync_frame(&v),
            Frame::Audio(_) => Err(Error::unsupported(
                "receive_arena_frame: audio frames not yet supported by default impl",
            )),
            Frame::Subtitle(_) => Err(Error::unsupported(
                "receive_arena_frame: subtitle frames have no arena-backed representation",
            )),
        }
    }

    /// Signal end-of-stream. After this, `receive_frame` will drain buffered
    /// frames and eventually return `Error::Eof`.
    fn flush(&mut self) -> Result<()>;

    /// Discard all carry-over state so the decoder can resume from a new
    /// bitstream position without producing stale output. Called by the
    /// player after a container seek.
    ///
    /// Unlike [`flush`](Self::flush) (which signals end-of-stream and
    /// drains buffered frames), `reset` is expected to:
    /// * drop every buffered input packet and pending output frame;
    /// * zero any per-stream filter / predictor / overlap memory so the
    ///   next `send_packet` decodes as if it were the first;
    /// * leave the codec id and stream parameters untouched.
    ///
    /// The default is a conservative "drain-then-forget": call
    /// [`flush`](Self::flush) and ignore any remaining frames. Stateful
    /// codecs (LPC predictors, backward-adaptive gain, IMDCT overlap,
    /// reference pictures, …) should override this to wipe their
    /// internal state explicitly — otherwise the first ~N output
    /// samples after a seek will be glitchy until the state re-adapts.
    fn reset(&mut self) -> Result<()> {
        self.flush()?;
        // Drain any remaining output frames so the next send_packet
        // starts clean. NeedMore / Eof both mean "no more frames"; any
        // other error is surfaced so the caller can see why.
        loop {
            match self.receive_frame() {
                Ok(_) => {}
                Err(Error::NeedMore) | Err(Error::Eof) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    /// Advisory: announce the runtime environment (today: a thread budget
    /// for codec-internal parallelism). Called at most once, before the
    /// first `send_packet`. Default no-op; codecs that want to run
    /// slice-/GOP-/tile-parallel override this to capture the budget.
    /// Ignoring the hint is always safe — callers must still work with
    /// a decoder that runs serial.
    fn set_execution_context(&mut self, _ctx: &ExecutionContext) {}
}

/// A frame-to-packet encoder.
pub trait Encoder: Send {
    fn codec_id(&self) -> &CodecId;

    /// Parameters describing this encoder's output stream (to feed into a muxer).
    fn output_params(&self) -> &CodecParameters;

    fn send_frame(&mut self, frame: &Frame) -> Result<()>;

    fn receive_packet(&mut self) -> Result<Packet>;

    fn flush(&mut self) -> Result<()>;

    /// Advisory: announce the runtime environment. Same semantics as
    /// [`Decoder::set_execution_context`].
    fn set_execution_context(&mut self, _ctx: &ExecutionContext) {}
}

/// Default-impl helper for [`Decoder::receive_arena_frame`]: copy a
/// heap-backed [`crate::VideoFrame`] into a freshly-leased
/// [`arena::sync::Frame`].
///
/// Allocates a single-slot, single-arena `arena::sync::ArenaPool`
/// sized to fit the planes verbatim. The pool is dropped at the end of
/// this call; the returned `Frame` keeps its leased buffer alive via
/// `Arc<FrameInner>` (the `Arena`'s `Weak` handle to the dropped pool
/// just stops upgrading — the buffer drops normally when the last
/// `Frame` clone goes away).
///
/// Width / height / pixel-format on the returned `FrameHeader` are
/// derived from the plane shape: `width = plane[0].stride`,
/// `height = plane[0].data.len() / stride`. Pixel format is left as
/// [`PixelFormat::Yuv420P`] when there are 3 planes, else the first
/// per-plane sensible default — this is a best-effort label for the
/// generic conversion path; decoders that override
/// `receive_arena_frame` themselves should set the correct pixel
/// format.
fn video_frame_to_arena_sync_frame(v: &crate::VideoFrame) -> Result<arena::sync::Frame> {
    if v.planes.is_empty() {
        return Err(Error::invalid(
            "receive_arena_frame: video frame has no planes",
        ));
    }
    let total_bytes: usize = v.planes.iter().map(|p| p.data.len()).sum();
    if total_bytes == 0 {
        return Err(Error::invalid(
            "receive_arena_frame: video frame planes are empty",
        ));
    }
    // One-shot pool sized exactly to the frame. The pool drops at end
    // of scope; the leased Arena lives on inside the returned Frame
    // (its Weak<ArenaPool> handle just won't upgrade in Drop, so the
    // Box<[u8]> falls through to a normal heap free).
    let pool = arena::sync::ArenaPool::with_alloc_count_cap(
        1,
        total_bytes,
        // One alloc per plane, plus a generous safety margin.
        (v.planes.len() as u32).saturating_add(4),
    );
    let arena = pool.lease()?;
    let mut plane_offsets: Vec<(usize, usize)> = Vec::with_capacity(v.planes.len());
    let mut cursor = 0usize;
    for plane in &v.planes {
        let dst = arena.alloc::<u8>(plane.data.len())?;
        dst.copy_from_slice(&plane.data);
        plane_offsets.push((cursor, plane.data.len()));
        cursor += plane.data.len();
    }
    // Best-effort header: width = stride of plane 0, height inferred
    // from plane 0's data length. Pixel format defaults to Yuv420P for
    // the common 3-plane case, Gray8 for single-plane, otherwise
    // Yuv444P. Decoders that care about exact pixel-format / width /
    // height should override `receive_arena_frame` themselves so they
    // can emit a correct `FrameHeader` straight from their arena
    // build path.
    let stride0 = v.planes[0].stride.max(1);
    let width = stride0 as u32;
    let height = (v.planes[0].data.len() / stride0) as u32;
    let pixel_format = match v.planes.len() {
        1 => PixelFormat::Gray8,
        3 => PixelFormat::Yuv420P,
        _ => PixelFormat::Yuv444P,
    };
    let header = arena::sync::FrameHeader::new(width, height, pixel_format, v.pts);
    arena::sync::FrameInner::new(arena, &plane_offsets, header)
}

/// Factory that builds a decoder for a given codec parameter set.
pub type DecoderFactory = fn(params: &CodecParameters) -> Result<Box<dyn Decoder>>;

/// Factory that builds an encoder for a given codec parameter set.
pub type EncoderFactory = fn(params: &CodecParameters) -> Result<Box<dyn Encoder>>;

// ───────────────────────── CodecInfo ─────────────────────────

/// A single registration: capabilities, decoder/encoder factories,
/// optional probe, and the container tags this codec claims.
///
/// Codec crates build one of these per codec id inside their
/// `register(reg)` function and hand it to
/// [`CodecRegistry::register`]. The struct is `#[non_exhaustive]` so
/// additional fields can be added without breaking existing codec
/// crates — construction is only possible through
/// [`CodecInfo::new`] plus the builder methods below.
#[non_exhaustive]
pub struct CodecInfo {
    pub id: CodecId,
    pub capabilities: CodecCapabilities,
    pub decoder_factory: Option<DecoderFactory>,
    pub encoder_factory: Option<EncoderFactory>,
    /// Probe function that returns a confidence in `0.0..=1.0` for a
    /// given [`ProbeContext`]. `None` means "confidence 1.0 for every
    /// claimed tag" — the correct default for codecs whose tag claims
    /// are unambiguous.
    pub probe: Option<ProbeFn>,
    /// Tags this codec is willing to be looked up under. One codec may
    /// claim many tags (an AAC decoder covers several WaveFormat ids,
    /// a FourCC, an MP4 OTI, and a Matroska CodecID string at once).
    pub tags: Vec<CodecTag>,
    /// Schema of the encoder's recognised option keys
    /// (`CodecParameters::options`). Attached with
    /// [`Self::encoder_options`]. Used for validation / `oxideav list`
    /// / pipeline JSON checks.
    pub encoder_options_schema: Option<&'static [OptionField]>,
    /// Schema of the decoder's recognised option keys.
    pub decoder_options_schema: Option<&'static [OptionField]>,
}

impl CodecInfo {
    /// Start a new registration for `id` with empty capabilities, no
    /// factories, no probe, and no tags. Chain the builder methods
    /// below to fill it in, then hand the result to
    /// [`CodecRegistry::register`].
    pub fn new(id: CodecId) -> Self {
        Self {
            capabilities: CodecCapabilities::audio(id.as_str()),
            id,
            decoder_factory: None,
            encoder_factory: None,
            probe: None,
            tags: Vec::new(),
            encoder_options_schema: None,
            decoder_options_schema: None,
        }
    }

    /// Replace the capability description. The default built by
    /// [`Self::new`] is a placeholder (audio-flavoured, no flags); every
    /// real registration should call this.
    pub fn capabilities(mut self, caps: CodecCapabilities) -> Self {
        self.capabilities = caps;
        self
    }

    pub fn decoder(mut self, factory: DecoderFactory) -> Self {
        self.decoder_factory = Some(factory);
        self
    }

    pub fn encoder(mut self, factory: EncoderFactory) -> Self {
        self.encoder_factory = Some(factory);
        self
    }

    pub fn probe(mut self, probe: ProbeFn) -> Self {
        self.probe = Some(probe);
        self
    }

    /// Claim a single container tag for this codec. Equivalent to
    /// `.tags([tag])` but avoids the array ceremony for single-tag
    /// claims.
    pub fn tag(mut self, tag: CodecTag) -> Self {
        self.tags.push(tag);
        self
    }

    /// Claim a set of container tags for this codec. Takes any
    /// iterable (arrays, `Vec`, `Option`, …) so the common case of a
    /// codec with 3-6 tags reads as one clean block.
    pub fn tags(mut self, tags: impl IntoIterator<Item = CodecTag>) -> Self {
        self.tags.extend(tags);
        self
    }

    /// Declare the options struct this codec's encoder factory expects.
    /// Attaches `T::SCHEMA` so the registry can enumerate recognised
    /// option keys (for `oxideav list`, pipeline JSON validation, etc.).
    /// The factory itself still has to call
    /// [`crate::parse_options::<T>()`] against
    /// `CodecParameters::options` at init time.
    pub fn encoder_options<T: CodecOptionsStruct>(mut self) -> Self {
        self.encoder_options_schema = Some(T::SCHEMA);
        self
    }

    /// Declare the options struct this codec's decoder factory expects.
    /// See [`Self::encoder_options`] for the encoder counterpart.
    pub fn decoder_options<T: CodecOptionsStruct>(mut self) -> Self {
        self.decoder_options_schema = Some(T::SCHEMA);
        self
    }
}

/// Internal per-impl record held inside the registry's id map. Kept
/// distinct from [`CodecInfo`] so the id map stays cheap to walk
/// during `make_decoder` / `make_encoder` lookups.
#[derive(Clone)]
pub struct CodecImplementation {
    pub caps: CodecCapabilities,
    pub make_decoder: Option<DecoderFactory>,
    pub make_encoder: Option<EncoderFactory>,
    /// Encoder options schema declared via
    /// [`CodecInfo::encoder_options`]. `None` means the encoder accepts
    /// no tuning knobs (any non-empty `CodecParameters::options` will
    /// still be rejected by the factory if the encoder calls
    /// `parse_options` — this is purely informational for discovery).
    pub encoder_options_schema: Option<&'static [OptionField]>,
    pub decoder_options_schema: Option<&'static [OptionField]>,
}

#[derive(Default)]
pub struct CodecRegistry {
    /// id → list of implementations. Each registered codec appends one
    /// entry here. `make_decoder` / `make_encoder` walk this list in
    /// preference order.
    impls: HashMap<CodecId, Vec<CodecImplementation>>,
    /// Append-only list of every registration — the `tag_index` stores
    /// offsets into this vector.
    registrations: Vec<RegistrationRecord>,
    /// Tag → indices into `registrations`. Indices are stored in
    /// registration order so tie-breaking in `resolve_tag` is
    /// deterministic (first-registered wins).
    tag_index: HashMap<CodecTag, Vec<usize>>,
}

/// Internal registry record. Mirrors the subset of [`CodecInfo`]
/// needed at resolve time.
struct RegistrationRecord {
    id: CodecId,
    probe: Option<ProbeFn>,
}

impl CodecRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one codec. Expands into:
    ///   * an entry in the id → implementations map (for
    ///     `make_decoder` / `make_encoder`);
    ///   * an entry in the tag index for every claimed tag (for
    ///     `resolve_tag`).
    ///
    /// Calling `register` multiple times with the same id is allowed
    /// and how multi-implementation codecs (software-plus-hardware
    /// FLAC, for example) are expressed.
    pub fn register(&mut self, info: CodecInfo) {
        let CodecInfo {
            id,
            capabilities,
            decoder_factory,
            encoder_factory,
            probe,
            tags,
            encoder_options_schema,
            decoder_options_schema,
        } = info;

        let caps = {
            let mut c = capabilities;
            if decoder_factory.is_some() {
                c = c.with_decode();
            }
            if encoder_factory.is_some() {
                c = c.with_encode();
            }
            c
        };

        // Only record an implementation entry when at least one factory
        // is present. A "tag-only" CodecInfo — used to attach extra tag
        // claims to a codec that was already registered with factories —
        // shouldn't pollute the impl list.
        if decoder_factory.is_some() || encoder_factory.is_some() {
            self.impls
                .entry(id.clone())
                .or_default()
                .push(CodecImplementation {
                    caps,
                    make_decoder: decoder_factory,
                    make_encoder: encoder_factory,
                    encoder_options_schema,
                    decoder_options_schema,
                });
        }

        let record_idx = self.registrations.len();
        self.registrations.push(RegistrationRecord {
            id: id.clone(),
            probe,
        });
        for tag in tags {
            self.tag_index.entry(tag).or_default().push(record_idx);
        }
    }

    pub fn has_decoder(&self, id: &CodecId) -> bool {
        self.impls
            .get(id)
            .map(|v| v.iter().any(|i| i.make_decoder.is_some()))
            .unwrap_or(false)
    }

    pub fn has_encoder(&self, id: &CodecId) -> bool {
        self.impls
            .get(id)
            .map(|v| v.iter().any(|i| i.make_encoder.is_some()))
            .unwrap_or(false)
    }

    /// Build a decoder for `params`. Walks all implementations matching the
    /// codec id in increasing priority order, skipping any excluded by the
    /// caller's preferences. Init-time fallback: if a higher-priority impl's
    /// constructor returns an error, the next candidate is tried.
    pub fn make_decoder_with(
        &self,
        params: &CodecParameters,
        prefs: &CodecPreferences,
    ) -> Result<Box<dyn Decoder>> {
        let candidates = self
            .impls
            .get(&params.codec_id)
            .ok_or_else(|| Error::CodecNotFound(params.codec_id.to_string()))?;
        let mut ranked: Vec<&CodecImplementation> = candidates
            .iter()
            .filter(|i| i.make_decoder.is_some() && !prefs.excludes(&i.caps))
            .filter(|i| caps_fit_params(&i.caps, params, false))
            .collect();
        ranked.sort_by_key(|i| prefs.effective_priority(&i.caps));
        let mut last_err: Option<Error> = None;
        for imp in ranked {
            match (imp.make_decoder.unwrap())(params) {
                Ok(d) => return Ok(d),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            Error::CodecNotFound(format!(
                "no decoder for {} accepts the requested parameters",
                params.codec_id
            ))
        }))
    }

    /// Build an encoder, with the same priority + fallback semantics.
    pub fn make_encoder_with(
        &self,
        params: &CodecParameters,
        prefs: &CodecPreferences,
    ) -> Result<Box<dyn Encoder>> {
        let candidates = self
            .impls
            .get(&params.codec_id)
            .ok_or_else(|| Error::CodecNotFound(params.codec_id.to_string()))?;
        let mut ranked: Vec<&CodecImplementation> = candidates
            .iter()
            .filter(|i| i.make_encoder.is_some() && !prefs.excludes(&i.caps))
            .filter(|i| caps_fit_params(&i.caps, params, true))
            .collect();
        ranked.sort_by_key(|i| prefs.effective_priority(&i.caps));
        let mut last_err: Option<Error> = None;
        for imp in ranked {
            match (imp.make_encoder.unwrap())(params) {
                Ok(e) => return Ok(e),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            Error::CodecNotFound(format!(
                "no encoder for {} accepts the requested parameters",
                params.codec_id
            ))
        }))
    }

    /// Default-preference shorthand for `make_decoder_with`.
    pub fn make_decoder(&self, params: &CodecParameters) -> Result<Box<dyn Decoder>> {
        self.make_decoder_with(params, &CodecPreferences::default())
    }

    /// Default-preference shorthand for `make_encoder_with`.
    pub fn make_encoder(&self, params: &CodecParameters) -> Result<Box<dyn Encoder>> {
        self.make_encoder_with(params, &CodecPreferences::default())
    }

    /// Iterate codec ids that have at least one decoder implementation.
    pub fn decoder_ids(&self) -> impl Iterator<Item = &CodecId> {
        self.impls
            .iter()
            .filter(|(_, v)| v.iter().any(|i| i.make_decoder.is_some()))
            .map(|(id, _)| id)
    }

    pub fn encoder_ids(&self) -> impl Iterator<Item = &CodecId> {
        self.impls
            .iter()
            .filter(|(_, v)| v.iter().any(|i| i.make_encoder.is_some()))
            .map(|(id, _)| id)
    }

    /// All registered implementations of a given codec id.
    pub fn implementations(&self, id: &CodecId) -> &[CodecImplementation] {
        self.impls.get(id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Lookup the encoder options schema for a registered codec. Walks
    /// implementations in registration order and returns the first
    /// schema found. `None` means either the codec isn't registered or
    /// no implementation declared an encoder schema.
    pub fn encoder_options_schema(&self, id: &CodecId) -> Option<&'static [OptionField]> {
        self.impls
            .get(id)?
            .iter()
            .find_map(|i| i.encoder_options_schema)
    }

    /// Lookup the decoder options schema — see
    /// [`encoder_options_schema`](Self::encoder_options_schema).
    pub fn decoder_options_schema(&self, id: &CodecId) -> Option<&'static [OptionField]> {
        self.impls
            .get(id)?
            .iter()
            .find_map(|i| i.decoder_options_schema)
    }

    /// Iterator over every (codec_id, impl) pair — useful for `oxideav list`
    /// to show capability flags per implementation.
    pub fn all_implementations(&self) -> impl Iterator<Item = (&CodecId, &CodecImplementation)> {
        self.impls
            .iter()
            .flat_map(|(id, v)| v.iter().map(move |i| (id, i)))
    }

    /// Iterator over every `(tag, codec_id)` pair currently registered —
    /// used by `oxideav tags` debug output and by tests that want to
    /// walk the tag surface.
    pub fn all_tag_registrations(&self) -> impl Iterator<Item = (&CodecTag, &CodecId)> {
        self.tag_index.iter().flat_map(move |(tag, idxs)| {
            idxs.iter().map(move |&i| (tag, &self.registrations[i].id))
        })
    }

    /// Inherent form of tag resolution that returns a reference.
    /// The owned-value form used by container code lives behind the
    /// [`CodecResolver`] trait impl below.
    ///
    /// Walks every registration that claimed `ctx.tag`, calls its
    /// probe with `ctx`, and returns the id of the registration that
    /// scored highest. Probes that return `0.0` are discarded; ties
    /// on confidence are broken by registration order (first wins).
    /// Registrations with no probe are treated as returning `1.0`.
    pub fn resolve_tag_ref(&self, ctx: &ProbeContext) -> Option<&CodecId> {
        let idxs = self.tag_index.get(ctx.tag)?;
        let mut best: Option<(f32, usize)> = None;
        for &i in idxs {
            let rec = &self.registrations[i];
            let conf = match rec.probe {
                Some(f) => f(ctx),
                None => 1.0,
            };
            if conf <= 0.0 {
                continue;
            }
            best = match best {
                None => Some((conf, i)),
                Some((bc, _)) if conf > bc => Some((conf, i)),
                other => other,
            };
        }
        best.map(|(_, i)| &self.registrations[i].id)
    }
}

/// Implement the shared [`CodecResolver`] interface so container
/// demuxers can accept `&dyn CodecResolver` without depending on
/// this crate directly — the trait lives in oxideav-core.
impl CodecResolver for CodecRegistry {
    fn resolve_tag(&self, ctx: &ProbeContext) -> Option<CodecId> {
        self.resolve_tag_ref(ctx).cloned()
    }
}

/// Check whether an implementation's restrictions are compatible with the
/// requested codec parameters. `for_encode` swaps the rare cases where a
/// restriction only applies one way.
fn caps_fit_params(caps: &CodecCapabilities, p: &CodecParameters, for_encode: bool) -> bool {
    let _ = for_encode; // reserved for future use (e.g. encode-only bitrate caps)
    if let (Some(max), Some(w)) = (caps.max_width, p.width) {
        if w > max {
            return false;
        }
    }
    if let (Some(max), Some(h)) = (caps.max_height, p.height) {
        if h > max {
            return false;
        }
    }
    if let (Some(max), Some(br)) = (caps.max_bitrate, p.bit_rate) {
        if br > max {
            return false;
        }
    }
    if let (Some(max), Some(sr)) = (caps.max_sample_rate, p.sample_rate) {
        if sr > max {
            return false;
        }
    }
    if let (Some(max), Some(ch)) = (caps.max_channels, p.channels) {
        if ch > max {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tag_tests {
    use super::*;
    use crate::CodecCapabilities;

    /// Probe: return 1.0 iff the peeked bytes look like MS-MPEG4 (no
    /// 0x000001 start code in the first few bytes).
    fn probe_msmpeg4(ctx: &ProbeContext) -> f32 {
        match ctx.packet {
            Some(d) if !d.windows(3).take(6).any(|w| w == [0x00, 0x00, 0x01]) => 1.0,
            Some(_) => 0.0,
            None => 0.5, // no data yet — weak evidence
        }
    }

    /// Probe: return 1.0 iff the peeked bytes look like MPEG-4 Part 2
    /// (starts with a 0x000001 start code in the first few bytes).
    fn probe_mpeg4_part2(ctx: &ProbeContext) -> f32 {
        match ctx.packet {
            Some(d) if d.windows(3).take(6).any(|w| w == [0x00, 0x00, 0x01]) => 1.0,
            Some(_) => 0.0,
            None => 0.5,
        }
    }

    fn info(id: &str) -> CodecInfo {
        CodecInfo::new(CodecId::new(id)).capabilities(CodecCapabilities::audio(id))
    }

    #[test]
    fn resolve_single_claim_no_probe() {
        let mut reg = CodecRegistry::new();
        reg.register(info("flac").tag(CodecTag::fourcc(b"FLAC")));
        let t = CodecTag::fourcc(b"FLAC");
        assert_eq!(
            reg.resolve_tag_ref(&ProbeContext::new(&t))
                .map(|c| c.as_str()),
            Some("flac"),
        );
    }

    #[test]
    fn resolve_missing_tag_returns_none() {
        let reg = CodecRegistry::new();
        let t = CodecTag::fourcc(b"????");
        assert!(reg.resolve_tag_ref(&ProbeContext::new(&t)).is_none());
    }

    #[test]
    fn unprobed_claims_tie_first_registered_wins() {
        // Two unprobed claims on the same tag: deterministic order.
        let mut reg = CodecRegistry::new();
        reg.register(info("first").tag(CodecTag::fourcc(b"TEST")));
        reg.register(info("second").tag(CodecTag::fourcc(b"TEST")));
        let t = CodecTag::fourcc(b"TEST");
        assert_eq!(
            reg.resolve_tag_ref(&ProbeContext::new(&t))
                .map(|c| c.as_str()),
            Some("first"),
        );
    }

    #[test]
    fn probe_picks_matching_bitstream() {
        // The core bug fix: every probe is asked and the highest
        // confidence wins regardless of registration order.
        let mut reg = CodecRegistry::new();
        reg.register(
            info("msmpeg4v3")
                .probe(probe_msmpeg4)
                .tag(CodecTag::fourcc(b"DIV3")),
        );
        reg.register(
            info("mpeg4video")
                .probe(probe_mpeg4_part2)
                .tag(CodecTag::fourcc(b"DIV3")),
        );

        let mpeg4_part2 = [0x00u8, 0x00, 0x01, 0xB0, 0x01, 0x00];
        let ms_mpeg4 = [0x85u8, 0x3F, 0xD4, 0x80, 0x00, 0xA2];
        let tag = CodecTag::fourcc(b"DIV3");

        let ctx_part2 = ProbeContext::new(&tag).packet(&mpeg4_part2);
        assert_eq!(
            reg.resolve_tag_ref(&ctx_part2).map(|c| c.as_str()),
            Some("mpeg4video"),
        );
        let ctx_ms = ProbeContext::new(&tag).packet(&ms_mpeg4);
        assert_eq!(
            reg.resolve_tag_ref(&ctx_ms).map(|c| c.as_str()),
            Some("msmpeg4v3"),
        );
    }

    #[test]
    fn unprobed_claim_wins_against_low_confidence_probe() {
        // One codec claims a tag without a probe (→ confidence 1.0)
        // and another claims it with a probe returning 0.3. The
        // unprobed one wins — a codec that knows it owns the tag
        // outright should not lose to a speculative probe.
        let mut reg = CodecRegistry::new();
        reg.register(info("owner").tag(CodecTag::fourcc(b"OWN_")));
        reg.register(
            info("speculative")
                .probe(|_| 0.3)
                .tag(CodecTag::fourcc(b"OWN_")),
        );
        let t = CodecTag::fourcc(b"OWN_");
        assert_eq!(
            reg.resolve_tag_ref(&ProbeContext::new(&t))
                .map(|c| c.as_str()),
            Some("owner"),
        );
    }

    #[test]
    fn probe_returning_zero_is_skipped() {
        let mut reg = CodecRegistry::new();
        reg.register(
            info("refuses")
                .probe(|_| 0.0)
                .tag(CodecTag::fourcc(b"MAYB")),
        );
        reg.register(info("fallback").tag(CodecTag::fourcc(b"MAYB")));
        let t = CodecTag::fourcc(b"MAYB");
        let ctx = ProbeContext::new(&t).packet(b"hello");
        assert_eq!(
            reg.resolve_tag_ref(&ctx).map(|c| c.as_str()),
            Some("fallback"),
        );
    }

    #[test]
    fn fourcc_case_insensitive_lookup() {
        let mut reg = CodecRegistry::new();
        reg.register(info("vid").tag(CodecTag::fourcc(b"div3")));
        // Registered as "DIV3" (uppercase via ctor); lookup using
        // lowercase / mixed case also hits.
        let upper = CodecTag::fourcc(b"DIV3");
        let lower = CodecTag::fourcc(b"div3");
        let mixed = CodecTag::fourcc(b"DiV3");
        assert!(reg.resolve_tag_ref(&ProbeContext::new(&upper)).is_some());
        assert!(reg.resolve_tag_ref(&ProbeContext::new(&lower)).is_some());
        assert!(reg.resolve_tag_ref(&ProbeContext::new(&mixed)).is_some());
    }

    #[test]
    fn wave_format_and_matroska_tags_work() {
        let mut reg = CodecRegistry::new();
        reg.register(info("mp3").tag(CodecTag::wave_format(0x0055)));
        reg.register(info("h264").tag(CodecTag::matroska("V_MPEG4/ISO/AVC")));
        let wf = CodecTag::wave_format(0x0055);
        let mk = CodecTag::matroska("V_MPEG4/ISO/AVC");
        assert_eq!(
            reg.resolve_tag_ref(&ProbeContext::new(&wf))
                .map(|c| c.as_str()),
            Some("mp3"),
        );
        assert_eq!(
            reg.resolve_tag_ref(&ProbeContext::new(&mk))
                .map(|c| c.as_str()),
            Some("h264"),
        );
    }

    #[test]
    fn mp4_object_type_tag_works() {
        let mut reg = CodecRegistry::new();
        reg.register(info("aac").tag(CodecTag::mp4_object_type(0x40)));
        let t = CodecTag::mp4_object_type(0x40);
        assert_eq!(
            reg.resolve_tag_ref(&ProbeContext::new(&t))
                .map(|c| c.as_str()),
            Some("aac"),
        );
    }

    #[test]
    fn multi_tag_claim_all_resolve() {
        let mut reg = CodecRegistry::new();
        reg.register(info("aac").tags([
            CodecTag::fourcc(b"MP4A"),
            CodecTag::wave_format(0x00FF),
            CodecTag::mp4_object_type(0x40),
            CodecTag::matroska("A_AAC"),
        ]));
        for t in [
            CodecTag::fourcc(b"MP4A"),
            CodecTag::wave_format(0x00FF),
            CodecTag::mp4_object_type(0x40),
            CodecTag::matroska("A_AAC"),
        ] {
            assert_eq!(
                reg.resolve_tag_ref(&ProbeContext::new(&t))
                    .map(|c| c.as_str()),
                Some("aac"),
                "tag {t:?} did not resolve",
            );
        }
    }
}

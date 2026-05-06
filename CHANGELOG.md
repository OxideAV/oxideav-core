# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.22](https://github.com/OxideAV/oxideav-core/compare/v0.1.21...v0.1.22) - 2026-05-05

### Other

- add HwDeviceInfo / HwCodecCaps types + CodecInfo engine_id / engine_probe

### Added

- `engine` module with `HwDeviceInfo`, `HwCodecCaps`, and
  `EngineProbeFn` (a `fn() -> Vec<HwDeviceInfo>` alias). Public types
  the HW-accel sibling crates use to describe per-device capability
  matrices.
- `CodecInfo::engine_id: Option<&'static str>` + `with_engine_id(&'static str)`
  builder method. Set by HW siblings to identify which backend a codec
  entry came from; consumers (CLI `info` command) group + dedupe by it.
- `CodecInfo::engine_probe: Option<EngineProbeFn>` + `with_engine_probe(fn)`
  builder method. Lets a HW sibling attach a probe function the consumer
  calls on demand to enumerate the backend's devices.
- Tests covering type roundtrip + the new `CodecInfo` builders.

### Notes

- No distributed slice, no macro: engine info travels with each
  `CodecInfo`, matching the explicit-calls pattern already used by
  `oxideav-meta`'s `register_all`.

## [0.1.21](https://github.com/OxideAV/oxideav-core/compare/v0.1.20...v0.1.21) - 2026-05-05

### Other

- apply cargo fmt to registry/codec.rs (rustfmt CI fix)
- trim core API to registration + first-match lookups; selection policy moves to oxideav-pipeline

## [0.1.20](https://github.com/OxideAV/oxideav-core/compare/v0.1.19...v0.1.20) - 2026-05-05

### Other

- add require_hardware to disable SW fallback

## [0.1.19](https://github.com/OxideAV/oxideav-core/compare/v0.1.18...v0.1.19) - 2026-05-05

### Other

- add Gbrp10/12/14Le + Gbrap10/12/14Le PixelFormat variants

### Added

- Six new `PixelFormat` variants for high-bit-depth planar GBR(A):
  `Gbrp10Le`, `Gbrap10Le`, `Gbrp12Le`, `Gbrap12Le`, `Gbrp14Le`,
  `Gbrap14Le` (discriminants 35..40, appended after `Yuv411P`). Planes
  are ordered G, B, R (and A on the alpha variants), each sample stored
  as a 16-bit little-endian word with the top bits zero — matching the
  ffmpeg `AV_PIX_FMT_GBRP*LE` family. Wired into `is_planar`,
  `plane_count`, `has_alpha`, and `bits_per_pixel_approx`. Unblocks
  `oxideav-magicyuv` 12/14-bit lossless GBR returns and the JPEG 2000 /
  OpenEXR / TIFF 12-bit GBR workflows.

## [0.1.18](https://github.com/OxideAV/oxideav-core/compare/v0.1.17...v0.1.18) - 2026-05-05

### Other

- add linkme-based distributed-slice auto-registration (REGISTRARS + register! macro)

## [0.1.17](https://github.com/OxideAV/oxideav-core/compare/v0.1.16...v0.1.17) - 2026-05-04

### Other

- add structured chapters() / attachments() API
- add VectorFrame::default() + small DX wins ([#367](https://github.com/OxideAV/oxideav-core/pull/367))

### Added

- New `crate::metadata` module — `Chapter` and `Attachment` structs
  for structured container metadata (re-exported at the crate root).
  `Chapter { id, start, end, title, language }` covers MKV
  `Chapters`, MP4 chapter tracks, and Ogg `CHAPTERnn=` Vorbis
  comments; `Attachment { name, mime, description, data }` covers
  MKV `Attachments`. Both fields-public, `Clone + Debug + PartialEq`.
- `Demuxer::chapters() -> &[Chapter]` and
  `Demuxer::attachments() -> &[Attachment]` trait methods. Both have
  default implementations returning `&[]`, so every existing demuxer
  (mkv, mp4, mp3, ogg, flac, avi, wav, …) compiles unchanged. The
  legacy flat `chapter:N:*` / `attachment:N:*` metadata keys keep
  working for now; demuxers will migrate to the structured accessors
  in follow-up rounds.

## [0.1.16](https://github.com/OxideAV/oxideav-core/compare/v0.1.15...v0.1.16) - 2026-05-03

### Other

- add Node::SoftMask + MaskKind enum

### Added

- `vector::Node::SoftMask { mask, mask_kind, content }` variant +
  `MaskKind { Luminance, Alpha }` enum — covers SVG `<mask>` and PDF
  `SMask` (subtype `Luminosity` vs. `Alpha`) at the IR level so the
  rasterizer can apply per-pixel alpha modulation. The `mask` subtree
  is rasterised separately and converted to coverage (BT.709 luminance
  for `Luminance`, the mask's own alpha channel for `Alpha`); the
  `content` subtree is then composited under that coverage. Matches
  the SVG semantics where a `<mask>` rasterises into a 1-channel
  bitmap that multiplies the masked content's per-pixel alpha.
- `vector::VectorFrame::default()` — empty 0×0 frame with default root
  group, no view box, no timestamp, and a `1/1` time base. Useful for
  builder-style construction or as a `std::mem::take` placeholder.
- DX wins on the `vector` module — purely additive convenience
  constructors and builder-style chainable setters. No existing field,
  signature, or behaviour changed.
  - inherent `new()` constructors: `VectorFrame::new(width, height)`,
    `Group::new()`, `PathNode::new(path)`, `LinearGradient::new(start, end)`,
    `RadialGradient::new(center, radius)`, `Stroke::new(width, paint)`,
    `DashPattern::new(array)`, `ViewBox::new(min_x, min_y, w, h)`,
    `Rect::new(x, y, w, h)`.
  - `with_*` builders that take `self` and return `Self`:
    `VectorFrame::{with_view_box, with_root, with_pts, with_time_base}`,
    `Group::{with_transform, with_opacity, with_clip, with_child,
    with_children, with_cache_key}`,
    `PathNode::{with_fill, with_stroke, with_fill_rule}`,
    `LinearGradient::{with_stops, with_stop, with_spread}`,
    `RadialGradient::{with_focal, with_stops, with_stop, with_spread}`,
    `Stroke::{with_paint, with_cap, with_join, with_miter_limit, with_dash}`,
    `DashPattern::with_offset`.
  - `From` conversions: `From<[f32; 2]>` and `From<(f32, f32)>` for
    `Point`; `From<(u8, u8, u8, u8)>`, `From<(u8, u8, u8)>`, and
    `From<[u8; 4]>` for `Rgba`; `From<Rgba>` for `Paint` (wraps in
    `Paint::Solid`).

## [0.1.15](https://github.com/OxideAV/oxideav-core/compare/v0.1.14...v0.1.15) - 2026-05-03

### Other

- add Group::cache_key + fix missing Frame::Vector match arm

### Added

- `vector::Group::cache_key: Option<u64>` — opaque memoisation key for
  cacheable scene-graph subtrees. Producers that emit identifiable
  cacheable content (e.g. a scribe-shaped glyph at a specific
  `(face_id, glyph_id, size_q8, subpixel_x)`) compute a deterministic
  hash and put it here; downstream rasterizers can use it as a bitmap-
  cache key. Optional and namespace-agnostic — `oxideav-core` never
  inspects the value.

### Fixed

- `registry::codec::Decoder::receive_arena_frame` default impl missed
  the `Frame::Vector(_)` arm (added in 0.1.14 alongside the new variant
  but the match wasn't updated). Returns `Error::Unsupported` to match
  the audio / subtitle behaviour — no arena-backed representation for
  vector frames today.

## [0.1.14] - 2026-05-04

### Added

- New `crate::vector` module — primitive types for resolution-independent
  vector-graphics frames. The set is the SVG 1.1 / PDF 1.4 intersection
  so the same `VectorFrame` round-trips through both formats without
  lossy conversion: `Path` (move / line / quadratic / cubic / elliptic-arc
  / close commands), `PathNode` with optional fill / stroke / fill rule,
  `Group` (transform / opacity / optional clip / children), `Paint`
  (`Solid` / `LinearGradient` / `RadialGradient` with `Pad` / `Reflect`
  / `Repeat` spread), `Stroke` (cap / join / miter limit / dash),
  `Transform2D` 2D affine matrix with `identity` / `translate` /
  `scale` / `rotate` / `skew_x` / `skew_y` / `compose` / `apply`,
  `Rgba`, `Rect`, `ViewBox`, and `ImageRef` for embedded raster
  passthrough (carries a child `VideoFrame`).
- `Frame::Vector(VectorFrame)` variant. The `Frame` enum is already
  `#[non_exhaustive]`, so adding the variant is an additive change for
  downstream `match` arms with a wildcard.
- All vector types are re-exported at the crate root.

### Changed

- `Decoder::receive_arena_frame` default impl gains a `Frame::Vector`
  arm returning `Error::Unsupported` (the arena `Frame` body is
  video-only — vector frames have no arena-backed representation
  today). Required to keep the in-tree exhaustive match compiling.

### Notes

- Text nodes are intentionally deferred — they need font handling and
  scribe coupling that lands alongside `oxideav-svg` (#349).
- No rasterizer / SVG parser / PDF writer in this crate; those are
  downstream tasks (#349 / #350 / #351). Round 1 ships only the IR.

## [0.1.13](https://github.com/OxideAV/oxideav-core/compare/v0.1.12...v0.1.13) - 2026-05-03

### Other

- add Yuv411P (8-bit YUV 4:1:1 planar)
- drop duplicate semver_check key
- replace never-match regex with semver_check = false
- drop enable_miri input (miri now manual-only via workflow_dispatch)

## [0.1.12](https://github.com/OxideAV/oxideav-core/compare/v0.1.11...v0.1.12) - 2026-05-02

### Other

- replace int-to-ptr sentinel with aligned static for strict-provenance
- migrate to centralized OxideAV/.github reusable workflows

### Fixed

- `arena::Buffer::new_zeroed` no longer uses an integer-to-pointer
  cast for the `cap == 0` empty-buffer sentinel. The previous
  `MAX_ALIGN as *mut u8` synthesis was rejected by Miri's
  `-Zmiri-strict-provenance` check (the resulting pointer has no
  provenance and cannot legally be reborrowed). Replaced with the
  address of a `#[repr(align(64))]`-aligned zero-sized static
  (`EMPTY_SENTINEL`), reached via `NonNull::from(&EMPTY_SENTINEL)
  .cast::<u8>()` — same runtime properties (non-null, `MAX_ALIGN`
  -aligned, never dereferenced) but with real strict-provenance
  -compatible provenance. A `const`-eval'd assert pins the static's
  alignment to `MAX_ALIGN` so the day someone bumps `MAX_ALIGN` they
  also remember to update the `#[repr(align(N))]` literal.

## [0.1.11](https://github.com/OxideAV/oxideav-core/compare/v0.1.10...v0.1.11) - 2026-05-02

### Other

- fix four UB issues surfaced by miri audit
- typed-source traits (Bytes / Packets / Frames)

### Changed

- **Breaking**: `SourceRegistry` now returns a `SourceOutput` enum from
  `open()` instead of a bare `Box<dyn ReadSeek>`. The enum has three
  variants — `Bytes`, `Packets`, `Frames` — backed by three new traits
  (`BytesSource`, `PacketSource`, `FrameSource`) that drivers implement.
  `BytesSource` is `Read + Seek + Send` and is blanket-implemented for
  every type that satisfies the bounds, so existing readers (`File`,
  `Cursor<Vec<u8>>`, the HTTP-Range adapter) work unchanged.
  `PacketSource` and `FrameSource` let transport-layer protocols (RTMP,
  …) and synthetic generators register themselves on the same opener
  API by skipping the demux / decode upstream stages.
- **Breaking**: The old `register(scheme, OpenSourceFn)` method and the
  `OpenSourceFn` type alias are removed. Drivers register via one of
  `register_bytes`, `register_packets`, or `register_frames` depending
  on the source shape. Every in-tree driver migrates atomically.

## [0.1.10](https://github.com/OxideAV/oxideav-core/compare/v0.1.9...v0.1.10) - 2026-05-01

### Added

- New `Decoder::receive_arena_frame() -> Result<arena::sync::Frame>`
  method on the public `Decoder` trait. Returns the next decoded frame
  as an arena-backed `arena::sync::Frame` so the caller can read plane
  bytes straight out of the decoder's arena buffer with no per-plane
  memcpy. Decoders that build their output through an
  `arena::sync::ArenaPool` should override this to expose true
  zero-copy frames (the first two such ports are oxideav-h261 and
  oxideav-h263). Default implementation delegates to `receive_frame()`
  and copies the resulting `VideoFrame` planes into a freshly-leased
  one-shot `arena::sync::ArenaPool` — additive change for every
  existing `Decoder` impl, no source changes required to keep
  compiling. Audio / subtitle frames return `Error::Unsupported` from
  the default impl (the arena `Frame` body is video-only in round 2).

### Notes

- 0.2.0 was published earlier today and immediately yanked + tag
  deleted. This 0.1.10 patch carries the same `receive_arena_frame()`
  addition under a patch version, since the change is purely additive
  (new method with default impl; no existing `Decoder` impl breaks).

## [0.1.9](https://github.com/OxideAV/oxideav-core/compare/v0.1.8...v0.1.9) - 2026-05-01

### Added

- New `crate::arena::sync` sibling module — `Send + Sync` mirror of
  `crate::arena`. Same four-type API (`ArenaPool`, `Arena`, `Frame`,
  `FrameInner`) with `Arena` backed by `AtomicUsize` / `AtomicU32`
  instead of `Cell`, and `Frame = Arc<FrameInner>` instead of
  `Rc<FrameInner>`. Built for the cross-thread decode path, where a
  decoder produces frames on one worker and a consumer (renderer /
  encoder / network sink) reads them on another. The `Rc` variant
  remains the cheaper choice for same-thread decode/consume.
- `arena::sync::Arena::alloc` uses a CAS loop on the cursor, so
  concurrent allocators on the same `&Arena` receive disjoint slices.
  The typical pattern is still alloc-then-freeze on the producer
  thread; the CAS path exists so the `Sync` bound is sound rather
  than as a perf-relevant feature.
- `FrameHeader` and `MAX_PLANES` are re-exported from `arena::sync`,
  so users of either module see the same metadata shape — there is
  no thread-safety angle to either of them.

## [0.1.8](https://github.com/OxideAV/oxideav-core/compare/v0.1.7...v0.1.8) - 2026-05-01

### Other

- DoS framework round 1: DecoderLimits + arena pool + Frame

### Added

- `DecoderLimits` (in new `crate::limits` module) — small `Copy + Default`
  cap struct (`max_pixels_per_frame`, `max_alloc_bytes_per_frame`,
  `max_alloc_count_per_frame`, `max_arenas_in_flight`,
  `max_decoded_audio_seconds_per_packet`) for cross-cutting decoder DoS
  protection. Conservative-but-finite defaults (32 k × 32 k pixels, 1 GiB
  per arena, 60 s of decoded audio per packet) so no real-world stream
  changes; harden via the `with_*` builder methods.
- `CodecParameters::limits()` accessor and `with_limits()` builder
  thread the caps through every decoder constructed from these
  parameters. New field on the existing `#[non_exhaustive]` struct;
  callers using the typed constructors (`audio()` / `video()` /
  `subtitle()` / `data()`) are unaffected.
- `Error::ResourceExhausted(String)` variant + `Error::resource_exhausted()`
  helper. Canonical "DoS protection fired" error for header-parse
  rejections and arena-pool exhaustion.
- New `crate::arena` module — refcounted arena pool for decoder frame
  allocations. `ArenaPool` (lazy-allocating buffer pool, `Send + Sync`),
  `Arena` (bump-pointer allocator over a `Box<[u8]>`, `!Send` by design),
  `FrameInner` / `Frame = Rc<FrameInner>` (refcounted handle whose last
  `Drop` returns its arena to the pool), and `FrameHeader` (minimal
  width / height / pixel-format / pts metadata). Hand-rolled bump
  allocator (no `bumpalo` dep yet); `Rc`-based `Frame` for the
  single-threaded decoder path — an `Arc` sibling type can be added
  later for the parallel-decoder path without breaking this one.

## [0.1.7](https://github.com/OxideAV/oxideav-core/compare/v0.1.6...v0.1.7) - 2026-04-26

### Other

- pin PixelFormat/SampleFormat discriminants + disable semver_check
- plan mid-stream ConfigChanged signal for Frame
- slim VideoFrame/AudioFrame — stream properties off the frame
- remove AudioFrame::layout() — layout is a stream property
- add ChannelLayout enum + AudioFrame/CodecParameters plumbing
- add Yuv422P12Le + Yuv444P12Le
- pin release-plz to patch-only bumps

### Added

- `PixelFormat::Yuv422P12Le` and `PixelFormat::Yuv444P12Le` — 12-bit
  4:2:2 and 4:4:4 planar YUV with little-endian 16-bit storage. Unblocks
  HEVC Main 12 4:2:2 / 4:4:4 surfaces in oxideav-h265.

## [0.1.6](https://github.com/OxideAV/oxideav-core/compare/v0.1.5...v0.1.6) - 2026-04-25

### Added

- add CodecParameters::subtitle() and ::data() builders

## [0.1.5](https://github.com/OxideAV/oxideav-core/compare/v0.1.4...v0.1.5) - 2026-04-25

### Other

- re-export Muxer trait from registry::container
- absorb codec/container/source/filter registries + RuntimeContext

## [0.1.4](https://github.com/OxideAV/oxideav-core/compare/v0.1.3...v0.1.4) - 2026-04-25

### Other

- add StreamFilter::reset() hook for seek barriers
- add StreamFilter trait + PortSpec + FilterContext
- bump thiserror 1 → 2

## [0.1.3](https://github.com/OxideAV/oxideav-core/compare/v0.1.2...v0.1.3) - 2026-04-20

### Added

- `options` module: `CodecOptions` (string key/value bag) + `OptionKind`,
  `OptionValue`, `OptionField`, `CodecOptionsStruct` trait, and a
  generic `parse_options::<T>()` that coerces the bag into a typed
  per-codec options struct using the struct's declared `SCHEMA`.
  Codec factories parse `CodecParameters::options` at init, so there's
  no per-packet overhead; strict validation rejects unknown keys and
  malformed values up front.
- `json-options` feature (off by default): adds a `serde_json`
  dependency and exposes `CodecOptions::from_json` /
  `from_json_value` / `parse_options_json`. Lets consumers such as
  `oxideav-pipeline` feed codec tuning in as JSON.
- `CodecParameters::options` field (`CodecOptions`) carrying codec
  tuning knobs. `matches_core` ignores it — tuning doesn't affect
  stream compatibility.

### Changed (breaking)

- `CodecParameters` is now `#[non_exhaustive]`. External crates that
  constructed it via `CodecParameters { ... }` struct-literal syntax
  must switch to the `::audio(id)` / `::video(id)` constructors (or
  functional-update `{ ..base }` syntax). The trade-off is that adding
  new fields from here on is no longer a semver break. Shipped under a
  patch version intentionally — downstream oxideav-* siblings pin
  `"0.1"` and widening them all to `"0.2"` in lockstep was not
  practical.

## [0.1.2](https://github.com/OxideAV/oxideav-core/compare/v0.1.1...v0.1.2) - 2026-04-19

### Other

- add PixelFormat::Cmyk
- release v0.2.0

## [0.1.1](https://github.com/OxideAV/oxideav-core/compare/v0.0.8...v0.1.1) - 2026-04-19

### Other

- core 0.1.1 — ProbeContext / ProbeFn / resolve_tag signature
- add ProbeContext + probe-confidence ProbeFn
- refresh README for 0.1 — add CodecTag/CodecResolver + bits module
- release v0.1.0
- oxideav-core 0.1.0

## [0.1.0](https://github.com/OxideAV/oxideav-core/compare/v0.0.8...v0.1.0) - 2026-04-19

### Other

- oxideav-core 0.1.0

## [0.0.8](https://github.com/OxideAV/oxideav-core/compare/v0.0.7...v0.0.8) - 2026-04-19

### Other

- wrap long assert! in NullCodecResolver test

## [0.0.6](https://github.com/OxideAV/oxideav-core/compare/v0.0.5...v0.0.6) - 2026-04-19

### Other

- add CodecTag enum for container → codec resolution

## [0.0.5](https://github.com/OxideAV/oxideav-core/compare/v0.0.4...v0.0.5) - 2026-04-19

### Other

- add read_unary + write_unary

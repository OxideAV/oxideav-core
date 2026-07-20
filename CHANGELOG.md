# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `ExecutionContext::auto()` — caller-side budget derived from
  `available_parallelism()` (fallback 1), and
  `ExecutionContext::effective_workers(work_units)` — the uniform
  codec-side clamp for every internal fan-out.
- Documented threading contract on the `execution` module: the context
  is the single threading authority; codecs run serial until granted a
  budget, bound all fan-out via `effective_workers`, and never query
  host parallelism directly.

## [0.1.31](https://github.com/OxideAV/oxideav-core/compare/v0.1.30...v0.1.31) - 2026-07-19

### Other

- README + CHANGELOG: deep Yuva formats and the significant-bits side-channel
- per-plane significant-bits side-channel on VideoFrame
- add deep alpha-carrying planar YUV (Yuva 4:2:2/4:4:4 at 10/12/16-bit)

### Added

- Deep alpha-carrying planar YUV pixel formats completing the Yuva
  family: `Yuva422P10Le` (46), `Yuva422P12Le` (47), `Yuva444P10Le`
  (48), `Yuva444P12Le` (49), `Yuva422P16Le` (50), `Yuva444P16Le` (51)
  — 4-plane planar with full-resolution alpha as plane 3, LE 16-bit
  words, pinned discriminants.
- Per-plane significant-bits side-channel on `VideoFrame`
  (`significant_bits` / `plane_significant_bits` /
  `set_significant_bits` / `with_significant_bits` /
  `take_significant_bits`): producers can express mixed per-plane bit
  depths (e.g. 12-bit luma + 10-bit chroma from a custom signal range)
  on an existing storage `PixelFormat`, LSB-anchored, composing with
  the palette side-channel on the same frame.

## [0.1.30](https://github.com/OxideAV/oxideav-core/compare/v0.1.29...v0.1.30) - 2026-07-17

### Other

- reflect 16-bit YUV, Yuva 4:2:2/4:4:4, and the palette side-channel
- add palette side-channel to VideoFrame
- add alpha-carrying planar YUV at 4:2:2 and 4:4:4 (Yuva422P/Yuva444P)
- add 16-bit planar YUV pixel formats (Yuv420P16Le/Yuv422P16Le/Yuv444P16Le)
- reword a historical entry to describe the GBR plane layout on its own terms
- README numeric-core/bits/error refresh + CHANGELOG for the r399 hardening round
- fix all broken intra-doc links — docs.rs-strict (RUSTDOCFLAGS=-D warnings) now clean
- crate-wide missing_docs sweep + #![warn(missing_docs)] lock-in
- criterion harness for the hot primitives (bits / rescale / Rational)
- taxonomy contract docs + constructors and classification predicates
- LSB reader/writer surface parity with the MSB pair
- property harness for Rational / rescale / bit-I/O foundations
- total rescale core — saturate instead of wrap, checked + rounding-mode variants
- total overflow-safe arithmetic — i64::MIN-safe reduce/neg/abs/cmp + checked_* ops + approximate-instead-of-wrap narrowing
- add CI / crates.io / docs.rs / MIT-license badges

### Fixed

- `Rational` is now total on every input — no debug-build panic, no
  release-build wrap, even with `i64::MIN` terms or zero denominators.
  Four latent overflow bugs fixed: `reduced()` overflowed negating an
  `i64::MIN` numerator (and a `u64→i64` gcd cast could go negative,
  silently skipping reduction of `i64::MIN / i64::MIN`); `Neg`
  overflowed on an `i64::MIN` numerator (the sign now moves to the
  denominator, denoting the same value); `abs()` / `signum()` /
  `cmp_value()` sign normalization overflowed on `i64::MIN` terms (now
  `i128`-wide); and the arithmetic operators narrowed the reduced
  `i128` result with a plain `as i64`, wrapping out-of-range results —
  they now return the closest representable approximation (saturated
  numerator for magnitude overflow, rescaled `i64::MAX` denominator
  for precision overflow).
- `rescale` no longer wraps: the rounded 128-bit result used to be
  narrowed `as i64` (silently corrupting e.g. `i64::MAX` seconds →
  milliseconds); it now saturates to `i64::MAX` / `i64::MIN` by sign.
  The 128-bit product itself is checked too (three near-`i64::MAX`
  terms could overflow `i128` and panic in debug builds), and
  negative-denominator conversions now round ties away from zero like
  every other sign combination (previously toward zero).
- `registry::container` docs described `ProbeData` / `ProbeScore` in
  terms of the codec-level `ProbeFn`; they now correctly reference
  `ContainerProbeFn`. All eleven broken/ambiguous intra-doc links
  fixed; `cargo doc` is clean under `RUSTDOCFLAGS="-D warnings"`.

### Added

- `PixelFormat::Yuv420P16Le` / `Yuv422P16Le` / `Yuv444P16Le` — 16-bit
  planar YUV at the three standard chroma samplings (discriminants
  41–43). Same three-plane little-endian-16-bit-word layout as the
  10/12-bit variants, but all 16 bits of every word are significant
  (full-scale is 65535). Unblocks the SMPTE VC-2 / Dirac video-format
  presets 7 and 8, whose signal range goes to 16 bits per component.
- `PixelFormat::Yuva422P` / `Yuva444P` — 8-bit planar YUV + alpha at
  4:2:2 and 4:4:4 chroma samplings (discriminants 44–45), completing
  the `Yuva420P` family. The alpha plane is always full resolution
  (never chroma-subsampled), appended after V as plane index 3.
  Closes the ProRes 4444 alpha-carriage gap.
- `VideoFrame` palette side-channel — an additive, in-band way for
  palette-indexed frames (`PixelFormat::Pal8`) to carry their color
  table: `palette()` / `palette_rgb(index)` readers, `set_palette` /
  `with_palette` / `take_palette` writers, and `image_planes()` /
  `image_plane_count()` for palette-agnostic pixel iteration. The
  table rides as a trailing `VideoPlane` with `stride == 0` and
  non-empty `data` — a shape impossible for an image plane (whose
  `data` is `stride × rows` long), so the sentinel is unambiguous and
  frames without a palette are byte-for-byte unchanged. Entries are
  packed 3-byte RGB. The generic `receive_arena_frame` conversion now
  counts only image planes for its best-effort pixel-format label.
- `Rational::checked_add` / `checked_sub` / `checked_mul` /
  `checked_div` — exact reduced results, `None` exactly where the
  operators would approximate. The overflow policy is documented on
  the type.
- `Rounding` enum (`NearestAway` default / `Floor` / `Ceil` /
  `TowardZero`, `#[non_exhaustive]`) with `rescale_rnd(value, from,
  to, rounding)`, `TimeBase::rescale_rnd`, and `Timestamp::rescale_rnd`
  — `Floor` is the DTS-safe choice, `Ceil` the end-stamp choice.
- `rescale_checked(value, from, to) -> Option<i64>`,
  `TimeBase::rescale_checked`, and `Timestamp::checked_rescale` —
  `None` on an undefined conversion factor or out-of-range result,
  for callers that must not accept a defaulted/saturated stamp.
  Root re-exports: `rescale`, `rescale_rnd`, `rescale_checked`,
  `Rounding`.
- LSB bit-I/O surface parity with the MSB pair: `BitReaderLsb` gains
  `with_position`, `byte_position`, `bits_remaining`, `align_to_byte`,
  `peek_u32`, `skip`/`consume`, `read_u1`, `read_bytes`;
  `BitWriterLsb` gains `byte_len`, `is_byte_aligned`, `write_bits`,
  `write_i32`, `write_byte`, `write_bytes`, `bytes`/`buffer`,
  `into_bytes`.
- `Error` helpers: `format_not_found` / `codec_not_found`
  constructors and `is_eof` / `is_need_more` / `is_starved` /
  `is_resource_exhausted` predicates, plus a caller-action-oriented
  variant taxonomy in the module docs.
- Property-test harness (`tests/props.rs`): 12 deterministic
  fixed-seed suites (~200k edge-biased cases) checking `Rational`
  arithmetic and `rescale` against independent `i128` oracles,
  rounding-mode bracketing, monotonicity-through-saturation, and
  bit-I/O roundtrip/position invariants.
- Criterion bench harness (`benches/primitives.rs`) covering the
  bit-I/O hot paths, the rescale kernel, and `Rational` arithmetic,
  with recorded baselines.
- Crate-wide documentation completeness: every public item now has a
  rustdoc comment and the crate root enforces
  `#![warn(missing_docs)]` (deny via CI's clippy gate).

## [0.1.29](https://github.com/OxideAV/oxideav-core/compare/v0.1.28...v0.1.29) - 2026-06-09

### Other

- TimeBase named constants + Timestamp arithmetic + ticks_of inverse
- drop release-plz.toml — use release-plz defaults across the workspace
- add PictureType::to_u8/is_known + AttachedPicture builders
- add flag builders + end_pts accessor

### Added

- `TimeBase` typed-helper round: `from_rate(u32)` constructor wrapping
  the `1/rate` convention (audio sample-clocks + 90 kHz MPEG / RTP +
  microsecond / millisecond bases); `num()` / `den()` `const`
  accessors over the underlying [`Rational`] (sugar over `tb.0.num`);
  `is_valid()` predicate (`num != 0 && den != 0`) for branching past
  the `1/0` placeholder demuxers stamp on data-only streams;
  `ticks_of(seconds: f64) -> i64` — overflow-clamped, half-away-from-zero
  inverse of the existing `seconds_of(ticks)`, so muxers that have a
  wall-clock target can land it on the stream's base without re-rolling
  the divide-and-round at every call site.
- Named `TimeBase` constants for the rates that recur across the
  workspace: `SECONDS` (`1/1`), `MILLIS` (`1/1000`), `MICROS`
  (`1/1_000_000`), `NANOS` (`1/1_000_000_000`), `MPEG_TS` (`1/90_000`,
  MPEG-TS / RTP video PTS clock), `AUDIO_48K` / `AUDIO_44K1` /
  `AUDIO_8K` (canonical audio sample-clocks). Replaces the
  `TimeBase::new(1, 90_000)` / `TimeBase::new(1, 48_000)` magic-numbers
  scattered across `oxideav-mp4` / `oxideav-mkv` / `oxideav-ts` /
  `oxideav-rtp` / dozens of audio-codec test fixtures with named
  constants, so grep for `MPEG_TS` finds every 90-kHz call site.
- `Timestamp::from_seconds(seconds: f64, base: TimeBase)` — sugar over
  `Timestamp::new(base.ticks_of(s), base)` so producers that have a
  wall-clock starting point can construct a timestamp in one call.
- `Timestamp::checked_add_ticks(i64)` / `checked_sub_ticks(i64)` —
  overflow-checked tick arithmetic that returns `Option<Timestamp>`
  rather than wrapping silently; the base is preserved across the
  operation.
- `Timestamp::checked_diff(other: Timestamp) -> Option<i64>` —
  overflow-checked difference in `self.base`'s tick units. Rescales
  `other` onto `self`'s base first, so timestamps from different
  sources (different demuxers in a remux pipeline, B-frame DTS minus
  reference-frame PTS) subtract cleanly.
- Inline test coverage for every new helper: `ticks_of` round-trips
  `seconds_of` on integer multiples; half-away-from-zero ties match
  the existing `rescale` rounding (`+0.5 → +1`, `-0.5 → -1`);
  invalid-base / non-finite-seconds inputs return `0` instead of
  panicking; `from_seconds` round-trips through `seconds()`;
  `checked_add_ticks` / `checked_sub_ticks` surface `i64::MAX` /
  `i64::MIN` overflow as `None`; `checked_diff` rescales mixed-base
  pairs correctly (1 s at 48 kHz − 500 ms at 1 kHz = 24 000 ticks).

- `PictureType::to_u8(self) -> u8` — explicit inverse of the existing
  `from_u8(b)`. Equivalent to a `self as u8` cast (the enum is
  `#[repr(u8)]` with stable spec-assigned discriminants), but the
  named method documents the round-trip contract and surfaces the
  `Unknown → 0xFF` sentinel caveat so ID3v2 / FLAC writers can decide
  whether to refuse the reserved byte rather than silently emit it.
- `PictureType::is_known(self) -> bool` — `true` for every spec-
  assigned variant (`Other` … `PublisherLogo`), `false` for `Unknown`.
  Consumer-side gate before strict-mode picture-frame serialisation.
- `AttachedPicture::new(mime_type, picture_type)` constructor +
  chainable `with_description(impl Into<String>)`,
  `with_data(Vec<u8>)`, and `with_picture_type(PictureType)` builders.
  Parser-friendly counterpart to the public-field struct literal for
  ID3v2 / FLAC / MP4 / Vorbis producers that fill the picture
  incrementally as bytes scroll past the parse position.
- `AttachedPicture::is_external_link(&self) -> bool` — sugar over the
  ID3v2 `"-->"` MIME sentinel that flags `data` as a URL string
  instead of inline image bytes. Spares consumers from re-stating the
  three-byte sentinel literal at every link-vs-inline branch.
- Inline test coverage: every `0x00..=0x14` byte round-trips through
  `from_u8 → to_u8`, the `Unknown` sentinel re-emits as `0xFF` and is
  flagged by `is_known()`, and the `AttachedPicture` builders chain
  cleanly with the `"-->"` external-link detection.

- `Packet` gains builders for the previously-unmirrored
  [`PacketFlags`](crate::packet::PacketFlags) fields:
  `with_header(bool)`, `with_corrupt(bool)`, `with_discard(bool)`,
  `with_unit_boundary(bool)`, plus a `with_flags(PacketFlags)`
  shorthand that replaces the full flag set in one call. Demuxers
  that compute every flag up front (FLV header / discard markers,
  RTMP sequence-end tags, MKV cluster boundaries) can now chain
  every flag through the builder instead of mutating
  `pkt.flags.<field>` after construction.
- `Packet::with_stream_index(u32)` and `Packet::with_time_base(TimeBase)`
  — chainable counterparts to the public fields, for remuxers that
  build packets with placeholder values and remap them downstream.
- `Packet::end_pts() -> Option<i64>` — overflow-checked
  `pts + duration` accessor. Returns `None` when either timestamp
  is unknown or when the sum would overflow `i64` (rather than
  silently wrapping). Replaces the hand-rolled `pts.zip(duration)
  .map(|(p, d)| p + d)` muxers have been duplicating.
- `Packet::is_keyframe()` / `is_header()` / `is_discard()`
  convenience accessors mirroring the matching `with_*` builders.
  Purely sugar over `pkt.flags.<field>` for the three flags
  consumers branch on most often.
- Inline test coverage for every new builder / accessor, including
  the `end_pts` overflow guard and negative-PTS branch (B-frame
  pre-roll).

## [0.1.28](https://github.com/OxideAV/oxideav-core/compare/v0.1.27...v0.1.28) - 2026-05-30

### Other

- add MultiTitleSource trait + SourceOutput::MultiTitle variant
- add CodecParameters::language for per-track BCP-47 / ISO 639 tag

## [0.1.27](https://github.com/OxideAV/oxideav-core/compare/v0.1.26...v0.1.27) - 2026-05-29

### Other

- add arithmetic ops + value comparison; fix rescale rounding doc

### Added

- `Rational` arithmetic: `impl Add / Sub / Mul / Div / Neg`. Every
  operator computes its result with `i128` intermediates and returns
  the fraction in lowest terms, so products that transiently overflow
  `i64` but reduce back into range still yield the correct answer.
- `Rational::cmp_value` / `Rational::equals_value` — value comparison
  by overflow-safe `i128` cross-product, so `30000/1001` and `30/1`
  order correctly without reducing or losing precision. These are
  deliberately *not* the `Ord` / `PartialOrd` traits: the derived
  `Eq` / `Hash` on `Rational` are **structural** (`1/2 != 2/4`) to
  preserve the exact on-wire fraction, and a value-based `Ord` would
  violate the `Ord`/`Eq` consistency contract against that structural
  `Eq`. Replaces the hand-rolled `num * d == n * den` cross-products
  that consumer crates (e.g. oxideav-prores' `frame_rate_code_*`)
  have been duplicating. Zero denominators get a defensive signed-
  infinity total order.
- `Rational::signum` (`-1` / `0` / `1`) and `Rational::abs`, both
  sign-normalizing a negative denominator onto the numerator first.

### Fixed

- `time::rescale` doc comment claimed half-to-even rounding; the code
  implements half-away-from-zero. Corrected the comment and added a
  tie-rounding regression test.

## [0.1.26](https://github.com/OxideAV/oxideav-core/compare/v0.1.25...v0.1.26) - 2026-05-06

### Other

- remove tag_for_codec; tags are stream-level via CodecParameters::tag

### Added

- `CodecParameters::tag: Option<CodecTag>` field — the on-wire tag
  for this stream, set by the **producer** (demuxer at read-time,
  encoder via `output_params()` at configure-time). Plus a
  `CodecParameters::with_tag(tag)` builder helper. Wire tags are
  per-stream state (different `mpeg4video` streams correctly
  identify as `DIVX` / `XVID` / `MP4V` / `FMP4`; different `h264`
  streams as `H264` vs `AVC1`), so a stream-level field is the
  right home — muxers read `params.tag` directly and round-trip
  the demuxed FourCC byte-for-byte instead of going through the
  registry.

### Removed (BREAKING)

- `CodecResolver::tag_for_codec(&CodecId, CodecTagKind) -> Option<CodecTag>`
  trait method (added in 0.1.25), `CodecTagKind` enum,
  `CodecTag::kind()` helper, and `CodecRegistry::tag_for_codec_ref(...)`
  inherent. The architecture was wrong: the registry's
  "first-declared tag for this codec_id" answer is arbitrary on
  multi-tag codecs and breaks round-trip preservation. Use
  `CodecParameters::tag` instead — set by whoever produced the
  stream (demuxer when parsing existing media, encoder at
  configure-time). Forward `resolve_tag` direction is unchanged.

## [0.1.25](https://github.com/OxideAV/oxideav-core/compare/v0.1.24...v0.1.25) - 2026-05-06

### Other

- add CodecResolver::tag_for_codec inverse-lookup

## [0.1.24](https://github.com/OxideAV/oxideav-core/compare/v0.1.23...v0.1.24) - 2026-05-06

### Other

- drop linkme, swap macro to wrapper-fn dispatch
- propagate CodecInfo engine_id + engine_probe to CodecImplementation

## [0.1.23](https://github.com/OxideAV/oxideav-core/compare/v0.1.22...v0.1.23) - 2026-05-06

### Other

- collapse assert!(...) layout in device_index tests (rustfmt)
- add CodecParameters::device_index for HW device selection

### Added

- `CodecParameters::device_index: Option<u32>` + `with_device_index(u32)`
  builder method. Lets callers bind a HW decoder/encoder to a specific
  physical device by index (matching `engine_probe`'s device order). SW
  codecs ignore the field; HW codecs read it as
  `params.device_index.unwrap_or(0)`.
- `CodecImplementation` gains `engine_id: Option<&'static str>` and
  `engine_probe: Option<EngineProbeFn>` fields. The registry's
  `register()` method now copies them verbatim from the originating
  `CodecInfo` (previously dropped with `_` destructure).
- This unblocks per-device iteration in pipeline bench, and lets the
  CLI `info` command drop its impl-name substring lookup table.

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
  as a 16-bit little-endian word with the top bits zero. Wired into `is_planar`,
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

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

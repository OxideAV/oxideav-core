# oxideav-core

Core types for the [oxideav](https://github.com/OxideAV/oxideav-workspace)
pure-Rust media framework:

* **`Packet`** — one compressed chunk belonging to one stream, with
  timestamps. Chainable `with_*` builders cover every
  [`PacketFlags`](crate::packet::PacketFlags) field
  (`with_keyframe` / `with_header` / `with_corrupt` / `with_discard` /
  `with_unit_boundary`, plus a bulk `with_flags`) and the
  stream-index / time-base / pts / dts / duration setters used by
  demuxers and remuxers. An `end_pts()` accessor returns the
  overflow-checked `pts + duration` for muxers that need a per-
  packet end timestamp.
* **`Frame`** — one uncompressed audio / video / subtitle chunk.
* **`StreamInfo`** / **`CodecParameters`** — what a demuxer advertises and
  what a decoder / encoder consumes.
* **`TimeBase`** / **`Rational`** — rational time per stream; timestamps
  are integers in that base. `Rational` supports exact `+ - * /` and
  unary `-` (results reduced via `i128` intermediates), plus
  `cmp_value` / `equals_value` for value comparison (`30000/1001` vs
  `30/1`) that doesn't disturb the structural `Eq`/`Hash` callers rely
  on to preserve the on-wire fraction.
* **`PixelFormat`** / **`SampleFormat`** — enum of supported raw formats
  (40+ pixel variants including 8/10/12-bit YUV, 10/12/14-bit planar
  GBR(A), packed RGB/RGBA, NV12/NV21, all common sample layouts).
* **`AttachedPicture`** / **`PictureType`** — ID3v2 `APIC` taxonomy
  shared by ID3v2 / FLAC / MP4 / Vorbis cover-art carriage. `PictureType`
  round-trips byte-for-byte through `from_u8` ↔ `to_u8` over the spec-
  assigned `0x00..=0x14` range; unassigned bytes collapse to `Unknown`,
  flagged via `is_known()` so strict writers can refuse to emit the
  `0xFF` sentinel. `AttachedPicture::new(mime, kind)` plus chainable
  `with_description` / `with_data` / `with_picture_type` builders cover
  the producer side (parsers writing into a partially-decoded picture
  as bytes arrive), and `is_external_link()` distinguishes ID3v2's
  `"-->"` URL-sentinel mime from inline image bytes without having to
  hardcode the string at every call site.
* **`CodecTag`** / **`CodecResolver`** — neutral abstraction for mapping
  container-level tags (AVI FourCC, WAVEFORMATEX `wFormatTag`, MP4 OTI,
  Matroska CodecID strings) to oxideav `CodecId`s. Lets codec crates own
  their own tag claims without pulling a codec registry into every
  container.
* **`bits`** — shared MSB-first / LSB-first `BitReader` / `BitWriter`
  plus unary helpers. Used by the FLAC, AAC, H.264, HEVC, Vorbis and a
  dozen other codecs in the workspace.
* **`SourceRegistry`** — URI scheme dispatch for sources. Drivers
  register as one of three shapes — `BytesSource` (file / http), 
  `PacketSource` (transport-layer protocols that pre-demux), or
  `FrameSource` (synthetic generators that emit decoded frames) —
  and `open(uri)` returns a `SourceOutput` enum the pipeline executor
  branches on.
* **`Error`** — one unified error enum used across the ecosystem.

Zero C dependencies. Zero FFI. Zero `*-sys` crates.

## Usage

```toml
[dependencies]
oxideav-core = "0.1"
```

Everything downstream in oxideav (codec traits, container traits, codec
implementations, the CLI) depends on this crate transitively, so the
surface is kept deliberately small. The 0.1 series is the first stable
semver line — additive changes are `0.1.x` patch bumps; breaking
reshapes go to `0.2.0`.

## License

MIT — see [LICENSE](LICENSE).

# oxideav-core

Core types for the [oxideav](https://github.com/OxideAV/oxideav-workspace)
pure-Rust media framework:

* **`Packet`** — one compressed chunk belonging to one stream, with
  timestamps.
* **`Frame`** — one uncompressed audio / video / subtitle chunk.
* **`StreamInfo`** / **`CodecParameters`** — what a demuxer advertises and
  what a decoder / encoder consumes.
* **`TimeBase`** / **`Rational`** — rational time per stream; timestamps
  are integers in that base.
* **`PixelFormat`** / **`SampleFormat`** — enum of supported raw formats
  (30+ pixel variants, all common sample layouts).
* **`CodecTag`** / **`CodecResolver`** — neutral abstraction for mapping
  container-level tags (AVI FourCC, WAVEFORMATEX `wFormatTag`, MP4 OTI,
  Matroska CodecID strings) to oxideav `CodecId`s. Lets codec crates own
  their own tag claims without pulling a codec registry into every
  container.
* **`bits`** — shared MSB-first / LSB-first `BitReader` / `BitWriter`
  plus unary helpers. Used by the FLAC, AAC, H.264, HEVC, Vorbis and a
  dozen other codecs in the workspace.
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

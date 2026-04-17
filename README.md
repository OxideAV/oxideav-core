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
* **`Error`** — one unified error enum used across the ecosystem.

Zero C dependencies. Zero FFI. Zero `*-sys` crates.

## Usage

```toml
[dependencies]
oxideav-core = "0.0"
```

Everything downstream in oxideav (codec traits, container traits, codec
implementations, the CLI) depends on this crate transitively, so the
surface is intentionally small and stable.

## License

MIT — see [LICENSE](LICENSE).

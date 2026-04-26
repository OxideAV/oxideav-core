# Mid-stream config changes — design plan

**Status:** planned, not implemented.
**Owner:** oxideav-core. Surface lives on `Frame`; consumers and codecs adopt as needed.

## Why

The frame slim-down (April 2026) removed `format` / `width` / `height` / `sample_rate` / `channels` / `time_base` from `VideoFrame` and `AudioFrame`. Stream properties now live exclusively on `CodecParameters`, read once at decoder open and held by the consumer.

This works for the common case but leaves a hole: a handful of real-world codecs change their stream params **mid-bitstream**. The consumer's snapshot then goes stale and the next frames are interpreted wrong.

Confirmed cases worth supporting:

- **AC-3** can change `sample_rate` and `channel_count` between sync frames (ATSC A/52 §5.4.1 frame headers are independent; broadcast streams legitimately splice frames at different rates).
- **H.264 / H.265 / H.266** can change `width` / `height` / `pixel_format` (chroma subsampling, bit depth) at an IDR boundary when the SPS changes mid-GOP. Common in adaptive streaming after a representation switch and in some surveillance feeds.
- **Opus** can change `channel_count` between packets (§7 of RFC 6716 — though most encoders don't).
- **MPEG-1/2 video** sequence-header repeats can carry new `frame_rate` mid-stream.
- **MOD / S3M / IT trackers** can change `tempo` and `samples-per-row` at the next pattern row, which translates to a `sample_rate` change at the audio output if the player rounds frame size to a fixed pattern row.

What we have today: each codec silently keeps decoding under the consumer's stale snapshot, producing garbled output until the consumer happens to re-open or restart the decoder. Containers like MP4 / Matroska that snapshot params at demux time miss the change entirely.

## Design choice: why a `Frame` variant

Three shapes were considered.

**A. Side channel from the decoder** (`Decoder::take_param_changes() -> Option<CodecParameters>`).
Rejected: callers have to remember to drain it after every `receive_frame`, and forgetting is silent — no compile-time signal. Also breaks the "frames are the only thing flowing" mental model.

**B. New trait method `Decoder::receive_frame() -> Result<DecoderOutput>`** with `enum DecoderOutput { Frame(Frame), ConfigChanged(CodecParameters) }`.
Rejected: changes the trait signature. Every codec migrated in the slim-down would need a second touch. Worse, the `Frame` value already flows through pipeline / muxer / player layers via channels typed as `Frame` — wrapping it in `DecoderOutput` would force a rename cascade through ~30 crates.

**C. Recommended: extend the `Frame` enum with a `ConfigChanged` variant.**
`Frame` is already `#[non_exhaustive]`, so external consumers must wildcard-match on it; adding a variant is a backwards-compatible change at the API boundary. Internal matches in oxideav-core itself need a one-line update. The variant flows through every existing channel that carries `Frame` without changing any function signature.

```rust
#[non_exhaustive]
pub enum Frame {
    Audio(AudioFrame),
    Video(VideoFrame),
    Subtitle(SubtitleCue),
    /// In-band signal that the stream's `CodecParameters` have changed.
    /// The boxed params replace the consumer's snapshot for every
    /// subsequent `Audio` / `Video` / `Subtitle` frame, until another
    /// `ConfigChanged` supersedes it.
    ///
    /// Boxed because `CodecParameters` is the largest variant (~200 B
    /// with extradata + options), and `Frame` is on a hot path where
    /// every byte of enum-tag size matters.
    ConfigChanged(Box<CodecParameters>),
}
```

The `Box` matters. Without it, `mem::size_of::<Frame>` rises to the size of `CodecParameters` (≈200 B) for every `Audio` / `Video` frame in the queue. With it, the variant adds one `usize` to the enum and the heap allocation only happens on the rare config-change event.

## Semantics

1. **Forward-looking, not retroactive.** A `ConfigChanged` between frame N and frame N+1 means "frame N+1 is decoded under the new params." Frame N is unaffected.
2. **Full snapshot, not delta.** The boxed `CodecParameters` is the complete new state. Consumers replace, they do not merge. This avoids three-way merge ambiguity when multiple fields change at once.
3. **Idempotent emission.** A codec MAY emit `ConfigChanged` even when nothing changed (e.g. on every IDR). Consumers should compare the new params against their snapshot and no-op if equal — this is cheap and lets codecs emit defensively without coordination.
4. **Pre-first-frame is allowed.** A codec MAY emit a `ConfigChanged` before its very first `Audio` / `Video` frame (e.g. once it has parsed the first SPS and discovered the *true* dimensions, which differ from the conservative defaults the demuxer gave the decoder factory). This is the cleanest fix for the "decoder discovers real width only after parsing the first packet" problem currently handled by every codec in an ad-hoc way.
5. **Single-channel, single-stream.** `Frame::ConfigChanged` only describes the params of the stream the frame belongs to. Multi-stream config-change is just "each stream emits its own."
6. **`pts()` and `time_base()`** (the latter has been removed from `Frame`) are not meaningful for `ConfigChanged`. `Frame::pts()` returns `None` for the variant.

## Trait surface — no change

`Decoder::receive_frame() -> Result<Frame>` stays exactly as today. Codecs that want to surface a config change emit a `Frame::ConfigChanged(...)` from `receive_frame` instead of (or before) the next data frame.

`Encoder::send_frame(&Frame)` learns to accept `Frame::ConfigChanged` as a directive: "from this point forward, encode as if you were constructed with these params." Most encoders won't support this and should return `Error::Unsupported` — that's fine. Reconfigure-aware encoders (live transcoding rigs) can opt in.

## Consumer-side helper (proposed, not required)

A small struct on `oxideav-core` that consumers can hold to track current params automatically:

```rust
pub struct StreamParamsTracker {
    current: CodecParameters,
}

impl StreamParamsTracker {
    pub fn new(initial: CodecParameters) -> Self { Self { current: initial } }
    pub fn current(&self) -> &CodecParameters { &self.current }

    /// Returns `true` if the frame was a `ConfigChanged` and the snapshot
    /// was updated. Consumers typically `match` on this to decide whether
    /// to flush downstream state (resamplers, scalers, mixers).
    pub fn observe(&mut self, frame: &Frame) -> bool {
        if let Frame::ConfigChanged(p) = frame {
            self.current = (**p).clone();
            true
        } else {
            false
        }
    }
}
```

Consumers loop:

```rust
let mut tracker = StreamParamsTracker::new(stream_info.params.clone());
while let Ok(frame) = decoder.receive_frame() {
    if tracker.observe(&frame) {
        // Stream params just changed — flush resampler / scaler / mixer state
        // and rebuild filters that were initialized against the old params.
        rebuild_processing_chain(tracker.current());
        continue;
    }
    process(&frame, tracker.current())?;
}
```

The helper is optional — consumers that don't care about config changes can ignore it and they'll just see the same garbled-output behaviour they had before. The opt-in nature is intentional: most consumers (transcoders that re-open codecs on stream switch, simple file-to-file converts) genuinely don't need it.

## Implementation phases

### Phase 1 (when adopted)
Add the `ConfigChanged` variant to `Frame` in `oxideav-core/src/frame.rs`. Update `Frame::pts()` to return `None` for it. Add the `StreamParamsTracker` helper. Internal `match Frame { ... }` exhaustivity errors in oxideav-core itself: fix in same commit. ~50 LOC total.

External crates: most use `_ =>` arms because of `#[non_exhaustive]`. Spot-check with `cargo build --workspace --tests` and patch any explicit-match sites that the compiler flags.

### Phase 2 (per-codec opt-in, no big-bang)
Codecs adopt as needed. Each adoption is local: the codec watches its own input for config-changing structures (SPS for H.264, AC-3 sync header for AC-3, etc.) and emits `Frame::ConfigChanged` ahead of the next data frame. No coordination required across codecs.

Likely first adopters, in order of payoff:
- **AC-3** — broadcast splices already break audibly today; smallest fix surface.
- **H.264** — adaptive-streaming representation switches; biggest user-visible payoff.
- **MOD / S3M tracker codecs** — cleanest, smallest scope (already have a "next pattern row" boundary to hook).

### Phase 3 (consumers that benefit)
- **oxideplay** — extend `set_source_audio_params` / `set_source_video_params` driver setters to accept updates (they already exist as one-shot setters from the slim-down). Drivers re-arm sysaudio with new sample_rate / channel layout, video pipeline rebuilds the swapchain at new dimensions.
- **oxideav-pipeline** — `adapt_frame_for_encoder` learns to forward `Frame::ConfigChanged` to the encoder (which may accept or reject) and rebuilds intermediate filters.
- **oxideav-audio-filter / oxideav-image-filter** — filters that cached params at construction (Spectrogram, Resample, Edge, …) gain a `reconfigure(&CodecParameters)` method. The filter adapter calls it when it sees `ConfigChanged`.

## What this design deliberately does NOT do

- **No cross-stream signalling.** A video stream's config change does not imply anything about the audio stream's config and vice versa.
- **No partial updates.** No `enum ConfigPatch { SampleRate(u32), Channels(u16), … }`. Snapshot replacement is simpler and codecs already produce full state on the boundaries that matter (SPS, sync header).
- **No pre-flush requirement on the codec.** A codec can emit `Frame::ConfigChanged` immediately after a data frame; it does not have to drain pending frames first. Consumers process frames in order anyway.
- **No backwards-compat shim.** Codecs that don't surface config changes are unchanged. Consumers that don't track changes are unchanged. The variant only matters for the pairing of producer and consumer that opt in.
- **No async / out-of-band channel.** Config changes flow through the same `Frame` stream as everything else; no `tokio::watch` or sideband signalling. Keeps the model uniform with the rest of the API.

## Open questions to resolve when implementing

1. **Should `ConfigChanged` be allowed inside `Frame::Subtitle` streams?** Probably yes — subtitle codecs can change their style block mid-stream (PGS, ASS). Worth confirming when the first subtitle codec wants it.
2. **Encoder reconfigure error model.** `Encoder::send_frame(Frame::ConfigChanged(_))` for an encoder that doesn't support reconfigure: return `Error::Unsupported`, or silently no-op? Lean toward `Error::Unsupported` so the pipeline can decide what to do (close + reopen the encoder, or fail the transcode).
3. **Pipeline transcode policy.** When the input stream emits a config change and the output encoder rejects it, the pipeline has three options: (a) close + reopen the encoder, possibly losing GOP-internal frames; (b) fail loudly; (c) attempt to bridge with a resampler / scaler. Decide as a pipeline-level policy with a sensible default and user-overridable knob.
4. **Container muxer behaviour.** Most muxers can't represent mid-stream config changes inside a single track (MP4 needs a new sample description; Matroska can but rarely is). Likely policy: muxer rejects mid-stream change unless its container format supports it. Document per-muxer.

## Migration cost estimate

- Phase 1 (core): ~half a day. One variant + helper + local matchups.
- Phase 2 (one codec): ~a day per codec. Codec needs to understand which bitstream events change params, hook them, and add a regression test.
- Phase 3 (one consumer surface): ~a day per surface (player drivers, pipeline adapter, filter trait).

Total to ship a useful first slice (core + AC-3 + player): ~3 days of focused work.

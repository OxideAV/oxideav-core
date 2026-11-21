//! Gapless-playback trim record for audio streams.
//!
//! Transform audio codecs cannot code a signal sample-for-sample: the
//! encoder prepends **priming** samples (also called encoder delay or
//! pre-skip) so the first real sample decodes correctly, and pads the
//! last packet with **remainder** samples up to the codec's frame
//! size. Decoding every packet therefore yields
//!
//! ```text
//! | priming | ........ source audio ........ | remainder |
//! ```
//!
//! and a player that wants the original signal — or wants two tracks
//! to follow each other without a gap or click — must drop both ends.
//! The QuickTime file format's appendix on audio priming works the
//! example of 5389 source samples coded as eight 1024-sample packets:
//! 2112 priming samples, 5389 samples of audio, 691 remainder samples
//! (8192 − 2112 − 691 = 5389).
//!
//! [`GaplessInfo`] carries those counts as a **stream-level** record in
//! [`CodecParameters::gapless`](crate::CodecParameters::gapless). All
//! counts are in samples per channel at the stream's output
//! `sample_rate`, and describe the decoder's output **as delivered**: a
//! decoder that trims the priming itself reports `0`, not the codec's
//! nominal delay, so a downstream stage never trims twice.

use std::ops::Range;

/// Leading and trailing sample counts to discard from a decoded audio
/// stream so that exactly the source signal remains.
///
/// Every part is optional; `Default` leaves all of them unspecified.
/// A producer states what it knows: a container with an edit list or a
/// gapless tag fills `priming` and `padding` (or `valid_samples`), a
/// codec header with a pre-skip field fills `priming` alone.
///
/// `#[non_exhaustive]`: build it with [`new`](Self::new) and the
/// `with_*` builders; read the public fields directly.
///
/// ```
/// # use oxideav_core::gapless::GaplessInfo;
/// let g = GaplessInfo::new().with_priming(2112).with_padding(691);
/// // Eight 1024-sample packets decode to 8192 samples.
/// assert_eq!(g.valid_range(8192), 2112..7501);
/// assert_eq!(g.resolved_valid_samples(8192), 5389);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GaplessInfo {
    /// Samples to discard at the start of the decoded stream (encoder
    /// delay / priming / pre-skip). `None` is unspecified.
    pub priming: Option<u32>,
    /// Samples to discard at the end of the decoded stream (remainder
    /// / trailing padding). `None` is unspecified; see
    /// [`valid_samples`](Self::valid_samples) for the other way of
    /// stating the same fact.
    pub padding: Option<u32>,
    /// Length of the source signal in samples, after both trims. Some
    /// formats state this instead of the trailing padding; when both
    /// are present, `padding` is authoritative. `None` is unspecified.
    pub valid_samples: Option<u64>,
}

impl GaplessInfo {
    /// A record with every part unspecified — same as `Default`.
    pub const fn new() -> Self {
        Self {
            priming: None,
            padding: None,
            valid_samples: None,
        }
    }

    /// `true` when no part is specified.
    pub const fn is_unspecified(&self) -> bool {
        self.priming.is_none() && self.padding.is_none() && self.valid_samples.is_none()
    }

    /// Builder: set the leading sample count to discard.
    pub const fn with_priming(mut self, samples: u32) -> Self {
        self.priming = Some(samples);
        self
    }

    /// Builder: set the trailing sample count to discard.
    pub const fn with_padding(mut self, samples: u32) -> Self {
        self.padding = Some(samples);
        self
    }

    /// Builder: set the length of the source signal in samples.
    pub const fn with_valid_samples(mut self, samples: u64) -> Self {
        self.valid_samples = Some(samples);
        self
    }

    /// Field-wise merge: each part of `self` that is specified wins,
    /// each part that is not is taken from `fallback`.
    pub fn or(self, fallback: Self) -> Self {
        Self {
            priming: self.priming.or(fallback.priming),
            padding: self.padding.or(fallback.padding),
            valid_samples: self.valid_samples.or(fallback.valid_samples),
        }
    }

    /// The range of decoded sample indices that belong to the source
    /// signal, given that decoding the whole stream produced
    /// `decoded_samples` samples.
    ///
    /// The start is `priming` (zero when unspecified). The end is
    /// `decoded_samples - padding` when the padding is stated, else
    /// `priming + valid_samples` when the length is stated, else
    /// `decoded_samples`. Both ends are clamped to `0..=decoded_samples`
    /// and the range is never inverted, so inconsistent counts yield a
    /// shorter (possibly empty) range rather than a panic.
    pub fn valid_range(&self, decoded_samples: u64) -> Range<u64> {
        let start = u64::from(self.priming.unwrap_or(0)).min(decoded_samples);
        let end = match (self.padding, self.valid_samples) {
            (Some(pad), _) => decoded_samples.saturating_sub(u64::from(pad)),
            (None, Some(valid)) => start.saturating_add(valid).min(decoded_samples),
            (None, None) => decoded_samples,
        };
        start..end.max(start)
    }

    /// Number of source-signal samples among `decoded_samples` decoded
    /// ones: the length of [`valid_range`](Self::valid_range).
    pub fn resolved_valid_samples(&self, decoded_samples: u64) -> u64 {
        let r = self.valid_range(decoded_samples);
        r.end - r.start
    }

    /// Trailing samples to discard among `decoded_samples` decoded
    /// ones: `decoded_samples` minus the end of
    /// [`valid_range`](Self::valid_range).
    pub fn resolved_padding(&self, decoded_samples: u64) -> u64 {
        decoded_samples - self.valid_range(decoded_samples).end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_unspecified_and_trims_nothing() {
        let g = GaplessInfo::default();
        assert_eq!(g, GaplessInfo::new());
        assert!(g.is_unspecified());
        assert_eq!(g.valid_range(1000), 0..1000);
        assert_eq!(g.resolved_valid_samples(1000), 1000);
        assert_eq!(g.resolved_padding(1000), 0);
        assert_eq!(g.valid_range(0), 0..0);
    }

    #[test]
    fn builders_set_each_part() {
        let g = GaplessInfo::new()
            .with_priming(312)
            .with_padding(40)
            .with_valid_samples(9000);
        assert_eq!(g.priming, Some(312));
        assert_eq!(g.padding, Some(40));
        assert_eq!(g.valid_samples, Some(9000));
        for part in [
            GaplessInfo::new().with_priming(0),
            GaplessInfo::new().with_padding(0),
            GaplessInfo::new().with_valid_samples(0),
        ] {
            assert!(!part.is_unspecified());
        }
    }

    #[test]
    fn worked_example_of_the_priming_appendix() {
        // 8 packets × 1024 = 8192 decoded; 2112 priming, 691 remainder.
        let by_padding = GaplessInfo::new().with_priming(2112).with_padding(691);
        assert_eq!(by_padding.valid_range(8192), 2112..7501);
        assert_eq!(by_padding.resolved_valid_samples(8192), 5389);
        assert_eq!(by_padding.resolved_padding(8192), 691);
        // The same fact stated through the source length.
        let by_length = GaplessInfo::new()
            .with_priming(2112)
            .with_valid_samples(5389);
        assert_eq!(by_length.valid_range(8192), 2112..7501);
        assert_eq!(by_length.resolved_padding(8192), 691);
    }

    #[test]
    fn padding_wins_over_valid_samples() {
        let g = GaplessInfo::new()
            .with_priming(10)
            .with_padding(20)
            .with_valid_samples(5);
        assert_eq!(g.valid_range(100), 10..80);
    }

    #[test]
    fn inconsistent_counts_clamp_instead_of_panicking() {
        let g = GaplessInfo::new().with_priming(500).with_padding(700);
        assert_eq!(g.valid_range(1000), 500..500);
        assert_eq!(g.resolved_valid_samples(1000), 0);
        let g = GaplessInfo::new().with_priming(5000);
        assert_eq!(g.valid_range(1000), 1000..1000);
        let g = GaplessInfo::new()
            .with_priming(10)
            .with_valid_samples(u64::MAX);
        assert_eq!(g.valid_range(1000), 10..1000);
        let g = GaplessInfo::new().with_padding(u32::MAX);
        assert_eq!(g.valid_range(7), 0..0);
        assert_eq!(g.resolved_padding(7), 7);
    }

    #[test]
    fn or_merges_field_wise() {
        let decoder = GaplessInfo::new().with_priming(0);
        let container = GaplessInfo::new().with_priming(2112).with_padding(691);
        let merged = decoder.or(container);
        assert_eq!(merged.priming, Some(0));
        assert_eq!(merged.padding, Some(691));
        assert_eq!(merged.valid_samples, None);
        assert_eq!(GaplessInfo::new().or(container), container);
        assert_eq!(container.or(GaplessInfo::new()), container);
    }
}

//! Shared audio copy-eligibility rule (P0.3).
//!
//! ONE definition of "this audio stream can be copied verbatim": the source
//! already satisfies the plan/output contract (codec, sample rate,
//! channels). Every path that moves audio — `mux_audio_copy` (final-audio certification),
//! future stock audio — routes through `is_compatible`, so a contract
//! change lands in one place instead of drifting per caller.
//!
//! The rule is pure (no ffmpeg): callers probe once and decide. Copying
//! incompatible audio into MP4 (e.g. PCM or 44.1kHz kept as-is) would break
//! downstream assembly, so ineligible streams always take exactly one
//! certified transcode — never a silent copy.

use crate::protocol::MediaMetadata;

/// Returns true when the probed source stream already satisfies the output
/// contract and may be copied verbatim (`-c:a copy`).
pub fn is_compatible(source: &MediaMetadata, codec: &str, sample_rate: i64, channels: i64) -> bool {
    source.has_audio
        && source
            .audio_codec
            .as_deref()
            .map(|actual| actual.eq_ignore_ascii_case(codec))
            .unwrap_or(false)
        && source.sample_rate == u32::try_from(sample_rate).ok()
        && source.channels == u32::try_from(channels).ok()
}

/// Canonical contract for everything the pipeline publishes: AAC-LC,
// 48kHz stereo. `mux_audio_copy` certifies the final master against this.
/// Declared in `config` (a leaf module) so the protocol and the compiled plan
/// can reference it without importing an executor; re-exported here because
/// this module owns the rule that consumes it.
pub use crate::config::{CANONICAL_CHANNELS, CANONICAL_CODEC, CANONICAL_SAMPLE_RATE};

/// Returns true when the probed stream is publishable as-is.
pub fn is_canonically_compatible(source: &MediaMetadata) -> bool {
    is_compatible(
        source,
        CANONICAL_CODEC,
        CANONICAL_SAMPLE_RATE,
        CANONICAL_CHANNELS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(
        codec: Option<&str>,
        sample_rate: Option<u32>,
        channels: Option<u32>,
    ) -> MediaMetadata {
        MediaMetadata {
            duration_sec: 10.0,
            width: 1920,
            height: 1080,
            fps: 24.0,
            stream_count: 2,
            video_stream_count: 1,
            audio_stream_count: 1,
            fps_num: 24,
            fps_den: 1,
            audio_codec: codec.map(str::to_string),
            sample_rate,
            channels,
            start_pts: None,
            has_video: true,
            has_audio: true,
            ..Default::default()
        }
    }

    #[test]
    fn compatible_stream_is_copyable() {
        let source = metadata(Some("aac"), Some(48000), Some(2));
        assert!(is_compatible(&source, "aac", 48000, 2));
        assert!(is_canonically_compatible(&source));
    }

    #[test]
    fn codec_case_does_not_block_copy() {
        let source = metadata(Some("AAC"), Some(48000), Some(2));
        assert!(is_compatible(&source, "aac", 48000, 2));
    }

    #[test]
    fn mismatched_contract_is_not_copyable() {
        for source in [
            metadata(Some("mp3"), Some(48000), Some(2)),
            metadata(Some("aac"), Some(44100), Some(2)),
            metadata(Some("aac"), Some(48000), Some(1)),
            metadata(None, Some(48000), Some(2)),
            metadata(Some("aac"), None, Some(2)),
        ] {
            assert!(!is_canonically_compatible(&source));
        }
        let mut silent = metadata(Some("aac"), Some(48000), Some(2));
        silent.has_audio = false;
        assert!(!is_canonically_compatible(&silent));
    }
}

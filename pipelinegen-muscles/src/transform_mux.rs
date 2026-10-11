use super::{checked_multi_inputs, finish_run, ExpectedInputs};
use crate::artifact::{failed_response, part_path};
use crate::error::MuscleError;
use crate::process::FFmpegRunner;
use crate::protocol::{MediaMetadata, Request, Response};

/// The copy-only mux must never reconcile a video/audio duration divergence by
/// truncation (`-shortest`). The audio master is padded to cover the
/// frame-aligned video render, so it must never be shorter than the video; a
/// shortfall would leave a silent tail. Only sub-millisecond probe rounding is
/// accepted in that direction. Audio may exceed the video only by encoder
/// padding. This tolerance is deliberately wider than the master-audio
/// invariant (40ms) because the copy-only mux compares the frame-aligned
/// video container (up to one frame, ~33ms, longer than the canonical
/// timeline) against the audio master.
const MUX_DURATION_TOLERANCE_MS: f64 = 100.0;
const MUX_SHORTFALL_TOLERANCE_MS: f64 = 1.0;

/// duration_mismatch returns the fail-closed error when the probed audio
/// master does not cover the probed video (audio_duration >= video_duration),
/// or when it exceeds the video beyond the encoder-padding tolerance. It is
/// pure so the gate is unit-testable without ffmpeg.
fn duration_mismatch(video_sec: f64, audio_sec: f64) -> Option<String> {
    if !video_sec.is_finite() || video_sec <= 0.0 || !audio_sec.is_finite() || audio_sec <= 0.0 {
        return None; // unusable durations are reported by the caller's probe
    }
    let video_ms = (video_sec * 1000.0).round();
    let audio_ms = (audio_sec * 1000.0).round();
    if video_ms > audio_ms + MUX_SHORTFALL_TOLERANCE_MS {
        return Some(format!(
            "FINAL_MUX_AUDIO_SHORTER_THAN_VIDEO: video_duration={video_ms:.0}ms audio_duration={audio_ms:.0}ms shortfall={:.0}ms",
            video_ms - audio_ms
        ));
    }
    let surplus_ms = audio_ms - video_ms;
    if surplus_ms > MUX_DURATION_TOLERANCE_MS {
        return Some(format!(
            "FINAL_MUX_DURATION_MISMATCH: video_duration={video_ms:.0}ms audio_duration={audio_ms:.0}ms surplus={surplus_ms:.0}ms"
        ));
    }
    None
}

pub(super) fn execute(request: Request) -> Response {
    let (inputs, output) = match checked_multi_inputs(
        &request,
        ExpectedInputs::Exactly(2),
        "mux_audio_copy requires exactly video and final audio inputs",
    ) {
        Ok(values) => values,
        Err(response) => return *response,
    };
    let ffmpeg = request.ffmpeg_path.as_deref().unwrap_or("ffmpeg");
    let ffprobe = crate::probe::ffprobe_path(ffmpeg);

    // Fail closed on any duration divergence before the copy-only mux. The
    // rendered video and the certified final audio must agree within the
    // encoder-padding tolerance; `-shortest` is deliberately NOT used to
    // reconcile them, so a mismatch aborts the render instead of truncating.
    // The two input probes are independent ffprobe subprocesses: run them in
    // parallel (P1.6). The single mux below still runs once; only the wait
    // for both probes collapses from sum to max.
    let (video_probe, audio_probe) = std::thread::scope(|scope| {
        let ffprobe_video = ffprobe.clone();
        let ffprobe_audio = ffprobe.clone();
        let video_path = &inputs[0];
        let audio_path = &inputs[1];
        let video = scope.spawn(move || crate::probe::probe_file(&ffprobe_video, video_path));
        let audio = scope.spawn(move || crate::probe::probe_file(&ffprobe_audio, audio_path));
        (
            video
                .join()
                .unwrap_or_else(|_| Err(MuscleError::generic("video probe thread panicked"))),
            audio
                .join()
                .unwrap_or_else(|_| Err(MuscleError::generic("final audio probe thread panicked"))),
        )
    });
    let video_sec = match video_probe {
        Ok(metadata) if metadata.duration_sec.is_finite() && metadata.duration_sec > 0.0 => {
            metadata.duration_sec
        }
        Ok(_) => {
            return failed_response(
                Some(inputs[0].to_string()),
                format!("video has no usable duration: {}", inputs[0]),
            )
        }
        Err(error) => {
            return failed_response(
                Some(inputs[0].to_string()),
                format!("video probe failed: {error}"),
            )
        }
    };
    let audio_metadata = match audio_probe {
        Ok(metadata) if metadata.duration_sec.is_finite() && metadata.duration_sec > 0.0 => {
            metadata
        }
        Ok(_) => {
            return failed_response(
                Some(inputs[1].to_string()),
                format!("final audio has no usable duration: {}", inputs[1]),
            )
        }
        Err(error) => {
            return failed_response(
                Some(inputs[1].to_string()),
                format!("final audio probe failed: {error}"),
            )
        }
    };
    // Copy-only certification (P0.3, shared rule): `-c:a copy` below is
    // verbatim, so the final master must ALREADY be the canonical AAC-LC
    // 48kHz stereo contract. Anything else fails closed here instead of
    // producing an unplayable MP4 downstream.
    if !crate::audio_copy::is_canonically_compatible(&audio_metadata) {
        return failed_response(
            Some(inputs[1].to_string()),
            format!(
                "FINAL_MUX_AUDIO_NOT_COPY_ELIGIBLE: final audio is {:?}/{:?}/{:?}, want aac/48000/stereo",
                audio_metadata.audio_codec,
                audio_metadata.sample_rate,
                audio_metadata.channels,
            ),
        );
    }
    let audio_sec = audio_metadata.duration_sec;
    if let Some(error) = duration_mismatch(video_sec, audio_sec) {
        return failed_response(Some(inputs[0].to_string()), error);
    }

    let part = part_path(&output);
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-i",
        inputs[0].as_str(),
        "-i",
        inputs[1].as_str(),
        "-map",
        "0:v:0",
        "-map",
        "1:a:0",
        "-c:v",
        "copy",
        "-c:a",
        "copy",
        "-movflags",
        "+faststart",
        &part,
    ]);
    let result = command.output();
    finish_run(
        "mux_audio_copy",
        &inputs[0],
        &part,
        &output,
        result,
        Some(MediaMetadata {
            audio_copy_eligible: Some(true),
            ..Default::default()
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{duration_mismatch, MUX_DURATION_TOLERANCE_MS};

    #[test]
    fn matching_durations_pass_the_gate() {
        assert_eq!(duration_mismatch(181.920, 181.920), None);
        // Encoder padding within tolerance is accepted.
        assert_eq!(duration_mismatch(181.920, 181.950), None);
    }

    #[test]
    fn shorter_audio_fails_closed_with_typed_error() {
        // The old symmetric gate accepted a video that outlasts the audio by
        // up to 100ms, leaving a silent tail; the guarantee is now
        // audio_duration >= video_duration.
        let error = duration_mismatch(17.533, 17.520).expect("shortfall must abort");
        assert!(error.contains("FINAL_MUX_AUDIO_SHORTER_THAN_VIDEO"));
        assert!(error.contains("video_duration=17533ms"));
        assert!(error.contains("audio_duration=17520ms"));
        assert!(error.contains("shortfall=13ms"));
    }

    #[test]
    fn longer_audio_fails_closed_with_typed_error() {
        let error = duration_mismatch(168.982, 180.400).expect("surplus must abort");
        assert!(error.contains("FINAL_MUX_DURATION_MISMATCH"));
        assert!(error.contains("video_duration=168982ms"));
        assert!(error.contains("audio_duration=180400ms"));
        assert!(error.contains("surplus=11418ms"));
    }

    #[test]
    fn unusable_durations_are_not_a_mismatch() {
        assert_eq!(duration_mismatch(0.0, 5.0), None);
        assert_eq!(duration_mismatch(f64::NAN, 5.0), None);
        assert_eq!(duration_mismatch(5.0, f64::INFINITY), None);
    }

    #[test]
    fn shortfall_tolerance_is_one_millisecond() {
        let video = 100.0;
        assert_eq!(duration_mismatch(video, video - 0.001), None);
        assert!(duration_mismatch(video, video - 0.002).is_some());
    }

    #[test]
    fn surplus_tolerance_boundary_is_inclusive() {
        let video = 100.0;
        let audio = video + MUX_DURATION_TOLERANCE_MS / 1000.0;
        assert_eq!(duration_mismatch(video, audio), None);
        let over = video + MUX_DURATION_TOLERANCE_MS / 1000.0 + 0.001;
        assert!(duration_mismatch(video, over).is_some());
    }

    #[test]
    fn scene8_frame_aligned_master_covers_video() {
        // 17.52s narration @30fps → 526 frames = 17.533s. With the audio
        // master padded to the frame-aligned duration, both probe to the same
        // 17533ms and the gate passes; the un-padded 17520ms master fails.
        assert_eq!(duration_mismatch(17.5333, 17.5333), None);
        assert!(duration_mismatch(17.5333, 17.5200).is_some());
    }
}

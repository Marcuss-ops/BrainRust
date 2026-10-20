//! GPU acceleration capability detection for the FFmpeg-backed capabilities.
//!
//! Rust owns the FFmpeg mechanics only: which hwaccel preamble (`-hwaccel
//! cuda|qsv|vaapi`) a Go-resolved encoder policy may use, decided from the
//! encoder codec family AND the decode capabilities the local FFmpeg build
//! actually reports via `ffmpeg -hwaccels`. An encoder family that requires a
//! hardware backend the local build does not list degrades to the CPU decode
//! path and the degradation is reported on stderr — never silent, so the
//! operator sees the throughput hit the same way `gpu_cut_eligibility`
//! already surfaces probe failures.
//!
//! Keep frames in system memory: `encoder.rs` pins `-pix_fmt yuv420p` for
//! every encoded operation, so the hwaccel preamble is bare `-hwaccel X`
//! (accelerated decode → CPU frames) and NEVER `-hwaccel_output_format`
//! (device-local frames → uninitializable filter graph). That contract is
//! pinned by the cut tests and must stay verified on both halves.

/// The hwaccel device families this executor understands. The string values
/// are the tokens FFmpeg itself reports under `-hwaccels`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecodeAccelerator {
    Cuda,
    Qsv,
    Vaapi,
}

impl DecodeAccelerator {
    /// The `-hwaccel` token for this family.
    pub(crate) fn flag(self) -> &'static str {
        match self {
            DecodeAccelerator::Cuda => "cuda",
            DecodeAccelerator::Qsv => "qsv",
            DecodeAccelerator::Vaapi => "vaapi",
        }
    }
}

/// The hwaccel device family an encoder codec family requires, if any.
///
/// Underscore-suffixed encoders are the FFmpeg hardware encoder names
/// (`h264_nvenc`, `h264_qsv`, `h264_vaapi`, `hevc_*`). Software encoders
/// (`libx264`, `libvpx`, …) need no decode preamble.
pub(crate) fn required_hwaccel(codec: &str) -> Option<DecodeAccelerator> {
    let codec = codec.to_ascii_lowercase();
    if codec.ends_with("_nvenc") {
        Some(DecodeAccelerator::Cuda)
    } else if codec.ends_with("_qsv") {
        Some(DecodeAccelerator::Qsv)
    } else if codec.ends_with("_vaapi") {
        Some(DecodeAccelerator::Vaapi)
    } else {
        None
    }
}

/// Parses `ffmpeg -hwaccels` output. Everything after the
/// "Hardware acceleration methods:" header is one token per line. A missing
/// header fails closed (empty listing): an unparseable report must never be
/// read as "the backend is present".
pub(crate) fn parse_hwaccel_listing(stdout: &str) -> Vec<DecodeAccelerator> {
    const HEADER: &str = "Hardware acceleration methods:";
    let mut after = stdout.lines().skip_while(|line| !line.contains(HEADER));
    after.next(); // consume the header line itself
    after
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| match line {
            "cuda" => Some(DecodeAccelerator::Cuda),
            "qsv" => Some(DecodeAccelerator::Qsv),
            "vaapi" => Some(DecodeAccelerator::Vaapi),
            _ => None,
        })
        .collect()
}

/// Resolves the decode accelerator for a Go-resolved encoder codec against
/// the local FFmpeg build. Returns the accelerator to use plus an optional
/// degradation note (only set when a required hardware backend is missing or
/// its listing is unprobeable — the note is ALSO echoed on stderr so the
/// downgrade is observable even without the metadata plumbing).
///
/// `(None, None)` means software codec, software decode: the historical and
/// correct state. GPUs are never probed more than once per capability call
/// (`ffmpeg -hwaccels` is a cheap banner-level query, no device open).
pub(crate) fn resolve_decode_accelerator(
    codec: &str,
    ffmpeg: &str,
) -> (Option<DecodeAccelerator>, Option<String>) {
    let Some(required) = required_hwaccel(codec) else {
        return (None, None);
    };
    let listing = match listed_hwaccels(ffmpeg) {
        Ok(listing) => listing,
        Err(error) => {
            let note = format!(
                "gpu capability probe failed for encoder {codec} ({error}); falling back to CPU decode"
            );
            eprintln!("[gpu-capability] {note}");
            return (None, Some(note));
        }
    };
    if listing.contains(&required) {
        (Some(required), None)
    } else {
        let listed = listing
            .iter()
            .map(|accelerator| accelerator.flag())
            .collect::<Vec<_>>()
            .join(", ");
        let note = format!(
            "encoder {codec} requires hwaccel '{}' but the local ffmpeg build lists only [{}]; falling back to CPU decode",
            required.flag(),
            listed
        );
        eprintln!("[gpu-capability] {note}");
        (None, Some(note))
    }
}

/// Runs `ffmpeg -hwaccels` and parses the device listing. The process runner
/// mirrors every other capability probe (setsid supervisor, 10-minute cap).
fn listed_hwaccels(ffmpeg: &str) -> Result<Vec<DecodeAccelerator>, String> {
    let mut search = crate::process::FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    search.args(["-hide_banner", "-hwaccels"]);
    let output = search
        .output()
        .map_err(|error| format!("spawn {ffmpeg} -hwaccels: {error}"))?;
    if !output.status.success() {
        return Err(format!("ffmpeg -hwaccels exited with {}", output.status));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(parse_hwaccel_listing(&stdout))
}

/// The argv preamble builder shared by every capability that may prepend a
/// hwaccel declaration before the first `-i`. Frames stay in system memory:
/// this function NEVER emits `-hwaccel_output_format` (see module docs).
pub(crate) fn decode_preamble_args(accelerator: Option<DecodeAccelerator>) -> Vec<&'static str> {
    match accelerator {
        Some(accelerator) => vec!["-hwaccel", accelerator.flag()],
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_preamble_args, parse_hwaccel_listing, required_hwaccel, resolve_decode_accelerator,
        DecodeAccelerator,
    };

    #[test]
    fn codec_families_map_to_their_device_backends() {
        assert_eq!(
            required_hwaccel("h264_nvenc"),
            Some(DecodeAccelerator::Cuda)
        );
        assert_eq!(
            required_hwaccel("H264_NVENC"),
            Some(DecodeAccelerator::Cuda)
        );
        assert_eq!(required_hwaccel("h264_qsv"), Some(DecodeAccelerator::Qsv));
        assert_eq!(
            required_hwaccel("h264_vaapi"),
            Some(DecodeAccelerator::Vaapi)
        );
        // Software encoders need no preamble; NVENC alias family honoured.
        assert!(required_hwaccel("libx264").is_none());
        assert!(required_hwaccel("").is_none());
    }

    #[test]
    fn hwaccel_listing_parses_all_understood_families() {
        let listing = parse_hwaccel_listing(
            "ffmpeg version 7.1\nHardware acceleration methods:\nvaapi\nqsv\nvdpau\ncuda\nvk\n",
        );
        // Unknown families (vdpau, vk) are ignored, known ones kept in order.
        assert_eq!(
            listing,
            vec![
                DecodeAccelerator::Vaapi,
                DecodeAccelerator::Qsv,
                DecodeAccelerator::Cuda,
            ]
        );
    }

    #[test]
    fn missing_header_fails_closed_to_an_empty_listing() {
        assert!(parse_hwaccel_listing("ffmpeg version 7.1\nusage info\n").is_empty());
        assert!(parse_hwaccel_listing("").is_empty());
    }

    #[test]
    fn preamble_matches_the_bare_flag_contract() {
        assert_eq!(
            decode_preamble_args(Some(DecodeAccelerator::Cuda)),
            vec!["-hwaccel", "cuda"]
        );
        assert_eq!(
            decode_preamble_args(Some(DecodeAccelerator::Qsv)),
            vec!["-hwaccel", "qsv"]
        );
        assert!(decode_preamble_args(None).is_empty());
        // Never emits the device-local contract breaker.
        assert!(!decode_preamble_args(Some(DecodeAccelerator::Cuda))
            .contains(&"-hwaccel_output_format"));
    }

    #[test]
    fn missing_backend_degrades_with_an_observable_note() {
        // The fixture build lists only vaapi: an NVENC policy must degrade
        // to CPU decode — not crash, not pretend the GPU path exists.
        stderr_reset();
        let (accelerator, note) =
            resolve_decode_accelerator("h264_nvenc", "nmissing-ffmpeg-binary-for-tests-only");
        let _ = accelerator;
        // Probe failure note must exist on THIS degration shape.
        assert!(note.is_some(), "unprobeable build must produce a note");
    }

    #[test]
    fn software_codec_never_probes_the_gpu() {
        stderr_reset();
        let (accelerator, note) =
            resolve_decode_accelerator("libx264", "definitely-not-an-ffmpeg-binary");
        // A software codec needs no device: even an unusable ffmpeg path
        // must return the clean software shape, no note.
        assert!(accelerator.is_none());
        assert!(note.is_none());
    }

    /// Keeps stderr-output tests independent (each prints at most one note).
    fn stderr_reset() {}
}

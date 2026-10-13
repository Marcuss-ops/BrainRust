use super::*;

pub(super) const BGM_SOURCE_LOUDNESS_TARGET_DB: f64 = -23.0;
pub(super) const BGM_LOUDNESS_TRIM_MAX_DB: f64 = 24.0;

// bgm_loudness_trim_db maps a measured source mean loudness to the static
// trim that lands it at the canonical target. Non-finite measurements
// (digital silence reports -inf) fail open with None.
pub(super) fn bgm_loudness_trim_db(mean_db: f64) -> Option<f64> {
    if !mean_db.is_finite() {
        return None;
    }
    Some(
        (BGM_SOURCE_LOUDNESS_TARGET_DB - mean_db)
            .clamp(-BGM_LOUDNESS_TRIM_MAX_DB, BGM_LOUDNESS_TRIM_MAX_DB),
    )
}

// loudness_trim_filter formats a trim as the volume-filter fragment that
// event_filter's normalize prefix can carry ("volume=-19.990000dB,").
pub(super) fn loudness_trim_filter(trim_db: f64) -> String {
    format!("volume={}dB,", num(trim_db))
}

// measure_mean_volume_db runs one volumedetect pass over the source and
// returns its mean_volume (the whole-file RMS loudness volumedetect
// reports on stderr).
pub(super) fn measure_mean_volume_db(ffmpeg: &str, path: &str) -> Result<f64, MuscleError> {
    let mut command = FFmpegRunner::from_ffmpeg_path(ffmpeg).ffmpeg();
    command.args([
        "-hide_banner",
        "-nostats",
        "-i",
        path,
        "-af",
        "volumedetect",
        "-f",
        "null",
        "-",
    ]);
    let output = command
        .output()
        .map_err(|error| MuscleError::generic(format!("volumedetect failed to start: {error}")))?;
    if !output.status.success() {
        return Err(MuscleError::generic(format!(
            "volumedetect exited with {}",
            output.status
        )));
    }
    parse_volumedetect_mean(&output.stderr).ok_or_else(|| {
        MuscleError::generic(format!("volumedetect produced no mean_volume for {path}"))
    })
}

// parse_volumedetect_mean extracts "mean_volume: <value> dB" from ffmpeg's
// volumedetect stderr. The -inf sentinel (digital silence) parses to None
// so the caller fails open instead of computing an absurd boost.
pub(super) fn parse_volumedetect_mean(output: &[u8]) -> Option<f64> {
    let text = String::from_utf8_lossy(output);
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        for (i, token) in fields.iter().enumerate() {
            if *token != "mean_volume:" {
                continue;
            }
            let value = fields.get(i + 1)?;
            if fields.get(i + 2).copied() != Some("dB") {
                return None;
            }
            if *value == "-inf" {
                return None;
            }
            return value.parse::<f64>().ok();
        }
    }
    None
}

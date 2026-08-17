use serde::Deserialize;
use std::path::Path;
use std::process::Command;

pub(super) const CLIP_PLAN_VERSION: &str = "clip-render-plan.v1";

pub(super) const BACKGROUND_NONE: &str = "none";
pub(super) const BACKGROUND_BLUR_SOURCE: &str = "blur_source";
pub(super) const BACKGROUND_ASSET: &str = "asset";

pub(super) const SUBTITLE_BURN: &str = "burn";
pub(super) const SUBTITLE_SIDECAR: &str = "sidecar";

pub(super) const AUDIO_COPY_IF_COMPATIBLE: &str = "copy_if_compatible";
pub(super) const AUDIO_TRANSCODE: &str = "transcode";

// ClipRenderPlan mirrors the Go ClipRenderPlanV1 JSON contract EXACTLY
// (field order + optional-field omission) because the Go boundary seals the
// plan with plan_sha256 over the canonical JSON. Rust never recomputes that
// digest (the drift gate lives Go-side, mirroring render_stock), but it must
// decode the same shape and validate every referenced artifact fail-closed.
#[derive(Debug, Deserialize)]
pub(super) struct ClipRenderPlan {
    pub version: String,
    pub run_id: String,
    pub source: ClipPlanSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<ClipPlanBackground>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<ClipPlanWatermark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<ClipPlanSubtitles>,
    pub output: ClipPlanOutput,
    pub audio: ClipPlanAudio,
    pub output_path: String,
    pub plan_sha256: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClipPlanSource {
    pub asset_id: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClipPlanBackground {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClipPlanWatermark {
    pub asset_id: String,
    pub path: String,
    pub sha256: String,
    pub position: String,
    pub opacity: f64,
    pub margin_px: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClipPlanSubtitles {
    pub mode: String,
    // Wire-contract field: decoded to enforce the sealed JSON shape, not
    // consumed by the render mechanics (the compiled ASS carries its own
    // styles; Rust never restyles subtitles).
    #[allow(dead_code)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_id: Option<String>,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClipPlanOutput {
    pub contract_id: String,
    pub container: String,
    pub video_codec: String,
    // Wire-contract field: decoded to enforce the sealed JSON shape, not
    // consumed by the render mechanics (the Go contract gate owns profile
    // verification after the render).
    #[allow(dead_code)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_profile: Option<String>,
    pub pixel_format: String,
    pub width: i64,
    pub height: i64,
    pub fps: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClipPlanAudio {
    pub mode: String,
    pub codec: String,
    pub sample_rate: i64,
    pub channels: i64,
}

/// decode_and_validate parses the sealed ClipRenderPlanV1 and enforces the
/// fail-closed contract: version, lowercase SHA256 digests, enum values, and
/// physical identity of every referenced artifact (source, background asset,
/// watermark, subtitles). A drifted or unreadable artifact rejects the plan
/// before any FFmpeg process starts — Rust never renders bytes it has not
/// verified.
pub(super) fn decode_and_validate(raw_plan: serde_json::Value) -> Result<ClipRenderPlan, String> {
    let plan: ClipRenderPlan = serde_json::from_value(raw_plan)
        .map_err(|error| format!("invalid clip_plan: {error}"))?;
    validate(&plan)?;
    Ok(plan)
}

fn validate(plan: &ClipRenderPlan) -> Result<(), String> {
    if plan.version != CLIP_PLAN_VERSION || plan.run_id.trim().is_empty() {
        return Err("invalid clip_plan version or run_id".to_string());
    }
    if !is_lower_sha256(&plan.plan_sha256) {
        return Err("clip_plan plan_sha256 must be a lowercase SHA256 value".to_string());
    }
    if plan.output_path.trim().is_empty() {
        return Err("clip_plan output_path is required".to_string());
    }
    validate_source(&plan.source)?;

    if let Some(background) = &plan.background {
        match background.mode.as_str() {
            BACKGROUND_NONE | BACKGROUND_BLUR_SOURCE => {}
            BACKGROUND_ASSET => {
                let path = background
                    .path
                    .as_deref()
                    .ok_or_else(|| "clip_plan background mode=asset requires path".to_string())?;
                let sha256 = background.sha256.as_deref().ok_or_else(|| {
                    "clip_plan background mode=asset requires sha256".to_string()
                })?;
                validate_artifact("background", path, sha256)?;
                if background
                    .asset_id
                    .as_deref()
                    .map_or(true, |value| value.trim().is_empty())
                {
                    return Err("clip_plan background mode=asset requires asset_id".to_string());
                }
            }
            _ => {
                return Err(format!(
                    "clip_plan invalid background mode {:?}",
                    background.mode
                ))
            }
        }
    }

    if let Some(watermark) = &plan.watermark {
        validate_artifact("watermark", &watermark.path, &watermark.sha256)?;
        if watermark.asset_id.trim().is_empty() {
            return Err("clip_plan watermark requires asset_id".to_string());
        }
        if !matches!(
            watermark.position.as_str(),
            "top_left" | "top_right" | "bottom_left" | "bottom_right"
        ) {
            return Err(format!(
                "clip_plan invalid watermark position {:?}",
                watermark.position
            ));
        }
        if !(0.0..=1.0).contains(&watermark.opacity) {
            return Err("clip_plan watermark opacity must be within [0,1]".to_string());
        }
        if watermark.margin_px < 0 {
            return Err("clip_plan watermark margin_px must be >= 0".to_string());
        }
    }

    if let Some(subtitles) = &plan.subtitles {
        if !matches!(subtitles.mode.as_str(), SUBTITLE_BURN | SUBTITLE_SIDECAR) {
            return Err(format!(
                "clip_plan invalid subtitle mode {:?}",
                subtitles.mode
            ));
        }
        validate_artifact("subtitles", &subtitles.path, &subtitles.sha256)?;
    }

    validate_output(&plan.output)?;
    validate_audio(&plan.audio)?;
    Ok(())
}

fn validate_source(source: &ClipPlanSource) -> Result<(), String> {
    if source.asset_id.trim().is_empty() {
        return Err("clip_plan source requires asset_id".to_string());
    }
    validate_artifact("source", &source.path, &source.sha256)
}

fn validate_output(output: &ClipPlanOutput) -> Result<(), String> {
    if output.contract_id.trim().is_empty()
        || output.container.trim().is_empty()
        || output.video_codec.trim().is_empty()
        || output.pixel_format.trim().is_empty()
        || output.width <= 0
        || output.height <= 0
        || output.fps <= 0
    {
        return Err("clip_plan output contract is incomplete".to_string());
    }
    Ok(())
}

fn validate_audio(audio: &ClipPlanAudio) -> Result<(), String> {
    if !matches!(
        audio.mode.as_str(),
        AUDIO_COPY_IF_COMPATIBLE | AUDIO_TRANSCODE
    ) {
        return Err(format!("clip_plan invalid audio mode {:?}", audio.mode));
    }
    if audio.codec.trim().is_empty() || audio.sample_rate <= 0 || audio.channels <= 0 {
        return Err("clip_plan audio contract is incomplete".to_string());
    }
    Ok(())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// validate_artifact verifies the referenced file exists and its content
/// SHA256 (computed via sha256sum) matches the plan's digest. Fail-closed:
/// a missing file or a digest mismatch rejects the plan.
fn validate_artifact(label: &str, path: &str, expected: &str) -> Result<(), String> {
    if !is_lower_sha256(expected) {
        return Err(format!("clip_plan {label} sha256 must be a lowercase SHA256 value"));
    }
    if !Path::new(path).is_file() {
        return Err(format!("clip_plan {label} path is not readable: {path}"));
    }
    let actual = sha256_file(path)?;
    if actual != expected {
        return Err(format!("clip_plan {label} hash mismatch for {path}"));
    }
    Ok(())
}

fn sha256_file(path: &str) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| format!("compute SHA256 for {path}: {error}"))?;
    if !output.status.success() {
        return Err(format!("compute SHA256 for {path} failed"));
    }
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if !is_lower_sha256(&digest) {
        return Err(format!("sha256sum returned an invalid digest for {path}"));
    }
    Ok(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const VALID_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    // fixture_dir returns a per-test temp dir so parallel tests never share
    // artifact files (each test's sha256s would collide otherwise).
    fn fixture_dir(tag: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cliprender-plan-{}-{tag}",
            std::process::id()
        ))
        .join(id.to_string());
        fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    fn plan_json(t: &std::path::Path, patch: &str) -> serde_json::Value {
        let source_path = t.join("source.mp4");
        fs::write(&source_path, b"source-bytes").expect("write source");
        let source_sha = sha256_file(source_path.to_str().unwrap()).unwrap();
        serde_json::from_str(&format!(
            r#"{{
                "version": "clip-render-plan.v1",
                "run_id": "job-1",
                "source": {{"asset_id": "asset-src", "path": "{}", "sha256": "{}"}},
                "background": {{"mode": "blur_source"}},
                "output": {{"contract_id": "velox-editing-clip-v1", "container": "mp4", "video_codec": "h264", "video_profile": "high", "pixel_format": "yuv420p", "width": 1080, "height": 1920, "fps": 60}},
                "audio": {{"mode": "copy_if_compatible", "codec": "aac", "sample_rate": 48000, "channels": 2}},
                "output_path": "/tmp/out.mp4",
                "plan_sha256": "{}"
            }}{}"#,
            source_path.to_string_lossy(),
            source_sha,
            VALID_SHA,
            patch
        ))
        .expect("plan fixture json")
    }

    #[test]
    fn minimal_blur_source_plan_validates() {
        let dir = fixture_dir("minimal");
        let plan = decode_and_validate(plan_json(&dir, "")).expect("plan must validate");
        assert_eq!(plan.run_id, "job-1");
        assert_eq!(plan.output.width, 1080);
        assert_eq!(plan.background.as_ref().unwrap().mode, "blur_source");
    }

    #[test]
    fn wrong_version_fails_closed() {
        let dir = fixture_dir("version");
        let mut value = plan_json(&dir, "");
        value["version"] = serde_json::json!("render-plan.v2");
        assert!(decode_and_validate(value).is_err());
    }

    #[test]
    fn missing_source_file_fails_closed() {
        let dir = fixture_dir("missing");
        let mut value = plan_json(&dir, "");
        value["source"]["path"] = serde_json::json!("/nonexistent/source.mp4");
        assert!(decode_and_validate(value).is_err());
    }

    #[test]
    fn source_hash_mismatch_fails_closed() {
        let dir = fixture_dir("hash");
        let mut value = plan_json(&dir, "");
        value["source"]["sha256"] = serde_json::json!(VALID_SHA);
        let error = decode_and_validate(value).unwrap_err();
        assert!(error.contains("hash mismatch"), "error: {error}");
    }

    #[test]
    fn invalid_background_mode_fails_closed() {
        let dir = fixture_dir("bgmode");
        let mut value = plan_json(&dir, "");
        value["background"]["mode"] = serde_json::json!("zoom");
        assert!(decode_and_validate(value).is_err());
    }

    #[test]
    fn asset_background_requires_verified_artifact() {
        let dir = fixture_dir("bgasset");
        let background_path = dir.join("bg.png");
        fs::write(&background_path, b"bg-bytes").unwrap();
        let bg_sha = sha256_file(background_path.to_str().unwrap()).unwrap();
        let mut value = plan_json(&dir, "");
        value["background"] = serde_json::json!({
            "mode": "asset",
            "asset_id": "bg-1",
            "path": background_path.to_string_lossy(),
            "sha256": bg_sha
        });
        decode_and_validate(value).expect("asset background must validate");
    }

    #[test]
    fn watermark_and_subtitles_validate_with_verified_files() {
        let dir = fixture_dir("wm");
        let wm_path = dir.join("watermark.png");
        let sub_path = dir.join("subtitles.ass");
        fs::write(&wm_path, b"wm-bytes").unwrap();
        fs::write(&sub_path, b"[Script Info]").unwrap();
        let wm_sha = sha256_file(wm_path.to_str().unwrap()).unwrap();
        let sub_sha = sha256_file(sub_path.to_str().unwrap()).unwrap();
        let mut value = plan_json(&dir, "");
        value["watermark"] = serde_json::json!({
            "asset_id": "wm-1",
            "path": wm_path.to_string_lossy(),
            "sha256": wm_sha,
            "position": "top_right",
            "opacity": 0.85,
            "margin_px": 40
        });
        value["subtitles"] = serde_json::json!({
            "mode": "burn",
            "style_id": "shorts-v1",
            "path": sub_path.to_string_lossy(),
            "sha256": sub_sha
        });
        decode_and_validate(value).expect("watermark + subtitles must validate");
    }

    #[test]
    fn sidecar_subtitles_are_accepted() {
        let dir = fixture_dir("sidecar");
        let sub_path = dir.join("subtitles.ass");
        fs::write(&sub_path, b"[Script Info]").unwrap();
        let sub_sha = sha256_file(sub_path.to_str().unwrap()).unwrap();
        let mut value = plan_json(&dir, "");
        value["subtitles"] = serde_json::json!({
            "mode": "sidecar",
            "path": sub_path.to_string_lossy(),
            "sha256": sub_sha
        });
        decode_and_validate(value).expect("sidecar subtitles must validate");
    }

    #[test]
    fn invalid_audio_mode_fails_closed() {
        let dir = fixture_dir("audiomode");
        let mut value = plan_json(&dir, "");
        value["audio"]["mode"] = serde_json::json!("reencode_always");
        assert!(decode_and_validate(value).is_err());
    }
}

//! Pure pre-render preflight checks.
//!
//! Go owns canonical state and decides what a job may render. This module only
//! reports what it sees, so it deliberately returns a structured report instead
//! of a `MuscleError`: the caller keeps the policy, the engine keeps the facts.
//!
//! Three groups of checks, all read-only:
//!
//! - assets: the path exists, `ffprobe` can open it, it has a video stream and
//!   a positive duration;
//! - subtitles: valid windows, no overlap, a comfortable maximum reading speed
//!   and no cue past the end of the video;
//! - overlays: valid windows, no overlap on the same track, and the same
//!   end-of-video bound.
//!
//! Nothing here creates, modifies or deletes a file: no output path is even
//! accepted by the request type.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::probe::{ffprobe_path, probe_file};

/// Reading speed above this is a warning, not an error: a dense cue is a
/// quality signal for the caller, while a structural overlap is a defect.
pub const MAX_READING_CHARS_PER_SEC: f64 = 17.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightIssue {
    pub severity: Severity,
    /// Stable machine code (`asset_missing`, `cue_overlap`, …). The free-text
    /// explanation stays in `detail` so a consumer never parses prose.
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightReport {
    /// True when no issue has [`Severity::Error`]. Warnings never block here.
    pub ok: bool,
    pub issues: Vec<PreflightIssue>,
}

impl PreflightReport {
    pub fn errors(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .count()
    }

    pub fn warnings(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.severity == Severity::Warning)
            .count()
    }

    fn push(
        &mut self,
        severity: Severity,
        code: &str,
        item_id: Option<&str>,
        path: Option<&str>,
        detail: impl Into<String>,
    ) {
        self.issues.push(PreflightIssue {
            severity,
            code: code.to_string(),
            item_id: item_id.map(str::to_string),
            path: path.map(str::to_string),
            detail: detail.into(),
        });
    }

    fn error(
        &mut self,
        code: &str,
        item_id: Option<&str>,
        path: Option<&str>,
        detail: impl Into<String>,
    ) {
        self.push(Severity::Error, code, item_id, path, detail);
    }

    fn warning(&mut self, code: &str, item_id: Option<&str>, detail: impl Into<String>) {
        self.push(Severity::Warning, code, item_id, None, detail);
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct PreflightRequest {
    /// Declared duration of the video the overlays and cues will sit on. When
    /// absent, the end-of-video checks are skipped instead of guessed.
    #[serde(default)]
    pub video_duration_ms: Option<i64>,
    #[serde(default)]
    pub assets: Vec<PreflightAsset>,
    #[serde(default)]
    pub subtitles: Vec<PreflightCue>,
    #[serde(default)]
    pub overlays: Vec<PreflightOverlay>,
    /// Same override the other operations accept; `ffprobe` is resolved next to
    /// an explicit `ffmpeg` path and from `PATH` otherwise.
    #[serde(default)]
    pub ffmpeg_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PreflightAsset {
    pub asset_id: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PreflightCue {
    pub item_id: String,
    pub in_us: i64,
    pub out_us: i64,
    #[serde(default)]
    pub text: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PreflightOverlay {
    pub item_id: String,
    pub track: u32,
    pub in_us: i64,
    pub out_us: i64,
}

/// Runs every check and returns the report in input order.
pub fn check(request: &PreflightRequest) -> PreflightReport {
    let mut report = PreflightReport {
        ok: true,
        issues: Vec::new(),
    };

    if let Some(duration_ms) = request.video_duration_ms {
        if duration_ms <= 0 {
            report.error(
                "invalid_video_duration",
                None,
                None,
                format!("video_duration_ms must be > 0, got {duration_ms}"),
            );
        }
    }

    check_assets(request, &mut report);
    check_subtitles(request, &mut report);
    check_overlays(request, &mut report);

    report.ok = report.errors() == 0;
    report
}

fn check_assets(request: &PreflightRequest, report: &mut PreflightReport) {
    let ffprobe = ffprobe_path(request.ffmpeg_path.as_deref().unwrap_or("ffmpeg"));
    for asset in &request.assets {
        let id = asset.asset_id.as_str();
        if asset.path.trim().is_empty() {
            report.error("asset_path_missing", Some(id), None, "asset has no path");
            continue;
        }
        if !Path::new(&asset.path).is_file() {
            report.error(
                "asset_missing",
                Some(id),
                Some(&asset.path),
                "asset file does not exist or is not a regular file",
            );
            continue;
        }
        match probe_file(&ffprobe, &asset.path) {
            Err(error) => report.error(
                "asset_unreadable",
                Some(id),
                Some(&asset.path),
                error.to_string(),
            ),
            Ok(metadata) => {
                if !metadata.has_video {
                    report.error(
                        "asset_no_video",
                        Some(id),
                        Some(&asset.path),
                        "asset has no video stream",
                    );
                }
                if metadata.duration_sec <= 0.0 {
                    report.error(
                        "asset_zero_duration",
                        Some(id),
                        Some(&asset.path),
                        format!("asset duration is {}", metadata.duration_sec),
                    );
                }
            }
        }
    }
}

fn check_subtitles(request: &PreflightRequest, report: &mut PreflightReport) {
    let limit_us = request.video_duration_ms.map(|ms| ms.saturating_mul(1000));
    for cue in &request.subtitles {
        let id = cue.item_id.as_str();
        if cue.in_us < 0 || cue.out_us <= cue.in_us {
            report.error(
                "cue_invalid_window",
                Some(id),
                None,
                format!(
                    "window [{}, {}] must satisfy 0 <= in_us < out_us",
                    cue.in_us, cue.out_us
                ),
            );
            continue;
        }
        if let Some(limit_us) = limit_us {
            if limit_us > 0 && cue.out_us > limit_us {
                report.error(
                    "cue_beyond_video",
                    Some(id),
                    None,
                    format!("cue ends at {}us, video ends at {}us", cue.out_us, limit_us),
                );
            }
        }
        let chars = cue.text.trim().chars().count();
        if chars == 0 {
            report.warning("cue_empty_text", Some(id), "cue has no text");
        } else {
            let seconds = (cue.out_us - cue.in_us) as f64 / 1_000_000.0;
            let speed = chars as f64 / seconds;
            if speed > MAX_READING_CHARS_PER_SEC {
                report.warning(
                    "cue_reading_too_fast",
                    Some(id),
                    format!(
                        "{:.1} chars/s over {:.2}s exceeds the {:.0} chars/s comfortable maximum",
                        speed, seconds, MAX_READING_CHARS_PER_SEC
                    ),
                );
            }
        }
    }

    // Overlap is a structural defect: sort by start and track the furthest end
    // so a long cue that swallows several short ones is reported once, on the
    // later cue, deterministically.
    let mut order: Vec<usize> = (0..request.subtitles.len())
        .filter(|index| {
            let cue = &request.subtitles[*index];
            cue.in_us >= 0 && cue.out_us > cue.in_us
        })
        .collect();
    order.sort_by_key(|index| (request.subtitles[*index].in_us, *index));
    let mut furthest: Option<usize> = None;
    for index in order {
        let cue = &request.subtitles[index];
        if let Some(previous_index) = furthest {
            let previous = &request.subtitles[previous_index];
            if cue.in_us < previous.out_us {
                report.error(
                    "cue_overlap",
                    Some(cue.item_id.as_str()),
                    None,
                    format!(
                        "starts at {}us before {} ends at {}us",
                        cue.in_us, previous.item_id, previous.out_us
                    ),
                );
            }
            if cue.out_us <= previous.out_us {
                continue;
            }
        }
        furthest = Some(index);
    }
}

fn check_overlays(request: &PreflightRequest, report: &mut PreflightReport) {
    let limit_us = request.video_duration_ms.map(|ms| ms.saturating_mul(1000));
    let mut by_track: BTreeMap<u32, Vec<&PreflightOverlay>> = BTreeMap::new();
    for overlay in &request.overlays {
        if overlay.in_us < 0 || overlay.out_us <= overlay.in_us {
            report.error(
                "overlay_invalid_window",
                Some(overlay.item_id.as_str()),
                None,
                format!(
                    "window [{}, {}] must satisfy 0 <= in_us < out_us",
                    overlay.in_us, overlay.out_us
                ),
            );
            continue;
        }
        if let Some(limit_us) = limit_us {
            if limit_us > 0 && overlay.out_us > limit_us {
                report.error(
                    "overlay_beyond_video",
                    Some(overlay.item_id.as_str()),
                    None,
                    format!(
                        "overlay ends at {}us, video ends at {}us",
                        overlay.out_us, limit_us
                    ),
                );
            }
        }
        by_track.entry(overlay.track).or_default().push(overlay);
    }

    for (track, mut overlays) in by_track {
        overlays.sort_by_key(|overlay| (overlay.in_us, overlay.item_id.as_str()));
        // Track the furthest end, not just the previous neighbour: a long
        // overlay can still be on screen when a later short one starts, and the
        // adjacent pair check would miss it (same rule as for cues).
        let mut furthest: Option<&PreflightOverlay> = None;
        for overlay in overlays {
            if let Some(active) = furthest {
                if overlay.in_us < active.out_us {
                    report.error(
                        "overlay_track_overlap",
                        Some(&overlay.item_id),
                        None,
                        format!(
                            "track {track}: starts at {}us before {} ends at {}us",
                            overlay.in_us, active.item_id, active.out_us
                        ),
                    );
                }
                if overlay.out_us <= active.out_us {
                    continue;
                }
            }
            furthest = Some(overlay);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> PreflightRequest {
        PreflightRequest {
            video_duration_ms: Some(60_000),
            assets: Vec::new(),
            subtitles: Vec::new(),
            overlays: Vec::new(),
            ffmpeg_path: None,
        }
    }

    fn cue(item_id: &str, in_us: i64, out_us: i64, text: &str) -> PreflightCue {
        PreflightCue {
            item_id: item_id.to_string(),
            in_us,
            out_us,
            text: text.to_string(),
        }
    }

    fn overlay(item_id: &str, track: u32, in_us: i64, out_us: i64) -> PreflightOverlay {
        PreflightOverlay {
            item_id: item_id.to_string(),
            track,
            in_us,
            out_us,
        }
    }

    #[test]
    fn an_empty_plan_is_clean() {
        let report = check(&request());
        assert!(report.ok);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn overlapping_cues_are_errors() {
        let mut request = request();
        request.subtitles = vec![
            cue("c1", 0, 3_000_000, "Prima battuta."),
            cue("c2", 2_500_000, 5_000_000, "Seconda battuta."),
        ];
        let report = check(&request);
        assert!(!report.ok);
        assert_eq!(report.errors(), 1);
        assert_eq!(report.issues[0].code, "cue_overlap");
        assert_eq!(report.issues[0].item_id.as_deref(), Some("c2"));
    }

    #[test]
    fn adjacent_cues_are_not_an_overlap() {
        let mut request = request();
        request.subtitles = vec![
            cue("c1", 0, 3_000_000, "Prima battuta."),
            cue("c2", 3_000_000, 5_000_000, "Seconda battuta."),
        ];
        assert!(check(&request).ok);
    }

    #[test]
    fn a_long_cue_swallowing_short_ones_is_reported_once_per_later_cue() {
        let mut request = request();
        request.subtitles = vec![
            cue("long", 0, 10_000_000, "Una battuta lunga."),
            cue("one", 1_000_000, 2_000_000, "Uno."),
            cue("two", 3_000_000, 4_000_000, "Due."),
        ];
        let report = check(&request);
        let overlaps: Vec<_> = report
            .issues
            .iter()
            .filter(|issue| issue.code == "cue_overlap")
            .collect();
        assert_eq!(overlaps.len(), 2);
        assert_eq!(overlaps[0].item_id.as_deref(), Some("one"));
        assert_eq!(overlaps[1].item_id.as_deref(), Some("two"));
    }

    #[test]
    fn a_cue_past_the_video_end_is_an_error() {
        let mut request = request();
        request.video_duration_ms = Some(2_000);
        request.subtitles = vec![cue("c1", 1_500_000, 2_500_000, "Troppo tardi.")];
        let report = check(&request);
        assert_eq!(report.issues[0].code, "cue_beyond_video");
        assert!(!report.ok);
    }

    #[test]
    fn a_dense_cue_is_only_a_warning() {
        let mut request = request();
        request.subtitles = vec![cue("c1", 0, 1_000_000, "1234567890123456789")];
        let report = check(&request);
        assert!(report.ok, "a warning must not block the render");
        assert_eq!(report.issues[0].severity, Severity::Warning);
        assert_eq!(report.issues[0].code, "cue_reading_too_fast");
    }

    #[test]
    fn an_invalid_cue_window_is_an_error_and_skips_the_overlap_scan() {
        let mut request = request();
        request.subtitles = vec![cue("bad", 2_000_000, 2_000_000, "Zero.")];
        let report = check(&request);
        assert_eq!(report.errors(), 1);
        assert_eq!(report.issues[0].code, "cue_invalid_window");
    }

    #[test]
    fn overlays_overlap_only_on_the_same_track() {
        let mut request = request();
        request.overlays = vec![
            overlay("a", 0, 0, 2_000_000),
            overlay("b", 1, 1_000_000, 3_000_000),
        ];
        assert!(check(&request).ok, "different tracks may overlap");

        request.overlays.push(overlay("c", 0, 1_000_000, 3_000_000));
        let report = check(&request);
        assert_eq!(report.errors(), 1);
        assert_eq!(report.issues[0].code, "overlay_track_overlap");
        assert_eq!(report.issues[0].item_id.as_deref(), Some("c"));
    }

    #[test]
    fn a_long_overlay_still_overlaps_a_later_short_one() {
        let mut request = request();
        request.overlays = vec![
            overlay("long", 0, 0, 10_000_000),
            overlay("one", 0, 1_000_000, 2_000_000),
            overlay("two", 0, 3_000_000, 4_000_000),
        ];
        let report = check(&request);
        let overlaps: Vec<_> = report
            .issues
            .iter()
            .filter(|issue| issue.code == "overlay_track_overlap")
            .collect();
        assert_eq!(overlaps.len(), 2, "both short overlays hit the long one");
        assert_eq!(overlaps[0].item_id.as_deref(), Some("one"));
        assert_eq!(overlaps[1].item_id.as_deref(), Some("two"));
    }

    #[test]
    fn adjacent_overlays_on_a_track_are_not_an_overlap() {
        let mut request = request();
        request.overlays = vec![
            overlay("a", 0, 0, 2_000_000),
            overlay("b", 0, 2_000_000, 3_000_000),
        ];
        assert!(check(&request).ok);
    }

    #[test]
    fn an_overlay_past_the_video_end_is_an_error() {
        let mut request = request();
        request.video_duration_ms = Some(1_000);
        request.overlays = vec![overlay("a", 0, 0, 2_000_000)];
        let report = check(&request);
        assert_eq!(report.issues[0].code, "overlay_beyond_video");
    }

    #[test]
    fn an_invalid_video_duration_is_an_error() {
        let mut request = request();
        request.video_duration_ms = Some(0);
        let report = check(&request);
        assert_eq!(report.issues[0].code, "invalid_video_duration");
        assert!(!report.ok);
    }

    #[test]
    fn a_missing_asset_path_is_an_error_without_touching_the_filesystem() {
        let mut request = request();
        request.assets = vec![PreflightAsset {
            asset_id: "a1".to_string(),
            path: "  ".to_string(),
        }];
        let report = check(&request);
        assert_eq!(report.issues[0].code, "asset_path_missing");
        assert_eq!(report.issues[0].item_id.as_deref(), Some("a1"));
    }

    #[test]
    fn the_report_serializes_with_stable_codes() {
        let mut request = request();
        request.subtitles = vec![cue("c1", 0, 1_000_000, "1234567890123456789")];
        let value = serde_json::to_value(check(&request)).expect("report serializes");
        assert_eq!(value["ok"], true);
        assert_eq!(value["issues"][0]["code"], "cue_reading_too_fast");
        assert_eq!(value["issues"][0]["severity"], "warning");
    }
}

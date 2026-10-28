//! Overlay-manifest projection math — OVERLAY-MANIFEST-CONTRACT.md §3.
//!
//! The worker projects manifest entries onto the master's frame grid as a
//! REPLACEMENT of frames (the §3 semantics): every window is start-inclusive
//! and end-exclusive on the ceiling grid — frames `floor(in)`..`ceil(out)`.
//! Each entry's artifact position comes from `artifact_in_us/artifact_out_us`
//! when present (packed timelines), otherwise from `in_us/out_us` — an
//! unpacked artifact is the plan canvas anchored at 0, so the plan position
//! IS the artifact position (§3.1).
//!
//! Overlapping windows are idempotent by construction: the producer packs
//! overlapping literal windows into one merged artifact interval with the
//! relative offsets preserved, so two entries covering the same master frame
//! agree on its artifact source. [`merge_pieces`] enforces exactly that —
//! on any disagreement the projection fails closed instead of guessing.

use crate::protocol::OverlayManifestEntry;

pub(crate) const US_PER_SECOND: i64 = 1_000_000;

/// Start-inclusive floor grid: the first master frame at or after `us`.
pub(crate) fn us_to_frame_floor(us: i64, fps_num: u32, fps_den: u32) -> i64 {
    (us * fps_num as i64) / (US_PER_SECOND * fps_den as i64)
}

/// End-exclusive ceiling grid: the first master frame at or after the end of
/// `us` coverage. A window is always `floor(in)..ceil(out)`.
pub(crate) fn us_to_frame_ceil(us: i64, fps_num: u32, fps_den: u32) -> i64 {
    let num = us * fps_num as i64;
    let den = US_PER_SECOND * fps_den as i64;
    (num + den - 1) / den
}

/// One resolved projection piece: the master frames `[master_start,
/// master_end)` are replaced by the artifact frames `[artifact_start,
/// artifact_end())`, one-to-one — so `artifact_end = artifact_start +
/// (master_end - master_start)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProjectionPiece {
    pub master_start: i64,
    pub master_end: i64,
    pub artifact_start: i64,
}

impl ProjectionPiece {
    pub(crate) fn artifact_end(&self) -> i64 {
        self.artifact_start + (self.master_end - self.master_start)
    }
}

/// Resolve one manifest entry to frames on the shared grid.
///
/// Clamping contract (§3 + the Go producer): the end is clamped to the
/// artifact's frame count (the master window truncates with it), and a start
/// beyond the artifact fails closed — the entry cannot be projected at all.
pub(crate) fn resolve_entry(
    entry: &OverlayManifestEntry,
    fps_num: u32,
    fps_den: u32,
    artifact_frames: i64,
    master_frames: i64,
) -> Result<ProjectionPiece, String> {
    let (artifact_in_us, artifact_out_us) = match (entry.artifact_in_us, entry.artifact_out_us) {
        (Some(in_us), Some(out_us)) => (in_us, out_us),
        _ => (entry.in_us, entry.out_us),
    };
    let artifact_start = us_to_frame_floor(artifact_in_us, fps_num, fps_den);
    let artifact_end = us_to_frame_ceil(artifact_out_us, fps_num, fps_den);
    if artifact_start >= artifact_frames {
        return Err(format!(
            "overlay manifest entry {} starts beyond the artifact (frame {} of {})",
            entry.item_id, artifact_start, artifact_frames
        ));
    }
    let artifact_end = artifact_end.min(artifact_frames);
    if artifact_end <= artifact_start {
        return Err(format!(
            "overlay manifest entry {} has an empty artifact window",
            entry.item_id
        ));
    }
    let master_start = us_to_frame_floor(entry.in_us, fps_num, fps_den);
    if master_start >= master_frames {
        return Err(format!(
            "overlay manifest entry {} starts beyond the master (frame {} of {})",
            entry.item_id, master_start, master_frames
        ));
    }
    // Clamping truncates the END of the window: the mapping stays
    // one-to-one from the head, and a master shorter than the window trims
    // the same trailing frames.
    let master_end = (master_start + (artifact_end - artifact_start)).min(master_frames);
    Ok(ProjectionPiece {
        master_start,
        master_end,
        artifact_start,
    })
}

/// Merge resolved pieces into disjoint runs on the master grid.
///
/// Two pieces that overlap on the master must agree on the artifact mapping
/// (idempotent replacement — the producer's packed windows preserve relative
/// offsets, so agreement holds by construction). Adjacent pieces merge only
/// when the artifact frames are contiguous too. Any disagreement fails
/// closed: the projection never guesses which artifact frame wins.
pub(crate) fn merge_pieces(mut pieces: Vec<ProjectionPiece>) -> Result<Vec<ProjectionPiece>, String> {
    if pieces.is_empty() {
        return Ok(Vec::new());
    }
    pieces.sort_by_key(|piece| (piece.master_start, piece.master_end));
    let mut merged: Vec<ProjectionPiece> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match merged.last_mut() {
            None => merged.push(piece),
            Some(current) => {
                if piece.master_start < current.master_end {
                    // Overlap: the mappings must agree at the overlap start.
                    let expected =
                        current.artifact_start + (piece.master_start - current.master_start);
                    if piece.artifact_start != expected {
                        return Err(format!(
                            "overlapping manifest windows disagree on the artifact mapping \
                             (master frame {}: {} vs {})",
                            piece.master_start, expected, piece.artifact_start
                        ));
                    }
                    current.master_end = current.master_end.max(piece.master_end);
                } else if piece.master_start == current.master_end
                    && piece.artifact_start == current.artifact_end()
                {
                    // Adjacent on both grids: one contiguous run.
                    current.master_end = piece.master_end;
                } else {
                    merged.push(piece);
                }
            }
        }
    }
    Ok(merged)
}

/// Translate a plan sub-window of one entry to its artifact positions
/// (the `scene.clip.{timeline_item_id,in_us,out_us}` wire): the offset
/// `entry.in_us -> entry.artifact_in_us` is preserved one-to-one, exactly as
/// the packed interval preserves relative offsets.
pub(crate) fn resolve_sub_window(
    entry: &OverlayManifestEntry,
    sub_in_us: i64,
    sub_out_us: i64,
) -> Result<(i64, i64), String> {
    if sub_in_us < entry.in_us || sub_out_us > entry.out_us || sub_out_us <= sub_in_us {
        return Err(format!(
            "timeline window [{sub_in_us}, {sub_out_us}) is outside entry {} window [{}, {})",
            entry.item_id, entry.in_us, entry.out_us
        ));
    }
    let base = entry.artifact_in_us.unwrap_or(entry.in_us);
    Ok((base + (sub_in_us - entry.in_us), base + (sub_out_us - entry.in_us)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::OverlayManifestEntry;

    fn entry(item_id: &str, in_us: i64, out_us: i64) -> OverlayManifestEntry {
        OverlayManifestEntry {
            item_id: item_id.to_string(),
            track: 1,
            in_us,
            out_us,
            content_hash: "a".repeat(64),
            placement: None,
            artifact_in_us: None,
            artifact_out_us: None,
        }
    }

    fn packed(mut entry: OverlayManifestEntry, artifact_in_us: i64, artifact_out_us: i64) -> OverlayManifestEntry {
        entry.artifact_in_us = Some(artifact_in_us);
        entry.artifact_out_us = Some(artifact_out_us);
        entry
    }

    #[test]
    fn grid_is_start_floor_end_ceil() {
        // 30 fps: 33_333 us lands inside frame 0 (floor) but its coverage
        // ends inside frame 1 (ceil).
        assert_eq!(us_to_frame_floor(33_333, 30, 1), 0);
        assert_eq!(us_to_frame_ceil(33_333, 30, 1), 1);
        assert_eq!(us_to_frame_floor(0, 30, 1), 0);
        assert_eq!(us_to_frame_ceil(1_000_000, 30, 1), 30);
        assert_eq!(us_to_frame_floor(1_000_000, 30, 1), 30);
        // Fractional fps (30000/1001): exact rational arithmetic.
        assert_eq!(us_to_frame_floor(1_001_000, 30000, 1001), 30);
    }

    #[test]
    fn aligned_entry_maps_plan_position_to_artifact_position() {
        // Unpacked: the artifact is the plan canvas anchored at 0.
        let piece = resolve_entry(&entry("phrase.1", 500_000, 1_000_000), 30, 1, 60, 60).unwrap();
        assert_eq!(
            piece,
            ProjectionPiece {
                master_start: 15,
                master_end: 30,
                artifact_start: 15
            }
        );
        assert_eq!(piece.artifact_end(), 30);
    }

    #[test]
    fn packed_entry_maps_through_artifact_positions() {
        // Plan [1.2s, 1.6s) but the content lives at artifact [0.1s, 0.5s).
        let entry = packed(entry("phrase.1", 1_200_000, 1_600_000), 100_000, 500_000);
        let piece = resolve_entry(&entry, 30, 1, 60, 60).unwrap();
        assert_eq!(
            piece,
            ProjectionPiece {
                master_start: 36,
                master_end: 48,
                artifact_start: 3
            }
        );
    }

    #[test]
    fn artifact_end_clamps_and_truncates_the_master_window() {
        // The artifact has 31 frames but the window claims up to frame 45:
        // the end clamps to 31 and the master window truncates to 3 frames.
        let entry = packed(entry("phrase.1", 0, 500_000), 950_000, 1_500_000);
        let piece = resolve_entry(&entry, 30, 1, 31, 60).unwrap();
        assert_eq!(piece.master_start, 0);
        assert_eq!(piece.master_end, 3); // clamped to artifact frame 31
        assert_eq!(piece.artifact_start, 28); // 950 ms -> frame 28
        assert_eq!(piece.artifact_end(), 31);
    }

    #[test]
    fn start_beyond_the_artifact_fails_closed() {
        let entry = packed(entry("phrase.1", 0, 500_000), 2_000_000, 2_500_000);
        let error = resolve_entry(&entry, 30, 1, 60, 60).unwrap_err();
        assert!(error.contains("starts beyond the artifact"), "{error}");
    }

    #[test]
    fn overlapping_pieces_agreeing_on_the_mapping_merge_idempotently() {
        // Producer packing: A plan [10,20)->art [50,60), B plan [15,25)->art
        // [55,65): at master 15 both map to artifact 55.
        let merged = merge_pieces(vec![
            ProjectionPiece { master_start: 10, master_end: 20, artifact_start: 50 },
            ProjectionPiece { master_start: 15, master_end: 25, artifact_start: 55 },
        ])
        .unwrap();
        assert_eq!(
            merged,
            vec![ProjectionPiece { master_start: 10, master_end: 25, artifact_start: 50 }]
        );
        assert_eq!(merged[0].artifact_end(), 65);
    }

    #[test]
    fn overlapping_pieces_with_conflicting_mappings_fail_closed() {
        let error = merge_pieces(vec![
            ProjectionPiece { master_start: 10, master_end: 20, artifact_start: 50 },
            ProjectionPiece { master_start: 15, master_end: 25, artifact_start: 30 },
        ])
        .unwrap_err();
        assert!(error.contains("disagree on the artifact mapping"), "{error}");
    }

    #[test]
    fn adjacent_pieces_merge_only_when_artifact_contiguous() {
        let contiguous = merge_pieces(vec![
            ProjectionPiece { master_start: 0, master_end: 10, artifact_start: 40 },
            ProjectionPiece { master_start: 10, master_end: 20, artifact_start: 50 },
        ])
        .unwrap();
        assert_eq!(contiguous.len(), 1);
        assert_eq!(contiguous[0].artifact_end(), 60);

        let disjoint = merge_pieces(vec![
            ProjectionPiece { master_start: 0, master_end: 10, artifact_start: 40 },
            ProjectionPiece { master_start: 10, master_end: 20, artifact_start: 70 },
        ])
        .unwrap();
        assert_eq!(disjoint.len(), 2);
    }

    #[test]
    fn sub_window_preserves_the_packed_offset() {
        let packed_entry = packed(entry("phrase.1", 1_000_000, 2_000_000), 3_000_000, 4_000_000);
        // Plan sub-window [1.2s, 1.5s) maps to artifact [3.2s, 3.5s).
        assert_eq!(
            resolve_sub_window(&packed_entry, 1_200_000, 1_500_000).unwrap(),
            (3_200_000, 3_500_000)
        );
        // Unpacked: the offset is identity.
        let plain = entry("phrase.2", 1_000_000, 2_000_000);
        assert_eq!(
            resolve_sub_window(&plain, 1_200_000, 1_500_000).unwrap(),
            (1_200_000, 1_500_000)
        );
    }

    #[test]
    fn sub_window_outside_the_entry_fails_closed() {
        let plain = entry("phrase.1", 1_000_000, 2_000_000);
        assert!(resolve_sub_window(&plain, 900_000, 1_500_000).is_err());
        assert!(resolve_sub_window(&plain, 1_200_000, 2_500_000).is_err());
        assert!(resolve_sub_window(&plain, 1_500_000, 1_500_000).is_err());
    }
}

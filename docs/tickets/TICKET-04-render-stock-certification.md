# TICKET 04 — certify the real `render_stock` output

- Priority: P1
- Status: DONE
- Source: audit roadmap PR 4
- Files: `pipelinegen-muscles/src/render_stock_canonical_executor.rs`,
  `probe.rs`

## Problem

`render_stock` checked that the intermediate file existed and was non-empty,
then renamed it and returned `ok: true` with metadata derived from the plan.
Fields such as `video_stream_count` were zero while `has_video` was true, so
success did not certify the bytes.

## Change

- Before publishing, the concatenated file is probed with `ffprobe`.
- `validate_final_output` checks: exactly one video stream, no audio stream,
  expected geometry, exact rational fps, codec, `yuv420p`, and duration within
  0.25 s of the plan.
- The response metadata now comes from the probe, with only the measured
  timing fields overlaid.
- A probe or certification failure removes the part and returns `ok: false`;
  the previously published file is left untouched.
- Added positive and negative unit tests for the certification contract.
- The synthetic integration fixture now writes the canonical bt709/tv VUI.
  Without it the fixture and the re-encoded delivery were interpreted with
  different YUV→RGB matrices, producing a constant pixel offset unrelated to
  seek or frame selection. The three formerly failing FFmpeg 7 checks now
  pass without touching their `< 8.0` thresholds.

## Acceptance evidence

```sh
cargo test -p pipelinegen-muscles --lib render_stock::executor
```

The two certification tests pass:

- `final_output_certification_accepts_a_matching_probe`
- `final_output_certification_rejects_contract_violations`

The full end-to-end suite also passes 4/4 on FFmpeg 7:

```sh
cargo test -p pipelinegen-muscles --test canonical_render_segments
```

`mid_asset_segments_input_seek_is_frame_exact`,
`mid_asset_freeze_holds_the_seeked_frame`,
`parallel_segments_concatenate_into_the_full_timeline` and
`multi_manifest_projection_matches_single_artifact_projection` all pass.

## Residual

The certification checks duration and exact fps rather than decoding every
frame to count `nb_frames`; the integration suite asserts frame counts where
the host FFmpeg baseline matches. A full frame-count decode remains an
expensive optional check.

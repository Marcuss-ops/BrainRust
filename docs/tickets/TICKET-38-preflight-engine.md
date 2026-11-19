# TICKET 38 — preflight engine before render

- Priority: P2
- Status: CLOSED 2026-11-19 (engine + real-media tests, full suite green).
  The wire operation and the Go caller moved to TICKET-39, which is
  cross-repo; this ticket's scope is complete.
- Source: product follow-up from the 2026-11-19 review (`Preflight prima del
  render`)
- Files: `pipelinegen-muscles/src/preflight.rs`,
  `pipelinegen-muscles/src/lib.rs`, `pipelinegen-muscles/tests/preflight.rs`

## Problem

A render only discovers a missing asset, an unreadable file, an overlapping cue
or an overlay past the end of the video after the job has started. The review
asked for a preflight that answers those questions *before* the render.

## Change

- `pipelinegen_muscles::preflight` exposes a typed request and a structured
  report:
  - `PreflightRequest { video_duration_ms, assets, subtitles, overlays,
    ffmpeg_path }`;
  - `PreflightReport { ok, issues[] }` with `PreflightIssue { severity, code,
    item_id, path, detail }`.
- Checks:
  - assets: path present, file exists, `ffprobe` can open it, it has a video
    stream and a positive duration;
  - subtitles: valid window, no overlap, comfortable reading speed (advisory
    warning above the 17 chars/s maximum) and no cue past the declared video
    duration;
  - overlays: valid window, no overlap on the same track (including an overlay
    enclosed by a longer one), same end-of-video bound.
- Stable machine codes (`asset_missing`, `asset_unreadable`, `asset_no_video`,
  `cue_overlap`, `cue_reading_too_fast`, `overlay_track_overlap`, …); prose only
  in `detail`. `ok` is false only when at least one issue is an error, so a
  caller can keep warnings advisory.
- The engine is pure: the request has no output path, it only reads assets
  through the existing `probe_file` owner of the ffprobe invocation, and it
  returns data instead of a `MuscleError`. The blocking decision stays with Go,
  as the boundary requires.

## Acceptance evidence

```sh
cargo fmt --all -- --check
cargo clippy -p pipelinegen-muscles --all-targets -- -D warnings
cargo test -p pipelinegen-muscles --lib preflight
cargo test -p pipelinegen-muscles --test preflight
```

Results (2026-11-19, `--offline`):

- `fmt`, `clippy` — rc 0.
- `--lib preflight` — **14 passed**: clean plan, cue overlap (including one long
  cue swallowing several short ones), adjacent cues, past-the-end cue, dense cue
  as warning only, invalid window, per-track overlay overlap (including a long
  overlay enclosing later short ones), adjacent overlays, overlay past the end,
  invalid duration, missing path and stable JSON codes.
- `--test preflight` — **4 passed** against real media rendered with ffmpeg:
  a real clip passes with zero issues and the directory is byte-for-byte
  unchanged (no writes), a missing asset is `asset_missing`, a text file is
  `asset_unreadable`, an audio-only file is `asset_no_video`.

## Residual & closed scope

- **Wire operation (`mediaexec.v2`) and Go caller — closed by TICKET-39 on
  2026-11-19.** The operation, schemas, golden and typed Go caller landed in
  the working trees; the pinned `v0.3.7` media binary predates the operation,
  so deployment waits for the next release.
- Reading speed and end-of-video checks use the duration the caller declares.
  When `video_duration_ms` is absent they are skipped rather than guessed.
- The concurrent-overlay budget is not checked: the request does not yet carry a
  budget, and inventing one here would move policy into Rust.

## Closure — 2026-11-19

`cargo test -p semantic-core -p semantic-engine -p pipelinegen-muscles`
finished green, including `pipelinegen-muscles --lib` 137, `--test preflight`
4 against real ffmpeg media, `canonical_render_segments` and
`render_audio_plan`. `rustfmt --check` on the touched files and
`clippy --all-targets -D warnings` are rc 0. The engine has no remaining
acceptance criterion; the consumer-side work closed with TICKET-39 on
2026-11-19.

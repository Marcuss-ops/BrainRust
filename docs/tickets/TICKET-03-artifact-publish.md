# TICKET 03 — unique intermediate files and protected publication

- Priority: P1
- Status: DONE
- Source: audit roadmap PR 3
- Files: `pipelinegen-muscles/src/artifact.rs`

## Problem

`part_path` was deterministic (`final.part.mp4`), so two concurrent attempts on
the same output wrote the same temporary file. Publication was a plain rename,
so a slow older attempt could replace a newer valid result.

## Change

- Every attempt gets `stem.<pid>.<generation>.part.ext`; 32 concurrent calls
  produce 32 distinct paths.
- Publication holds a process-local per-final-path generation map. A part whose
  generation is older than the latest published generation is removed and
  rejected with an explicit error. A newer attempt always wins.
- The rename remains atomic; the final file is always one complete attempt,
  never a mixture.
- Added a concurrent publish test that asserts the newest complete attempt is
  the published file.

## Acceptance evidence

```sh
cargo test -p pipelinegen-muscles --lib artifact::tests
```

All artifact tests pass, including:

- `part_paths_are_unique_for_the_same_final_path`
- `concurrent_publishes_leave_one_complete_file`

## Residual

The generation map is process-local, matching the worker that owns an output
path. Across separate worker processes, uniqueness and atomic rename still
prevent a corrupt file, but stale-generation ordering would need a shared
lease/lock owned by the caller.

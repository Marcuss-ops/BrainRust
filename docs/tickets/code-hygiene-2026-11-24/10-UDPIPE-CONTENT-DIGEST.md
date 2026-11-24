# BR-CACHE-01 — UDPipe model identity must be content-addressed

- **Status:** CLOSED (2026-11-24)
- **Priority:** P1
- **Owner:** `semantic-engine`
- **Evidence date:** 2026-11-24

## Problem

`semantic-engine/src/lib.rs::model_profile` identifies a UDPipe model by file
name and `metadata.len()`. A model replaced with different bytes of the same
size keeps the same persistent-cache key, so a worker can serve a manifest
produced by a different model. File size is not an identity.

The digest must not be recomputed on every request: the models are large and
`cache_identity` is called for every disk-cache lookup.

## Acceptance criteria

- [x] The model component of `cache_identity` includes a SHA-256 digest of the
      model file content, in addition to the file name.
- [x] The digest is computed at most once per process for a given path and the
      per-process cache is bounded.
- [x] `absent` (no model) and unreadable-file behaviour stay explicit and do
      not panic.
- [x] A unit test writes two same-name, same-size files with different bytes
      and proves their profiles differ; the same path stays stable across calls.
- [x] The existing disk-cache key test still proves that an identity change
      moves the key and that the same identity is stable.
- [x] The module docs in `cache/disk.rs` and `lib.rs` describe content, not
      name+size.

## Verification (2026-11-24)

- `cargo test -p semantic-engine --lib model_profile --locked` -> **2 passed,
  0 failed**. The tests cover same-name/same-size different bytes and the
  missing-model case.
- Direct test binary `semantic_engine-58b0504b0fd6a2d5 cache` -> **29 passed,
  0 failed**, including `the_key_carries_the_build_and_content_identity` and
  `the_key_changes_when_the_injected_identity_changes`.
- `rustfmt --edition 2021 --check semantic-engine/src/lib.rs semantic-engine/src/cache/disk.rs`
  -> RC 0.
- `git diff --check` on the touched paths -> RC 0.

Not run: `cargo clippy -p semantic-engine --lib --locked -- -D warnings` and
the full `semantic-engine --lib` suite. The shared target was held by a
workspace build compiling `gliner25-rs`/`ort`, and the 600 s window expired
during compilation. The targeted tests above ran from the already-built test
binary; the cache suite is the relevant regression surface for this change.

## Non-goals

- Replacing the wire-visible manifest fingerprint (it stays FNV-1a).
- Hashing the model path: operator-specific paths must not change the key.

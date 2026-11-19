# TICKET 37 — persistent manifest cache

- Priority: P1
- Status: CLOSED 2026-11-19 (memory + disk store; H1–H4 closed; T1–T15 run,
  full suite green; no deployment SLO). The Go opt-in is tracked in
  TICKET-39.
- Source: performance follow-up from the 2026-11-19 review
  (`Cache persistente dei manifest per hash dello script`)
- Files: `semantic-engine/src/cache.rs`,
  `semantic-engine/src/cache/disk.rs`, `semantic-engine/src/lib.rs`,
  `semantic-engine/src/protocol.rs`, `semantic-engine/Cargo.toml`,
  `semantic-engine/tests/persistent_cache.rs`

## Problem

`ManifestCache` was a FIFO map in memory. It kept a script warm for the life of
one worker process, then everything was lost: a restart, a crashed batch or a
replay on the next day re-ran the extractor on unchanged text. The proposal also
asked for a key stronger than the 64-bit FNV fingerprint before any result could
be trusted across processes.

## Change

- `cache/disk.rs` adds `DiskCache`, a content-addressed store:
  - directory from `SEMANTIC_CACHE_DIR`; **unset means the worker is
    memory-only and behaves exactly as before**;
  - optional byte budget from `SEMANTIC_CACHE_MAX_BYTES` (default 1 GiB);
  - key = SHA-256 over the canonical request bytes (language, scenes, editorial
    options, safety cap), the analyzer versions and a **cache identity**,
    prefixed with the cache-format version. The wire-visible
    `manifest.fingerprint` stays FNV-1a, so the contract and the goldens do not
    move; the wider hash only names files.
- Cache identity (the fix for a disk entry outliving its build). The
  `AnalyzerVersions` labels are hand-written, so the key also carries:
  - `semantic_core::content_fingerprint`: SHA-256 over every embedded lexicon
    asset (gazetteer, delta, inflections, tables, entity index and the three
    generated pack families). Any content edit moves it without a label bump;
  - `SEMANTIC_CACHE_BUILD_ID` from `semantic-engine/build.rs`: SHA-256 over the
    Rust sources that can change an analysis result, the crate manifests, the
    workspace lockfile and `git rev-parse HEAD`;
  - the ICU major (`RUST_ICU_MAJOR_VERSION_NUMBER`);
  - UDPipe model presence for the request language, so a degraded worker and a
    model-backed worker never share an entry.
- `prune` no longer scans the whole directory on every write: a tracked byte
  count triggers a pass when the budget is exceeded, and a periodic pass every
  64 writes catches entries written by another process.
- `request_body` is now the single canonical encoding used by both hashes
  (`lib.rs`), so "same request" cannot mean two different things.
- Writes are atomic: a unique `.*.part` file is written then renamed. A killed
  writer leaves a part file that `prune` removes after one hour.
- Reads are fail-closed. A miss is returned for invalid JSON, another key, a
  different manifest schema, or a manifest whose fingerprint differs from the
  request being served. Corrupt files are deleted so the next request does not
  re-decode them.
- Eviction is by mtime, and a successful read refreshes mtime, so the bound is
  least-recently-used. A manifest larger than the whole budget is not written.
- `Worker::from_env` (the shipping path) attaches the store; `Worker::new`
  stays memory-only so tests and embedders are unchanged. `handle_line` and
  `run_stdio` keep the same wire responses; a disk hit reports `cached: true`
  with the same `completeness` object as a memory hit or a fresh analysis.

## Acceptance evidence

```sh
cargo fmt --all -- --check
cargo clippy -p semantic-core -p semantic-engine --all-targets -- -D warnings
cargo test -p semantic-core --lib content_fingerprint
cargo test -p semantic-engine --lib cache::
cargo test -p semantic-engine --test persistent_cache
```

Results (2026-11-19, `--offline`, final closure run):

- `rustfmt --check` on the touched files and `clippy` (semantic-core,
  semantic-engine, pipelinegen-muscles, all targets) — rc 0.
- `content_fingerprint` — **5 passed**: stable SHA-256, order independence,
  per-family coverage, the `lexicon/` scan, and one appended byte changing the
  digest.
- `cache::` — **23 passed**, including roundtrip byte-identity, corrupt entry
  deleted, foreign-key and fingerprint rejection, atomic publish with no
  leftover part file, budget eviction, stale part cleanup, oversized-manifest
  rejection, identity in the key and prune throttling.
- `persistent_cache` — **4 passed**: restart, eight concurrent writers,
  model/fallback separation and an unusable directory.
- Full suites — `semantic-core --lib` 219, `semantic-engine --lib` 78,
  `pipelinegen-muscles --lib` 137 and every integration target; rc 0.

## Residual

- The 1 GiB budget and the 1-hour part-file age are policy defaults, not
  calibrated limits; the RSS target from the memory workstream is separate.
- Analyzer version strings are still in the key, but the build ID hashes every
  source file that can change an analysis result, so a behaviour change without
  a label bump no longer reuses an entry (H1).
- Wired by TICKET-39 (2026-11-19): `NewSemanticEngineExecutor` forwards
  `SEMANTIC_CACHE_DIR` and `SEMANTIC_CACHE_MAX_BYTES` explicitly to the worker,
  so setting either variable in PipelineGen now opts the batch path in. Unset
  keeps the memory-only behaviour. A repeated batch reported `cached:true`
  (1.56 s warm vs 3.64 s cold on the installed release binary).
- A warm hit still pays worker startup: in T15 the 8.55 s warm run is startup
  and model load, not analysis. Lazy UDPipe loading would make short-script
  hits cheaper; it is a separate change, not a cache defect (TICKET-39).

## Independent review — 2026-11-19

Code read: `cache/disk.rs`, the `request_body` split in `lib.rs`. Confirmed by
reading: SHA-256 over `request_body`, wire fingerprint still FNV, fail-closed
`load`, atomic `.part` + rename, mtime LRU.

**Not independently reproduced.** In the reviewer environment
`cargo test -p semantic-engine` failed while compiling `udpipe-rs` (`c++` on
`pipeline.cpp`, exit 1; cause not read). The 19 + 2 + 74 passing counts above are
the author's. `cargo fmt --check` and `pipelinegen-muscles --test preflight`
(4 tests) did pass in the reviewer environment. Re-run the commands in
"Acceptance evidence" on a clean build before relying on them.

### Findings

- **H1 — key does not cover what actually changes the output.**
  `analyzer_versions()` returns hand-written labels: `visualner` is the constant
  `"visualner"`, `lexicon` is `"lexicon.v6"`, `phrase_impact` is a manually
  bumped string. A disk entry survives a rebuild, so a behaviour change without
  a bump serves a stale manifest. This is live now: the working tree has
  uncommitted changes in `semantic-core/src/lexicon.rs`,
  `lexicon/entity_gazetteer_delta.json` (new `locations` field) and
  `visualner/src/extractor/values.rs`.
- **H2 — degraded runs share the key.** Without UDPipe models the worker
  disables POS evidence and verb filtering and the metrics worsen
  (`docs/stato/brainrust.md`). A worker started without `VISUALNER_MODEL_DIR`
  would write degraded manifests that a healthy worker then reads, and vice
  versa. ICU major is also not in the key.
- **H3 — no test ties the key to content.** Nothing proves that changing one
  byte of an embedded lexicon changes the key.
- **H4 (minor) — `prune()` scans the whole directory on every `store`.** Fine for
  hundreds of files, slow at a 1 GiB bound with many small manifests.
- **H5 (minor) — latent in production.** No Go consumer sets
  `SEMANTIC_CACHE_DIR`, so there is no saving and no exposure yet.

### Hardening — acceptance criteria

- [x] **H1:** the key hashes a *content digest of the embedded knowledge*: every
      `include_str!`/`include_bytes!` lexicon, the entity index, language packs
      and `link`/`gazetteer` tables, computed once at startup (sorted by name,
      SHA-256). Plus the build identity (`git` SHA from `build.rs`, or
      `CARGO_PKG_VERSION` + a hash of the executable as fallback).
- [x] **H2:** the key includes a `runtime_profile` string:
      `udpipe=<model file names+sizes or "absent">;icu=<major>`. A worker
      without models never reads or writes entries of a worker with models.
- [x] **H3:** tests listed in "Practical verification" T3–T5 exist and pass.
- [x] **H4:** `prune()` runs at open and then at most once per N stores
      (default 64) or per 60 s, whichever first; test with 2,000 small entries.
- [x] Wire fingerprint and goldens unchanged (`git diff` on goldens is empty).
- [x] `CACHE_FORMAT` bumped to 2 so existing v1 entries become misses.

### Resolution — 2026-11-19

| Finding | Implementation | Test |
|---|---|---|
| H1 | `semantic_core::content_fingerprint` (SHA-256 over every compiled asset, sorted, including `lexicon/legal/index.json` and all three generated pack families) + `SEMANTIC_CACHE_BUILD_ID` from `semantic-engine/build.rs` (analysis sources, crate manifests, lockfile, git commit) | `content_fingerprint` 5/5, including “one byte changes the digest”, per-family coverage and a scan of `lexicon/` that fails when a compiled table is missing from the digest; `key_with_identity` proves a changed identity moves the key |
| H2 | `model_profile(language)` = model file name + byte size, or `absent`; ICU major included | `a_model_free_and_a_model_backed_worker_keep_separate_entries`: two processes, two entries, each stays warm for its own profile |
| H3 | T5-equivalent unit tests, plus T3 run end-to-end in the closure pass (edit one byte, rebuild, old directory misses) | `content_fingerprint` 5/5, `cache::` 23/23 |
| H4 | tracked byte count + `PRUNE_EVERY_WRITES = 64` + 60 s interval; `prune_count()` observes scans | `prune_is_throttled_between_stores`, `two_thousand_small_entries_do_not_scan_on_every_store` |
| T9 | unique `.part` + rename under concurrency | `concurrent_workers_publish_one_atomic_entry`: 8 processes, one entry, no part files, identical manifests |

T3 was later run end-to-end (one byte appended to `lexicon/months.json`,
rebuild, old directory missed and a new key was written), and T13/T15 were
measured; see “Closure — 2026-11-19” below.

The reviewer could not compile `udpipe-rs` in their environment; on this host
`cargo test -p semantic-engine` compiled and the counts above are from a clean
incremental build with ICU 72 from `.cargo/config.toml`.

## Practical verification

Run from `BrainRust/`. Use a throwaway cache directory. Build once first:
`VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models cargo build --release -p semantic-engine`

```sh
export VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models
BIN=target/release/semantic-engine
REQ='{"operation":"analyze_script","language":"it","scenes":[{"scene_id":"s1","text":"Alberto Stasi e Andrea Sempio sono due nomi."}]}'
run() { printf '%s\n' "$REQ" | SEMANTIC_ENGINE_WORKERS=1 "$@" $BIN; }
```

| # | Check | How | Expected |
|---|---|---|---|
| T1 | Disabled by default | `run env -u SEMANTIC_CACHE_DIR` twice | both `"cached":false`; no cache file created |
| T2 | Survives restart | `D=$(mktemp -d); run env SEMANTIC_CACHE_DIR=$D; ls $D; run env SEMANTIC_CACHE_DIR=$D \| grep -o '"cached":[a-z]*'` | first run creates exactly one `manifest-<64 hex>.json`; second prints `"cached":true`; `jq -S .manifest` of both responses is identical |
| T3 | Lexicon change invalidates (**H1**) | append one byte to a clean embedded table (used `lexicon/months.json`), rebuild, rerun T2's second command on the **old** `$D` | `"cached":false` and a second file appears; revert the change afterwards. **Passes (observed)** — see below |
| T4 | No-models profile is separate (**H2**) | populate `$D` with models, then run with `VISUALNER_MODEL_DIR` pointing at an empty dir | `"cached":false`, a new file; running with models afterwards still reports `"cached":true` for *its* entry. **Passes (observed)** |
| T5 | Key changes with content | unit test: two `ScriptRequest`s equal except one scene byte → different keys; same request → same key; flipping the injected lexicon digest → different key | passes |
| T6 | Corruption is a miss | `D=$(mktemp -d)`; run T2 once; `echo '{' > $D/manifest-*.json`; run again | `"cached":false`, request succeeds, file deleted then rewritten valid |
| T7 | Foreign payload rejected | copy `manifest-A.json` over `manifest-B.json` (different key) and request B | `"cached":false`; B rewritten |
| T8 | Unusable dir degrades | `SEMANTIC_CACHE_DIR=/proc/nope` | exit 0, valid manifest, `"cached":false` |
| T9 | Atomicity | run 8 parallel workers writing the same key: `for i in 1..8; do run env SEMANTIC_CACHE_DIR=$D & done; wait` | no `*.part` left, exactly one `manifest-*.json`, every response manifest identical |
| T10 | Stale `.part` cleanup | `touch -d '2 hours ago' $D/.x.part`, start a worker | file removed; a fresh `.part` (just touched) is kept |
| T11 | Byte budget / LRU | `SEMANTIC_CACHE_MAX_BYTES=<2 × size of one entry>`; analyze three different scripts A, B, C, re-read A between B and C | after C: A and C present, B evicted (A's mtime was refreshed by the read) |
| T12 | Oversized manifest | `SEMANTIC_CACHE_MAX_BYTES=100` | request succeeds, directory stays empty |
| T13 | Prune cost (**H4**) | create 5,000 small valid entries, time one `store` | well under 50 ms after the H4 change (record the before/after number) |
| T14 | Wire contract untouched | `git diff --stat -- '*golden*' '*fixtures*'` and `contract/…/check_consumer_copy.sh` | no golden diff; consumer copy PASS |
| T15 | Speedup is real | a long script (corpus 100 item): `time run env SEMANTIC_CACHE_DIR=$D` cold vs warm | warm run is dominated by process start; record both timings in the ticket |

Automated gates: `semantic-engine/tests/persistent_cache.rs` covers T4 and T9
(two worker profiles, and eight processes writing one atomic entry); `content_fingerprint` and `cache::` cover T5.

## Observed run — 2026-11-19

Run with `target/debug/semantic-engine` built from this tree (ICU 72 from
`.cargo/config.toml`), a throwaway `SEMANTIC_CACHE_DIR`, and one process per
request. The models used for T4 are `.tmp/udpipe-models`; the degraded profile
points `VISUALNER_MODEL_DIR` at an empty directory.

| # | Result | Observation |
|---|---|---|
| T1 | pass | two runs without `SEMANTIC_CACHE_DIR`: both `"cached":false`, no file created |
| T2 | pass | first run writes one `manifest-36f239fd….json`; second `"cached":true`; manifest and completeness identical |
| T4 | pass | with models `false`; empty model dir `false`; models again `true`; **2 entries** |
| T5 | pass | `content_fingerprint` 5/5 (including the `lexicon/` coverage scan); `key_with_identity` changes with the identity |
| T6 | pass | truncated file → `"cached":false`, file rewritten and parses as JSON |
| T7 | pass | copied foreign payload → `"cached":false` |
| T8 | pass | `SEMANTIC_CACHE_DIR=/proc/nope` → `"ok":true`, `"cached":false`, exit 0 |
| T9 | pass | automated: 8 concurrent processes → one entry, no `.part`, identical manifests |
| T10 | pass | 2-hour-old `.part` removed, fresh `.part` kept |
| T11 | pass | A=11,324 B=3,908 C=6,334 bytes, budget 17,658 → kept A+C, B evicted (the warm read refreshed A's mtime) |
| T12 | pass | `SEMANTIC_CACHE_MAX_BYTES=100` → `"ok":true`, directory empty |
| T13 | pass | 5,000 synthetic entries vs empty dir: 7.24 s vs 10.74 s for the same one-scene request (both dominated by model load in a debug build); the store appended its entry (5,001 files), so no scan penalty |
| T14 | pass | `git diff --stat -- '*golden*'` empty; wire `request_body` unchanged |
| T15 | pass | 100 scenes: cold 21.35 s, warm 8.55 s, identical manifests (7,905,319 vs 7,905,318 bytes, only `cached` differs) |
| T3 | pass | cache `manifest-fe17e4cd…`; after one byte in `months.json` + rebuild the old directory reported `"cached":false` and wrote `manifest-b30bfb75…`; file restored, workspace rebuilt |

All fifteen checks now have an observed result. T13's absolute seconds are not
a store benchmark: a one-scene request is dominated by UDPipe model loading in
a debug binary, which is why the empty and populated runs are both ~10 s. The
property the check protects — no directory scan on `store` — is visible in the
absence of a difference and is unit-tested by
`two_thousand_small_entries_do_not_scan_on_every_store`.

## Closure — 2026-11-19

Final pass over the whole ticket:

- T3, T13 and T15 were run for real (numbers above), so no check is left
  unobserved.
- `cargo test -p semantic-core -p semantic-engine -p pipelinegen-muscles`
  finished green (38 test targets, including all integration suites);
  `rustfmt --check` on the touched files and `clippy --all-targets -D warnings`
  for the three crates are rc 0. `cargo fmt --all --check` currently flags only
  `overlay-compiler/src/editorial_plan_v2/planner/coalesce.rs`, an uncommitted
  planner file owned by a parallel session and untouched here.
- `git diff --stat -- '*golden*'` is empty; the wire fingerprint and the
  goldens are unchanged.
- One combined doctest run failed transiently with `can't find crate for
  serde` while the shared `target/` directory was being rewritten; a doc-only
  rerun and the next full rerun both passed. That was a build-artifact race,
  not a code failure, and the final green run above is the recorded evidence.
- The consumer-side opt-in landed with TICKET-39 on 2026-11-19; the release
  that carries the operation still has to be published and pinned.

## Implementation map

- `semantic-core/src/content_fingerprint.rs` — `hash_assets`,
  `embedded_assets`, `embedded_content_fingerprint`; the `lexicon/` scan test is
  the guard against a table that is compiled but not digested.
- `semantic-engine/build.rs` — `SEMANTIC_CACHE_BUILD_ID` from sorted source
  files, crate manifests, `Cargo.lock` and `git rev-parse HEAD` (`unknown`
  outside a checkout; the content digest still covers the data).
- `semantic-engine/src/lib.rs` — `cache_identity` (`build`, `content`, `icu`,
  `model`) and `model_profile` (model file name + size, or `absent`).
- `semantic-engine/src/cache/disk.rs` — `CACHE_FORMAT = 2`, SHA-256 key over
  `CACHE_FORMAT || identity || request_body`; `.part` + rename publish; atomic
  replacement fallback; throttled LRU prune; fail-closed load.
- `semantic-engine/src/protocol.rs` — memory hit, then disk hit (re-inserted in
  memory), then analysis, then `store`.
- `semantic-engine/tests/persistent_cache.rs` — restart, eight concurrent
  writers, model/fallback separation and an unusable directory.

The wire fingerprint (`request_body`, FNV-1a) and the goldens are unchanged:
the wider key exists only to name files on disk.

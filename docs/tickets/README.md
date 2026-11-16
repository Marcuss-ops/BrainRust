# BrainRust audit tickets — 2026-11-15

Canonical status for the 2026-11-15 architecture audit and its roadmap.
Detailed acceptance evidence lives in each ticket. This file is the only status
matrix; `ACCEPTANCE_TODO.md` remains the historical gate evidence.

Last verified: 2026-11-16. **16 DONE, 0 PARTIAL, 2 OPEN (18, 20), 2 BLOCKED.**

Tickets 15–20 are the 2026-11-16 external-audit follow-ups. 15 (phrase
selection), 16 (MediaMemory ranker), 17 (visual budget/chain) and 19
(phrase-timing alignment) are DONE; 18 (linguistics) and 20 (identity) remain
OPEN. The audit reconciliation is
[`docs/stato/AUDIT-RICONCILIAZIONE-2026-11-16.md`](../../../docs/stato/AUDIT-RICONCILIAZIONE-2026-11-16.md).

Status vocabulary:

- **DONE** — implemented and covered by the referenced checks.
- **DONE (bound/Rust)** — the local contract is complete; a named external
  parity/benchmark gate remains outside this checkout.
- **BLOCKED** — cannot be completed inside this repository without an external
  owner, dataset, credential or cross-repo contract change.
- **PARTIAL** — some implementation or evidence exists, but acceptance criteria
  remain open.
- **OPEN** — scoped and ready; implementation has not started.

| Ticket | Priority | Status | Outcome |
| --- | --- | --- | --- |
| [01](TICKET-01-ndjson-backpressure.md) | P0 | DONE | End-to-end NDJSON backpressure and panic-safe responses. |
| [02](TICKET-02-imagecatalog-policy.md) | P0 | DONE | Fail-closed catalog policy, snapshot integrity, real dates, `evaluated_at`. The Go policy now agrees on both audited cases: an unrecognized status classifies `unknown` and `retired` never re-enters the recertification pool (verified 2026-11-16). |
| [03](TICKET-03-artifact-publish.md) | P1 | DONE | Unique `.part` files, atomic publish, newest-generation guard. |
| [04](TICKET-04-render-stock-certification.md) | P1 | DONE | `render_stock` probes and certifies the final file before publishing; canonical render integration is 4/4. |
| [05](TICKET-05-manifest-completeness.md) | P1 | DONE | Response-level `completeness` with rejected scene count and reasons. |
| [06](TICKET-06-cache-byte-budget.md) | P1 | DONE (bound + measured) | Entry cap plus 128 MiB serialized-byte budget; 20-script replay peak RSS measured at 1,418,300 KiB. |
| [07](TICKET-07-manifest-reuse.md) | P1 | DONE | Optional `semantic_manifest.v2` on the worker request; same type, validated and reused without a second analysis. No Go consumer exists to wire: `imagecatalog` stays offline by the 13b decision. |
| [08](TICKET-08-editorial-gold-set.md) | P1 | BLOCKED | Requires independent human annotations; no such dataset ships here. |
| [09](TICKET-09-hotspot-contraction.md) | P2 | DONE | Source preparation extracted; duplicate cache fingerprint removed; remaining splits would be cosmetic. |
| [10](TICKET-10-multimodal-pgvector.md) | P2 | BLOCKED | Requires SigLIP/pgvector, a schema and a calibrated held-out dataset. |
| [11](TICKET-11-discourse-continuity.md) | P0 | DONE (deterministic core) | Additive `discourse.v1` references, ambiguity, threads and scene continuity. |
| [12](TICKET-12-mvp-semantic-contract.md) | P0 | DONE (bound/Rust) | v3 linked-entity projection, local index/linker, typed intents, Go v3 consumer, intent/index cache key, verified-hit admission, stop-on-satisfaction and cross-repo smoke pass; only the revised SLO agreement remains external. |
| [13](TICKET-13-go-demolition-closeout.md) | P1 | DONE | 13a DONE (`b6f5379` + `c4c12ac98`): the linker owns NER repair, prompt-echo rejection and image admission; annotations project the decision. 13b DONE (`8b4482660`): verified assets owned by the Go `entitycatalog`, Rust `imagecatalog` remains offline by design. 13c DONE and pushed. 13d DONE 2026-11-16: the four per-concern ports and `legacy_overlay_plan.go` were deleted; production uses one `CompileOverlayPlanV2`. WP5/WP7 closed 2026-11-16: six-fixture production-adapter gate, one-request real-worker counter, deletion guard and `make verify-agent` / `make verify-main` PASS. The producer no longer calls `projectEditorialOverlays`/`projectChapterBullets`; they remain test-only helpers. the request-side surface builder was removed from production on 2026-11-16; TICKET-14 closed 2026-11-16, so 13d and 13 are DONE. |
| [14](TICKET-14-overlay-planner-single-call.md) | P1 | DONE | Rust single-call compiler landed: complete `editorial_plan.v2` request (seal, annotations, entity timeline, media, budgets, windows), final-plan response with `validate_for`, and one `compile_overlay_plan_v2` call that projects editorial/chapter bullets, entity cards, budgets, map trim/resolution, cascade and seal. Eight request fixtures plus eight planner-generated goldens and a 30-test contract suite are checked in; the worker routes by schema version before deserializing. Entity-card motion/style selection is ported and cards now merge after the editorial admission. Go cutover LANDED 2026-11-16: one `CompileOverlayPlanV2` in production, sealed-plan adoption, legacy planner and four ports deleted, captioned-card chain fusion moved to Rust, chart placements computed in Rust. WP5/WP7 closed 2026-11-16: the stale Go-vs-Rust differential was replaced by an eight-fixture production-adapter gate, the real-worker one-request counter and a deletion guard both PASS, and `make verify-agent` / `make verify-main` PASS. WP4 request-side surface projection DONE 2026-11-16: raw phrases/entities/numbers/dates/maps/charts/heavy/categories are projected in Rust; `overlaySceneInput` no longer has a production caller. All parity lanes closed 2026-11-16: general phrase/image/mixed chains, presentation/light leaks, seeded dates, verified intent images and recap. Go only verifies/materializes media bytes. TICKET-13 13d is DONE. |
| [15](TICKET-15-phrase-engine-demolition.md) | P1 | DONE | `SceneIRSegmentEnricher`, translated NLP and the overlay projection use `select_phrases`; `scripts/phrases/` is deleted and the affected Go/Rust suites pass. No production file imports/calls the retired Go selector or `locatePhraseTimingWithEndpointFallback`; the endpoint helper survives only as a test oracle. The pinned release `v0.3.5` predates `select_phrases`/`align_phrases`, so a release and pin bump are still required for deployment. |
| [16](TICKET-16-mediamemory-ranker-migration.md) | P1 | DONE | `mediasampler::rank` owns gates, weights, both penalty seats, duration fit, stable order and the diversity pick. `rustexec.MediaRankerAdapter` sends one `rank_candidates` call per slot pool; Go `resolver_orchestration.go` projects the result and does not re-sort. `PickTopFromRose` has no production caller; Go and Rust parity suites pass. A release/pin bump is still needed for deployment. |
| [17](TICKET-17-visual-budget-chain-cutover.md) | P1 | DONE (2026-11-16) | Captioned-card and general chain fusion moved to Rust and the Go chain producer plus its tests were deleted; `model/chain_coalesce.go` is wire-only. The Rust `admit` pass owns production admission after ranking and before seal; no production Go file calls the legacy budget producers, and the WP7 guard pins them. The legacy `BuildPlan`/visual-budget helpers remain only as a Go test oracle. Eight fixtures / 30 contract tests, real-worker 8/8, and `make verify-main` PASS. |
| [18](TICKET-18-linguistics-go-demolition.md) | P2 | OPEN (next dedicated block) | Go still loads `LexiconRegistry` and applies language-aware entity rules (`translated_nlp_identity.go` 571, `entity_annotations.go` 561, `entity_projection.go` 457). Target: Rust owns lexicons, morphology and projection. |
| [19](TICKET-19-linguistic-temporal-alignment.md) | P2 | DONE | `semantic-core::phrase_align` + `align_phrases` + `SemanticEngineAdapter.AlignPhrases` own exact phrase alignment; `CompilePhraseTimings`/`CompileSceneSpeechTimings` project the worker result. The endpoint fallback has no production caller. `audio.LocatePhrase` remains for entity/moment projection as a separate follow-up. |
| [20](TICKET-20-identity-localization-consolidation.md) | P2 | OPEN (next dedicated block) | Localized identity matching/inflection remains Go (`translated_nlp_identity.go`). Target: Rust carries the source identity through translation; Go validates the verbatim surface. |

## Verification — 2026-11-16 (workspace gate, Rust ↔ Go boundary)

Run on the `BrainRust` + `refactored` working trees. ICU comes from
`.cargo/config.toml` (72): the `RUST_ICU_MAJOR_VERSION_NUMBER=70` line in older
logs is stale and now fails to link, because this host's unversioned
`libicuuc.so` points at ICU 70 while `.cargo/icu72-link` redirects to `.so.72`.

- `cargo fmt --all -- --check` — rc 0 after formatting the new
  `semantic-core/src/phrase_select.rs`.
- `cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib` —
  64 + 161 + 54 passed.
- `cargo test -p overlay-compiler --test overlay_plan_contract` — 30 passed.
- `cargo test -p semantic-engine --test editorial_plan_protocol` — 6 passed.
- `cargo test -p semantic-engine --test phrase_selection_protocol` — 13 passed
  (12 in process plus the real-worker one-request path).
- `cargo clippy --workspace --all-targets -- -D warnings` — rc 0 after removing
  the `chars_last_cmp` lint in `phrase_select.rs`.
- `go test ./internal/capabilities/scripts/... ./internal/platform/media/rustexec/... -count=1`
  — every package passes (`rustexec` 81.982 s).
- `go test ./internal/capabilities/images/entitycatalog/... ./internal/capabilities/images/... -count=1`
  — pass (ticket 02 Go policy).
- `make verify-agent` — PASS; every changed component ran (not cached) and the
  `rust` (91.5 s) and `script` (32.9 s) legs were green.
- `make verify-main` — PASS (foundation, static, unit, changed components,
  archcheck/C2 and `CLIP_RENDER_CUTOVER=PASS`).
- `bash contract/mediaexec.v2/check_consumer_copy.sh refactored/testdata/mediaexec/v2`
  — PASS, consumer goldens match.
- `python3 lexicon/tools/check_go_stopword_parity.py` — PASS, 0 drift on the six
  shared languages.

## Verification — 2026-11-16 (second pass, phrase selector, alignment and ranker)

Run on the current working trees (`refactored@c8cae4e8a` plus uncommitted
changes, `BrainRust@01f01e1` plus the uncommitted phrase/alignment work).

- `go build ./internal/capabilities/scripts/... ./internal/app/wiring/...` —
  rc 0.
- production imports of `capabilities/scripts/phrases` — **0**;
  `phrase_supplement.go` is deleted.
- `go test ./internal/capabilities/scripts -count=1` — **PASS**, 15.723 s. The
  two tests that failed in the previous pass now pass without weakened
  assertions.
- `go test ./internal/capabilities/audio ./internal/platform/media/rustexec -count=1`
  — **PASS**, 0.026 s + 99.322 s.
- `go test ./internal/capabilities/mediamemory/... ./internal/platform/media/rustexec/... -count=1`
  — **PASS**, 0.043 s + 85.019 s.
- `cargo test -p mediasampler` — **42 lib + 7 bin + 4 parity** pass.
- `cargo test -p semantic-core phrase_align --lib` — **8/8** pass;
  `cargo test -p semantic-engine align_phrases --lib` — **1/1** pass;
  `cargo test -p semantic-engine --test phrase_selection_protocol` — **13/13**
  pass.
- Production caller audit: `PickTopFromRose`, `PopulateRepetitionPenalty`,
  `buildFilterFlags`, `buildRankingInput` and `sortByFinalScoreDesc` have 0
  production callers; the overlay projection helpers that reach the endpoint
  fallback (`overlaySceneInput`, `projectChapterBullets`,
  `projectEditorialOverlays`) also have 0 production callers.
- `select_phrases`, `align_phrases` and `rank_candidates` are absent from
  `v0.3.5`, while `refactored/make/build.mk` pins
  `BRAINRUST_VERSION ?= v0.3.5`. A release and pin bump are still required
  before production runs the switched callers; this is a deployment
  prerequisite, not an open code path.

No test failure remains in the 15/16/19 scopes. 18 and 20 stay OPEN because
their Go authorities (lexicon/morphology and localized identity) have not been
ported.

## Verification — 2026-11-15 (ticket 14, Rust single-call compiler)

- `cargo fmt --all -- --check` — rc 0.
- `cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib`
  — 39 + 151 + 53 passed.
- `cargo test -p overlay-compiler --test overlay_plan_contract` — 28 passed:
  frozen-golden byte comparison, determinism, verified/text-only entity media,
  entity-card motion rotation and caption/style stamping, missing-pool and
  uncertified-selector rejection, render-key clearing, map trim and final
  resolution, chart and chapter-bullet admission, localized surfaces, malformed
  and out-of-window rejection.
- `cargo test -p semantic-engine --test editorial_plan_protocol` — 6 passed:
  v2 routed to the full compiler, unknown version rejected before
  deserialization, malformed v2 fails closed, legacy batch unchanged.
- `cargo clippy --workspace --all-targets -- -D warnings` — rc 0.
- `gofmt -l` on the touched Go packages — empty; `go vet` — rc 0;
  `TestBuildOverlayPlanV2Request` and
  `TestCompileEditorialPlan|TestOverlayPortsUseTheSingleBatchOperation` — pass.
- Fixtures are produced by `overlay-compiler/examples/compile_fixture`, so the
  plan goldens come from the real planner and not from a hand-written document.

## Verification — 2026-11-15 (ticket 12, Rust core)

- `cargo test -p semantic-core --lib` — 150 passed.
- `cargo test -p semantic-engine --test mvp_semantic_contract` — 7 passed
  (T1–T7).
- `cargo test -p semantic-engine --test protocol_validation` — 4 passed.
- `cargo test -p semantic-core -p semantic-engine -p visualner -p imagecatalog`
  — all passed.
- `cargo fmt --all -- --check` and clippy on the touched crates — clean.
- Release probe (20 scenes): with UDPipe p50/p95/p99 = 1.438/1.503/1.551 s;
  model-free 77.9/88.7/98.6 ms.
- No-network: the release binary analyzed the ESA/NASA fixture inside
  `unshare -n` and returned `ok:true`.
- RSS: 1,000 analyses plateaued at 228,264 KiB (0 KiB growth across the last
  three samples).

## Verification — 2026-11-15 (ticket 13c, Go audio timing)

- `go vet ./internal/capabilities/audio ./internal/capabilities/scripts/...`
  — exit 0.
- `go test ./internal/capabilities/audio ./internal/capabilities/scripts/...
  -count=1` — all packages pass.
- The 40 ms AAC padding tolerance now has one Go definition; the literals in
  the final-audio validator, master invariants and final-job payload/plan are
  gone. The Go MP3 frame-duration parser was deleted: synthesized audio
  duration comes only from the shared media executor probe.

## Verification — 2026-11-15 (ticket 13 cleanup)

- `make verify-main` — PASS (foundation, static, full Go tests, changed
  components `final=PASS`, archcheck/C2, `CLIP_RENDER_CUTOVER=PASS`).
- `make verify-agent` — PASS (`final: PASS`); every component PASS or
  cache-hit, including `script`, `rust` and `timeline`.
- `go test ./internal/capabilities/scripts/... -count=1` — all packages pass.
- `go test ./internal/platform/media/rustexec/... -count=1` — pass (74s);
  `TestSemanticEngineRealBinaryProjectsLinkedEntities` — pass (2.94s).
- `gofmt -l .` and `git diff --check` — clean.
- `cargo fmt --all -- --check` — clean.
- `cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib`
  — pass (8 + 151 + 53 tests).

## Verification — 2026-11-15 (13d v2 contract)

- `cargo test -p overlay-compiler --lib` — 38 passed, including the 8
  chapter-bullet parity cases, the editorial projection, the first
  single-call planner slice and the v2 request/response validators.
- `cargo test -p overlay-compiler --test overlay_plan_contract` — 23 passed
  (frozen goldens, fixture matrix, determinism, fail-closed schemas and
  map-trim-before-budget ordering).
- `cargo test -p semantic-engine --lib` — 53 passed; the worker dispatches
  `editorial_plan.v2` to `compile_overlay_plan_v2` and never parses it as the
  sparse legacy batch.
- `cargo fmt --all -- --check` and
  `cargo clippy -p overlay-compiler -p semantic-engine --all-targets -- -D warnings`
  — clean with ICU 72.
- `go test ./internal/platform/media/rustexec -count=1` — pass (83.4 s);
  `TestCompileEditorialPlanV2ReturnsTheChapterProjection` and the
  out-of-window rejection pass, and the adapter validates the projection
  without repairing it.
- `go test ./internal/capabilities/scripts -run 'TestBuildOverlayPlanV2Request|TestProjectChapterBullets|TestChapterBullet|TestProjectEditorialOverlays' -count=1`
  — pass; the producer now carries certified phrase annotations.
- `go vet ./internal/platform/media/rustexec ./internal/capabilities/scripts`
  — clean.
- `python3 BrainRust/lexicon/tools/check_go_stopword_parity.py` — PASS, 0
  drift on the six shared languages (de/en/es/fr/it/pt).

The full `go test ./internal/capabilities/scripts -count=1` suite still fails
on `TestBuildFinalJobPayloadsUsesLocallyRenderedStockVisual`
(`unknown stock fixture "obama-stock-db-clip"`), a concurrent final-job
fixture issue unrelated to the v2 compiler work.

## Verification — 2026-11-15 (TICKET-14 parity harness)

- `go test ./internal/capabilities/scripts -run TestOverlayPlanV2ParityFixtureShape -count=1`
  — PASS; the `minimal_phrase_only` request/golden pair is readable and
  internally consistent.
- `OVERLAY_PLAN_PARITY=1 go test ./internal/capabilities/scripts -run TestOverlayPlanV2ParityMinimalPhraseOnly -count=1 -v`
  — runs the differential and currently reports Go 2 items vs Rust 3, plus
  item/role/window differences. This is the cutover gate, not a green parity
  claim.

## Verification — 2026-11-15 (ticket 12, Go consumer)

- Cache-key tests: intent, query class, provider, verifier and entity-index
  changes all invalidate the key; unlinked document-scoped identities do not
  collide.
- Scheduler tests: `bestVerifiedSemanticHit` rejects provider-order
  `Hits[0]`; a hit settles the composition and later rungs do not run.
- v3 projection tests: role, gloss, negative constraints and typed intents
  survive the Go boundary; an unlinked entity carrying a canonical id fails
  closed.
- Cross-repo: `TestSemanticEngineRealBinaryProjectsLinkedEntities` passes
  against the release BrainRust binary in 3.28 s with UDPipe models.
- The broad Go suite is green as of the ticket-13 cleanup above; the earlier
  `overlay_plan_*` signature churn has landed.

## Verification — 2026-11-11 (tickets 01–11)

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models RUST_ICU_MAJOR_VERSION_NUMBER=70
  cargo test --workspace -j 2` — **646 passed, 0 failed, 1 ignored** across 41
  test binaries. Log: `BrainRust/.tmp/discourse-final-tests.log`.
- `cargo build --release -p semantic-engine` — clean.
- `cargo test -p pipelinegen-muscles --test canonical_render_segments` — 4/4.
- `go test ./internal/capabilities/entities
  ./internal/capabilities/images/entitycatalog
  ./internal/capabilities/scripts/adapters -count=1` — pass in `refactored`.

The 646 count includes the prior audit suites plus the 14 discourse integration
tests and the language-scoped reference unit tests. `discourse.v1` is additive:
older manifests decode with an empty projection.

## External residuals

These do not reopen the tickets; they are owner gates outside this checkout:

- **06** — the 20-script run is a host observation (peak 1,418,300 KiB, P50
  0.403 s, P95 3.600 s), not a deployment SLO. The 128 MiB value remains a
  policy bound.
- **07** — no consumer to wire: the Rust `imagecatalog` crate is offline by the
  13b decision and the Go `entitycatalog` is the live registry, so the manifest
  saving stays latent. Adopting the crate would replace `entitycatalog`.
- **08/10** — human gold annotations and the multimodal model/schema/calibrated
  association set are still required.
- **12** — the revised measured SLO needs owner agreement. The Rust contract,
  local index/linker, typed intents, Go v3 consumer, intent/index cache key,
  verified-hit admission, stop-on-satisfaction, cross-repo smoke, no-network
  and RSS gates are complete in this checkout.

No extra platform or persistence layer was introduced. PipelineGen remains the
owner of canonical state and business policy.

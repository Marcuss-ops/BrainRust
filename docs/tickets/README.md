# BrainRust audit tickets — 2026-11-15

Canonical status for the 2026-11-15 architecture audit and its roadmap.
Detailed acceptance evidence lives in each ticket. This file is the only status
matrix; `ACCEPTANCE_TODO.md` remains the historical gate evidence.

Last verified: 2026-11-16. **10 DONE, 2 PARTIAL, 6 OPEN, 2 BLOCKED.**

Tickets 15–20 are the 2026-11-16 external-audit follow-ups. They cover the Go
authorities that remain outside TICKET-13's 13a–13e inventory; none of them is
DONE. The audit reconciliation is
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
| [02](TICKET-02-imagecatalog-policy.md) | P0 | DONE (Rust) | Fail-closed catalog policy, snapshot integrity, real dates, `evaluated_at`. Go parity fixture remains external. |
| [03](TICKET-03-artifact-publish.md) | P1 | DONE | Unique `.part` files, atomic publish, newest-generation guard. |
| [04](TICKET-04-render-stock-certification.md) | P1 | DONE | `render_stock` probes and certifies the final file before publishing; canonical render integration is 4/4. |
| [05](TICKET-05-manifest-completeness.md) | P1 | DONE | Response-level `completeness` with rejected scene count and reasons. |
| [06](TICKET-06-cache-byte-budget.md) | P1 | DONE (bound + measured) | Entry cap plus 128 MiB serialized-byte budget; 20-script replay peak RSS measured at 1,418,300 KiB. |
| [07](TICKET-07-manifest-reuse.md) | P1 | DONE | Optional `semantic_manifest.v2` on the worker request; same type, validated and reused without a second analysis. |
| [08](TICKET-08-editorial-gold-set.md) | P1 | BLOCKED | Requires independent human annotations; no such dataset ships here. |
| [09](TICKET-09-hotspot-contraction.md) | P2 | DONE | Source preparation extracted; duplicate cache fingerprint removed; remaining splits would be cosmetic. |
| [10](TICKET-10-multimodal-pgvector.md) | P2 | BLOCKED | Requires SigLIP/pgvector, a schema and a calibrated held-out dataset. |
| [11](TICKET-11-discourse-continuity.md) | P0 | DONE (deterministic core) | Additive `discourse.v1` references, ambiguity, threads and scene continuity. |
| [12](TICKET-12-mvp-semantic-contract.md) | P0 | DONE (bound/Rust) | v3 linked-entity projection, local index/linker, typed intents, Go v3 consumer, intent/index cache key, verified-hit admission, stop-on-satisfaction and cross-repo smoke pass; only the revised SLO agreement remains external. |
| [13](TICKET-13-go-demolition-closeout.md) | P1 | PARTIAL | 13a DONE (`b6f5379` + `c4c12ac98`): the linker owns NER repair, prompt-echo rejection and image admission; annotations project the decision. 13b DONE (`8b4482660`): verified assets owned by the Go `entitycatalog`, Rust `imagecatalog` remains offline by design. 13c DONE and pushed. 13d Rust op + Go batch ports DONE (`ded9f28`, `68ddb83f5`, `652c03217`): all four overlay ports use `compile_editorial_plan`, but the planner still calls them at four phases. `make verify-agent` and `make verify-main` PASS; the versioned `editorial_plan.v2` envelope + Go producer landed (2026-11-15), but the single-call planner port and deletion of Go editorial/card selection remain; tracked by [14](TICKET-14-overlay-planner-single-call.md). 13 remains PARTIAL until 14 is DONE. |
| [14](TICKET-14-overlay-planner-single-call.md) | P1 | PARTIAL | Rust single-call compiler landed: complete `editorial_plan.v2` request (seal, annotations, entity timeline, media, budgets, windows), final-plan response with `validate_for`, and one `compile_overlay_plan_v2` call that projects editorial/chapter bullets, entity cards, budgets, map trim/resolution, cascade and seal. Five request fixtures plus five planner-generated goldens and a 27-test contract suite are checked in; the worker routes by schema version before deserializing. Entity-card motion/style selection is ported (certified pool rotation by run offset, independent caption lane, below-caption style, `image_scale_in`) and the seal clears derived render keys for Go to recompute. Open: Go cutover (the four phase ports and Go editorial/card selection remain), Go-oracle parity, the one-request-per-plan counter and `make verify-main` on the cutover tree. Blocks TICKET-13 13d closure. |
| [15](TICKET-15-phrase-engine-demolition.md) | P1 | OPEN | Go still owns important-phrase selection and phrase timing (`phrases/` + `phrase_*.go`, 8 files / 1,846 LOC). Target: Rust returns selected spans and alignment; delete the Go selector, supplement and endpoint fallback. |
| [16](TICKET-16-mediamemory-ranker-migration.md) | P1 | OPEN | `mediamemory` still owns final asset ranking (`PickTopFromRose`, 29 production files / 5,069 LOC). Target: `mediasampler` receives candidates + history and returns the ordered selection; Go projects. |
| [17](TICKET-17-visual-budget-chain-cutover.md) | P1 | OPEN | Visual budget and chain coalescing are still Go (`overlays/visual_budget*`, `model/chain_coalesce.go` 732 LOC). Target: fold both into `compile_editorial_plan` v2 after TICKET-14. |
| [18](TICKET-18-linguistics-go-demolition.md) | P2 | OPEN | Go still loads `LexiconRegistry` and applies language-aware entity rules (`translated_nlp_identity.go` 571, `entity_annotations.go` 561, `entity_projection.go` 457). Target: Rust owns lexicons, morphology and projection. |
| [19](TICKET-19-linguistic-temporal-alignment.md) | P2 | OPEN | Phrase-to-word alignment remains Go (`audio/phrase_locator.go`, `scripts/phrase_timing.go` 340, endpoint fallback). Target: Rust returns exact local/global spans; Go only projects. |
| [20](TICKET-20-identity-localization-consolidation.md) | P2 | OPEN | Localized identity matching/inflection remains Go (`translated_nlp_identity.go`). Target: Rust carries the source identity through translation; Go validates the verbatim surface. |

## Verification — 2026-11-15 (ticket 14, Rust single-call compiler)

- `cargo fmt --all -- --check` — rc 0.
- `cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib`
  — 39 + 151 + 53 passed.
- `cargo test -p overlay-compiler --test overlay_plan_contract` — 27 passed:
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

- **02** — the Go owner still maps an unrecognized candidate status to `fresh`
  and still lets a verified materialization keep `retired` rows in the pool.
  Rust deliberately fails closed on both. The shared parity fixture stays
  blocked until PipelineGen changes those Go behaviors.
- **06** — the 20-script run is a host observation (peak 1,418,300 KiB, P50
  0.403 s, P95 3.600 s), not a deployment SLO. The 128 MiB value remains a
  policy bound.
- **07** — PipelineGen must send the optional manifest to realize the CPU
  saving; the Rust contract and wire tests are complete.
- **08/10** — human gold annotations and the multimodal model/schema/calibrated
  association set are still required.
- **12** — the revised measured SLO needs owner agreement. The Rust contract,
  local index/linker, typed intents, Go v3 consumer, intent/index cache key,
  verified-hit admission, stop-on-satisfaction, cross-repo smoke, no-network
  and RSS gates are complete in this checkout.

No extra platform or persistence layer was introduced. PipelineGen remains the
owner of canonical state and business policy.

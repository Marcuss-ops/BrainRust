# BrainRust audit tickets — 2026-11-11

Canonical status for the 2026-11-11 architecture audit and its ten-PR roadmap.
Detailed acceptance evidence lives in each ticket. This file is the only status
matrix; `ACCEPTANCE_TODO.md` remains the historical gate evidence.

Last verified: 2026-11-11. **9 DONE, 0 PARTIAL, 2 BLOCKED.**

Status vocabulary:

- **DONE** — implemented and covered by the referenced checks.
- **DONE (bound/Rust)** — the local contract is complete; a named external
  parity/benchmark gate remains outside this checkout.
- **BLOCKED** — cannot be completed inside this repository without an external
  owner, dataset, credential or cross-repo contract change.
- **PARTIAL** — not used by the final audit state.

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

## Verification — 2026-11-11

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models RUST_ICU_MAJOR_VERSION_NUMBER=70
  cargo test --workspace -j 2` — **645 passed, 0 failed, 1 ignored** across 41
  test binaries. Log: `BrainRust/.tmp/discourse-final-tests.log`.
- `cargo build --release -p semantic-engine` — clean.
- `cargo test -p pipelinegen-muscles --test canonical_render_segments` — 4/4.
- `go test ./internal/capabilities/entities
  ./internal/capabilities/images/entitycatalog
  ./internal/capabilities/scripts/adapters -count=1` — pass in `refactored`.

The 645 count includes the prior audit suites plus the 13 discourse integration
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

No extra platform or persistence layer was introduced. PipelineGen remains the
owner of canonical state and business policy.

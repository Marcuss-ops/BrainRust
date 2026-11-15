# TICKET 12 — BrainRust MVP semantic contract, entity linking and latency

- Priority: P0
- Status: DONE (bound/Rust)
- Source: 2026-11-15 MVP regression run and cross-repository architecture audit
- Owners: BrainRust semantic-engine / visualner; PipelineGen resolver owner
- Files: `semantic-core`, `visualner`, `semantic-engine`, PipelineGen manifest
  adapter, retrieval resolver, cache and verifier

## Goal

Make BrainRust a deterministic, network-free producer of stable entity
identities and short visual search intents, and make PipelineGen consume that
contract without reconstructing identity or rewriting queries. The release
gate must establish semantic correctness and steady-state performance from a
clean, reproducible deployment.

This ticket does not claim that a surface-derived slug is a globally resolved
identity. A global ID is emitted only when the local linker resolves an entity
to a versioned knowledge-base entry. Unresolved identities remain explicitly
document-scoped.

## Completion evidence — 2026-11-15 (Rust core and Go consumer)

- `semantic_manifest.v3` adds a script-level `entities` projection:
  `local_id`, collision-safe `surface_key`, nullable linked `canonical_id`,
  `link_status` (`linked`/`ambiguous`/`unlinked`), `canonical_name`,
  `entity_type`, `role` (`named_entity`/`visual_concept`),
  `disambiguation_gloss`, `negative_constraints`, verbatim `mentions` and
  ordered `search_intents` (`identity` first, then `contextual`). v2 manifests
  stay decodable: absent `entities` defaults to empty and absent
  `entity_index` defaults to an empty analyzer version; `imagecatalog` accepts
  both `semantic_manifest.v2` and `semantic_manifest.v3`.
- `IdentityRegistry` normalizes NFC and apostrophe variants (ASCII, U+2019,
  U+02BC, grave) for matching only; stored surfaces and byte offsets stay
  verbatim. `stable_surface_key` hashes the full typed key, so distinct
  surfaces that slug-collide cannot share a cache key.
- A packaged `entity_index.v1` seed (ESA space vs software, Nike sport vs
  statue, NASA, ANSA, CERN, SpaceX, Michael Jordan vs Michael B. Jordan)
  resolves aliases with deterministic sentence-local context. Ties return
  `ambiguous`; an alias outside the index returns `unlinked`.
- Typed intents are built in the same pass: at most three, identity first,
  no sentence punctuation, and contextual intents only from named co-mentions.
  Generic visual concepts never receive an identity intent.
- The MVP suite passes T1–T7. T8 is measured, not claimed at 25 ms: release
  p50/p95/p99 is **1.438 s / 1.503 s / 1.551 s** with UDPipe and **77.9 ms /
  88.7 ms / 98.6 ms** model-free for the 20-scene fixture.
- The no-network gate passes under an isolated network namespace (`unshare
  -n`): the real release binary analyzes the ESA/NASA fixture with zero
  outbound access. The 1,000-analysis RSS gate reports a stable **228,264
  KiB** plateau (0 KiB growth across the last three samples).
- `cargo test -p semantic-core --lib` — 150 passed. `cargo test -p
  semantic-engine --test mvp_semantic_contract` — 7 passed. `cargo test -p
  semantic-engine --test protocol_validation` — 4 passed. `cargo test -p
  semantic-core -p semantic-engine -p visualner -p imagecatalog` — all
  passed. `cargo fmt --all -- --check` and clippy on the touched crates are
  clean.

### Go consumer (WP5)

- `refactored`'s `rustexec` adapter accepts `semantic_manifest.v3` and projects
  `canonical_id`, `link_status`, `role`, `disambiguation_gloss`,
  `negative_constraints` and typed `search_intents` into
  `scriptgen.SemanticIdentity`. The dead v2 `queriesByIdentity` fallback was
  removed; the consumer is v3-only.
- The entity-image discovery cache key is v6:
  `canonical_id + normalized intent + query class + provider +
  verifier_version + entity_index`. Unlinked entities keep the document-scoped
  `local:<fingerprint>:<id>` identity. The analyzer index version is propagated
  `VisualEntity → ExtractedEntity → SegmentInsights`.
- The semantic search scheduler admits only `isVerifiedSemanticHit`
  (`accepted` + named verifier + SHA-256) and selects
  `bestVerifiedSemanticHit`; it never falls back to provider-order `Hits[0]`.
- Stop-on-satisfaction is owned by
  `SemanticProductionPlan.RecordSearchOutcome`: a hit settles the composition
  and no later rung runs.
- Evidence: the cache-key, best-hit, stop-on-satisfaction, v3 projection and
  ORG fail-closed tests pass; `TestSemanticEngineRealBinaryProjectsLinkedEntities`
  passes against the release BrainRust binary (3.28 s with UDPipe models).
- The full Go suite is currently red on unrelated in-flight `overlay_plan_*`
  signature changes; those files are outside the WP5 diff.

## Work packages and implementation order

### WP0 — Freeze the cross-repository contract

Before changing the wire schema, agree on one versioned DTO and rollout plan
with PipelineGen. Keep the current `semantic_manifest.v2` decoder available
during migration. Publish a v3 contract only after both sides have fixtures
for it; do not change a schema string while leaving Go's schema validation on
v2.

The entity projection must include at least:

```json
{
  "local_id": "org_01",
  "canonical_id": "wd:Q42262",
  "link_status": "linked",
  "canonical_name": "European Space Agency",
  "entity_type": "ORG",
  "role": "named_entity",
  "disambiguation_gloss": "European intergovernmental space agency",
  "negative_constraints": ["entertainment software association", "video games"],
  "mentions": [],
  "search_intents": [
    {"kind": "identity", "query": "European Space Agency ESA", "lang": "en"},
    {"kind": "contextual", "query": "ESA space missions Mars", "lang": "en"}
  ]
}
```

Contract rules:

- `local_id` is stable only within the document and is used for manifest
  references. Preserve it for v2 compatibility.
- `canonical_id` is a linked KB ID when resolution succeeds. Otherwise use a
  documented document-scoped key; do not label a surface slug as linked.
- `link_status` is one of `linked`, `ambiguous`, or `unlinked`.
- Mentions preserve verbatim source text and UTF-8 byte offsets.
- Roles distinguish named identity lookup from generic visual-concept/B-roll
  lookup. A VLM identity gate applies only to named entities.
- Search intents are ordered, typed and short. Context, gloss and exclusions
  are verifier inputs; they are never silently concatenated into provider
  queries.
- Serialization order is explicit and deterministic. New fields are additive
  where possible; incompatible changes require a schema version increment.

**Acceptance:** Rust and Go share checked-in v2/v3 JSON fixtures; both decoders
accept the versions they claim to support, reject unknown required versions,
and preserve IDs, intent ordering, offsets and verifier constraints on a
round-trip.

### WP1 — Stable identity and source-preserving normalization

Keep `id`/`local_id` for document references. Add a separate `canonical_id`
field to every entity consumed by the resolver, sourced from the shared
identity registry. Normalize Unicode for matching while retaining original
surfaces and byte offsets. Apostrophe folding must cover ASCII apostrophe,
U+2019, U+02BC and grave accent without rewriting evidence.

Clitic handling must use token boundaries and evidence, not a blind prefix
trim. A valid implementation separates an elided article before a proper
name but preserves lexicalized or ambiguous names such as `L'Aquila`,
`d'Artagnan` and `O'Brien`. Normalize possessive `NASA's` for identity
matching without changing the source span.

For unlinked entities, derive document-scoped identities from a normalized
typed key. Ensure punctuation/Unicode normalization cannot collapse distinct
keys to the same slug. Keep names with the same surface but different types
separate. Do not merge partially overlapping names such as Michael Jordan and
Michael B. Jordan.

**Acceptance:** T1 and dedicated collision tests pass; the same entity has the
same appropriate linked ID across documents/languages, while distinct
unlinked documents remain namespaced. Every mention round-trips to its exact
original byte slice.

### WP2 — Local entity linking and ambiguity evidence

Add a versioned, local-only concept index with multilingual labels, aliases,
entity type, canonical name, short gloss, query anchor and negative
constraints. Begin with a small, auditable seed set for the release corpus
(ESA, NASA, ANSA, Nike, Michael Jordan, Michael B. Jordan, CERN and SpaceX),
then define how a larger Wikidata-derived snapshot is generated, licensed,
versioned, packaged and memory-mapped. The seed set is a bounded MVP resource,
not a claim of broad world coverage.

Resolve ambiguous aliases by deterministic context scoring over grounded
co-occurring linked entities and typed nominal evidence. Return `ambiguous`
when evidence ties or is insufficient; never choose an unrelated homonym just
to fill an image slot. Record the index version in analyzer provenance and
health output. Runtime analysis must perform no HTTP or other network I/O.

**Acceptance:** T3 distinguishes Nike sportswear from Greek mythology/statues;
ESA in NASA/Mars/space context links to the space agency and carries explicit
video-game exclusions; ESA in E3/software context resolves differently or is
reported ambiguous. T5 IT/EN/PT aliases converge on the same linked ID. Tests
run against the packaged index without network access.

### WP3 — Candidate roles, BOS gate and query intent builder

Retain the structural BOS gate, but define and test the decision using POS,
gazetteer evidence, acronym/mixed-case structure and document-level recurrence.
It must not promote an ordinary sentence opener solely due to capitalization.
Preserve exact gazetteer names, validated acronyms and names with internal
capitalization. Emit generic concrete phrases under `visual_concept`, not as
named entities. Normalize company-like entities such as SpaceX consistently
to the agreed taxonomy rather than accepting a relaxed test that equates
`BRAND` and `ORG` without an explicit mapping.

Generate a maximum of three typed intents. The identity query is first and is
limited to a canonical name/short identity anchor. Contextual queries may use
only typed co-mentioned entities or approved nominal heads. Never use an
adjacent raw-token window, the whole sentence, a location unrelated to the
entity, or hardcoded geography/news suffixes. Enforce word-count and
punctuation limits in the builder.

**Acceptance:** T4 and T6 pass with strict role/type assertions. Tests assert
that `NASA Roma`, inflected verbs, discourse openers and irrelevant sentence
text do not enter identity queries; visual concepts are not sent through the
identity verifier.

### WP4 — Performance and resource gate

Profile the warmed pipeline by phase on the release workload. Current evidence
shows a large UDPipe-associated cost (1.692 s with models versus 86.1 ms
without), but the model-free path also misses the target; measure tokenization,
POS, extraction, identity linking, relation/query construction, consolidation
and serialization independently before choosing optimizations.

Investigate replacing per-scene full UDPipe dependency parses with a shared
per-script pass or a tag-only path, while retaining deterministic POS and
source-span alignment. Avoid introducing a second NLP parse during query
generation. Keep immutable lexicon/index resources process-wide and bounded.
Do not silently disable POS or fall back to different semantics on hosts
missing a declared language model.

Benchmark protocol:

- Release build, fixed runner/CPU, pinned model/index hashes and recorded
  environment.
- Separate cold process/model load from warm request latency.
- At least 100 warm samples for a meaningful p99, with the 20-scene /
  ~750-word fixture and representative non-duplicate scenes.
- Report p50/p95/p99, CPU time, peak RSS and per-stage timings.
- Verify zero outbound connections during analysis using an OS-level test
  harness or isolated network namespace; absence of application HTTP calls is
  not sufficient evidence.
- Run at least 1,000 analyses in one process and verify RSS reaches a stable
  plateau under the configured cache/index budget.

**Acceptance:** Warm p99 below 25 ms on the declared reference runner, zero
network connections, and bounded RSS with no monotonic growth. If the target
cannot be achieved with the supported POS models, publish a revised measured
SLO and get it agreed before calling the MVP release-ready.

### WP5 — One PipelineGen consumer and cache/verifier integration

PipelineGen must accept the blueprint unchanged and route all callers through
one `EntityVisualResolver`. Remove query manipulation, surface-based entity
reclassification and independent `Hits[0]` admission from the legacy paths.
Providers return untrusted candidates; only a verifier receipt may satisfy an
entity. Retrieval intents for one entity run sequentially, and contextual
retrieval stops after primary coverage is achieved.

Cache keys use `canonical_id + intent_id + verifier_version + catalog/index
version` only for linked entities. Unlinked entries include the document
fingerprint and local ID. The VLM request carries canonical name, gloss,
scene context and negative constraints as structured fields. Downloaded bytes
are hashed once and the exact verified bytes are materialized.

**Acceptance:** Go contract tests prove it does not alter Rust query strings;
cache tests show cross-language reuse for linked IDs and no collision for
unlinked identities; verifier tests reject the Nike statue and ESA's unrelated
software association; stop-on-satisfaction avoids later intents after a
verified primary asset. A cross-repository smoke test runs the same fixtures
through the released BrainRust binary and Go resolver.

**Status (2026-11-15):** implemented in `refactored`. The Go consumer is v3-only;
 the dead v2 fallback was removed. The VLM scorer remains an external runtime
dependency, but the admission gate is fail-closed once a scorer is wired. The
cross-repo smoke test passes with the release binary. The full Go suite is
currently blocked by unrelated in-flight `overlay_plan_*` signature changes, not
by this WP5 diff.

### WP6 — Public release and reproducibility

- Publish one BrainRust release whose tag, git SHA, binary manifest, semantic
  schema, entity-index version and language-model hashes agree.
- Make PipelineGen pin that exact release/SHA; test the downloaded release
  binary, not only the workspace library.
- Make readiness fail when a supported language model or required local index
  is absent or has the wrong hash.
- Add release CI for workspace tests, contract fixtures, deterministic replay,
  benchmark, no-network test and SBOM/license inventory for bundled data.
- Document the supported languages, linked-entity coverage and explicit
  `ambiguous`/`unlinked` behavior.

## MVP regression matrix

| Test | Required result | Current status |
| --- | --- | --- |
| T1 Apostrophes / identity | Canonical name and identity stable; raw offsets preserved | Pass: v3 linked entity, stable `surface_key`, offsets round-trip |
| T2 Jordan ambiguity | Two disjoint people; exact spans; Chicago Bulls is one org | Pass |
| T3 Ontological disambiguation | Nike and ESA glosses plus negative constraints | Pass for the packaged seed: Nike sport vs statue, ESA space vs software |
| T4 Query hygiene | Short typed identity-first intents; no raw windows | Pass: v3 typed intents, max 3, identity first |
| T5 Cross-language identity | IT/EN/PT aliases share a linked ID | Pass for the packaged seed (ESA `wd:Q42262`) |
| T6 BOS / roles | No sentence-opener false positives; strict types and named-vs-concept role | Pass: SpaceX `ORG` + `named_entity`; `Innovazione`/`Tuttavia` rejected |
| T7 Determinism | Parallel byte-identical serialization | Pass |
| T8 Performance / I/O / memory | Warm p99 <25 ms, no network, bounded RSS | Measured: 1.551 s with UDPipe, 98.6 ms model-free; no-network pass; RSS plateau 228,264 KiB. The 25 ms target is not met; revised SLO below. |

### Revised measured SLO (2026-11-15)

The original 25 ms warm p99 for the 20-scene fixture is not reachable on this
baseline: even the model-free p50 is 77.9 ms (3.9 ms/scene). The measured
release baseline is:

- With UDPipe models: p50 1.438 s, p95 1.503 s, p99 1.551 s.
- Model-free: p50 77.9 ms, p95 88.7 ms, p99 98.6 ms.
- One-scene `visualner::extract` with models: p50 35.3 ms, p99 38.3 ms.

Proposed release SLO until the per-scene parse is redesigned: p99 ≤ 1.6 s for
a 20-scene script with UDPipe, and p99 ≤ 100 ms model-free. The release test
uses a 3.0 s max-of-20 guard; the 1.6 s figure is the 100-sample probe, not
the test threshold. This requires owner agreement; it is published as
measured evidence, not claimed as the original 25 ms target.

## Dependencies and sequencing

1. BrainRust and PipelineGen owners approve the v3 JSON fixture and rollout
   compatibility before implementation of linked IDs in the Go cache.
2. BrainRust lands WP0–WP3 and keeps v2 decoding available during migration.
3. WP4 profiling starts immediately and runs alongside contract work; do not
   wait for the KB snapshot to identify the performance bottleneck.
4. PipelineGen lands WP5 against the agreed fixture; no release pin moves until
   the real binary smoke test passes.
5. WP6 release checks run only after all required acceptance evidence is
   archived.

## Definition of done

All WP acceptance criteria above pass in CI and on the declared reference
runner; the eight regression tests have explicit coverage; the full workspace
and Go integration suites are green; a released BrainRust binary passes the
cross-repository smoke test; and the public release manifest identifies the
exact schema, build, model and entity-index versions used.

The Rust core is now an ontologically linked producer for the packaged seed
set, with typed intents and OS-level no-network/RSS evidence. The PipelineGen
Go consumer is now wired to v3 with intent/index-aware cache keys,
verified-hit admission, stop-on-satisfaction and a passing cross-repo smoke
test. The only remaining external gate is owner agreement on the revised
measured SLO above. The broad Go suite is currently blocked by unrelated
in-flight `overlay_plan_*` signature changes outside this WP5 diff.

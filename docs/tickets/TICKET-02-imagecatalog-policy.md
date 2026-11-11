# TICKET 02 — imagecatalog policy and snapshot integrity

- Priority: P0
- Status: DONE (Rust policy); Go/Rust parity fixture remains external
- Source: audit roadmap PR 2
- Files: `imagecatalog/src/catalog.rs`, `clock.rs`, `plan.rs`, `types.rs`,
  `imagecatalog/tests/manifest_reuse.rs`

## Problems

- `status_usable || durable` let a `retired` candidate through when a verified
  materialization existed, contrary to the documented policy.
- Any unrecognized candidate status fell back to `fresh`.
- `CatalogIndex::new` used `or_insert`, so two contradictory rows with the same
  `candidate_id` resolved to whichever appeared first in the export.
- The timestamp parser accepted impossible calendar dates such as 31 April.
- The plan did not record the instant at which freshness/ranking were evaluated.

## Change

- A durable materialization now bypasses only a `broken` remote URL. `retired`
  and `unknown` statuses remain ineligible.
- Only `fresh`, `stale`, `broken`, `retired` and the explicit legacy `active`
  are recognized. `active` maps to `fresh`; anything else maps to `unknown`.
- `CatalogIndex::new` returns `Result`. Identical duplicate rows are
  deduplicated; contradictory duplicates for an entity, candidate or
  materialization return an integrity error. The production `plan()` path
  propagates that error instead of producing a plan.
- Timestamp validation uses month lengths and leap years.
- `ImagePlan` now carries `evaluated_at` (UTC seconds), and the replay contract
  is documented as same text + analyzers + catalog + evaluation instant.

## Acceptance evidence

```sh
cargo test -p imagecatalog --lib --tests
```

39/39 tests pass:

- 24 unit tests, including `a_retired_candidate_is_never_selectable_even_with_a_materialization`,
  `an_unrecognized_status_is_not_treated_as_fresh`,
  `contradictory_duplicate_candidates_are_rejected`, calendar-date cases and
  `matches_go_canonical_ids_for_accented_punctuated_and_non_ascii_names`;
- 3 live-fixture tests;
- 7 manifest-reuse tests;
- 5 plan integration tests.

## Go parity audit (2026-11-11)

The PipelineGen checkout is available locally. Its own packages pass:

```sh
go test ./internal/capabilities/entities \
  ./internal/capabilities/images/entitycatalog \
  ./internal/capabilities/scripts/adapters -count=1
```

The Go `CanonicalEntityID("PERSON", name)` derivation matches the Rust
projection for case/whitespace variants, possessives, accented names,
punctuation and non-ASCII names; the expanded Rust battery now pins those
cases. The two policies are deliberately **not** byte-identical today:

- Go `ClassifyCandidateStatus` still maps an unrecognized stored status to
  `fresh`; Rust maps it to `unknown` and rejects it, as the audit requested.
- The Go image adapter still lets a verified materialization keep a `retired`
  row in the pool (`!IsUsableCandidateStatus(...) && !durableImage` is false);
  Rust explicitly rejects `retired` regardless of materialization, matching
  the documented policy.

Those are Go-owner behavior changes, not Rust defects. No Go file was modified
here, and the shared decision fixture remains blocked until the Go policy is
changed to the same fail-closed contract.

## Residual

The shared Go/Rust decision fixture is not committed because the two policies
differ on the two cases above, which are owned by PipelineGen. The Rust
behavior is fail-closed and covered; the parity gate remains owned by the Go
catalog policy.

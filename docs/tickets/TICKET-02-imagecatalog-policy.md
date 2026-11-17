# TICKET 02 — imagecatalog policy and snapshot integrity

- Priority: P0
- Status: DONE (Rust policy; Go fail-closed behavior verified 2026-11-16)
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
cargo test --manifest-path imagecatalog/Cargo.toml --lib --tests
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
cases.

## Go policy verification (2026-11-16)

The two cases the 2026-11-11 audit recorded as Go-side divergences are now
fail-closed in PipelineGen, so the policies agree on the decision the fixture
was meant to pin:

- `ClassifyCandidateStatus` returns `CandidateStatusUnknown` for an
  unrecognized or empty stored status, and `IsUsableCandidateStatus` excludes
  every status outside `fresh`/`active`/`stale`
  (`internal/capabilities/images/entitycatalog/state_policy.go`, `port.go`).
  Rust maps the same input to `unknown` and rejects it.
- The recertification pool selects only the usable statuses plus `broken`
  retries (`entity_image_catalog_adapter.go` `ListCandidatesForRecertification`),
  so a `retired` row never re-enters the pool; `TransitionCandidateStatus`
  requires the explicit `revive` event to leave `retired`. Rust rejects
  `retired` regardless of a durable materialization.
- `defaultSemanticStatus` maps an empty verdict to `unknown`, never `fresh`.

Evidence: `go test ./internal/capabilities/images/entitycatalog/...
./internal/capabilities/images/... -count=1` passes on the 2026-11-16 tree;
`state_policy_test.go` pins the unknown classification and the
retired-only-exit transition, and `entity_image_catalog_state_test.go` covers
the adapter reads.

## Residual

None inside this checkout. The two assertions live in the owning suites (Go
`state_policy_test.go`, Rust `imagecatalog/src/catalog.rs`) instead of one
cross-repo file; a shared fixture would be a refactor of the test layout, not
a behavior change.

# TICKET 18 — Linguistics Go demolition

- Priority: P2
- Status: OPEN
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-15 (phrase selection), TICKET-20 (identity)
- Blocks: none

## Why

Go still loads and applies a linguistic profile:

- `internal/capabilities/linguistics/lexicon_registry.go` and loaders scan
  `config/lexicons/**` and build `LexiconRegistry`.
- `internal/app/wiring/script/lexicon.go` wires it into production.
- `scripts/entity_annotations.go` (561 LOC) and `entity_projection.go` (457
  LOC) still contain language-aware projection and grounding rules.
- `scripts/translated_nlp_identity.go` (571 LOC) contains localized identity
  heuristics for person inflection (pl/de/tr/ru).

Rust already owns the lexicons and the entity linker. The Go bootstrap is a
second authority even when the stopword files are byte-identical today.

## Target

Rust `semantic-core` / `semantic-engine` owns tokenization, stopwords,
morphology and identity projection. Go receives typed spans and identities and
only validates them against the scene text.

## Acceptance

- No production `NewLexiconRegistry` / `LexiconRegistry` caller outside tests.
- Go stopword/bootstrap files deleted or reduced to a generated, read-only
  projection with a parity gate.
- `translated_nlp_identity.go` no longer inflects or matches names; it projects
  the Rust-provided localized mention.
- Parity fixtures for de/en/es/fr/it/pt plus pl/ru/tr inflection.
- `make verify-main` on the cutover tree.

## Risks

- Language packs include a Go-only `fallback` set for unsupported languages;
  the Rust contract must cover or explicitly reject those languages.
- Removing the registry can change startup validation; keep a fail-closed
  check on the Rust profile version.

## Verification 2026-11-16 (workspace)

Still a live Go authority; no acceptance criterion has landed.

- `internal/app/wiring/script/lexicon.go:21` calls `NewLexiconRegistry` on the
  production path and feeds the intent resolvers
  (`internal/capabilities/brain/intent/registry.go`).
- `runner_overlay_prepare.go:338` and `translated_nlp.go:429` call
  `projectEntityAnnotations`; the language-aware rules in
  `entity_annotations.go` (561 LOC) and `entity_projection.go` (457 LOC) are
  live.
- `internal/capabilities/linguistics/` is 796 non-test LOC. No Rust operation
  returns the tokenization/morphology projection that would let Go delete the
  registry. `semantic-engine/src/protocol.rs` now registers `select_phrases`
  and `align_phrases` in addition to `health`, `analyze_script`,
  `split_sentences`, `compile_editorial_plan`, `resolve_map_overlaps`,
  `trim_map_windows_before_budget`, `rank_entity_overlays` and
  `cascade_clustered_entrances`; none of them replaces `LexiconRegistry` or
  `translated_nlp_identity.go`.
- The 2026-11-16 phrase/alignment work is not a linguistics cutover: phrase
  selection and word alignment are Rust-owned, but stopwords/morphology,
  `projectEntityAnnotations` and localized identity matching remain Go.

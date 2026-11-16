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

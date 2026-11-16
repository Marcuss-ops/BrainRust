# TICKET 20 — Identity and localization consolidation

- Priority: P2
- Status: OPEN
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-18 (linguistics), TICKET-19 (alignment)
- Blocks: none

## Why

`scripts/translated_nlp_identity.go` (571 LOC) is the localization seam: it
matches a source entity surface in translated text and stamps the source
canonical id and image binding onto the localized annotation. It includes
ordered-person fallbacks for ru/tr/pl and inflection matching for pl/de/tr.

That is identity resolution in Go. It can disagree with the Rust linker and it
is not covered by the v3 entity contract.

## Target

Rust owns identity across languages:

- `semantic-engine` receives source and translated scenes in one request (or a
  linked identity map) and returns the localized mention with the source
  canonical id, type and image binding.
- Go validates the mention occurs verbatim in the translated scene and
  projects it; it does not match, inflect or complete identities.

## Acceptance

- Rust localized-identity contract with fixtures for ru/tr/pl/de inflection and
  writing-system changes.
- Go projection carries the source id unchanged; a missing id fails closed.
- `matchLocalizedSourceEntities` and `completeOrderedPersonMatches` have no
  production caller.
- `go test ./internal/capabilities/scripts/...` and the Rust linker suite pass;
  `make verify-main` on the cutover tree.

## Risks

- Translation can legitimately omit or rename a person; the contract must
  distinguish "not present" from "unlinked".
- The image binding belongs to the source identity; do not re-resolve it from
  the translated surface.

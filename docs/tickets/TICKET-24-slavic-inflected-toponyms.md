# TICKET-24 — inflected Slavic toponyms miss the gazetteer

- Priority: P1
- Status: OPEN
- Source: `PER-LANGUAGE-GOLD-REPORT-2026-10-28.md` §6; `docs/stato/brainrust.md` §3
- Files: `visualner` (gazetteer lookup), `semantic-engine` (query generation),
  `lexicon/entity_gazetteer.json`

## Problem

Contextual image queries are 0/2 in Polish and Russian; typed-object recall is
0.3 in both languages. The four measured misses are inflected toponyms —
`Poznaniu`, `Krakowie`, `Казани`, `Самаре` — that the exact gazetteer lookup
does not resolve to their canonical entry. The gazetteer has 69,433 places but
no canonical ID/gloss grouping, so a blind fuzzy match would create false
links.

## Acceptance criteria

- [ ] The four named surfaces resolve to the canonical place used by the
      sentence, with the source span preserved.
- [ ] Polish and Russian contextual-query recall reach 2/2 each; typed-object
      recall does not fall below 0.3 and the other eight languages do not
      regress.
- [ ] A negative list of common words with the same endings does not become a
      place (0 false positives on the named list).
- [ ] The matching strategy is locale-scoped and documented (case/suffix
      handling or an inflected alias set); no fixture table and no global
      fuzzy match.
- [ ] The four cases run in the per-language gold report.

## Non-goals

- Expanding the gazetteer with automatically generated IDs; that needs a
  canonical dataset and a quality measure.
- Resolving every Slavic case; the ticket is scoped to the four measured
  surfaces plus a documented bound.

## Next action

Turn the four names into failing per-language cases with their canonical
targets, then add the locale-scoped match and the negative list in the same
change.

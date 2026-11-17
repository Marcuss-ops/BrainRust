# TICKET-24 — inflected Slavic toponyms miss the gazetteer

- Priority: P1
- Status: DONE (2026-11-17)
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

- [x] The four named surfaces resolve to the canonical place used by the
      sentence, with the source span preserved.
- [x] Polish and Russian contextual-query recall reach 2/2 each; typed-object
      recall does not fall below 0.3 and the other eight languages do not
      regress.
- [x] A negative list of common words with the same endings does not become a
      place (0 false positives on the named list).
- [x] The matching strategy is locale-scoped and documented (case/suffix
      handling or an inflected alias set); no fixture table and no global
      fuzzy match.
- [x] The four cases run in the per-language gold report.

## Non-goals

- Expanding the gazetteer with automatically generated IDs; that needs a
  canonical dataset and a quality measure.
- Resolving every Slavic case; the ticket is scoped to the four measured
  surfaces plus a documented bound.

## Evidence — 2026-11-17

- `lexicon/place_inflections.json` is a locale-scoped alias table: exact
  surfaces, canonical target, `query_form` and a named blocklist. There is no
  global fuzzy match.
- Polish `Poznaniu`/`Krakowie` resolve to `Poznań`/`Kraków` and the query uses
  the nominative (`canonical`); Russian `Казани`/`Самаре` keep the source case
  (`source`). Source spans are preserved.
- Polish and Russian contextual queries: **2/2 each**, overall **20/20**.
- Named negative list: **4/4** cases remain non-places.
- Typed-object recall stays at **93/105** on both untouched HEAD `d3b77d6` and
  this change; the 2026-10-28 report's 99/105 is stale at HEAD. The other eight
  languages are unchanged.
- Tests: `cargo test -p semantic-core --test place_inflections` — 4 passed;
  `cargo test -p semantic-engine --test slavic_places` — 2 passed.

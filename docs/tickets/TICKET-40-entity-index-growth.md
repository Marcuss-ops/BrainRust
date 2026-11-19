# TICKET-40 — grow the packaged entity index for disambiguation coverage

- Priority: P1
- Status: CLOSED (2026-11-19)
- Depends on: —
- Files: `lexicon/entity_index_v1.json`, `lexicon/tools/build_entity_index.py`,
  entity-linking gold and metrics

## Problem

TICKET-35 requires a confusable set with at least 40 surfaces and 2–3 senses
per surface, but `entity_index_v1.json` currently contains 10 entries and only
two ambiguous aliases (Nike and ESA). TICKET-35 explicitly excludes adding KB
entries, so its acceptance set cannot be met until index growth has its own
scope, provenance and collision checks.

## Scope

Grow the deterministic, offline packaged index with a curated seed sufficient
to exercise disambiguation. Target at least 40 confusable surfaces and 80
distinct senses across PERSON, ORG, LOCATION, BRAND, PRODUCT and FACILITY,
including the existing seed unchanged. Every row must have a stable external
identifier, canonical name, type, short gloss, localized aliases, context
terms and counter terms. Index rows are identity metadata, not evidence that a
textual claim is true.

Use a reproducible builder and record source, licence, retrieval date, schema
version and checksum. Prefer CC0 Wikidata exports for curated rows; any row
from a source with attribution or share-alike obligations must carry required
attribution and be reviewed before packaging. Do not scrape at runtime or add
unrestricted bulk dumps. Keep a human-reviewable source manifest.

## Acceptance criteria

- [x] At least 40 explicitly listed confusable surfaces, each with 2–3
      distinct sense IDs and at least 3 grounded context/counter terms per
      sense.
- [x] Every row validates: unique stable ID, nonempty canonical name/type/gloss,
      at least one alias, valid language tags and no duplicate aliases within
      one sense.
- [x] A generated collision report lists every cross-sense alias collision;
      each intended ambiguity is explicitly represented and no unintended
      collision is silently accepted.
- [x] Source manifest records source, licence, retrieval date, row provenance
      and checksum; packaged output is at most 5 MiB.
- [x] Existing linked IDs and current entity/index goldens remain unchanged;
      expanded rows have grounded positive and negative lookup fixtures.
- [x] Deterministic build check runs in CI. Rust performs no network access at
      runtime.

## Non-goals

Changing the TICKET-35 scoring or decision rule, online lookup, full Wikidata
coverage, inferred popularity priors, or asserting factual truth from a link.

## Implementation steps

1. [x] Define the row/source manifest schema and validator before adding data.
2. Curate a source-controlled ambiguity list with stable IDs, senses and
   localized aliases; review licence/provenance for every row.
3. Generate the packaged index and collision report deterministically.
4. Add the ≥40-surface fixtures and CI checks, then let TICKET-35 consume the
   expanded seed for scoring calibration.

## Progress

The packaged seed now has 86 rows and 46 explicitly declared alias-collision
surfaces (40 English surfaces plus six localized collisions) spanning two
senses each. `entity_index_confusables_v1.json` supplies 276 positive and 276
counter-context lookups, with frozen expected IDs. `entity_index_v1.json` is
54 KiB, below the 5 MiB limit. The validator verifies row provenance, exact
collision declarations, fixture coverage and SHA-256 checksums. CI runs it;
Rust continues to use only the embedded index, with no network access.

The original ten index rows and their IDs are retained. Existing identity,
localized-identity and MVP entity-linking contracts pass with the expanded
index.

The source manifest points to Wikidata structured data under CC0-1.0 and records
the retrieval date, item URL and source note for each row. Wikidata's licensing
policy says its structured data is released under CC0:
<https://www.wikidata.org/wiki/Wikidata:Licensing>.

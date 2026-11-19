# Semantic expansion roadmap — tickets 29–36 and 40 (2026-11-19)

Scope: extend what `semantic-engine` **understands**. Nothing here renders,
searches providers or touches PipelineGen state. Semantic tickets add typed,
grounded facts to `semantic_manifest.v3`; TICKET-40 supplies the packaged
identity data needed to measure disambiguation. Planning and rendering stay in
Go / Chronon.

## What exists today (verified in code, 2026-11-19)

| Area | Current state | Where |
|---|---|---|
| Numbers | `NumericFact` kind `NUMBER`/`PERCENT`/`MONEY`/`FRACTION`; `form` is `digits`, `words` or `mixed`. ICU word scanning folds case, strips internal conjunctions, composes additive/multiplicative runs, resolves fractions and halves, includes approximators and rejects idioms; scale factors now cover es/pt/nl forms | `semantic-core/src/numbers.rs`, `semantic-core/src/number_words.rs`, `visualner/src/spellout.rs`, `lexicon/number_words.json` |
| Dates | `TemporalFact` (`day`/`month`/`year`/`unresolved`, BCE era, order-ambiguity flag); **absolute dates only** | `semantic-core/src/dates.rs` + `dates/*.rs` |
| Entities | `CanonicalIdentity` (PERSON, ORG, LOCATION, FACILITY, BRAND, PRODUCT…), `LinkedEntity` with `LinkStatus` linked/ambiguous/unlinked, gloss, `context_terms`/`counter_terms` in `entity_index_v1.json` | `identity.rs`, `entity_linker.rs` |
| Places | Toponyms are LOCATION entities; `location_cues.json`, `place_inflections.json` (pl/ru…). **No coordinates, no kind, no hierarchy, no routes** | `lexicon/` |
| Facts | `EntityFact` (predicate, value, unit, operator, status asserted/negated/uncertain) | `semantic-engine/src/facts.rs` |
| Relations | `AssertedRelation` ownership/funding/collaboration… | `semantic-core/src/relations.rs`, `lexicon/relation_cues.json` |
| Discourse | `discourse.v1`: references, threads, scene continuity; pronouns resolve only with one compatible antecedent | `semantic-engine/src/discourse/` |
| Groups | chronology, ranking, process, comparison, cause-effect, ordered sequences | `semantic-engine/src/groups.rs` |

## Tickets

| Ticket | Title | Depends on | Size |
|---|---|---|---|
| [29](TICKET-29-spoken-number-words.md) | Numbers written as words | — | M |
| [30](TICKET-30-typed-quantities.md) | Typed quantities with units, scale, year and comparison | 29 | L |
| [31](TICKET-31-relative-dates.md) | Relative dates anchored to a reference | 29 | M |
| [32](TICKET-32-places-and-routes.md) | Places with kind/coordinates/hierarchy and routes | 35 | L |
| [33](TICKET-33-person-roles.md) | People with role, affiliation, family and age | 31, 35 | L |
| [34](TICKET-34-events.md) | Event frames (type, participants, place, date) | 31, 32, 33 | L |
| [35](TICKET-35-entity-disambiguation.md) | Explainable entity disambiguation | 40 | M |
| [36](TICKET-36-coreference-pronouns.md) | Pronoun / role-nominal coreference | 33 (roles), 35 | M |

Recommended order: **40 → 35 → 32 → 33 → 36 → 34**, alongside the completed
29/31 sequence and the remaining TICKET-30 acceptance gates.
40 precedes 35 because the scorer needs a provenance-reviewed confusable seed;
then a wrong link ("Washington" the person vs the city) can be caught before
it poisons places, roles and events downstream.

```mermaid
flowchart LR
  T40[40 Entity index growth] --> T35[35 Disambiguation]
  T35 --> T32[32 Places]
  T35 --> T33[33 Roles]
  T35 --> T36[36 Coreference]
  T29[29 Number words] --> T30[30 Quantities]
  T29 --> T31[31 Relative dates]
  T31 --> T33
  T31 --> T34[34 Events]
  T32 --> T34
  T33 --> T34
  T33 --> T36
```

## Invariants shared by every ticket (copy into each PR description)

1. **Grounded or absent.** Every emitted item carries `start_byte`/`end_byte`
   (UTF-8, exclusive end) and verbatim `evidence`. No span → no item.
2. **Fail closed.** Ambiguity yields `ambiguous`/`unresolved` with candidates,
   never a guess. Empty categories stay empty.
3. **Deterministic.** Same input → byte-identical output across runs and across
   `VISUALNER_WORKERS` values. Ties break by stable id, never by hash order.
4. **Additive schema.** New fields use `#[serde(default)]` and
   `skip_serializing_if`; `schema_version` stays `semantic_manifest.v3` unless a
   field changes meaning. Go consumer copy + goldens
   (`contract/`, `refactored/testdata/…`) updated in the same change.
5. **Cache safety.** Bump `AnalyzerVersions` for the touched analyzer so the
   fingerprint (and any manifest cache) invalidates.
6. **Data, not code, per language.** Vocabularies live in `lexicon/…` and are
   generated/checked by `lexicon/tools/build_language_packs.py --check`.
   No fixture-name or language-specific branches in Rust.
7. **Measured.** Each ticket ships a gold set (≥ 20 positive, ≥ 20 negative
   cases per capability, all 10 languages represented), a metric in
   `score_long_gold.py`, and the Pareto bar (≥ 90 % recall, 0 false positives
   on the negative set). Annotations authored by the agent are labelled
   `assistant_draft_pending_human_review` exactly as in TICKET-08.
8. **No external truth.** The manifest records what the text says. Coordinates
   and KB ids come from the packaged, versioned indexes; they are identity
   data, not a claim that the text is correct.
9. **Verification command** (with UDPipe models, ICU 72):
   `VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models cargo test --workspace -j 2`
   plus `cargo fmt --all -- --check` and
   `cargo clippy --workspace --all-targets -- -D warnings`.

## Readiness audit — 2026-11-19

I ticket 30–36 non possono essere dichiarati CLOSED in questo checkout senza
inventare dati. Il motivo è verificabile nei file, non una stima:

| Ticket | Cosa manca davvero | Evidenza |
|---|---|---|
| 29 | **CLOSED 2026-11-19** | lessico, grammatica, frazioni/mezzi, 220+220 casi e gate di build |
| 30 | **PARTIAL 2026-11-19**; corpus positive/negative completo e metriche per lingua/classe di dimensione | `lexicon/units.json` e pack generati, `Quantity` nel manifest, scale/periodi/range/confronti e chart bridge implementati; `build_units.py --check`, suite `charts_numeric` 17/17 e test mirati verdi |
| 31 | **CLOSED 2026-11-19** | lessico 11 lingue, ancora esplicita/riferimento/narrativa, aritmetica civile, 231 casi gold |
| 32 | `lexicon/place_index_v1.json` e `build_place_index.py` | entrambi assenti; servono GeoNames/Wikidata con licenza e data di retrieval |
| 33 | `lexicon/roles/<lang>.json` (~80 codici × 10 lingue), relazioni e età | nessun file ruoli o `person_roles` nel manifest |
| 34 | `lexicon/events/<lang>.json` (12 tipi × 10 lingue) e merge cross-scena | nessun `event.rs` o `events` nel manifest |
| 35 | Explainability/scoring | **OPEN**; acceptance gold requires index coverage tracked separately by TICKET-40 |
| 36 | `pronouns.json` strutturato per lingua e gold ≥40 × 10 | i pronomi sono ancora `pronouns.txt` piatti; nessun `resolution_basis` |
| 40 | Crescita indice entity per gold ambiguo | **OPEN**; fonte/licenza/provenienza, builder, collision report e ≥40 superfici confondibili |

Conseguenza operativa: 29 e 31 sono chiusi, 30 è parziale; restano da
completare la sua matrice gold e i ticket 32–36. TICKET-35 dipende da TICKET-40
per la crescita dell'indice; lo scorer resta invariato finché non esistono un
seed confondibile con provenienza verificata e i relativi fixture di regressione.

## Out of scope for the whole series

Rendering/overlay design, provider search, currency conversion, fact-checking,
generative models inside Rust, and any Go-side planner decision. Go consumes the
new fields later under its own ticket.

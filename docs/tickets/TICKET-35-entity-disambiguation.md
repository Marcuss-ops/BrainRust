# TICKET-35 — entity linking is not explainable and under-uses context

- Priority: P1
- Status: OPEN
- Depends on: TICKET-40 (confusable index coverage)
- Files: `semantic-core/src/entity_linker.rs`, `lexicon/entity_index_v1.json`,
  `semantic-engine/src/identity_lookup.rs`, `LinkedEntity`

## Problem

`LinkStatus` is `linked | ambiguous | unlinked` and entries carry
`context_terms` / `counter_terms`, but the manifest does not show **why** an
entity was linked, which alternatives lost, or by what margin. "Washington"
(person/city/state), "Paris" (city/person), "Jordan", "Georgia", "Mercury",
"Victoria" can end up with the wrong image, and the failure is not auditable.
A wrong link is also the root cause of wrong coordinates (32), roles (33) and
events (34), so this ticket comes first.

## Design

1. **Candidate generation** (unchanged): alias match, cross-language, inflected
   forms (TICKET-20/24).
2. **Feature scoring** per candidate, all deterministic and logged:

   | Feature | Signal | Weight source |
   |---|---|---|
   | `alias_exactness` | exact vs normalised vs inflected | table in index header |
   | `type_cue` | clause cues: locative preposition → LOCATION; subject of speech/action verb → PERSON; `di/of` + org noun → ORG | `lexicon/type_cues.json` |
   | `context_overlap` | `context_terms` hits in same sentence / scene / script window (decaying with distance) | index rows |
   | `counter_penalty` | `counter_terms` hits | index rows |
   | `discourse_prior` | same surface already resolved in the same thread (one-sense-per-discourse) | discourse threads |
   | `geo_parent` | a containing place cited nearby (`Springfield, Illinois`) | place index (32) |
   | `kb_prior` | optional curated popularity prior (only when present in the row; never inferred) | index rows |

   Weights are constants in a versioned table (`link_weights.v1.json`), not
   learned at runtime.
3. **Decision rule**: link iff `top ≥ θ_link` **and** `top − second ≥ δ_margin`;
   otherwise `ambiguous` with the candidate list. `θ`/`δ` are calibrated on the
   gold confusable set and stored with the weights. Ties break by id.
4. **Explainability (additive)**:
   ```jsonc
   "link_candidates": [
     { "kb_id":"wd:Q61", "name":"Washington, D.C.", "score":0.82,
       "features":[{"name":"type_cue","value":1.0,"span":[..]},
                   {"name":"context_overlap","value":0.4,"terms":["Casa Bianca"]}] },
     { "kb_id":"wd:Q23", "name":"George Washington", "score":0.31, "features":[…] } ],
   "link_margin": 0.51, "link_rule": "v1"
   ```
   Emitted for `linked` entities too (top-3 max) so a reviewer can see near
   misses; size bounded (≤ 3 candidates, ≤ 5 features each).
5. **Script-level pass**: after scoring all mentions, ambiguous mentions inherit
   the resolved sense of the same surface in the same discourse thread; never
   across threads, never from a different type.
6. **Downstream contract**: `ambiguous`/`unlinked` keep today's behaviour
   (no identity image search); the new fields are read-only metadata.

## Acceptance criteria

- [ ] Confusable gold set ≥ 40 surfaces × 2–3 senses, ≥ 3 contexts each,
      spread over the 10 languages: **wrong-link rate ≤ 1 %**, correct-link
      recall ≥ 85 %, remainder `ambiguous` (abstention), never a silent wrong id.
- [ ] 0 regressions on the existing identity/localized-identity tests
      (`entities_identity`, `localized_identity_protocol`) and on the entity
      gazetteer golden.
- [ ] Every `linked` entity has `link_candidates` with at least the winning
      feature evidence; spans are byte-exact.
- [ ] Weights/thresholds file is versioned, checksummed and reported by
      health/readiness.
- [ ] Deterministic across worker counts; `AnalyzerVersions.linker` bumped.

## Non-goals

Embedding models, online KB lookup, crowd/popularity learning, adding new KB
entries (index growth is separate), changing the Go image-gate behaviour.

## Implementation steps

1. Instrument the current scorer to emit features (no behaviour change; golden
   proves it).
2. Introduce `type_cues.json` and `discourse_prior`; calibrate on the gold set.
3. Margin rule and abstention; additive schema; Go copy.
4. Script-level propagation; negative/regression suites.

## Prossimi passi (2026-11-19)

1. **Strumentare lo scorer senza cambiare comportamento:** emettere feature e
   punteggi per ogni candidato; i golden devono restare identici, altrimenti
   la strumentazione ha cambiato la decisione.
2. **`type_cues.json` + `discourse_prior`** e calibrazione sul gold
   (`Washington` persona/città/stato, `Paris`, `Jordan`, `Georgia`).
3. **Regola di margine e astensione:** sotto il margine → `ambiguous`, non
   `linked`; il manifest deve dire perché e con quanto margine.
4. **Propagazione a script** e suite negative, poi la copia Go.

Questo ticket sblocca TICKET-32 (coordinate) ed è prerequisito di 33/34/36:
va eseguito prima, non in parallelo.

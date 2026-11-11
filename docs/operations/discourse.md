# Discourse continuity (`discourse.v1`)

`semantic_manifest.v2` now carries an additive `discourse` projection. It is a
deterministic continuity layer over the identities and source spans the engine
already grounded. It is not a second entity extractor, a language model or a
calibrated probability.

## What it contains

| Field | Meaning |
|---|---|
| `references` | Explicit names, surname aliases, pronouns and role nominals with exact byte offsets. |
| `threads` | One chain per entity with at least two resolved references, including `starts`, `continues` and `returns` beats. |
| `scene_continuity` | Entity-id overlap between consecutive scenes, with `start`, `continues`, `shift` or `empty`. |
| `counts` | Reference status totals and cross-scene thread totals. |

Every reference carries the verbatim sentence, the mention span, the scene id
and the sentence index. `target_entity_ids` is always present and is empty
unless the reference is resolved. `candidate_entity_ids` lists the plausible
antecedents when the status is `ambiguous`.

## Resolution rules

1. **Explicit name** — resolved by the shared `IdentityRegistry`.
2. **Surname alias** — resolved to the full-name identity already registered
   in the script; it never creates a second person.
3. **Pronoun** — resolved only when one compatible antecedent remains:
   same-sentence antecedent, nearest previous sentence, or global uniqueness.
   Two compatible people produce `ambiguous`, never a guess.
4. **Role nominal** (`l'imprenditore`, `the founder`, `la società`) — resolved
   only when the scene has one compatible person/org. A scene with two
   plausible people produces `ambiguous`.
5. **Plural phrase** (`i due`, `the two`) — resolved only when exactly two
   compatible people are in the nearest antecedent sentence.
6. **Quoted speech** — a pronoun inside quotes is `unresolved` with reason
   `quoted_speech`; the engine does not attribute the speaker.
7. **No antecedent** — `unresolved` with `no_compatible_antecedent`.

`resolution_basis` names the rule. It is not a confidence.

## What is deliberately not claimed

- Subjectless-verb **ellipsis** is not resolved in this version.
- Entity-centric generated prose, narrative-importance scoring and visual asset
  reuse are not part of this projection.
- The Jaccard overlap in `scene_continuity` is an entity-continuity signal, not
  a semantic topic probability.
- No human editorial judgment or provider precision@10 is certified by these
  tests.

## Tests

`semantic-engine/tests/discourse.rs` covers:

- explicit returns after a skipped scene;
- a pronoun after twenty intervening sentences;
- two plausible people producing `ambiguous`;
- a negated predicate not disambiguating a pronoun or inventing a target;
- role nominals with one person resolving and with two people staying
  `ambiguous`;
- same-surname people staying distinct;
- plural pair resolution;
- quoted speech staying unresolved;
- surname/full-name identity sharing;
- legacy manifests decoding without the projection;
- status counts and scene continuity.

`semantic-engine/src/discourse/references.rs` additionally tests pronoun/role
detection across the ten supported languages and rejects cross-language false
positives such as Italian `il`, English `to` and German/Dutch `die`.

## Planner use

PipelineGen may use a resolved thread to keep one visual reference across
consecutive scenes and to prefer a visual change only when the continuity
status shifts. The engine only reports the deterministic evidence; asset
selection, reuse and timing remain planner decisions.

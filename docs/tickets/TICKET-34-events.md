# TICKET-34 — events are not a first-class fact

- Priority: P2
- Status: OPEN
- Depends on: TICKET-31 (dates), TICKET-32 (places), TICKET-33 (roles)
- Files: `semantic-core/src/event.rs` (new), `lexicon/events/<lang>.json`,
  `semantic-engine/src/events.rs` (new), `semantic-engine/src/groups.rs`,
  `SemanticManifest.events`

## Problem

Today an event is spread over `entity_facts` (predicate + value),
`temporal_facts`, chronology `semantic_groups` and location entities. Nothing
says "the arrest of X, in Y, on date Z" as one object, deduplicated across
scenes. A timeline or an event card has to rebuild this itself.

## Data model (additive, script level)

```jsonc
// SemanticManifest.events[]
{ "id": "ev_03",
  "event_type": "arrest",             // closed enum, see below
  "trigger": { "surface": "è stato arrestato", "span": [..] },
  "participants": [ {"role":"patient","entity_id":"per_02"},
                    {"role":"agent","entity_id":"org_04"} ],
  "place_id": "pl_07",                // from TICKET-32, optional
  "time": { "fact_index": 4, "normalized_date": "2007-08-14", "derived": false },
  "status": "asserted|negated|uncertain",
  "scene_ids": ["scene_02","scene_05"],      // merged mentions
  "evidence": [ {"scene_id":"scene_02","span":[..],"text":"…"} ],
  "confidence_basis": ["trigger","date","participant"]   // which slots are grounded
}
```

Initial closed `event_type` set (12, extendable by data): `birth`, `death`,
`arrest`, `trial_verdict`, `election`, `appointment_resignation`, `founding`,
`acquisition_merger`, `launch_release`, `accident_disaster`, `attack_battle`,
`match_competition_result`.

## Design

1. **Trigger lexicon** (`events/<lang>.json`): per `event_type`, verb lemmas and
   nominalisations with inflected surfaces (`arrestato/arrestò/arresto`,
   `was arrested/arrest`, `verhaftet/Festnahme`…), voice (active/passive) and
   the participant-role mapping per voice (`è stato arrestato X` → X=patient;
   `X ha arrestato Y` → X=agent, Y=patient). Voice and argument order are data.
2. **Frame assembly per sentence** (deterministic, no ML):
   trigger match → participants = grounded entities in the clause by
   proximity and voice mapping → time = nearest `TemporalFact` in the sentence
   (stated or TICKET-31 derived) → place = nearest `place` with role `setting`
   in the clause. A frame needs a **trigger plus at least one other grounded
   slot** (participant, date or place); trigger-only frames are dropped.
3. **Merge across scenes** (cross-scene dedup): two frames merge when same
   `event_type` and (same `normalized_date` **or** same place) and at least one
   shared participant identity (via discourse threads, TICKET-36 helps).
   Conflicting dates → keep both as separate events; never average.
4. **Reuse, don't duplicate**: `entity_facts` and chronology groups stay as
   they are; events *reference* them (`source_fact_ids`) so goldens for those
   remain stable. `groups.rs` chronology can later order by event time.
5. **Status** reuses negation/uncertainty cues (`non è stato arrestato`,
   `allegedly`, `presunto`).
6. **Bounded output**: per-script cap on event count (config, default 200),
   deterministic ordering by `(first_scene_index, span start)`.

## Acceptance criteria

- [ ] Gold ≥ 25 events per language across ≥ 8 event types: type accuracy
      ≥ 90 %, participant role accuracy ≥ 90 %, date/place attachment
      precision ≥ 95 %, event recall ≥ 85 % (Pareto, documented).
- [ ] Negative set ≥ 20 per language (generic statements, metaphors
      `è morto dal ridere`, hypotheticals, trigger without any other slot,
      past-perfect background): 0 false positives.
- [ ] Merge correctness: 0 wrong merges on a 20-case confusable set (two
      arrests of different people on the same day; same person, two trials).
- [ ] Existing goldens for facts, groups, chronology byte-identical.
- [ ] Deterministic; `AnalyzerVersions.events` bumped; Go copy updated.

## Non-goals

Causal chains between events (groups.rs cause-effect stays as is), event
importance ranking, timeline layout, open-ended event types, any claim about
truth.

## Implementation steps

1. `event.rs` types and the trigger lexicon for 4 types in it/en.
2. Single-sentence frame assembly + tests.
3. Cross-scene merge + discourse-thread integration.
4. Remaining types/languages; gold and metrics.

## Prossimi passi (2026-11-19)

1. **Prerequisiti:** TICKET-31 (date), 32 (luoghi), 33 (ruoli). Senza i tre
   pezzi un evento è di nuovo un fatto sparso e non va emesso.
2. **`event.rs` + trigger lexicon** per 4 tipi in it/en (arresto, elezione,
   accordo, incidente) con i frame a frase singola.
3. **Merge cross-scena** solo sullo stesso thread di discorso, con
   deduplicazione per (tipo, soggetto, data).
4. **Gold e metriche** dopo che i tre prerequisiti sono verdi.

Gate: frame con soggetto+data+luogo corretti sul gold, nessun evento senza
trigger esplicito, nessuna duplicazione tra scene.

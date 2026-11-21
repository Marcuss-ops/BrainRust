# TICKET-35 — entity linking is not explainable and under-uses context

- Priority: P1
- Status: PARTIAL (verified 2026-11-20)
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
   "link_margin": 0.51, "link_rule": "entity_linker.v2"
   ```
   Emitted for `linked` entities too (top-3 max) so a reviewer can see near
   misses; size bounded (≤ 3 candidates, ≤ 6 features each).
5. **Script-level pass**: after scoring all mentions, ambiguous mentions inherit
   the resolved sense of the same surface in the same discourse thread; never
   across threads, never from a different type.
6. **Downstream contract**: `ambiguous`/`unlinked` keep today's behaviour
   (no identity image search); the new fields are read-only metadata.

## Acceptance criteria

- [ ] Confusable gold set ≥ 40 surfaces × 2–3 senses, ≥ 3 contexts each,
      spread over the 10 languages: **wrong-link rate ≤ 1 %**, correct-link
      recall ≥ 85 %, remainder `ambiguous` (abstention), never a silent wrong id.
- [x] 0 regressions on the existing identity/localized-identity tests
      (`entities_identity`, `localized_identity_protocol`) and on the entity
      gazetteer golden.
- [x] Every `linked` entity has `link_candidates` with at least the winning
      feature evidence; spans are byte-exact.
- [x] Weights/thresholds file is versioned, checksummed and reported by
      health/readiness.
- [x] Deterministic across worker counts; `AnalyzerVersions.linker` bumped.

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
   *(`type_cues` fatto con 35a; restano `discourse_prior` e calibrazione.)*
3. **Regola di margine e astensione:** sotto il margine → `ambiguous`, non
   `linked`; il manifest deve dire perché e con quanto margine.
4. **Propagazione a script** e suite negative, poi la copia Go.

Questo ticket sblocca TICKET-32 (coordinate) ed è prerequisito di 33/34/36:
va eseguito prima, non in parallelo.

## Avanzamento implementazione (2026-11-19)

- `LinkedEntity` include fino a tre candidati ordinati con punteggio, feature e
  span byte-exact associati a `scene_id`; `link_margin` e `link_rule` rendono
  osservabile la decisione. Le feature includono alias, prior di tipo,
  sovrapposizione di contesto e penalità.
- `lexicon/link_weights_v1.json` conserva i pesi e le soglie correnti senza
  cambiare il comportamento. Il suo SHA-256 è esposto da health e registrato
  in `AnalyzerVersions.linker`; il fingerprint incorpora la tabella.
- Il consumer Go copia i campi e convalida numero di candidati/feature e ogni
  span contro il testo originale della scena.
- Le suite identity, localized identity, MVP golden e worker deterministico
  sono verdi; il test linker verifica i riferimenti byte-exact.

Il ticket resta **PARTIAL**: il gold copre 46 superfici, ma le frasi sono
fixture brevi in inglese con alias localizzati solo in parte (`en/it/pt`), non
il campione naturale su dieci lingue richiesto. Restano il prior di discourse
per thread e la calibrazione indipendente del margine/soglia (`type_cues` è stato completato con 35a il 2026-11-20).

## Verifica 2026-11-20

Ricontrollato sul working tree corrente (`2bbeca1` più le modifiche non
committate).

- Implementato e osservabile: `LinkedEntity.link_candidates` (max 3),
  `link_margin`, `link_rule` e feature con span byte-exact
  (`semantic-core/src/entity_linker.rs`). `lexicon/link_weights_v1.json` è
  versionato, il suo SHA-256 è esposto da health e incluso nel fingerprint.
- **Fatto (35a):** `lexicon/type_cues.json` esiste (19 KiB) e i cue localizzati
  contribuiscono a `context_score`; ogni cue attivo è esposto come evidenza
  `type_cue` con span byte-exact. Il file è incluso nel fingerprint del
  contenuto e `type_cue_weight` è nella tabella dei pesi (`entity_linker.v2`).
- `cargo test -p semantic-core --lib entity_linker` — **9 passed**, incluso
  `every_confusable_index_sense_has_three_positive_and_negative_contexts`
  (46 superfici, 276 lookup positivi + 276 di contro-contesto) e il caso dei
  cue localizzati; l'alias senza contesto resta `ambiguous`.
- **35b — slice iniziale implementato:** il linker riceve i thread discourse
  dopo la risoluzione dei riferimenti; un senso risolto da un'altra menzione
  dello stesso thread e tipo aggiunge la feature `discourse_prior`, con peso
  versionato e prove byte-exact. Thread assenti, identità diverse o tipo
  diverso non trasferiscono il prior. Restano casi confondibili e la
  propagazione più ampia prevista dal gold.
- Assenti: un gold naturale su dieci lingue (35c) e la calibrazione
  indipendente di soglia e margine (35d). Il
  seed confondibile è 40 superfici inglesi più 6 collisioni localizzate, non
  il campione multilingue richiesto.
- Stato **PARTIAL** confermato: 35a chiuso; TICKET-32/33/36 hanno iniziato i
  passi che non dipendono dal link definitivo e ora sono tutti PARTIAL.

## Piano di chiusura e split (2026-11-20)

La parte osservabile è fatta; mancano le feature che usano il contesto e il
gold che permette di calibrarle. Il ticket è troppo grande per un solo
cambiamento: split in 4 parti indipendenti.

### 35a — Type cues (S, parte subito)

- Nuovo `lexicon/type_cues.json` (chiavi per lingua, cue condivisi in `*`):
  preposizione locativa → LOCATION; soggetto di verbo di parola/azione →
  PERSON; `di/of` + nome di organizzazione → ORG; indirizzo/`via` → FACILITY.
- Aggiungere la feature `type_cue` in `explain_candidate` **e** nel calcolo
  di `context_score`, con il peso in `lexicon/link_weights_v1.json` (bump
  versione + checksum). Non cambiare la regola del margine in questo passo:
  prima si misura, poi si calibra.
- Test: i 46 casi esistenti restano identici; nuovi casi per cue locativo,
  persona e organizzazione; ogni feature ha span byte-exact.
- Fatto quando: `cargo test -p semantic-core --lib entity_linker` verde e
  nessuna regressione sui golden identity/localized.

### 35b — Discourse prior (M)

- Oggi `link_all` gira prima di `discourse::build`. Due opzioni, da scegliere
  in PR: (a) primo pass di link, prior `superficie → kb_id` dai thread, secondo
  pass solo per gli `ambiguous`; (b) link → discourse → relink degli
  `ambiguous` → rebuild discourse.
- La feature `discourse_prior` vale solo dentro lo stesso thread e lo stesso
  tipo; mai cross-thread, mai cross-type. Il prior non crea un id: conferma un
  candidato già presente.
- Test: superficie ripetuta in scene diverse dello stesso thread; due thread
  con la stessa superficie ma sensi diversi restano separati; un prior non
  cambia un `unlinked`.
- Fatto quando: deterministico e con span del termine che ha attivato il
  prior.

### 35c — Gold naturale su 10 lingue (L, parte subito in parallelo)

- Costruire `semantic-engine/fixtures/linking/confusables_gold.json`: ≥40
  superfici per lingua, 2–3 sensi, ≥3 contesti positivi e 3 di contro-contesto
  per senso, alias localizzati (non solo `en/it/pt`).
- Estendere `score_long_gold.py` (o aggiungere uno scorer dedicato) con
  wrong-link rate, recall e tasso di astensione per lingua.
- Fatto quando: wrong-link ≤1%, recall ≥85%, il resto `ambiguous`, e i numeri
  sono riproducibili con `VISUALNER_WORKERS` diversi.

### 35d — Calibrazione e chiusura (M)

- Calibrare `θ_link`/`δ_margin` su 35c, congelarli in
  `link_weights_v1.json` (versione + checksum), esporli in health/readiness.
- Aggiornare la copia Go (`link_candidates`, span, margine), bumpar
  `AnalyzerVersions.linker`, rieseguire identity/localized/MVP golden.
- Fatto quando: soglie e pesi sono nel file versionato, il test confusable
  passa, e il report 35c è allegato al ticket.

Ordine: **35a e 35c in parallelo → 35b → 35d**. 35c è il collo di bottiglia:
senza gold multilingue 35d non può calibrare. 35a/35b non richiedono 36.

## Avanzamento 2026-11-20 — 35a

- Added localized `lexicon/type_cues.json` and `type_cue_weight` to the
  versioned linker weights (`entity_linker.v2`). Cue terms contribute to the
  candidate context score and are exposed as byte-exact `type_cue` evidence.
- Added the cue asset to the semantic content fingerprint, so cache identities
  change when the cue inventory changes. Existing linker thresholds are
  unchanged.
- Verified: `cargo test -p semantic-core --lib entity_linker` — **9 passed**,
  including the existing 46-surface positive/counter-context matrix and a
  localized cue span case. Discourse prior, natural 10-language gold and
  independent calibration remain open.

## Avanzamento 2026-11-21 — 35b slice thread-aware

- The script pipeline builds the discourse projection before final entity
  linking and passes its resolved threads to `link_all_with_threads`.
- A uniquely resolved candidate sense may add the versioned
  `discourse_prior` feature only when its evidence is inside a multi-scene
  thread for the same local identity and type. Conflicting local senses cancel
  the prior; it cannot create a candidate or link an unindexed alias.
- The feature exposes the supporting context term and exact scene byte span.
  Link weights were `entity_linker.v3` before whole-word evidence validation;
  see the 2026-11-21 update below for the current version. The explanation
  budget is six features.
- Unit coverage verifies the prior evidence and rejects mismatched identity
  and type; an integration test verifies that Washington resolves using its
  same-thread geographic context. Natural gold and calibration remain open.

## Matching context terms as whole words — 2026-11-21

- Context and counter-context scoring now use word boundaries, so `film` does
  not score inside `filmmaker` and `city` does not penalize `cityscape`.
  Evidence spans for those features also require a whole-word match and remain
  byte offsets into the original scene.
- Added a regression for positive whole-word evidence and both substring
  negatives. The linker weight asset is now `entity_linker.v4`, the emitted
  candidate rule is `entity_linker.v3`, and the asset checksum remains part of
  health, fingerprint and analyzer cache identity.
- `cargo test -p semantic-core --lib entity_linker --locked` — **11/11 PASS**,
  including the existing 46-surface confusable matrix. Broader independent
  gold and margin calibration remain required; the ticket stays **PARTIAL**.

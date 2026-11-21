# TICKET-34 — events are not a first-class fact

- Priority: P2
- Status: PARTIAL (verified 2026-11-21)
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
- [x] Deterministic; `AnalyzerVersions.events` bumped; Go copy updated.

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
   accordo, incidente) con i frame a frase singola. *(Fatto con 34a/34b:
   lessico, `event.rs`, frame e proiezione manifest; restano altri tipi e
   lingue.)*
3. **Merge cross-scena** solo sullo stesso thread di discorso, con
   deduplicazione per (tipo, soggetto, data). *(Slice iniziale 34c
   conservativo: partecipante condiviso + data o luogo esatti; resta il merge
   thread-aware.)*
4. **Gold e metriche** dopo che i tre prerequisiti sono verdi.

Gate: frame con soggetto+data+luogo corretti sul gold, nessun evento senza
trigger esplicito, nessuna duplicazione tra scene.

## Verifica 2026-11-20

Ricontrollato sul working tree corrente (`2bbeca1` più le modifiche non
committate).

- **Parziale (34a):** `lexicon/events/en.json`, `it.json` ed `es.json` coprono
  i 12 tipi core con voce e mapping dei partecipanti; `agreement` resta come
  tipo addizionale legacy. `python3 lexicon/tools/check_event_lexicons.py` —
  **PASS** (`en + it + es; 12 core event types each; valid`).
- **34a verificato:** lessici eventi registrati nel fingerprint; il test
  `every_compiled_lexicon_table_is_in_the_fingerprint` passa. Il controllo
  `check_event_lexicons.py` è ora incluso anche nel gate canonico
  `build_language_packs.py --check`.
- **34b/34c — slice iniziali:** `semantic-core/src/event.rs`,
  `SemanticManifest.events`, `AnalyzerVersions.events =
  event_frames.v2.cross_scene_facts` e `semantic-engine/src/events.rs`
  producono frame grounded (trigger + partecipante/data/luogo, status di
  negazione, cap 200). Otto test unitari e due test protocol passano. Il merge
  cross-scene fonde solo asserti con partecipante canonico condiviso e data
  normalizzata o luogo identici; date in conflitto restano separate e
  `source_fact_ids` collega i fatti entità sovrapposti.
- **Restano:** i 12 tipi × 11 lingue (oggi tutti i tipi sono coperti solo
  en/it/es), il merge
  thread-aware con discourse e gold/metriche indipendenti. Il consumer Go ora
  copia e valida i frame evento. Stato
  **PARTIAL**.

## Slice francese, portoghese, tedesca e olandese — 2026-11-21

- Aggiunto `lexicon/events/fr.json` con trigger attivi/passivi per tutti i 12
  tipi core; le superfici, le voci e il mapping dei partecipanti restano dati
  localizzati e il contenuto entra nel fingerprint.
- L’estrattore carica il lessico francese e conserva lo stato negato su
  `n'a pas été arrêtée` e incerto su `aurait été arrêtée`. Test runtime coprono
  tutti i tipi core, negazione e incertezza; `check_event_lexicons.py` e
  `build_language_packs.py --check` passano.
- Aggiunto `lexicon/events/pt.json` con trigger per i 12 tipi core; i test
  runtime coprono i 12 tipi, `não` come negazione e `supostamente` come
  incertezza anche prima della punteggiatura finale. I cue di status ora
  rispettano i confini di parola invece di dipendere dagli spazi.
- Aggiunto `lexicon/events/de.json` per i 12 tipi core. I test runtime coprono
  tutti i tipi, `nicht` come negazione e `angeblich` come incertezza; i
  marcatori tedeschi vengono tokenizzati senza dipendere dagli spazi attorno
  alla punteggiatura.
- Aggiunto `lexicon/events/nl.json` per i 12 tipi core. I test runtime coprono
  tutti i tipi, `niet` come negazione e `vermoedelijk` come incertezza.
- I quattro lessici sono inclusi nel fingerprint. Il test di completezza del
  fingerprint ignora anche l’alias override dei luoghi, un input del generatore
  già rappresentato dall’indice generato fingerprintato.
- Restavano quattro lingue, il gold indipendente, il merge discourse-thread-aware
  e le metriche Pareto previste dai criteri di accettazione.

## Slice indonesiana, polacca, russa e turca — 2026-11-21

- Aggiunti lessici `event_triggers.v1` per id/pl/ru/tr, ciascuno con superfici
  localizzate, voce e mapping dei ruoli per tutti i 12 tipi core.
- Il runtime carica ora lessici per tutte le 11 lingue supportate. Test runtime
  esercitano ogni tipo nelle quattro lingue nuove, oltre a cue localizzati di
  negazione e incertezza.
- I nuovi file entrano nel content fingerprint e nel gate
  `build_language_packs.py --check`; il validatore conferma 11 lingue e 12 tipi
  core ciascuna.
- La copertura dei trigger core è completa per 11/11 lingue. Restano gold
  naturale con annotazione indipendente e metriche di accettazione. Il merge
  cross-scena ora richiede anche che il partecipante condiviso abbia un thread
  discourse che copre entrambe le scene; test unitari verificano il rifiuto
  senza thread e il merge con identità/date coerenti. Restano casi confondibili
  e metriche indipendenti; il ticket resta PARTIAL.

## Piano di chiusura e split (2026-11-20)

Un evento è l'ultimo strato: dipende da date, luoghi e ruoli. Il ticket va
spezzato in 4 parti, con la prima indipendente.

### 34a — Tipi e lessico trigger (M, parte subito)

- `semantic-core/src/event.rs`: `EventFrame`, `EventType` (12 valori),
  `ParticipantRole`, `EventTime`, `EventEvidence`. Proiezione additiva
  `SemanticManifest.events`.
- `lexicon/events/<lang>.json`: per tipo, lemmi/superfici, voce attiva/passiva
  e mapping argomenti per voce (`è stato arrestato X` → patient; `X ha
  arrestato Y` → agent/patient).
- Seed it/en per 4 tipi (arrest, election, agreement, accident); nessuna
  emissione in questo passo.
- Fatto quando: schema validato e lessico generato/checkato dalla CI.

### 34b — Frame a frase singola (M, dopo 34a; 32b/33b migliorano gli slot)

- `semantic-engine/src/events.rs`: trigger → partecipanti per prossimità e voce
  → tempo dal `TemporalFact` più vicino (31) → luogo dal place con ruolo
  `setting` (32).
- Un frame richiede trigger + almeno uno tra partecipante, data o luogo;
  trigger-only viene scartato. Status da negazione/incertezza.
- Riferimenti a `entity_facts`/`groups` esistenti (`source_fact_ids`) senza
  riscriverli; cap per script (default 200), ordine deterministico.
- Fatto quando: frame it/en corretti su 4 tipi, 0 frame senza trigger, golden
  facts/groups invariati.

### 34c — Merge cross-scena (M, dopo 34b)

- Merge se stesso `event_type` e (stessa data normalizzata **o** stesso luogo)
  e almeno un partecipante condiviso/thread; date in conflitto restano eventi
  separati; nessuna media.
- Deterministico, cap e ordinamento per `(first_scene_index, span start)`.
- Fatto quando: 0 merge errati su 20 casi confondibili (due arresti di persone
  diverse lo stesso giorno; stessa persona, due processi).

### 34d — Restanti tipi/lingue, gold e chiusura (L)

- 12 tipi × 11 lingue; ≥25 eventi per lingua su ≥8 tipi; ≥20 negativi
  (metafore `è morto dal ridere`, ipotetici, trigger senza altri slot).
- Metriche: type ≥90%, participant role ≥90%, date/place precision ≥95%,
  recall ≥85%. La copia Go e `AnalyzerVersions.events` sono aggiornati; stato
  CLOSED solo al superamento dei gate gold/multilingue.
- Fatto quando: tutti i gate + suite verdi.

Ordine: **34a → 34b → 34c → 34d**. 34a parte subito; 34b può emettere con
data/partecipante prima che 32/33 siano chiusi, ma il gate completo richiede
entrambi.

## Avanzamento 2026-11-20 — 34a

- Added language-scoped `event_triggers.v1` seed lexicons for English and
  Italian with trigger, lemma, voice, and participant-role mappings for
  arrest, election, agreement, and accident/disaster.
- Added `lexicon/tools/check_event_lexicons.py` to validate the initial event
  types and trigger shapes. Verified: `python3
  lexicon/tools/check_event_lexicons.py` — PASS.
- The English/Italian trigger assets are included in the semantic content
  fingerprint; `cargo test -p semantic-core --lib
  content_fingerprint::tests::every_compiled_lexicon_table_is_in_the_fingerprint`
  passes.
- 34a e lo slice iniziale 34b sono completati; il merge 34c è coperto da
  un'implementazione conservativa tra scene. Altri tipi/lingue, gold e
  calibrazione restano aperti; la parity Go è stata completata e verificata.


## Avanzamento 2026-11-20 — single-sentence frames (34b initial slice)

- Aggiunti i tipi condivisi in `semantic-core/src/event.rs`, la proiezione
  additiva `SemanticManifest.events`, `AnalyzerVersions.events` e il linker
  `semantic-engine/src/events.rs`. Il runtime usa i trigger en/it, voce e
  mapping di ruolo del lessico; partecipanti provengono solo da entità
  grounded, tempo dalla data più vicina nella frase, luogo da una place `setting`.
- Scarta i trigger senza slot aggiuntivi, conserva negazione/incertezza,
  ordina deterministicamente e limita l'output a 200 frame per script.
- Sette test unitari coprono arresto attivo/passivo, slot data/luogo, elezione,
  accordo, incidente, negazione, trigger-only e cap; un test protocol verifica
  la proiezione end-to-end. Clippy e il fingerprint content gate passano.
- I frame asserted si fondono tra scene solo con partecipante canonico comune
  e data normalizzata identica o luogo canonico identico. Date esplicite in
  conflitto bloccano il merge; eventi negati/incerti e persone diverse restano
  separati.
- `source_fact_ids` collega i fatti d'entità la cui evidenza è nello span della
  menzione evento e il cui soggetto o target è un partecipante. La versione
  analyzer/cache è `event_frames.v2.cross_scene_facts`.
- Otto test unitari eventi e due test protocol evento passano. Restano più
  tipi/lingue, gold e merge completo con discourse threads.

## Verifica workspace 2026-11-20

- `cargo test --workspace` e `cargo clippy --workspace --all-targets -- -D
  warnings` — **PASS**.
- `python3 lexicon/tools/build_language_packs.py --check` valida anche i trigger
  evento en/it, oltre a pack e snapshot Go — **PASS**.
- Restano i tipi e le lingue non coperte, gold indipendente e merge discourse-
  aware; il ticket resta **PARTIAL**.
- PipelineGen ora decodifica e proietta i frame evento in ogni bundle, validando
  trigger/evidenza su span sorgente, scene e identità partecipanti. Il test
  `TestSemanticEngineAdapterCarriesGroundedPlacesAndEventsIntoGoBundle` esercita il
  passaggio end-to-end del frame. `go test
  ./internal/platform/media/rustexec` — **PASS**.

## Estensione lessicale 2026-11-20

- I pack `en`, `it` ed `es` ora hanno trigger per tutti i 12 tipi core: birth,
  death,
  arrest, trial verdict, election, appointment/resignation, founding,
  acquisition/merger, launch/release, accident/disaster, attack/battle e
  match/competition result. `agreement` resta come tipo addizionale legacy.
- Il checker richiede i 12 tipi core nei tre pack; test runtime verificano un
  frame con partecipante grounded per ciascun tipo in ogni lingua (30 casi).
  `check_event_lexicons.py`,
  `build_language_packs.py --check` e `cargo test -p semantic-engine events::
  --locked` passano.
- Questo completa la copertura dei tipi nei tre seed, non la matrice 12×11 né
  l'acceptance gold: le altre otto lingue, i falsi positivi, il merge
  discourse-aware e la revisione indipendente tengono il ticket **PARTIAL**.
- Dopo l'aggiunta dello spagnolo, `cargo test --workspace --tests --locked`
  è passato nell'intero workspace; `cargo clippy -p semantic-engine
  --all-targets --locked -- -D warnings`, `cargo fmt --all -- --check`,
  `build_language_packs.py --check` e `git diff --check` passano.

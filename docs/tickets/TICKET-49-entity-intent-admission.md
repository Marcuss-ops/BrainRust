# TICKET-49 — Entity intents: contratto di ammissione e igiene dell'insieme

**Data:** 2026-11-22
**Priorità:** P1
**Repo:** `BrainRust` (planner/estrazione) + `refactored` (lineage dell'audit)
**Stato:** DONE (bound/Rust) (2026-11-24) — implementati il gate di estrazione (niente DATE/NUMBER/slug/UUID negli intenti entità), il join di lineage per identità canonica, l'ammissione planner di una card per identità canonica per scena, la metrica di concordanza `image_status`/`overlay_timing`, il conteggio per identità e la policy ORG/GPE. Resta esterno solo il criterio che richiede un run Augias reale (Master/render).

## Sintomo misurato

Run Augias R30 (`job_1795360130568426251_81398905`, SUCCEEDED), audit
`overlay_timing`:

```
authored_intents 88 · frozen_intents 31 · unresolved_intents 57
by_source {entity: 54, important_phrase: 34}
reasons {entity_not_planned: 36, source_text_not_in_plan: 21}
```

36 intenti entità non diventano un item temporizzato, su 5 scene
(`scene-01 7 · scene-02 4 · scene-03 7 · scene-04 12 · scene-05 6`).
Nello stesso run l'image contract dichiara 4 entità bound su 4 occorrenze e
`missing 0`, quindi **non si tratta di byte mancanti**: è un disallineamento fra
ciò che il piano ha deciso e ciò che il lineage conta come risolto.

## Diagnosi (dai dati, non per ipotesi)

Dei 36 intenti non risolti, 35 entità distinte:

| Classe | N | Esempi | Chi deve cambiare |
| --- | --- | --- | --- |
| date e anni | 16 | `26 gennaio 1935`, `2024`, `1960`, `1987`, `1993`, `1990`, `2023`, `2006`, `2009`, `2010`, `2026`, `19 novembre`, `novembre 2026`, `21 novembre`, `26 gennaio`, `gennaio 1935` | estrazione: le DATE non devono autore intenti entità (sono lane numerica) |
| numeri e quantità | 7 | `7` (×2), `6.000 lettere`, `Novanta`, `100 storie`, `quattro`, `91`, `19` | estrazione: NUMBER → lane numerica, non card entità |
| frammenti di slug/UUID | 8 | `addio-a-corrado-augias-…_57daaef8-24d4-444a-913b-7088122eeb88`, `57`, `8-24`, `4`, `444`, `913`, `7088122`, `88` | estrazione: bug di tokenizzazione su slug/URL (il tipo assegnato è NUMBER) |
| persone/org/luoghi reali | 8 | `Corrado Augias`, `Mauro Pesce`, `Vito Mancuso`, `Rai` (×3), `La7` (×2), `New York`, `Libia`, `Einaudi` | planner + lineage: ammissione vera |

Due fatti che chiudono la diagnosi:

1. **Lo stesso run si contraddice.** `Corrado Augias` compare in
   `detected_entities` con `image_status: planned`, `selected_for_video: true`
   e `image_required: true` in **tutte e 5 le scene**, mentre il piano ha **un
   solo** item `entity_image` (`image-fade-reveal__corrado-augias__entity_image__01`,
   solo `scene-05`) e il lineage riporta `intent-scene-01-corrado-augias →
   entity_not_planned`. Due regole di matching diverse sullo stesso dato: il
   contract immagini risolve per entità, il lineage per testo/scena.
2. **Il piano non ha bisogno di 54 card.** Budget immagini 25, usato 11,
   `selected_entities 4`, `bound 4`: la lane entità è già sostenibile. Il
   problema non è la capienza, è che 23 dei 36 intenti non sono entità e i
   restanti 8 non trovano il loro item.

## Decisione di contratto

**Una card per entità per scena, con le occorrenze come span sulla stessa
card.** Non una card per occorrenza.

Motivazione:

- **Editoriale:** lo stesso ritratto ripetuto 5 volte in 90 secondi è un errore
  di montaggio, non una copertura. R30 lo mostra già: una card Augias in
  `scene-05`, riusata dalle altre scene.
- **Budget:** 54 intenti entità su un budget di 25 item immagine. La lettura
  "card per occorrenza" satura il budget con lo stesso volto.
- **Semantica del dato:** l'entità è identificata da `entity_id`
  (`ent_8795a002f487cd34` per Augias), stabile fra scene; l'occorrenza è la
  coppia `(entity_id, scene_id, start_us, end_us)`. Il piano deve legare
  l'identità, il timing resta dell'occorrenza.
- **Audit:** un intento entità è risolto se la sua scena porta una card per
  quell'`entity_id`; il numero di occorrenze non entra nel verdetto.

Conseguenza sul lineage: `entity_not_planned` significa "la scena non ha una
card per questa identità", non "il testo dell'item non coincide". È l'unica
lettura compatibile con l'image contract già in produzione.

## Lavoro richiesto

1. **Estrazione — non autorare entità per DATE/NUMBER.** Gli intenti entità
   nascono solo da `PERSON`, `ORG`, `GPE`, `FAC`, `EVENT` (la tassonomia già
   usata da `role_of`); date e quantità vanno alla lane numerica. Atteso su R30:
   36 → 13 intenti.
2. **Estrazione — stop alla tokenizzazione di slug/URL.** Un testo che contiene
   `_<uuid>` o un path non deve produrre entità, e i suoi frammenti numerici non
   devono diventare `NUMBER`. Atteso: spariscono i 7 frammenti e lo slug.
3. **Planner — ammissione per identità.** `entity_id` è la chiave della card;
   la stessa identità in più scene produce una card per scena (o una card
   riusata) e le occorrenze come span, senza duplicare l'asset.
4. **Lineage — match per `entity_id`.** In `refactored`, il calcolo degli
   intenti non risolti deve usare l'identità legata al plan item, non il testo.
   R30 è il caso di prova: `intent-scene-01-corrado-augias` deve risultare
   risolto perché la card Augias esiste, anche se il suo testo non è in
   `scene-01`.
5. **ORG/GPE: decidere una volta.** `Rai` (3 scene), `La7` (2), `New York`,
   `Libia`, `Einaudi` non ricevono card. ORGANIZATION ha già 4 item pianificati
   in R30, quindi la ricetta esiste: o la si applica a tutte le org/gpe
   ammesse, o si dichiara esplicitamente che solo PERSON ha diritto alla card
   entità e l'audit riporta `entity_card_not_in_policy` invece di
   `entity_not_planned`.

## Implementazione 2026-11-23

| Pezzo | Dove | Test |
| --- | --- | --- |
| Gate di ammissione: `DATE`/`TIME`/`NUMBER`/`MONEY`/`PERCENT` e slug/UUID non autorano intenti entità | `refactored/internal/capabilities/scripts/editing_timeline.go` (`entityIntentEligible`, applicato in `sceneEntityInput`) | `TestSceneEntityInputDropsNumericAndSlugIntentSources`, `TestEntityIntentEligibleRejectsNumericFamiliesAndSlugUUID` |
| Join di lineage per identità: `CanonicalEntityID` confrontato con `OverlayItem.EntityID` e `EntityRef.CanonicalEntityID` prima del fallback per nome | `refactored/internal/capabilities/scripts/overlay_plan_freeze.go` (`intentMatchesEntityItem`) | `TestFreezeOverlayIntentsJoinsByCanonicalEntityID` |
| Ammissione planner: una card per identità canonica per scena, con fallback all'`entity_id` quando il canonico è vuoto; le occorrenze restano span | `BrainRust/overlay-compiler/src/editorial_plan_v2/planner/media.rs` (`entity_card_identity`, `append_entity_cards`) | `r30_entity_cards_are_deduplicated_by_canonical_identity_per_scene` |

Il gate usa `entitytype.Fold`, quindi `GPE`/`FAC`/`LOCATION` restano eleggibili e
le famiglie numeriche sono escluse una volta sola. `PERSON`/`ORG`/`PLACE`/`EVENT`
e le lane non numeriche già esistenti continuano a passare; nessun tipo nuovo
viene inventato.

Residuo non eseguibile da questo checkout:

1. concordanza su R30 fra `image_status` e `overlay_timing.reasons` per lo
   stesso `entity_id` e metrica `selected_entities` che non cresce con le
   occorrenze;
2. prova su un run Augias reale (`entity_not_planned` PERSON = 0), oggi
   dipendente dal Master (CODEX-04) e dal render.

Verifiche eseguite il 2026-11-23 sul tree `refactored`:

| Comando | Esito |
| --- | --- |
| `go test ./internal/capabilities/scripts/ -run 'TestFreezeOverlayIntents\|TestSceneEntityInputDropsNumeric\|TestEntityIntentEligibleRejects' -count=1` | **PASS** |
| `go test ./internal/capabilities/overlays/... ./internal/capabilities/scripts/ -count=1` | **PASS** — 3 package ok |
| `go vet ./internal/capabilities/scripts/ ./internal/capabilities/overlays/...` | **rc 0** |
| `cargo test -p overlay-compiler --locked` | **PASS** — 166 unit + 33 contract |
| `cargo fmt --all -- --check` | **rc 0** |

## Criteri di accettazione

- [ ] su un run Augias, `entity_not_planned` per PERSON = **0** (tutte le
      occorrenze di Augias/Pesce/Mancuso risolte, anche fuori dalla scena della
      card)
- [ ] `entity_not_planned` complessivo ≤ 8 e ogni residuo ha un motivo
      dichiarato. Decisione 2026-11-24: PERSON, ORG, PLACE (GPE/FAC), CONCEPT ed
      EVENT sono **tutti** nella lane card del planner
      (`entity_card_kind`), quindi nessuna identità è fuori policy e
      `entity_card_not_in_policy` non viene emesso; un residuo ORG/GPE è una
      perdita di ammissione, contata in `intake.dropped_by_reason`.
- [x] nessun intento entità con tipo `DATE`/`NUMBER` — gate `entityIntentEligible`
      applicato alla proiezione degli intenti; copre anche `TIME`/`MONEY`/`PERCENT`
- [x] nessuna entità che contenga `_` seguito da un UUID, nessun frammento
      numerico di slug (`57`, `8-24`, `444`, `913`, `7088122`, `88`) — regex UUID
      nel gate; i frammenti numerici sono esclusi dalla famiglia `NUMBER`
- [x] due occorrenze della stessa entità nella stessa scena non producono due
      asset distinti: `selected_entities` raggruppa per identità canonica
      (`auditEntityImageGroupKey`), non per occorrenza. Coperto da
      `TestOverlayIntentLineageCoversIdentityReusedAcrossScenes` e dal dedupe
      planner `r30_entity_cards_are_deduplicated_by_canonical_identity_per_scene`
- [x] `image_status` e `overlay_timing.reasons` concordano sullo stesso
      `entity_id`: l'audit pubblica `entity_concordance.checked/mismatched` e
      un intento con card altrove è `covered`, non non risolto. Coperto da
      `TestEntityConcordanceFlagsPlannedImageWithUnplannedIntent`
- [x] fixture di contratto: R30 ridotto a `entity_id` + occorrenze, con il caso
      Augias cross-scene come regressione — `tests/fixtures/r30-entity-admission.json`,
      test `r30_entity_cards_are_deduplicated_by_canonical_identity_per_scene`

## Implementazione 2026-11-24 — concordanza, metrica e policy

| Pezzo | Dove | Test |
| --- | --- | --- |
| Copertura run-level: un intento entità è risolto se la stessa identità canonica ha una card in un'altra scena (la card è riusata, il timing resta della sua scena) | `refactored/internal/capabilities/scripts/overlay_plan_freeze.go` (`entityIntentCoveredByIdentity`, `Covered`) | `TestOverlayIntentLineageCoversIdentityReusedAcrossScenes` |
| Metrica di concordanza `entity_concordance` con `checked`, `mismatched` e l'elenco dei conflitti; un'immagine `planned` più un intento non risolto sulla stessa identità è un errore di misura | `refactored/internal/capabilities/scripts/overlay_audit.go` (`buildOverlayAuditEntityConcordance`) | `TestEntityConcordanceFlagsPlannedImageWithUnplannedIntent` |
| `selected_entities` conta identità, non occorrenze: la chiave di gruppo preferisce `canonical_entity_id` | `refactored/internal/capabilities/scripts/overlay_audit_images.go` (`auditEntityImageGroupKey`) | test cross-scene sopra (una sola promise per due occorrenze) |
| Contratto additivo: `covered_intents`, `entity_id`/`canonical_entity_id` sugli item non risolti, `entity_concordance` | `refactored/internal/capabilities/scripts/contract/overlay_audit*.go` | round-trip JSON dei campi nel pacchetto `scripts` |
| Policy ORG/GPE: tutte le famiglie con card (`PERSON`, `ORG`, `PLACE`, `CONCEPT`, `EVENT`) sono eleggibili; nessun `entity_card_not_in_policy` | `BrainRust/overlay-compiler/src/editorial_plan_v2/planner/media.rs` (`entity_card_kind`) | `TestOverlayIntentLineageCoversIdentityReusedAcrossScenes` (ORG coperta) |

Verifica: `go vet ./internal/capabilities/scripts/` → **rc 0**;
`go test ./internal/capabilities/scripts/ -count=1` → **PASS** (24,3 s), inclusi i
test mirati `TestOverlayIntentLineageCovers|TestEntityConcordance|TestFreezeOverlayIntents|TestEntityIntentEligible|TestSceneEntityInput`.

## Rischi / rollback

Il cambio 4 (match per identità) può mascherare una card mancante: per questo
il criterio chiede che `image_status` e lineage concordino sullo stesso
`entity_id`, non solo che il conteggio scenda. Rollback: tornare al match per
testo, che riporta R30 a 36 non risolti e lascia l'incoerenza visibile.

## Evidenza

- `ops/jobs/augias_biography_20261121/r30-final.json` →
  `result.result.overlay_audit.overlay_timing` (36 voci, `unresolved_items`) e
  `overlay_audit.detected_entities` (48 entità, `image_status`).
- `refactored/internal/capabilities/scripts/overlay_audit_images.go` —
  `OverlayIntentReasonEntityNotPlanned` e la proiezione del lineage.
- Ticket collegati: ENT-03 (lane ritratti, chiuso in
  `docs/tickets/augias-delivery-2026-11-22/03-ENTITY-PORTRAIT-LANE.md`).

# BR-AUDIT-21 / BR-67 — Eliminare il clone completo della richiesta

- **Priorità:** P1
- **Stato:** PARTIAL (2026-11-27): clone rimosso dal percorso di produzione; manca la misura allocazioni/tempo
- **Owner:** `overlay-compiler`
- **Origine:** audit 2026-11-26 §9.1, ticket proposto BR-67.

## Problema

`overlay-compiler/src/editorial_plan_v2/planner/mod.rs:107` inizia con:

```rust
let mut projected_request = request.clone();
```

Per un documento grande il clone copia l'intera richiesta editoriale: scene,
annotazioni, timing, media e metadati. È una copia profonda che esiste solo per
poter mutare la proiezione, non perché il chiamante debba conservare
l'originale.

## Cosa serve per risolverlo

1. **Mappare i campi effettivamente mutati** da `compile_plan` dopo il clone:
   quali vengono solo letti, quali vengono proiettati, quali vengono riscritti.
2. **Scegliere la forma di ownership:**
   - `compile_plan(request: CompileEditorialPlanRequest)` per valore, quando il
     chiamante non riusa la richiesta; oppure
   - una vista `ProjectedRequest<'a>` che prende in prestito le parti immutabili
     e possiede solo quelle proiettate.
3. **Rimuovere il clone** dal percorso di produzione; se l'API pubblica deve
   restare compatibile, aggiungere un wrapper che consuma la richiesta e
   deprecare quello per riferimento.
4. **Misurare** allocazioni, tempo di compilazione e picco RSS su una richiesta
   grande (es. 100 scene con media e annotazioni) prima/dopo.
5. **Verificare la parità** dei piani sigillati: i golden dell'overlay contract
   devono restare byte-identici.
6. **Non introdurre `unsafe`** né lifetime che rendano il planner più difficile
   da leggere: se la vista borrowata complica troppo, preferire il consumo per
   valore.

## Criteri di accettazione

- [x] Nessun `request.clone()` nel percorso di produzione del planner.
- [x] I golden dei piani restano byte-identici.
- [ ] Allocazioni, tempo e RSS sono misurati su una richiesta grande.
- [x] L'API pubblica è documentata (consumo per valore o vista borrowata).
- [x] Nessun `unsafe` aggiunto.
- [x] Test del planner e contract suite verdi.

## Esito 2026-11-27

- `planner::compile_plan_owned(EditorialPlanV2Request)` proietta la richiesta
  in place: nessuna copia profonda di scene, annotazioni, timing e media.
- `compile_overlay_plan_v2_owned` è l'entry point di produzione; il worker
  NDJSON deserializza una richiesta posseduta e la passa per valore.
- `compile_plan(&request)` resta come wrapper di compatibilità documentato per
  test ed embedder che non possono cedere l'ownership.

Verifica eseguita:

```sh
cargo test -p overlay-compiler --lib
cargo test -p overlay-compiler --test overlay_plan_contract
cargo clippy -p overlay-compiler --all-targets -- -D warnings
cargo test -p semantic-engine --lib protocol
```

## Verifica

```sh
cd BrainRust
cargo test -p overlay-compiler --lib
cargo test -p overlay-compiler --test overlay_plan_contract
cargo clippy -p overlay-compiler --all-targets -- -D warnings
```

## Dipendenze / rischi

- Il chiamante NDJSON deserializza già una richiesta posseduta: il consumo per
  valore è probabilmente il percorso più semplice e va preferito se non ci sono
  altri consumatori.
- Un guadagno di memoria non misurato non giustifica il cambio di firma: il
  ticket si chiude solo con la misura.

# BR-AUDIT-24 / BR-70 — Ritirare le operazioni NDJSON granulari

- **Priorità:** P2
- **Stato:** CLOSED (2026-11-27)
- **Owner:** `semantic-engine`, `overlay-compiler`, PipelineGen
- **Origine:** audit 2026-11-26 §10.1, ticket proposto BR-70.

## Problema

Il worker espone ancora quattro operazioni che il planner completo svolge
internamente:

- `resolve_map_overlaps`;
- `trim_map_windows_before_budget`;
- `rank_entity_overlays`;
- `cascade_clustered_entrances`.

Se PipelineGen non le chiama più, il worker mantiene una seconda API con cui un
consumer potrebbe ricomporre il piano pezzo per pezzo, aggirando le regole
dell'editorial planner. Questo è debito di contratto, non solo codice non
usato.

## Cosa serve per risolverlo

1. **Censire le chiamate reali** da PipelineGen: grep dei nomi operazione,
   metriche/log del worker, e ricerca nei job di produzione. Il risultato va
   allegato al ticket.
2. **Distinguere implementazione interna e superficie wire.** Le funzioni usate
   dal planner restano; le operazioni NDJSON che nessuno chiama vengono rimosse.
3. **Rimuovere le braccia di dispatch** in `Worker::handle_line` e i relativi
   test di protocollo, oppure marcarle deprecate con una data di rimozione e un
   owner se un consumer esterno le usa.
4. **Aggiornare la documentazione del protocollo** e il contratto condiviso.
5. **Verificare il fail-closed**: un'operazione rimossa deve restituire
   `unknown operation` / `UnsupportedOperation`, non un successo vuoto.
6. **Misurare l'effetto**: numero di operazioni esposte, righe di protocollo,
   test rimossi. Non serve un benchmark, serve la prova che il wire è più
   piccolo.

## Criteri di accettazione

- [ ] Il censimento dei chiamanti è nel ticket, con comando e data.
- [ ] Ogni operazione senza chiamanti di produzione è rimossa dal dispatch.
- [ ] Le implementazioni interne usate dal planner restano e sono coperte.
- [ ] Un'operazione rimossa fallisce in modo esplicito.
- [ ] La documentazione del protocollo elenca solo le operazioni vive.
- [ ] Nessun test viene cancellato se copre ancora un comportamento del
      planner: va spostato sul percorso interno.

## Verifica

```sh
cd BrainRust
grep -rn "resolve_map_overlaps\|trim_map_windows_before_budget\|\
rank_entity_overlays\|cascade_clustered_entrances" --include=*.rs --include=*.go .
cargo test -p semantic-engine protocol
cargo test -p overlay-compiler --lib
```

## Esito 2026-11-27

Censimento (workspace root, 2026-11-27):

```sh
grep -rn "resolve_map_overlaps\|trim_map_windows_before_budget\|\
rank_entity_overlays\|cascade_clustered_entrances" --include=*.go refactored
```

Nessun producer Go: le uniche occorrenze sono commenti in
`refactored/internal/capabilities/overlays/planner_input.go` e
`model/chain_coalesce.go`, non nomi di operazione inviati al worker. I consumer
Go reali inviano `analyze_script`, `compile_editorial_plan`,
`compile_overlay_plan_v2`, `select_phrases`, `split_sentences`,
`lexicon_profile` e `subtitles_kernel`.

Modifiche:

- Rimosse le quattro braccia di dispatch in `Worker::handle_line`.
- Rimosse le funzioni `*_line` in `semantic-engine/src/protocol.rs`.
- Le funzioni interne (`resolve_map_overlaps`, `trim_map_windows_before_budget`,
  `rank_entity_overlays`, `cascade_clustered_entrances`) restano: il planner
  completo le usa e `overlay-compiler/src/lib_tests.rs` le copre.
- Un'operazione rimossa ora risponde `ok:false` con
  `error_code:"unsupported_operation"`; il test
  `granular_planner_operations_are_retired` lo verifica per tutte e quattro.

Verifica eseguita:

```sh
cargo test -p semantic-engine --lib protocol   # 21 passed
cargo test -p overlay-compiler --lib           # verde
cd refactored && go test ./internal/platform/media/rustexec/... -count=1
# ok 71.919s
```

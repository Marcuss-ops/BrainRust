# BR-AUDIT-03 — Fallimento GLiNER esplicito

- **Priorità:** P0
- **Stato:** CLOSED
- **Owner:** `visualner`
- **Origine:** audit 2026-11-26, finding P0 su `extractor/ner.rs`.

## Problema

`extract_candidates` convertiva ogni errore di `extract_long_with` in
`Vec::new()`. Il worker restituiva `Ok((true, vec![]))`, quindi il chiamante
considerava il modello attivo e `build_candidate_pool` filtrava i candidati
euristici contro un insieme vuoto, perdendo entità. Il risultato degradato
poteva anche essere memorizzato nella cache.

## Soluzione

`extract_candidates` restituisce `Result<Vec<Candidate>, String>`. L'errore di
inferenza viene propagato attraverso il canale del worker fino a
`candidates()`, e da lì a `analyze_local_scene`, che produce una scena
`rejected` con il motivo esplicito. Non esiste più un falso successo vuoto.

## Criteri di accettazione

- [x] Un errore di inferenza non produce `Ok((true, vec![]))`.
- [x] Il motivo dell'errore contiene "GLiNER2.5 inference failed".
- [x] Un modello assente/illeggibile produce un errore esplicito.
- [x] Test di regressione sul percorso di errore.

## Verifica

```sh
cd BrainRust
cargo test -p visualner ner
```

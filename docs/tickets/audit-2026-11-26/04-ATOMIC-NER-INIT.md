# BR-AUDIT-04 — Inizializzazione atomica del worker NER

- **Priorità:** P0
- **Stato:** CLOSED
- **Owner:** `visualner`
- **Origine:** audit 2026-11-26, finding P1 sulla race di cold start.

## Problema

`worker()` controllava `NER_WORKER`, creava un thread e solo dopo registrava il
sender con `get_or_init()`. Più chiamanti concorrenti potevano creare più
thread, ognuno dei quali eseguiva `load_model()` prima di accorgersi di essere
il perdente. Il risultato era caricamento duplicato del modello e picco di RAM.

## Soluzione

Il valore del `OnceLock` è ora `Result<Sender<ModelRequest>, String>` e
l'inizializzazione (canale + spawn del thread) avviene **dentro** la closure di
`get_or_init`. La closure viene eseguita una sola volta; i chiamanti
concorrenti ricevono lo stesso sender o lo stesso errore.

## Criteri di accettazione

- [x] La closure di inizializzazione viene eseguita una sola volta sotto
      concorrenza.
- [x] Un errore di spawn è memorizzato e restituito a tutti i chiamanti.
- [x] `status().worker_started` riflette un worker avviato con successo.
- [x] Test concorrente sul primitivo di inizializzazione.

## Verifica

```sh
cd BrainRust
cargo test -p visualner ner
```

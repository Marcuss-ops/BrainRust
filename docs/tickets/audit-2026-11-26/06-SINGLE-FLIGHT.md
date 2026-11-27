# BR-AUDIT-06 — Single-flight delle analisi concorrenti

- **Priorità:** P1
- **Stato:** CLOSED
- **Owner:** `semantic-engine`
- **Origine:** audit 2026-11-26, finding P1 su `protocol/worker.rs`.

## Problema

Due richieste identiche concorrenti potevano entrambe mancare la cache ed
eseguire la stessa analisi. Il worker aveva solo cache memoria/disco, non una
mappa delle analisi in corso.

## Soluzione

`Worker` mantiene una mappa `in_flight` protetta da mutex. Il primo chiamante
diventa leader ed esegue l'analisi; gli altri attendono su una `Condvar` e
ricevono lo stesso `Arc<SemanticManifest>` o lo stesso errore. Il leader
rimuove l'entry dopo aver notificato. Un guard di `Drop` completa con errore se
il leader termina in modo imprevisto, evitando attese infinite.

## Criteri di accettazione

- [x] Richieste identiche concorrenti eseguono una sola analisi.
- [x] Tutti i chiamanti ricevono lo stesso esito (successo o errore).
- [x] La mappa non cresce oltre il numero di richieste in volo.
- [x] Test concorrente con contatore delle analisi.

## Verifica

```sh
cd BrainRust
cargo test -p semantic-engine single_flight
cargo test -p semantic-engine protocol
```

## Nota 2026-11-27

La suite completa ha fatto emergere due cose:

1. Il test concorrente era sensibile allo scheduling: senza una barriera un
   thread poteva partire dopo che il leader aveva già chiuso il flight e
   diventare legittimamente un secondo leader. Ora gli 8 thread si
   sincronizzano su una `Barrier` prima di entrare e il leader trattiene il
   flight per 500 ms.
2. Restava una finestra reale: un chiamante poteva mancare la cache, poi
   arrivare alla mappa dopo che il leader aveva rimosso l'entry, ed eseguire
   una seconda analisi. Il leader ora ricontrolla la cache dentro lo slot di
   leadership (`ManifestCache::peek_encoded`, che non altera i contatori
   hit/miss) e riusa il risultato appena scritto. In quel caso la risposta
   riporta `cached: true`.

La finestra è chiusa per costruzione, ma non esiste un test deterministico
   dell'interleaving esatto: servirebbe un hook di test tra il lookup e la
   mappa. Il test concorrente copre il caso simultaneo, non quello sequenziale
   ritardato.

# BR-AUDIT-13 / BR-73 — Memoria del gazetteer e degli indici

- **Priorità:** P3
- **Stato:** DEFERRED
- **Owner:** `semantic-core`
- **Origine:** audit 2026-11-26, finding P2 sul gazetteer da 19 MB.

## Problema

`entity_gazetteer.json` pesa 19.003.616 byte (~60% del repository a
`accb6d30`). Non è chiaro quanto occupino contemporaneamente JSON embedded,
strutture deserializzate, indici hash e copie normalizzate.

## Perché è deferred

Un formato binario compatto o l'interning delle stringhe va deciso dopo il
profiling RSS. Introdurlo senza misura aumenta complessità e latenza.

## Cosa serve per risolverlo

1. Profilare RSS per sezione su un corpus lungo: JSON embedded, strutture
   deserializzate, indici hash delle località, indici alias, indici identità,
   stringhe normalizzate.
2. Misurare il costo di `place_index`, alias e canonical con un heap profiler,
   non con la sola dimensione del file.
3. Eliminare le rappresentazioni ridondanti che il profilo dimostra doppie
   (es. due indici della stessa chiave, copie normalizzate non necessarie).
4. Valutare `Arc<str>`/interning o un formato binario generato in build **solo
   se** il profiling lo giustifica; il digest del file resta nella cache
   identity.
5. Non introdurre un database esterno: i lookup sono locali e la latenza
   peggiorerebbe.

## Criteri di accettazione

- [ ] Il profilo RSS per sezione è allegato al ticket.
- [ ] Le rappresentazioni ridondanti individuate sono rimosse o motivate.
- [ ] Un eventuale formato binario ha digest, builder offline e gate di
      verifica.
- [ ] La latenza di lookup non peggiora.
- [ ] La cache identity cambia se cambia il contenuto del gazetteer.

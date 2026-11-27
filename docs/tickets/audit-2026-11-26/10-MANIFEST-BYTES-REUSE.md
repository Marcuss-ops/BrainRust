# BR-AUDIT-10 / BR-63 — Riuso dei byte canonici del manifest

- **Priorità:** P2
- **Stato:** CLOSED (2026-11-27)
- **Owner:** `semantic-engine`
- **Origine:** audit 2026-11-26, finding P1 sulla serializzazione ripetuta.

## Problema

Nel percorso cold il worker serializza il manifest con `serde_json::to_vec()`
per le cache, poi `manifest_response()` lo riconverte in `serde_json::Value`
con `json!` e infine in stringa con `.to_string()`.

## Perché è deferred

Un envelope tipizzato con `serde_json::value::RawValue` richiede di conservare
i byte canonici nella cache memoria e di cambiarne il tipo di ritorno; il
guadagno va misurato su manifest grandi. Il rischio di alterare l'ordine o
l'escaping del wire è più alto del beneficio finché non c'è una misura.

## Cosa serve per risolverlo

1. Conservare `Arc<str>`/`Arc<[u8]>` in `CacheEntry`, così i byte canonici
   scritti su disco sono gli stessi che la risposta può riusare.
2. Costruire la risposta con un envelope tipizzato che usa
   `serde_json::value::RawValue` per il manifest e serializza `completeness` a
   parte, senza un albero `serde_json::Value` intermedio.
3. Gestire il percorso memory hit, disk hit e cold con la stessa funzione, così
   l'envelope non diverge.
4. Misurare allocazioni e wall time su un manifest grande, cold vs cache hit.
5. Test byte-for-byte della risposta cold vs cache hit, incluso l'ordine delle
   chiavi e l'escaping.

## Criteri di accettazione

- [x] Il percorso cold non costruisce un `serde_json::Value` intermedio del
      manifest.
- [x] La risposta riusa i byte canonici già serializzati per la cache.
- [x] Cold e cache hit producono lo stesso envelope a meno di `cached`.
- [x] Il test byte-for-byte è verde.
- [x] Il guadagno è misurato (`response_encoding_cost`, vedi Esito).

## Esito 2026-11-27

- `CacheEntry` conserva `encoded: Arc<[u8]>` accanto al manifest.
- `ManifestCache::insert_encoded`/`get_encoded` riusano i byte prodotti una
  sola volta; `insert`/`get` restano come wrapper per test ed embedder.
- `manifest_response_bytes` costruisce la risposta per splice, senza albero
  `serde_json::Value`. La usano il percorso cold, la cache memoria e la cache
  disco, quindi i tre corpi sono identici a meno del flag `cached`.
- L'ordine delle chiavi dell'oggetto risposta segue ora l'ordine dei campi
  serde del manifest invece dell'ordine alfabetico di `json!`. Il JSON resta
  semanticamente equivalente e i consumer decodificano per chiave; il test
  cold/hit lo fissa byte per byte.

Verifica eseguita:

```sh
cargo test -p semantic-engine --lib protocol
# 21 passed, 0 failed; include
# cold_and_cached_responses_reuse_the_same_manifest_bytes
```

Misura (`response_encoding_cost`, manifest di `it_stadium.txt`, 413.633 byte,
20 iterazioni dopo warm-up, build release, host locale 2026-11-27):

```
ENCODING_PROBE manifest_bytes=413633 old_json_us=27147.1 spliced_us=259.6
```

La costruzione della risposta scende da ~27,1 ms a ~0,26 ms per manifest
(~104x). La misura copre solo la costruzione dell'envelope, non l'analisi.

# BR-AUDIT-20 / BR-65 — Profilare la coda e l'inferenza GLiNER

- **Priorità:** P1
- **Stato:** OPEN
- **Owner:** `visualner`, `semantic-engine`
- **Origine:** audit 2026-11-26 §5, ticket proposto BR-65.

## Problema

Esiste un solo worker GLiNER (`static NER_WORKER`), con un thread proprietario
del modello: ogni scena invia la richiesta sul canale e attende. Il
parallelismo Rayon accelera le altre fasi ma **non** rende parallela
l'inferenza. Oggi non c'è misura che separi:

- tempo di attesa nella coda del worker;
- tempo di inferenza del modello;
- post-processing e filtro per soglia;
- numero di menzioni estratte e scartate.

Senza questa separazione non si può decidere se conviene fare batching,
cambiare il numero di worker o ridurre il post-processing: si rischia di
aggiungere thread (e RAM) senza aumentare il throughput.

## Cosa serve per risolverlo

1. **Strumentare il worker** con timestamp `enqueued_at`, `started_at`,
   `finished_at` per ogni richiesta e contatori per file:
   - `ner_queue_wait_us`;
   - `ner_inference_us`;
   - `ner_postprocess_us`;
   - `ner_candidates_in/out`;
   - `ner_failures`.
2. **Propagare la misura** in modo additivo nella risposta di analisi o in un
   log strutturato per richiesta; non cambiare il manifest semantico.
3. **Misurare su 100 scene** (corpus `multilingual_100`) con GLiNER attivo:
   p50/p95/p99 di coda, inferenza e post-processing, più throughput aggregato.
4. **Valutare il batching.** Se `extract_long_with` accetta una sola sorgente,
   verificare se il runtime supporta un batch di scene; in caso contrario
   documentare il limite e non introdurre un'API finta.
5. **Escludere la moltiplicazione del modello.** Non caricare una copia del
   modello per thread Rayon: la memoria del modello (~584 MB per GLiNER2.5) è
   il vincolo dominante.
6. **Scrivere una decisione**: un worker con batching, più worker con modello
   condiviso, o stato attuale documentato. La scelta va motivata con la misura
   del punto 3.
7. **Collegare il budget** (BR-AUDIT-17): il worker NER non deve ricevere più
   thread di quanti il budget CPU consenta.

## Criteri di accettazione

- [ ] Coda, inferenza e post-processing sono misurati separatamente.
- [ ] La misura p50/p95/p99 su 100 scene è allegata al ticket.
- [ ] Esiste una decisione scritta su batching / numero di worker.
- [ ] Una sola istanza del modello è residente, indipendentemente dai thread.
- [ ] I contatori sono esposti in un punto diagnosticabile senza parsing di
      log liberi.
- [ ] Nessuna regressione di latenza o RSS non spiegata dalla misura.

## Verifica

```sh
cd BrainRust
cargo test -p visualner ner
VISUALNER_MODEL_DIR=… BRAINRUST_NER_MODEL=gliner25 \
  cargo test -p semantic-engine --release analysis_equivalence -- --nocapture
```

## Dipendenze / rischi

- La misura richiede i pesi GLiNER installati e un host con RAM sufficiente;
  senza questi il ticket resta `OPEN` con il piano pronto.
- Il batching può cambiare l'ordine o il numero di menzioni: va confrontato
  byte-for-byte prima di adottarlo.

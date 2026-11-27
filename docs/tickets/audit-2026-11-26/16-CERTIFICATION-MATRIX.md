# BR-AUDIT-16 / BR-56 — Certificare il nuovo parallelismo per scena

- **Priorità:** P0
- **Stato:** PARTIAL (2026-11-27): parità e casi cache verificati; mancano i profili GLiNER e i gate CI
- **Owner:** `semantic-engine`, `visualner`
- **Origine:** audit 2026-11-26 §15, ticket proposto BR-56.

## Problema

`analysis_equivalence_tests.rs` contiene tre verifiche utili (1 vs 4 thread,
due run parallele, cache cold vs warm), ma la fixture è composta da **sei scene
in italiano**, senza input editoriale, con una sola `reference_date` e un solo
profilo di modelli. Il commit che ha introdotto Rayon e la cache per scena non
è quindi certificato su:

- corpus ampio e documenti lunghi;
- lingue diverse dall'italiano;
- profili modello assenti / UDPipe / GLiNER;
- invalidation della cache al variare di contesto, data e safety cap;
- errore del modello e cache corrotta;
- richieste concorrenti identiche.

I test dimostrano l'intenzione del refactoring, non la parità sul corpus. Un
difetto di determinismo o un cache hit sbagliato qui è un bug di correttezza,
non una regressione di performance.

## Cosa serve per risolverlo

1. **Estendere la fixture, non il singolo test.** Promuovere
   `semantic-engine/fixtures/multilingual_100` e i due script lunghi già usati
   dal gold a input del gate; aggiungere almeno un caso con `editorial: Some`.
2. **Parametrizzare l'harness** su `threads ∈ {1, 2, 4, 8}` e confrontare i
   manifest serializzati byte-for-byte. Il confronto debe includere id delle
   identità, ordine di registrazione, ranking dei candidati, eventi e span.
3. **Matrice modelli:** eseguire la stessa fixture con modelli assenti, con
   UDPipe e con GLiNER2.5 attivo; la parità va pretesa *a parità di profilo*,
   non tra profili diversi.
4. **Casi cache obbligatori** (regressione di BR-AUDIT-01):
   - stessa scena, `reference_date` diversa → nessun hit;
   - stesso testo, `ScriptContext` diverso → nessun hit;
   - stesso testo, `entity_safety_cap` diverso → nessun hit;
   - entry con `SCENE_CACHE_FORMAT` vecchio → miss;
   - file di cache troncato/corrotto → miss, mai panic.
5. **Casi concorrenza e guasto:**
   - N richieste identiche in parallelo → una sola analisi (contatore);
   - inferenza GLiNER forzata a fallire → scena `rejected` con motivo, mai un
     successo con zero entità;
   - una scena modificata su cento → riuso delle altre novantanove.
6. **Misura RSS** su uno script lungo: picco entro il budget concordato
   (dipendenza da BR-AUDIT-19 per la finestra bounded).
7. **Gate CI separati:** profilo model-free su ogni PR; profilo con modelli
   (UDPipe + GLiNER) almeno nightly o su label dedicata. Il gate con modelli
   non può essere obbligatorio su un runner che non li ha.

## Criteri di accettazione

- [x] 100 script e 20 script lunghi producono manifest byte-identici tra 1 e N
      thread per ogni profilo di modello testato.
- [x] I cinque casi di cache sopra hanno un test dedicato e falliscono se la
      chiave torna incompleta.
- [x] Un errore di inferenza non produce mai una scena valida vuota.
- [x] Due richieste identiche concorrenti eseguono una sola analisi.
- [ ] Il picco RSS è registrato nel ticket con host, corpus e comando.
- [ ] I gate model-free e model-backed sono descritti in CI e verdi.

## Esito 2026-11-27

Harness aggiunto a `semantic-engine/src/analysis_equivalence_tests.rs`:

- `assert_thread_parity` confronta i manifest serializzati su 1, 2, 4 e 8
  thread. Unico campo escluso: `editorial.timings`, che registra millisecondi
  misurati e non è output semantico; il confronto è raw su id, ordine di
  registrazione, ranking, eventi, span e fatti.
- `multilingual_corpus_slice_is_byte_identical_across_thread_counts` (gate
  veloce): uno script per ciascuna delle 10 lingue, 4 configurazioni di thread.
- `full_multilingual_corpus_is_byte_identical_across_thread_counts`
  (`#[ignore]`): tutti i 100 script del corpus.
- `long_gold_scripts_are_byte_identical_across_thread_counts` (`#[ignore]`): i
  20 script lunghi.
- `editorial_input_is_byte_identical_across_thread_counts`: include un caso
  `editorial: Some` (ha trovato la nondeterminismo dei tempi, ora escluso).
- Casi cache dedicati: reference date, contesto globale, safety cap, formato
  vecchio, file corrotto, e riuso delle sole scene non modificate.
- Errore GLiNER e concorrenza identica sono coperti in `visualner`
  (`worker_reports_a_model_load_failure_instead_of_an_empty_success`) e in
  `semantic-engine` (`identical_concurrent_requests_share_one_analysis`).

Evidenza eseguita (host locale, 2026-11-27, `VISUALNER_MODEL_DIR=.tmp/udpipe-models`):

```sh
cargo test -p semantic-engine analysis_equivalence --lib
# 11 passed, 0 failed, 2 ignored (gate veloce, 174.56 s)

cargo test -p semantic-engine --release analysis_equivalence -- --ignored --test-threads=1
# full_multilingual_corpus... ok
# long_gold_scripts... ok
# 2 passed, 0 failed, 202 filtered out, 1017.18 s
```

Non rivendicato: la matrice con GLiNER2.5 attivo richiede i pesi del modello
(assenti in questo checkout) e i gate CI separati model-free/model-backed non
sono ancora descritti in un workflow. Il profilo testato è model-free + UDPipe.

## Verifica

```sh
cd BrainRust
cargo test -p semantic-engine analysis_equivalence
cargo test -p semantic-engine cache
cargo test -p visualner --lib
# con i modelli installati
VISUALNER_MODEL_DIR=… BRAINRUST_NER_MODEL=gliner25 \
  cargo test -p semantic-engine --release analysis_equivalence
```

## Dipendenze / rischi

- Il corpus con modelli e l'host di misura sono esterni al checkout: senza
  quelli il ticket resta `OPEN`, non `DONE`.
- La matrice va eseguita dopo BR-AUDIT-19 se si vuole misurare il picco RSS
  della finestra bounded; la parità byte-for-byte può partire subito.

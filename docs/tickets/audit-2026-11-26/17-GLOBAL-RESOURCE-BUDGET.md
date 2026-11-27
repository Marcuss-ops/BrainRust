# BR-AUDIT-17 / BR-57 — Budget globale CPU e RAM

- **Priorità:** P1
- **Stato:** OPEN
- **Owner:** `ndjson-stdio`, `semantic-engine`, `visualner`
- **Origine:** audit 2026-11-26 §5, ticket proposto BR-57.

## Problema

La concorrenza è distribuita su tre livelli che oggi non si conoscono:

| Livello | Default attuale | Dove |
| --- | --- | --- |
| Worker NDJSON | `available_parallelism` clampato `1..=8`, tetto esplicito 32 | `ndjson-stdio/src/lib.rs` |
| Scene Rayon | pool globale Rayon (nessun `ThreadPoolBuilder` in produzione) | `semantic-engine/src/lib.rs` |
| ONNX Runtime | thread intra/inter-op non configurati | `visualner/src/extractor/ner.rs` |
| GLiNER | un solo thread proprietario del modello | `visualner/src/extractor/ner.rs` |

Il prodotto `worker NDJSON × scene Rayon × thread ONNX` non è limitato da
nessuna parte. Con più richieste `analyze_script` nello stesso processo, ogni
worker può saturare tutti i core e la RAM dei draft, mentre il modello GLiNER
resta serializzato: si consuma più memoria senza aumentare il throughput.
`VISUALNER_WORKERS` e `RAYON_NUM_THREADS` esistono come variabili separate, ma
non esiste un'autorità che le renda coerenti.

## Cosa serve per risolverlo

1. **Definire una struttura unica** `SemanticResourceBudget` (CPU disponibili,
   worker NDJSON, thread per scena, thread ONNX, finestra scene) letta una
   volta all'avvio del worker.
2. **Inizializzare il pool Rayon globale una sola volta** con
   `rayon::ThreadPoolBuilder::new().num_threads(...)` in `Worker::from_env`;
   nessuna richiesta deve creare o ridimensionare pool.
3. **Derivare il pool NDJSON dal budget** invece di leggere solo
   `VISUALNER_WORKERS`: `workers = clamp(budget, 1, MAX_WORKERS)`.
4. **Configurare ONNX Runtime** con un numero esplicito di thread intra-op e
   inter-op (session builder), così il modello non usa tutti i core mentre
   Rayon sta già lavorando.
5. **Documentare la formula** `worker × scene_threads × ort_threads` e
   scegliere un default che non superi i core fisici.
6. **Esporre i valori effettivi in `health`** (`rayon_threads`,
   `ndjson_workers`, `ort_threads`, `scene_window`) per diagnosi e supporto.
7. **Misurare** una matrice `workers × scene_threads` su un corpus fisso:
   wall time, CPU, RSS, throughput. Il default si sceglie dalla misura, non a
   intuito.
8. **Allineare `pipelinegen-muscles`** allo stesso principio: il budget media
   (job × stem × thread FFmpeg) non deve essere moltiplicato per il budget
   semantico dello stesso host (vedi BR-AUDIT-14).

## Criteri di accettazione

- [ ] Un solo punto di inizializzazione del pool Rayon, usato da tutte le
      richieste.
- [ ] Il numero di worker NDJSON non può superare il budget CPU configurato.
- [ ] ONNX Runtime riceve un limite esplicito di thread.
- [ ] `health` riporta i valori effettivi, non quelli richiesti.
- [ ] La matrice di misura è allegata con wall/CPU/RSS e il default scelto.
- [ ] Con due richieste concorrenti il processo non supera il budget RAM
      dichiarato.

## Verifica

```sh
cd BrainRust
cargo test -p ndjson-stdio
cargo test -p semantic-engine protocol
cargo clippy -p semantic-engine -p visualner --all-targets -- -D warnings
# misura: due analyze_script concorrenti, campionamento RSS/CPU
```

## Dipendenze / rischi

- Il tuning dei thread ONNX dipende dalla build di `onnxruntime`; va verificato
  che il session builder esponga le opzioni nella versione usata.
- Il budget non sostituisce la finestra bounded dei draft (BR-AUDIT-19): sono
  due limiti complementari, CPU/thread e memoria per documento.

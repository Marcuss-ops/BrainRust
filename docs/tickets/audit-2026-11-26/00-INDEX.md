# Audit semantico 2026-11-26 — piano di intervento

**Origine:** audit del motore semantico su `BrainRust@accb6d30`, integrato con
il backlog proposto BR-52…BR-76.
**Stato:** i sette finding P0/P1 a basso rischio sono implementati e chiusi
(BR-AUDIT-01…07). Il secondo cut del 2026-11-27 chiude altri tre ticket e porta
tre a PARTIAL con l'evidenza eseguita. Il terzo cut prepara il cutover al
manifest normalizzato v4 (BR-AUDIT-08 CLOSED) e porta BR-AUDIT-18 a PARTIAL: la
tabella globale è sul wire e il consumer Go la legge dietro routing di versione,
mentre la struttura decodificata Rust resta v3. I restanti hanno un ticket
dedicato con il lavoro richiesto, i criteri di accettazione e il comando di
verifica.

Legenda: **CLOSED** = implementato e verificato in questo checkout;
**PARTIAL** = parte reachable implementata e verificata, il resto è bloccato da
un gate esterno o da una misura non eseguita (elencato nel ticket);
**OPEN** = ticket pronto, lavoro non iniziato; **DEFERRED** = pronto ma
bloccato da un gate esterno (contratto cross-repo, corpus, modelli o host).

## Primo cut — finding P0/P1 a basso rischio

| ID locale | ID audit | Priorità | Stato | Titolo |
| --- | --- | --- | --- | --- |
| [BR-AUDIT-01](01-SCENE-CACHE-KEY.md) | BR-52 | P0 | CLOSED | Chiave della cache per scena completa |
| [BR-AUDIT-02](02-RUNTIME-CACHE-IDENTITY.md) | BR-53 | P0 | CLOSED | Identità runtime GLiNER e lessico override |
| [BR-AUDIT-03](03-GLINER-EXPLICIT-FAILURE.md) | BR-54 | P0 | CLOSED | Fallimento GLiNER esplicito |
| [BR-AUDIT-04](04-ATOMIC-NER-INIT.md) | BR-55 | P0 | CLOSED | Inizializzazione atomica del worker NER |
| [BR-AUDIT-05](05-SCENE-CACHE-OWNERSHIP.md) | BR-59 | P1 | CLOSED | Cache per scena posseduta dal worker |
| [BR-AUDIT-06](06-SINGLE-FLIGHT.md) | BR-58 | P1 | CLOSED | Single-flight delle analisi concorrenti |
| [BR-AUDIT-07](07-SENTENCE-SPAN-REUSE.md) | BR-60 | P1 | CLOSED | Riuso degli span di frase già calcolati |

## Secondo cut — 2026-11-27

| ID locale | ID audit | Priorità | Stato | Titolo |
| --- | --- | --- | --- | --- |
| [BR-AUDIT-10](10-MANIFEST-BYTES-REUSE.md) | BR-63 | P2 | CLOSED | Riuso dei byte canonici del manifest |
| [BR-AUDIT-24](24-GRANULAR-OPERATIONS.md) | BR-70 | P2 | CLOSED | Ritirare le operazioni NDJSON granulari |
| [BR-AUDIT-27](27-REPORT-ARCHIVE.md) | BR-76 | P3 | CLOSED | Archiviare i report storici |
| [BR-AUDIT-16](16-CERTIFICATION-MATRIX.md) | BR-56 | P0 | PARTIAL | Certificare il nuovo parallelismo per scena |
| [BR-AUDIT-19](19-BOUNDED-SCENE-WINDOW.md) | BR-64 | P1 | PARTIAL | Finestra bounded delle scene in-flight |
| [BR-AUDIT-21](21-REQUEST-OWNERSHIP.md) | BR-67 | P1 | PARTIAL | Eliminare il clone completo della richiesta |

## Terzo cut — 2026-11-27 (cutover manifest v4)

| ID locale | ID audit | Priorità | Stato | Titolo |
| --- | --- | --- | --- | --- |
| [BR-AUDIT-08](08-NORMALIZED-MANIFEST.md) | BR-62 | P2 | CLOSED | Manifest normalizzato (contratto v4) |
| [BR-AUDIT-18](18-GLOBAL-IDENTITY-REGISTRY.md) | BR-61 | P1 | PARTIAL | Registro globale delle identità |

## Backlog completo

| ID locale | ID audit | Priorità | Stato | Titolo |
| --- | --- | --- | --- | --- |
| [BR-AUDIT-17](17-GLOBAL-RESOURCE-BUDGET.md) | BR-57 | P1 | OPEN | Budget globale CPU e RAM |
| [BR-AUDIT-20](20-GLINER-PROFILING.md) | BR-65 | P1 | OPEN | Profilare coda e inferenza GLiNER |
| [BR-AUDIT-22](22-PROJECTION-CENSUS.md) | BR-68 | P1 | OPEN | Censimento delle proiezioni inutilizzate |
| [BR-AUDIT-23](23-LEGACY-PLANNER.md) | BR-69 | P2 | OPEN | Ritirare il planner editoriale legacy |
| [BR-AUDIT-25](25-AUDIO-CHART-FONT-FALLBACKS.md) | BR-71 | P2 | OPEN | Ritirare i fallback audio, chart e font |
| [BR-AUDIT-26](26-PRIMITIVE-UNIFICATION.md) | BR-72 | P2 | OPEN | Unificare le primitive equivalenti |
| [BR-AUDIT-09](09-SHARED-TOKENIZATION.md) | BR-60 | P2 | DEFERRED | Rappresentazione linguistica condivisa |
| [BR-AUDIT-11](11-PLANNER-SIMPLIFICATION.md) | BR-66 | P2 | DEFERRED | Semplificazione del planner editoriale |
| [BR-AUDIT-12](12-LEGACY-RETIREMENT.md) | BR-69/70/71 | P3 | DEFERRED | Registro del ritiro legacy (umbrella) |
| [BR-AUDIT-13](13-GAZETTEER-MEMORY.md) | BR-73 | P3 | DEFERRED | Memoria del gazetteer e degli indici |
| [BR-AUDIT-14](14-FFMPEG-AUDIT.md) | BR-74 | P3 | DEFERRED | Audit dei processi FFmpeg |
| [BR-AUDIT-15](15-BENCHMARK-MATRIX.md) | BR-75 | P3 | DEFERRED | Benchmark automatici |

## Evidenza del secondo cut

```sh
cd BrainRust
export VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models

cargo test -p semantic-engine --lib analysis_equivalence
# 11 passed, 0 failed, 2 ignored

cargo test -p semantic-engine --release analysis_equivalence -- --ignored --test-threads=1
# full_multilingual_corpus_is_byte_identical_across_thread_counts ... ok
# long_gold_scripts_are_byte_identical_across_thread_counts ... ok
# 2 passed, 1017.18 s

cargo test -p semantic-engine --lib protocol
# 21 passed, include cold_and_cached_responses_reuse_the_same_manifest_bytes
# e granular_planner_operations_are_retired

cd ../refactored
go test ./internal/platform/media/rustexec/... -count=1
# ok 71.919s
```

## Evidenza del terzo cut (v4)

```sh
cd BrainRust
export VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models

python3 contract/semantic-manifest.v4/validate.py
# ✅ semantic_manifest.v4 contract: 2 scenes, 1 identities, v3 golden checked

bash contract/semantic-manifest.v4/check_consumer_copy.sh \
  ../refactored/internal/platform/media/rustexec/testdata/semantic-manifest/v4
# ✅ consumer goldens match contract/semantic-manifest.v4/goldens

cargo test -p semantic-core manifest::v4
# 3 passed — include shared_contract_golden_satisfies_v4_invariants

cargo test -p semantic-engine --lib v4_normalization
# 1 passed — v3 126.345 B, v4 102.555 B, 175 copies vs 44 unique

cargo test -p semantic-engine --lib analyze_script_routes_manifest_versions
# 1 passed — v3 default, v4 opt-in, v5 rejected

BRAINRUST_V4_REPEATS=10 cargo test -p semantic-engine --lib \
  manifest_v4_wire_probe -- --ignored --nocapture
# scenes=70 v3_bytes=8699179 v4_bytes=2647005 saved_pct=69.57
# identity_copies=9900 unique_identities=147 VmHWM: 390116 kB

cd ../refactored
go test ./internal/platform/media/rustexec -count=1
# ok 69.674s; go vet ✅
```

## Ordine di lavoro

1. **Correttezza** — chiusa per il profilo model-free; restano i profili
   GLiNER e i gate CI di BR-AUDIT-16.
2. **Prestazioni** — BR-AUDIT-26 (primitive), BR-AUDIT-20 (profilo NER).
3. **Memoria** — BR-AUDIT-18 (normalizzazione interna del registro; il
   contratto v4 è pronto), BR-AUDIT-13 (gazetteer), BR-AUDIT-17 (budget).
4. **Demolizione** — BR-AUDIT-22 (censimento), poi 23/25/12.
5. **Semplificazione editoriale** — BR-AUDIT-11, dopo la parità dei piani.

## Gate comuni

```sh
cd BrainRust
cargo test -p semantic-core -p visualner -p semantic-engine --locked
cargo clippy -p semantic-core -p visualner -p semantic-engine --all-targets --locked -- -D warnings
cargo fmt --check
```

## Regola di chiusura

Un ticket passa a CLOSED solo con i criteri di accettazione verificati e il
comando di verifica eseguito. Una misura stimata o un test non eseguito non
chiude un ticket: va scritto nel ticket cosa manca e chi lo possiede.

# Audit architetturale — BrainRust workspace

**Data:** 2026-11-23
**Oggetto:** 8 crate Rust, `main` a `TICKET-49` (PARTIAL)
**Metodo:** misure sul codice corrente, non impressioni. Comandi in §9.
**Precedente:** [TICKET-21](../tickets/TICKET-21-hotspot-remediation.md) (2026-11-16), [TICKET-09](../tickets/TICKET-09-hotspot-contraction.md).

## Verdetto

La base è solida: **zero cicli fra crate, zero `unsafe`, zero `TODO/FIXME`, `cargo clippy -D warnings` verde, duplicazione di codice praticamente assente**. I rischi residui non sono "spaghetti": sono **tre punti in cui la conoscenza è duplicata fuori dalla sua SSOT** (e quindi può già essere andata alla deriva), più **tre funzioni sopra soglia** e **33 costruzioni a mano dell'envelope `Response`**.

Il finding più importante non è stilistico: `overlay-compiler` ha una propria tassonomia dei tipi entità, parallela a `semantic-core::entity_capabilities`, e in quella copia locale `MEDIA_OUTLET` non esiste. Un'entità che `semantic-core` dichiara "named subject" (Ansa, Repubblica, Il Post…) viene scartata dal planner in **due lane indipendenti**. È la stessa classe di difetto che TICKET-49 sta misurando (due regole di matching sullo stesso dato).

---

## 1. Architettura e accoppiamento

### Grafo delle dipendenze (aciclico, verificato sui `Cargo.toml`)

```
ndjson-stdio          (nessuna dipendenza interna)
semantic-core         (serde, unicode-normalization, sha2)
   ├── overlay-compiler
   ├── script-qa
   └── visualner ── ndjson-stdio
semantic-engine ── visualner, semantic-core, overlay-compiler, script-qa, ndjson-stdio
mediasampler    ── ndjson-stdio
pipelinegen-muscles ── ndjson-stdio
```

Nessun ciclo. La direzione `core ← {visualner, overlay-compiler, script-qa} ← semantic-engine` è rispettata. `pipelinegen-muscles` non dipende dal semantic engine, come dichiarato nel README.

### Cosa è a posto

| Area | Evidenza |
| --- | --- |
| `unsafe` | `unsafe_code = "deny"` a livello workspace; 0 occorrenze in `*/src` |
| I/O ai confini | `overlay-compiler`, `script-qa`, `ndjson-stdio`: 0 siti I/O. `semantic-core`: 1 solo sito di produzione (`lexicon.rs:246`, override `$SEMANTIC_LEXICON_DIR` con fallback embedded, documentato) |
| Stato globale | Nessun `static mut`. Solo cache `OnceLock` immutabili + 1 `Mutex<HashMap>` process-local (`artifact.rs:63`) per la generation degli output |
| Retry | Nessun loop di retry fragile nel workspace: i retry sono del chiamante Go; qui c'è solo classificazione degli errori |
| Concorrenza | `thread::scope` con fan-out limitato (`MAX_PARALLEL_IO`, `parallel.rs`); niente runtime async |

### Finding

#### F1 — P1 · Tassonomia dei tipi entità duplicata fuori dal SSOT (impatto funzionale)

> Stato pre-remediation: i riferimenti `file:riga` descrivono il codice com'era
> all'audit. Cosa è cambiato: [§8-bis](#8-bis-stato-implementazione-2026-11-23).

`semantic-core/src/entity_capabilities.rs` esiste proprio per eliminare questo problema, e lo dice nel doc di modulo: *«The same kind list used to be written by hand in five places … Adding an additive type to four of them and forgetting the fifth is exactly how ANIMAL/FOOD/… ended up extracted but never pictured»*. Oggi è usato da `visualner`, `semantic-engine`, `semantic-core` — e da **zero punti di `overlay-compiler`**, che si è tenuto tre mappe locali:

| Sito | Regola locale | `MEDIA_OUTLET` |
| --- | --- | --- |
| `editorial_plan_v2/projection.rs:889` | `fold_entity_type` | cade in `other` → resta `"MEDIA_OUTLET"` |
| `editorial_plan_v2/projection.rs:874` | `entity_kind` | `""` → il match a `:350` finisce in `_ => {}`; per un'organizzazione è corretto (nessuna annotazione), quindi questa lane non è rotta |
| `planner/media.rs:422` | `entity_card_kind` | `_ => None` → `media.rs:185` fa `continue` → **card mai creata** |
| `planner/media.rs:295-299` | gate tipi per il recupero immagini | non incluso → **nessun recupero** |
| `lib.rs:856-860` | `entity_type_weight` | `_ => 0.5`, mentre `ORG` vale `0.9` → **rank declassato** |

Nel frattempo `semantic-core/src/identity.rs:396` mappa `MEDIA_OUTLET` su `"org"` e `entity_capabilities("MEDIA_OUTLET")` restituisce `named_subject: true`. Quindi lo stesso tipo è "soggetto nominato con id `org:*`" per il core e "tipo sconosciuto" per il planner: la contraddizione è già nel codice, non è un'ipotesi.

**Impatto:** le testate curate nel lessico (`lexicon/entity_type_terms.json`: ansa, la repubblica, il post, il sole 24 ore, corriere della sera…) possono essere estratte, linkate e conteggiate come risolte dal lineage, ma non producono alcun item nel piano. È lo stesso sintomo `entity_not_planned` di TICKET-49.

**Cautela:** nel run R30 citato da TICKET-49, `Rai` e `La7` **non** sono nel lessico outlet italiano (verificato), quindi arrivano probabilmente come `ORG` e il loro caso va spiegato altrove. Questo finding va confermato sul payload R30 leggendo `entity_type` degli 8 intenti non risolti: se è `MEDIA_OUTLET`, la causa è questa.

**Fix minimo:** un solo `EntityKind` (enum) + fold in `semantic-core`, consumato da `overlay-compiler`; `fold_entity_type`, `entity_card_kind` e `entity_type_weight` diventano un `match` su quell'enum. I tipi sconosciuti restano `Other` (fail-closed sulle lane) e un test di copertura confronta il fold con i tipi dichiarati nel lessico. Un fixture test che passa `MEDIA_OUTLET` nelle lane reali (card, asset verificato, recupero immagine) e nel ranking.

#### F2 — P2 · Il README descrive un confine che il codice ha superato

`README.md` definisce `semantic-engine` come *«dedicated CPU-only text-analysis worker … it does not search for assets or perform media I/O»* e separa il planner in `overlay-compiler`. Ma `semantic-engine/src/protocol.rs:249-253` espone `compile_editorial_plan`, `resolve_map_overlaps`, `trim_map_windows_before_budget`, `rank_entity_overlays`, `cascade_clustered_entrances`, cioè le cinque fasi del planner.

La scelta è legittima (TICKET-14 l'ha voluta: una sola chiamata), ma il documento è rimasto indietro e chi legge il README per capire dove vive il planning viene indirizzato male. **Costo del fix: una pagina di README.** Non toccare l'architettura: il worker unificato è deliberato.

#### F3 — P2 · `CANONICAL_SAMPLE_RATE` non è più l'unica fonte per 48 kHz

`config.rs:17` definisce la costante, usata da `audio_copy.rs:34`, `render_audio_plan.rs:102`, `protocol.rs:914-915`. TICKET-09 dichiara «unified … instead of repeating `48000` in each validator»: vero per i validator, **non per il grafo ffmpeg e gli argv**, dove il letterale resta in produzione:

- `render_audio_execution.rs:41` (`aresample=48000`), `:158` (`anullsrc=r=48000`), `:660`, `:677`, `:721` (`-ar 48000`);
- `render_audio_probe.rs:136` confronta la **stringa** `"48000"`.

Se la costante cambia, il validator approva un piano che l'encoder non rispetta, oppure il probe rifiuta un output corretto. Non è teorico: è esattamente il tipo di drift che TICKET-09 pensava di aver chiuso.

**Fix:** `format!` con la costante nei punti del grafo e negli argv, e nel probe confrontare `sample_rate.parse::<i64>() == Ok(CANONICAL_SAMPLE_RATE)`. I letterali restanti in `encoder.rs:229` e `transform_assemble.rs:345` sono sotto `#[cfg(test)]`: si possono lasciare o derivare anch'essi.

#### F4 — P3 · `failed_response` dichiara sempre `operation: "cut_batch"`

`artifact.rs:18` hardcoda `operation: "cut_batch"`, ma la funzione ha **113 chiamate in 12 moduli** (`transform_manifest` 44, `transform_operations` 15, `render_stock_canonical_executor` 14, …). L'envelope è corretto solo perché `dispatcher.rs:61-63` lo riscrive a posteriori:

```rust
if !response.ok {
    response.operation = operation.as_str().to_string();
}
```

Il dispatcher è l'unico entry point di produzione, quindi oggi non c'è un bug osservabile — ma l'invariante è implicita e non testata: qualunque chiamata diretta (test, futuro entry point, uso della lib) produce un `operation` sbagliato, e `Operation` esiste già (`protocol.rs:29`) con `as_str()`.

**Fix:** `failed_response(operation: Operation, source, error)`; il patch nel dispatcher sparisce.

---

## 2. Complessità e manutenibilità

### Complessità cognitiva (clippy `cognitive_complexity`, soglia default 25)

Produzione:

| Funzione | Valore | File |
| --- | --- | --- |
| `select_phrases` | **38** | `semantic-core/src/phrase_select.rs:89` |
| `noun_phrase_candidates` | **35** | `visualner/src/extractor/phrases.rs:249` |
| `value_candidates` | **29** | `visualner/src/extractor/values.rs:522` |

Nel test: 6 funzioni tra 27 e 38 (`overlay_plan_contract.rs`, `content_fingerprint`, `lexicon/tests.rs`) — non è debito di produzione.

### Funzioni lunghe / profonde

182 funzioni di produzione superano 70 righe **oppure** 6 livelli di annidamento. Molte sono `match` di dominio legittimi; queste sono quelle che meritano un ticket, perché uniscono dimensione e churn o complessità misurata:

| Righe | Nest | Funzione | File |
| --- | --- | --- | --- |
| 316 | 6 | `enrich_contextual_search_queries` | `semantic-engine/src/consolidate/contextual_queries.rs:441` |
| 311 | 7 | `run_with_chapter_options` | `semantic-core/src/phrase_impact.rs:432` |
| 297 | 6 | `extract_entity_facts` | `semantic-engine/src/facts.rs:251` |
| 276 | 6 | `noun_phrase_candidates` | `visualner/src/extractor/phrases.rs:249` |
| 232 | 5 | `select_phrases` | `semantic-core/src/phrase_select.rs:89` |
| 266 | 4 | `execute_canonical_render` | `pipelinegen-muscles/src/render_stock_canonical_executor.rs:47` |
| 231 | 5 | `execute` | `pipelinegen-muscles/src/transform_operations.rs:10` |
| 221 | 3 | `classify_type` | `visualner/src/extractor/values.rs:245` |
| 196 | 7 | `project_editorial_scenes` | `overlay-compiler/src/editorial_plan_v2/editorial.rs:22` |
| 180 | 7 | `extract` | `semantic-engine/src/legal.rs:41` |
| 168 | 6 | `cut_one` | `pipelinegen-muscles/src/cut.rs:120` |
| 169 | 5 | `analyze_script` | `semantic-engine/src/lib.rs:611` |

Nota: `select_phrases` e `noun_phrase_candidates` compaiono in entrambe le liste (complessità alta **e** dimensione alta): sono i due candidati veri, non i 182.

### Change amplification: l'envelope `Response`

33 costruzioni letterali di `Response` in 19 file di `pipelinegen-muscles` (7 campi ognuna). Aggiungere un campo significa toccare 19 file, e il compilatore non può aiutare oltre un certo punto perché ogni literal ripete `items: Vec::new(), preflight: None, error: None`.

**Fix:** due costruttori (`Response::ok(op, …)`, `Response::failed(op, source, error)`) e i literal restano solo nei test di golden. È lo stesso principio già applicato in TICKET-21 a `ProductionCandidate` (una factory invece di 24 campi copiati).

### Errori: due dialetti, nessun danno

`MuscleError` è usato 185 volte, `Result<_, String>` 36 volte (`pipelinegen-muscles` 16, `visualner` 11, `semantic-core` 8, `semantic-engine` 1). La convivenza è deliberata (errori utente vs errori interni) e il confine è al dispatcher. Non vale un refactor.

### `unwrap`/`expect` in produzione: ~45, tutti guardati

Al lordo dei moduli di test dentro `src` sono 82; togliendoli restano ~45 su 8 crate. Campione ispezionato: `entity_linker.rs:54/59/126` (JSON embedded, validato dai test), `places.rs:33/82/90` (idem), `recipes.rs:560/661` (`len() != 1` / non-vuoto controllati subito prima), `lexicon.rs:264` (tabella embedded, unico `panic!` di produzione).

Un punto di attenzione: 4 `expect("validated …")` in `overlay-compiler` (`planner/mod.rs:374`, `planner/media.rs:257`, `planner/admission.rs:8,39`) dipendono dal fatto che `compile_plan` chiami `validate_compilable` all'ingresso (`planner/mod.rs:58`). Oggi è vero; è un accoppiamento temporale non espresso dal tipo. Basso rischio, ma se nasce un secondo entry point diventa un panic.

---

## 3. Prestazioni

**Non ho eseguito benchmark e non riporto speedup.** Segnalo solo dove guardare, in ordine di rapporto rischio/incertezza:

1. **Clone density** nei consolidatori: `consolidate/recipes.rs` 46 `.clone()` / 821 righe, `contextual_queries.rs` 42, `producers.rs` 38. Sono percorsi una-volta-per-run, non hot loop: da misurare prima di toccare.
2. **`format!` in percorsi caldi**: `planner/presentation.rs` 73 occorrenze, `facts.rs` 34 — sono stringhe da serializzare comunque, quindi probabilmente fisiologiche.
3. **Lookup lineari in loop**: la scansione euristica trova ~150 candidati, ma la maggior parte sono `(range).contains(&x)` che sono O(1). Non ho identificato un O(n²) dimostrato. Il posto da guardare con un profilo è `entity_linker::link_all_with_threads` (160 righe) e `consolidate/recipes.rs` (costruzione di `by_evidence` con chiavi tuple).
4. **Nessun allocatore/GC da combattere**: niente `unsafe`, niente buffer manuali; il costo dominante è ICU/UDPipe, non il codice Rust.

Raccomandazione: un bench `criterion` su `analyze_script` e `compile_plan` prima di qualunque ottimizzazione; senza numeri, questo capitolo resta vuoto per scelta.

---

## 4. Dead code, YAGNI, igiene

| Segnale | Esito |
| --- | --- |
| `TODO` / `FIXME` / `HACK` / `XXX` | **0** |
| `#[deprecated]` | **0** |
| `#[allow(dead_code)]` | **3** (1 campo schema, 2 helper di test) |
| File `.rs` orfani (nessun `mod`) | **0** su 311 dichiarazioni `mod` |
| `panic!` / `unreachable!` / `todo!` / `unimplemented!` in produzione | 1 `panic!` (`lexicon.rs:264`, dataset embedded), 0 degli altri |
| Crate ritirati | `imagecatalog` rimosso il 2026-11-18 (TICKET-28) — nessun residuo nel workspace |

Questo è il capitolo migliore dell'audit: la disciplina di rimozione funziona. Non c'è dead code da cacciare.

---

## 5. Duplicazione

Scansione a finestre su tutto il codice di produzione, con normalizzazione degli identificatori:

- blocchi identici da 60 righe tra file diversi: **0**;
- cloni strutturali da 45 righe: **0**;
- coppie near-clone da 20 righe: **2** (`mediasampler/src/rank.rs:154` ↔ `overlay-compiler/.../request/types.rs`; `planner/candidates.rs:49` ↔ `semantic-core/.../phrase_impact/selection.rs`) — entrambe brevi e structural, non vale intervenire.

La duplicazione reale non è di codice, è di **conoscenza**, e sono i finding F1/F3/F4. C'è anche una duplicazione *deliberata e documentata* che va lasciata stare: `script-qa/src/dates.rs` (`GATE_MONTHS`) vs `semantic-core` CLDR — il commento spiega che il gate deve restare byte-identico al port Go. È la scelta giusta; non "unificare".

---

## 6. "Successo senza completamento"

Ho controllato tutti i percorsi che emettono `ok: true` su output su disco:

| Percorso | Ordine verificato |
| --- | --- |
| `render_stock_canonical_executor` | probe → `validate_final_output` (stream, dimensioni, fps, codec, durata) → `publish_output` |
| `transform_assemble` | ffmpeg → probe post-concat → `cert.verify_metadata` → publish |
| `publish_output` (`artifact.rs`) | file esiste e non è vuoto → generation guard → `rename` atomico → registra la generation |

L'ordine è corretto: l'`ok: true` arriva solo dopo certificazione e publish, e il publish è l'ultima operazione. Il caso TICKET-04 (output troncato) è coperto dalla certificazione, non dalla fiducia nell'exit code.

Un dettaglio di osservabilità, non di correttezza: `render_stock_canonical_executor.rs:298-299` assegna `metadata.ffmpeg_ms` e `metadata.encode_ms` **lo stesso valore**, entrambi con floor a 1 ms. Sono due metriche che sembrano indipendenti ma misurano una sola cosa; chi legge i tempi per capire se il collo di bottiglia è encode o I/O viene ingannato. P3: o si misura separatamente, o si emette una sola metrica.

---

## 7. Matrice hotspot (churn × dimensione)

Churn = commit che hanno toccato il file dal 2026-08-01 (255 commit totali).

| File | Commit | Funzione più lunga | Priorità |
| --- | --- | --- | --- |
| `pipelinegen-muscles/src/protocol.rs` | 45 | `validate` 131 | **Alta** (envelope + F4) |
| `semantic-engine/src/lib.rs` | 37 | `analyze_script` 169 | Media |
| `visualner/src/extractor/values.rs` | 30 | `classify_type` 221 | **Alta** (compl. 29) |
| `pipelinegen-muscles/src/probe.rs` | 30 | `probe_file` 153 | Media |
| `pipelinegen-muscles/src/render_audio_execution.rs` | 27 | `start` 107 | **Alta** (F3) |
| `pipelinegen-muscles/src/transform_assemble.rs` | 21 | `execute` 126 | Bassa |
| `visualner/src/analysis.rs` | 20 | `analyze_with_context_mode` 97 | Bassa |
| `pipelinegen-muscles/src/cut.rs` | 18 | `cut_one` 168 | Bassa |
| `pipelinegen-muscles/src/dispatcher.rs` | 18 | `process` 59 | Media (F4) |
| `pipelinegen-muscles/src/encoder.rs` | 19 | `build_video_args` 110 | Bassa |
| `semantic-core/src/lexicon.rs` | 19 | `global` 95 | Bassa |
| `pipelinegen-muscles/src/render_stock_canonical_executor.rs` | 17 | `execute_canonical_render` 266 | Media (metriche) |
| `semantic-engine/src/protocol.rs` | 17 | `handle_line` 80 | **Alta** (F2) |
| `semantic-engine/src/consolidate.rs` | 15 | `build` 183 | Media |
| `pipelinegen-muscles/src/render_stock.rs` | 15 | `execute` 32 | Bassa |
| `pipelinegen-muscles/src/render_audio_probe.rs` | 15 | `probe_audio` 103 | Media (F3) |

La concentrazione è chiara: 10 dei primi 16 file sono in `pipelinegen-muscles`, ma è anche il crate che esegue ffmpeg e quindi cambia per forza a ogni cambio di formato. Il churn lì non è di per sé un problema.

I quadranti che contano davvero (alta frequenza **e** alta complessità): `values.rs` (`value_candidates` 29 + `classify_type` 221 righe), `render_audio_execution.rs` (F3), `protocol.rs` (33 envelope + F4).

---

## 8. Piano d'azione

**P1 — questa settimana**

1. **F1**: `EntityKind` unico in `semantic-core`, consumato da `overlay-compiler`; eliminare `fold_entity_type` e `entity_card_kind`; validazione fail-closed per i tipi sconosciuti; fixture `MEDIA_OUTLET` sulle tre lane. Prima però: leggere `entity_type` degli 8 intenti non risolti del run R30 per confermare che il caso reale sia questo.
2. **F3**: derivare i 6 letterali `48000` di produzione da `CANONICAL_SAMPLE_RATE` (grafo ffmpeg, argv, probe).

**P2 — prossimo giro**

3. **F2**: aggiornare il README (o la sezione "Boundary") per dire che `semantic-engine` è il worker unificato che esegue anche le cinque fasi del planner. Nessuna modifica di codice.
4. **Costruttori `Response`** + `failed_response(Operation, …)` (chiude anche F4): togli il patch nel dispatcher, i literal restano nei golden.
5. **`select_phrases` (38) e `noun_phrase_candidates` (35)**: estrarre le fasi con test di parità sui corpus esistenti. Non toccare `classify_type`/`value_candidates` senza corpus: sono tabellari.

**P3 — quando si passa di lì**

6. `metadata.encode_ms` = `metadata.ffmpeg_ms`: una metrica sola o due misure vere.
7. `expect("validated …")` → newtype `ValidatedRequest`, così l'invariante è nel tipo e non nella sequenza di chiamate.

**Cosa NON fare**

- Non splittare `protocol.rs` o `consolidate.rs` per numero di righe: TICKET-09 e TICKET-21 hanno già valutato e motivato il no.
- Non unificare `script-qa/src/dates.rs` con il calendario CLDR: la duplicazione è un requisito di parità col gate Go.
- Non aggiungere astrazioni "per il futuro" a `entity_capabilities` o al protocollo: la tabella funziona perché è chiusa e piccola.
- Non inseguire le 182 funzioni sopra soglia: contano solo quelle nei quadranti alti della matrice.

---

## 8-bis. Stato implementazione (2026-11-23)

| Finding | Stato |
| --- | --- |
| F1 tassonomia entità | **DONE** — `EntityKind` + `fold_entity_kind` in `semantic-core`; `overlay-compiler` non ha più tabelle locali; test card/asset/recupero/ranking |
| F2 confine README | **DONE** — il README descrive il worker unificato e la separazione "planner decide, Go materializza" |
| F3 sample rate | **DONE** — i 6 letterali di produzione derivano da `CANONICAL_SAMPLE_RATE` |
| F4 `operation` negli errori | **DONE** — `Response::failure(Operation, …)`, 112 call site convertiti, patch del dispatcher rimosso |
| Envelope `Response` | **DONE** — `Response::success`/`with_preflight`; restano letterali solo nella richiesta non valida ("invalid") e nel fallimento parziale di `cut_batch`, che deve trasportare gli item |
| `encode_ms` = `ffmpeg_ms` | **DONE (documentato)** — nessuna seconda misura esiste; il codice lo dichiara |
| Complessità `select_phrases` / `noun_phrase_candidates` | **APERTO** — richiede un run di parità sul corpus, fuori da questo passaggio |
| `ValidatedRequest` newtype | **APERTO (P3)** — invariante oggi garantita da `compile_plan` |

Evidenze e conteggi: [TICKET-50](../tickets/TICKET-50-entity-kind-ssot-and-envelope.md).

## 9. Verifica

```sh
cargo clippy --workspace --all-targets -- -D warnings   # rc 0, 29.3s
```

Misure raccolte con: `git log --since=2026-08-01 --name-only` (churn), parser di funzioni (lunghezza/nesting, esclusi test), scansione a finestre normalizzate (duplicazione), grep mirati su `unsafe`/`static mut`/I/O/`unwrap`/`panic!`/`TODO`.

Non eseguiti: `cargo test --workspace` (l'audit non modifica codice), benchmark, profilazione. Le cifre di performance assenti sono assenze dichiarate, non zeri.

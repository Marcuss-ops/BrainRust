# Roadmap 2026-11-19 — performance e prodotto

Sei workstream emersi dalla revisione del 2026-11-19. Ogni voce ha passi
ordinati, criteri di accettazione verificabili e il blocco reale. Lo stato è
aggiornato al 2026-11-19: **cache, motore preflight e loro wire cross-repo
DONE (TICKET-37, 38, 39); parallelismo da avviare; memoria, planner e lingue
bloccate.**

Il lavoro è ordinato per rapporto costo/beneficio, non per numero.

## 1. Cache persistente dei manifest — DONE

Ticket: [`TICKET-37-persistent-manifest-cache.md`](TICKET-37-persistent-manifest-cache.md).

Era l'unica voce interamente eseguibile senza credenziali, annotatori o
modifiche cross-repo. Implementata e verificata.

- [x] Chiave SHA-256 sui byte canonici della richiesta, con versione del
      formato; il fingerprint wire resta FNV-1a.
- [x] Identità nella chiave: digest SHA-256 degli asset compilati (incluso
      `lexicon/legal/index.json`), hash di build, ICU major e profilo modello
      UDPipe (nome + dimensione, o `absent`).
- [x] `SEMANTIC_CACHE_DIR` opt-in; nessuna variabile, nessun cambiamento.
- [x] Scrittura atomica `.part` + rename; byte contati solo dopo la
      pubblicazione riuscita.
- [x] Lettura fail-closed: JSON corrotto, chiave estranea, schema o fingerprint
      diversi sono un miss; il file corrotto viene rimosso.
- [x] Budget in byte (default 1 GiB) con sfratto per mtime, aggiornato in
      lettura (LRU reale); prune limitato per byte, ogni 64 scritture o 60 s.
- [x] Test: 23 unitari sulla cache, 5 sul fingerprint (incluso lo scan di
      `lexicon/`), 4 cross-process (restart, 8 scrittori, separazione modello,
      directory inutilizzabile).

Wire chiuso da TICKET-39 (2026-11-19): `NewSemanticEngineExecutor` inoltra
esplicitamente `SEMANTIC_CACHE_DIR` e `SEMANTIC_CACHE_MAX_BYTES` al worker;
senza variabili il comportamento resta memory-only. Misura su un batch ripetuto:
3,64 s cold (`cached:false`) vs 1,56 s warm (`cached:true`), una sola entry.

## 2. Parallelismo nel worker — OPEN, non avviata

Oggi `analyze_script` elabora le scene in sequenza (`semantic-engine/src/lib.rs`,
cicli su `request.scenes`). Il pool di `ndjson-stdio` parallelizza richieste
diverse, non una singola sceneggiatura lunga.

**Blocco di progetto, non di tempo:** l'analisi per scena non è indipendente.
`IdentityRegistry` è creato una volta e passato `&mut` a ogni scena; le scene
successive leggono le identità registrate dalle precedenti. Un `par_iter`
diretto cambierebbe l'output, non solo la velocità.

Passi:

1. Misurare prima: una sceneggiatura lunga, un solo processo, `perf`/`dhat`,
   per confermare dove va il tempo (estrazione, UDPipe, consolidamento).
2. Separare le fasi per dipendenza: per-scena (candidati, fatti, ancore) vs
   script-wide (consolidamento, discourse, thread).
3. Se il registry resta condiviso, il parallelismo va fatto su un registro per
   scena più una fusione deterministica: è un cambio di semantica delle
   identità, quindi richiede una decisione esplicita, non un refactor.
4. Con `rayon` (o pool esplicito) legare i thread a `SEMANTIC_ENGINE_WORKERS`
   per non sommare due livelli di parallelismo.
5. Test di determinismo: 1 thread vs N thread devono produrre manifest
   byte-identici sul corpus da 100 script e sul long gold.

Criteri di accettazione:

- manifest byte-identici tra 1 e N thread su 100 script + 20 long script;
- `cargo test --workspace` verde con e senza `VISUALNER_MODEL_DIR`;
- utilizzo CPU del worker > 1 core misurato su una sceneggiatura lunga, con
  numero riportato (non stimato);
- nessuna regressione su recall/objects/queries/charts/sequences del long gold.

**Misura e decisione 2026-11-24.** Baseline sul worker di release ricostruito
dal tree corrente, 20 long script / 10 lingue / 5.669 parole, modelli UDPipe
attivi (`VISUALNER_MODEL_DIR`): due esecuzioni dello stesso binario danno
**40,34–51,95 s** di parete e **90–97% di un core** (47,12 s di CPU
nell'esecuzione più lenta: 43,69 user + 3,43 sys), RSS massimo **~1,46 GiB**,
replay identico, 0 errori di grounding. Il worker resta sequenziale e non
supera un core: non c'è parallelismo nascosto da rimuovere, ma neppure il
beneficio promesso.

**Decisione: nessun `par_iter` in questa wave.** Il registry è condiviso e
mutabile e le scene leggono le identità registrate dalle precedenti; un
parallelismo per scena cambia gli id canonici e quindi il manifest, mentre il
criterio di accettazione richiede manifest byte-identici tra 1 e N thread sul
corpus 100 + 20 long script. La sequenza corretta resta quella del piano:
profilo della singola sceneggiatura, separazione fasi per-scena/script-wide,
decisione esplicita sulla semantica delle identità, poi pool legato a
`SEMANTIC_ENGINE_WORKERS`. I criteri CPU>1 core e 1-vs-N restano **non
soddisfatti per decisione**, non per omissione: aggirarli con un registry
per-scena senza la fusione deterministica produrrebbe un output diverso, non
più veloce. La misura sopra è la baseline da confrontare quando la decisione
verrà presa.

Rischio: dipendenze nascoste tra scene (identità canoniche, contatori di id).
Il test di determinismo è la condizione per procedere, non un extra.

## 3. Memoria e benchmark in CI — BLOCKED (misura + runner)

RSS osservato: 1.7 GiB di picco sul corpus da 100 script. Non è ancora noto
quale fase lo determini; senza profilo ogni riduzione è una scommessa.

Passi:

1. Profilare `heaptrack` o `dhat` sul corpus da 100 script (release worker) e
   attribuire il picco: cache, modelli UDPipe per thread, manifest clonati o
   `String` duplicate.
2. Ridurre solo la causa misurata; rieseguire lo stesso profilo.
3. Aggiungere un binario di benchmark che emetta JSON con parole/s, P50/P95 e
   picco RSS.
4. Job CI che confronta con una baseline versionata: fallisce oltre il 10% di
   peggioramento sull'RSS; per le parole/s tolleranza larga perché i runner
   sono rumorosi.
5. Registrare la baseline nel ticket con host, data e comando.

Criteri di accettazione:

- profilo allegato con la causa dominante del picco;
- riduzione documentata **prima/dopo** sullo stesso host e corpus;
- job CI che fallisce su una regressione introdotta di proposito;
- nessuna soglia basata su un singolo campione.

Blocchi: `heaptrack`/`dhat` non sono installati in questo ambiente e i runner
CI del repository non sono disponibili da qui. La parte di harness è
implementabile, la prova di regressione no.

## 4. Piano editoriale spiegabile — DONE (2026-11-24)

`overlay-compiler/src/editorial_plan_v2/planner/` ha già i passi `candidates`,
`coalesce`, `media`, `presentation`. Il grosso del planner è atterrato in
`359d29e`; il residuo planner è stato consolidato e congelato in `7012c62`.
I golden possono ora essere estesi per aggiungere il campo `decisions`.

Passi:

1. Estendere i golden congelati insieme al nuovo campo, mantenendo il rendering
   corrente byte-identico.
2. Estendere `editorial_plan.v2` in modo additivo: `decisions[]` con
   `candidate_id`, `stage`, `outcome`, `reason_code` e riferimento alle prove.
3. `reason_code` come enum stabile (`BUDGET_EXCEEDED`, `OVERLAP_LOST_TO`,
   `NO_ASSET`, `LOW_SCORE`, `CLUSTER_CASCADED`, …). Niente testo libero nel
   contratto: i test devono restare deterministici.
4. Registrare la decisione nei punti in cui il pianificatore già scarta.
5. Rigenerare i golden congelati con `examples/compile_fixture` e verificare
   che il campo sia additivo per i consumatori v2.
6. Esporre il campo nel report del job Go; un comando "perché manca l'overlay a
   00:42?" può venire dopo.

Criteri di accettazione:

- ogni candidato scartato ha una decisione con motivo tipizzato;
- golden rigenerati dal planner reale, non scritti a mano;
- `overlay_plan_contract` e i test di determinismo verdi;
- nessun campo obbligatorio nuovo per un piano v2 già valido.

Chiusura 2026-11-24. `FinalPlanV2.decisions` è additivo
(`#[serde(default, skip_serializing_if = "Vec::is_empty")]`) e ogni voce porta
`candidate_id`, `scene_id`, `entity_id`, `canonical_entity_id`, `stage`,
`outcome` e `reason_code` (enum stabile). I motivi coprono la lane entità
(`ENTITY_KIND_UNSUPPORTED`, `MAP_COVERS_ENTITY`, `DUPLICATE_IDENTITY`,
`TEXT_CARD_LIMIT`, `NO_TIMING_WINDOW`), l'ammissione (`MAPS_ONLY`,
`CATEGORY_DISABLED`, `GROUP_LIMIT`, `BUDGET_EXCEEDED`, `PER_SCENE_LIMIT`) e il
layout (`OVERLAP_LOST`, `CLIPPED_OUT`). Due golden congelati sono stati
rigenerati dal planner reale (`maps_trim_overlap` +16 voci,
`chain_collision_resolution` +16); gli altri otto restano byte-identici perché
non perdono candidati. Il test
`every_decision_is_typed_and_names_its_candidate` gira sull'intera matrice e
`cargo test -p overlay-compiler --locked` passa **166 lib + 34 contract**.
Non sono registrate come decisioni le perdite di `chain_overlap`, che restano
contate in `intake.dropped_by_reason`.

## 5. Più lingue — BLOCKED (annotatore madrelingua + modelli)

Oggi 10 lingue; la morfologia è coperta solo per alcune. Il gate è
`lexicon/tools/build_language_packs.py --check`.

Passi:

1. Scegliere 2–3 lingue in base all'uso reale (candidati: `ar`, `ja`, `hi`).
2. Verificare che esista un modello UDPipe per ciascuna: per `ja`/`zh` servono
   segmentazione e offset UTF-8 diversi, non è "un lessico in più".
3. Per lingua: pack di stopword e lessico, alias dei toponimi flessi (come
   TICKET-24), 2 copioni gold e query contestuali.
4. Checklist "lingua pronta": pack, test di determinismo, recall sul gold,
   `--check` verde.
5. Preferire il lemmatizzatore del modello UDPipe a una tabella scritta a mano.

Criteri di accettazione:

- `build_language_packs.py --check` verde con la nuova lingua;
- 2 script gold per lingua, con provenienza e certificazione dichiarate;
- recall/precision per lingua riportati separatamente (non aggregati);
- nessuna regola specifica di fixture.

Blocco: senza annotazione madrelingua la qualità non è certificabile (stesso
blocco di TICKET-08). La parte di impianto è fattibile; la certificazione no.

## 6. Preflight prima del render — DONE (motore e wire)

Confine: `pipelinegen-muscles` esegue il render; l'esistenza degli asset e lo
stato canonico sono di Go. Un preflight che decidesse di bloccare in Rust
violerebbe quel confine.

Fatto (TICKET-38): `pipelinegen_muscles::preflight::check` implementa i controlli
su asset, sottotitoli e overlay (incluse le sovrapposizioni fra un overlay
lungo e quelli successivi), riusa `probe_file` per aprire i media, restituisce
un report con codici stabili e severità, e non scrive nulla. 14 test unitari +
4 test su media reale.

Fatto con TICKET-39 (2026-11-19):

1. `preflight` è un'operazione `mediaexec.v2` con payload tipizzato, report
   opzionale nella `Response` (i literal sono aggiornati; le risposte senza
   report restano byte-identiche) e un golden condiviso con il consumer.
2. Il chiamante Go `rustexec.Client.Preflight` traduce il report in una
   `PreflightDecision` Go-owned: blocca sulle issue `error`, avvisa sulle
   `warning`, e fallisce chiuso se `ok:false` non ha una issue `error`.
3. Resta fuori scope il budget delle sovrapposizioni concorrenti: la richiesta
   non lo porta e inventarlo in Rust sposterebbe la policy.

Criteri di accettazione:

- l'operazione non crea, modifica o cancella file;
- ogni errore ha un codice tipizzato e un percorso;
- i test coprono i quattro casi negativi e un caso pulito;
- il chiamante Go è l'unico a decidere il blocco;
- `cargo test -p pipelinegen-muscles` e il gate Go restano verdi.

Tutti i criteri sono soddisfatti e verificati il 2026-11-19 (4 golden di
contratto validati, copia consumer senza drift, `cargo test -p
pipelinegen-muscles` e `go test ./internal/platform/{rustworker,media/rustexec}`
verdi). Il binario media pinnato `v0.3.7` precede l'operazione: il deployment
attende la release che la contiene (TICKET-27).

## Ordine consigliato

1. Committare il planner e i golden (prerequisito di 4 e della release).
2. Misurare la memoria (3) prima di qualunque ottimizzazione.
3. Decidere la semantica delle identità e poi parallelizzare (2).
4. Preflight (6) come prossima feature a rischio basso.
5. Lingue (5) quando esiste un annotatore.

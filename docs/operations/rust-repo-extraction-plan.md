# Piano di estrazione del workspace Rust in una repo separata

Stato: piano storico importato da `refactored/docs/operations/` il 2026-10-23.
L'estrazione è stata eseguita il 2026-10-22 con l'approccio submodule
(`refactored/rust` → BrainRust). Il 2026-10-23 sono stati aggiunti la release
di artefatti (`release.yml` + `scripts/release/build-artifacts.sh`), il
consumatore `make rust-fetch` in PipelineGen, il contratto esplicito
`contract/mediaexec.v1` (schema + golden + `verify-rust-contract`) e il gate
"niente cargo" sul percorso di build ordinario; il fallback `make build-muscles`
resta disponibile e compila ancora dal submodule. I path `rust/...` citati sono
quelli del monorepo PipelineGen.
Data: 2026-10-22.
Oggetto: estrarre `refactored/rust/` in una repository dedicata e consumarlo
dal monorepo come submodule pinnato a tag, con artefatti binari verificabili.

## 1. Obiettivo e non-obiettivi

### Obiettivo

1. Togliere ICU4C, cargo, `rustup` e il download dei modelli UDPipe dal
   percorso di build ordinario del monorepo Go.
2. Dare al motore Rust un CI, un rilascio e un versionamento propri, senza
   cambiare il contratto osservabile (NDJSON stdio, `mediaexec.v1`).
3. Rendere i crate riusabili fuori dal monorepo (`visualner`, `mediasampler`,
   `ndjson-stdio`) senza duplicare il codice.

### Non-obiettivi

- Non introdurre linking statico, cgo, FFI o chiamate in-process: il confine
  resta processo + NDJSON.
- Non pubblicare su crates.io in questa fase.
- Non cambiare le responsabilità di dominio descritte in `rust/README.md`: Go
  resta composition root, proprietario di stato, job e decisioni editoriali.
- Non spostare `semantic_acceptance/` né `testdata/mediaexec/`: sono fixture di
  certificazione del consumatore Go.

## 2. Cos'è già pronto

| Fatto | Evidenza |
| --- | --- |
| Il confine è già di processo | `internal/platform/media/rustexec/protocol.go:14` `ProtocolVersion = "mediaexec.v1"`, NDJSON su stdio |
| I binari sono già path-configurabili | `internal/platform/config/types_external.go:25,36` (`VELOX_RUST_*_PATH`) |
| Il workspace è autonomo nel codice | `rust/Cargo.toml`, 6 crate, 110 file `.rs`, ~33k righe, 139 file tracciati |
| `target/` è già escluso | `.gitignore:85`, `.dockerignore:30` |
| Il modello multi-repo è già in uso | `.gitmodules`: i tre repository del motore Chronon, `RenderingGen`, `refactored` |
| Git history separabile per path | nessun submodule dentro `rust/`, quindi `filter-repo --subdirectory-filter` è sicuro |

## 3. Accoppiamenti da recidere

Ogni riga è un blocco: finché non è chiuso, la repo standalone non è
buildabile o il monorepo non può smettere di compilare cargo.

| # | Accoppiamento | File | Decisione |
| --- | --- | --- | --- |
| C1 | I test `semantic-core` leggono i lessici Go | `rust/semantic-core/src/phrase_impact_tests/common.rs:12` → `../../config/lexicons` | Vendorizzare in `rust/lexicon/languages/` con manifest di checksum e gate di parità nel monorepo |
| C2 | Il contratto è duplicato a mano | struct JSON in `rustexec/*.go` vs `*.rs`, `schema_version` letterali | Schema JSON versionato nel repo Rust + contract test consumer-driven nel monorepo |
| C3 | I binari si costruiscono dal sorgente locale | `make/build.mk:95-104`, `Dockerfile:71-93` | Release di artefatti firmati/verificati + fallback di build da submodule |
| C4 | Il pin ICU è incoerente | `rust/.cargo/config.toml` fissa ICU 70, Docker fissa 72, CI legge `pkg-config` | Un solo pin, esplicito, verificato dal consumatore |
| C5 | I modelli UDPipe (234 MB) sono provisioning fuori repo | `rust/visualner/tools/install_udpipe_models.sh` | Restano provisioning CI; la release li dichiara come requisito runtime |
| C6 | Il registro dei componenti mescola path Rust e fixture Go | `config/verify-components.json:464-483` (`rust-muscles`) | Componente Go separato per il contratto; il componente Rust migra nel nuovo CI |

## 4. Inventario completo dei punti di contatto

Da aggiornare nella fase di integrazione, uno per uno. Nessun path `rust/` deve
sopravvivere nel monorepo senza un motivo esplicito.

### Build e gate

- `make/build.mk:42-44` — `RUST_CARGO`, `ICU_MAJOR`, `RUST_ICU_ENV`.
- `make/build.mk:95-104` — `build-muscles`: 4 binari distinti installati in 6
  nomi (`phrase_impact` e `scene_intelligence` sono copie di
  `pipelinegen-muscles`, dispatch per nome binario).
- `make/build.mk:106-112` — `rust-models`.
- `make/build.mk:114` — `build: go-version-check build-muscles`.
- `make/verify.mk:13-14` — `verify-rust-muscles`.
- `make/vidrush.mk:20-23,39` — `verify-visualner`, `verify-mediasampler`.
- `make/audio.mk:10,15` — test su `rust/pipelinegen-muscles/Cargo.toml`.
- `Makefile:27-52` — elenco target; `Makefile:81` — help text.
- `scripts/ci/scene-intelligence-certification.sh:7` — `RUST_MANIFEST`.
- `.github/workflows/ci.yml:224-266` — job `rust` (test + clippy `-D warnings`).

### Runtime e packaging

- `Dockerfile:71-93` — stage `rust-builder` (Debian Bookworm, ICU 72, `--locked`).
- `Dockerfile:100-116` — `libicu72` a runtime, copia dei 4 binari, `VELOX_RUST_*_PATH`.
- `internal/platform/config/types_external.go:25,36` — default `bin/*`.
- `internal/platform/media/rustexec/` — 61 file, ~12k righe: consumer del protocollo.

### Metadati e documentazione

- `config/verify-components.json:464-483` — componente `rust-muscles`
  (`rust/`, `semantic_acceptance/`, `testdata/mediaexec/`, `RUST_SEMANTIC_ENGINE_TODO.md`).
- `.gitignore:85`, `.dockerignore:30`.
- Commenti che citano path Rust: `internal/capabilities/stockintelligence/local_search.go:13`,
  `internal/capabilities/scripts/vidrush_semantic_ports.go:8-9`,
  `internal/capabilities/scripts/editorial_runtime_cert_test.go:24`.
- `semantic_acceptance/gazetteer_garlasco_measure.py:17` — esempio `--worker rust/target/release/semantic-engine`.
- `rust/README.md` — va diviso: parte "repo Rust" resta, parte "integrazione
  con Go" migra in `docs/operations/`.

## 5. Passi

### Fase 0 — Preparazione nel monorepo (nessun cambio di repo)

Tutti i passi sono reversibili e verificabili con i gate esistenti.

**0.1 C1 — Lessici self-contained.**
- Aggiungere `rust/lexicon/languages/<lang>/{stopwords,function_words}.txt`
  generati da `config/lexicons/` con uno script versionato, più
  `rust/lexicon/languages/MANIFEST.sha256`.
- `phrase_impact_tests/common.rs` legge solo da `CARGO_MANIFEST_DIR` (nessun
  `../..`), con fail-closed se il manifest manca.
- Nel monorepo un gate confronta i due alberi e fallisce su divergenza, così la
  copia non può derivare in silenzio.
- Uscita: `grep -rn '\.\./\.\./config' rust/` non restituisce nulla.

**0.2 C4 — Pin ICU unico.**
- Rimuovere i valori hardcoded da `rust/.cargo/config.toml` (lasciare solo
  `LD_LIBRARY_PATH` documentato) e dichiarare il major in un solo posto
  (`rust/rust-toolchain.toml` + variabile CI).
- Allineare Docker (`72`), CI (derivato da `pkg-config`) e documentazione allo
  stesso valore; il binario pubblicato dichiara il major con cui è stato linkato.
- Uscita: build su host con ICU diverso fallisce con messaggio esplicito, non
  con errore di linker.

**0.3 C2 — Contratto esplicito.**
- Aggiungere `contract/` nel workspace Rust: JSON Schema per
  `mediaexec.v1`, `semantic_manifest.v2`, `chapter_manifest.v1`,
  `scene_intelligence.v1`, più un golden set NDJSON per richiesta/risposta.
- Nel monorepo, un contract test esegue i golden contro il binario pinnato: se
  il Rust cambia una forma senza bump di versione, il test fallisce.
- Documentare la matrice `tag Rust ↔ commit Go ↔ versioni di contratto`.
- Uscita: mutazione volontaria di un campo in un golden ⇒ test rosso.

**0.4 CI autonomo.**
- Job: `cargo test --workspace --locked`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo fmt --check`, provisioning modelli con
  cache, build release `--locked` su toolchain pinnata.
- Uscita: la suite passa su un checkout pulito senza il monorepo.

**0.5 C3 — Rilascio artefatti.**
- Tag `vX.Y.Z` ⇒ release con i 4 binari, `sha256sums.txt`, `TOOLCHAIN`,
  `ICU_MAJOR`, `CONTRACT_VERSIONS`, note di breaking change.
- Nel monorepo: target `rust-fetch` che scarica, verifica checksum e major ICU,
  e installa in `bin/` — stesso layout che `build-muscles` produce oggi.
- `build-muscles` resta disponibile come fallback (build dal submodule) per
  Docker e ambienti offline.
- Uscita: `make rust-fetch RUST_VERSION=vX.Y.Z` produce un `bin/` identico a
  quello di `make build-muscles` (confronto hash dei binari sul runner).

### Fase 1 — Estrazione della history

- `git filter-repo --path rust/ --path-rename rust/:` su un clone dedicato,
  poi `git remote add origin` e push. Conserva autori e messaggi.
- Verifiche post-estrazione: `git log --oneline | wc -l` = 113 (i commit che
  toccano `rust/`), nessun file fuori dal workspace, `cargo test` verde.
- Creare `rust-toolchain.toml`, `LICENSE`, `README.md` (parte repo) e
  `.github/workflows/ci.yml`.
- Decisione da prendere qui: nome repo (`pipelinegen-rust` proposto) e
  visibilità.

### Fase 2 — Integrazione nel monorepo

- Aggiungere `rust` a `.gitmodules`, pinnato al tag della Fase 0.5.
- Aggiornare `make/build.mk` (`rust-fetch`, `build-muscles` via submodule),
  `make/verify.mk`, `make/vidrush.mk`, `make/audio.mk`.
- `Dockerfile`: lo stage `rust-builder` compila dal submodule pinnato con
  `--locked`, oppure copia gli artefatti della release verificati per checksum.
  Scegliere uno dei due, non entrambi, e documentarlo.
- `config/verify-components.json`: separare `rust-muscles` (ora solo fixture Go
  del contratto) dal componente Rust, che vive nel nuovo CI.
- Aggiornare commenti e `semantic_acceptance` che citano `rust/target/...`.
- Uscita: `make build`, `make verify-main`, `docker build` verdi.

### Fase 3 — Cutover

- Rimuovere `rust/` dal monorepo (il contenuto è nel submodule), aggiornare
  `.gitignore`/`.dockerignore`, spostare la documentazione di integrazione.
- Aggiungere un gate che vieti `cargo` nei target di build ordinari:
  `grep -q cargo make/*.mk` deve fallire.
- Provare il rollback: ripristinare la build da sorgente locale in meno di
  un'ora, senza toccare il codice Go.

## 6. Rischi

| # | Rischio | Impatto | Mitigazione |
| --- | --- | --- | --- |
| R1 | Drift del contratto tra i due repo | Alto: manifest accettato ma interpretato male | Schema + golden + contract test consumer-driven; bump obbligatorio (0.3) |
| R2 | Disallineamento ABI ICU | Alto: binario non parte o crash a runtime | Pin unico (0.4), `ICU_MAJOR` nella release, check lato consumatore |
| R3 | Il monorepo continua a compilare cargo | Medio: nessun beneficio, doppio costo | Gate di Fase 3 + misura del tempo di `make build` prima/dopo |
| R4 | Catena di supply chain degli artefatti | Alto: binario non verificato | Release su repo noto, `sha256sums`, pin per tag, niente `latest` |
| R5 | Due PR per ogni cambio semantico | Medio: attrito quotidiano | Rilasci frequenti, tag mobile in pre-release, contratto stabile prima dello split |
| R6 | Deriva dei lessici vendorizzati (C1) | Medio: test Rust non più "onesti" | Manifest di checksum + gate di parità nel monorepo |
| R7 | Perdita di contesto nei commit | Basso | `filter-repo` conserva la history; memo nel README sul commit di split |
| R8 | Modelli UDPipe assenti in CI nuova | Medio: suite fail-closed | Cache + retry + timeout espliciti; documentare il requisito runtime |
| R9 | Raddoppio del costo CI | Basso/Medio | Il repo Rust gira solo su `rust/**`; il monorepo smette di pagare ICU |
| R10 | Nessuna copertura: solo `linux/amd64` testato | Medio | Dichiarare il supporto limitato; non pubblicare piattaforme non testate |
| R11 | `unsafe_code = "deny"` e lints di workspace persi | Basso | Riportare `[workspace.lints]` nel nuovo `Cargo.toml` e verificarlo in CI |

## 7. Criteri di uscita

Lo split è completo solo quando **tutti** questi punti sono verificati, con
comando e output registrati.

### Repo Rust autonomo

1. `git clone <rust-repo> && cd <rust-repo> && cargo test --workspace --locked`
   verde su host con solo `libicu-dev` e modelli provisionati, **senza** alcun
   checkout di PipelineGen sul filesystem.
2. `cargo clippy --workspace --all-targets -- -D warnings` e
   `cargo fmt --check` verdi.
3. `grep -rn '\.\./\.\./' src/` non trova path che escono dal repo; nessun
   riferimento a `config/lexicons`, `testdata/` o `bin/` del monorepo.
4. `[workspace.lints.rust] unsafe_code = "deny"` presente e attivo.
5. Release `v0.1.0` pubblicata con 4 binari, `sha256sums.txt`, `ICU_MAJOR`,
   `CONTRACT_VERSIONS`, toolchain pinnata; build `--locked` riproducibile.

### Monorepo consumatore

6. `make build` **non** invoca `cargo` (gate `grep` in CI) e produce un `bin/`
   con gli stessi 6 nomi di oggi.
7. `make rust-fetch RUST_VERSION=<tag>` installa i binari verificando checksum
   e major ICU; con checksum sbagliato fallisce.
8. `docker build` verde con ICU 72 e i 4 binari nel runtime.
9. `make verify-main` e `scripts/ci/scene-intelligence-certification.sh` verdi;
   i gate Rust (`verify-rust-muscles`, `verify-visualner`, `verify-mediasampler`)
   girano nel nuovo repo oppure sono sostituiti da un job di contratto sul
   binario pinnato — nessun target del monorepo invoca più `cargo`.
10. Contract test consumer-driven verde contro il binario pinnato; la mutazione
    volontaria di un golden lo rende rosso (prova che il gate funziona).
11. Matrice di compatibilità scritta (`tag Rust ↔ commit Go ↔ versioni di
    contratto`) e usata dal test al punto 10.
12. Rollback provato: build dal submodule sorgente ripristinata in < 1 ora,
    senza modifiche al codice Go.

### Criterio di stop

Se entro la Fase 0 i punti 0.1–0.3 non sono ottenibili a costo ragionevole,
convergere sull'alternativa: **restare nel monorepo e pubblicare solo artefatti
e crate**. Si ottengono build più veloci e riuso senza il costo del doppio PR;
lo split completo slitta a quando il motore semantico si stabilizza.

## 8. Ordine consigliato e stima

| Fase | Contenuto | Stima |
| --- | --- | --- |
| 0 | Lessici, pin ICU, contratto, CI, release | 3–5 giorni |
| 1 | Estrazione history + repo nuovo | 0,5–1 giorno |
| 2 | Submodule, Makefile, Docker, registro componenti | 1–2 giorni |
| 3 | Cutover, gate, rollback provato | 0,5–1 giorno |

Le fasi 0.1–0.3 sono utili anche in caso di stop: rendono il confine verificabile
e riducono il drift attuale indipendentemente dallo split.

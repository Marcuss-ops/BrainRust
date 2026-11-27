# BR-AUDIT-18 / BR-61 — Registro globale delle identità

- **Priorità:** P1
- **Stato:** PARTIAL (normalizzazione sul wire completata; heap decodificato ancora v3)
- **Owner:** `semantic-core`, `semantic-engine`
- **Origine:** audit 2026-11-26 §7, ticket proposto BR-61.

## Problema

`semantic-engine/src/scene_facts.rs:298` scrive in ogni scena una copia
dell'intero registro accumulato:

```rust
identities: registry.identities().to_vec(),
```

Con N scene e identità che crescono, il manifest contiene O(N²) copie dello
stesso dato. La simulazione dell'audit (50 scene × 3 identità nuove) dà 3.825
record copiati contro 150 identità effettive; il costo non è solo RAM Rust:
si propaga a JSON, cache persistente, traffico NDJSON e memoria del consumer
Go.

## Cosa è stato fatto

1. **Censimento consumer** in PipelineGen: nessun sito legge il registro
   cumulativo; `scene.identities` serve solo a `projectSemanticPersonRoles`,
   `projectSemanticPersonAges`, `projectSemanticPersonRelations` e alla
   validazione delle entità linkate. Dettaglio in
   [BR-AUDIT-08](08-NORMALIZED-MANIFEST.md).
2. **Contratto v4** (`contract/semantic-manifest.v4/`): una tabella
   `identities` a livello documento + `identity_ids` per scena. La conversione
   è `semantic_core::to_v4_value`, costruita sui byte canonici v3 per non
   alterare la codifica dei campi non-identità.
3. **Determinismo**: gli id mantengono l'ordine di prima registrazione; la
   tabella usa l'ultima occorrenza di ogni id, che è l'unica con la mention list
   completa. La conversione è ripetibile e non muta il manifest v3.
4. **Routing**: il worker produce v3 di default e v4 su richiesta; una versione
   sconosciuta fallisce senza fallback. La cache in memoria è version-scoped; la
   disk cache conserva v3 e ri-proietta sulla hit v4.
5. **Consumer Go**: accetta v3 e v4; per v4 idrata la vista della scena con i
   soli id referenziati. La forma nuova è dietro `v4`, quindi il rollback è
   omettere `manifest_version`.
6. **Misura** su 70 scene (147 identità uniche): JSON 8.699.179 B → 2.647.005 B
   (−69,57%), record identità sul wire 9.900 → 147, risposta NDJSON e entry di
   cache in memoria con lo stesso delta. VmHWM 390.116 kB invariato perché il
   manifest decodificato resta v3.

## Cosa resta

La struttura interna `SemanticManifest`/`SceneManifest` continua a contenere la
copia cumulativa per scena: `to_v4_value` è una proiezione wire. Per azzerare
anche l'heap Rust e le entry della disk cache serve separare la struttura
interna dal contratto:

1. tenere nel `SemanticManifest` una sola tabella `identities`;
2. sostituire `SceneManifest.identities` con `identity_ids` e aggiornare tutti i
   consumer Rust (`discourse`, `story`, `consolidate`, `groups`, `facts`,
   `relations`, `candidates`);
3. mantenere la serializzazione v3 come vista calcolata finché il cutover non è
   completo;
4. ripetere la misura su 50 e 200 scene, includendo RSS e disk cache.

Questo refactor è ampio e tocca la parità byte-for-byte: va fatto dopo il flip
di produzione a v4, non insieme al contratto.

## Criteri di accettazione

- [x] Il registro delle identità esiste una sola volta per manifest **sul wire
      v4**; la struttura decodificata resta cumulativa finché non si chiude il
      refactor interno.
- [x] Ogni scena contiene solo riferimenti (id) alle identità che usa, in v4.
- [x] Il manifest resta deterministico e byte-identico a parità di input.
- [x] JSON, RSS e traffico NDJSON sono misurati prima/dopo su un corpus lungo
      (RSS invariato per design, dichiarato nel ticket).
- [x] Il consumer Go è migrato o la nuova forma è dietro `v4`: entrambe.
- [x] Nessun test o golden viene riscritto per mascherare una differenza.

## Verifica

```sh
cd BrainRust
cargo test -p semantic-core manifest
cargo test -p semantic-engine analysis_equivalence
cargo test -p semantic-engine protocol

cd ../refactored
go test ./internal/platform/media/rustexec -count=1
```

## Dipendenze / rischi

- Dipende dal cutover di contratto tracciato in BR-AUDIT-08, ora preparato.
- La rimozione della copia v3 richiede una release esplicita e il flip di
  PipelineGen a v4; fino ad allora v3 resta la forma di rollback.
- La normalizzazione interna è il passo che riduce l'RSS decodificato e le entry
  della disk cache; il contratto v4 da solo riduce wire, risposta e cache in
  memoria.

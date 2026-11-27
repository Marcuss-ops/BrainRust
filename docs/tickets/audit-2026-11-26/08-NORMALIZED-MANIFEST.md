# BR-AUDIT-08 / BR-62 — Manifest normalizzato (contratto v4)

- **Priorità:** P2
- **Stato:** CLOSED (cutover preparato; v3 resta il default)
- **Owner:** `semantic-core`, `semantic-engine`, PipelineGen
- **Origine:** audit 2026-11-26, finding P1 sulle copie cumulative.

## Problema

`scene_facts.rs` scrive `identities: registry.identities().to_vec()` in ogni
`SceneManifest`. Con N scene e identità che crescono, il manifest contiene
O(N²) copie e lo stesso costo si propaga a JSON, cache, NDJSON e memoria Go.

## Censimento consumer (PipelineGen)

`scene.identities` è letto solo dove serve la vista della singola scena:

| Sito Go | Uso | Serve il registro cumulativo? |
| --- | --- | --- |
| `semantic_engine.go` `projectSemanticSceneLanguage` | passa `scene.Identities` a `projectSemanticPersonRoles`, `projectSemanticPersonAges`, `projectSemanticPersonRelations` | no, solo gli id citati nella scena |
| `semantic_engine_entity_linking.go` | risolve le surface delle identità durante la validazione delle entità linkate | no |
| `semantic_engine.go` `semanticSceneCollectionsMissing` | richiede che la collezione sia presente | no, basta una vista non-nil |
| `semantic_engine.go` `buildSemanticCandidateBundles` | deriva `IdentityIDs` dal contenuto della scena | no |

Nessun consumer Go legge il registro cumulativo: la proiezione documentale usata
dal planner è già `manifest.Entities` → `SemanticScript.Identities`. Il
censimento è stato eseguito con:

```sh
rg -n "\.Identities|scene\.Identities|Identities:" refactored/internal/platform/media/rustexec/*.go
```

## Contratto v4

Il contratto condiviso vive in `BrainRust/contract/semantic-manifest.v4/`:

- `README.md` — forma wire, invarianti, piano di cutover e rollback.
- `manifest.schema.json` — schema JSON Draft-7 della forma normalizzata.
- `goldens/manifest-v3.json` e `goldens/manifest-v4.json` — lo stesso contenuto
  nelle due versioni, per la revisione del delta.
- `validate.py` — schema + controlli incrociati (id unici, riferimenti
  risolvibili, ogni identità referenziata, scene delle mention esistenti,
  nessuna copia per scena).
- `check_consumer_copy.sh` — gate di parità byte per byte con la copia
  PipelineGen in `internal/platform/media/rustexec/testdata/semantic-manifest/v4/`.

Forma v4:

- `identities` una sola volta a livello documento, in ordine di prima
  registrazione, con la mention list completa (l'ultima occorrenza vince).
- ogni scena porta `identity_ids` (gli id con una mention in quella scena,
  nell'ordine della tabella) e **non** porta `identities`.
- ogni identità della tabella è referenziata da almeno una scena.
- ogni campo non-identità mantiene valore e codifica v3: la proiezione parte
  dai byte canonici v3, non da un `serde_json::Value` che allargherebbe gli f32
  (`0.76000005` → `0.7600000500679016`).

## Implementazione

- `semantic-core/src/manifest/v4.rs`: `SCHEMA_VERSION_V4`,
  `PRODUCIBLE_SCHEMA_VERSIONS`, `ManifestVersion::parse`, `global_identities`,
  `identity_ids_for_scene`, `to_v4_value`.
- `semantic-engine/src/protocol.rs`: envelope `AnalyzeScriptRequest` con
  `manifest_version` opzionale (default v3). Il campo sta sull'envelope, non su
  `ScriptRequest`, perché seleziona la codifica della risposta e non è un input
  dell'analisi.
- `semantic-engine/src/protocol/worker.rs`: routing esplicito della versione,
  chiave di cache con suffisso di versione, disk cache sempre in v3 e
  ri-proiezione sulla hit v4.
- PipelineGen `rustexec`: accetta `semantic_manifest.v3` e
  `semantic_manifest.v4`; una versione sconosciuta fallisce con
  `unsupported schema_version`. Per v4 `hydrateNormalizedIdentities` valida la
  tabella e idrata la vista della scena con i soli id referenziati (shim di
  cutover, non una seconda rappresentazione permanente).

## Misura

Corpus lungo: `it_stadium.txt` ripetuto 10 volte (70 scene, 147 identità uniche).

| Metrica | v3 | v4 | Delta |
| --- | ---: | ---: | ---: |
| Manifest JSON | 8.699.179 B | 2.647.005 B | −69,57% |
| Record identità sul wire | 9.900 | 147 | −98,5% |
| Risposta NDJSON (una riga) | 8.699.179 B + envelope | 2.647.005 B + envelope | ≈ −69,6% |
| Entry di cache in memoria (`encoded`) | 8.699.179 B | 2.647.005 B | −69,57% |
| VmHWM del processo di analisi | 390.116 kB | 390.116 kB | invariato per design* |

\* Il motore analizza ancora con il registro v3 e proietta v4 sul wire; l'heap
decodificato non cambia finché non si chiude la normalizzazione interna di
BR-AUDIT-18. La disk cache continua a conservare il manifest v3, quindi le entry
su disco non diminuiscono in questo cutover: diminuiscono la risposta, l'entry
in memoria e il traffico NDJSON.

Corpus di equivalenza (6 scene): v3 126.345 B → v4 102.555 B (−18,8%), 175
copie contro 44 identità uniche.

Comando della misura:

```sh
cd BrainRust
export VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models
BRAINRUST_V4_REPEATS=10 cargo test -p semantic-engine --lib \
  manifest_v4_wire_probe -- --ignored --nocapture
# V4_WIRE_PROBE scenes=70 repeats=10 v3_bytes=8699179 v4_bytes=2647005 \
#   saved_bytes=6052174 saved_pct=69.57 identity_copies=9900 \
#   unique_identities=147 VmHWM: 390116 kB
```

## Criteri di accettazione

- [x] Esiste un contratto v4 documentato con tabella globale + riferimenti.
- [x] v3 resta byte-identico finché il consumer non è migrato
      (`v4_normalization_keeps_v3_byte_identical_and_collapses_identity_copies`).
- [x] Il worker rifiuta una versione sconosciuta senza interpretarla come v3
      (Rust `unsupported manifest_version`, Go `unsupported schema_version`).
- [x] JSON e traffico NDJSON misurati prima/dopo su un corpus lungo; RSS
      misurato e invariato per design (l'heap decodificato resta v3); entry di
      cache in memoria misurata.
- [x] Nessun golden esistente è stato rigenerato; i due golden v3/v4 sono nuovi
      e derivati dallo stesso fixture per revisione del delta.

## Verifica

```sh
cd BrainRust
python3 contract/semantic-manifest.v4/validate.py
bash contract/semantic-manifest.v4/check_consumer_copy.sh \
  ../refactored/internal/platform/media/rustexec/testdata/semantic-manifest/v4

cargo test -p semantic-core manifest::v4
cargo test -p semantic-engine --lib v4_normalization
cargo test -p semantic-engine --lib analyze_script_routes_manifest_versions

cd ../refactored
go test ./internal/platform/media/rustexec -count=1
```

Esiti eseguiti il 2026-11-27: `validate.py` e parity check ✅; i quattro test
Rust ✅ (`semantic-core` 3, `semantic-engine` 1); `go test
./internal/platform/media/rustexec` ✅ (69,7 s); `go vet` ✅.

## Residuo (post-cutover, fuori da questo ticket)

1. Flip del producer a v4 per job/ambiente aggiungendo
   `"manifest_version":"semantic_manifest.v4"` alla richiesta.
2. Rimozione della copia v3 e dello shim Go dopo che tutti i consumer leggono la
   tabella globale, con una release che lo dichiara.
3. Normalizzazione dell'heap decodificato Rust: tracciata da BR-AUDIT-18.

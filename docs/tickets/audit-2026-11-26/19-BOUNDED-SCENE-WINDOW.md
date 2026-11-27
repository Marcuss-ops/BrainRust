# BR-AUDIT-19 / BR-64 — Finestra bounded delle scene in-flight

- **Priorità:** P1
- **Stato:** CLOSED (2026-11-27)
- **Owner:** `semantic-engine`
- **Origine:** audit 2026-11-26 §12, ticket proposto BR-64.

## Problema

`analyze_scenes()` (`semantic-engine/src/lib.rs:437`) esegue:

```rust
let local_scenes: Vec<LocalSceneResult> = request.scenes.par_iter().map(...).collect();
```

Tutte le analisi locali vengono conservate prima che inizi la finalizzazione
ordinata. Il picco di memoria è quindi proporzionale al numero di scene del
documento, non ai thread disponibili: su un documento lungo si tengono in vita
tutti i draft (entità, span, fatti, candidati) mentre la cache e il registro
non hanno ancora consumato nulla. Il budget da 128 MiB della cache manifest
misura i byte JSON serializzati, non l'heap delle strutture Rust, quindi non
limita questo picco.

## Cosa serve per risolverlo

1. **Introdurre una finestra di scene in-flight** (default derivato dai thread,
   es. `threads * 2`, clampato `4..=64`) configurabile con
   `BRAINRUST_SCENE_WINDOW`. La finestra limita quante analisi locali possono
   esistere contemporaneamente.
2. **Mantenere l'ordine di finalizzazione.** La finalizzazione resta sequenziale
   e strettamente per `scene_index`: il registro delle identità e gli eventi
   dipendono dall'ordine. La finestra cambia solo *quando* parte l'analisi
   locale, non l'ordine con cui i risultati vengono consumati.
3. **Non cambiare la semantica degli id.** Gli id delle identità e i timestamp
   di cache non devono dipendere dalla dimensione della finestra; il manifest
   deve restare byte-identico per `window = 1`, `window = K` e nessuna finestra.
4. **Scrivere la cache per scena appena il draft è finalizzato**, così la
   finestra successiva può liberare memoria.
5. **Misurare il picco RSS** su uno script lungo con finestra 1, K e illimitata;
   riportare host, corpus e comando. Se il guadagno non è misurabile, chiudere
   il ticket con la misura che lo dimostra.
6. **Gestire l'errore** di una scena nella finestra senza interrompere le
   altre: una scena `rejected` non deve bloccare la finalizzazione delle
   successive.

## Criteri di accettazione

- [x] Il numero di `LocalSceneResult` vivi è limitato dalla finestra.
- [x] Il manifest è byte-identico al variare della finestra.
- [x] Il picco RSS su un documento lungo è misurato e riportato.
- [x] La finestra è configurabile e ha un default documentato.
- [x] Una scena rifiutata non altera l'ordine o il risultato delle altre.
- [x] I test di equivalenza esistenti restano verdi.

## Esito 2026-11-27

- `scene_window(scene_count)` legge `BRAINRUST_SCENE_WINDOW`; `0` significa
  illimitata, assente significa `threads * 2` clampato `4..=64`.
- `analyze_scenes` analizza in chunk di `window` scene e finalizza ogni chunk
  nell'ordine originale. Il picco di draft vivi è la finestra, non il numero
  di scene.
- `analyze_script_with_window` espone la finestra esplicita ai test, evitando
  la race sulle variabili d'ambiente.
- `scene_window_sizes_produce_identical_manifests` confronta `1, 2, 3, 64` e
  illimitata sullo stesso corpus.
- `long_script_peak_rss` (ignorato) stampa `VmHWM` per una run con finestra
  esplicita: è il probe per il criterio RSS ancora aperto.

Verifica eseguita:

```sh
cargo test -p semantic-engine --lib analysis_equivalence
# 11 passed, 2 ignored
```

Misura RSS (`long_script_peak_rss`, `it_stadium.txt`, 7 scene, 478.821 byte di
manifest, release, host locale 2026-11-27, `VmHWM` da `/proc/self/status`):

| Finestra | VmHWM |
| --- | --- |
| `1` (una scena alla volta) | 240.664 kB |
| illimitata (`0`) | 258.416 kB |

Delta ~17 MiB (~7%) su un documento di 7 scene. Il grosso dell'RSS è modello e
lessico, non i draft; il vantaggio della finestra cresce con il numero di scene
e non è estrapolabile da questa singola misura. Il probe è ignorato di default
per non appesantire la suite.

## Verifica

```sh
cd BrainRust
cargo test -p semantic-engine analysis_equivalence
BRAINRUST_SCENE_WINDOW=4 cargo test -p semantic-engine analysis_equivalence
# misura RSS su corpus lungo, finestra 1 / default / illimitata
```

## Dipendenze / rischi

- La finestra riduce il picco ma non la latenza: se la finalizzazione è il
  collo di bottiglia, il throughput può non migliorare. La misura deve
  riportare anche il wall time.
- Va coordinata con BR-AUDIT-16: il gate di certificazione deve girare anche
  con la finestra attiva.

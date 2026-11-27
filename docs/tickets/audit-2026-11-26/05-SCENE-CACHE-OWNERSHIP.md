# BR-AUDIT-05 — Cache per scena posseduta dal worker

- **Priorità:** P1
- **Stato:** CLOSED
- **Owner:** `semantic-engine`
- **Origine:** audit 2026-11-26, finding P1 su `analyze_script()`.

## Problema

`analyze_script()` chiamava `SceneCache::from_env()` a ogni richiesta.
`SceneCache::open()` esegue `create_dir_all` e `prune()`, quindi ogni nuova
analisi scandiva la directory delle scene per la retention. La cache manifest,
al contrario, vive nel `Worker`.

## Soluzione

`Worker` possiede `Option<Arc<SceneCache>>`, inizializzata una volta in
`Worker::from_env()`. `analyze` usa
`analyze_script_with_scene_cache(&request, self.scene_cache.as_deref())`.
`Worker::new`/`with_disk` restano memory-only per test ed embedder. Il campo è
esposto in `health` come `scene_cache_enabled`.

## Criteri di accettazione

- [x] Nessuna chiamata a `SceneCache::from_env()` nel percorso di analisi.
- [x] La cache per scena è condivisa tra le richieste del worker.
- [x] `Worker::new` non abilita la cache per scena.
- [x] Health espone lo stato della cache per scena.

## Verifica

```sh
cd BrainRust
cargo test -p semantic-engine protocol
cargo test -p semantic-engine analysis_equivalence
```

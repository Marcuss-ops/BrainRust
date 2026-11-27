# BR-AUDIT-01 — Chiave della cache per scena completa

- **Priorità:** P0
- **Stato:** CLOSED
- **Owner:** `semantic-engine`
- **Origine:** audit 2026-11-26, finding P0 sulla cache `cache/scene.rs`.

## Problema

La chiave della cache per scena copriva testo, lingua, `AnalyzerVersions` e
`cache_identity`, ma non `ScriptContext`, `reference_date` ed
`entity_safety_cap`. Tutti e tre gli input raggiungono `analyze_local_scene` e
possono cambiare il draft:

- il contesto globale dei nomi cambia la canonicalizzazione e la selezione
  delle menzioni;
- la data di riferimento risolve i relativi deittici (`ieri`, `3 anni fa`);
- il safety cap limita `ExtractOptions.entity_count` e quindi `top_n`.

Il ticket di progetto `DEMOL-10` prescriveva già
`sha256(scene.text || language || safety_cap || reference_date ||
person_context_hash || analyzer_versions_local)`; l'implementazione aveva
omesso tre componenti.

## Soluzione

`SceneCache::key` ora accetta e hasha anche:

1. il contesto effettivo (`person_name_tokens` + `person_surnames`);
2. `reference_date` con un marcatore esplicito per `None`;
3. `entity_safety_cap`.

`SCENE_CACHE_FORMAT` è passato a `2`, così le entry scritte con la chiave
incompleta non vengono più lette.

## Criteri di accettazione

- [x] La chiave cambia al variare di testo, lingua, analyzer, contesto, data e
      safety cap.
- [x] `analyze_local_scene` passa i tre input alla chiave.
- [x] Le entry del vecchio formato sono un miss.
- [x] Test unitari sulla chiave e test di equivalenza esistenti verdi.

## Verifica

```sh
cd BrainRust
cargo test -p semantic-engine cache::scene
cargo test -p semantic-engine analysis_equivalence
```

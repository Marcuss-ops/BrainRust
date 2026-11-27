# BR-AUDIT-07 — Riuso degli span di frase già calcolati

- **Priorità:** P1
- **Stato:** CLOSED
- **Owner:** `visualner`, `semantic-engine`
- **Origine:** audit 2026-11-26, finding P1 sulla tokenizzazione ripetuta.

## Problema

Per ogni scena, `visualner::analyze_with_context_preserving_mentions_and_reference`
calcolava gli span di frase e `semantic-engine::analyze_local_scene` li
ricalcolava con una seconda chiamata a `split_sentences`. Lo stesso testo veniva
segmentato due volte.

## Soluzione

`visualner::Analysis` espone `sentence_spans`, popolato dall'unica
segmentazione già eseguita in `analysis.rs`. `LocalSceneDraft` non duplica più
il campo: `finalize_scene_with_search` preleva gli span dall'analisi. La cache
per scena serializza gli span dentro l'analisi.

## Criteri di accettazione

- [x] `analyze_local_scene` non chiama più `split_sentences`.
- [x] Gli span sono presenti nell'analisi e sopravvivono al round-trip cache.
- [x] Il manifest resta byte-identico nei test di equivalenza.

## Verifica

```sh
cd BrainRust
cargo test -p visualner analysis
cargo test -p semantic-engine analysis_equivalence
```

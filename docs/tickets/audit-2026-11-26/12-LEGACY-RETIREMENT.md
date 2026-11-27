# BR-AUDIT-12 — Registro del ritiro legacy (umbrella BR-69/70/71)

- **Priorità:** P3
- **Stato:** DEFERRED
- **Owner:** `semantic-engine`, `overlay-compiler`, `pipelinegen-muscles`,
  PipelineGen
- **Origine:** audit 2026-11-26 §10, BR-LEGACY-01.

Questo file è il registro unico delle superfici legacy ancora presenti. Il
lavoro operativo è diviso in tre ticket dedicati, così ogni superficie ha un
censimento e un criterio di cutover propri:

| Superficie | Ticket |
| --- | --- |
| `compile_legacy_editorial_plan_line`, `CompileEditorialPlanRequest` | [BR-AUDIT-23](23-LEGACY-PLANNER.md) |
| `resolve_map_overlaps`, `trim_map_windows_before_budget`, `rank_entity_overlays`, `cascade_clustered_entrances` | [BR-AUDIT-24](24-GRANULAR-OPERATIONS.md) |
| `background_music`/`sfx`, parser chart fallback, `CHAIN_HANDOFF_US`, `chain_camera`, `playfair_display`, `udpipe_model_slots` | [BR-AUDIT-25](25-AUDIO-CHART-FONT-FALLBACKS.md) |

## Stato del censimento

| Superficie | Consumer noto | Stato |
| --- | --- | --- |
| `compile_legacy_editorial_plan_line` | dispatch interno su versione != v2 | da censire su PipelineGen |
| `CompileEditorialPlanRequest` | percorso legacy | da censire |
| Operazioni NDJSON granulari | dispatch `Worker::handle_line` | da censire su PipelineGen |
| `background_music` / `sfx` | contratto audio PipelineGen | da censire |
| Parser chart fallback | `groups/charts/series.rs` | attende copertura quantità tipizzate |
| `CHAIN_HANDOFF_US` | planner/tests | da censire |
| `chain_camera` fallback | planner/tests | da censire |
| `playfair_display` | `presentation.rs` + preset RenderingGen | da censire |
| `udpipe_model_slots` | `health` + `protocol_tests` | duplicazione probe |

## Cosa serve per chiudere l'umbrella

1. Completare i censimenti dei tre ticket figli.
2. Ritirare le superfici senza consumer, con una finestra di deprecazione
   esplicita per quelle che ne hanno uno.
3. Aggiornare contratto, documentazione e pin di release.
4. Verificare con un batch reale PipelineGen che il solo contratto canonico
   sia sufficiente.

## Criteri di accettazione

- [ ] Ogni superficie della tabella ha un esito: rimossa o mantenuta con owner
      e data di ritiro.
- [ ] Nessuna superficie resta "da censire".
- [ ] Il wire reale non usa più il percorso legacy.
- [ ] BR-AUDIT-23/24/25 sono chiusi o esplicitamente bloccati con motivo.

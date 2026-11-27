# BR-AUDIT-25 / BR-71 — Ritirare i fallback audio, chart e font

- **Priorità:** P2
- **Stato:** OPEN
- **Owner:** `overlay-compiler`, `semantic-engine`, `pipelinegen-muscles`,
  PipelineGen
- **Origine:** audit 2026-11-26 §10 e BR-LEGACY-01, ticket proposto BR-71.

## Problema

Restano superfici di compatibilità con scadenze diverse, ancora tutte vive nel
codice. Ognuna ha un consumer e un rischio diverso; vanno trattate una per una,
non con un unico interruttore.

| Superficie | Dove | Cosa serve |
| --- | --- | --- |
| `background_music` / `sfx` legacy | contratto audio PipelineGen ↔ Rust | migrare al contratto audio canonico; verificare che il payload legacy non arrivi più al worker |
| Parser fallback chart | `semantic-engine/src/groups/charts/series.rs` | rimuoverlo quando le quantità tipizzate coprono tutti i casi; prima serve la matrice di copertura |
| `CHAIN_HANDOFF_US` legacy | `overlay-compiler/src/editorial_plan_v2/planner` | richiedere timing esplicito; rimuovere la costante quando nessun piano la usa |
| `chain_camera` fallback | `overlay-compiler/src/editorial_plan_v2/planner` | migrare i piani che non specificano la camera; poi rimuovere il default implicito |
| `playfair_display` | `overlay-compiler/.../planner/presentation.rs` | ritirare l'alias: la famiglia runtime è `playfair_display_italic`; aggiornare i piani e il catalogo |
| `udpipe_model_slots` | `semantic-engine/src/protocol/worker.rs` health | duplicazione delle probe; rimuoverla dopo il cutover delle probe di health |

## Cosa serve per risolverlo

1. **Per ogni riga della tabella**, censire i consumer (Rust, Go, fixture,
   job reali) e allegare il comando/risultato.
2. **Definire il criterio di cutover** specifico:
   - audio: payload canonico obbligatorio e validazione che rifiuta il legacy;
   - chart: copertura delle quantità tipizzate >= casi del parser fallback;
   - chain: timing esplicito presente in tutti i piani golden;
   - camera: nessun piano senza `chain_camera` nel corpus;
   - font: nessun piano o preset usa l'alias;
   - health: probe migrata e test aggiornato.
3. **Rimuovere la superficie** solo dopo che il criterio è verificato; in caso
   contrario mantenere il fallback con una data di ritiro e un owner.
4. **Aggiornare fixture e golden** in modo esplicito: un golden che cambia
   perché il fallback è stato rimosso va motivato nel ticket.
5. **Verificare il wire con PipelineGen** su un batch reale per audio e chain,
   non solo con i test Rust.

## Criteri di accettazione

- [ ] Ogni superficie ha un censimento dei consumer e un criterio di cutover.
- [ ] Le superfici ritirate non hanno chiamanti di produzione.
- [ ] I fallback ancora necessari hanno owner e data di ritiro.
- [ ] I fixture/golden aggiornati sono spiegati, non rigenerati in blocco.
- [ ] Un batch reale PipelineGen passa con il contratto canonico.
- [ ] `playfair_display` non è più accettato come alias.

## Verifica

```sh
cd BrainRust
cargo test -p overlay-compiler --all-targets
cargo test -p semantic-engine --test editorial_plan_protocol
grep -rn "playfair_display\|CHAIN_HANDOFF_US\|udpipe_model_slots" \
  --include=*.rs --include=*.go .
```

## Dipendenze / rischi

- Il parser chart fallback non va rimosso prima della matrice di copertura
  delle quantità: è una rete di sicurezza, non codice morto finché la copertura
  non è dimostrata.
- L'alias font è consumato anche da preset di rendering esterni: la rimozione
  richiede l'allineamento con RenderingGen.

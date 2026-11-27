# BR-AUDIT-23 / BR-69 — Ritirare il planner editoriale legacy

- **Priorità:** P2
- **Stato:** OPEN
- **Owner:** `semantic-engine`, `overlay-compiler`, PipelineGen
- **Origine:** audit 2026-11-26 §10 e BR-LEGACY-01, ticket proposto BR-69.

## Problema

Il worker mantiene due contratti editoriali:

- `compile_overlay_plan_v2`, il percorso completo usato dalla produzione;
- `compile_legacy_editorial_plan_line` (`semantic-engine/src/protocol.rs:450`),
  raggiunto quando la richiesta non è v2, con `CompileEditorialPlanRequest`.

Il secondo esiste per compatibilità, ma finché resta attivo il worker accetta
due rappresentazioni della stessa decisione editoriale. Ogni nuova regola va
verificata due volte e il percorso legacy può divergere silenziosamente.

## Cosa serve per risolverlo

1. **Censire i producer** che inviano ancora il contratto legacy: versioni di
   schema, job, test e consumer PipelineGen.
2. **Migrare i producer residui** a `editorial_plan.v2`; per ognuno registrare
   il commit o la configurazione che completa la migrazione.
3. **Rimuovere `compile_legacy_editorial_plan_line`**, la struttura
   `CompileEditorialPlanRequest` e i relativi fixture/test, oppure isolarli
   dietro una feature di compatibilità con scadenza esplicita se un consumer
   esterno non è aggiornabile subito.
4. **Mantenere il fail-closed**: una versione sconosciuta deve rispondere con
   `UnsupportedOperation`, non essere interpretata come legacy.
5. **Aggiornare la documentazione del contratto** e il pin di release.
6. **Verificare con il worker reale** che PipelineGen invii solo v2 per un
   batch completo.

## Criteri di accettazione

- [ ] Nessun producer di produzione invia il contratto legacy.
- [ ] Il percorso legacy è rimosso (o dietro feature con scadenza e owner).
- [ ] Una versione sconosciuta fallisce con errore tipizzato.
- [ ] I test del protocollo coprono v2 e il rifiuto delle versioni ignote.
- [ ] Il pin di release e la documentazione sono aggiornati.
- [ ] Un batch reale PipelineGen passa senza fallback legacy.

## Verifica

```sh
cd BrainRust
cargo test -p semantic-engine --test editorial_plan_protocol
cargo test -p overlay-compiler --test overlay_plan_contract
grep -rn "compile_legacy_editorial_plan\|CompileEditorialPlanRequest" \
  --include=*.rs --include=*.go .
```

## Dipendenze / rischi

- Dipende dal censimento di BR-AUDIT-22: una superficie legacy può essere
  raggiunta da un'operazione NDJSON granulare ancora chiamata.
- La rimozione anticipata rispetto alla migrazione rompe il wire: il ticket si
  chiude dopo la migrazione, non prima.

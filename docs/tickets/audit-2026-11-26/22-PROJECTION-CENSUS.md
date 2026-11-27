# BR-AUDIT-22 / BR-68 — Censimento delle proiezioni inutilizzate

- **Priorità:** P1
- **Stato:** OPEN
- **Owner:** `overlay-compiler`, `semantic-engine`, PipelineGen
- **Origine:** audit 2026-11-26 §10.1 e §18, ticket proposto BR-68.

## Problema

Dopo il cutover a `compile_overlay_plan_v2` restano superfici di proiezione che
potrebbero non avere più un consumer di produzione. Sul lato Go il deletion
guard elenca già `projectEditorialOverlays`, `projectChapterBullets`,
`overlaySceneInput`, `buildOverlaySceneSurfaces` e
`allocateSceneImageCandidates`; sul lato Rust non esiste un censimento
equivalente. Il rischio è mantenere due rappresentazioni della stessa richiesta
e due percorsi che possono divergere, oltre a codice che i test coprono ma la
produzione non usa.

## Cosa serve per risolverlo

1. **Inventariare ogni funzione di proiezione** in `overlay-compiler` e
   `semantic-engine` (richiesta editoriale, scene, bullet, media, chart, mappe)
   con: file, visibilità, chiamanti di produzione, chiamanti di test.
2. **Classificare** ogni superficie:
   - viva in produzione;
   - usata solo dai test come oracolo di parità;
   - morta.
3. **Rimuovere le morte** e marcare come `#[cfg(test)]` le funzioni che servono
   solo come oracolo, così non entrano nel binario di produzione.
4. **Verificare il wire NDJSON**: nessuna operazione deve sopravvivere solo
   perché un test la chiama (collegamento con BR-AUDIT-24).
5. **Produrre una tabella nel ticket** con la decisione per ogni superficie e
   il comando che dimostra l'assenza di chiamanti di produzione.
6. **Non rimuovere un oracolo di parità** se è l'unico modo di verificare un
   contratto: in quel caso va solo isolato dietro `#[cfg(test)]`.

## Criteri di accettazione

- [ ] Esiste una tabella completa delle superfici di proiezione con i chiamanti.
- [ ] Ogni superficie morta è stata rimossa.
- [ ] Le funzioni test-only non sono compilate nel binario di produzione.
- [ ] Nessuna operazione NDJSON resta solo per i test.
- [ ] I golden e i test di parità restano verdi.
- [ ] Il diff non cambia il comportamento del planner.

## Verifica

```sh
cd BrainRust
grep -rn "project_editorial_scenes\|overlaySceneInput\|projectChapterBullets" \
  --include=*.rs --include=*.go .
cargo test -p overlay-compiler --all-targets
cargo test -p semantic-engine --test editorial_plan_protocol
```

## Dipendenze / rischi

- Il censimento Go è già in parte fatto dal deletion guard; va ripetuto sul
  commit corrente perché il consumer può essere cambiato.
- Una superficie "morta" può essere usata da un job esterno non presente nel
  repository: la verifica include una ricerca nei log/chiamate di produzione
  prima della rimozione.

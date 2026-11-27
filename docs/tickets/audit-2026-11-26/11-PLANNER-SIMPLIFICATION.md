# BR-AUDIT-11 / BR-66 — Semplificazione del planner editoriale

- **Priorità:** P2
- **Stato:** DEFERRED
- **Owner:** `overlay-compiler`
- **Origine:** audit 2026-11-26, finding P1 sul numero di passaggi che si
  correggono a vicenda.

## Problema

`compile_plan()` esegue admission, dedup, occlusion, gap pacing, motion,
coalescing, overlap, presentazione, nuova dedup e clipping. Alcune fasi
invalidano decisioni precedenti e richiedono una seconda correzione.

## Perché è deferred

È una riscrittura incrementale delle regole editoriali, non un bug isolato.
Richiede una matrice di parità sui piani sigillati e il consenso dei consumer
PipelineGen.

## Cosa serve per risolverlo

1. Definire per ogni fase le proprietà certificate che le fasi successive non
   possono invalidare (admission, scheduling, composition, presentation, seal).
2. Introdurre il costo temporale della presentazione **prima** del seal: se una
   frase viene divisa in tre elementi, il costo deve essere noto quando il
   layout viene certificato.
3. Ridurre le correzioni ripetute: una sola risoluzione degli overlap e una
   sola deduplicazione dopo che le chain hanno modificato le finestre, oppure
   una fase che le renda impossibili a valle.
4. Misurare i piani su un corpus golden prima/dopo: numero di item, copertura,
   durata, scarti.
5. Non riscrivere l'algoritmo in un colpo: introdurre il principio di
   proprietà certificate una fase alla volta, con i golden byte-identici come
   rete di sicurezza.

## Criteri di accettazione

- [ ] Ogni fase ha un contratto scritto su cosa garantisce e cosa non può
      cambiare a valle.
- [ ] Nessuna finestra temporale viene ricalcolata due volte per lo stesso
      item senza una motivazione.
- [ ] Il costo della presentazione è noto prima del seal.
- [ ] I piani sul corpus golden sono confrontati prima/dopo.
- [ ] I contratti `overlay_plan_contract` restano verdi.

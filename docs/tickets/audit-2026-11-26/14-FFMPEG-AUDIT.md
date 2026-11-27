# BR-AUDIT-14 / BR-74 — Audit dei processi FFmpeg

- **Priorità:** P3
- **Stato:** DEFERRED
- **Owner:** `pipelinegen-muscles`
- **Origine:** audit 2026-11-26, finding P2 su `render_audio_execution.rs`.

## Problema

`render_audio_execution.rs` usa quattro stem paralleli per default e
`PIPELINE_MUSCLES_AUDIO_STEMS` li configura; il valore viene ridotto al numero
di chain, senza un tetto indipendente delle risorse. Con più job audio
contemporanei il prodotto job × stem × thread FFmpeg può saturare la macchina.

## Perché è deferred

Non si possono dichiarare inutili singoli processi FFmpeg senza un trace di
esecuzione e misure per job.

## Cosa serve per risolverlo

1. Tracciare per job: numero di processi FFmpeg, numero di `ffprobe` per
   asset, stem paralleli effettivi (`PIPELINE_MUSCLES_AUDIO_STEMS`), thread
   interni FFmpeg, byte PCM intermedi, numero di ricodifiche e cache hit della
   loudness.
2. Misurare il prodotto `job × stem × thread` su un batch reale, non su un
   singolo job isolato.
3. Introdurre un tetto indipendente delle risorse per i processi media (non
   solo `min(stems, chains)`) e un budget globale condiviso con il motore
   semantico ([BR-AUDIT-17](17-GLOBAL-RESOURCE-BUDGET.md)).
4. Eliminare i probe/decode ripetuti solo dove il trace dimostra la
   duplicazione; niente rimozioni basate sul codice statico.
5. Riportare i numeri nel ticket: processi per job, picco RSS, wall time,
   cache hit.

## Criteri di accettazione

- [ ] Esiste un trace per job con i contatori sopra.
- [ ] Il numero di processi media è limitato da un budget esplicito.
- [ ] I probe ridondanti dimostrati dal trace sono eliminati.
- [ ] Le misure di wall/RSS sono allegate.
- [ ] Nessun job supera il budget in un batch concorrente.

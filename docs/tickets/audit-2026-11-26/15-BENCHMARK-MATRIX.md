# BR-AUDIT-15 / BR-75 — Benchmark automatici e parità

- **Priorità:** P3
- **Stato:** DEFERRED
- **Owner:** `semantic-engine`
- **Origine:** audit 2026-11-26, finding P1 sulla copertura dei test di
  parallelismo.

## Problema

`analysis_equivalence_tests.rs` copre 1 vs 4 thread, due run parallele e cache
cold/warm, ma la fixture ha sei scene italiane, nessun input editoriale e una
sola data di riferimento. Non certifica il corpus completo.

## Perché è deferred

La matrice richiesta (100 script, 20 lunghi, 11 lingue, modelli assenti/UDPipe/
GLiNER, cache corrotta, RSS) richiede corpus e host dedicati. BR-AUDIT-01/02
aggiungono i casi di non-collisione della chiave; il resto richiede una
campagna di misura.

## Cosa serve per risolverlo

1. Distinguere due gate: **correttezza** (parità byte-for-byte, tracciata in
   [BR-AUDIT-16](16-CERTIFICATION-MATRIX.md)) e **benchmark** (wall, CPU, RSS,
   cache hit, qualità). Questo ticket copre il secondo.
2. Aggiungere al gate i casi di cache key già coperti, così un benchmark non
   misura un percorso che non è più quello di produzione.
3. Eseguire la matrice su host con modelli installati: 100 script e 20 lunghi,
   profili assenti/UDPipe/GLiNER, thread 1/2/4/8.
4. Registrare per ogni run: wall, CPU, picco RSS, cache hit/miss, qualità del
   manifest e diff byte-for-byte rispetto alla baseline.
5. Produrre un report machine-readable e una soglia di regressione: un run che
   peggiora wall o RSS oltre la soglia fallisce il gate.
6. Archiviare solo baseline e golden, secondo [BR-AUDIT-27](27-REPORT-ARCHIVE.md).

## Criteri di accettazione

- [ ] Esiste un comando unico che produce un report di benchmark.
- [ ] Il report contiene wall, CPU, RSS, cache hit e qualità.
- [ ] Esiste una soglia di regressione applicata automaticamente.
- [ ] La matrice copre i profili modello e i livelli di thread dichiarati.
- [ ] I report storici non vengono versionati a ogni run.

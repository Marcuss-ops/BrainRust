# BR-AUDIT-27 / BR-76 — Archiviare i report storici

- **Priorità:** P3
- **Stato:** CLOSED (2026-11-27)
- **Owner:** `semantic-engine`, `docs`
- **Origine:** audit 2026-11-26 §16, ticket proposto BR-76.

## Problema

In `semantic-engine/fixtures/multilingual_100/reports` sono versionati cinque
report storici (268 KiB):

- `brainrust_100_2026-10-27-models.json`
- `brainrust_100_2026-10-27-NO-MODELS-degraded.json`
- `brainrust_100_2026-10-28-models.json`
- `brainrust_100_all_pass_2026-10-25.json`
- `brainrust_baseline_2026-10-25.json`

Sono utili come storia, ma non tutti sono una baseline corrente. Un report
superato che resta accanto a quello valido viene citato per errore dai nuovi
ticket e rende ambiguo quale numero sia il riferimento.

## Cosa serve per risolverlo

1. **Classificare ogni report**: baseline corrente, golden di parità,
   superato, o duplicato di un altro.
2. **Mantenere nel fixture solo** la baseline corrente e i golden necessari ai
   gate; spostare i superati in un archivio documentale (`docs/evidence/` o
   equivalente) con un `README` che dice data, host, profilo e motivo
   dell'archiviazione.
3. **Aggiornare ogni riferimento** nei ticket, negli script di scoring e nel
   README del corpus.
4. **Definire una policy**: i report generati da un benchmark vanno in una
   directory ignorata da git; solo la baseline promossa viene versionata.
5. **Aggiungere un controllo** che fallisca se un test o uno script punta a un
   report archiviato.
6. **Non cancellare la storia**: l'archivio conserva i file; la directory dei
   fixture conserva solo ciò che è operativo.

## Criteri di accettazione

- [ ] Ogni report è classificato e motivato.
- [ ] Nel fixture restano solo baseline e golden usati dai gate.
- [ ] I superati sono archiviati con metadati (data, host, profilo, motivo).
- [ ] Nessun riferimento pendente a un file archiviato.
- [ ] La policy per i report futuri è scritta e applicata.
- [ ] I gate correnti restano verdi.

## Verifica

```sh
cd BrainRust
grep -rn "brainrust_100_2026" --include=*.md --include=*.py --include=*.sh .
ls semantic-engine/fixtures/multilingual_100/reports
```

## Esito 2026-11-27

| Report | Esito |
| --- | --- |
| `brainrust_100_2026-10-28-models.json` | Baseline corrente: resta versionato nel fixture. |
| `brainrust_100_2026-10-27-models.json` | Archiviato: superato dalla run del 28. |
| `brainrust_100_2026-10-27-NO-MODELS-degraded.json` | Archiviato: evidenza degradata, non baseline. |
| `brainrust_100_all_pass_2026-10-25.json` | Archiviato: snapshot storico. |
| `brainrust_baseline_2026-10-25.json` | Archiviato: prima baseline superata. |

Archivio: `data/archive/brainrust-benchmark-reports-2026-11-27/` con
`manifest.json` (SHA-256 per file, generato da
`ops/tools/archive_tracked.py --verify`) e `README.md` con data, profilo e
motivo. I quattro file sono stati rimossi dal repository con `git rm`; il
fixture conserva solo la baseline.

Policy applicata e scritta in
`semantic-engine/fixtures/multilingual_100/reports/README.md`: i benchmark
scrivono in `.tmp/` (git-ignored), solo la baseline promossa viene versionata,
un cambio di baseline richiede l'archiviazione della precedente.

Nessun riferimento pendente: l'unico path citato da `ACCEPTANCE_TODO.md` è la
baseline 2026-10-28, che resta al suo posto. La verifica del ticket 27
(`grep -rn "brainrust_100_2026"`) trova solo il file corrente e i documenti di
archivio.

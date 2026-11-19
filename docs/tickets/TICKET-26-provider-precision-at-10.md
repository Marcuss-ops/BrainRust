# TICKET-26 — provider precision@10 and the real cascade are unmeasured

- Priority: P1
- Status: BLOCKED
- Source: Gate D in `ACCEPTANCE_TODO.md`; `docs/stato/brainrust.md` §2
- Files: `refactored` provider adapter, opt-in probe

## Problem

Gate D still has three unchecked items: image-query precision@10 against
independently judged provider results, the production provider cascade, and
1,000 overlays/minute. The live probe is opt-in, executes queries but does not
download bytes, and is not the production cascade. Literal source queries and
derived queries are not separated in the measurement.

## Acceptance criteria

- [ ] 20 long-gold queries run through the production cascade; top-10 results
      are judged against the query intent.
- [ ] Precision@10 is reported separately for literal queries and derived
      contextual queries.
- [ ] The run records provider name, response provenance and downloaded bytes;
      failures are fail-closed, not silently empty.
- [ ] Throughput is measured on the same run, or the existing
      `TICKET-MAX-THROUGHPUT-OVERLAY-ARCHITECTURE` measurement is cited and
      the 1,000/minute gate stays there.

## Blocker

This needs provider credentials, a production cascade owner and an independent
judge. No code in this repository can manufacture them; a mocked provider would
not satisfy the gate.

## Local scoring harness — 2026-11-17

`semantic-engine/fixtures/gold_multilingual/long_scripts/provider_precision.py`
now validates and scores a frozen run. It does **not** call a provider and
does not create labels. It requires:

- the 20 derived queries to match `long_scripts/gold.json` exactly;
- exactly 10 unique HTTP results per query, ranks 1..10, each with provider,
  provenance and positive downloaded bytes;
- a separate judgments file with an independent judge id/date and one boolean
  label for every result;
- no relevance labels inside the results file and no provider errors.

It reports precision@10 separately for literal and derived queries. `measured`
requires at least one literal query; otherwise the status is
`blocked_literal_queries_missing`. The command exits 2 for every non-measured
status.

Tests: `python3 provider_precision_test.py` — **8 passed**. The fixture is
synthetic and exercises validation/arithmetic only; it is not a provider
measurement. The acceptance criteria above remain unchecked until a real
production-cascade run and an independent judge provide the two artifacts.

## Next owner action

PipelineGen provider owner runs the 20 queries through the production cascade,
freezes the top-10 results without relevance labels, and has an independent
judge produce the separate labels. Run `provider_precision.py` over both
artifacts, then link the report and the two frozen inputs here. The harness
fails closed rather than treating missing credentials or judges as a pass.

## Prossimi passi (2026-11-19)

1. **Congelare i due artefatti:** top-10 di retrieval senza etichette di
   rilevanza, e le etichette prodotte da un judge indipendente. Senza i due
   file il ticket resta bloccato, non misurabile.
2. **Eseguire `provider_precision.py`** su entrambi e linkare report + input
   congelati in questa scheda, con data.
3. **Sblocco esterno:** il judge e le credenziali del provider non sono nel
   repository; se il judge non è disponibile, il ticket resta `BLOCKED` e non
   si dichiara PASS.

Gate: precision@10 misurata sul report, con i due input riproducibili.

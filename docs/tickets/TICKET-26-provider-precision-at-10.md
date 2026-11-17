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

## Next owner action

PipelineGen provider owner runs the 20 queries, freezes the top-10 results and
the judge's labels, then links the run artifact here.

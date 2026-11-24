# BR-PERF-01 — Scene analysis is sequential and unmeasured

- **Status:** OPEN
- **Priority:** P2
- **Owner:** `semantic-engine`
- **Evidence date:** 2026-11-24

## Problem

`analyze_script` walks the scene batch sequentially and shares one
`IdentityRegistry`, which is what keeps cross-scene identities coherent. The
2026-11-24 review flags this as a possible CPU opportunity but there is no
baseline for time or RSS, and no proof that a parallel schedule preserves the
registry's deterministic resolution order.

The historical 1.7 GiB peak on the 100-script corpus is documentation, not a
reproducible gate.

## Acceptance criteria

- [ ] A reproducible baseline records wall time and peak RSS for the 100-script
      corpus at a pinned commit and machine class.
- [ ] The ticket names which phases are independent (per-scene extraction) and
      which are order-dependent (identity resolution, cross-scene continuity).
- [ ] Any parallel implementation produces byte-identical manifests on the
      existing gold corpus; otherwise it is not merged.
- [ ] The memory bound and the worker's model-slot limit are documented.
- [ ] A regression gate fails if the measured time or RSS exceeds the agreed
      budget.

## Non-goals

- Parallelising identity resolution or changing the registry's tie-breaking.
- Optimising `pipelinegen-muscles`; this ticket is CPU semantic analysis only.

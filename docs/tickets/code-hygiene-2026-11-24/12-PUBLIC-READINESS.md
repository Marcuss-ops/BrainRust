# BR-PUB-01 — Public presentation, license and reproducible demo

- **Status:** OPEN — the license choice is an owner decision, not an agent
  default.
- **Priority:** P3
- **Owner:** repository owner
- **Evidence date:** 2026-11-24

## Problem

The 2026-11-24 review found no tracked `LICENSE`, a README written for people
who already know PipelineGen, and no single copy-paste demo with a known
input/output pair. The technical documentation is strong; the public entry
point is not.

## Acceptance criteria

- [ ] The owner chooses a license and adds the matching tracked `LICENSE` file.
- [ ] The README has a quick start that builds one binary, runs one small
      input and shows the exact output contract.
- [ ] A reproducible benchmark command and its measured environment are linked
      from the README.
- [ ] A minimal fixture or demo script exists and is exercised in CI.
- [ ] The public description states the boundary: BrainRust is the semantic and
      editorial compiler; Go owns state, jobs and external services.

## Why this stays open

Choosing a license has legal consequences and cannot be inferred from the code.
The demo and README work can start only after the owner confirms the intended
audience and the license.

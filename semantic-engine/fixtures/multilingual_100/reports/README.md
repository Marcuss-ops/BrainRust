# multilingual_100 reports

Only the promoted baseline is versioned here.

- `brainrust_100_2026-10-28-models.json` — current baseline (UDPipe models
  installed), cited by `ACCEPTANCE_TODO.md`.

Superseded reports (`2026-10-25` baseline/all-pass, `2026-10-27` models and
no-models) were archived on 2026-11-27 with hash verification to
`data/archive/brainrust-benchmark-reports-2026-11-27/` at the workspace root;
see the README and `manifest.json` there (BR-AUDIT-27).

## Policy

- Benchmark output goes to `.tmp/` (git-ignored).
- A report is committed only after it is promoted to baseline.
- Replacing the baseline requires archiving the previous one with
  `ops/tools/archive_tracked.py` and recording host, profile and reason.

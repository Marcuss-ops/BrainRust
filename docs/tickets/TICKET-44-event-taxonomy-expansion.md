# TICKET-44 — Expand event taxonomy beyond the 12 core types

- Priority: P2
- Status: OPEN (created 2026-11-22)
- Depends on: TICKET-34
- Files: `lexicon/events/*.json`,
  `semantic-engine/src/events.rs`,
  `semantic-core/src/manifest.rs`,
  `semantic-engine/tests/` (event protocol/multilingual suites)

## Problem

`lexicon/events/*.json` (introduced under TICKET-34) currently covers 12 core
event types plus legacy `agreement`:
`arrest`, `election`, `agreement`, `accident_disaster`, `birth`, `death`,
`trial_verdict`, `appointment_resignation`, `founding`, `acquisition_merger`,
`launch_release`, `attack_battle`, `match_competition_result`.

Common documentary, investigative, economic, and science scripts frequently
rely on event types not covered by those 13 buckets:
- `protest_strike` (strikes, demonstrations, riots)
- `bankruptcy_layoff_ipo` (insolvency, mass layoffs, stock listings)
- `sanction_embargo` (trade/diplomatic sanctions, asset freezes)
- `scientific_discovery` (breakthroughs, excavations, astronomical discoveries)
- `outbreak_epidemic` (pandemics, disease outbreaks, quarantines)
- `natural_disaster` (earthquakes, floods, eruptions — separated from human
  `accident_disaster`)
- `crime_incident` (homicide, theft, fraud, kidnapping — distinct from `arrest`
  and `trial_verdict`)
- `ceasefire_truce` (truces, armistices, peace treaties)
- `record_milestone` (world records, all-time highs/lows)

## Scope & Design

1. **Add trigger & role-slot lexicons in `lexicon/events/*.json`:**
   - Author triggers for the new event types across all 11 supported languages
     (`de`, `en`, `es`, `fr`, `id`, `it`, `nl`, `pl`, `pt`, `ru`, `tr`),
     including inflectional/nominalized forms where required by the existing
     event matcher.
   - Verify no trigger collisions with existing types (e.g. `attack_battle` vs
     `ceasefire_truce`, `accident_disaster` vs `natural_disaster`, `arrest` vs
     `crime_incident`).

2. **Wire into `semantic-engine/src/events.rs`:**
   - Reuse existing grounded frame assembly (participants, `place_id`,
     `date`, `status: asserted|negated|uncertain`, `source_fact_ids`,
     thread-aware cross-scene merge, 200-frame safety cap).
   - Keep `SemanticManifest.events` schema additive and backward compatible.

## Acceptance criteria

- [ ] All new event types have trigger coverage in all 11 languages in
      `lexicon/events/*.json` and pass `build_language_packs.py --check`.
- [ ] Runtime tests exercise each new type (asserted, negated, uncertain) in
      all 11 languages, with exact byte spans and 0 false positives on a
      negative non-event corpus.
- [ ] Zero regression on existing TICKET-34 tests and long-gold metrics.
- [ ] `AnalyzerVersions.events` bumped so cached manifests invalidate cleanly.

## Non-goals

- Closing TICKET-34's independent human-annotated gold gate (stays tracked in
  TICKET-34).
- Generating free-form event summaries not verbatim in the script.

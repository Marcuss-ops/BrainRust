# Per-language gold report — 2026-10-28

This report supersedes `PER-LANGUAGE-GOLD-REPORT-2026-10-27.md`. It records the
2026-10-28 re-measurement after the segment-retention restoration, the
multilingual chart/location cue completion, the German and English identity
repairs, and the Indonesian negation fix. Every number below was measured on
this host with the release worker built from `main@10f38db` plus the
2026-10-28 worktree changes.

## 0. Measurement precondition (read this first)

Every number below was produced with the UDPipe models exported:

```sh
cd BrainRust
export VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models      # or ~/.local/share/visualner/models
export RUST_ICU_MAJOR_VERSION_NUMBER=70
```

Without `VISUALNER_MODEL_DIR` the worker logs
`visualner: no UDPipe model installed for <lang>; POS evidence and verb
filtering are disabled` and the same corpora measure worse. Any run that omits
the variable is a *degraded-mode* run and must be labelled as such.

## 1. Commands and artifacts

| Artifact | Command |
|---|---|
| `gold_multilingual/reports/short_gold_report_2026-10-28.json` | `python3 score_gold.py --report …` |
| `gold_multilingual/reports/long_gold_report_2026-10-28.json` | `python3 long_scripts/score_long_gold.py --report …` |
| long-gold shape | `python3 long_scripts/validate_long_gold.py` |
| `multilingual_100/reports/brainrust_100_2026-10-28-models.json` | `python3 benchmark.py --worker target/release/semantic-engine --report …` |
| Go projection (`refactored`) | `VELOX_RUST_SEMANTIC_ENGINE_PATH=…/semantic-engine go test ./internal/platform/media/rustexec/... -count=1` |
| Gate log | `.tmp/closeout5-verification.log` (fmt/lint/release/corpus/golds) |
| Workspace log | `.tmp/closeout5-tests.log` (564 passed, 0 failed, 1 ignored) |

## 2. Short gold — entities and noise, per language

Global: recall **1.000** (98/98), precision **1.000** (112/112), noise leaks
**0**, scorer exit **0 (PASS)**.

| Lang | Scripts | Gold | Found | Recall | Precision | Critical errors |
|---|---:|---:|---:|---:|---:|---|
| de | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| en | 2 | 9 | 9 | 1.000 | 1.000 | 0 |
| es | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| fr | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| id | 2 | 10 | 10 | 1.000 | **1.000** | 0 |
| it | 2 | 11 | 11 | 1.000 | 1.000 | 0 |
| nl | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| pl | 2 | 9 | 9 | 1.000 | 1.000 | 0 |
| pt | 2 | 10 | 10 | 1.000 | 1.000 | 0 |
| ru | 2 | 9 | 9 | 1.000 | 1.000 | 0 |

The only 2026-10-27 precision loss — Indonesian `Bukan` ("not") typed as
LOCATION — is fixed in the shared place predicate: a single-token surface must
now be a known place, not merely a capitalized word after a negation cue. The
short gold is clean in all ten languages.

Determinism: `--check-reproducible` → `PASS (20 scripts × 2 cold runs,
byte-identical manifests)`.

## 3. Long-form gold — 20 scripts, ten languages

`long_scripts/gold.json` is release-sized: 20 scripts, 5,669 words, all ten
corpus languages. **19 of the 20 scripts are assistant drafts** and declare
`authority: assistant_draft_pending_human_review`; only the Italian pilot
inherits `annotation_policy.authority = human_annotator`, and even that label is
historic and not re-verified here. The scorer therefore reports
`certification: blocked_pending_human_review` and never promotes silence to a
certified label.

```
{"script_count": 20, "language_count": 10, "word_count": 5669,
 "is_release_sized": true, "draft_script_count": 19,
 "capability_gap_count": 4,
 "certification": "blocked_pending_human_review"}
```

Overall on the drafts (diagnostic, not certified): named entities **86/124
(0.694)**, surface-only visual objects **99/105 (0.943)**, typed visual objects
**70/105 (0.667)**, important phrases **82/82 (1.000)**, important segments
**19/20 (0.950)**, facts **6/6 (1.000, two Italian scripts only)**, contextual
queries **16/20 (0.800)**, positive charts **17/20 (0.850)**, sequences
**0/80**, bullets **161**, grounding errors **0**, cached replay identical on
every script. The sequence count remains zero: ordered steps are still not
emitted as separate bullets.

| Lang | Ent. | Obj. surface | Obj. typed | Phrases | Segments | Facts | Queries | Charts |
|---|---|---|---:|---:|---:|---:|---:|---:|
| de | 9/12 | 10/10 | 8/10 | 8/8 | 2/2 | n/a | 2/2 | 2/2 |
| en | 9/12 | 10/10 | 7/10 | 8/8 | 2/2 | n/a | 2/2 | 2/2 |
| es | 8/12 | 10/10 | 8/10 | 8/8 | 2/2 | n/a | 2/2 | 2/2 |
| fr | 8/12 | 10/10 | 7/10 | 8/8 | 2/2 | n/a | 2/2 | 2/2 |
| id | 9/12 | 10/10 | 9/10 | 8/8 | 2/2 | n/a | 2/2 | 2/2 |
| it | 16/16 | 15/15 | 9/15 | 10/10 | 2/2 | 6/6 | 2/2 | 2/2 |
| nl | 8/12 | 10/10 | 10/10 | 8/8 | 2/2 | n/a | 2/2 | 2/2 |
| pl | 5/12 | 5/10 | 3/10 | 8/8 | 2/2 | n/a | 0/2 | 1/2 |
| pt | 8/12 | 9/10 | 6/10 | 8/8 | 2/2 | n/a | 2/2 | 2/2 |
| ru | 6/12 | 10/10 | 3/10 | 8/8 | 1/2 | n/a | 0/2 | 0/2 |

Zero source-grounding errors and identical cached replays in every language.

### What changed since 2026-10-27

- **Segment retention repaired.** `d9c3779` had reintroduced a three-window,
  non-overlap cap plus a 0.4 average-score threshold in
  `segment_candidates`, and the anchoring rule required a grounded signal in
  **both** the first and the last sentence. A complete passage that closes on
  contextual information was therefore dropped. The cap and threshold are gone
  again, and a window now needs at least one grounded signal while every
  sentence must remain substantive; every such 2–5 sentence window is retained
  in source order. The Italian pilot's complete three-sentence passage is found
  again (`1/1`), and the long-gold segment recall rises from `0/20` to `19/20`.
  The remaining miss is one Russian script; the per-language table records it
  without attributing a cause.
- **Chart comparisons across the corpus languages.** The comparison-signal
  vocabulary lacked the ordinary contrastive conjunctions the scripts use
  (`während`, `while`, `whereas`, `mientras que`, `tandis`, `alors que`,
  `sedangkan`, `terwijl`, `natomiast`, `enquanto`). With them, chart recall
  rises from `2/20` to `17/20`; the remaining three use bare `a`/`а`, which is
  not safe as a global cue (it is an article or preposition in Italian,
  Spanish and Portuguese) and is deliberately not added.
- **Contextual location queries.** Added `lies in`, `stands in`, `está situado
  en`, `staat in`, `stoi w`, `situa-se em`, `стоит в` to the locative
  predicate list. Queries rise from `10/20` to `16/20`; the remaining four are
  the Polish/Russian inflected city forms below.
- **German identity repair.** A common noun after a location cue is no longer a
  place when the tagger says `NOUN` and the surface is not a known place
  (`seit 1958 in Betrieb`), and `Stadt Leipzig` is an organization cue, so it
  no longer seeds `Leipzig` as a person-name fragment. `de_stadion_long_02`
  now emits `located_in Leipzig` and the query `Stadion Süd Leipzig`.
- **English determiner repair.** `collect_facilities` no longer absorbs a
  sentence-initial article when the same facility was mentioned before:
  `The Harbour Stadium` becomes `Harbour Stadium`, and the English queries are
  `Harbour Stadium Bristol` / `Central Stadium Norwich`.
- **Company and `di`-compound ORG.** The 2026-10-27 defects are closed:
  `Aurora Sport` / `Lumen Energia` are ORGANIZATION, `Comune di Torino` and
  `Università di Torino` are formed, and the Italian pilot reaches `9/9`
  entities with zero typed misses. Regression tests cover both shapes.

## 4. Template corpus — 100/100 with models

**100 PASS, 0 structural problems**, 203,254 words, **446.85 words/s**, P50
4.32 s, P95 6.73 s, **100/100 byte-identical cache replays**, peak RSS
1,733,864 KiB, 442.36 CPU-seconds (0.973 core average). The runner checks
schema, scene counts, byte-grounded entities/bullets/phrases/quotes, grounded
facts, grounded visual queries and contextual-query component integrity.

## 5. Morphology, compounds, scripts — what was reviewed

| Language | Cases | Anchors exercised per case | Observed 2026-10-28 |
|---|---:|---|---|
| de | 10 | compound ORGs (`Prisma-Labor`, `Stella-Rat`, `Aurora-Institut`, `Lumen-Genossenschaft`, `Horizont-Stiftung`), hyphenated persons, `elektrische Straßenbahn`, `Wasserstandssensor` | 10/10 PASS; short-gold persons 10/10; common-noun cue guard added |
| nl | 10 | particles, `noodaggregaat`, `waterstandsensor` | 10/10 PASS |
| pl, ru | 10 each | inflected surnames, transliterated facilities | 10/10 PASS on the synthetic corpus; long-gold inflected toponyms remain a limit (below) |
| id | 10 | one-word surnames, no morphological marking | 10/10 PASS; short-gold `Bukan` FP fixed |
| it, es, pt, fr | 10 each | articulated prepositions, accented surnames, elision | 10/10 PASS |
| en | 10 | anglo compounds (`water-level sensor`), determiner-initial names | 10/10 PASS; determiner trim added |

The Italian long gold now forms `Comune di Torino`, `Università di Torino`,
`Aurora Sport` and `Lumen Energia`. The residual morphology limit is explicit:
Polish and Russian city names appear in inflected forms (`Poznaniu`,
`Krakowie`, `Казани`, `Самаре`) that the gazetteer does not resolve, so their
contextual queries and some entity mentions are missed. Fixing that needs
morphological normalization, not a fixture edit.

## 6. Open gaps, ranked by measured impact

1. **Inflected Slavic toponyms** (`Poznaniu`, `Krakowie`, `Казани`, `Самаре`)
   → pl/ru contextual queries `0/2` and part of the entity/typed-object
   shortfall. Requires a morphology-aware place lookup.
2. **Ordered sequences are not emitted** → `0/80` sequence items exactly
   extracted; the four-step passages are not separate bullets. This is an
   unimplemented product capability, not a tuning issue.
3. **Negated arbitrary predicates** (`pompa.fault = false`,
   `risparmio annuale.demonstrated = false`) → declared
   `capability_gap_expectations`: the shipped fact contract covers
   `inaugurated_year`, `capacity`, `located_in`, `owned_by`; a negated
   `located_in` is emitted and scored (`it_stadio_long_02`, 3/3 facts).
4. **Draft-gold entity and typed-object recall** (`0.694`, `0.667`): the labels
   are unreviewed assistant drafts and include inflected names and one-word
   surnames; these numbers are diagnostic, not a certified quality claim.
5. **Chart recall `0.85`**: the three misses use bare `a`/`а` as the only
   contrastive cue. Adding a single-letter cue would create false charts in
   Italian/Spanish/Portuguese, so it is refused until a language-scoped cue
   mechanism exists.
6. **Provider precision@10 and 1,000 overlays/minute** remain unmeasured: no
   provider requests and no PipelineGen integration run certify them.

## 7. What this report certifies

- **Certified here:** short-gold entity recall/precision and critical-error
  counts in all ten languages (1.000/1.000, zero leaks); byte-identical cold
  replay; 100-script structural grounding with zero problems; the long-gold
  corpus shape, language scope, release size and provenance rules; the
  mechanical fixes for `Bukan`, the Italian company/compound ORGs, the German
  common-noun and `Stadt` cues, the English determiner, the segment anchoring
  rule and the multilingual chart/location cues; the measurement precondition
  (`VISUALNER_MODEL_DIR`) that makes any of it reproducible.
- **Not certified:** human editorial review of the 19 long-script drafts —
  certification is `blocked_pending_human_review`; summary faithfulness and
  subject/negation/date invariants on long scripts; image-query precision@10
  (needs a provider run); object and entity recall as language-wide metrics;
  ordered sequences; production provider cascades and overlay throughput.

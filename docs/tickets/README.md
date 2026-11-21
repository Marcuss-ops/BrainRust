# BrainRust audit tickets — 2026-11-15

Canonical status for the 2026-11-15 architecture audit and its roadmap.
Detailed acceptance evidence lives in each ticket. This file is the only status
matrix; `ACCEPTANCE_TODO.md` remains the historical gate evidence.

Last verified: 2026-11-21, snapshot ongoing local implementation pass. **31 DONE/CLOSED, 6 PARTIAL (30, 32–36), 0 OPEN, 3 BLOCKED (08, 10, 26).**
The final recheck rebuilt the release worker (`3e9bb942…`): `semantic-engine`
passes **345 tests in 28 suites**, the quantity draft scores **1,784 cases
(1,643/1,643)** and the coref draft **440/440**, both with zero false
positives; details in [Verification — 2026-11-21 (final status
recheck)](#verification--2026-11-21-final-status-recheck).
TICKET-27 is CLOSED (`v0.3.8` published on 2026-11-19) and was still counted
among the blocked entries in the previous line; TICKET-40 is CLOSED too.
TICKET-37 adds the persistent manifest cache, TICKET-38 the preflight engine
and TICKET-39 their cross-repo wire to the Go consumer. The
six-workstream performance and product plan with acceptance criteria and
blockers is
[`ROADMAP-2026-11-19-performance-prodotto.md`](ROADMAP-2026-11-19-performance-prodotto.md).

Tickets 15–20 are the 2026-11-16 external-audit follow-ups, and all six are
now DONE (15 phrase selection, 16 MediaMemory ranker, 17 visual budget/chain,
18 linguistics, 19 phrase-timing alignment, 20 identity localization), along
with 21 (Rust-side hotspot remediation). TICKET 22 ports the editorial quality
gate to Rust (`script-qa` plus `quality_gate` on `semantic-engine`) and is DONE
in the working tree as of 2026-11-18: the Go implementation, its flag and its
knob are deleted, the pin is `v0.3.8`, and a 27-scenario Go↔Rust differential
is byte-identical. The cutover and the Rust goldens are committed as of
2026-11-18 (see TICKET-22); TICKET-27 closed on 2026-11-19 with the published
`v0.3.8` release. Tickets 23–26 record the measured
engine and Gate D gaps that had no sheet; 23 (ordered sequences), 24 (Slavic
inflected toponyms) and 25 (bare chart cue) are DONE on 2026-11-17, and 26
remains externally blocked. The audit reconciliation is
[`docs/stato/AUDIT-RICONCILIAZIONE-2026-11-16.md`](../../../docs/stato/AUDIT-RICONCILIAZIONE-2026-11-16.md).

Status vocabulary:

- **DONE** — implemented and covered by the referenced checks.
- **DONE (bound/Rust)** — the local contract is complete; a named external
  parity/benchmark gate remains outside this checkout.
- **BLOCKED** — cannot be completed inside this repository without an external
  owner, dataset, credential or cross-repo contract change.
- **PARTIAL** — some implementation or evidence exists, but acceptance criteria
  remain open.
- **OPEN** — scoped and ready; implementation has not started.

| Ticket | Priority | Status | Outcome |
| --- | --- | --- | --- |
| [01](TICKET-01-ndjson-backpressure.md) | P0 | DONE | End-to-end NDJSON backpressure and panic-safe responses. |
| [02](TICKET-02-imagecatalog-policy.md) | P0 | DONE | Fail-closed catalog policy, snapshot integrity, real dates, `evaluated_at`. The Go policy now agrees on both audited cases: an unrecognized status classifies `unknown` and `retired` never re-enters the recertification pool (verified 2026-11-16). |
| [03](TICKET-03-artifact-publish.md) | P1 | DONE | Unique `.part` files, atomic publish, newest-generation guard. |
| [04](TICKET-04-render-stock-certification.md) | P1 | DONE | `render_stock` probes and certifies the final file before publishing; canonical render integration is 4/4. |
| [05](TICKET-05-manifest-completeness.md) | P1 | DONE | Response-level `completeness` with rejected scene count and reasons. |
| [06](TICKET-06-cache-byte-budget.md) | P1 | DONE (bound + measured) | Entry cap plus 128 MiB serialized-byte budget; 20-script replay peak RSS measured at 1,418,300 KiB. |
| [07](TICKET-07-manifest-reuse.md) | P1 | DONE | Optional `semantic_manifest.v2` on the worker request; same type, validated and reused without a second analysis. No Go consumer exists to wire: `imagecatalog` stays offline by the 13b decision. |
| [08](TICKET-08-editorial-gold-set.md) | P1 | BLOCKED | Requires independent human annotations; no such dataset ships here. |
| [09](TICKET-09-hotspot-contraction.md) | P2 | DONE | Source preparation extracted; duplicate cache fingerprint removed; remaining splits would be cosmetic. |
| [10](TICKET-10-multimodal-pgvector.md) | P2 | BLOCKED | Requires SigLIP/pgvector, a schema and a calibrated held-out dataset. |
| [11](TICKET-11-discourse-continuity.md) | P0 | DONE (deterministic core) | Additive `discourse.v1` references, ambiguity, threads and scene continuity. |
| [12](TICKET-12-mvp-semantic-contract.md) | P0 | DONE (bound/Rust) | v3 linked-entity projection, local index/linker, typed intents, Go v3 consumer, intent/index cache key, verified-hit admission, stop-on-satisfaction and cross-repo smoke pass; only the revised SLO agreement remains external. |
| [13](TICKET-13-go-demolition-closeout.md) | P1 | DONE | 13a DONE (`b6f5379` + `c4c12ac98`): the linker owns NER repair, prompt-echo rejection and image admission; annotations project the decision. 13b DONE (`8b4482660`): verified assets owned by the Go `entitycatalog`, Rust `imagecatalog` remains offline by design. 13c DONE and pushed. 13d DONE 2026-11-16: the four per-concern ports and `legacy_overlay_plan.go` were deleted; production uses one `CompileOverlayPlanV2`. WP5/WP7 closed 2026-11-16: six-fixture production-adapter gate, one-request real-worker counter, deletion guard and `make verify-agent` / `make verify-main` PASS. The producer no longer calls `projectEditorialOverlays`/`projectChapterBullets`; `projectChapterBullets` and the Go presentation layer were deleted on 2026-11-16, `projectEditorialOverlays` remains a test-only helper. the request-side surface builder was removed from production on 2026-11-16; TICKET-14 closed 2026-11-16, so 13d and 13 are DONE. |
| [14](TICKET-14-overlay-planner-single-call.md) | P1 | DONE | Rust single-call compiler landed: complete `editorial_plan.v2` request (seal, annotations, entity timeline, media, budgets, windows), final-plan response with `validate_for`, and one `compile_overlay_plan_v2` call that projects editorial/chapter bullets, entity cards, budgets, map trim/resolution, cascade and seal. Eight request fixtures plus eight planner-generated goldens and a 30-test contract suite are checked in; the worker routes by schema version before deserializing. Entity-card motion/style selection is ported and cards now merge after the editorial admission. Go cutover LANDED 2026-11-16: one `CompileOverlayPlanV2` in production, sealed-plan adoption, legacy planner and four ports deleted, captioned-card chain fusion moved to Rust, chart placements computed in Rust. WP5/WP7 closed 2026-11-16: the stale Go-vs-Rust differential was replaced by an eight-fixture production-adapter gate, the real-worker one-request counter and a deletion guard both PASS, and `make verify-agent` / `make verify-main` PASS. WP4 request-side surface projection DONE 2026-11-16: raw phrases/entities/numbers/dates/maps/charts/heavy/categories are projected in Rust; `overlaySceneInput` no longer has a production caller. All parity lanes closed 2026-11-16: general phrase/image/mixed chains, presentation/light leaks, seeded dates, verified intent images and recap. Go only verifies/materializes media bytes. TICKET-13 13d is DONE. |
| [15](TICKET-15-phrase-engine-demolition.md) | P1 | DONE | `SceneIRSegmentEnricher`, translated NLP and the overlay projection use `select_phrases`; `scripts/phrases/` is deleted and the affected Go/Rust suites pass. No production file imports/calls the retired Go selector or `locatePhraseTimingWithEndpointFallback`; the endpoint helper survives only as a test oracle. Release `v0.3.6` contains `select_phrases`/`align_phrases` and `refactored@4bde5e66e` pins `v0.3.6`, so the deployment prerequisite is closed. |
| [16](TICKET-16-mediamemory-ranker-migration.md) | P1 | DONE | `mediasampler::rank` owns gates, weights, both penalty seats, duration fit, stable order and the diversity pick. `rustexec.MediaRankerAdapter` sends one `rank_candidates` call per slot pool; Go `resolver_orchestration.go` projects the result and does not re-sort. `PickTopFromRose` has no production caller; Go and Rust parity suites pass. Release `v0.3.6` contains `rank_candidates` and the pin is `v0.3.6`, so the deployment prerequisite is closed. |
| [17](TICKET-17-visual-budget-chain-cutover.md) | P1 | DONE (2026-11-16) | Captioned-card and general chain fusion moved to Rust and the Go chain producer plus its tests were deleted; `model/chain_coalesce.go` is wire-only. The Rust `admit` pass owns production admission after ranking and before seal; no production Go file calls the legacy budget producers, and the WP7 guard pins them. The legacy `BuildPlan`/visual-budget helpers remain only as a Go test oracle. Eight fixtures / 30 contract tests, real-worker 8/8, and `make verify-main` PASS. |
| [18](TICKET-18-linguistics-go-demolition.md) | P2 | DONE (2026-11-16) | Rust owns the named-set language profiles (`lexicon_profile`, 12 languages including fallback). Go embeds a generated read-only snapshot; `config/lexicons/**`, `LexiconRegistry` and the filesystem loaders are deleted, and `lexicon_root` is no longer a config knob. Parity gate: `build_language_packs.py --check`. |
| [19](TICKET-19-linguistic-temporal-alignment.md) | P2 | DONE | `semantic-core::phrase_align` + `align_phrases` + `SemanticEngineAdapter.AlignPhrases` own exact phrase alignment; `CompilePhraseTimings`/`CompileSceneSpeechTimings` project the worker result. The endpoint fallback has no production caller. The entity/moment follow-up is closed 2026-11-17: `audio/phrase_locator.go` and `audio.LocatePhrase` are gone (`refactored@8f159549b`) and the voiceover/entity paths send one `align_phrases` request per segment. |
| [20](TICKET-20-identity-localization-consolidation.md) | P2 | DONE (2026-11-16) | Rust owns localized identity matching (`localize_identities`: exact, pl/de/tr inflection, ru/tr/pl ordered fallback). Go builds the typed request, validates every span and projects the source id/image/geo; a missing id fails closed. The old Go matcher is test-only. |
| [21](TICKET-21-hotspot-remediation.md) | P2 | DONE (2026-11-16) | Rust-side hotspot remediation: `render_audio_execution::execute` 419→39 lines with the stages split and `RenderError` at the boundary; `planner.rs` 2,671 production lines split into five modules with `compile_plan` untouched and the frozen goldens byte-identical (64 lib + 30 contract pass); one `entity_search_candidate` factory; one `run_ffprobe` invocation with per-caller wording preserved; `visualner` no-op "fail-closed" dead code deleted; pre-existing `match_like_matches_macro` lint in `phrase_align.rs` fixed. |
| [22](TICKET-22-QUALITY-GATE-RUST-PORT.md) | P2 | DONE (committed 2026-11-18; released in v0.3.8) | Rust `script-qa` is the only editorial gate in the working tree: 8 rules, `quality_gate` on `semantic-engine`, 27 frozen Go goldens plus 3 protocol tests green, and a 27/27 byte-identical Go↔Rust differential against the release binary. The Go package (866 lines), `VELOX_RUST_QUALITY_GATE` and the knob are deleted; `refactored` pins `v0.3.8`. |
| [23](TICKET-23-ordered-sequences.md) | P1 | DONE (2026-11-17) | Ordered sequences: **80/80** items in **20/20** grounded, source-ordered four-member groups; negative set 0 false positives. |
| [24](TICKET-24-slavic-inflected-toponyms.md) | P1 | DONE (2026-11-17) | Locale-scoped inflected alias table resolves `Poznaniu`, `Krakowie`, `Казани`, `Самаре`; pl/ru contextual queries **2/2 each**, overall **20/20**; objects unchanged at 93/105 vs untouched HEAD. |
| [25](TICKET-25-chart-bare-contrastive-cue.md) | P2 | DONE (2026-11-17) | Chart recall **20/20**; the bare `a`/`а` rule requires two grounded values and substantive words on both sides, and a 20-paragraph negative set produces **0 charts**. |
| [26](TICKET-26-provider-precision-at-10.md) | P1 | BLOCKED | Provider precision@10, the real cascade and the byte-level probe need provider credentials and an independent judge. The offline frozen-run scorer (`provider_precision.py`) is ready and tested (9/9), but it cannot manufacture a provider run or labels. |
| [27](TICKET-27-publish-v0.3.7.md) | P0 | CLOSED (2026-11-19) | `v0.3.8` published with the full gate suite, matching release manifest/checksums and four binaries; `refactored` pins the release and `make rust-fetch` passes. |
| [28](TICKET-28-imagecatalog-retirement.md) | P2 | CLOSED (2026-11-18) | The Rust `imagecatalog` crate is retired: workspace entry, crate and tests deleted, shared policy fixture moved under the surviving Go `entitycatalog` test. |
| [31](TICKET-31-relative-dates.md) | P1 | CLOSED (2026-11-19) | Relative dates in 11 languages: `relative_time.json`, token scan, explicit/reference/narrative anchor, calendar-correct arithmetic, `TemporalFact.relative` + `derived`, optional `ScriptRequest.reference_date` in fingerprint/cache, `AnalyzerVersions.dates`; 231-case positive + 220-case negative matrices and 9 focused tests. |
| [29](TICKET-29-spoken-number-words.md) | P1 | CLOSED (2026-11-19) | Word numbers in 11 languages: conjunctions, additive/multiplicative runs, fractions, additive/fused halves, approximators and idiom negatives; `NumericFact.form`; 20 positive + 20 negative cases per language (assistant draft), mixed `3 milioni e mezzo` = 3.5e6, `build_language_packs.py --check` gate, `AnalyzerVersions.numbers` bump. 119 visualner + 225 semantic-core lib tests and all semantic-engine integration suites green. |
| [30](TICKET-30-typed-quantities.md) | P1 | PARTIAL (verified 2026-11-21) | Unit packs for 11 languages, grounded quantities, chart bridge and Go validation are implemented. The release draft scorer covers 1,784 cases: 1,643/1,643 positive quantity matches, 33/33 ratio/difference/change comparisons, and 33 incompatible comparisons rejected, with zero false positives. Full `semantic-engine`, Clippy and Go `rustexec` suites pass; a new regression verifies Top 10 rankings coexist with ten typed EUR quantities without charting ranks. Broader natural contexts and independent annotation review remain. Report authority is `assistant_draft_pending_human_review`. |
| [32](TICKET-32-places-and-routes.md) | P1 | PARTIAL (verified 2026-11-21) | `place_index.v1` contains 195 countries and 199 capitals with Wikidata labels across all 11 supported languages; missing capital labels fall back to English. The builder is offline/checksummed, declares 36 alias collisions and stays under 5 MiB; inflected PL/RU/TR aliases retain exact spans and coordinates. Explicit class cues, indexed context terms and same-sentence parent coherence disambiguate supported collisions such as Luxembourg; role cues, waypoint routes and consecutive-hop merging exist. Broad multilingual gold, wider geo-parent disambiguation and inflection coverage remain. Roadmap: [ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md](ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md). |
| [33](TICKET-33-person-roles.md) | P2 | PARTIAL (verified 2026-11-21) | 40-code role lexicons and grounded extraction now cover all 11 languages; byte-exact spans, same-sentence explicit-date binding, gendered test forms in DE/PL/RU and conservative Turkish gender handling pass. Numeric/spelled ages and directional family/work relations are projected and Go validated. Full inflection coverage and independent gold remain. Roadmap: [ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md](ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md). |
| [34](TICKET-34-events.md) | P2 | PARTIAL (verified 2026-11-21) | Trigger lexicons cover all 12 core event types in all 11 supported languages; runtime tests exercise each type in id/pl/ru/tr and localized negation/uncertainty. Grounded frame assembly and cross-scene merge now require a discourse thread spanning both scenes. Independent natural event gold, confusable-merge metrics and acceptance thresholds remain. Roadmap: [ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md](ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md). |
| [35](TICKET-35-entity-disambiguation.md) | P1 | PARTIAL (verified 2026-11-21) | Rust emits scored candidates with byte-exact feature spans; versioned weights are hashed into health/fingerprint and confusable tests cover 46 surfaces / 276+276 lookups. Context/counter evidence now matches whole words (`entity_linker.v4`/rule `v3`), with localized `type_cues` and a same-thread/same-type `discourse_prior`. Natural 10-language gold and independent margin calibration remain. Roadmap: [ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md](ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md). |
| [36](TICKET-36-coreference-pronouns.md) | P2 | PARTIAL (verified 2026-11-21) | `pronouns.v1` and `discourse_reference_terms.v1` cover 11 languages; discourse now uses `discourse.v7.role_nominal_basis`. Release draft gold scores 440/440 status and target matches with zero false resolutions; role nominals emit the dedicated `role_nominal` basis. Independent gold validation, broader agreement/number and salience/margin calibration remain open. Roadmap: [ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md](ROADMAP-SEMANTIC-EXPANSION-2026-11-19.md). |
| [37](TICKET-37-persistent-manifest-cache.md) | P1 | CLOSED (2026-11-19) | Opt-in disk cache keyed by SHA-256 over the request plus a cache identity (embedded lexicon content including `legal/index.json`, build hash, ICU, model presence): atomic publishes, fail-closed reads, mtime LRU bound. 23 unit + 5 fingerprint + 4 cross-process tests (restart, 8 concurrent writers, model-profile separation, broken dir); T1–T15 observed; memory-only default unchanged. |
| [38](TICKET-38-preflight-engine.md) | P2 | CLOSED (2026-11-19) | Pure pre-render checks for assets, subtitle timing and per-track overlays (including enclosed overlays), with stable codes and an advisory/blocking split. 14 unit + 4 real-ffprobe tests; no writes. |
| [39](TICKET-39-cross-repo-wire.md) | P2 | CLOSED (2026-11-19) | `preflight` is a `mediaexec.v2` operation (schemas + golden + consumer copy) with a typed Go caller and a Go-owned `Blocked` decision; `NewSemanticEngineExecutor` forwards `SEMANTIC_CACHE_DIR`/`SEMANTIC_CACHE_MAX_BYTES`, and a `cached:true` hit is logged. Verified cold 3.64 s vs warm 1.56 s on a repeated batch; `v0.3.8` carries the operation and `refactored` pins it. |
| [40](TICKET-40-entity-index-growth.md) | P1 | CLOSED (verified 2026-11-20) | 86 provenance-tracked rows cover 46 declared collisions; 276 positive and 276 counter-context lookups are frozen and checked in CI. |

## Verification — 2026-11-20 (status audit, open and blocked tickets)

One-by-one recheck against the working tree (`2bbeca1` plus the uncommitted
work described under “Local Rust progress”). The first pass of this audit
predates the 33a/34a/35a/36b work and still reported 33, 34 and 36 as OPEN; the
bullets below are the current state. No non-closed ticket is closable today.

- **30 — PARTIAL confirmed.** `lexicon/units.json`, `semantic-core/src/{units,quantity}.rs`,
  `semantic-engine/src/quantities.rs` and the `quantities` /
  `quantity_comparisons` manifest fields exist and are untouched in this
  working tree. The committed `semantic-engine/fixtures/quantities/report.json`
  (`brainrust.quantity_gold_report.v1`, regenerated on 2026-11-20 from a fresh
  release build and six sentence templates per language) reports **1,686
  cases, 1,469/1,469 positive matches, 0 negative and 0 positive false
  positives**; `annotation_authority` is
  `assistant_draft_pending_human_review`. Independent annotation review and
  broader natural contexts remain.
- **32 — PARTIAL (32a done; 32b/32c/32d initial slices).** The frozen index,
  scene place projection, exact-alias linking, localized role and class cues,
  same-sentence waypoint routes, and same-scene consecutive-hop merging for
  the same grounded person are in place. Cross-scene multi-hop,
  advanced disambiguation, Go parity and gold remain open.
- **33 — PARTIAL (33a complete seed coverage; runtime 33b/33c and initial 33d slices).** `lexicon/roles/*.json` carry
  40 role codes each in all 11 languages; `python3 lexicon/tools/check_roles.py` — **PASS**. The
  working tree also contains `PersonRole` / `RoleRank` / `RoleTense` /
  `RoleStatus`, `SceneManifest.person_roles`, `AnalyzerVersions.roles =
  person_roles.v4.all_11`, `semantic-engine/src/roles.rs` and its `extract()` wiring,
  with all 11 role lexicons in the fingerprint and Rust discourse gender inventory. Nine focused role tests pass, including same-sentence explicit-date binding, localized spans in de/fr/id/nl/pl/pt/ru/tr, and inflected case forms in de/pl/ru/tr;
  `person_relations` and numeric `age_facts` now have additive scene projections;
  dated-age linking is covered in all 11 supported languages;
  the family/work phrase table covers 11 languages, with three relation unit
  tests and age unit tests. Spelled-out ages pass an end-to-end manifest test
  in all 11 languages, and duration phrases remain excluded. The updated Go
  real-worker suite passes. Independent gold and full inflection/form coverage
  remain open.
- **34 — PARTIAL (34a–34c initial slices done).** Trigger lexicons cover all
  12 core types in all 11 languages; grounded frames reach
  `SemanticManifest.events`; asserted cross-scene repeats require a participant
  thread spanning both scenes plus matching date/place; overlapping entity
  facts are referenced through `source_fact_ids`. Gold and acceptance metrics
  remain.
- **35 — PARTIAL (35a done; initial 35b slice).** `LinkedEntity.link_candidates`,
  `link_margin` and `link_rule` with byte-exact feature spans are implemented;
  `lexicon/link_weights_v1.json` is versioned and its SHA-256 is exposed by
  health and included in the fingerprint. `lexicon/type_cues.json` now feeds
  `context_score` and emits `type_cue` evidence. `discourse_prior` now applies
  only to a resolved same-type thread and includes its exact support span;
  weights are `entity_linker.v3`, link explanations allow six features, and the
  Go validator accepts six while rejecting seven. Linker gold covers 46
  surfaces with 276 positive + 276 counter-context lookups. Missing: natural
  10-language gold (35c) and independent calibration (35d).
- **36 — PARTIAL (36a/36b done locally; initial 36c slices).** `pronouns.v1` inventories exist for all 11
  languages and `build_language_packs.py --check` — **PASS** without changing
  generated pack bytes; `resolution_basis` still exists in
  `discourse/resolution.rs`. Gender compatibility now uses structured pronoun
  hints and explicitly gendered role surfaces; a small curated given-name
  lexicon adds known gender for selected names, while absent names remain
  unknown and compatible. Gender and name lexicons are fingerprinted; role and
  name integration cases pass. A 440-case draft baseline is recorded per
  language; it shows 14 confidently wrong resolutions against unreviewed
  draft labels. Missing: independent gold review, broad name/agreement coverage
  (36c), salience/margin resolver (36d), role nominals (36e), traps and Go
  parity (36f). The detector terms now live in `discourse_reference_terms_v1.json`,
  are checked by `build_language_packs.py --check`, and bump `AnalyzerVersions.discourse`
  to `discourse.v7.role_nominal_basis`; role nominal resolutions now expose
  the dedicated `role_nominal` basis and invalidate prior cache entries.
  Recheck 2026-11-21 added Turkish `Selin` after a missing antecedent caused a
  false resolution: the release draft baseline is now 440/440 status and target
  matches with zero false resolutions. Gold review, agreement breadth and
  salience margin remain open.
- **08 — BLOCKED confirmed.** The long gold still has **1** `human_annotator`
  script and **19** `assistant_draft_pending_human_review`; no independent
  reviewer or held-out set arrived.
- **10 — BLOCKED confirmed.** No SigLIP/pgvector reference, schema or calibrated
  association set exists in the workspace; the only mentions are in audit and
  ticket documents.
- **26 — BLOCKED confirmed.** Only `provider_precision.py` and its synthetic
  test exist (**9 tests pass**, not the 8 previously recorded); there is no
  frozen provider run or independent judge file.
- **40 — CLOSED confirmed.** `entity_index_v1.json` has 86 rows (52.6 KiB),
  `entity_index_confusables_v1.json` 46 surfaces / 276+276 lookups, and CI runs
  `build_entity_index.py --check`.

## Local Rust progress — 2026-11-20

- **30a context expansion and 30c rerun initial gates complete.** Replaced the
  three recycled templates with six language-specific measurement contexts per
  language, rebuilt the release worker and regenerated the 1,686-case report.
  All 1,469 positives match exactly and all 220 negatives stay empty. The report
  remains an assistant draft; broader natural contexts and independent review
  remain open.

- **32a complete.** Frozen Wikidata CC0 snapshot builds 195 countries and 199
  capitals with coordinates, stable item IDs, country-parent links, and six
  declared alias collisions. The builder check and four Python tests pass.
- **32b initial runtime complete.** Added `Place`, `SceneManifest.places`,
  `AnalyzerVersions.places`, exact-alias linking and the place asset fingerprint.
  Linked unique aliases receive curated coordinates; ambiguous and unlinked
  mentions receive none.
- **32c initial cue-role slice complete; 32d waypoint and consecutive-hop slices.** `place_roles.json` supplies
  setting/origin/destination cues for all 11 supported languages; class cues
  also type unlinked places without coordinates. The route engine projects
  explicitly cued same-sentence waypoints; consecutive asserted hops merge only
  when the same grounded person and linked intermediate place recur. Nine place
  unit tests, nine route unit tests, and two manifest protocol tests pass.
  Cross-scene routes,
  Go parity and gold remain.
- **33a seed and 33b runtime slice complete; 33c initial temporal binding.**
  Forty English/Italian roles feed a grounded same-sentence extractor. An
  explicit resolved date in the same sentence is attached to the role; five
  focused tests pass. Broader role languages, inflected relation/age forms,
  Go parity and gold remain.
- **34a–34c initial slices complete.** Four event types in English/Italian
  produce grounded frames in `SemanticManifest.events`; asserted repeats merge
  only with the same grounded participant and an exact shared date or place.
  Frames reference overlapping entity facts. Eight event unit tests and two
  protocol tests pass. Broader merge semantics, other types/languages, gold and
  Go parity remain.
- **35a complete.** Localized type cues affect linker scores and emit exact-span
  evidence; discourse prior, natural 10-language gold and calibration remain.
- **BONIFICA-05 hotspot split complete in the working tree.** All 20 measured Rust hotspots are now below 1,000 lines, including `semantic-engine/candidates.rs` and `overlay-compiler/editorial_plan_v2/request.rs`; the latter retains the pre-existing animation/style pool fields. Its request tests pass 107/107. `cargo fmt`, workspace Clippy, `git diff --check`, and `cargo test --workspace --tests --locked` pass on the final tree.
- **GitHub issue #1 chart gate rechecked and expanded.** Added eleven-language annual-series and scaled-currency gates with year/category binding, exact source spans and EUR/USD rejection; Turkish cases now cover assertion denial, suffix-attached years, count units and euro/dollar mismatch. Chart item text retains rate periods such as `20 euro al mese`. The 36-case gold also asserts those unit/period surfaces. Added inflected Polish dollar/pound surfaces to the generated unit pack after the mixed-currency gate exposed a missing USD match. `chart_acceptance_36`, `chart_bare_cue`, `chart_multilingual`, `charts_numeric`, and the complete `semantic-engine` test suite pass. One historical query regression test still exists and passes; the other was superseded by the identity-only query contract plus separate grounded-context tests, so its old name/expectation is no longer present. Draft fixtures still need native-language review; the issue remains open for independent annotation and broader natural context/unit coverage. Workspace configuration links ICU 72; overriding it to 70 causes linker ABI errors.
- Recheck 2026-11-21: Turkish annual-series support handles years with apostrophe suffixes; `avro` is an EUR alias, and Turkish `yanlış`/`doğru değil` are explicit chart vetoes. All three `chart_multilingual` tests pass across eleven languages. The chart gates `chart_acceptance_36`, `chart_bare_cue`, `chart_multilingual`, and `charts_numeric` pass. TICKET-30’s latest release draft scorer covers **1,784 cases**, **1,643/1,643** positive quantities and 33/33 localized ratio/difference/change comparisons; 33 incompatible comparisons produce no false outputs. The coref and quantity reports share the current worker hash. `semantic-engine` passes **345 tests in 28 suites**, Clippy, formatter and `git diff --check` pass.
- **36a draft, 36b complete; initial 36c slices complete.** The 440-case
  coref gold and baseline remain an assistant draft pending human review.
  Recheck 2026-11-21 added Turkish `Selin` after the gold exposed a missing
  person antecedent: the release scorer now records 440/440 status and target
  matches, 0 false resolutions, and 100% draft precision/recall. Structured
  pronoun inventories cover all 11 languages; role nominal resolution now
  reports `role_nominal` under `discourse.v7`. Agreement morphology, salience
  margin and independent validation remain open.
- Verification on the current source: the full `cargo test -p semantic-engine --tests`
  suite passed after the route, event and given-name changes; place unit tests **9/9**, route unit tests
  **9/9**, place/route manifest protocol tests **2/2**, place-index Python tests
  **4/4**, content-fingerprint test PASS, and format/diff checks PASS. The
  waypoint and consecutive-hop slices pass route tests and
  `cargo clippy -p semantic-engine --all-targets -- -D warnings`. The release
  quantity scorer also passes all 11 languages (1,469/1,469 positives, zero
  false positives across 220 negative cases).

## Closure plans and splits — 2026-11-20

Every non-closed ticket now carries a “Piano di chiusura e split” section with
the files to touch, the measurable gate and the ordered sub-tickets.

| Ticket | Split | Can start now | Waits for |
| --- | --- | --- | --- |
| 30 | 30a quantity matrix and localized ratio/difference/change comparisons (**1,784 draft cases pass**), 30b independent review, 30c close | broader natural contexts | 30b external reviewer |
| 35 | 35a type cues (**done**), 35b discourse prior (**initial thread-aware slice done**), 35c 10-language gold, 35d calibration | 35c | 35d after 35c |
| 32 | 32a multilingual place index (**11 languages**), 32b exact/contextual linking, 32c localized cue roles, 32d waypoint and cross-scene same-thread routes (**initial slices done**), 32e gold | 32e natural gold and broader inflection | independent place/route reviewer |
| 33 | 33a role lexicon (**40 codes × 11 languages**), 33b appositions, 33c explicit-date binding (initial slice), 33d relations/age, 33e gold | broader inflection and natural gold | 33e independent precision/gold gates |
| 34 | 34a types/triggers (**11 languages done**), 34b single-sentence frames, 34c conservative merge/thread and fact references (**initial slice done**), 34d natural gold + metrics | 34d gold and confusable merge set | independent event reviewer |
| 36 | 36a technical gold/baseline (**draft pending human review**), 36b structured pronouns (**done**), 36c gender compatibility and given-name seed (**initial slices done**)/agreement features, 36d salience, 36e role nominals (**dedicated basis done**), 36f gold | remaining 36c, 36d and human-reviewed gold | 36d after 35 |

## Verification — 2026-11-21 (final status recheck)

Re-run on HEAD `b541f1d`. The release worker on disk (`3e9bb942…`) was built
from that tree; the one-line lint fix below changes no behavior and does not
alter the scored output.

- `cargo test -p semantic-engine --locked --tests` — **PASS**: **28 suites,
  345 passed, 0 failed, 4 ignored** (the ignored tests are manual probes).
- `cargo fmt --all -- --check` — rc 0. `cargo clippy -p semantic-engine
  --all-targets --locked -- -D warnings` — rc 0 after fixing one real
  `needless_borrow` at `semantic-core/src/entity_linker.rs:615`.
- Quantity draft scorer on `target/release/semantic-engine` (SHA-256
  `3e9bb9425e82f699761b8a8a124d16d00d5669060fbd825c2251d2f101e6bd79`):
  **1,784 cases**, **1,643/1,643** positive quantities, 33 positive
  ratio/difference/change comparisons, 33 incompatible comparisons rejected,
  **0** quantity or comparison false positives.
- Coreference draft scorer on the same worker: **440/440** status and target
  matches, **0** false resolutions, draft precision/recall 1.0.
- **Limitation:** this checkout has no `.tmp/udpipe-models`, so the run is
  model-free. Per the measurement precondition these are contract/regression
  results, not certified semantic-quality numbers.
- Both reports remain `assistant_draft_pending_human_review`; this recheck
  moves no ticket to CLOSED. The remaining gates per ticket are in
  “Closure plans and splits” below and in the 2026-11-21 table of
  [`docs/stato/brainrust.md`](../../../docs/stato/brainrust.md).
- `VELOX_RUST_SEMANTIC_ENGINE_PATH=../BrainRust/target/release/semantic-engine
  go test ./internal/platform/media/rustexec -count=1` — PASS on the previous
  release worker (**82.869 s**); unchanged by this documentation recheck.
- GitHub issue #1 remains open for the independent gold and annotation review
  required by its acceptance criteria.

## Verifica — 2026-11-21 (TICKET-32 label luoghi multilingue)

- Aggiornato lo snapshot Wikidata CC0 congelato: 195 paesi, 199 capitali e
  label EN/IT/DE/ES/FR/ID/NL/PL/PT/RU/TR; capitali senza label locale usano
  l'inglese. Generatore offline, checksum sorgente verificato, 36 collisioni
  dichiarate, output 321 KiB entro il limite 5 MiB. Rimosso il file override
  ridondante; `AnalyzerVersions.places=place_index.v3.multilingual_labels`.
- Builder Python **5/5**, `build_place_index.py --check`, pack check e
  fingerprint — PASS. `places::tests` — **12/12**; protocollo place — PASS;
  suite completa `semantic-engine` — PASS e Clippy con `-D warnings` — PASS.
- Release scorer aggiornati sul worker SHA-256
  `3e9bb9425e82f699761b8a8a124d16d00d5669060fbd825c2251d2f101e6bd79`:
  quantity **1,784 casi**, **1,643/1,643** match, zero falsi positivi;
  coreference draft **440/440** match e zero falsi positivi.
- Verifica Go `rustexec` con il worker release aggiornato — **PASS** (85 s),
  incluse le scene che prima venivano respinte per falsi age fact su `due`
  filoni e `cinque` consulenze.
  Due falsi positivi di età (`due filoni`, `cinque consulenze`) sono stati
  corretti nel riconoscimento Rust rimuovendo il generico `di N` come cue.
  I gold
  indipendenti e le route naturali richieste restano gate aperti di TICKET-32.

Critical path: `35c`, remaining `36c` and `30a` can run independently; `35b` is
unblocked too. Then `35b → contextual relinking`; `32d cross-scene → 32e`;
`33e → 34d`; `36c → 36d → 36e → 36f`. The initial `32b–32d`, `33b–33d`,
`34b–34c` and `36a` slices are in the working tree.
TICKET-30b (independent human review) is the only external gate among the
non-blocked tickets; 08, 10 and 26 remain owner-gated and cannot be split into
code that closes them.

## Verification — 2026-11-19 (TICKET-31, relative dates)

- `cargo test -p semantic-core --lib dates::relative` — **9 passed**, including
  the 231-case positive matrix and the 220-case negative matrix (20 per
  language), leap/month-end arithmetic and explicit/reference/narrative anchors.
- `cargo test -p semantic-core` — **233 lib tests** + integration green.
- `cargo test -p visualner` — **121 lib tests** + integration green.
- `cargo test -p semantic-engine --tests` — all suites green.
- `cargo clippy -p semantic-core -p visualner --all-targets -- -D warnings` —
  rc 0.
- `python3 lexicon/tools/build_language_packs.py --check` — 11 packs; the gate
  now also rejects a missing `relative_time.json` language.

## Verification — 2026-11-19 (TICKET-29, word numbers)

- `cargo test -p visualner` — **121 lib tests passed** plus all integration
  suites green (multilingual regression included).
- `cargo test -p semantic-core --lib` — **225 passed**.
- `cargo test -p semantic-engine --tests` — green (lib 78 + integration
  suites); `cargo test -p semantic-engine` still fails only at rustdoc
  doctests with `E0463`, a toolchain limitation.
- `cargo clippy -p semantic-core -p visualner --all-targets -- -D warnings` —
  rc 0.
- `python3 lexicon/tools/build_language_packs.py --check` — 11 packs up to
  date; the gate rejects a pack language missing from `number_words.json`.
- Word-number gold: 20 positive + 20 negative cases per language, 11
  languages; `3 milioni e mezzo` is one `mixed` fact of 3.5e6.

## Verification — 2026-11-19 (TICKET-39, cross-repo wire)

Run on the `BrainRust` and `refactored` working trees.

- `cargo fmt -p pipelinegen-muscles -- --check` and
  `cargo clippy -p pipelinegen-muscles --all-targets --offline -- -D warnings`
  — rc 0.
- `cargo test -p pipelinegen-muscles --offline` — green, including 16 preflight
  unit tests and 4 real-media integration tests.
- `python3 contract/mediaexec.v2/validate.py` — **4 shared goldens validated**;
  `check_consumer_copy.sh ../refactored/testdata/mediaexec/v2` — **no drift**.
- Real worker: `preflight` returns `ok:true` with `asset_missing` (error) and
  `cue_reading_too_fast` (warning) issues; `health` keeps the original envelope
  with no `preflight` key.
- `go test ./internal/platform/rustworker ./internal/platform/media/rustexec
  -count=1` — both ok (`rustexec` 89.4 s), including protocol, golden, Go-owned
  decision, cache-env and cached-log tests.
- Persistent cache on the installed `semantic-engine`, same batch twice: cold
  3.64 s `cached:false`, warm 1.56 s `cached:true`, one manifest entry; the Go
  e2e `TestSemanticEngineRealBinaryServesRepeatedBatchesFromTheCache` passes
  through `NewSemanticEngineExecutor` in 3.17 s.

## Verification — 2026-11-17 (tickets 23–25)

Run on the `v0.3.7` release-candidate tree (`a4bbdbf` plus the final facility
dedup and ticket closeout), with
`VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models`.

- `cargo fmt --all -- --check` — rc 0.
- `cargo clippy -p semantic-core -p semantic-engine --all-targets --locked -- -D warnings` — rc 0.
- `cargo test -p semantic-core -p semantic-engine --lib --locked` — rc 0
  (61 semantic-core lib tests passed).
- `cargo test -p semantic-core --test place_inflections` — 4 passed.
- `cargo test -p semantic-engine --test process_sequences --test chart_bare_cue
  --test slavic_places` — 13 + 4 + 2 passed.
- `score_long_gold.py` — entities **88/124**, objects **93/105**, contextual
  queries **20/20**, positive charts **20/20**, sequences **80/80** in **20/20**
  groups, grounding errors 0, replay identical; chart negative set **20 cases /
  0 charts**.
- Untouched HEAD `d3b77d6` baseline in a temporary worktree — entities 86/124,
  objects **93/105**, queries 16/20, charts 17/20. The 2026-10-28 object
  baseline (99/105) is stale at HEAD; tickets 23–25 leave objects unchanged and
  raise entities, queries, charts and sequences.
- `validate_long_gold.py --require-release-size` — rc 0.
- `git diff --check` in the root and in `BrainRust` — rc 0.

## Verification — 2026-11-17 (ticket 22, quality gate Rust port)

Run on the `BrainRust` working tree; the Go gate ran on `refactored`. No Go
production path was changed.

- `cargo fmt --all -- --check` — rc 0.
- `cargo clippy -p script-qa -p semantic-engine --all-targets -- -D warnings` — rc 0.
- `cargo test -p script-qa` — 28 passed, 0 failed (parity corpus).
- `cargo test -p semantic-engine --test quality_gate_protocol` — 3 passed.
- `cargo test -p script-qa -p semantic-engine` — 29 suites, 240 passed, 0 failed.
- `go test ./internal/capabilities/scripts/usecase/qualitygate -count=1` — ok, 0.022 s.
- `git diff --check` in the root and in `BrainRust` — rc 0.
- Commit state: `d3b77d6` on `HEAD`; the tag `v0.3.6` predates it. The Go
  `qualitygate` package is untouched.

## Verification — 2026-11-16 (workspace gate, Rust ↔ Go boundary)

Run on the `BrainRust` + `refactored` working trees. ICU comes from
`.cargo/config.toml` (72): the `RUST_ICU_MAJOR_VERSION_NUMBER=70` line in older
logs is stale and now fails to link, because this host's unversioned
`libicuuc.so` points at ICU 70 while `.cargo/icu72-link` redirects to `.so.72`.

- `cargo fmt --all -- --check` — rc 0 after formatting the new
  `semantic-core/src/phrase_select.rs`.
- `cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib` —
  64 + 161 + 54 passed.
- `cargo test -p overlay-compiler --test overlay_plan_contract` — 30 passed.
- `cargo test -p semantic-engine --test editorial_plan_protocol` — 6 passed.
- `cargo test -p semantic-engine --test phrase_selection_protocol` — 13 passed
  (12 in process plus the real-worker one-request path).
- `cargo clippy --workspace --all-targets -- -D warnings` — rc 0 after removing
  the `chars_last_cmp` lint in `phrase_select.rs`.
- `go test ./internal/capabilities/scripts/... ./internal/platform/media/rustexec/... -count=1`
  — every package passes (`rustexec` 81.982 s).
- `go test ./internal/capabilities/images/entitycatalog/... ./internal/capabilities/images/... -count=1`
  — pass (ticket 02 Go policy).
- `make verify-agent` — PASS; every changed component ran (not cached) and the
  `rust` (91.5 s) and `script` (32.9 s) legs were green.
- `make verify-main` — PASS (foundation, static, unit, changed components,
  archcheck/C2 and `CLIP_RENDER_CUTOVER=PASS`).
- `bash contract/mediaexec.v2/check_consumer_copy.sh refactored/testdata/mediaexec/v2`
  — PASS, consumer goldens match.
- `python3 lexicon/tools/check_go_stopword_parity.py` — PASS, 0 drift on the
  eleven shared languages (gate repointed to the Go embedded projection on
  2026-11-18).

## Verification — 2026-11-17 (gates rerun, entity/moment follow-up closed)

Run on `BrainRust@527b756`; the Go gates ran on `refactored@eb65a6a30`, before
the pin commit `4bde5e66e`.

- `cargo test -p semantic-core --lib` — **184 passed**.
- `cargo test -p semantic-engine align_phrases --lib` — **3 passed**.
- `cargo test -p semantic-engine --test phrase_selection_protocol --test language_profile_protocol --test localized_identity_protocol` — **13 + 4 + 5 passed**; the real worker answers one `select_phrases` request.
- `cargo test -p mediasampler --test rank_parity` — **4 passed** (Go order,
  scores and winner reproduced).
- Production caller audit: `LocatePhrase`, `LocatePhraseTimings` and
  `locatePhraseTimingWithEndpointFallback` have **0** non-test callers in
  `internal/` and `cmd/`; `audio/phrase_locator.go` is deleted.
- `go build ./...` and `go vet ./...` — rc 0.
- `go test ./internal/app/knobs/... ./internal/capabilities/overlays -count=1`
  — **PASS** (0.024 s + 11.931 s): the catalog drift and knob failures are
  fixed, not sidestepped.
- `make verify-architecture` — **PASS** (C2-C 0/0, C2-E 0/0).
- Release prerequisite closed: `v0.3.6` was published on 2026-11-17 at tag
  `527b756` (run `44032711209`, eight ICU 72 assets). The release binaries carry
  all five operations and their SHA-256 match `BRAINRUST_RELEASE.json`;
  `refactored@4bde5e66e` pins `BRAINRUST_VERSION ?= v0.3.6` and `bin/` holds the
  release binaries (`git_sha=527b756`).
- The downloaded release binaries were also exercised over NDJSON: the `v0.3.6`
  `semantic-engine` returns `ok:true` for `lexicon_profile` (it) and
  `select_phrases`, and the `v0.3.6` `mediasampler` routes `rank_candidates` and
  applies its candidate gate (`malformed candidate envelope` for a minimal
  candidate). No operation returns `unknown operation`.

## Verification — 2026-11-16 (second pass, phrase selector, alignment and ranker)

Run on the current working trees (`refactored@c8cae4e8a` plus uncommitted
changes, `BrainRust@01f01e1` plus the uncommitted phrase/alignment work).

- `go build ./internal/capabilities/scripts/... ./internal/app/wiring/...` —
  rc 0.
- production imports of `capabilities/scripts/phrases` — **0**;
  `phrase_supplement.go` is deleted.
- `go test ./internal/capabilities/scripts -count=1` — **PASS**, 15.723 s. The
  two tests that failed in the previous pass now pass without weakened
  assertions.
- `go test ./internal/capabilities/audio ./internal/platform/media/rustexec -count=1`
  — **PASS**, 0.026 s + 99.322 s.
- `go test ./internal/capabilities/mediamemory/... ./internal/platform/media/rustexec/... -count=1`
  — **PASS**, 0.043 s + 85.019 s.
- `cargo test -p mediasampler` — **42 lib + 7 bin + 4 parity** pass.
- `cargo test -p semantic-core phrase_align --lib` — **8/8** pass;
  `cargo test -p semantic-engine align_phrases --lib` — **1/1** pass;
  `cargo test -p semantic-engine --test phrase_selection_protocol` — **13/13**
  pass.
- Production caller audit: `PickTopFromRose`, `PopulateRepetitionPenalty`,
  `buildFilterFlags`, `buildRankingInput` and `sortByFinalScoreDesc` have 0
  production callers; the surviving overlay projection helpers that reach the
  endpoint fallback (`overlaySceneInput`, `projectEditorialOverlays`) also have
  0 production callers (`projectChapterBullets` was deleted on 2026-11-16).
- `select_phrases`, `align_phrases` and `rank_candidates` were absent from
  `v0.3.5`, which `refactored/make/build.mk` pinned at the time. Closed
  2026-11-17: release `v0.3.6` carries all five operations and the pin is
  `BRAINRUST_VERSION ?= v0.3.6` (`refactored@4bde5e66e`).

No test failure remains in the 15/16/19 scopes. 18 and 20 landed on
2026-11-16 and are DONE; their previously unrelated gate failures were fixed and
rerun on 2026-11-17.

## Verification — 2026-11-15 (ticket 14, Rust single-call compiler)

- `cargo fmt --all -- --check` — rc 0.
- `cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib`
  — 39 + 151 + 53 passed.
- `cargo test -p overlay-compiler --test overlay_plan_contract` — 28 passed:
  frozen-golden byte comparison, determinism, verified/text-only entity media,
  entity-card motion rotation and caption/style stamping, missing-pool and
  uncertified-selector rejection, render-key clearing, map trim and final
  resolution, chart and chapter-bullet admission, localized surfaces, malformed
  and out-of-window rejection.
- `cargo test -p semantic-engine --test editorial_plan_protocol` — 6 passed:
  v2 routed to the full compiler, unknown version rejected before
  deserialization, malformed v2 fails closed, legacy batch unchanged.
- `cargo clippy --workspace --all-targets -- -D warnings` — rc 0.
- `gofmt -l` on the touched Go packages — empty; `go vet` — rc 0;
  `TestBuildOverlayPlanV2Request` and
  `TestCompileEditorialPlan|TestOverlayPortsUseTheSingleBatchOperation` — pass.
- Fixtures are produced by `overlay-compiler/examples/compile_fixture`, so the
  plan goldens come from the real planner and not from a hand-written document.

## Verification — 2026-11-15 (ticket 12, Rust core)

- `cargo test -p semantic-core --lib` — 150 passed.
- `cargo test -p semantic-engine --test mvp_semantic_contract` — 7 passed
  (T1–T7).
- `cargo test -p semantic-engine --test protocol_validation` — 4 passed.
- `cargo test -p semantic-core -p semantic-engine -p visualner -p imagecatalog`
  — all passed (recorded before the 2026-11-17 exclusion: `imagecatalog` is now
  standalone, `cargo test --manifest-path imagecatalog/Cargo.toml`).
- `cargo fmt --all -- --check` and clippy on the touched crates — clean.
- Release probe (20 scenes): with UDPipe p50/p95/p99 = 1.438/1.503/1.551 s;
  model-free 77.9/88.7/98.6 ms.
- No-network: the release binary analyzed the ESA/NASA fixture inside
  `unshare -n` and returned `ok:true`.
- RSS: 1,000 analyses plateaued at 228,264 KiB (0 KiB growth across the last
  three samples).

## Verification — 2026-11-15 (ticket 13c, Go audio timing)

- `go vet ./internal/capabilities/audio ./internal/capabilities/scripts/...`
  — exit 0.
- `go test ./internal/capabilities/audio ./internal/capabilities/scripts/...
  -count=1` — all packages pass.
- The 40 ms AAC padding tolerance now has one Go definition; the literals in
  the final-audio validator, master invariants and final-job payload/plan are
  gone. The Go MP3 frame-duration parser was deleted: synthesized audio
  duration comes only from the shared media executor probe.

## Verification — 2026-11-15 (ticket 13 cleanup)

- `make verify-main` — PASS (foundation, static, full Go tests, changed
  components `final=PASS`, archcheck/C2, `CLIP_RENDER_CUTOVER=PASS`).
- `make verify-agent` — PASS (`final: PASS`); every component PASS or
  cache-hit, including `script`, `rust` and `timeline`.
- `go test ./internal/capabilities/scripts/... -count=1` — all packages pass.
- `go test ./internal/platform/media/rustexec/... -count=1` — pass (74s);
  `TestSemanticEngineRealBinaryProjectsLinkedEntities` — pass (2.94s).
- `gofmt -l .` and `git diff --check` — clean.
- `cargo fmt --all -- --check` — clean.
- `cargo test -p overlay-compiler -p semantic-core -p semantic-engine --lib`
  — pass (8 + 151 + 53 tests).

## Verification — 2026-11-15 (13d v2 contract)

- `cargo test -p overlay-compiler --lib` — 38 passed, including the 8
  chapter-bullet parity cases, the editorial projection, the first
  single-call planner slice and the v2 request/response validators.
- `cargo test -p overlay-compiler --test overlay_plan_contract` — 23 passed
  (frozen goldens, fixture matrix, determinism, fail-closed schemas and
  map-trim-before-budget ordering).
- `cargo test -p semantic-engine --lib` — 53 passed; the worker dispatches
  `editorial_plan.v2` to `compile_overlay_plan_v2` and never parses it as the
  sparse legacy batch.
- `cargo fmt --all -- --check` and
  `cargo clippy -p overlay-compiler -p semantic-engine --all-targets -- -D warnings`
  — clean with ICU 72.
- `go test ./internal/platform/media/rustexec -count=1` — pass (83.4 s);
  `TestCompileEditorialPlanV2ReturnsTheChapterProjection` and the
  out-of-window rejection pass, and the adapter validates the projection
  without repairing it.
- `go test ./internal/capabilities/scripts -run 'TestBuildOverlayPlanV2Request|TestProjectChapterBullets|TestChapterBullet|TestProjectEditorialOverlays' -count=1`
  — pass; the producer now carries certified phrase annotations.
- `go vet ./internal/platform/media/rustexec ./internal/capabilities/scripts`
  — clean.
- `python3 BrainRust/lexicon/tools/check_go_stopword_parity.py` — PASS, 0
  drift on the eleven shared languages (gate repointed to the Go embedded
  projection on 2026-11-18).

The full `go test ./internal/capabilities/scripts -count=1` suite still fails
on `TestBuildFinalJobPayloadsUsesLocallyRenderedStockVisual`
(`unknown stock fixture "obama-stock-db-clip"`), a concurrent final-job
fixture issue unrelated to the v2 compiler work.

## Verification — 2026-11-15 (TICKET-14 parity harness)

- `go test ./internal/capabilities/scripts -run TestOverlayPlanV2ParityFixtureShape -count=1`
  — PASS; the `minimal_phrase_only` request/golden pair is readable and
  internally consistent.
- `OVERLAY_PLAN_PARITY=1 go test ./internal/capabilities/scripts -run TestOverlayPlanV2ParityMinimalPhraseOnly -count=1 -v`
  — runs the differential and currently reports Go 2 items vs Rust 3, plus
  item/role/window differences. This is the cutover gate, not a green parity
  claim.

## Verification — 2026-11-15 (ticket 12, Go consumer)

- Cache-key tests: intent, query class, provider, verifier and entity-index
  changes all invalidate the key; unlinked document-scoped identities do not
  collide.
- Scheduler tests: `bestVerifiedSemanticHit` rejects provider-order
  `Hits[0]`; a hit settles the composition and later rungs do not run.
- v3 projection tests: role, gloss, negative constraints and typed intents
  survive the Go boundary; an unlinked entity carrying a canonical id fails
  closed.
- Cross-repo: `TestSemanticEngineRealBinaryProjectsLinkedEntities` passes
  against the release BrainRust binary in 3.28 s with UDPipe models.
- The broad Go suite is green as of the ticket-13 cleanup above; the earlier
  `overlay_plan_*` signature churn has landed.

## Verification — 2026-11-11 (tickets 01–11)

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models RUST_ICU_MAJOR_VERSION_NUMBER=70
  cargo test --workspace -j 2` — **646 passed, 0 failed, 1 ignored** across 41
  test binaries. Log: `BrainRust/.tmp/discourse-final-tests.log`.
- `cargo build --release -p semantic-engine` — clean.
- `cargo test -p pipelinegen-muscles --test canonical_render_segments` — 4/4.
- `go test ./internal/capabilities/entities
  ./internal/capabilities/images/entitycatalog
  ./internal/capabilities/scripts/adapters -count=1` — pass in `refactored`.

The 646 count includes the prior audit suites plus the 14 discourse integration
tests and the language-scoped reference unit tests. `discourse.v1` is additive:
older manifests decode with an empty projection.

## External residuals

These do not reopen the tickets; they are owner gates outside this checkout:

- **06** — the 20-script run is a host observation (peak 1,418,300 KiB, P50
  0.403 s, P95 3.600 s), not a deployment SLO. The 128 MiB value remains a
  policy bound.
- **07** — no consumer to wire: the Rust `imagecatalog` crate is offline by the
  13b decision and the Go `entitycatalog` is the live registry, so the manifest
  saving stays latent. Adopting the crate would replace `entitycatalog`.
- **22 / release** — TICKET-27 is closed: `v0.3.8` passed the release gates
  and is published with matching manifest/checksums; `refactored/make/build.mk`
  pins the installed release.
- **08/10/26** — human gold annotations, the multimodal model/schema/calibrated
  association set, and the provider precision@10/cascade/independent judge are
  still required.
- **12** — the revised measured SLO needs owner agreement. The Rust contract,
  local index/linker, typed intents, Go v3 consumer, intent/index cache key,
  verified-hit admission, stop-on-satisfaction, cross-repo smoke, no-network
  and RSS gates are complete in this checkout.

No extra platform or persistence layer was introduced. PipelineGen remains the
owner of canonical state and business policy.

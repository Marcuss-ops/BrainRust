# BrainRust — 100 multilingual synthetic 2K-word scripts

A deterministic, CPU-only **stress benchmark**: **100 actual UTF-8 text files**, 10 languages × 10 scenarios. Word counts range **1976–2074**, total **203254** (whitespace-token count).

Languages: Italian, English, Spanish, Portuguese, French, German, Dutch, Polish, Russian and Indonesian. Topics for every language: transport, stadium, hospital, energy, school, museum, flood protection, agriculture, budget and digital infrastructure.

These are **synthetic, template-based fictional narratives**. They are NOT 100 independent journalistic scripts, an external fact-check, nor a hand-annotated gold dataset. Repeating subjects and vocabulary are intentional tests for entity deduplication and co-mentions. Real city names appear as geographic anchors; people, institutions and events are illustrative.

Each script contains:
- 2 recurrent persons, an organisation, a city and at least one visual object.
- Five distinct years tied to a textual chronology (source anchors in `cases.json`).
- A direct numerical comparison of two percentages, to probe chart generation.
- A quoted statement with attribution and a negated assertion (hypothesis ≠ evidence).
- A numbered list: **10 items** for even domain indices (Top 10); **6 items** for odd ones (Checklist). A six-item list must not magically become a Top 10.
- Uppercase headings and ambiguous context terms to test heading leakage, wrong PERSON/LOCATION types and noisy visual keywords.

## Run

From the BrainRust root (after provisioning system ICU and UDPipe models):

```sh
cargo build --release -p semantic-engine
python3 semantic-engine/fixtures/multilingual_100/benchmark.py --worker target/release/semantic-engine

# Quick smoke in one language
python3 semantic-engine/fixtures/multilingual_100/benchmark.py --worker target/release/semantic-engine --language it --limit 2

# Compare persistent-worker requests with restarting Rust once per scene
python3 semantic-engine/fixtures/multilingual_100/performance.py --worker target/release/semantic-engine --language it --limit 5

# Recreate all fixture files exactly, if needed (Node used ONLY by the offline fixture generator)
node semantic-engine/fixtures/multilingual_100/generate.mjs
```

The runner uses a **persistent NDJSON worker** so ICU/UDPipe model initialization isn't repeated for every document. It verifies source counts, manifest schema, UTF-8 spans, literal grounded queries, contextual query components/evidence, named people, organizations, places, visual objects, normalized dates and the exact pair of expected chart percentages. Contextual queries are a separate manifest collection and are emitted only for an asserted `located_in` fact; each is composed from the exact subject and location spans plus the sentence that supports the relation. For objects it reports both exact composed-phrase coverage and typed-head coverage, so a detected "sensor" does not hide that the full "sensor de agua" phrase was not preserved. It also reports chart/micro-summary/group coverage per language and compares a cache replay for every request with its original manifest.

On Linux, the JSON report includes the worker's peak RSS/virtual memory and cumulative CPU time, plus corpus throughput and P50/P95 request latency. These measurements describe this host and run; compare them on the deployment hardware before setting service-level targets.

`performance.py` uses distinct source scenes from one corpus script, compares a
single persistent worker with one cold worker process per scene, and requires
identical manifests. It writes per-request latency and aggregate speedup to
`/tmp/brainrust_worker_performance.json` by default. Measurements are local to
the machine and should be repeated on deployment hardware; this helper does
not impose a latency threshold.

Exit codes: **0** structural and candidate checks pass, **1** contract/runtime errors, **2** missing expected candidates / quality review. Result JSON defaults to `/tmp/brainrust_multilingual_100_report.json`, not a committed file.

**Limits:** This tests extraction and source grounding, not provider image quality, image feedback, semantic coverage in PipelineGen, or Chronon video rendering. Independent human-annotated tests remain essential for precision/recall and real editorial usefulness. Do not present a passing score here as semantic accuracy certification.

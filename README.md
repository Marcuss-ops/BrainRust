# BrainRust — PipelineGen Rust components

This repository is the isolated Rust boundary for PipelineGen's native
execution components (the “muscles”). It is built and tested on its own; the
PipelineGen checkout consumes it as the `rust/` submodule pinned to a
released commit, so every `rust/...` build path there stays valid. The Go
application remains the composition root and owns canonical state,
application decisions, jobs, and the transactional outbox.

## Boundary

Rust components must communicate through explicit typed contracts. They must
not directly open PipelineGen SQLite databases, publish to Google Drive, write
Qdrant state, or load credentials. Those responsibilities remain behind the
existing Go application and infrastructure ports until a separate migration
contract is approved.

The workspace now has two deliberately separate execution boundaries:

- `semantic-engine` is the dedicated CPU-only text-analysis worker. One
  `analyze_script` request carries all scenes and returns the versioned
  `semantic_manifest.v2`; it does not search for assets or perform media I/O.
- `pipelinegen-muscles` remains the media executor. It owns FFmpeg-related
  operations and must not become the semantic engine's dependency. The
  standalone Phrase Impact and Scene Intelligence compatibility workers were
  retired on 2026-10-25 after measured parity; their shared algorithms live in
  `semantic-core` and in the unified semantic worker batch.

`semantic-core` owns shared semantic contracts and deterministic primitives
(grounding, sentence spans, numeric/date facts, identity and relation types).
`visualner` remains the extraction library, while `mediasampler` and
`ndjson-stdio` remain separate reusable crates. Go sends the complete scene
batch to `bin/semantic-engine`; it verifies the returned source spans and
keeps asset selection, timing and planning in PipelineGen. Chronon owns
rendering.

The manifest preserves candidates and evidence. It does not assert that a
textual claim is externally true, and empty or unsupported categories stay
empty. Editorial top-N and asset availability remain planner decisions.

The current V2 engine emits chronology groups only when multiple normalized
dates occur in sentences with explicit event predicates. It recognizes
consecutive numbered items (one per line or inline in a paragraph) as
enumerations or rankings; a declared Top 10, or ten consecutive explicit
ranks, carries its actual member count and completeness flag. It does not fill
missing ranks. Process groups (including completion phases), explicit
comparison groups, asserted cause-effect edges and their cross-scene variants
are emitted with grounded per-scene evidence. Facility facts link inauguration,
capacity, ownership and location to the scene's canonical facility, including
sentences that refer to it anaphorically when the scene names only one
facility. A separate conservative pass (`semantic-engine::legal`) links
acquittal, conviction, confirmation, annulment-with-remand, finality,
investigation status, procedural acts, sentence length, age and partner facts
to grounded people, courts and roles. It only uses a date stated in the same
sentence, keeps negation/uncertainty in `status`, and splits the canonical
outcome (`normalized_value`) from the ISO ordering date
(`normalized_date`); `semantic-engine::micro` projects asserted facts into
fact-backed micro phrases and chronological or comparison groups without
rewriting `not convicted` as innocence. Every retained visual entity gets a
source-grounded search query.
Bullet candidates and their editorial salience come from one shared
`semantic-core::bullets` implementation; the editorial ranking combines that
score with embedding importance and owns duplicate/novelty selection. The
extractive summary, bullets, heavy sentences, chapters and scene highlights
are computed inside the same unified worker batch. The legacy standalone
Phrase Impact worker and the opt-in Scene Intelligence shadow path were
retired on 2026-10-25 after parity evidence was recorded (see
[`../docs/stato/brainrust.md`](../docs/stato/brainrust.md)).

## Local check

VisualNER uses the system ICU4C RBNF/CLDR spellout rules. The canonical release
ABI is **ICU 72** (Debian Bookworm), declared once in `ICU_MAJOR`; CI and
`scripts/release/build-artifacts.sh` refuse to build when `pkg-config` reports
a different major. Install the ICU **development** package (`libicu-dev`,
including `pkg-config`) on build hosts; runtime hosts that execute
`bin/visualner` need `libicu72`. Do not copy that binary to a host with a
different major.

The Rust toolchain is pinned in `rust-toolchain.toml`, so plain `cargo` picks
it up automatically. On a host with a different ICU (for example ICU 70),
override the `.cargo/config.toml` default explicitly.

The tests also need the UDPipe 2.5 models declared by
`lexicon/language_profiles.json`. Provision them once (the installer is
idempotent and reads the profile table, so a new language adds its model
automatically):

```sh
./visualner/tools/install_udpipe_models.sh
```

The suite fails closed when a declared model is missing; it never silently
degrades to the model-free extractor.

```sh
cd <this repository>
ICU_MAJOR=$(pkg-config --modversion icu-i18n | cut -d. -f1)
RUST_ICU_MAJOR_VERSION_NUMBER="$ICU_MAJOR" cargo test --workspace

# Prints distinct language identifiers with installed ICU spellout rules.
RUST_ICU_MAJOR_VERSION_NUMBER="$ICU_MAJOR" cargo run -p visualner -- --locale-count

# Build the executables consumed by the Go adapter (see rustexec in PipelineGen).
cargo build --release
```

## Test layout

Tests follow two conventions, both split by topic — never by size:

- **Unit tests that need crate-private items stay inside `src/`, in one
  folder per topic.** `semantic-core` keeps the phrase-impact suite in
  `src/phrase_impact_tests/` (bullets, chapters, splitting, scenes_titles,
  summary, rank, corpus_metrics plus `common.rs` for shared fixtures) and
  `visualner` keeps the extractor suite in `src/extractor_tests/`
  (tokenizer_pos, persons_names, places_locations, orgs_concepts,
  objects_values, scoring_guards plus `common.rs`). In both, the folder is
  wired with `#[cfg(test)] #[path = "<folder>/mod.rs"]`; each topic file sees
  production items through `super::super::*` and shared helpers through
  `super::common::*`. `pipelinegen-muscles` follows the same rule with its
  two small `render_clip_tests*` modules. These paths exist because their
  subjects expose `pub(crate)` items that integration binaries cannot see
  (for example `visualner`'s test-only `tokenize`); do not move them to
  `tests/` without making private items public.
- **Integration tests against the public API live in `tests/`, one file per
  topic.** `semantic-engine` splits its manifest-level suite into topic
  binaries (`entities_identity`, `search_queries`, `charts_numeric`,
  `relations_groups`, `process_sequences`, `facilities_legal_micro`,
  `editorial_segments`, `story_coverage`, `protocol_validation`) plus
  `common/` for the shared request/manifest helpers — not every topic
  binary uses every helper, so helpers may be `#[allow(dead_code)]` there.

When adding tests: join the topic file that matches the behavior, or open a
new topic file in the same style. Do not grow a file into a new catch-all
module, and do not ressurect a single large file.

## Languages

Linguistic knowledge is data, not Rust: one directory per language under
`lexicon/languages/`, generated into `lexicon/packs/<lang>.json` by
`lexicon/tools/build_language_packs.py`. The filter order is POS tag (when a
UDPipe model is installed) → the language pack (closed class + curated
blocklist). There is no cross-language fallback table: a language without a
pack has no word-class filter (declared degradation), and adding one is a data
directory. Legal facts and micro-summary templates live in optional domain
packs (`lexicon/legal/<lang>.json`); a language without one degrades instead
of rendering another language's vocabulary.

Adding a language never touches Rust code. See
[`docs/operations/languages.md`](docs/operations/languages.md) for the tiers
(model + pack, pack only, CLDR only), the generator commands and the CI gate
(`--check` fails when a committed pack is stale).

## Contract

`contract/mediaexec.v1/` owns the request/response JSON Schemas and the shared
goldens consumed by PipelineGen. Validate them with
`python3 contract/mediaexec.v1/validate.py`; `check_consumer_copy.sh <dir>` is
the parity gate for the Go consumer copy. See
`contract/mediaexec.v1/README.md`.

## Release

Pushing a `v*` tag runs `.github/workflows/release.yml`, which tests the tagged
tree on Bookworm, builds the four binaries and publishes `sha256sums.txt`,
`TOOLCHAIN` and `ICU_MAJOR` alongside them. PipelineGen consumes that bundle
with `make rust-fetch RUST_VERSION=vX.Y.Z`; the source build remains available
there as the explicit `make build-muscles` fallback.

VisualNER requests must include a BCP-47 `language`. ICU does not provide
spellout rules for every locale and some rule sets are incomplete; missing
locale data fails closed. `--locale-count` reports the installed ICU count,
not a guarantee of equivalent parsing quality in every language.

### Decision: ICU is the single spellout backend

VisualNER's number-word recognition (RBNF/CLDR spellout) is served **only** by
the system ICU4C libraries through `rust_icu_sys` / `rust_icu_uloc` /
`rust_icu_unum`. A pure-Rust rule engine was evaluated and rejected, so the
dependency stays mandatory rather than sitting behind an optional feature:

- The reference RBNF rule sets live in ICU data, not in a Rust crate: ICU4X
  (`icu` 2.x / `icu_segmenter`) does not ship RBNF spellout parsing. A
  pure-Rust path would have to re-implement every locale's rules and would
  silently diverge from the certified behaviour for the supported languages.
- `visualner/src/spellout.rs` depends on ICU-specific semantics
  (`UNUM_PARSE_ALL_INPUT`, resolved-data-locale matching, spellout/ordinal
  styles) that have no equivalent outside ICU4C.
- An optional `spellout` feature would turn a certified capability into a
  build-time switch that silently degrades entity extraction for numbers, so
  the dependency is not feature-gated. `RUST_ICU_MAJOR_VERSION_NUMBER` must
  match the host ICU major or the build fails closed; hosts without
  `libicu-dev` cannot build `visualner` at all instead of producing a binary
  with missing number-word rules.

Revisit only with a concrete pure-Rust RBNF implementation plus a parity
corpus over the supported languages.

## Phrase impact (extractive summary, heavy sentences, bullets)

`pipelinegen-muscles::phrase_impact` is the extractive editorial summary
engine. It never loads models, invokes inference, or touches the media
executor JSON protocol: the caller supplies either one-vector-per-segment
embeddings (the canonical E5 passage vectors) or asks for the deterministic
`lexical_only` fallback, and may pass per-sentence microsecond timings. The
summary and bullets are extractive source sentences, selected separately from
the heavy-sentence ranking, so nothing it returns is a generative,
unsupported paraphrase.

The core is invoked in-process by the unified `semantic-engine`
`analyze_script` batch, which carries the editorial products (`summary`,
`bullet_points`, `heavy_sentences`, `chapter_manifest`, scene highlights) in
the same manifest the Go runner already consumes. The standalone
`bin/phrase_impact` worker, its Go adapter and its compatibility benchmarks
were retired on 2026-10-25: the same core had been proven byte-identical to
the unified path by the real-binary parity tests before removal.

The permanently stored label fixture is explicitly synthetic and cannot be
used as evidence that human Precision@K targets pass. CPU percentage and RSS
are emitted by the benchmark only when measurable via Linux `/proc`; embedding
inference measurements must come from the external caller.

The same analysis emits `chapter_manifest.v1`. The versioned
`segmentation.v1` profile combines semantic and lexical boundary evidence,
optional scene-boundary evidence, editor-configured size constraints, and a
complexity penalty. A global dynamic program selects the partition rather than
making a local threshold decision. Titles rank source-derived keyphrases and
brief topics by chapter overlap and token specificity; bullets remain
contiguous source spans selected using sentence importance and adjacent
semantic coherence. Every chapter and bullet carries sentence-index provenance.
Timestamps are emitted only when aligned sentence timings are supplied. The
profile weights are defaults, not empirically calibrated claims: changing them
requires a separate annotated development/validation set, with a held-out
quality evaluation before claiming segmentation quality.

The chapter regression suite includes behavioral and metamorphic checks for
unrelated topic blocks, repeated and paraphrased topics, scene-boundary
neutrality, gradual transitions, returning topics, grounded conceptual titles,
negation-preserving extractive bullets, duplicate reduction, profile bounds,
cross-language operation, deterministic replay, lexical-only fallback, and
empty/malformed/short inputs. These tests intentionally assert structural and
source-grounding properties rather than fixed title strings. They are not a
replacement for the planned 60/20/20 human-annotated split: no independent
100-script corpus or held-out human title judgments currently ship with this
repository, so WindowDiff, Boundary F1, baseline comparisons, and the proposed
90% title-acceptance target remain unclaimed.

The worker-only latency benchmark was retired with the compatibility worker.
The core's behavior is covered by the `semantic-core` phrase-impact regression
suite and by the real-binary editorial parity tests in PipelineGen, which used
the retired worker as the comparison surface before removal.

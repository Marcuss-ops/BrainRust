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
- `pipelinegen-muscles` remains the media executor and hosts the existing
  Phrase Impact and Scene Intelligence workers during their migration. It
  owns FFmpeg-related operations and must not become the semantic engine's
  dependency.

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
`semantic-core::bullets` implementation; Phrase Impact combines that score
with its embedding importance and still owns duplicate/novelty selection.
Phrase Impact is computed inside the same worker batch; Scene Intelligence
remains an explicitly opt-in shadow path pending its own retirement evidence.

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

## Languages

Linguistic knowledge is data, not Rust: one directory per language under
`lexicon/languages/`, generated into `lexicon/packs/<lang>.json` by
`lexicon/tools/build_language_packs.py`. The filter order is POS tag (when a
UDPipe model is installed) → generated closed class → the legacy flat table,
which is a bridge measured by `lexicon/tools/pack_coverage.py`. Legal facts
and micro-summary templates live in optional domain packs
(`lexicon/legal/<lang>.json`); a language without one degrades instead of
rendering another language's vocabulary.

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

The worker is exposed over the same newline-delimited JSON stdio contract as
the other muscles and is dispatched by binary name (`bin/phrase_impact`) or by
the `phrase-impact` argument. Besides the full analysis request it serves
`{"operation":"split_sentences"}`, the canonical sentence splitter the Go
adapter uses before embedding. The Go side
(`internal/platform/media/rustexec.PhraseImpactAnalyzer`) drives this worker
through the persistent process runner and is wired into the script generation
runner by `wirePhraseImpactAnalyzer`
(`external.rust_phrase_impact_path`, default `bin/phrase_impact`, installed by
`make build-muscles`). A missing or broken worker degrades to "no summary"
rather than failing the run, and the extractive fields are persisted on
`GenerateResult`/`GenerationResult` as `summary`, `bullet_points` and
`heavy_sentences`.

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

Per-title latency of the production worker (one NDJSON request/response per
titled narration, measured against `bin/phrase_impact`) is measured from a
PipelineGen checkout, which installs the worker and owns the Go benchmark:

```sh
PHRASE_IMPACT_BIN="$PWD/bin/phrase_impact" go test ./scripts/bench \
  -run '^$' -bench BenchmarkPhraseImpactTitles -benchmem
```

The offline Python comparison of extractors over archived scripts stays in
PipelineGen's `scripts/bench/phrase_impact.py`.

Run the optimized local benchmark (10 warmups / 100 samples at each requested
word count, concurrency sweep, and 10,000-run memory stress):

```sh
cargo bench -p pipelinegen-muscles --bench phrase_impact
```

The JSON report defaults to the ignored
`pipelinegen-muscles/.cache/phrase-impact/rust_benchmark.json` (Cargo runs
the benchmark from the package directory); override its path with
`PHRASE_IMPACT_BENCH_OUTPUT`.

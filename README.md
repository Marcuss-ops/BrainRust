# PipelineGen Rust components

This directory is the isolated Rust boundary for native execution
components (the “muscles”). The Go application remains the composition root
and owns canonical state, application decisions, jobs, and the transactional
outbox.

## Boundary

Rust components must communicate through explicit typed contracts. They must
not directly open PipelineGen SQLite databases, publish to Google Drive, write
Qdrant state, or load credentials. Those responsibilities remain behind the
existing Go application and infrastructure ports until a separate migration
contract is approved.

The first capability is `pipelinegen-muscles`, a newline-delimited JSON
executor for `health` and `cut_batch`. Add one focused capability at a time;
do not turn the process into a god module or expose arbitrary command
execution.

## Local check

VisualNER uses the system ICU4C RBNF/CLDR spellout rules. Install the ICU
**development** package (`libicu-dev`, including `pkg-config`) on build hosts;
runtime hosts that execute `bin/visualner` need the matching ICU shared-library
major. The build pins rust_icu's generated symbol names to the installed major
via `RUST_ICU_MAJOR_VERSION_NUMBER`; do not copy that binary to a host with a
different major. The project Dockerfile builds and runs against Bookworm ICU 72.

```sh
ICU_MAJOR=$(pkg-config --modversion icu-i18n | cut -d. -f1)
RUST_ICU_MAJOR_VERSION_NUMBER="$ICU_MAJOR" RUSTUP_TOOLCHAIN=stable rustup run stable cargo test --manifest-path rust/Cargo.toml

# Prints distinct language identifiers with installed ICU spellout rules.
RUST_ICU_MAJOR_VERSION_NUMBER="$ICU_MAJOR" RUSTUP_TOOLCHAIN=stable rustup run stable cargo run --manifest-path rust/Cargo.toml -p visualner -- --locale-count

# Build the executable consumed by the Go adapter.
make build-muscles
```

VisualNER requests must include a BCP-47 `language`. ICU does not provide
spellout rules for every locale and some rule sets are incomplete; missing
locale data fails closed. `--locale-count` reports the installed ICU count,
not a guarantee of equivalent parsing quality in every language.

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

Per-title latency of the production worker (one NDJSON request/response per
titled narration, measured against `bin/phrase_impact`):

```sh
PHRASE_IMPACT_BIN="$PWD/bin/phrase_impact" go test ./scripts/bench \
  -run '^$' -bench BenchmarkPhraseImpactTitles -benchmem
```

The offline Python comparison of extractors over archived scripts stays in
`scripts/bench/phrase_impact.py`.

Run the optimized local benchmark (10 warmups / 100 samples at each requested
word count, concurrency sweep, and 10,000-run memory stress):

```sh
cargo bench --manifest-path rust/Cargo.toml -p pipelinegen-muscles --bench phrase_impact
```

The JSON report defaults to the ignored
`rust/pipelinegen-muscles/.cache/phrase-impact/rust_benchmark.json` (Cargo runs
the benchmark from the package directory); override its path with
`PHRASE_IMPACT_BENCH_OUTPUT`.

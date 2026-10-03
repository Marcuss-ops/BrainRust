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

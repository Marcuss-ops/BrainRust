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

```sh
RUSTUP_TOOLCHAIN=stable rustup run stable cargo test --manifest-path rust/Cargo.toml

# Build the executable consumed by the Go adapter.
make build-muscles
```

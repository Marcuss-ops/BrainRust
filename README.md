# PipelineGen Rust components

This directory is the isolated Rust boundary for future native execution
components (the “muscles”). The Go application remains the composition root
and owns canonical state, application decisions, jobs, and the transactional
outbox.

## Boundary

Rust components must communicate through explicit typed contracts. They must
not directly open PipelineGen SQLite databases, publish to Google Drive, write
Qdrant state, or load credentials. Those responsibilities remain behind the
existing Go application and infrastructure ports until a separate migration
contract is approved.

The initial crate is intentionally empty. Add one focused crate or module per
execution capability rather than putting all native work into one god module.

## Local check

```sh
cargo test --manifest-path rust/Cargo.toml
```

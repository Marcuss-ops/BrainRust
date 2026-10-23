# mediaexec.v1 contract

This directory is the versioned contract between the Rust executor
(`pipelinegen-muscles`) and the Go consumer (`internal/platform/media/rustexec`
in PipelineGen). BrainRust owns the schema and the canonical goldens; the Go
side keeps a byte-identical copy in `testdata/mediaexec/v1/` because its CI
checks out the monorepo without submodules.

## Files

| File | Owner | Purpose |
| --- | --- | --- |
| `request.schema.json` | BrainRust | Golden-certified request envelope. |
| `response.schema.json` | BrainRust | Golden-certified success response envelope. |
| `goldens/*.json` | BrainRust | Canonical NDJSON request/response pairs. |
| `validate.py` | BrainRust | Schema validation of every golden. |
| `check_consumer_copy.sh` | BrainRust | Byte-parity gate for a consumer copy. |
| `testdata/mediaexec/v1/` | PipelineGen | Consumer copy, kept in lockstep. |

The golden set covers the operations both sides certify end-to-end: `probe`,
`cut_batch` and `normalize`. `render_stock` is intentionally absent: production
sends a sealed `render_plan`, whose shape is owned by the typed consumers and
their e2e suites, not by this envelope contract.

## Changing the contract

1. Edit the canonical schema and/or `goldens/` here.
2. Run `python3 contract/mediaexec.v1/validate.py`.
3. Copy the changed goldens to PipelineGen's `testdata/mediaexec/v1/` and run
   `bash contract/mediaexec.v1/check_consumer_copy.sh <that-directory>`.
4. If the change is not backward compatible, bump the protocol version in
   `pipelinegen-muscles/src/protocol.rs` and the Go `ProtocolVersion`
   constant; never change a shape under the same version.

## CI

BrainRust runs `validate.py` in the Rust workspace job. PipelineGen runs
`make verify-rust-contract` with the submodule checked out; it validates the
canonical set, checks the consumer copy, and runs the Go golden test.

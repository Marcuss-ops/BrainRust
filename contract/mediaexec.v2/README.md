# mediaexec.v2 contract

This directory is the versioned contract between the Rust executor
(`pipelinegen-muscles`) and the Go consumer (`internal/platform/media/rustexec`
in PipelineGen). BrainRust owns the schema and the canonical goldens; the Go
side keeps a byte-identical copy in `testdata/mediaexec/v2/` because its CI
checks out the monorepo without submodules.

## Files

| File | Owner | Purpose |
| --- | --- | --- |
| `request.schema.json` | BrainRust | Golden-certified request envelope. |
| `response.schema.json` | BrainRust | Golden-certified success response envelope. |
| `goldens/*.json` | BrainRust | Canonical NDJSON request/response pairs. |
| `metadata-full.json` | BrainRust | Fully populated metadata: exercises every declared field. |
| `metadata-audio-only.json` | BrainRust | Nullable metadata shape of an audio-only probe. |
| `validate.py` | BrainRust | Schema validation of every golden and metadata fixture. |
| `check_consumer_copy.sh` | BrainRust | Byte-parity gate for a consumer copy. |
| `testdata/mediaexec/v2/` | PipelineGen | Consumer copy, kept in lockstep. |

The golden set covers the operations both sides certify end-to-end: `probe`,
`cut_batch`, `normalize` and `preflight`. `render_stock` is intentionally
absent: production sends a sealed `render_plan`, whose shape is owned by the
typed consumers and their e2e suites, not by this envelope contract.

The metadata fixtures are not goldens: they exist because the golden set only
carries the fields the four certified operations needed, so it cannot notice a
metadata field that the Rust struct emits but the schema does not declare —
with `additionalProperties: false` that is a contract break, not a doc gap.
`metadata-full.json` is pinned to the Rust struct by
`protocol::tests::metadata_covers_every_declared_schema_field`, which asserts
that the emitted keys, the declared keys and the fixture keys are the same set.

`preflight` is read-only and carries a structured report. The envelope `ok` is
true whenever the check ran; Go derives the blocking decision from the
`error`-severity issues, so a blocked plan still returns its reasons.

## Changing the contract

1. Edit the canonical schema and/or `goldens/` here.
2. Run `python3 contract/mediaexec.v2/validate.py`.
3. Copy the changed goldens to PipelineGen's `testdata/mediaexec/v2/` and run
   `bash contract/mediaexec.v2/check_consumer_copy.sh <that-directory>`.
4. If the change is not backward compatible, bump the protocol version in
   `pipelinegen-muscles/src/protocol.rs` and the Go `ProtocolVersion`
   constant; never change a shape under the same version.

## CI

BrainRust runs `validate.py` in the Rust workspace job. The workspace also
runs a Rust drift test: every `Operation` variant must be declared in
`request.schema.json`, and the response envelope must carry every
schema-required field. PipelineGen runs
`make verify-rust-contract` with the submodule checked out; it validates the
canonical set, checks the consumer copy, and runs the Go golden test.

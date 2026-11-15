# imagecatalog — experimental, not wired

This crate is an **experimental snapshot planner**. It is not called by any
PipelineGen Go composition root and it is not the live entity-image flow.

The live flow is owned by the Go packages:

- `internal/capabilities/scripts/adapters` — discovery, selection, binding;
- `internal/capabilities/images/entitycatalog` — durable candidate/verdict
  contract;
- `internal/platform/sqlite/assets/entitycatalog` — catalog persistence.

The canonical contract is documented in
`refactored/docs/operations/entity-image-pipeline-flow.md`. A search hit is
`found`, not bindable; acquisition produces `content_sha256`; the verifier
produces a versioned verdict; only a selected, verified, materialized asset can
be `bound`.

Do not add a second runtime planner here without replacing the Go flow and
updating the contract. The crate remains useful for offline snapshot analysis
and fixtures only.

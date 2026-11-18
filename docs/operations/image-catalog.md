# Image catalog module (RETIRED)

**Status:** retired on 2026-11-18 (TICKET-28). `BrainRust/imagecatalog/` was
deleted: it was an offline experiment with no Go consumer and a second copy of
the catalog policy.

The live entity-image flow is documented in
`refactored/docs/operations/entity-image-pipeline-flow.md`; the durable catalog
is owned by `refactored/internal/capabilities/images/entitycatalog/`.

Historical context and the retirement decision:

- `BrainRust/docs/tickets/TICKET-28-imagecatalog-retirement.md`
- `BrainRust/docs/tickets/TICKET-02-imagecatalog-policy.md`
- `BrainRust/docs/tickets/TICKET-07-manifest-reuse.md`

The commands, fixtures, snapshot exports and `cargo test --manifest-path
imagecatalog/Cargo.toml` instructions that used to live here no longer resolve.
Do not restore them: there is no crate and no consumer.

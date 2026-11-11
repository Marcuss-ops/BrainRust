# TICKET 10 — multimodal image association with pgvector

- Priority: P2
- Status: BLOCKED
- Source: audit roadmap PR 10
- Files: `imagecatalog`, PipelineGen database owner

## Problem

`imagecatalog` currently selects PERSON identity images from the existing
catalog using certification gates and optional text annotations. It does not
compare image and text embeddings, and `score` is a deterministic ranking
score, not a calibrated probability.

## Acceptance criteria

- SigLIP (or an explicitly chosen multimodal model) embeddings stored in
  pgvector alongside the existing catalog identity, without creating a second
  catalog.
- A schema and migration owned by PipelineGen, plus a read-only snapshot or
  typed contract for Rust.
- A held-out set of correct and incorrect text–image associations, judged
  independently.
- Precision/recall and calibration measured before any automatic image is
  admitted at a 90% confidence target.
- A fail-closed fallback to the current `catalog_miss` path when the model or
  index is unavailable.

## Blocker

This needs a model choice, embedding storage/schema, pgvector credentials and
an independently judged association dataset. None of those are present in this
repository. Adding a fake embedding or an uncalibrated threshold would violate
the audit's own conclusion that a ranking score must not be called calibrated
probability.

## Next owner action

Decide the model and storage owner, then land the schema/contract in
PipelineGen before adding any Rust ranking code.

# TICKET 05 — explicit semantic manifest completeness

- Priority: P1
- Status: DONE
- Source: audit roadmap PR 5
- Files: `semantic-engine/src/protocol.rs`

## Problem

A manifest could contain `status: "rejected"` scenes while the worker response
was still `ok: true`. Consumers had to inspect every scene to distinguish a
complete analysis from a partial one.

## Change

Both cache-hit and fresh `analyze_script` responses now include an additive
`completeness` object:

```json
{
  "status": "complete | partial | rejected",
  "rejected_scenes": 1,
  "rejections": [{"scene_id": "s1", "reason": "..."}]
}
```

`ok` continues to mean “the request was processed”. `completeness.status`
means “all scenes were analysed” (`complete`), some were refused (`partial`),
or all were refused (`rejected`).

## Acceptance evidence

```sh
cargo test -p semantic-engine --lib protocol::tests
```

47/47 `semantic-engine` lib tests pass, including:

- `completeness_reports_partial_and_rejected_manifests`;
- `fresh_and_cached_wire_responses_both_carry_completeness`;
- `cached_partial_wire_response_lists_every_rejected_scene`, which verifies
  the cache-hit wire response lists the rejected scene id and reason, not only
  a scalar count.

## Residual

The field is additive on the Rust wire. PipelineGen must decide which
downstream stages accept `partial`; that policy remains in the Go owner, as the
audit requires.

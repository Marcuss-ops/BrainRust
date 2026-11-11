# TICKET 06 — cache with a serialized-byte budget

- Priority: P1
- Status: DONE (bound + representative long-script measurement)
- Source: audit roadmap PR 6
- Files: `semantic-engine/src/cache.rs`

## Problem

The persistent worker cache bounded only the number of manifests (256). Long
scripts could make total RSS much larger than the entry count suggested, and a
cache hit cloned the manifest before serialization.

## Change

- Kept the 256-entry default cap.
- Added `DEFAULT_MAX_BYTES = 128 MiB` and `ManifestCache::with_budget`.
- Each entry records the length of its serialized manifest. Insert evicts FIFO
  until both the entry cap and the byte budget hold; a manifest larger than the
  whole budget is not cached.
- `bytes()` exposes the current serialized size for observability.
- Added tests for byte-budget eviction and oversized-manifest rejection.

## Acceptance evidence

```sh
cargo test -p semantic-engine --lib cache::tests
```

All cache tests pass, including:

- `byte_budget_evicts_old_entries_before_the_entry_cap`
- `a_manifest_larger_than_the_byte_budget_is_not_cached`

## Long-script RSS measurement (2026-11-11)

Release worker, 20 long gold scripts split into scene batches, each script
analysed once and replayed once (40 requests):

| Metric | Result |
| --- | --- |
| Peak RSS (`VmHWM`) | 1,418,300 KiB |
| Peak virtual size | 2,010,508 KiB |
| Request P50 | 0.403 s |
| Request P95 | 3.600 s |
| Total request time | 37.87 s |

Raw report: `BrainRust/.tmp/ticket06-long-script-rss.json`. The peak is
dominated by the installed UDPipe/ICU working set, not by cached manifests;
the byte-budget tests remain the proof that cache memory itself is capped. This
is a host observation, not a deployment SLO.

## Residual

The 128 MiB value is a conservative policy default, not a calibrated RSS
target. The cache-hit manifest clone was not removed because the current
response serializer owns an owned value; that is a separate allocation
optimization.

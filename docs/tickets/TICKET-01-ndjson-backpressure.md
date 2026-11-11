# TICKET 01 — NDJSON backpressure end-to-end

- Priority: P0
- Status: DONE
- Source: audit roadmap PR 1
- Files: `ndjson-stdio/src/lib.rs`

## Problem

The request queue was bounded, but the result queue and the reorder buffer were
not. If request 0 was slow, completed later requests could accumulate without a
global bound, so memory depended on how many responses were waiting behind the
head.

## Change

- Added an in-flight permit window of `workers * 4`. A permit is released only
  after the response has been written in input order.
- The result channel is bounded, so a blocked writer applies backpressure to
  workers instead of growing an unbounded queue.
- A panicking request handler is converted into one `{"ok":false,...}` line;
  the worker thread does not silently lose a response.
- A stdout write failure drops the permit sender, which unblocks the reader,
  closes the job queue and terminates the pool. Previously the reader could
  wait forever for a permit after the writer had already failed.
- Explicit `workers_from_env` values are capped at `MAX_WORKERS = 32`; the
  default remains available parallelism clamped to `[1, 8]`.
- Added slow-head, panic, write-failure and worker-ceiling tests.

## Acceptance evidence

```sh
cargo test -p ndjson-stdio --lib
```

10/10 tests pass, including:

- `parallel_pool_bounds_in_flight_work_behind_a_slow_head`
- `parallel_pool_turns_a_panicking_request_into_one_error_line`
- `parallel_pool_writes_responses_in_input_order`
- `parallel_pool_terminates_when_the_output_writer_fails`
- `explicitly_configured_worker_counts_are_clamped`

## Residual

The limits are per `run_stdio_parallel` invocation. A caller that bypasses the
helper and uses its own channels is outside this guarantee.

# BR-DRY-02 — `.clone()` storm and duplicate tokenize/sentence/word primitives

- **Status:** OPEN — needs a cross-crate ownership decision for
  tokenize/split_sentences/word_count and a clone-by-clone review of
  `producers.rs`; this is not a mechanical change. Deferred until the
  concurrent workstream releases the shared files.
- **Priority:** P2
- **Owner:** `semantic-engine`, `semantic-core`, `visualner`, `overlay-compiler`, `script-qa`
- **Evidence date:** 2026-11-24

## Problem

1. `semantic-engine/src/consolidate/producers.rs` contains **38 `.clone()`**
   calls in one file. Some are necessary for ownership; the ticket must
   classify them, not mechanically remove them.
2. The workspace still has multiple implementations of core text primitives:
   - **3** `word_count`
   - **3** `split_sentences`
   - **4** `tokenize`
   - **2** `is_acronym`
   - **2** `locate_phrase`
   - **2** `overlaps`
   spread across `semantic-core`, `semantic-engine`, `visualner`,
   `overlay-compiler` and `script-qa`.
3. `numeric.rs` exists in four different versions across crates; some are
   deliberately different (Go parity, projection-only), some are drift.

## Acceptance criteria

- [ ] Each duplicated primitive has one named owner, or a written reason why
      the copy must stay (different contract, Go parity, performance).
- [ ] `producers.rs` clone count is reduced where ownership allows, and every
      remaining clone has a comment or is covered by a benchmark showing the
      allocation is required.
- [ ] No new copy of a shared primitive is introduced; the ticket lists the
      owner module for each.
- [ ] Existing golden/contract tests stay byte-identical.
- [ ] `cargo test --workspace --locked` and `cargo clippy --workspace
      --all-targets --locked -- -D warnings` are green.

## Checks

```sh
cd BrainRust
rg -n "fn (word_count|split_sentences|tokenize|is_acronym|locate_phrase|overlaps)" \
  semantic-core/src semantic-engine/src visualner/src overlay-compiler/src script-qa/src
rg -c "\.clone\(\)" semantic-engine/src/consolidate/producers.rs
cargo test --workspace --locked
```

## Out of scope

- The exact clones already owned by BR-DRY-01.
- Removing a deliberate Go-parity copy in `script-qa`.

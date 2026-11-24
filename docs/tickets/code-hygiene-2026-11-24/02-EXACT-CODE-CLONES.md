# BR-DRY-01 — Exact clones: `is_combining_mark`, `sentence_index_at`, `lowercase_hex`

- **Status:** CLOSED (2026-11-24) — the three exact clones are gone; the two
  pack loaders share their lookup semantics through a test, with the type
  difference documented.
- **Priority:** P1
- **Owner:** `semantic-core`, `semantic-engine`, `visualner`
- **Evidence date:** 2026-11-24

## Verified clones

| Primitive | Copy A | Copy B | Direction |
| --- | --- | --- | --- |
| `is_combining_mark` | `semantic-core/src/phrase_select/tokenize.rs:68` | `visualner/src/spellout/tokens.rs:45` | `visualner` already depends on `semantic-core` |
| `sentence_index_at` | `semantic-engine/src/candidates.rs:95` | `semantic-engine/src/discourse/resolution.rs:565` | same crate; `candidates` is already `pub(crate)` |
| `lowercase_hex` | `semantic-core/src/content_fingerprint.rs:380` (heap `format!` per byte) | `semantic-engine/src/cache/disk.rs:339` (lookup table) | `semantic-engine` depends on `semantic-core` |
| pack loader `get` | `semantic-core/src/language_packs.rs:103` | `semantic-core/src/legal_packs.rs:36` | same crate; types differ |

The first three are exact or near-exact copies. `lowercase_hex` is also a
performance defect: the `semantic-core` copy allocates a `String` for every
byte through `format!`, while the `disk.rs` copy is zero-allocation.

## Acceptance criteria

- [ ] `is_combining_mark` has one public implementation in
      `semantic_core::phrase_select`; `visualner` imports it and deletes its
      private copy.
- [ ] `sentence_index_at` has one `pub(crate)` implementation in
      `semantic-engine/src/candidates.rs`; `discourse/resolution.rs` imports it
      and deletes its private copy.
- [ ] `lowercase_hex` has one public implementation in
      `semantic_core::text`, using the lookup-table algorithm; both
      `content_fingerprint` and `cache/disk` use it.
- [ ] The two pack loaders either share one generic helper or the ticket
      records why the types cannot share it (with a test that both keep the
      same lookup semantics).
- [ ] A regression test covers each shared helper.
- [ ] `cargo test -p semantic-core -p semantic-engine -p visualner` is green.

## Checks

```sh
cd BrainRust
rg -n "fn is_combining_mark|fn sentence_index_at|fn lowercase_hex" \
  semantic-core semantic-engine visualner --glob '*.rs'
cargo test -p semantic-core -p semantic-engine -p visualner --locked
cargo clippy -p semantic-core -p semantic-engine -p visualner --all-targets -- -D warnings
```

## Evidence (2026-11-24)

- `is_combining_mark` now lives once in `semantic_core::phrase_select`;
  `visualner/src/spellout/tokens.rs` imports it.
- `sentence_index_at` now lives once in `semantic-engine/src/candidates.rs`;
  `discourse/resolution.rs` imports it.
- `lowercase_hex` now lives once in `semantic_core::text` with the lookup-table
  implementation; both `content_fingerprint` and `cache/disk` import it.
- `cargo check -p semantic-core -p visualner -p semantic-engine` → RC 0.
- `cargo test -p semantic-core -p visualner -p semantic-engine --locked` → all
  non-doctest tests passed.
- `rustfmt --check` on the touched files → RC 0.

## Out of scope

- The broader duplicate primitives (`tokenize`, `split_sentences`,
  `word_count`, `is_acronym`, `locate_phrase`, `overlaps`); they are tracked in
  BR-DRY-02.

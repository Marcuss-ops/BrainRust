# TICKET-29 — numbers written as words are not recognised

- Priority: P1
- Status: CLOSED (2026-11-19)
- Depends on: —
- Files: `semantic-core/src/numbers.rs` (+ new `number_words.rs`),
  `lexicon/number_words.json`, `visualner/src/spellout.rs`,
  `lexicon/tools/build_language_packs.py`, `semantic-engine` numeric extraction,
  gold fixtures

## Problem

Narration is written for the ear: "mille e duecento persone", "due milioni e
mezzo", "un terzo degli elettori", "zweihundertdreißig", "quatre-vingt-dix".
`numbers.rs` only parses digit forms (`25%`, `$1,200`, `1.200 euro`) and
deliberately returns `None` for scale words. A script that says "tre milioni"
therefore produces no numeric fact, no chart value and no counter. This ticket
is the foundation for TICKET-30 (quantities) and TICKET-31 (relative dates).

## Design

New module `semantic-core::number_words`:

```rust
pub struct NumberWordMatch {
    pub value: f64,            // exact for integers < 2^53, else None-equivalent
    pub kind: NumberWordKind,  // Cardinal | Fraction | Ordinal | Approximate
    pub start_byte: usize,
    pub end_byte: usize,       // span of the whole number phrase
}
pub fn scan_number_words(text: &str, language: &str) -> Vec<NumberWordMatch>;
```

1. **Tokenise** with the existing `discourse::word_spans` (byte-exact).
2. **Lexicon per language** (`lexicon/number_words/<lang>.json`):
   `units` (0–19), `tens`, `hundreds` (for pl/ru/tr/id that are separate words),
   `scales` (`mille/mila`, `milione/milioni`, `miliardo/i`… with `power`),
   `conjunctions` (`e`, `and`, `und`, `et`, `y`, `en`), `fractions`
   (`mezzo`, `un terzo`, `three quarters`, `halb`), `approximators` (`circa`,
   `about`, `rund`, `environ`…), `ordinals`.
   Inflected forms are listed explicitly (pl/ru/tr) the same way
   `place_inflections.json` does; no stemming heuristics.
3. **Grammar = additive/multiplicative accumulation** over a maximal run of
   number tokens (classic algorithm: `current += unit`, `current *= 100`,
   when a scale token appears `total += current * scale; current = 0`).
   Per-language adapters, all data-driven:
   - **it/es/pt**: fused compounds (`milleduecento`, `ventitré`) are expanded
     by a dictionary of prefixes; `e` joins tens+units only.
   - **de/nl**: inverted units-tens (`einundzwanzig`, `eenentwintig`) via the
     `inverted_tens: true` flag; compound splitting by longest-match against
     the unit/tens/scale lists, rejecting any leftover characters.
   - **fr**: vigesimal (`quatre-vingt-dix` = 90, `soixante-dix` = 70) as
     explicit lexicon entries; hyphen is a separator.
   - **pl/ru**: case-inflected hundreds/thousands/millions as lexicon rows.
   - **tr/id**: agglutinative-free forms (`yüz yirmi`, `seribu dua ratus`);
     `se-`/`ribu` prefix rules as data.
   - **en**: `and` after hundreds (`one hundred and five`); `a/one` before scale.
4. **Boundary rules (precision)**: a lone `un/une/one/ein/uno` is **not** a
   number unless followed by a scale/fraction/unit noun in the lexicon
   (`un milione`, `un terzo`); `a` (it/pt preposition) and `mil`/`bin` as
   ordinary words need the preceding-number context.
5. **Fail closed**: grammatically impossible runs (`due tre`, `cento cento`)
   → no match. A run that parses but is ambiguous (`mille e duecento` vs two
   numbers) follows the lexicon `conjunctions` rule; if still unclear, no match.
6. **Integration**:
   - `parse_unambiguous_value` accepts word numbers and now also returns a
     value for `3,2 miliardi` / `2 millions` (digit + scale word), since the
     scale table is data. The old "scale word → None" rule is replaced by
     "scale word with a unique power → exact product"; ambiguous grouping still
     returns `None`.
   - `NumericFact` gains `form: "digits" | "words" | "mixed"` and
     `kind: "FRACTION"` (value in (0,1], with `numerator/denominator` fields).
     `PERCENT` is derived for `un quarto dei…` only when the text says
     `per cento`; fractions are **not** silently turned into percentages.

## Acceptance criteria

- [ ] Gold: ≥ 20 spoken-number cases per language (200 total) spanning units,
      teens, tens+units, hundreds, thousands, scale+conjunction, fractions,
      approximators; recall ≥ 90 %, exact-value accuracy 100 % of matches.
- [ ] Negative set ≥ 20 per language (articles `un/a/ein`, `mil` as a name,
      `bin` as a noun, `second` as time vs ordinal, idioms such as `mille grazie`):
      0 false positives.
- [ ] Spans are byte-exact and the digit/word mixed form (`3 milioni e mezzo`)
      is one fact.
- [ ] `build_language_packs.py --check` covers `number_words`; adding a language
      without the file fails the gate.
- [ ] Existing numeric/chart tests unchanged (charts keep requiring grounded
      values; word numbers become eligible only through `parse_unambiguous_value`).
- [ ] Replay byte-identical; `AnalyzerVersions.numbers` bumped.

## Non-goals

- Locale-specific "short vs long scale" guessing (`billion`): documented table
  per language, no inference. `miliardo`/`billion` → 10⁹ only where the
  language profile says so.
- Number-to-words generation, ordinal dates (handled in `dates/words.rs`).

## Implementation steps

1. Lexicon schema + generator + `--check` (it, en first).
2. `scan_number_words` + unit tests per grammar family.
3. Wire into `numbers.rs`; extend `NumericFact` additively; Go consumer copy.
4. Remaining 8 languages one by one, each with its gold slice.
5. Report metric in `score_long_gold.py`.

## Percorso seguito (2026-11-19)

Tutti i punti sotto sono stati eseguiti; le evidenze sono in «Verification
2026-11-19».

1. **Primo blocco eseguibile:** schema `lexicon/number_words/<lang>.json` +
   generatore + `--check`, con it ed en complete; finché manca lo schema non si
   può scrivere né lo scanner né il gold.
2. **Secondo blocco:** `scan_number_words` con i test per famiglia
   (it/es/pt fusi, de/nl invertiti, fr vigesimale, pl/ru flessi, tr/id).
3. **Terzo blocco:** innesto in `numbers.rs` + `NumericFact.form` additivo; la
   copia Go e i gold si aggiornano nello stesso commit.
4. **Dipendenze:** TICKET-30 e TICKET-31 non partono prima del punto 3; il
   ticket 31 usa gli importi in parole per `amount`.

Gate per dichiararlo chiuso: 200 casi positivi (20 per lingua) con recall
≥ 90 % e valore esatto sul 100 % dei match, 0 falsi positivi sul set negativo,
`build_language_packs.py --check` che fallisce senza il file di lingua.

## Verification 2026-11-19

Implemented and measured on the working tree:

- `lexicon/number_words.json` (schema `number_words.v1`) carries conjunctions,
  fraction articles, additive/denominator fractions, approximators, fused half
  suffixes and idiom negatives for all 11 pack languages; no language branch
  lives in Rust.
- `visualner/src/spellout.rs`: case folding before CLDR parsing, internal
  conjunction variants, additive/multiplicative run composition, denominator
  fractions (`un terzo`), additive halves (`due milioni e mezzo`,
  `two and a half million`), fused halves (`zweieinhalb`, `tweeënhalf`),
  approximator spans and idiom rejection.
- Mixed `NumericFact.form` (`digits|words|mixed`); `3 milioni e mezzo` is one
  fact with value 3.5e6, and `AnalyzerVersions.numbers = number_words.v1`.
- Gold: **20 positive + 20 negative cases per language** (it, en, es, pt, fr,
  de, nl, pl, ru, tr, id = 220 + 220), assistant draft pending human review.
  Positive values are checked exactly; negatives require zero candidates.
- `python3 lexicon/tools/build_language_packs.py --check` now fails when a pack
  language has no `number_words` entry; it passes.
- Checks: `cargo test -p visualner` **121 passed** plus integration suites;
  `cargo test -p semantic-core` **225 passed**; `cargo test -p semantic-engine
  --tests` green; `cargo clippy -p semantic-core -p visualner --all-targets -D
  warnings` rc 0.
- Limitation: `cargo test -p semantic-engine` still fails at rustdoc doctests
  with `E0463` (rustdoc cannot find the already-built crates); `--tests` is the
  gate. `cargo clippy -p semantic-engine` cannot run because `overlay-compiler`
  has pre-existing compile errors unrelated to this ticket.
- Go consumer: the new `form` field is skipped when absent, so existing
  manifests and goldens are byte-identical; word forms only add facts.

# Languages: generated packs, not word lists in Rust

This repository is built so that **adding a language is a data change**. No
Rust match arm, no per-language word array, no copied stopword file.

## The three sources of linguistic knowledge

| Layer | Where it lives | Who consumes it |
|---|---|---|
| Closed class (articles, adpositions, pronouns, conjunctions, particles, auxiliaries) | `lexicon/languages/<lang>/*.txt` → generated `lexicon/packs/<lang>.json` | the extractor's candidate filter |
| Structure (script, casing, date order, separators) | `lexicon/language_profiles.json` | tokenizer, date parser, entity typing |
| Calendar (month names, format/standalone contexts) | `lexicon/months.json`, generated from CLDR by `lexicon/tools/sync_cldr_months.py` | date parser, entity typing |

POS tagging is the first filter: when a UDPipe model for the language is
installed, `is_stop_token` uses the tag (`DET`, `ADP`, `PRON`, `CCONJ`,
`SCONJ`, `PART`, `INTJ`, `PUNCT`, `SYM`, `VERB` never start or extend a
candidate). The generated pack is the deterministic fallback for hosts and
languages without a model.

There is **no cross-language fallback table**: the pack is the whole fallback.
A language without a pack has no word-class filter (tier C below), which is a
declared degradation, not a silent guess. Words that name a class rather than
an entity ("company", "contains", "traditionally") live in that language's
`entity_blocklist.txt` and are generated into the pack's `blocked` set.

## Generated vs. domain packs

**Generated** (`lexicon/tools/build_language_packs.py`):

```sh
python3 lexicon/tools/build_language_packs.py          # write
python3 lexicon/tools/build_language_packs.py --check  # CI gate
```

It emits, deterministically:

- `lexicon/packs/<lang>.json` — `closed_class` (the filter set) and `blocked`
  (class nouns/verbs from `entity_blocklist.txt`), plus `function_words` as
  data only: that wider Go list contains open-class words such as `great` and
  `mountains` that belong **inside** names, so it is never a filter;
- `lexicon/packs/index.json`;
- `lexicon/legal/index.json`;
- `semantic-core/src/generated_language_packs.rs`;
- `semantic-core/src/generated_legal_packs.rs`.

CI fails when a committed artifact differs from the generator output, so a
lexicon edit cannot leave a stale pack behind.

**Domain packs** are optional: `lexicon/legal/<lang>.json` carries the legal
markers, entity cues, micro-summary templates and comparison titles for one
language. A language without a legal pack produces **no legal facts and no
micro summaries**; the generic fact pipeline still runs, and nothing is
rendered in another language.

## The 100-language target: tiers, not promises

| Tier | What is available | Behaviour |
|---|---|---|
| A | UDPipe model + closed-class data + CLDR months | full candidate filtering, POS-driven typing |
| B | closed-class data + CLDR months (no model) | pack-driven filtering; capitalization guards stand in for the tagger |
| C | CLDR months only | numeric dates and month names; no word-class filtering (degraded, declared) |
| Domain | legal pack present | legal facts + localized micro summaries for that language |

CLDR covers hundreds of locales; UDPipe 2.5 ships models for roughly seventy
languages; legal extraction is a curated domain that will never be universal.
Tier C is a declared degradation, not a silent guess: the extractor never
applies another language's markers to a text.

## Adding a language

1. Create `lexicon/languages/<lang>/` with the closed-class files
   (`stopwords.txt`, `articles.txt`, `pronouns.txt`, `possessives.txt`,
   `pronoun_clitics.txt` / `pro_drop_aux.txt` where the language needs them,
   `subordinate_markers.txt`, `negative_particles.txt`, `contrast_openers.txt`).
2. Add the profile (`treebank`, `script`, `date_order`) to
   `lexicon/language_profiles.json`; run `lexicon/tools/sync_cldr_months.py`
   for the month table.
3. Run the generator (`--check`) and commit the artifacts.

Optional: add `lexicon/legal/<lang>.json` to enable the legal domain. Without
it the language degrades to generic facts, by design.

## Verification

```sh
python3 lexicon/tools/build_language_packs.py --check
RUST_ICU_MAJOR_VERSION_NUMBER=$(pkg-config --modversion icu-i18n | cut -d. -f1) \
  cargo test -p semantic-core -p visualner -p semantic-engine
```

`visualner/tests/language_profiles.rs` fails closed when a profile declares a
UDPipe treebank and the model is missing, so a language cannot be declared
supported while its model is absent.

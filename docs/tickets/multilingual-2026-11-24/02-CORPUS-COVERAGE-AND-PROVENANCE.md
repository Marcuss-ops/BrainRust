# BR-MULTI-02 — The 17-language/51-script corpus is not reproducible

- **Status:** BLOCKED
- **Priority:** P0
- **Owner:** corpus owner / BrainRust evaluation
- **Checked:** 2026-11-24

## Problem

The linked Drive folder was downloaded with `gdown --folder`. It contains
**11 files**: ten Italian `.txt` scripts and one `README.md`.

The README describes a larger package:

- `gold.json` with annotations, textual evidence, alternatives and negative
  controls;
- `scripts/` with readable scripts;
- `requests/` with individual JSON requests;
- `requests.ndjson` with one `analyze_script` request per line, including the
  `editorial` block;
- a scorer for the 51-script, 17-language corpus.

None of those files is present in the linked folder. The folder therefore
cannot measure the promised 17 languages, 51 scripts, 85 scenes or 680 spans.

## Repository coverage

- `semantic-engine/fixtures/gold_multilingual/` has **20 short scripts**
  (10 languages × 2) and **20 long scripts** (10 languages × 2).
- `semantic-engine/fixtures/multilingual_100/` has **100 synthetic scripts**
  (10 languages × 10), but it is a structural stress suite, not a gold set.
- `lexicon/language_profiles.json` declares **13 treebanks** (`de`, `en`, `es`,
  `fr`, `id`, `it`, `ja`, `nl`, `pl`, `pt`, `ru`, `tr`, `zh`) and four
  profile-only languages without a treebank (`ko`, `ar`, `hi`, `th`).
- There is no 17-language long gold, no 51-script gold, and no native-gold
  corpus for the four profile-only languages.

## Final 10-language evidence

The 2026-11-24 run supports a **10-language** statement:

- short gold: recall 1.000 (98/98), precision 0.991 (112/113), leaks 0;
- long gold: entity recall 0.984 (122/124), typed object recall 0.905
  (95/105), surface object recall 0.933 (98/105), contextual queries 19/20,
  grounding 0, replay identical;
- the enforced long-gold gate passes and exits 0.

It cannot support a **17-language** statement. The four profile-only languages
have no treebank, and the linked corpus does not contain the promised native
or parallel gold. Reporting a 17-language average would hide that gap.

## Acceptance criteria

- The complete corpus is available in a stable location, with a checksum and a
  provenance README.
- `gold.json`, `requests.ndjson` and the scorer are present and mutually
  consistent; UTF-8 spans validate.
- The 51 requests run through the release worker; the report is per language.
- The corpus is explicitly marked draft or human-certified; no certification
  is inferred from file presence.
- The 10-language and 17-language claims are reported separately.

## Non-goals

- Inventing annotations for the ten `.txt` files.
- Treating the existing 100-script synthetic suite as a 17-language gold set.
- Adding `ko`, `ar`, `hi` or `th` models in this ticket.

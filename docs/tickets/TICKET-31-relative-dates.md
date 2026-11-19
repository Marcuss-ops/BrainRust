# TICKET-31 — relative dates are not anchored

- Priority: P1
- Status: CLOSED (2026-11-19)
- Depends on: TICKET-29 (spoken numbers)
- Files: `semantic-core/src/dates/{relative.rs (new),resolve.rs,scan.rs}`,
  `lexicon/relative_time.json`, `semantic-core/src/dates.rs` (`TemporalFact`),
  request type (`reference_date`), `semantic-engine` date pass

## Problem

`TemporalFact` covers absolute dates only. "Tre anni dopo", "il giorno
seguente", "l'anno scorso", "nel decennio successivo", "10 years ago" are not
extracted, so chronology groups and timelines lose half the story and a
narrative like "Nel 2008 … tre anni dopo …" cannot be ordered.

## Design

Extend `TemporalFact` additively:

```jsonc
{ "type": "DATE", "precision": "year", "normalized_date": "2011",
  "relative": {
     "kind": "offset|named|span",       // "tre anni dopo" | "ieri" | "negli anni 80"
     "direction": "before|after|same",
     "amount": 3, "unit": "year",       // day|week|month|year|decade|century
     "anchor": { "type": "fact", "fact_index": 0 }   // or "reference" | "unresolved"
  },
  "derived": true }
```

1. **Lexicon `relative_time.json`** per language: units (`giorno/i`, `Tag`,
   `jour`…), direction cues (`dopo`, `prima`, `fa`, `later`, `ago`, `earlier`,
   `nach`, `vor`, `después`, `antes`, `через`, `назад`, `sonra`, `önce`),
   named relatives (`ieri`, `oggi`, `domani`, `l'anno scorso`, `next year`,
   `il giorno dopo`), prefix cues for decade/century. `amount` comes from
   digits or TICKET-29 number words.
2. **Scan** reuses the word-span tokenizer; the relative phrase span is the
   fact span (`tre anni dopo`, not the whole sentence).
3. **Anchor resolution** (deterministic, fail closed), in this order:
   1. explicit anchor in the same phrase (`tre anni dopo il 2008`);
   2. **narrative anchor** = nearest preceding *absolute* `TemporalFact` in the
      same scene thread (same discourse thread / same sentence group). Never
      jump across a scene that introduces a different dated thread;
   3. `reference` only for *deictic* forms (`ieri`, `3 years ago`, `l'anno
      scorso`) and **only if the request carries `reference_date`** (new
      optional field, ISO `YYYY-MM-DD`; absent by default → `unresolved`);
   4. otherwise `unresolved` with the relative payload preserved
      (`normalized_date: None`).
4. **Arithmetic**: calendar-correct month/year arithmetic via the existing
   `resolve.rs` day-of-month rules (clamp, leap years); resulting precision =
   `min(anchor precision, unit precision)` (3 years after `2008-05` → `2011-05`
   precision month; `il giorno dopo` with a year-only anchor → unresolved).
   BCE anchors → unresolved (same boundary as today).
5. **`derived: true`** distinguishes inferred dates from stated ones so that
   `groups.rs` chronology can order them but a downstream consumer can still
   choose to display only stated dates. Chronology group dates use
   `normalized_date`, ordering is unchanged for stated-only scripts.
6. **Ambiguity**: `dopo` without a preceding anchor in a different temporal
   sense (`dopo la guerra`) is **not** a relative date (no unit amount) → no
   fact. A relative expression whose anchor candidate set has two competing
   absolute dates in the same thread → `unresolved`, candidates recorded in
   `anchor.candidates` for auditing.

## Acceptance criteria

- [ ] Gold ≥ 20 relative expressions per language (offset, named, span,
      deictic): recall ≥ 90 %, resolved-date accuracy 100 % of resolved,
      unresolved rather than wrong when the anchor is missing/ambiguous.
- [ ] Negative set ≥ 20 per language (`dopo di te`, `due ore fa` without a
      reference when `reference_date` is absent stays `unresolved` and is not
      reported as resolved, `prima che`, idioms): 0 false positives.
- [ ] With `reference_date` absent the output for scripts that contain only
      absolute dates is byte-identical to today's goldens.
- [ ] Month/leap-year arithmetic covered by table tests (29 Feb, month ends).
- [ ] `AnalyzerVersions.dates` bumped; fingerprint includes `reference_date`.
- [ ] Go consumer copy/goldens updated.

## Non-goals

Time-of-day and timezones, recurring dates (`ogni lunedì`), fuzzy periods
(`all'epoca`) without a numeric offset, inferring today's date from a clock.

## Implementation steps

1. Lexicon + generator (it/en), `relative.rs` scan with unit tests.
2. Anchor resolver over `TemporalFact` list + discourse threads.
3. Arithmetic + precision rules; additive schema; fingerprint field.
4. Other languages + gold; metric in `score_long_gold.py`.

## Prossimi passi (2026-11-19)

1. **Decisione da prendere subito:** `reference_date` entra nella richiesta o
   resta assente e ogni forma deittica esce `unresolved`? La seconda opzione
   è più piccola e non tocca il fingerprint; va scelta e scritta qui.
2. **`relative_time.json` + `relative.rs`** (it/en) con i test di scansione,
   prima del resolver.
3. **Anchor resolver** su `TemporalFact` + thread di discorso, con
   `anchor.candidates` quando due date assolute competono.
4. **Aritmetica calendario** (29 febbraio, fine mese) e precisione
   `min(anchor, unit)`.

Gate: 20 espressioni relative per lingua con recall ≥ 90 %, date risolte
esatte sul 100 % dei risolti, e output byte-identico ai golden attuali quando
`reference_date` è assente.

## Verification 2026-11-19 — CLOSED

Implementato e verificato sull'albero di lavoro:

- `lexicon/relative_time.json` (`relative_time.v1`) per 11 lingue: unità
  giorno/settimana/mese/anno/decennio/secolo, cue `after`/`before`/`ago`,
  prefissi (`hace`, `vor`, `il y a`, `через`, `dans`…), forme nominate
  deittiche e narrative, span di decennio.
- `semantic-core/src/dates/relative.rs`: scansione a token, importi da cifre o
  span TICKET-29, àncorа esplicita > riferimento > ancora narrativa, altrimenti
  `unresolved`; aritmetica civile con clamp di fine mese e 29 febbraio;
  `TemporalFact.relative` + `derived: true` additivi.
- `ScriptRequest.reference_date` opzionale (`YYYY-MM-DD`), incluso nel
  fingerprint e nella cache; assente/invalido non inventa la data odierna.
- `AnalyzerVersions.dates = relative_time.v1` e fingerprint del lessico.
- Gold matrix: **21 espressioni × 11 lingue = 231 casi** (named deittici e
  narrativi, offset per tutte le unità, before/ago, importo in parole, ancora
  esplicita, span); valori ISO attesi letterali. Matrice negativa: **20 casi
  × 11 lingue** (cue senza importo, importo senza unità, unità senza
  direzione) a 0 fatti relativi.
- Gate: `cargo test -p semantic-core --lib dates::relative` 9/9; suite
  semantic-core, visualner e semantic-engine `--tests` verdi; clippy rc 0;
  `build_language_packs.py --check` valida anche `relative_time.json`.
- Limite noto invariato: `cargo test -p semantic-engine` fallisce solo sui
  doctest rustdoc con `E0463`; il gate è `--tests`.

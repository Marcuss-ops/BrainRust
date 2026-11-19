# TICKET-33 — people have no role, affiliation, family tie or age

- Priority: P2
- Status: OPEN
- Depends on: TICKET-31 (validity dates), TICKET-35 (correct person link)
- Files: `lexicon/roles/<lang>.json`, `lexicon/family_relations.json`,
  `semantic-core/src/{roles.rs (new),relations.rs}`,
  `semantic-engine/src/roles.rs` (new), `SceneManifest.person_roles`

## Problem

"Mario Rossi, CEO di Acme" produces a PERSON and an ORG; the link between them
and the title are lost. `relations.rs` only knows ownership/funding/
collaboration between co-mentioned entities. A lower third, an org chart or a
"who is this?" caption has no data to read.

## Data model (additive)

```jsonc
// SceneManifest.person_roles[]
{ "id": "pr_01", "person_id": "per_02",
  "role_code": "chief_executive",      // closed vocabulary (roles.json), language independent
  "role_surface": "CEO", "rank": "head|deputy|member|former|acting",
  "organization_id": "org_01", "place_id": null,   // exactly one scope or none
  "temporal": { "from": "2015", "to": null, "fact_index": 1 },
  "tense": "current|former|appointed|resigned|unknown",
  "status": "asserted|negated|uncertain",
  "span": [..], "evidence": "…", "sentence_index": 0 }

// SceneManifest.person_relations[]
{ "from_id":"per_02","to_id":"per_05","kind":"parent_of|spouse_of|sibling_of|child_of|partner_of|colleague_of|employer_of|lawyer_of",
  "status":"asserted", "span":[..], "evidence":"…" }

// SceneManifest.age_facts[]
{ "person_id":"per_02", "age": 45, "as_of": {"fact_index": 2}, "derived_birth_year": null }
```

## Design

1. **Role vocabulary** (`roles/<lang>.json`): `role_code` → surfaces with
   gender/number/case forms (`presidente/presidentessa`, `ministro/ministra`,
   `Geschäftsführer/in`, `prezes/prezesa/prezesem`), scope type expected
   (`org|place|none`), and tense markers (`ex`, `former`, `ehemalig`, `neo`,
   `acting`, `ff.`). Codes are a small closed set (~80) that starts with
   politics, business, law, sport, media, science, police/justice.
2. **Patterns** (clause-local, byte-grounded):
   - apposition: `Name, ROLE di/of ORG` and reverse `ROLE di ORG Name`;
   - possessive/genitive: `ORG's CEO Name`, `Name der Geschäftsführer von ORG`;
   - copular/appointment verbs from `relation_cues.json`: `è stato nominato`,
     `was appointed`, `ha lasciato`, `resigned` → `tense`;
   - title + surname only (`il presidente Mattarella`) when the surname maps
     to exactly one PERSON identity in the registry.
3. **Disambiguation of capitalised titles** vs names: a role surface is only a
   role when the lexicon matches it as a whole word *and* it precedes/follows a
   PERSON entity within the apposition window; otherwise it stays an ordinary
   word. Names that equal a title (`Presidente` as a surname) are guarded by
   the PERSON entity span taking precedence.
4. **Scope**: `organization_id` only when the ORG is grounded in the same
   clause; a role without scope is allowed (`il ministro`), never completed from
   elsewhere. A bare role with a pronoun subject is resolved only through
   TICKET-36.
5. **Validity** (`temporal`): `dal 2015`, `since 2015`, `fino al 2019` bound the
   role; `ex`/`former` set `tense: former`; relative dates come from TICKET-31.
   No inferred end date.
6. **Family and work relations**: closed predicate table with direction
   (`padre di` → `parent_of(from=father,to=child)`), in all 10 languages;
   gendered/inflected forms enumerated. Extends `RelationView` with a
   `person_relations` projection; existing ownership/funding stay unchanged.
7. **Age**: `a 45 anni`, `45-year-old`, `45enne`, `im Alter von 45` → age fact;
   `derived_birth_year` only when the same sentence/thread has an explicit
   absolute date and the age is stated for that date (`nel 1998, a 30 anni`
   → 1968). Never derived from "today".

## Acceptance criteria

- [ ] Gold ≥ 30 role mentions and ≥ 15 family/work relations per language:
      role_code accuracy ≥ 90 %, scope binding precision ≥ 95 %, tense accuracy
      ≥ 90 %, relation direction 100 % of emitted.
- [ ] Negative set ≥ 20 per language (title used generically, `il presidente`
      without a person, titles inside organisation names, surname = role word,
      negated/hypothetical appointments): 0 false positives.
- [ ] Gender/inflection coverage for pl/ru/tr/de verified by dedicated tests.
- [ ] Output deterministic; `AnalyzerVersions.roles` bumped; Go copy updated.

## Non-goals

Verifying that someone really holds a role, resolving fictional/historical
title systems (nobility), photo/face association, org-chart layout.

## Implementation steps

1. `roles.json` schema + generator; seed 40 codes in it/en.
2. Apposition and genitive patterns with unit tests.
3. Tense/validity binding (needs TICKET-31 for relative expressions).
4. Family/work relations + age facts.
5. Remaining 8 languages + gold + metrics.

## Prossimi passi (2026-11-19)

1. **`roles.json` + generatore**, 40 codici in it/en: è l'unico blocco che non
   dipende da altri ticket.
2. **Apposizione e genitivo** (`Mario Rossi, CEO di Acme`) con test; se il
   legame persona↔organizzazione è sbagliato, il ruolo non si emette.
3. **Validità temporale** solo dopo TICKET-31 (per le forme relative).
4. **Parentele/età** dopo i legami di lavoro.

Gate: ruolo corretto ≥ 90 % sul gold, nessun ruolo inventato quando
l'apposizione è ambigua, golden dei fatti entità invariati per gli script
senza ruoli.

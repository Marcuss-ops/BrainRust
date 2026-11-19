# TICKET-36 — pronoun and role-nominal coreference is too conservative

- Priority: P2
- Status: OPEN
- Depends on: TICKET-35 (reliable antecedent identities); benefits from 33
- Files: `semantic-engine/src/discourse/{references.rs,resolution.rs,threads.rs}`,
  `semantic-core/src/narrative.rs`, `lexicon/languages/<lang>/pronouns.txt`
  (→ structured `pronouns.json`), `lexicon/given_names.json`

## Problem

`discourse.v1` resolves a pronoun or role nominal only "when one compatible
antecedent remains"; everything else is `ambiguous`/`unresolved`. That is safe
but loses recall: "Stasi fu arrestato. **Lui** negò", "Acme ha annunciato…
**La società** ha poi…", "Il sindaco … **il primo cittadino**…". Threads stay
short, and TICKET-33/34 lose their subjects.

## Design (keep the safety rule: abstain when unsure)

1. **Structured pronoun lexicon** per language:
   `{surface, person, gender, number, animacy, case, kind: personal|possessive|demonstrative|relative|reflexive}`.
   Currently several languages have flat lists (`ru/pronouns.txt`,
   `it/function_words.txt`); convert to `pronouns.json` and generate checks in
   `build_language_packs.py`.
2. **Antecedent features**: for each identity, `gender` and `number`:
   - from the entity index row (type + gloss) when linked;
   - from `given_names.json` (name → gender, curated, with `unknown` allowed);
   - from role forms (`la ministra` → feminine, TICKET-33);
   - from agreement evidence in text (participle/adjective ending, determiner:
     `è stata arrestata`, `la presunta`, `sie wurde`);
   - ORG/LOCATION = inanimate; plural for coordinated names.
   Unknown stays unknown and *does not* filter candidates.
3. **Candidate window**: antecedents from the previous 2 sentences in the scene
   (and the last sentence of the previous scene only if the thread continues
   there), never forward (no cataphora).
4. **Salience ranking** (deterministic): recency, grammatical subject of the
   previous clause (UDPipe dependency `nsubj` when the model is loaded; else
   position heuristic), first mention in paragraph, being the topic of the
   current thread. Resolve iff the top candidate beats the second by a margin;
   otherwise `ambiguous` with candidates (as today).
5. **Pro-drop languages** (it/es/pt/pl/tr/ru/id…): zero-subject clauses are not
   spans, so they are *not* emitted as references. What is emitted: reflexive/
   possessive/relative pronouns and agreement-driven participles
   (`è stata arrestata`) as **evidence for gender** of the preceding subject
   (feeds step 2), not as a new reference.
6. **Role nominals** (`la società`, `il presidente`, `the company`, `der
   Konzern`): resolve to an identity of compatible type via roles/ORG-type
   nouns, same salience rule; a nominal compatible with two identities stays
   `ambiguous`.
7. **Threads**: resolved references join the identity thread; counts keep the
   same `total/resolved/ambiguous/unresolved` shape so consumers are unchanged.
   Add `resolution_basis` (`agreement`, `salience`, `unique_candidate`,
   `role_nominal`) for auditing.

## Acceptance criteria

- [ ] Coref gold ≥ 40 pronoun/nominal references per language incl. mixed-gender
      and same-type competitors: resolved recall ≥ 80 % (up from the current
      measured baseline, to be recorded first), **resolved precision ≥ 97 %**,
      remainder ambiguous/unresolved.
- [ ] Negative/trap set ≥ 20 per language (two same-gender candidates, generic
      `they`, `it` referring to a clause, pleonastic `es/il`, cataphora): 0
      confident wrong resolutions.
- [ ] Existing `discourse` tests and goldens: 0 resolved→wrong regressions;
      previously resolved items stay resolved.
- [ ] Baseline report (before/after) per language in `score_long_gold.py`.
- [ ] Deterministic; `AnalyzerVersions.discourse` bumped; Go copy updated.

## Non-goals

Event/clausal coreference, split antecedents (`i due`), bridging anaphora
(`the engine` → car), neural coreference models, zero-pronoun span emission.

## Implementation steps

1. Measure the current baseline on the new gold (no code change).
2. Structured pronoun lexicon + generator checks.
3. Gender/number feature extraction.
4. Salience resolver + margin rule; `resolution_basis`.
5. Role nominals (after TICKET-33), per-language gold, metrics.

## Prossimi passi (2026-11-19)

1. **Misurare il baseline** sul nuovo gold senza toccare il codice: senza il
   numero di partenza non si distingue il guadagno dalla varianza.
2. **Lessico pronomi strutturato** + check del generatore (i file
   `pronouns.txt` per lingua sono ancora testo libero).
3. **Feature genere/numero** e resolver di salienza con regola di margine:
   sotto il margine resta `ambiguous` (la regola di sicurezza non cambia).
4. **Nominali di ruolo** solo dopo TICKET-33.

Gate: recall dei pronomi in aumento sul gold senza falsi antecedentati;
`resolution_basis` presente su ogni risoluzione.

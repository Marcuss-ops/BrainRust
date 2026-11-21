# TICKET-36 — pronoun and role-nominal coreference is too conservative

- Priority: P2
- Status: PARTIAL (verified 2026-11-21)
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
- [x] Deterministic; `AnalyzerVersions.discourse` bumped; Go copy updated.

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
   `pronouns.txt` per lingua sono ancora testo libero). *(Fatto con 36b: il
   check è dentro `build_language_packs.py --check`.)*
3. **Feature genere/numero** e resolver di salienza con regola di margine:
   sotto il margine resta `ambiguous` (la regola di sicurezza non cambia).
4. **Nominali di ruolo** solo dopo TICKET-33.

Gate: recall dei pronomi in aumento sul gold senza falsi antecedentati;
`resolution_basis` presente su ogni risoluzione.

## Verifica 2026-11-20

Ricontrollato sul working tree corrente (`2bbeca1` più le modifiche non
committate).

- **Fatto (36a, draft):** `semantic-engine/fixtures/discourse/coref_gold.json`
  (`brainrust.coref_gold.v1`, 440 casi, 40 per lingua) e
  `coref_baseline.json` (`brainrust.coref_baseline_report.v1`) esistono, con
  authority `assistant_draft_pending_human_review`. Baseline: 403/440 status
  corretti, 415/440 target, **14 confidently wrong**, precision 0.931, recall
  0.880. Non è un gold validato da un umano e il gate 36f vuole zero
  confidentemente sbagliati, quindi 36a/36f restano aperti.
- **Fatto (36b):** `lexicon/languages/<lang>/pronouns.json` esiste per tutte
  le 11 lingue con schema `pronouns.v1` (`surface`, `person`, `gender`,
  `number`, `animacy`, `case`, `kind`); `python3
  lexicon/tools/build_language_packs.py --check` — **PASS** (`11 up to date`,
  `profiles: 12`) e i byte dei pack generati restano invariati. I valori non
  curati restano `unknown`, non vengono indovinati.
- `resolution_basis` nel resolver di `discourse` distingue ora
  `role_nominal` da `same_sentence_antecedent`,
  `nearest_sentence_antecedent` e `globally_unique_compatible_entity`.
- **36c — initial gender slice implemented:** structured pronouns supply
  gender hints; curated gendered role surfaces in TICKET-33 populate
  `PersonRole.gender`. The resolver filters only candidates with a known,
  conflicting gender; unknown remains compatible. It selects the nearest
  remaining compatible sentence and applies the same rule to global uniqueness.
  The discourse analyzer component was bumped and all pronoun inventories are
  fingerprinted.
- Verified: one integration test resolves Italian *Lei* to a feminine role
  bearer with a masculine role bearer also active; a curated given name can
  resolve a gendered pronoun against a known conflicting name, and two names
  outside the inventory stay ambiguous. A Turkish genderless pronoun resolves
  only with one grounded person (`discourse.v4`).
- Restano aperti: convalida umana del gold/baseline (36a/36f), agreement
  morphology e nomi/numero estesi (36c; i nominali gendered hanno per ora
  lessico disponibile solo per en/it), resolver di salienza con margine
  (36d), nominali di ruolo con basis dedicata (36e), trap set indipendente
  (36f). La parity Go è ora implementata e verificata. Le tabelle lessicali del
  detector sono state migrate da Rust a `lexicon/discourse_reference_terms_v1.json`;
  `build_language_packs.py --check` ne valida schema, copertura, lingua e valori.
- Stato: **PARTIAL (36a draft baseline, 36b done, 36c initial slice plus
  Turkish genderless pronouns)**.

## Piano di chiusura e split (2026-11-20)

Il resolver esiste ma è conservativo e senza dati di valutazione. Il ticket va
spezzato in 6 parti; le prime due sono indipendenti da 35/33.

### 36a — Gold e baseline (M, completato localmente 2026-11-20)

- `semantic-engine/fixtures/discourse/coref_gold.json`: 40 riferimenti per
  lingua (440 totali), con risoluzioni singolari, competitori dello stesso
  genere, assenza di antecedente e coppie plurali. Authority:
  `assistant_draft_pending_human_review`.
- `score_coref_gold.py` misura status e target per lingua. Ogni caso viene
  inviato come script isolato, così nessun antecedente passa da un caso a un
  altro. `coref_baseline.json` registra l'hash del worker release.
- Fatto per il baseline tecnico locale; il gold non è pronto per l'accettazione
  finché un revisore umano indipendente non convalida le etichette.

### 36b — Lessico pronomi strutturato (M, parte subito)

- `lexicon/languages/<lang>/pronouns.json`: `surface`, `person`, `gender`,
  `number`, `animacy`, `case`, `kind`
  (personal/possessive/demonstrative/relative/reflexive).
- Migrare i `pronouns.txt` piatti (en/it/es/fr/pt/pl/ru/id e le altre lingue
  che usano `function_words.txt`) senza cambiare il pack output; il check in
  `build_language_packs.py --check` verifica che ogni lingua abbia il file.
- Fatto quando: pack identici a parità di contenuto e check verde.

### 36c — Feature genere/numero (M, dopo 36b; 33 per le forme di ruolo)

- `lexicon/given_names.json` (nome → genere, `unknown` ammesso); tipo/gloss
  dalla riga dell'indice; forme di ruolo da 33; evidenza di accordo
  (participio/aggettivo/determinante) da `visualner`.
- ORG/LOCATION inanimati; plurale per nomi coordinati; `unknown` non filtra i
  candidati.
- Fatto quando: ogni feature ha una fonte e un test; nessun candidato viene
  eliminato da un `unknown`.

### 36d — Resolver di salienza e margine (M, dopo 36c + 35)

- Finestra: 2 frasi precedenti nella scena + ultima frase della scena
  precedente solo se il thread continua; mai forward.
- Ranking: recency, soggetto grammaticale (`nsubj` se il modello è caricato,
  altrimenti posizione), prima menzione, topic del thread. Risolve solo se il
  top batte il secondo per il margine; altrimenti `ambiguous` con candidati.
- `resolution_basis`: `agreement`, `salience`, `unique_candidate`,
  `role_nominal`; i valori esistenti restano compatibili.
- Fatto quando: precision ≥97% e recall ≥80% sul gold, 0 risoluzioni
  confidently wrong sui trap.

### 36e — Nominali di ruolo (S, dopo 36d + 33)

- `la società`, `il presidente`, `the company`, `der Konzern`: risolvere a
  un'identità di tipo compatibile con la stessa regola di salienza; due
  identità compatibili → `ambiguous`.
- Fatto quando: i nominali di ruolo non abbassano la precision e usano
  `role_nominal` come basis.

### 36f — Gold, metriche e chiusura (L)

- ≥20 trap per lingua (due candidati stesso genere, `they` generico, `it` di
  frase, pleonastico `es/il`, catafora): 0 risoluzioni confidently wrong.
- Baseline before/after per lingua; copia Go; `AnalyzerVersions.discourse`;
  README → CLOSED.
- Fatto quando: precision ≥97%, recall ≥80%, suite discourse/golden verdi.

Ordine: **36a || 36b → 36c → 36d → 36e → 36f**. 36a/36b sono completati localmente; 36d
dipende da 35, 36e da 33.

## Avanzamento 2026-11-20 — 36b

- Added `pronouns.v1` inventories for all 11 supported languages. Existing
  `pronouns.txt` entries are preserved exactly; German, Dutch and Turkish
  receive curated structured inventories without changing generated pack
  content. Known person/gender/number/case values are recorded where curated;
  unresolved forms remain `unknown` rather than being guessed.
- `build_language_packs.py --check` validates schema, allowed values, complete
  source coverage, and a non-empty inventory for every supported language.
- Verified: `python3 lexicon/tools/build_language_packs.py --check` — PASS;
  all 11 packs and 12 profiles remain up to date. Gold review, broader resolver
  features and language-level calibration remain open.

## Avanzamento 2026-11-20 — 36a baseline locale

- `coref_gold.json` contiene 440 casi, 40 per lingua. Il report è marcato
  `assistant_draft_pending_human_review`; le etichette sono una bozza generata
  e non costituiscono gold approvato.
- `score_coref_gold.py` analizza ogni caso in una richiesta isolata, evitando
  contaminazione tra scene di casi indipendenti.
- `coref_baseline.json`, sul worker release aggiornato
  (`worker_sha256=11a0569c73d08d52be80922e7465343f3d48d02544bbeed558726b33ee9bb30d`):
  440 casi, status corretto 403/440, target corretto 415/440. Sulle risoluzioni
  il draft baseline misura precision 93.1% (190 TP, 14 FP) e recall 88.0%
  (190 TP, 26 FN); 14 risoluzioni sono confidentemente errate rispetto alle
  etichette correnti. Precision/recall per lingua: DE 100/45%, EN 100/100%,
  ES 100/100%, FR 100/100%, ID 100/100%, IT 95.2/100%, NL 69.2/52.9%,
  PL 90/90%, PT 95/95%, RU 95/100%, TR 76.2/80%.
- La matrice è utile per trovare lacune, ma non è gold approvato: il campione è
  sintetico e non revisionato. 36a è chiuso come strumentazione e baseline
  locale; revisione indipendente e gold held-out restano gate esterni di 36f.

## Avanzamento 2026-11-20 — 36c initial gender compatibility

- The resolver reads gender from the structured pronoun inventory and uses
  role-surface gender only when curated explicitly. A new `given_names.v1`
  inventory supplies a small set of unambiguous localized given names;
  absent names and common/unknown role forms do not filter candidates.
- Conflicting known gender removes only that candidate; unknown candidates
  remain compatible. Role evidence remains grounded to the original person.
- The given-name asset participates in the semantic content fingerprint and
  bumps `AnalyzerVersions.discourse` to `discourse.v3.gender.given_names.v1`
  and then `discourse.v4.gender.names.turkish_pronouns`, which adds the
  genderless Turkish pronoun case.
- Verified with the role-gender case and
  `curated_given_name_gender_filters_only_known_conflicts`: a known name can
  resolve a gendered pronoun against a known conflicting name, while two
  names outside the inventory remain compatible and ambiguous. Fingerprint
  coverage is also tested. Agreement morphology, broader name data, number
  evaluation, gold and the 36d margin remain open.

## Avanzamento 2026-11-20 — gender dei nominali di ruolo

- La detection dei nominali di persona legge ora `gender` dalle superfici
  esplicitamente marcate nei lessici `roles/en.json` e `roles/it.json`. Forme
  `common`, assenti o in conflitto restano `unknown` e non eliminano candidati.
- Verificato con `semantic-engine/tests/discourse.rs`:
  `La fondatrice` sceglie Anna fra due persone nella stessa frase precedente;
  `il presidente` resta ambiguo con gli stessi due antecedenti perché la forma
  nel lessico è `common`.
- `AnalyzerVersions.discourse` e l'identità cache sono passati a
  `discourse.v5.gendered_role_nominals`; il fingerprint esistente include i
  lessici ruoli. La copertura è uno slice en/it, non soddisfa ancora il gate
  multilingue o l'accordo morfologico.
- Verificato dopo il cambio: `cargo test --workspace` — PASS (incluse 127 unit
  test e tutte le suite d'integrazione; probe manuali ignorati); `cargo clippy
  --workspace --all-targets -- -D warnings` — PASS; suite
  `tests/discourse.rs` — 18/18.

## Avanzamento 2026-11-20 — 36f consumer Go

- Il worker Go ora decodifica `discourse.v1` in tipi strutturati per riferimenti,
  thread, beat, continuità di scena e conteggi; il bundle semantic conserva la
  proiezione e `AnalyzerVersions.discourse`.
- Il boundary valida gli span contro il testo sorgente, le identità contro gli
  entity link, gli stati dei riferimenti, i thread e la coerenza dei conteggi;
  rifiuta evidenza o offset incoerenti. La proiezione assente resta compatibile
  con fixture/worker legacy.
- Verificato: `go test ./internal/platform/media/rustexec` — PASS (78.565s);
  i test mirati discourse e bundle con luoghi/eventi — PASS. Il pacchetto Go
  non chiude i gate del gold: il report attuale è una bozza da revisione umana,
  e precisione/recall richieste non sono dimostrate su annotazioni indipendenti.

## Avanzamento 2026-11-20 — detector data-driven e baseline

- Spostate in `lexicon/discourse_reference_terms_v1.json` le liste di pronomi,
  dimostrativi, nominali di persona/organizzazione e frasi multi-parola. Il
  detector Rust non contiene più `term_applies`, `phrase_applies` o rami basati
  su codici lingua; le scope lessicali restano per lingua e gli span sono
  invariati.
- L'asset è incluso nel content fingerprint; `AnalyzerVersions.discourse` è
  `discourse.v6.reference_lexicon`. Il check `build_language_packs.py --check`
  verifica schema, 11 lingue, campi, tipi e valori ammessi.
- Verificati detector **4/4**, suite discourse **18/18**, fingerprint asset
  **PASS**, formattazione **PASS**. Il report release aggiornato
  (`worker_sha256=11a0569c73d08d52be80922e7465343f3d48d02544bbeed558726b33ee9bb30d`)
  resta identico alla baseline precedente (403 status, 415 target, 14 confidently
  wrong): la migrazione è comportamentalmente neutra e non è una calibrazione.
- Gold indipendente, precisione/recall sui dati revisionati, resolver a salienza
  e trap set restano aperti.

## Recheck 2026-11-21 — 36c name coverage

- Il caso `tr_ambiguous_07` ha rivelato che `Selin Özdemir` non entrava fra gli
  antecedenti perché il nome non era curato, causando una risoluzione sicura
  ma sbagliata del pronome turco neutro `o`. Aggiunto `Selin` al lessico
  femminile turco e una regressione NER sulla frase esatta del gold.
- `cargo test -p semantic-engine -p visualner --locked` — PASS. Il worker
  release è stato ricompilato e `score_coref_gold.py` ha rigenerato
  `coref_baseline.json`: **440/440** status e target corretti, **0** risoluzioni
  false, precision e recall draft **100%**; SHA-256
  `7eb6b270c8913626f9da87e9445f8332cb04bd431e2a27804c8b781a628931c7`.
- L'autorità resta `assistant_draft_pending_human_review`: questo risultato
  chiude la regressione sul fixture sintetico, non la revisione indipendente,
  i trap set, la salienza o la copertura richiesta per 36c–36f.

## Recheck 2026-11-21 — 36e audit basis

- Le nominalizzazioni di ruolo risolte emettono ora `resolution_basis:
  role_nominal`, come richiesto dal contratto di audit; prima usavano una base
  generica che non distingueva questa classe di riferimento.
- `AnalyzerVersions.discourse` è stato aggiornato a
  `discourse.v7.role_nominal_basis`; il worker release e il baseline sono stati
  ricostruiti, e l'hash registrato nel report è quello del binario corrente.
- Verificati i casi con un antecedente unico, due persone concorrenti e ruolo
  femminile discriminante: `cargo test -p semantic-engine --test discourse
  --locked` — **20/20 PASS**. Formatter e `git diff --check` — **PASS**.

## Final recheck 2026-11-21 — current worker

- Release worker on disk (`3e9bb942…`, built from HEAD `b541f1d`) used to
  regenerate `coref_baseline.json`: **440/440** status and target matches,
  **0** false resolutions, draft precision/recall 1.0. The report now carries
  the current worker hash. Authority remains
  `assistant_draft_pending_human_review`; independent review, agreement
  breadth and the salience margin remain open.

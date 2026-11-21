# TICKET-33 — people have no role, affiliation, family tie or age

- Priority: P2
- Status: PARTIAL (verified 2026-11-21, HEAD `06ea22c`; suite 355/0/4)
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
- [x] Output deterministic; `AnalyzerVersions.roles` bumped; Go copy updated.

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
dipende da altri ticket. *(Seed fatto con 33a il 2026-11-20; restano le nove
lingue e il generatore agganciato alla CI.)*
2. **Apposizione e genitivo** (`Mario Rossi, CEO di Acme`) con test; se il
   legame persona↔organizzazione è sbagliato, il ruolo non si emette. *(Fatto
   con 33b: 5 test, span byte-exact, scope solo se grounded.)*
3. **Validità temporale** solo dopo TICKET-31 (per le forme relative). *(Slice
   iniziale 33c: solo la prima data assoluta risolta nella stessa frase; nessun
   intervallo o data di fine inferita.)*
4. **Parentele/età** dopo i legami di lavoro. *(Slice iniziale 33d: relazioni
   direzionali e `age_facts` numerici in 11 lingue; restano le forme flesse
   estese e il gold.)*

Gate: ruolo corretto ≥ 90 % sul gold, nessun ruolo inventato quando
l'apposizione è ambigua, golden dei fatti entità invariati per gli script
senza ruoli.

## Verifica 2026-11-20

Ricontrollato sul working tree corrente (`2bbeca1` più le modifiche non
committate).

- **Fatto (33a seed):** `lexicon/roles/{en,it,es}.json` contengono 40
  codici ciascuno, con scope e marcatori di tempo; `python3
  lexicon/tools/check_roles.py` — **PASS** (`40 codes each in en, it, es; valid`).
  Il checker è ora richiamato anche da `build_language_packs.py --check`, quindi
  il controllo lessico ruoli fa parte del gate canonico CI dei pack.
- **33b verificato (runtime iniziale):**
  `semantic-core/src/manifest.rs` definisce `PersonRole`, `RoleRank`,
  `RoleTense`, `RoleStatus` e il campo additivo `SceneManifest.person_roles`;
  `AnalyzerVersions.roles` è `person_roles.v3.es`; `semantic-engine/src/roles.rs`
  contiene `extract()` per apposizione clause-local, cablato in
  `semantic-engine/src/lib.rs`; `lexicon/roles/{en,it,es}.json` sono nel
  fingerprint del contenuto. I test runtime dedicati passano; il binding degli anni espliciti nella stessa
  frase è stato aggiunto in questa passata e viene verificato separatamente.
- **33d — slice età implementato:** `SceneManifest.age_facts` porta età numeriche
  esplicite collegate a PERSON nella stessa frase; la derivazione dell'anno di
  nascita richiede un anno assoluto esplicito e una costruzione datata di età.
  I collettivi come “both aged 30” vengono ignorati quando l'attribuzione
  individuale non è sicura. `AnalyzerVersions.age_facts` separa le chiavi cache.
  Due test unitari e un test protocollo coprono lo slice.
- **33d — slice relazioni implementato:** `SceneManifest.person_relations`
  contiene relazioni esplicite direzionali family/work in 11 lingue;
  richiede due PERSON grounded, mantiene negazione/incertezza e usa span/evidence
  della frase. Il lessico `family_relations.v1` è incluso nel fingerprint e
  `AnalyzerVersions.person_relations` invalida la cache. Tre test unitari
  verificano direzione, grounding, negazione e offset Unicode; un test
  protocollo copre l'output nel manifest.
- Il lessico ora ha seed family/work per tutte le 11 lingue, ma copre solo le
  forme enumerate e non ha ancora gold multilingue. Forme flesse/ambigue,
  categorie più ampie, revisione gold ed età word-number restano aperte.

## Verifica workspace 2026-11-20

- `cargo test --workspace` e `cargo clippy --workspace --all-targets -- -D
  warnings` — **PASS**.
- `python3 lexicon/tools/build_language_packs.py --check` valida ora anche il
  lessico ruoli en/it/es oltre a verificare pack e snapshot Go — **PASS**.
- Restano i ruoli nelle altre 8 lingue, i gate linguistici pl/ru/tr/de, il gold
  indipendente e il gold multilingue; il ticket resta **PARTIAL**.
- PipelineGen ora decodifica e proietta ruoli, età e relazioni, validando span,
  identity scope e stato; le versioni analyzer sono incluse nel bundle.
  `go test ./internal/platform/media/rustexec -count=1` sul release aggiornato — **PASS**, inclusi i casi di
  grounding e rifiuto di relazioni con identità mancanti.
- Il prerequisito TICKET-31 è CLOSED; TICKET-35 è PARTIAL. Stato: **PARTIAL
  (33a, slice runtime 33b/33c e slice 33d età/relazioni)**.

## Verifica età scritte in lettere 2026-11-21

- La proiezione delle età ora gestisce anche il caso in cui l'estrattore di
  numeri includa l'unità nello span (`thirty years`, `trente ans`): richiede
  comunque un predicato d'età localizzato o un suffisso esplicito, così una
  durata come `worked thirty years of experience` non diventa un'età.
- Un test end-to-end sul manifest verifica `age=30`, span che inizia sul numero
  scritto in lettere e nessun anno di nascita derivato, in tutte le 11 lingue.
  Verificato: `env -u RUST_ICU_MAJOR_VERSION_NUMBER cargo test -p
  semantic-engine --test protocol_validation
  spelled_out_person_ages_are_projected_in_all_supported_languages --locked`
  — **PASS (1 test, 11 lingue)**. Inoltre `cargo test -p semantic-engine
  --locked` e `cargo clippy -p semantic-engine --all-targets --locked -- -D
  warnings` — **PASS**. La suite usa ICU 72 del workspace; impostare
  la variabile a 70 forza simboli ICU incompatibili con le librerie installate.
- Verifica dedicata aggiunta anche per non confondere una durata lavorativa con
  un'età.

Il ticket resta **PARTIAL**: mancano gold indipendente, copertura completa di
ruoli/inflection e una revisione linguistica esterna; la nuova prova chiude
solo la lacuna runtime delle età in parole.

## Verifica lessico spagnolo 2026-11-21

- Aggiunto `lexicon/roles/es.json` con i 40 codici condivisi e superfici
  spagnole; `AnalyzerVersions.roles` è passato a `person_roles.v3.es` e il
  lessico entra nel fingerprint del contenuto.
- `check_roles.py` e `build_language_packs.py --check` validano ora en/it/es.
- Test unitario e test protocollo verificano `alcaldesa` con genere femminile,
  codice `mayor`, evidenza e span byte-exact. Entrambi passano.
- Le restanti otto lingue, la revisione linguistica indipendente e il gold
  d'accettazione restano necessari; il ticket rimane **PARTIAL**.

## Copertura lessicale runtime 11 lingue — 2026-11-21

- Aggiunti i 40 codici ruolo anche in DE/FR/ID/NL/PL/PT/RU/TR, con superfici
  localizzate e marcatori di tempo/state. Checker e gate dei language pack
  controllano ora tutti gli undici file; l'estrattore Rust e il resolver di
  gender discourse li caricano tutti. Gli undici file entrano nel fingerprint;
  `AnalyzerVersions.roles` è `person_roles.v4.all_11`.
- Nove test di estrazione coprono le lingue aggiunte con span byte-exact,
  incluse flessioni di caso dedicate in DE/PL/RU/TR; forme femminili in
  DE/PL/RU sono verificate esplicitamente. Il test turco
  conferma `gender=unknown` per `başkan`, forma che non codifica il genere.
  Test ruoli **8/8**, protocollo semantic-engine **16/16**, checker ruoli e
  `build_language_packs.py --check` — **PASS**. La suite completa
  `cargo test -p semantic-engine --locked` passa (155 unit test più tutte le
  integrazioni); Clippy e formatter passano. Il test unitario della gender
  inventory discourse in de/pl/ru/tr passa. La suite Go `rustexec` col worker
  release aggiornato passa (78.5 s); worker SHA-256
  `3e9bb9425e82f699761b8a8a124d16d00d5669060fbd825c2251d2f101e6bd79`.
- Il ticket rimane **PARTIAL** finché le forme flesse complete e il gold
  indipendente richiesti dall'accettazione non saranno verificati.

- Verifica completa aggiornata: `env -u RUST_ICU_MAJOR_VERSION_NUMBER cargo
  test -p semantic-engine --locked` — **PASS** (suite completa); Clippy con
  `-D warnings`, `git diff --check`, checker ruoli e pack — **PASS**.

## Regressione age cue italiano — 2026-11-21

- Rimosso `di N` come age cue autonomo: generava età spurie da normali conteggi
  come “due filoni” e “cinque consulenze”. Restano i suffissi d'età e le frasi
  esplicite “all'età di N” / “all’eta di N”. Due regressioni Rust mirate passano.
- La proiezione Go accetta gli span spellout con unità incluse, mantenendo
  grounding dello span, identity valida, range 0–120 e contesto d'età.
- Suite `go test ./internal/platform/media/rustexec -count=1` con percorso
  assoluto al nuovo worker release — **PASS** (85 s), incluse le tre fixture
  Garlasco/Sempio; rimane necessaria la revisione gold indipendente.

## Piano di chiusura e split (2026-11-20)

Il lessico ruoli è il pezzo indipendente; l'emissione dipende dal link persona
di 35 solo per la precisione, non per il formato (si possono usare i local id).
Split in 5 parti.

### 33a — Lessico ruoli (M, parte subito)

- `lexicon/roles/<lang>.json` + generatore/check in
  `build_language_packs.py --check` (o `build_roles.py` dedicato).
- Schema: `role_code` chiuso (~80), superfici con genere/numero/caso, `scope`
  atteso (`org|place|none`), marcatori di tempo (`ex`, `former`, `ehemalig`,
  `neo`, `acting`, `ff.`).
- Seed it/en, poi le 11 lingue; test dedicati per pl/ru/tr/de.
- Fatto quando: ogni codice ha almeno una superficie per lingua e il check
  rigenera i pack senza drift.

### 33b — Tipi, manifest e apposizioni (M)

- `semantic-core/src/roles.rs`: `PersonRole`, `RoleRank`, `RoleTense`.
  `SceneManifest.person_roles` additivo.
- Pattern clause-local: `Name, ROLE di ORG`, `ROLE di ORG Name`, genitivo
  (`ORG's CEO`), `è stato nominato`/`was appointed` da `relation_cues.json`.
- Un ruolo si emette solo se la superficie è nel lessico come parola intera e
  c'è una PERSON nell'apposizione; il ruolo senza scope è ammesso; lo scope
  ORG solo se grounded nella stessa clause. Titoli generici (`il presidente`)
  senza persona → nessun ruolo.
- Fatto quando: apposizione it/en corretta, nessun ruolo inventato su 20
  negativi, span byte-exact.

### 33c — Tempo e stato (S, dopo 33b)

- `dal 2015`, `since 2015`, `fino al 2019`, `until 2019` → `temporal`; usare
  TICKET-31 per le forme relative (già CLOSED).
- `ex`/`former` → `tense: former`; `nominato`/`appointed`/`resigned` →
  `appointed`/`resigned`; negazione/incertezza da `relation_cues.json`.
- Nessuna data di fine inferita.
- Fatto quando: tense accuracy ≥90% sul gold e nessuna data inventata.

### 33d — Relazioni e età (M, dopo 33b, in parallelo a 33c)

- `lexicon/family_relations.json` con direzione (`padre di` →
  `parent_of(from=father,to=child)`) in 11 lingue; `person_relations`.
- Età: `a 45 anni`, `45-year-old`, `45enne`, `im Alter von 45`; `age_facts`;
  `derived_birth_year` solo con data assoluta esplicita nella stessa frase.
- Test per forme gendered/inflected.
- Fatto quando: direzione 100% sul gold e nessuna età derivata da “oggi”.

### 33e — Gold e chiusura (L)

- ≥30 ruoli e ≥15 relazioni per lingua; ≥20 negativi (titolo generico, titolo
  dentro un nome di organizzazione, cognome = ruolo, nomina negata o
  ipotetica).
- Metriche: role_code ≥90%, scope precision ≥95%, tense ≥90%, direzione
  100%, 0 falsi positivi.
- Copia Go e `AnalyzerVersions.roles` aggiornati; lo stato passa a CLOSED solo
  al superamento dei gate gold/multilingue.
- Fatto quando: tutti i gate + suite Rust/Go verdi.

Ordine: **33a → 33b → (33c || 33d) → 33e**. 33a parte subito; 33b può partire
senza 35 usando i local id, ma il gate di precisione vuole 35 chiuso.

## Avanzamento 2026-11-20 — seed 33a

- Added `lexicon/roles/{en,it}.json` with the initial 40 language-independent
  role codes, scope classes, inflected gender forms where curated, and
  former/acting/appointment/resignation markers.
- Added `lexicon/tools/check_roles.py` to validate role coverage and data
  shape. Verified: `python3 lexicon/tools/check_roles.py` — PASS.
- This is the planned English/Italian role seed only. Broader role-language
  coverage and gold remain open.
- 33b now adds `PersonRole`, `RoleRank`, `RoleTense`, and `RoleStatus`, the
  additive `SceneManifest.person_roles` projection and `AnalyzerVersions.roles`.
  The extractor requires a grounded PERSON near an exact title span in the
  same sentence, attaches scope only to a grounded same-sentence entity, and
  abstains on tied candidate entities. Role lexicons are included in the
  content fingerprint.
- Verified: `cargo test -p semantic-engine --lib roles::tests` — **4 passed**
  (generic-title negative, English apposition, Italian former feminine title,
  UTF-8 byte offsets); full `cargo test -p semantic-engine --tests` passed
  before the final additional role cases. Go projection and gold remain open.


## Avanzamento 2026-11-20 — binding temporale iniziale (33c)

- `roles::extract` riceve ora i fatti data già grounded dal parser canonico e
  associa al ruolo la prima data assoluta risolta della stessa frase; le date
  derivate o unresolved non vengono usate. Nessun intervallo o estremo viene
  inferito.
- Test: `role_binds_only_an_explicit_date_in_its_sentence` verifica anno `2018`.

## Avanzamento 2026-11-20 — 33d initial family/work and age slices

- Added numeric `age_facts` for recognized explicit age phrases, grounded to a
  unique nearby PERSON; derived birth year requires a matching absolute date
  and explicit dated-age wording in the same sentence. Collective ages abstain.
- Date-to-age linking now recognizes explicit constructions for all 11
  supported languages (for example, English `was 30 years old`, Italian
  `aveva 30 anni`, and Russian `было 30 лет`). The extraction version is
  `person_age_facts.v2.localized_dated_age`; age phrases still need a grounded
  resolved year and a single nearby person before deriving a birth year.
- Added directional `person_relations` for enumerated family/work forms in all
  11 languages. Both people must be grounded in the same sentence. Negation and
  uncertainty status are carried, evidence is source-grounded, and matching
  preserves Unicode byte offsets. The lexical table is fingerprinted and has
  its own analyzer version.
- Added common instrumental case forms to the Polish and Russian relation
  inventory (`ojcem`, `matką`, `отцом`, `матерью`, and related family/work
  markers). Tests verify grounded direction and marker byte spans in Polish
  and Russian copular clauses.
- Verified: age — **3 unit + 1 protocol tests**; relations — **4 unit + 1
  protocol tests**; content fingerprint, `cargo clippy -p semantic-engine
  --all-targets -- -D warnings`, formatting and diff checks pass. Gold, broad
  inflection coverage and Go projection remain open.

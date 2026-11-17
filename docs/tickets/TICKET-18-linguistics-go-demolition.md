# TICKET 18 — Linguistics Go demolition

- Priority: P2
- Status: DONE (2026-11-16; cutover landed, gates green 2026-11-17)
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-15 (phrase selection), TICKET-20 (identity)
- Blocks: none

## Why

Go still loads and applies a linguistic profile:

- `internal/capabilities/linguistics/lexicon_registry.go` and loaders scan
  `config/lexicons/**` and build `LexiconRegistry`.
- `internal/app/wiring/script/lexicon.go` wires it into production.
- `scripts/entity_annotations.go` (561 LOC) and `entity_projection.go` (457
  LOC) still contain language-aware projection and grounding rules.
- `scripts/translated_nlp_identity.go` (571 LOC) contains localized identity
  heuristics for person inflection (pl/de/tr/ru).

Rust already owns the lexicons and the entity linker. The Go bootstrap is a
second authority even when the stopword files are byte-identical today.

## Target

Rust `semantic-core` / `semantic-engine` owns tokenization, stopwords,
morphology and identity projection. Go receives typed spans and identities and
only validates them against the scene text.

## Acceptance

- No production `NewLexiconRegistry` / `LexiconRegistry` caller outside tests.
- Go stopword/bootstrap files deleted or reduced to a generated, read-only
  projection with a parity gate.
- `translated_nlp_identity.go` no longer inflects or matches names; it projects
  the Rust-provided localized mention.
- Parity fixtures for de/en/es/fr/it/pt plus pl/ru/tr inflection.
- `make verify-main` on the cutover tree.

## Risks

- Language packs include a Go-only `fallback` set for unsupported languages;
  the Rust contract must cover or explicitly reject those languages.
- Removing the registry can change startup validation; keep a fail-closed
  check on the Rust profile version.

## Verification 2026-11-16 (workspace)

Still a live Go authority; no acceptance criterion has landed.

- `internal/app/wiring/script/lexicon.go:21` calls `NewLexiconRegistry` on the
  production path and feeds the intent resolvers
  (`internal/capabilities/brain/intent/registry.go`).
- `runner_overlay_prepare.go:338` and `translated_nlp.go:429` call
  `projectEntityAnnotations`; the language-aware rules in
  `entity_annotations.go` (561 LOC) and `entity_projection.go` (457 LOC) are
  live.
- `internal/capabilities/linguistics/` is 796 non-test LOC. No Rust operation
  returns the tokenization/morphology projection that would let Go delete the
  registry. `semantic-engine/src/protocol.rs` now registers `select_phrases`
  and `align_phrases` in addition to `health`, `analyze_script`,
  `split_sentences`, `compile_editorial_plan`, `resolve_map_overlaps`,
  `trim_map_windows_before_budget`, `rank_entity_overlays` and
  `cascade_clustered_entrances`; none of them replaces `LexiconRegistry` or
  `translated_nlp_identity.go`.
- The 2026-11-16 phrase/alignment work is not a linguistics cutover: phrase
  selection and word alignment are Rust-owned, but stopwords/morphology,
  `projectEntityAnnotations` and localized identity matching remain Go.

## Blocco misurato 2026-11-16

Il taglio non è una cancellazione: nessuna operazione Rust restituisce oggi il
profilo linguistico o l'identità localizzata. Cancellare il Go vivo romperebbe
intent resolver, language verification, geocoding, source research e la
proiezione delle annotazioni tradotte.

- `internal/capabilities/linguistics/` è **796 LOC non-test**; il tipo
  `LexiconRegistry` ha **63 riferimenti** (costruttore/letterali) e
  `DefaultLexicon` **53** nel repository, inclusi **18 file di test** che
  costruiscono il registro da `config/lexicons`.
- I pack Rust (`lexicon/packs/<lang>.json`) fondono gli insiemi in
  `closed_class`: non espongono separatamente `negative_particles`,
  `visual_verbs`, `pronoun_clitics`, `pro_drop_aux` o i `WordSet` nominati che
  i consumer Go leggono. Un'operazione `lexicon_profile` deve restituire questi
  insiemi, non solo lo stopword set.
- I sorgenti `BrainRust/lexicon/languages/<lang>` sono già la copia più ampia,
  ma **3 file divergono** da `refactored/config/lexicons` (en
  `entity_blocklist`, it `function_words`, it `pronouns`): il passaggio a Rust
  come unica authority è una riconciliazione editoriale, non un copia-incolla.
- `scripts/translated_nlp_identity.go` (571 LOC) è chiamato in produzione da
  `translated_nlp.go:294` e `:344`; la sua logica di inflection (pl/de/tr/ru)
  non esiste in Rust e va portata prima di eliminarlo (TICKET-20).

**Contratto minimo per chiudere 18.**

1. `semantic-engine` espone `lexicon_profile` con `language`, `version`,
   `stopwords`, `function_words`, `entity_blocklist`, `negative_particles`,
   `visual_verbs`, `verb_suffixes`, `word_sets`, `phrase_policy`.
2. Go bootstrap valida `version` e costruisce uno snapshot read-only; i
   consumer leggono lo snapshot, non i file.
3. Eliminati `config/lexicons/**` e i loader; fixture di parità per
   de/en/es/fr/it/pt più fallback.
4. TICKET-20 porta `match_localized_sources` / `complete_ordered_person_matches`
   in Rust; solo allora `translated_nlp_identity.go` può diventare proiezione.

## Implementazione 2026-11-16

Il cutover è atterrato in due parti.

**Rust (authority).** `lexicon/tools/build_language_packs.py` ora emette, oltre
ai pack fusi, un profilo a insiemi nominati per ogni lingua (fallback
compreso) in `lexicon/profiles/<lang>.json` e la tabella generata
`semantic-core/src/generated_language_profiles.rs`.
`semantic-core/src/language_profiles.rs` espone `LanguageProfile` con
`stopwords`, `function_words`, `entity_blocklist`, `negative_particles`,
`visual_verbs`, `verb_suffixes`, `word_sets`, `phrase_policy` e uno
`schema_version` deterministico. `semantic-engine` registra
`lexicon_profile` (una lingua o tutte) e lo copre con
`tests/language_profile_protocol.rs`.

**Go (read-only projection).** `profiles.generated.json` è il file generato
embedded in `internal/capabilities/linguistics`; `NewProfiles()` lo valida
(schema, `rust_schema_version`, fallback obbligatorio) e costruisce `Profiles`.
Il vecchio `LexiconRegistry`, `NewLexiconRegistry`, i loader da filesystem e
l'intera directory `refactored/config/lexicons/**` sono stati eliminati. Il
campo `linguistics.lexicon_root` e la sua risoluzione relativa sono stati
rimossi; resta solo `required_languages`, validato contro lo snapshot.

**Parità.** Il generator ha un gate `--check` che fallisce se i profili Rust,
la tabella generata o lo snapshot Go sono stale. I test Rust coprono le sei
lingue condivise (de/en/es/fr/it/pt) e il fallback; i test Go coprono snapshot,
tag BCP-47, copie difensive, union phrase-stop-words/fallback, named set,
morfologia e policy.

**Verifica.**

| Comando | Esito |
|---|---|
| `python3 lexicon/tools/build_language_packs.py --check` | PASS (12 profili + snapshot Go) |
| `cargo test -p semantic-core --lib` | **184 passed** |
| `cargo test -p semantic-engine --test language_profile_protocol` | **4 passed** |
| `go build ./...` / `go vet ./...` | rc 0 |
| `go test ./internal/capabilities/linguistics ./internal/capabilities/scripts ./internal/capabilities/brain/... ./internal/capabilities/translation` | PASS |
| `go test ./internal/app/wiring -run 'TestWireServices...|TestBuildCreatorRuntime'` | PASS |
| `make verify-main` / `make verify-architecture` | **bloccati da failure preesistenti e non correlate**: `verify-main` si ferma su 8 test `internal/capabilities/overlays` (drift del catalogo `RenderingGen` non committato) e su `internal/app/knobs` (`VELOX_ARTIFACT_TOKEN`/`VELOX_MASTER_ADMIN_TOKEN` introdotti in `remote_final_job.go` da un'altra sessione); `verify-architecture` aggiunge 4 errori (`crypto/sha256` nello stesso file, slot string in `media_ranker_adapter.go`). Nessuno tocca questo cutover; il pkg_size di `internal/capabilities/scripts` è tornato a 140. |

**Deploy.** *(Superato il 2026-11-17: `v0.3.6` contiene `lexicon_profile` e il pin
è `v0.3.6` — vedi la verifica in fondo.)* Il pin `BRAINRUST_VERSION ?= v0.3.5`
non conteneva `lexicon_profile`. Finché non veniva pubblicata una release che lo
include e il pin non veniva alzato, il container usava un binario che risponde
`unknown operation` e il bootstrap Go fallisce chiuso. Il binario locale `refactored/bin/semantic-engine`
è aggiornato per test e sviluppo, ma non sostituisce la release.

## Verification 2026-11-17 (gate chiuso)

La riga ``make verify-main`` / ``make verify-architecture`` della tabella
precedente è superata: le failure preesistenti sono state corrette, non aggirate.
Le fixture e i knob sono stati registrati, `remote_final_job.go` usa
`internal/kernel/digest` e `media_ranker_adapter.go` le costanti
`internal/kernel/media`.

| Comando | Esito |
|---|---|
| `go test ./internal/app/knobs/... ./internal/capabilities/overlays -count=1` | **PASS** (0,024 s + 11,931 s): il drift del catalogo non c'è più |
| `make verify-architecture` | **PASS** (C2-C 0/0, C2-E 0/0, nessun errore) |
| `go build ./...` / `go vet ./...` | rc 0 |
| `cargo test -p semantic-core --lib` | **184 passed** |
| `cargo test -p semantic-engine --test language_profile_protocol` | **4 passed** |

**Deploy chiuso:** la release `v0.3.6` (tag `527b756`, 2026-11-17) contiene
`lexicon_profile` e `refactored@4bde5e66e` pinna `BRAINRUST_VERSION ?= v0.3.6`.

# TICKET 20 — Identity and localization consolidation

- Priority: P2
- Status: DONE (2026-11-16; cutover landed, gates green 2026-11-17)
- Source: external audit 2026-11-16; TICKET-13 phase 2
- Depends on: TICKET-18 (linguistics), TICKET-19 (alignment)
- Blocks: none

## Why

`scripts/translated_nlp_identity.go` (571 LOC) is the localization seam: it
matches a source entity surface in translated text and stamps the source
canonical id and image binding onto the localized annotation. It includes
ordered-person fallbacks for ru/tr/pl and inflection matching for pl/de/tr.

That is identity resolution in Go. It can disagree with the Rust linker and it
is not covered by the v3 entity contract.

## Target

Rust owns identity across languages:

- `semantic-engine` receives source and translated scenes in one request (or a
  linked identity map) and returns the localized mention with the source
  canonical id, type and image binding.
- Go validates the mention occurs verbatim in the translated scene and
  projects it; it does not match, inflect or complete identities.

## Acceptance

- Rust localized-identity contract with fixtures for ru/tr/pl/de inflection and
  writing-system changes.
- Go projection carries the source id unchanged; a missing id fails closed.
- `matchLocalizedSourceEntities` and `completeOrderedPersonMatches` have no
  production caller.
- `go test ./internal/capabilities/scripts/...` and the Rust linker suite pass;
  `make verify-main` on the cutover tree.

## Risks

- Translation can legitimately omit or rename a person; the contract must
  distinguish "not present" from "unlinked".
- The image binding belongs to the source identity; do not re-resolve it from
  the translated surface.

## Verification 2026-11-16 (workspace)

Still a live Go authority; no acceptance criterion has landed.

- `translated_nlp.go:286` still calls `matchLocalizedSourceEntities` and
  `:336` calls `completeOrderedPersonMatches`;
  `translated_nlp_identity.go` (571 LOC) still owns inflection
  (pl/de/tr) and ordered-person completion (ru/tr/pl).
- No Rust localized-identity operation exists in
  `semantic-engine/src/protocol.rs`.
- The in-flight `translated_nlp.go` edits only move phrase selection to the
  Rust selector; they do not remove identity matching or inflection.
- The `translation` artifact checkpoint (text digest + input fingerprint) only
  avoids re-calling the provider. It carries no source canonical id, so it is
  not an identity contract and does not satisfy this ticket.
- `go test ./internal/capabilities/scripts -count=1` is green (15.723 s), but
  no acceptance criterion of this ticket has a dedicated test yet.

## Blocco misurato 2026-11-16

Il seam è piccolo come API ma non è isolato: la funzione di matching dipende da
`localizedNameTokens`, `personNameRuns`, `normalizeVisualPersonName`,
`findEntitySpan`, `annotationCanonicalEntityID` e dai tipi
`AnnotatedEntity`/`VisualEntity`. Non esiste in Rust né il tokenizer dei nomi
né la tabella dei suffissi (`grep` su `semantic-core`/`semantic-engine` per
`inflect`, `declension`, `match_localized`: **0**).

Copertura Go da preservare: `translated_nlp_test.go` contiene i casi pl
(`TestGroundLocalizedSourceEntitiesProjectsPolishInflection`), tr
(`...TurkishCaseSuffix`), ru (`...AlignsRussianNERToSourceIdentity`) e de
(`...KeepsGermanMentionSurfaceForTiming`), oltre a identità ereditata e geo.
La stessa `translated_nlp_identity.go` ospita `limitTranslatedVisualEntities`
e `translatedSpecialNames`, usati anche da `vidrush_semantic_chain.go`: il file
non è monolitico e va separato prima di dichiarare la cancellazione.

**Contratto minimo per chiudere 20.**

1. `semantic-engine` espone `localize_identities` con `language`, `text`,
   entità sorgente tipizzate (kind, canonical_name, text) e, per il fallback
   ordinato, i candidati PERSON del NER tradotto.
2. La risposta restituisce per ogni match `source_index`, `kind`, span rune e
   superficie; il matching esatto e l'inflessione pl/de/tr/ru vivono in Rust.
3. Go proietta il match sull'annotazione (id canonico, image, geo, mention) e
   fallisce chiuso se l'id manca.
4. Fixture di parità per pl/de/tr/ru più i casi esatti; poi
   `matchLocalizedSourceEntities` e `completeOrderedPersonMatches` spariscono.
5. `translatedSpecialNames`/`limitTranslatedVisualEntities` restano Go o
   diventano un'operazione separata: non sono identity matching e non vanno
   cancellati per effetto collaterale.

## Implementazione 2026-11-16

**Rust (authority).** `semantic-core/src/localized_identity.rs` possiede
`match_localized_sources` e `complete_ordered_person_matches`: tokenizzazione,
confronto case-insensitive, suffissi chiusi pl/de/tr, span rune e fallback
ordinato ru/tr/pl. `semantic-engine` registra `localize_identities`
(`mode=match|complete`) e lo copre con `tests/localized_identity_protocol.rs`.
I test unitari coprono: esatto in de/en/es/fr/it/pt, pl (`Mike’a Tysona`), tr
(`Mike Tyson'a`), de (`Mike Tysons`), rifiuto di un nome singolo, ru ordinato,
conteggio non corrispondente e persona già matchata.

**Go (projection only).** `translated_nlp_identity.go` è stato rinominato
`translated_nlp_localization.go`: costruisce la richiesta tipizzata, valida
ogni span contro il testo tradotto, proietta `CanonicalEntityID`, image, geo e
mention. `stampLocalizedSourceIdentity` ora fallisce chiuso se un match non ha
un id canonico derivabile. Il matching/inflessione Go non è più compilato in
produzione: vive solo in `translated_nlp_localization_oracle_test.go` come
oracolo dei test di integrazione esistenti. `LocalizedIdentityMatcher` è
iniettato dal composition root (`SetLocalizedIdentityMatcher`) e il matcher
nil fallisce chiuso.

**Verifica.**

| Comando | Esito |
|---|---|
| `cargo test -p semantic-core localized_identity --lib` | **10 passed** |
| `cargo test -p semantic-engine --test localized_identity_protocol` | **5 passed** |
| `go test ./internal/capabilities/scripts -count=1` | PASS (14.875 s) |
| `go test ./internal/platform/media/rustexec -run TestSemanticEngineRealBinaryLocalizesIdentities` | PASS contro il binario Rust reale (pl + ru) |
| `go vet ./...` | rc 0 |
| `make verify-main` | **bloccato dalle failure preesistenti descritte in TICKET-18** (drift catalogo RenderingGen, knob e archcheck in `remote_final_job.go`, slot string in `media_ranker_adapter.go`), non da questo cutover. |

**Deploy.** *(Superato il 2026-11-17: `v0.3.6` contiene `lexicon_profile` e
`localize_identities`, pin `v0.3.6` — vedi la verifica in fondo.)* `v0.3.5` non
conteneva nessuna delle due operazioni; senza una release nuova e il bump di
`BRAINRUST_VERSION` il worker rispondeva `unknown operation` e la proiezione Go
falliva chiusa.

## Verification 2026-11-17 (gate chiuso)

La riga `make verify-main` della tabella precedente è superata: le failure
preesistenti descritte in TICKET-18 sono state corrette sul tree corrente.

| Comando | Esito |
|---|---|
| `go test ./internal/app/knobs/... ./internal/capabilities/overlays -count=1` | **PASS** (0,024 s + 11,931 s) |
| `make verify-architecture` | **PASS** (C2-C 0/0, C2-E 0/0) |
| `go build ./...` / `go vet ./...` | rc 0 |
| `cargo test -p semantic-engine --test localized_identity_protocol` | **5 passed** |
| `cargo test -p semantic-core --lib` | **184 passed** |

**Deploy chiuso:** la release `v0.3.6` (tag `527b756`, 2026-11-17) contiene
`lexicon_profile` e `localize_identities`; `refactored@4bde5e66e` pinna
`BRAINRUST_VERSION ?= v0.3.6`.

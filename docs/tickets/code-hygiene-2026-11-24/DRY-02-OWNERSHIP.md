# BR-DRY-02 — Ownership decision for the duplicated primitives

**Data:** 2026-11-24
**Stato:** decisione registrata; il gate workspace resta condizionato dal
workstream concorrente su `visualner`/`lexicon`.

## Regola

Una primitiva ha **un solo owner**. Una copia resta solo se ha un contratto
diverso, un confine di crate che non può essere attraversato senza un ciclo, o
un vincolo di parità Go documentato. Non si unificano implementazioni con
semantiche diverse solo perché hanno lo stesso nome.

## Tabella

| Primitiva | Owner | Copie e motivo |
|---|---|---|
| `split_sentences` | `semantic-core::sentences` | `phrase_impact::sentences` è una proiezione tipizzata (`Segment`) sull'owner, non una seconda implementazione. `semantic-engine::protocol::split_sentences` è un line-splitter NDJSON, non un sentence splitter. |
| `word_count` | `semantic-core::sentences::word_count` | `phrase_impact::scoring::word_count` e le due copie in `overlay-compiler` misurano budget editoriali locali; unificarle cambierebbe il confine di crate e il significato di "parola" del planner. Restano con nota nel codice. |
| `tokenize` | `semantic-core::phrase_select::tokenize` | `visualner::extractor::tokenizer` è UDPipe-aware e restituisce token con dipendenze; `script-qa` è un tokenizer di QA senza dipendenze; `dates/relative` è uno scanner di date. Contratti diversi, nessuna merge. |
| `is_acronym` | `semantic-core::entity_linker` | La copia in `visualner::extractor::location` è extractor-local e non può importare `semantic-core` senza invertire il confine già esistente. Resta con commento. |
| `locate_phrase` | `semantic-core::phrase_align` | Le copie in `overlay-compiler` lavorano su `TimingV2`/`ProjectionPlan`, non su `SpeechWord`; stessa nome, contratto diverso. Restano locali. |
| `overlaps` | `semantic-core::localized_identity` per gli span; `semantic-engine::consolidate::recipes` per gli intervalli di recipe | I due tipi di input sono diversi (`IdentityMatch` vs `(usize, usize)`); una funzione generica nasconderebbe la differenza. Restano separate. |
| `numeric.rs` (4 versioni) | `semantic-core::dates::numeric` per la normalizzazione condivisa | `visualner::extractor::numeric` è l'estrattore NER; `overlay-compiler::numeric` e `projection::numeric` sono projection-only/Go parity. Le differenze sono dichiarate nei rispettivi moduli. |

## `producers.rs` — clone policy

`semantic-engine/src/consolidate/producers.rs` contiene 37 `.clone()`. La
revisione del 2026-11-24 li classifica così:

- **Campi di contratto posseduti** (`ProductionCandidate`, `ReferenceNumber`,
  `ChartItem`): il candidato sopravvive alla scena sorgente, quindi il valore
  deve essere posseduto.
- **Chiavi di `HashSet`/`HashMap`**: `seen.insert(key.clone())` e
  `asset_key`/`dedup_key` richiedono due owned key perché il set e il candidato
  hanno vite diverse.
- **Evidenza/testo**: `proposal.evidence`, `proposal.suggested_visual`,
  `chart_meta` sono campi owned del contratto.

Nessun clone è un prestito che possa essere sostituito da `&str` senza cambiare
la firma pubblica o la durata del candidato. La politica è registrata qui e nel
commento di testa del file; una futura riduzione richiede un cambio di contratto,
non una rimozione meccanica.

## Verifica

- `rg -c '\.clone\(\)' semantic-engine/src/consolidate/producers.rs` → **37**.
- `cargo test -p semantic-engine --lib` → **181 passed**.
- `cargo clippy -p semantic-engine --lib --no-deps -- -D warnings` → **RC 0**.
- Il gate `cargo test --workspace` e il clippy workspace restano rossi per il
  workstream concorrente su `semantic-core/src/lexicon/tests.rs` (42 vs 40) e
  `visualner` (`ptr_arg`); non sono attribuibili a questa scheda.

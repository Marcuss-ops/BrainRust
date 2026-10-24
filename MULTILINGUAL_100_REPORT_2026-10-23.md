# BrainRust — benchmark multilingue 100/100

Data: 23 ottobre 2026
Repository: BrainRust, commit `8ec5cf9` + correzioni multilingui del 2026-10-23
Ambiente: Linux, ICU 70 (`RUST_ICU_MAJOR_VERSION_NUMBER=70`), 13 modelli UDPipe 2.5 (250 MB)

## Obiettivo e criterio di accettazione

Obiettivo: verificare se BrainRust regge i 100 copioni sintetici multilingue pubblicati in
`semantic-engine/fixtures/multilingual_100`, senza indebolire i check.

Criterio di accettazione del runner: `PASS` = nessun problema strutturale e nessuna review.
I problemi strutturali includono schema/spans/offset, query non grounded, cache replay.
Le review includono persone attese mancanti, anni non normalizzati, chart assente, bullet
assenti, LOCATION assente e ricerca visuale assente. Un `NEEDS_REVIEW` vale exit code 2,
quindi non è accettabile per il traguardo 100/100.

## Baseline prima delle correzioni

Il primo run ufficiale ha prodotto:

- **86 FAIL, 14 NEEDS_REVIEW, 0 PASS** (exit 1)
- **crash del worker** su `pt-05-school` e `pl-05-school`
- **65 casi** falliti a cascata con `Broken pipe` dopo il primo crash

Cause isolate:

1. **Panico UTF-8** in `semantic-core/src/dates.rs:435`: il cursore della scansione dei mesi
   avanzava usando la larghezza del carattere letto alla base sbagliata
   (`hay[found..]` invece di `hay[offset + found..]`), finendo a metà di un carattere
   accentato. Caso minimo riproducibile: `xfévrierxéfévrierx`.
2. **Query non grounded**: `multiple_entities` e `chart` suggerivano l'etichetta unita
   (`Sofia Rinaldi Marta Greco`, `41% 15%`), che non è una slice del testo. Il runner
   segnalava 154 violazioni nei 34 casi analizzati prima del crash.
3. **Leakage delle headline nel runner**: `make_scenes` univa i blocchi con uno spazio,
   cancellando il confine `\n\n` tra heading e corpo. `PADOVA Nel progetto...` diventava
   una PERSON. Il test delle headline era quindi invalido, non l'engine.
4. **Spagnolo e portoghese senza PERSON**: i modelli UDPipe restituiscono lemmi PROPN
   minuscoli (`Lucía` → `lucía`, portoghese → `_`), e la regola che richiedeva il lemma
   maiuscolo scartava tutti i nomi. Lo spagnolo restava anche senza LOCATION.
5. **Chart assente in 7 lingue** (es, pt, fr, nl, pl, ru, id): mancavano i cue di
   confronto usati dal corpus (`frente al`, `contra`, `contre`, `tegenover`, `wobec`,
   `против`, `dibandingkan dengan`).
6. **`micro_summaries` = 0** in tutti i casi: `entity_facts` è vuoto perché la proiezione
   richiede una FACILITY riconosciuta o un pattern legale; i pack legali esistono solo
   per italiano e inglese.

## Correzioni applicate

| Difetto | File | Correzione | Test |
|---|---|---|---|
| Panico UTF-8 | `semantic-core/src/dates.rs` | indice corretto `hay[offset + found..]` | `unbounded_month_inside_a_word_never_bisects_the_following_code_point` |
| Query non grounded | `semantic-engine/src/lib.rs` | una query per entità, niente etichetta unita; evidence come rung | `visual_query_suggestions_are_always_verbatim_source_slices` |
| Heading fuso col corpo | `semantic-engine/fixtures/multilingual_100/benchmark.py` | i blocchi si uniscono con `\n\n` | verifica su 100 casi (nessuna PERSON da headline) |
| PERSON/LOCATION es/pt/id | `lexicon/language_profiles.json`, `semantic-core/src/lexicon.rs`, `visualner/src/extractor.rs` | campo dati `lowercase_propn_lemmas`; PROPN vale come evidenza solo per i modelli che lowercasano i lemmi | `spanish_propn_with_lowercase_lemma_is_a_person_and_a_place`, `portuguese_propn_with_underscore_lemma_is_a_person_and_a_place` |
| Chart assenti | `lexicon/discourse_cues.json` | cue di confronto per es/pt/fr/nl/pl/ru/id | `multilingual_comparison_cues_are_recognized` |
| Cognomi olandesi | `lexicon/entity_gazetteer.json`, `visualner/src/extractor.rs`, profilo `nl` | tabella `name_particles` (`de`, `van`, `der`, ...), modello `dutch-alpino` | `dutch_surname_particle_stays_inside_the_person_name`, `a_place_word_as_a_given_name_stays_in_the_person` |
| Città russa assorbita | `lexicon/entity_gazetteer.json`, `visualner/src/extractor.rs` | città russe nel seed; un place noto non apre un nome di 3+ token | `a_known_city_before_two_names_stays_a_location` |
| `João Ribeiro` → LOCATION | `semantic-core/src/lexicon.rs` | alias di località sotto 50k abitanti non battono il nome proprio; nomi primari e seed restano | `minor_place_aliases_do_not_override_person_names` |

Nota: la regola "città + due nomi" non spezza `Milan van Dijk`, perché la particella
`van` segnala un cognome; `Milan` resta dentro la PERSON.

## Risultato finale

**100/100 PASS, 0 FAIL, 0 NEEDS_REVIEW**, con exit code 0 in ogni run linguistico.

| Lingua | Casi | PASS | Chart per caso | Review |
|---|---:|---:|---|---|
| Italiano | 10 | 10 | 2 | vuota |
| Inglese | 10 | 10 | 1 | vuota |
| Spagnolo | 10 | 10 | 1 | vuota |
| Portoghese | 10 | 10 | 1 | vuota |
| Francese | 10 | 10 | 1 | vuota |
| Tedesco | 10 | 10 | 1 | vuota |
| Olandese | 10 | 10 | 1 | vuota |
| Polacco | 10 | 10 | 1 | vuota |
| Russo | 10 | 10 | 1 | vuota |
| Indonesiano | 10 | 10 | 1 | vuota |

`micro_summaries` resta 0 in tutte le lingue (vedi limiti). Le persone attese, i cinque
anni normalizzati, i bullet, le LOCATION e le proposte `entity_search` sono presenti in
tutti i casi; la review è vuota.

## Verifiche eseguite

Modelli: 13 file UDPipe 2.5 (it, en, es, pt, fr, de, nl, pl, ru, id, tr, ja, zh), 250 MB.

Test (modelli installati via `VISUALNER_MODEL_DIR`):

- `cargo test -p semantic-core -p ndjson-stdio -p mediasampler -p pipelinegen-muscles`:
  333 test passati, 0 falliti
- `cargo test -p visualner`: 83 test passati, 0 falliti
- `cargo test -p semantic-engine`: 82 test passati, 0 falliti
- `cargo clippy --workspace --all-targets -- -D warnings`: pulito
- `cargo fmt --all -- --check`: pulito

Il comando aggregato `cargo test --workspace` è stato spezzato in blocchi per il limite di
600 s per comando; tutti i pacchetti del workspace sono stati eseguiti e passano.

Benchmark: il runner completo richiede circa 40 minuti. Per rispettare il limite di 600 s
per comando è stato eseguito per lingua con lo stesso worker persistente e la stessa
funzione `inspect`; it/en/es/pt/fr/de sono stati eseguiti prima dell'ultima guardia
olandese, che non tocca i loro percorsi (`name_particles` contiene solo `nl`), e
nl/pl/ru/id con il binario finale. Russo e indonesiano sono stati verificati in blocchi
da 4+4+2 casi per lo stesso limite di tempo, senza modificare i check.

## Limiti residui

1. **Micro-summary assenti (0/100)**. Non è un criterio PASS del runner, ma il README
   chiede di riportarle. `entity_facts` richiede una FACILITY (oggi riconosce solo
   `Stadio/Stadium/Arena` + nome maiuscolo) oppure un pattern legale; i pack legali
   esistono solo per it/en. Il corpus usa `stadio comunale` minuscolo e non contiene
   pattern legali. Per coprire davvero le micro-summary servono lessico facility e pack
   legali per le altre lingue.
2. **Corpus sintetico**. I 100 testi sono generati da template, non un gold set annotato
   a mano. Il 100/100 certifica struttura, grounding e candidate check, non precisione e
   recall semantiche reali.
3. **Olandese model-only**. È stato aggiunto profilo `nl` + modello `dutch-alpino`; non
   esiste ancora `lexicon/languages/nl` (pack). Il percorso model-free olandese resta
   senza filtro di classe lessicale, anche se le particelle dei cognomi sono dati.
4. **Alias di località minori**. Gli alias di località sotto 50k abitanti non vengono
   tipati LOCATION; i nomi primari e il seed restano. È una scelta di precisione, non un
   limite del corpus.
5. **Evidenza pre-commit**. Il 100/100 è stato misurato sul working tree; la suite non è
   stata rieseguita su un checkout pulito del commit che accompagna questo report.

## Riproduzione

```sh
cd BrainRust
RUST_ICU_MAJOR_VERSION_NUMBER=70 cargo build --release -p semantic-engine
VISUALNER_MODEL_DIR="$PWD/.tmp/udpipe-models" ./visualner/tools/install_udpipe_models.sh

VISUALNER_MODEL_DIR="$PWD/.tmp/udpipe-models" \
python3 semantic-engine/fixtures/multilingual_100/benchmark.py \
  --worker ./target/release/semantic-engine \
  --report /tmp/brainrust_multilingual_100_report.json
```

Per una singola lingua:

```sh
VISUALNER_MODEL_DIR="$PWD/.tmp/udpipe-models" \
python3 semantic-engine/fixtures/multilingual_100/benchmark.py \
  --worker ./target/release/semantic-engine --language it --limit 2
```

## File modificati

- `lexicon/discourse_cues.json`
- `lexicon/entity_gazetteer.json`
- `lexicon/language_profiles.json`
- `semantic-core/src/dates.rs`
- `semantic-core/src/discourse.rs`
- `semantic-core/src/lexicon.rs`
- `semantic-engine/fixtures/multilingual_100/benchmark.py`
- `semantic-engine/src/lib.rs`
- `semantic-engine/tests/semantic_engine.rs`
- `visualner/src/extractor.rs`
- `visualner/src/extractor_tests.rs`

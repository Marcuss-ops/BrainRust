# BR-AUDIT-26 / BR-72 — Unificare le primitive equivalenti

- **Priorità:** P2
- **Stato:** OPEN
- **Owner:** `semantic-core`, `visualner`, `script-qa`, `overlay-compiler`
- **Origine:** audit 2026-11-26 §11 e audit BR-DRY-02 del 2026-11-24, ticket
  proposto BR-72.

## Problema

Le primitive linguistiche sono duplicate:

| Primitive | Copie | Esempi |
| --- | --- | --- |
| `tokenize` | 4 | `semantic-core/src/phrase_select/tokenize.rs`, `semantic-core/src/dates/relative.rs`, `visualner/src/extractor/tokenizer.rs` (`tokenize`/`tokenize_untyped`), `script-qa/src/tokenize.rs` |
| `word_count` | 3+ | `semantic-core/src/sentences.rs`, `semantic-core/src/phrase_impact/scoring.rs`, `overlay-compiler/.../projection.rs` e `planner/media.rs` |
| `split_sentences` | 3 | `semantic-core/src/sentences.rs`, `semantic-core/src/phrase_impact/sentences.rs`, `semantic-engine/src/protocol.rs` |
| `is_acronym` | 2 | da censire |
| `locate_phrase` | 2 | da censire |
| `overlaps` | 2 | da censire |

Non tutte le implementazioni sono semanticamente equivalenti: `script-qa` ha
regole di parità con il vecchio contratto Go che possono differire
intenzionalmente dalle regole editoriali. Unificare per nome è un errore;
unificare per comportamento è l'obiettivo.

## Cosa serve per risolverlo

1. **Costruire una matrice di equivalenza** con input per tutte le 11 lingue,
   inclusi:
   - testi con abbreviazioni, acronimi, numeri, date, unità;
   - punteggiatura non latina e spazi Unicode;
   - frasi lunghe e casi di confine (ultimo token senza spazio finale);
   - testi vuoti.
2. **Classificare ogni coppia** come:
   - identica (stesso output su tutta la matrice);
   - intenzionalmente diversa (documentare la regola che differisce);
   - bug (una delle due è sbagliata).
3. **Spostare in `semantic-core` solo le primitive identiche**, con una sola
   API pubblica e shim di re-export temporanei per i chiamanti.
4. **Non unificare `script-qa`** con le regole editoriali se la matrice mostra
   differenze dovute al contratto Go: quelle regole restano locali e vanno
   etichettate come policy.
5. **Rimuovere le copie** dopo il cutover e aggiungere un test che fallisce se
   viene introdotta una nuova implementazione della stessa primitiva.
6. **Verificare la parità** dei manifest e dei piani dopo ogni cutover: la
   tokenizzazione cambia gli span, quindi un'unificazione sbagliata si vede
   subito.

## Criteri di accettazione

- [ ] La matrice di equivalenza è versionata e copre 11 lingue.
- [ ] Ogni primitiva duplicata è classificata (identica / diversa / bug).
- [ ] Le copie identiche sono state rimosse e i chiamanti usano `semantic-core`.
- [ ] Le differenze intenzionali sono documentate e coperte da test.
- [ ] I manifest restano byte-identici sui fixture esistenti.
- [ ] Un test impedisce di reintrodurre una copia della primitiva.

## Verifica

```sh
cd BrainRust
cargo test -p semantic-core --lib
cargo test -p visualner --lib
cargo test -p script-qa
cargo test -p semantic-engine analysis_equivalence
```

## Dipendenze / rischi

- La matrice richiede il corpus multilingua e, per le differenze di
  tokenizzazione, i modelli UDPipe: senza quelli il ticket può avanzare solo
  sulle primitive model-free.
- Unificare `tokenize` senza preservare gli span byte-exact rompe la
  validazione delle citazioni: il confronto deve includere offset, non solo
  testo.

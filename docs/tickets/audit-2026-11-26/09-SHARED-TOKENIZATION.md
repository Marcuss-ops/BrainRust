# BR-AUDIT-09 / BR-60 — Rappresentazione linguistica condivisa

- **Priorità:** P2
- **Stato:** DEFERRED
- **Owner:** `visualner`, `semantic-engine`, `semantic-core`
- **Origine:** audit 2026-11-26, finding P1 sulla tokenizzazione ripetuta.

## Problema

Il contesto globale dei nomi (`pipeline/names.rs`) tokenizza tutte le scene;
l'estrattore (`pipeline.rs`) le tokenizza di nuovo. Restano inoltre
segmentazioni di frase in punti diversi. BR-AUDIT-07 ha eliminato la seconda
`split_sentences` per scena; la doppia tokenizzazione resta.

## Perché è deferred

Un `SceneDocument` condiviso (`text`, `tokens`, `sentence_spans`,
annotazioni) cambia il confine tra `visualner` e `semantic-engine` e richiede
una matrice di equivalenza multilingua. Non è una modifica meccanica e non va
fatta senza il corpus di parità.

## Cosa serve per risolverlo

1. Definire `SceneDocument` con ownership chiara (`Arc<str>`, token, span di
   frase, annotazioni linguistiche) e un solo punto che lo costruisce.
2. Far consumare a NER, POS, date, quantità e generazione candidati la stessa
   rappresentazione, senza ri-tokenizzare.
3. Eliminare le chiamate duplicate in `pipeline/names.rs` e `pipeline.rs`
   lasciando un solo percorso di tokenizzazione per scena.
4. Confrontare i manifest su 11 lingue prima/dopo, con offset byte-exact.
5. Non fondere tokenizer con contratti diversi (es. `script-qa`) senza la
   matrice di equivalenza di [BR-AUDIT-26](26-PRIMITIVE-UNIFICATION.md).

## Criteri di accettazione

- [ ] Ogni scena viene tokenizzata una sola volta nel percorso completo.
- [ ] `SceneDocument` è l'unica fonte di token e span per le fasi locali.
- [ ] I manifest restano byte-identici sui fixture esistenti.
- [ ] La matrice multilingua è verde o le differenze sono motivate.
- [ ] Nessuna API pubblica duplica la tokenizzazione.

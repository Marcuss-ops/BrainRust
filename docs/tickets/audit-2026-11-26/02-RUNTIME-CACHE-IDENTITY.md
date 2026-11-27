# BR-AUDIT-02 — Identità runtime GLiNER e lessico override

- **Priorità:** P0
- **Stato:** CLOSED
- **Owner:** `semantic-engine`, `visualner`, `semantic-core`
- **Origine:** audit 2026-11-26, finding P0 su `cache/identity.rs`.

## Problema

`cache_identity` copriva build, contenuto embedded, ICU e modello UDPipe, ma
non:

- il modello GLiNER2.5 effettivamente usato (o la sua assenza);
- `BRAINRUST_NER_MODEL` (disabled/auto/enabled);
- le soglie `BRAINRUST_NER_THRESHOLD`, `_LOCATION_THRESHOLD`, `_ORG_THRESHOLD`;
- il contenuto effettivo di `SEMANTIC_LEXICON_DIR`, che sostituisce le tabelle
  embedded a runtime.

Due processi con la stessa directory cache ma configurazioni NER/lessico
diverse potevano quindi condividere la stessa entry.

## Soluzione

- `visualner::ner_cache_identity()` espone mode, presenza del modello, digest
  SHA-256 della variante del modello (`fp32_25`/`fp16_25`/`fp16`) e le soglie
  effettive. Il digest è calcolato una volta per processo.
- `semantic_core::lexicon::override_content_fingerprint()` calcola il digest
  SHA-256 ricorsivo della directory `SEMANTIC_LEXICON_DIR` (nomi relativi +
  contenuto), una volta per processo; `None` quando si usano le tabelle
  embedded.
- `cache_identity` include entrambi i valori.

## Criteri di accettazione

- [x] Cambiare mode, modello o soglie cambia l'identità della cache.
- [x] Cambiare il contenuto della directory override cambia l'identità.
- [x] Nessuna identità viene calcolata a ogni richiesta (caching per processo).
- [x] Test del digest della directory e dell'identità composta.

## Verifica

```sh
cd BrainRust
cargo test -p visualner ner
cargo test -p semantic-core lexicon
cargo test -p semantic-engine cache::identity
```

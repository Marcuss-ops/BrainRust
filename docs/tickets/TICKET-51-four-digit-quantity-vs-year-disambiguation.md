# TICKET 51 — Disambiguazione quantità a 4 cifre (`1000..=2099`) vs anni (`DATE`) nei grafici

- **Priority**: P1
- **Status**: **OPEN (2026-11-25)**
- **Component**: `visualner`, `semantic-engine`, `lexicon/chart_cues.json`
- **Files coinvolti**:
  - `BrainRust/visualner/src/extractor/values.rs` (`calendar_span`, L426-445; `classify_value_type`, L59-74)
  - `BrainRust/visualner/src/extractor/numeric.rs` (`numeric_value_spans`, L18-38)
  - `BrainRust/visualner/src/analysis.rs` (`numeric_facts` da `numeric_value_spans`, L248)
  - `BrainRust/semantic-engine/src/groups/charts.rs` (costruzione `ChartItem` / `VisualCandidate`)
  - `BrainRust/semantic-engine/src/chart_rules.rs` (`is_year_token`, `chart_unit`)
  - `BrainRust/lexicon/chart_cues.json` (`count_unit_cues`, `display_unit_cues`)

---

## 1. Problema

Quando uno script o una trascrizione parlata (ASR / TTS / input utente) contiene **valori numerici interi compresi tra `1000` e `2099` scritti senza separatore delle migliaia** (es. `1500`, `1200`, `1800`), `visualner` li classifica **ciecamente come anni (`DATE`)** invece che come quantità numeriche (`NUMBER`).

Di conseguenza:
1. I numeri vengono promossi a `temporal_facts` (date fasulle) invece che a `numeric_facts`.
2. `numeric_value_spans` li scarta prima che arrivino a `semantic-engine`.
3. `semantic-engine` riceve `0` `ChartPoint` e **non genera alcun grafico**, anche in presenza di una serie temporale chiarissima.

Il problema è particolarmente grave su testi provenienti da parlato/ASR o prompt veloci, dove i numeri fra `1000` e `2099` sono quasi sempre scritti senza il punto delle migliaia (`1500` invece di `1.500`).

---

## 2. Riproduzione reale (eseguita su `BrainRust/target/release/semantic-engine`)

Test eseguito il 2026-11-25 via protocollo NDJSON (`operation: "analyze_script"`, `language: "it"`):

| # | Input | `temporal_facts` | `numeric_facts` | Grafici emessi |
|---|---|---:|---:|---|
| 1 | `"Rick Ross nel 2015 ha venduto 1500 dischi, nel 2016 1200, nel 2018 1800."` | **6** (`2015`, `1500`, `2016`, `1200`, `2018`, `1800`) | **0** | **0** (FAIL) |
| 2 | `"Rick Ross nel 2015 ha venduto 1.500 dischi, nel 2016 1.200, nel 2018 1.800."` | 3 (`2015`, `2016`, `2018`) | 3 (`1500`, `1200`, `1800`) | **1** `timeline_growth` (`series_type: "time"`, `[('2015', 1500), ('2016', 1200), ('2018', 1800)]`) |
| 3 | `"Rick Ross nel 2015 ha venduto 500 dischi, nel 2016 2500, nel 2018 3200."` | 3 (`2015`, `2016`, `2018`) | 3 (`500`, `2500`, `3200`) | **1** `timeline_growth` (`series_type: "time"`, `[('2015', 500), ('2016', 2500), ('2018', 3200)]`) |
| 4 | `"Rick Ross nel 2015 ha venduto 500 auto, nel 2016 2500, nel 2018 3200."` | 3 (`2015`, `2016`, `2018`) | 3 (`500`, `2500`, `3200`) | **1** `timeline_growth` (`series_type: "time"`, `unit: "count:auto"`, `text: "500 auto"`) |
| 5 | `"Nel 2015 Rick Ross ha venduto 1500 dischi. Nel 2016 ne ha venduti 1200. Nel 2018 ne ha venduti 1800."` | **6** | **0** | **0** (FAIL) |

Nota: tutti i test di regressione esistenti (`chart_acceptance_36`, `chart_bare_cue`, `chart_multilingual`, `chart_series_metadata`, `charts_numeric` — 33/33 integration test + 11/11 unitari con `ORT_DYLIB_PATH`) passano perché nessuno dei casi di test contiene quantità nude senza punto nell'intervallo `1000..=2099`.

---

## 3. Root Cause nel codice

### 3.1 Punto primario — `calendar_span` in `visualner/src/extractor/values.rs`

In `BrainRust/visualner/src/extractor/values.rs:426-444`:

```rust
fn calendar_span(
    source_text: &str,
    number_start: usize,
    number_end: usize,
    start: usize,
    end: usize,
    kind: &'static str,
) -> (usize, usize, &'static str) {
    let mut start = start;
    let mut end = end;
    let mut kind = kind;
    let number = &source_text[number_start..number_end];
    if kind == "NUMBER" && number.len() == 4 {
        if let Ok(year) = number.parse::<u32>() {
            if (1000..=2099).contains(&year) {
                kind = "DATE";
            }
        }
    }
```

Il controllo non esamina alcun contesto locale prima o dopo `number`:
- `1500` in `"ha venduto 1500 dischi"` ha `kind == "NUMBER"` (perché `dischi` non è una valuta/unità fisica in `scale_words`), `number.len() == 4`, `1500 ∈ 1000..=2099` → viene riscritto in `"DATE"`.
- `1200` in `"nel 2016 1200"` e `1800` in `"nel 2018 1800"` subiscono la stessa sorte, anche se seguono immediatamente un vero anno ancorato da preposizione temporale (`nel 2016`, `nel 2018`).

### 3.2 Propagazione a valle

1. `BrainRust/visualner/src/extractor/numeric.rs:18-38` (`numeric_value_spans`):
   ```rust
   value_candidates(source_text)
       .into_iter()
       .filter(|candidate| {
           matches!(
               candidate.entity_type.as_deref(),
               Some("NUMBER") | Some("PERCENT") | Some("MONEY") | Some("MEASUREMENT")
           )
       })
   ```
   Poiché `1500`, `1200`, `1800` arrivano come `Some("DATE")`, vengono filtrati via del tutto.
2. `BrainRust/visualner/src/analysis.rs:248`: `scene.numeric_facts` rimane vuoto (`[]`).
3. `BrainRust/semantic-engine/src/groups/charts.rs`: `chart_points` è vuoto → nessuna proposta `chart` viene costruita.

### 3.3 Punto secondario — `classify_value_type` in `visualner/src/extractor/values.rs`

In `BrainRust/visualner/src/extractor/values.rs:59-74`:
```rust
    if text.chars().any(|c| c.is_ascii_digit()) {
        let lower = text.to_lowercase();
        let year = text
            .trim()
            .parse::<u32>()
            .ok()
            .is_some_and(|year| (1000..=2099).contains(&year));
        ...
        if year || has_month || text.contains('/') {
            return "DATE".to_string();
        }
        return "NUMBER".to_string();
    }
```
Anche questa funzione, quando riceve uno span numerico isolato di 4 cifre, applica la stessa euristica priva di contesto.

### 3.4 Gap minore di lessico — `chart_cues.json`

Nel test #3 vs #4 si nota che `"auto"` è in `count_unit_cues` / `display_unit_cues` di `BrainRust/lexicon/chart_cues.json` (quindi `chart_meta.unit = "count:auto"` e `item.text = "500 auto"`), mentre `"dischi"`, `"copie"`, `"album"`, `"libri"`, `"biglietti"` non ci sono. Non impedisce la creazione del grafico, ma lascia `chart_meta.unit` vuoto e priva l'etichetta della sua unità di misura.

---

## 4. Proposta di soluzione

### 4.1 Disambiguazione contestuale anno vs quantità in `visualner/src/extractor/values.rs`

In `calendar_span` (dove abbiamo l'intero `source_text` e gli offset `number_start..number_end`), un intero di 4 cifre in `1000..=2099` **NON** deve essere classificato come `DATE` quando il contesto indica chiaramente una quantità e non una data.

Casi sicuri ad alta precisione da trattare come `NUMBER` (non `DATE`):
1. **Numero immediatamente preceduto da un anno temporale** (pattern `"nel 2016 1200"`, `"in 2016 1200"`, `"nel 2018: 1800"`): se prima di `number_start` (al netto di spazi, virgole o `:`) compare già un anno `1900..=2099` (specie se introdotto da preposizione temporale `nel/in/en/em/im/w/pada/в/dal/al`), il secondo numero è il valore associato a quell'anno, non un secondo anno.
2. **Numero preceduto da verbo/predicato quantitativo e NON da preposizione temporale** (es. `"ha venduto 1500"`, `"ne ha venduti 1200"`, `"sold 1500"`, `"prodotto 1800"`, `"registrato 1200"`, `"raggiunto 1500"`): se il token immediatamente precedente (saltando eventuali approssimatori come `circa`, `oltre`, `quasi`, `più di`, `about`, `over`) è un verbo di conteggio/vendita/produzione (non una preposizione temporale come `nel`, `del`, `dal`, `al`, `in`, `since`, `until`, `from`, `to`), il numero è una quantità.
3. **Numero seguito da un sostantivo/unità contabile** (es. `1500 dischi`, `1200 copie`, `1800 auto`, `1500 persone`, `1200 unità`, `1500 dipendenti`): se dopo `number_end` c'è un'unità in `count_unit_cues` (o un sostantivo plurale contabile quando non preceduto da preposizione temporale), il numero è una quantità.
4. **Restrizione del range predefinito per anni "nudi" senza preposizione temporale**: valutare se anni `1000..=1899` senza preposizione temporale/mese/era (`nel 1500`, `1500 d.C.`, `marzo 1500`) debbano richiedere almeno un cue temporale prima di diventare `DATE`, mantenendo `1900..=2099` come anno di default solo quando non smentito dalle regole 1–3.

> **Guardia anti-regressione**: date reali come `"nel 1500"`, `"nel 1789"`, `"il 15 marzo 2008"`, `"tra il 2015 e il 2018"`, `"dal 2016 al 2018"`, `"2021 verkaufte Aurora 120 Autos"` devono continuare a essere classificate come `DATE`. In particolare, un anno separato da un altro anno da congiunzioni/preposizioni di intervallo (`e`, `al`, `a`, `to`, `-`, `–`, `and`, `bis`) deve restare `DATE` (`"dal 2015 al 2018"` = due `DATE`, non `DATE + NUMBER`).

### 4.2 Arricchimento opzionale `lexicon/chart_cues.json`

Aggiungere unità editoriali frequenti in `count_unit_cues` e `display_unit_cues` (es. `disco`/`dischi`, `album`, `copia`/`copie`, `copy`/`copies`, `record`/`records`, `libro`/`libri`, `book`/`books`, `biglietto`/`biglietti`, `ticket`/`tickets`) così `chart_meta.unit` e `item.text` risultino popolati in modo coerente.

---

## 5. Criteri di accettazione

1. **Regression test su `visualner`**:
   - `"Rick Ross nel 2015 ha venduto 1500 dischi, nel 2016 1200, nel 2018 1800."` produce:
     - `temporal_facts`: `2015`, `2016`, `2018` (3 date)
     - `numeric_facts`: `1500`, `1200`, `1800` (3 numeri)
   - `"Dal 2015 al 2018 non ci furono modifiche."` e `"Tra il 1998 e il 2004..."` continuano a produrre solo `DATE` e `0` `numeric_facts`.
   - `"Nel 1500 Leonardo arrivò a Venezia."` continua a classificare `1500` come `DATE`.
2. **Integration test su `semantic-engine`** (`charts_numeric.rs` o `chart_series_metadata.rs`):
   - Sia la forma in frase unica (`"Rick Ross nel 2015 ha venduto 1500 dischi, nel 2016 1200, nel 2018 1800."`) sia la forma multi-frase (`"Nel 2015 Rick Ross ha venduto 1500 dischi. Nel 2016 ne ha venduti 1200. Nel 2018 ne ha venduti 1800."`) emettono **1 grafico `timeline_growth`** (`series_type: "time"`) con i 3 punti `[('2015', 1500.0), ('2016', 1200.0), ('2018', 1800.0)]`.
3. **Zero regressioni** sull'intera suite esistente:
   - `cargo test -p visualner`
   - `ORT_DYLIB_PATH=... cargo test -p semantic-engine --test chart_acceptance_36 --test chart_bare_cue --test chart_multilingual --test chart_series_metadata --test charts_numeric`

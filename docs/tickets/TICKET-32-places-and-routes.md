# TICKET-32 — places have no kind, coordinates, hierarchy or routes

- Priority: P1
- Status: PARTIAL (verified 2026-11-20)
- Depends on: TICKET-35 (a wrong link gives wrong coordinates)
- Files: `lexicon/place_index_v1.json` (+ `lexicon/tools/build_place_index.py`),
  `semantic-core/src/{place.rs (new),entity_linker.rs}`,
  `semantic-engine/src/{places.rs,routes.rs}` (new),
  `lexicon/{location_cues,place_inflections,motion_cues}.json`,
  `SceneManifest.places` / `SceneManifest.routes`

## Problem

A toponym is a `LOCATION` entity with a surface and (maybe) a KB id. The
manifest does not say whether it is a country, a city, a street or a building,
where it is, what contains it, or that "da Milano a Roma passando per Firenze"
describes an ordered route. Today `location_cues.json` and `place_inflections`
only help to *find* and *normalise* the surface (TICKET-24).

## Data model (additive)

```jsonc
// SceneManifest.places[]
{ "id": "pl_01", "entity_local_id": "loc_03",
  "kb_id": "geo:3173435",             // only when link_status == linked
  "kind": "city",                     // country|region|province|city|district|street|facility|landmark|water|mountain|island|unknown
  "name": "Milano",
  "lat": 45.4642, "lon": 9.1900,      // null unless linked
  "bbox": [..] ,                      // optional, for countries/regions
  "parent_ids": ["geo:3174618","geo:3175395"],   // Lombardia, Italia
  "role": "setting|origin|destination|waypoint|mention",
  "span": [..], "evidence": "…", "sentence_index": 1 }

// SceneManifest.routes[]  (and cross-scene in manifest.routes_global)
{ "id": "rt_01", "mode": "road|rail|air|sea|walk|unknown",
  "stops": [ {"place_id":"pl_01","role":"origin"},
             {"place_id":"pl_04","role":"waypoint"},
             {"place_id":"pl_02","role":"destination"} ],
  "straight_line_km": 477.3,          // great-circle, informative only
  "evidence_span": [..], "status": "asserted|negated|uncertain" }
```

## Design

1. **`place_index_v1.json`**: curated, versioned, checksummed seed (countries,
   admin-1 regions, cities ≥ 100k population, capitals, ~500 landmarks/
   facilities already in the entity index). Each row: id, canonical name,
   `kind`, lat/lon, `parent_ids`, aliases per language (BCP-47 keyed, matched
   cross-language like `EntityIndex`), `context_terms`, `counter_terms`.
   Built by `build_place_index.py` from an open dataset; **record source,
   licence and retrieval date in the file header** (GeoNames CC-BY or
   Wikidata CC0). The generator is reproducible and the file is size-budgeted
   (≤ 5 MB packaged). Rust never reads the network.
2. **Linking** goes through the same scorer as TICKET-35 (feature
   `geo_parent_coherence`: `Springfield` + `Illinois` in the window picks the
   Illinois one). Unlinked/ambiguous places keep `kind: unknown` (or the kind
   inferred from an explicit cue word: `la città di X`, `il fiume X`, `via X`
   → `kind` from the cue, **no coordinates**).
3. **Role** from prepositions in `location_cues.json`: `a/in/at` → setting;
   `da/from/von/de` → origin; `verso/to/nach/à/para` → destination.
4. **Routes**: a clause pattern engine using `lexicon/motion_cues.json`
   (verbs and nouns: `partire, arrivare, viaggio, volo, rotta, traversata,
   fly, drive, voyage, von…nach`) + origin/destination prepositions:
   - `da A a B`, `from A to B`, `von A nach B`, `de A à B`, `de A a B`,
     `z A do B`, `из A в B`, `A'dan B'ye`, `dari A ke B`;
   - waypoints: `passando per`, `via`, `through`, `über`, `par`, `por`;
   - multi-hop chains across consecutive sentences only if the discourse
     thread keeps the same subject and the destination of hop n is the origin
     of hop n+1.
   Stops are kept in **source order**; none invented, none merged. `mode` from
   the motion cue; `unknown` otherwise. `straight_line_km` only when both
   endpoints have coordinates.
5. **Sanity checks (reject, don't guess)**: a route with identical origin and
   destination, or with a stop whose place is `ambiguous`, is emitted with
   `status: uncertain` and no distance.
6. **Negation / hypothesis** (`non è mai andato a…`, `would have flown`) reuse
   the status vocabulary from `EntityFact`.

## Acceptance criteria

- [x] Place index: schema validated, every row has coordinates in range,
      parents resolve, aliases collision report (same alias in two rows) is
      empty or explicitly flagged ambiguous.
- [ ] Gold ≥ 30 places and ≥ 20 routes per language: kind accuracy ≥ 90 %,
      coordinates exact for linked places (100 %), route stop order 100 % of
      emitted, route recall ≥ 90 %.
- [ ] Negative set: `Via Appia` as a person-name pun, `Paris` the person,
      `Washington` the person, `in Italia` mentions without movement, `da A a B`
      meaning a range (`da 5 a 10`): 0 false positives; ambiguous → no
      coordinates.
- [ ] Offsets byte-exact across inflected forms (pl/ru/tr).
- [x] Output deterministic; `AnalyzerVersions.places` bumped; Go copy updated.

## Non-goals

Geocoding street addresses, polygons, travel time/distance by road, fetching
maps, route optimisation, and any rendering decision.

## Implementation steps

1. Index builder + schema + validation tests (small seed first: countries and
   capitals).
2. `place.rs` types; link-through-scorer integration (after TICKET-35).
3. Roles and kinds; unit tests from existing `slavic_places.rs` fixtures.
4. Route engine, per-language motion lexicons, gold.
5. Index growth (regions, cities) with the collision report in CI.

## Prossimi passi (2026-11-19)

1. **Attende TICKET-35** per le coordinate: un link sbagliato produce una
   coordinata sbagliata, quindi il seed dell'indice si può preparare ma il
   collegamento non si accende prima dello scorer spiegabile.
2. **Seed minimo:** paesi + capitali con `build_place_index.py`, schema,
   validazione (coordinate in range, parent risolti, alias collision report),
   fonte/licenza/dataset nell'header del file.
3. **`place.rs` + ruoli** (`setting|origin|destination|waypoint`) da
   `location_cues.json`, con i test presi da `slavic_places.rs`.
4. **Route engine** (`da A a B`, `via`, `passando per`) con stop in ordine di
   sorgente e nessuna distanza se manca una coordinata.

Gate: 30 luoghi e 20 rotte per lingua, kind ≥ 90 %, coordinate esatte per i
linkati, ordine degli stop 100 %, negativi (`Paris` persona,
`da 5 a 10`) a 0 falsi positivi.

## Verifica 2026-11-20

Ricontrollato sul working tree corrente (`2bbeca1` più le modifiche non
committate).

32a è completato e i primi slice runtime 32b/32c/32d sono nel working tree:
`semantic_core::Place`, `SceneManifest.places`, `AnalyzerVersions.places` e
`semantic-engine/src/places.rs`. Il linker accetta solo span LOCATION esatti;
alias unici ricevono coordinate, collisioni senza parent grounded restano
ambigue e senza coordinate. Cue localizzati assegnano setting/origin/destination;
`place_kind_cues.v1` assegna inoltre country/city/river/mountain/island da cue
espliciti anche agli alias non linkati, senza coordinate. Il motore rotte ora
include gli waypoint cues localizzati in stop intermedi grounded e ordinati;
un waypoint ambiguo rende incerta la route senza distanza. Nove test `places`
e nove test `routes` passano. Restano forme flesse, disambiguazione avanzata e
gold. Il merge dei multi-hop consecutivi tra scene è stato aggiunto
e coperto dal protocollo (verifica 2026-11-20).

TICKET-35 è PARTIAL: 35a (`type_cues`) è completato, mentre 35b/35c/35d
restano aperti. Il link iniziale 32b usa alias esatti e conserva l'ambiguità;
la disambiguazione contestuale dello scorer 35b resta necessaria per ampliarlo.
Stato: **PARTIAL**.

### Avanzamento 2026-11-20 — cue espliciti per la classe

- Aggiunto `lexicon/place_kind_cues.json` per tutte le 11 lingue e incluso nel
  fingerprint semantico.
- I cue sono accettati solo immediatamente prima di una menzione LOCATION con
  span valido; non cambiano linking o coordinate. Un luogo sconosciuto rimane
  senza coordinate anche quando il tipo è inferito.
- Verificato: `cargo test -p semantic-engine --lib places::tests` — 9 pass;
  test del fingerprint — pass; `cargo clippy -p semantic-engine --all-targets
  -- -D warnings` — pass.

## Piano di chiusura e split (2026-11-20)

Il ticket è troppo grande per un solo PR: indice, linking, ruoli, rotte e gold
hanno dipendenze diverse. Split consigliato in 5 parti.

### 32a — Indice luoghi v1 (M, parte subito)

- `lexicon/place_index_v1.json` + `lexicon/tools/build_place_index.py`.
- Riga: id stabile, nome canonico, `kind` (country/region/province/city/
  district/street/facility/landmark/water/mountain/island), lat/lon, `bbox`
  opzionale, `parent_ids`, alias per BCP-47, `context_terms`, `counter_terms`.
- Header con fonte, licenza (GeoNames CC-BY o Wikidata CC0), data di retrieval
  e checksum; output ≤5 MiB; validazione coordinate in range, parent risolti,
  alias collision report.
- CI: `build_place_index.py --check`. Nessun comportamento Rust in questo passo.
- Fatto quando: seed paesi+capitali (poi città ≥100k e landmark) valida e il
  collision report è vuoto o dichiara ogni ambiguità.

### 32b — Tipi, manifest e linking (M, dopo 35a/b)

- `semantic-core/src/place.rs`: `Place`, `PlaceKind`, `PlaceRole`,
  `PlaceSpan`. Campi additivi `SceneManifest.places` e (dopo 32d)
  `SemanticManifest.routes_global`.
- Collegamento tramite lo scorer di 35 con feature `geo_parent_coherence`
  (`Springfield` + `Illinois`); unlinked/ambiguous → `kind: unknown` o kind da
  cue, **nessuna coordinata**.
- `AnalyzerVersions.places`, fingerprint e cache key.
- Fatto quando: un luogo linkato ha coordinate e parent; uno ambiguo non ha
  coordinate; span byte-exact su forme flesse.

### 32c — Ruoli e kind da cue (S, dopo 32b)

- Estendere `location_cues.json`: setting (`a/in/at`), origin (`da/from/von/
  de`), destination (`verso/to/nach/à/para`); riusare `place_inflections.json`
  per pl/ru/tr.
- Cue di classe (`la città di X`, `il fiume X`, `via X`) → `kind` senza
  coordinate.
- Test presi da `slavic_places.rs` e `place_inflections`.
- Fatto quando: ruolo corretto sul gold dei cue e 0 coordinate inventate.

### 32d — Motore rotte (M, dopo 32c; initial same-sentence waypoint slice landed)

- `lexicon/motion_cues.json`: verbi/nomi (`partire`, `arrivare`, `viaggio`,
  `volo`, `rotta`, `fly`, `drive`, `von…nach`).
- Pattern per lingua: `da A a B`, `from A to B`, `von A nach B`, `de A à B`,
  `z A do B`, `из A в B`, `A'dan B'ye`, `dari A ke B`; waypoint `passando
  per`, `via`, `through`, `über`, `par`, `por`.
- Stop in ordine di sorgente; `mode` dal cue o `unknown`;
  `straight_line_km` solo con entrambe le coordinate; origine=destinazione o
  stop ambiguo → `uncertain` senza distanza; waypoint cue now projects ordered
  intermediate stops in the same sentence. Consecutive asserted hops merge
  only when the same grounded person and linked intermediate place recur;
  adjacent-scene hop merging is implemented in the whole-script manifest.
- Fatto quando: 20 rotte per lingua sul gold con ordine 100% e 0 rotte
  inventate.

### Avanzamento 2026-11-20 — waypoint e hop consecutivi

- `RouteCues` ora legge `waypoint` da `route_cues.json`. Per ogni origin/destination
  grounded, i luoghi intermedi entrano negli stop solo se un waypoint cue appare
  tra l'origine e la menzione; nessuno stop viene inventato.
- Gli stop conservano l'ordine nel testo e il loro ruolo è `waypoint`. Un
  waypoint ambiguo rende incerta la route e sopprime la distanza.
- Verificato: `cargo test -p semantic-engine --lib routes::tests` — **9 pass**,
  inclusi waypoint ordinati, waypoint ambiguo e merge di hop con soggetto
  identico; soggetti differenti restano separati. La proiezione Go è ora
  presente; restano gold e metriche multilingue.

### 32e — Gold e chiusura (L)

- ≥30 luoghi e ≥20 rotte per lingua; negativi (`Via Appia` persona, `Paris`
  persona, `Washington` persona, `in Italia` senza movimento, `da 5 a 10`
  range).
- Metriche in `score_long_gold.py`: kind accuracy ≥90%, coordinate esatte
  100% sui linkati, stop order 100%, recall rotte ≥90%, 0 falsi positivi.
- Copia Go, `AnalyzerVersions.places` e README aggiornati; lo stato può passare
  a CLOSED solo dopo il superamento dei gold e degli altri gate sopra.
- Fatto quando: tutti i gate sopra + `cargo fmt/clippy/test` e
  `build_place_index.py --check` verdi.

Ordine: **32a → 32b exact alias → 32c cue roles → 32d routes → 32e gold**.
La disambiguazione per coerenza geografica si innesta sul prior 35b; il linker
conservativo e le cue locali non lo richiedono.

## Slice disambiguazione esplicita città/paese — 2026-11-21

- I cue di classe espliciti (`city of`, `country of` e le varianti localizzate)
  ora filtrano i candidati dell'indice prima della risoluzione. Così
  “city of Luxembourg” seleziona la città e “country of Luxembourg” seleziona
  il paese senza dipendere da un parent citato separatamente.
- La risoluzione dei collisioni usa anche i `context_terms`/`counter_terms`
  versionati dell'indice e i parent geografici menzionati nella stessa frase.
  “Luxembourg (LU)” seleziona il paese; un pareggio senza evidenza resta
  ambiguous.
- Un cue esplicito incompatibile con l'indice non assegna coordinate: “country
  of Paris” resta unlinked con il tipo dichiarato dal testo.
- Verificato con `cargo test -p semantic-engine --lib places::tests`: **11
  pass**, inclusi collisione città/paese, context term, incoerenza del cue e
  alias ambiguo senza contesto; Clippy e `git diff --check` passano. Il ticket
  resta PARTIAL per gold/metriche e copertura disambiguazione più ampia.

## Verifica workspace 2026-11-20

- `cargo test --workspace` — **PASS** (suite completa; include test unitari e
  protocollo per luoghi, rotte e forme flesse PL/RU).
- `cargo clippy --workspace --all-targets -- -D warnings` — **PASS**.
- `python3 lexicon/tools/build_place_index.py --check` — **PASS** (schema,
  coordinate, parent, collisioni e budget dimensione).
- PipelineGen ora decodifica e proietta places, routes e cross-scene routes;
  il consumer valida span/evidence, coordinate, ruoli e ordine degli stop,
  riferimenti a luoghi e scene, continuità degli endpoint linkati e coerenza
  degli hop. `go test ./internal/platform/media/rustexec` — **PASS**; i test
  specifici verificano anche che due soggetti diversi non vengano uniti e che
  un merge con una scena saltata venga rifiutato.
- Restano fuori dal gate locale: gold multilingue ≥30/20 per lingua, linking
  geo-parent nel disambiguatore, forme suffissate (incluso turco) e review dei
  dati. Il ticket resta **PARTIAL**.

## Avanzamento 2026-11-20 — 32a

- Added the frozen Wikidata country/capital snapshot and deterministic
  `build_place_index.py`. The generated index has 195 country entries and 199
  capital entries with Wikidata IDs, English/Italian labels, coordinates,
  country parent links for capitals, and six explicit alias collisions.
- The builder verifies the frozen result SHA-256, coordinate ranges, unique
  IDs, resolved parents, collision declarations, and the 5 MiB size bound.
  Source is Wikidata structured data under CC0, retrieved 2026-11-20.
- Verification: `python3 lexicon/tools/build_place_index.py --check` and
  `python3 -m unittest discover -s lexicon/tools -p 'test_build_place_index.py'`
  — **4 passed**. Runtime exact-alias linking and localized cue roles now have
  their own Rust gates; routes and multilingual gold remain open.


## Avanzamento 2026-11-20 — runtime place linking (32b slice)

- Aggiunto `Place` al manifest di scena e la versione `place_index.v1`
  all'identità analyzer/cache. `semantic-engine/src/places.rs` carica l'indice
  embedded, abbina solo span LOCATION esatti e assegna coordinate solo ad alias
  univoci; le collisioni restano `ambiguous`, `kind: unknown`, senza KB id e
  coordinate. Le menzioni non presenti nell'indice restano `unlinked`.
- Otto unit test coprono link univoco e span, collisioni senza coordinate,
  cue role localizzate e input non grounded; sei test route verificano
  ordine, distanza, negazione, ambiguità e assenza di cue movimento; due test
  protocol verificano la proiezione place/route effettiva nel manifest.
- Aggiunto `lexicon/place_roles.json` con cue localized per gli 11 language
  pack e ruoli setting/origin/destination a contatto con il toponimo; in assenza
  di cue il ruolo resta `mention`. I test verificano cue inglesi e italiani e
  la copertura di tutti i pack.
- Restano linking contestuale avanzato, forme flesse e gold.


## Avanzamento 2026-11-20 — route initial slice (32d)

- `lexicon/route_cues.json` fornisce pattern origine/destinazione e cue di
  movimento, oltre a cue di modo, per gli 11 language pack; il file è incluso
  nel content fingerprint.
- `semantic-engine/src/routes.rs` emette una rotta solo con due place span
  grounded nello stesso periodo e un verbo di movimento esplicito. Gli stop
  restano in ordine sorgente; cue espliciti proiettano waypoint intermedi nella
  stessa frase. Distanza Haversine solo tra endpoint linkati e con status
  asserted; negazioni e luoghi ambigui sopprimono la distanza.
- La copertura runtime include nove test `routes` (waypoint ordinato, waypoint
  ambiguo, merge di hop consecutivi) e un test protocol per la coppia
  origine/destinazione. Test route mirati, protocollo, Clippy, formatting e
  diff sono passati. Forme suffissate (turco) e gold rimangono aperti. Il
  protocollo verifica anche che hop consecutivi con soggetti diversi non
  vengano uniti.

## Linking inflected capitals — 2026-11-21

- Il builder `place_index.v1` usa ora uno snapshot Wikidata CC0 che include le
  etichette dei paesi e delle capitali in tutte le 11 lingue supportate; il
  risultato è frozen con checksum e il generatore resta offline e riproducibile.
  Gli alias manuali sono stati rimossi perché duplicavano etichette Wikidata.
- L'indice mantiene 195 paesi e 199 capitali, passa da 6 a 36 collisioni
  dichiarate in modo esplicito e resta sotto il limite di 5 MiB. Se Wikidata
  non ha una label locale per una capitale, il generatore usa la label inglese;
  tutti gli entry espongono comunque una superficie per lingua.
- Aggiunti alias seed `Warszawa` → Warsaw, `Москва` → Moscow e `Ankara` → Ankara;
  le forme `Warszawie`, `Москве` e `Ankara'da` vengono normalizzate attraverso
  il pack inflections e risolte sul place index. L'output mantiene coordinates
  solo dopo il match canonico esatto e span/evidence restano la forma flessa
  originale.
- Il pack include anche le forme `Polsce`, `России`, `Türkiye'de`,
  `Türkiye'den` e `Türkiye'ye`; solo forme esattamente associate a un entry del
  seed producono coordinate. Nessun fuzzy match.
- Verificato il 2026-11-21: 12 test del modulo `places`, 5 test Python del
  builder e `build_place_index.py --check`; `cargo test -p semantic-engine
  --tests --locked`, Clippy, fmt e diff-check passano.
- Aggiunto un test Rust per il linking grounded di Germania/Germany con lo
  stesso QID e span esatto in tutte le 11 lingue; il builder verifica le label
  ufficiali localizzate e fallback esplicito ai label inglesi.
- `AnalyzerVersions.places` è ora `place_index.v3.multilingual_labels`; il
  fingerprint include l'indice generato e il vecchio file degli override è
  stato rimosso.
- È uno slice iniziale per le forme flesse, non il gate ≥30 luoghi/lingua né
  il linking geo-parent generale: TICKET-32 resta **PARTIAL**.

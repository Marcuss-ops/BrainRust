# TICKET-32 — places have no kind, coordinates, hierarchy or routes

- Priority: P1
- Status: OPEN
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

- [ ] Place index: schema validated, every row has coordinates in range,
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
- [ ] Output deterministic; `AnalyzerVersions.places` bumped; Go copy updated.

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

# Image catalog module (images only)

`imagecatalog` is the RustBrain module that turns script text into deterministic
image decisions over the **existing** PipelineGen entity image catalog. It is
the first image-only slice; video, stock clips and sound effects are out of
scope. An optional semantic sidecar adds descriptions and visible identities
keyed by the existing `candidate_id`; it never creates a second catalog.

## Boundary: existing database, no second catalog

The durable catalog stays where it already is:

- `entity_image_catalog_entities`
- `entity_image_catalog_candidates`
- `entity_image_catalog_materializations`

The Rust component never opens PipelineGen's SQLite database, loads
credentials, downloads bytes or renders. PipelineGen (or the read-only bridge
in `tools/export_image_catalog.py`) passes the current rows as a snapshot in
the worker request. This preserves the existing single-catalog ownership and
avoids the divergence the architecture already forbids.

## Pipeline

```text
script text
  -> semantic-engine (existing)
       grounded entity_search / contextual / composed queries
  -> canonical PERSON id (same derivation as PipelineGen)
  -> existing catalog snapshot
       state + quality + materialization gates
  -> deterministic per-scene image plan
```

For every `entity_search` proposal the module resolves the canonical
`person:<slug>` and selects at most one catalog candidate. Contextual and
composed queries are reported as `not_catalog_eligible` because the current
catalog stores PERSON identity images only; the caller keeps its existing
retrieval path for them.

## Selection policy

The policy is a Rust projection of the existing Go catalog rules, not a new
threshold:

| Check | Behavior |
| --- | --- |
| Semantic status | Only `accepted` is eligible. `unknown`/legacy and `rejected` fail closed. |
| Semantic score | If recorded and below `0.60`, reject. |
| Technical score | If recorded and below `0.50`, reject. |
| Dimensions | When both sides are known: long side `>= 800px` and `>= 400000px`. |
| Freshness | `fresh` at exactly 7 days, `stale` one second later. `stale` stays usable as fallback. `broken`/`retired` are not usable by age. |
| Materialization | Reusable only when `materialized`, with `asset_id`, `drive_link` and a 64-hex SHA-256. A verified materialization keeps a `broken` remote URL renderable; it does not make `retired` rows selectable. |
| Reuse | A candidate already selected by an earlier scene is demoted but not dropped. |

The selected candidate carries the source URL, dimensions and scores; when a
verified materialization exists it also carries `asset_id`, `content_sha256`
and `drive_link`, so the renderer can use the content address without a
download.

`score` is a deterministic ranking score, **not** a calibrated probability
that the image is editorially correct. There is no fixed cosine `0.90` gate.

## Semantic annotation sidecar

A snapshot may carry `annotations` keyed by `candidate_id`. The exporter
merges them; the Rust worker only reads the resulting snapshot. Fields:

| Field | Meaning |
| --- | --- |
| `description` | What is actually visible, in the editor's language. |
| `subjects` | Canonical identities visible in the image. When non-empty, the candidate's `canonical_entity_id` must be among them. |
| `tags` | Actions, places, composition and provenance terms. |
| `depiction` | `photograph`, `composite`, `illustration`, `synthetic`, `generated` or `unknown`. |
| `identity_status` | `verified_visual`, `catalog_claimed`, `unverified` or `mismatch`. |
| `review_status` | `reviewed`, `unverified` or `pending`. |
| `source`, `evidence` | Who produced the annotation and why it is credible. |

The policy is additive and fail-closed:

- `mismatch` makes the candidate ineligible, even when the owner catalog
  certified it. This is how a wrong filename or wrong person is recorded
  without deleting the durable row.
- `synthetic`, `illustration` and `generated` are not documentary identity
  images and are ineligible for a PERSON query.
- If `subjects` is non-empty and does not include the candidate identity, the
  candidate is ineligible unless the annotation explicitly says `mismatch`.
- A reviewed solo portrait outranks an otherwise comparable composite
  (`composite` carries a small ranking penalty).
- An `unverified` identity is usable as a fallback but carries a small ranking
  penalty.
- Other identities in the same scene promote a candidate whose annotation
  lists them, with a bounded bonus (maximum `0.10`). This is how a triptych or
  group shot can win when the scene names both people, without letting a large
  group dominate a solo query.

Descriptions do not override the existing certification. A candidate with
`semantic_status = rejected` stays rejected even if it has a beautiful
annotation; the annotation can explain what is visible, but recertification
belongs to PipelineGen's catalog workflow.

See
[`image-catalog-visual-review.md`](image-catalog-visual-review.md) for the
images actually inspected and the corrections found.

## Decisions

| Decision | Meaning |
| --- | --- |
| `selected` | A usable catalog candidate was frozen for the query. |
| `catalog_miss` | The query is catalog-eligible but no row passed the gates. The caller uses the existing external retrieval path. |
| `not_catalog_eligible` | Non-PERSON, contextual or composed query; this image slice does not cover it yet. |

The plan is versioned (`image_plan.v1`) and includes the semantic fingerprint
plus a catalog snapshot fingerprint, so replay is byte-identical for the same
text, analyzer versions and catalog rows.

## Commands

Export the existing database read-only:

Export the existing database read-only, optionally with a metadata overlay
and a curated visual-review overlay (later files win):

```sh
python3 tools/export_image_catalog.py \
  --db /path/to/pipelinegen.db \
  --annotations imagecatalog/fixtures/media_annotations_postgres_20261110.json \
  --annotations imagecatalog/fixtures/curated_visual_annotations_20261110.json \
  --out /tmp/image-catalog.json
```

Regenerate the Postgres-derived annotations from a read-only CSV export:

```sh
# Run against the existing Postgres database; no table is written.
psql "$PG_DSN" --csv -c "
  SELECT id, content_sha256, name,
         COALESCE(metadata_json,'') AS metadata,
         COALESCE(search_text,'') AS search_text,
         COALESCE(tags,'') AS tags
  FROM media_assets
  WHERE asset_kind = 'web_image' OR source = 'internet_images'
  ORDER BY id" > /tmp/web_images.csv

python3 tools/export_media_annotations.py \
  --db /path/to/pipelinegen.db \
  --web-images-csv /tmp/web_images.csv \
  --out imagecatalog/fixtures/media_annotations_postgres_20261110.json
```

The bridge joins by content SHA-256. It never downloads an image and never
creates a table. `--media-annotations` on `export_image_catalog.py` can also
derive unverified annotations directly from the SQLite `media_assets` table.

Send one plan request to the worker:

```sh
jq -n \
  --argjson catalog "$(cat /tmp/image-catalog.json)" \
  '{version:"imagecatalog.v1", operation:"plan", language:"it",
    scenes:[{scene_id:"s1", text:"Alberto Stasi entra nel tribunale."}],
    catalog:$catalog, now:"2026-11-10T00:00:00Z"}' \
  | ./target/release/imagecatalog
```

The worker is NDJSON: one request per line, one plan per line. Errors are
reported on the same line with `error` and `error_code`, never by exiting.

## Current limits

- PERSON identities only. A future organizations/locations catalog extends the
  same snapshot shape and policy; it does not create a second database.
- Semantic annotations cover the rows passed by the exporter. The committed
  seed reviews 20 images and carries 130 automatic annotations; it is not a
  claim that all 3,613 candidates have verified descriptions.
- No SigLIP/pgvector embedding comparison yet. Selection relies on the
  existing catalog certification (`semantic_status`, scores, dimensions) plus
  the optional annotation gates above.
- No face recognition. `verified_visual` means the inspected images were
  consistent by eye, not a biometric match.
- No internet search, acquisition, materialization or rendering. A
  `catalog_miss` is intentionally returned to the caller instead of forcing a
  wrong match.
- No provider precision@10 claim and no calibrated confidence. Those remain
  measured acceptance work.

## Tests

```sh
RUST_ICU_MAJOR_VERSION_NUMBER=70 \
VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models \
cargo test -p imagecatalog
```

Unit tests cover canonical identity, the seven-day boundary, every quality
gate, durable materialization reuse, deterministic tie-breaking and snapshot
fingerprints. They also cover annotation mismatch, missing primary subjects,
synthetic depictions, composite demotion and scene-context promotion. The
integration tests cover grounded entity selection, cross-scene preference,
explicit misses, deferred contextual queries and a replay over the real
`fixtures/catalog_seed_20261110.json` slice.

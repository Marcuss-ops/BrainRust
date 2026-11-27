# semantic_manifest.v4 — normalized identity table

This directory is the versioned contract between the BrainRust semantic engine
and PipelineGen's Go consumer (`internal/platform/media/rustexec`). v4 is the
**cutover version**: it removes the O(N²) cumulative identity copies from the
wire without changing v3, so both sides can migrate independently and roll back
by omitting one request field.

## Why v4

`semantic_manifest.v3` writes the whole accumulated identity registry into every
`SceneManifest`. With N scenes and identities that keep appearing, the manifest
contains O(N²) copies of the same objects. The cost is not only Rust memory: it
propagates to JSON, the persistent cache, the NDJSON response and the Go
consumer's heap.

Measured on a 70-scene corpus (`it_stadium.txt` repeated 10×, 147 unique
identities):

| Metric | v3 | v4 | Delta |
| --- | ---: | ---: | ---: |
| Manifest JSON | 8,699,179 B | 2,647,005 B | −69.57% |
| Identity records on the wire | 9,900 | 147 | −98.5% |
| Peak RSS (same analysis process) | 390,116 kB | 390,116 kB | decoded heap unchanged* |

\* The engine still analyses with the v3 registry; v4 is a wire projection. The
decoded-heap normalization is tracked separately by BR-AUDIT-18. The cache entry
and the response body shrink with the JSON column above.

## Wire shape

v4 keeps every v3 field. Only the identity representation changes:

```json
{
  "schema_version": "semantic_manifest.v4",
  "language": "it",
  "fingerprint": "…",
  "identities": [
    {
      "id": "person_01",
      "canonical_id": "person:alberto-stasi",
      "canonical_surface": "Alberto Stasi",
      "type": "PERSON",
      "mentions": [
        {"scene_id": "s01", "surface": "Alberto Stasi", "start_byte": 0, "end_byte": 13}
      ]
    }
  ],
  "scenes": [
    {"scene_id": "s01", "status": "valid", "identity_ids": ["person_01"], "…": "…"}
  ]
}
```

Rules:

1. `identities` appears once at the document level, in first-registration order.
2. Every identity object appears exactly once, with its complete mention list.
3. A scene carries `identity_ids` — the ids whose `mentions` include that scene,
   in document-table order. It never carries an `identities` key.
4. A scene with no identities carries `identity_ids: []`.
5. Every identity in the table is referenced by at least one scene.
6. Every non-identity field keeps the v3 value and encoding byte-for-byte. The
   projection is built from the canonical v3 wire bytes, not from a widened
   `serde_json::Value`, so f32 scores keep their short representation.

## Version routing

- `analyze_script` without `manifest_version` returns v3, unchanged.
- `"manifest_version": "semantic_manifest.v4"` (or `"v4"`) returns v4.
- Any other value fails closed with a `decode` error naming the unsupported
  version. The worker never falls back to v3 by field shape.
- The cache key is version-scoped; the disk cache always stores the canonical
  v3 manifest and re-projects on a v4 hit.
- PipelineGen accepts both versions and rejects anything else. A v4 manifest is
  hydrated into the existing scene view, using only the ids that scene
  references (never the cumulative registry).

## Consumer census (PipelineGen)

`scene.identities` is read at these sites, all of which only need identities
referenced by that scene:

| Site | Use |
| --- | --- |
| `semantic_engine.go` `projectSemanticSceneLanguage` | passes `scene.Identities` to person-role, age and person-relation projection |
| `semantic_engine_entity_linking.go` | resolves identity surfaces while validating linked entities |
| `semantic_engine.go` `semanticSceneCollectionsMissing` | requires the collection to be present (v4 hydration makes it non-nil) |
| `semantic_engine.go` `buildSemanticCandidateBundles` | derives `IdentityIDs` from the scene content, not from the wire list |

No Go consumer needs the cumulative registry. The document-level projection used
by the planner (`manifest.Entities` → `SemanticScript.Identities`) is unchanged.

## Cutover plan

1. **BrainRust ships v4 opt-in** (this change). v3 remains the default.
2. **PipelineGen accepts v3 and v4** (this change), so a mixed deployment is
   safe in both directions.
3. **Flip the producer per job/environment** by adding
   `"manifest_version": "semantic_manifest.v4"` to the `analyze_script` request.
   No consumer change is needed for the flip.
4. **Measure** JSON, cache entry and NDJSON bytes on the long corpus; compare
   with the table above.
5. **Retire v3** only after every consumer reads the document table directly
   (remove the hydration shim) and a release declares the removal. Until then,
   omitting `manifest_version` is the rollback.

## Files

| File | Owner | Purpose |
| --- | --- | --- |
| `manifest.schema.json` | BrainRust | v4 normalized envelope: global table, per-scene refs, no per-scene copy. |
| `goldens/manifest-v4.json` | BrainRust | Canonical v4 wire manifest used by both sides. |
| `goldens/manifest-v3.json` | BrainRust | The same content in v3, for parity review. |
| `validate.py` | BrainRust | Schema + cross-reference validation of both goldens. |
| `check_consumer_copy.sh` | BrainRust | Byte-parity gate for PipelineGen's copy. |
| `testdata/semantic-manifest/v4/` | PipelineGen | Byte-identical consumer copy. |

## Verify

```sh
cd BrainRust
python3 contract/semantic-manifest.v4/validate.py
bash contract/semantic-manifest.v4/check_consumer_copy.sh \
  ../refactored/internal/platform/media/rustexec/testdata/semantic-manifest/v4
cargo test -p semantic-core manifest::v4
cargo test -p semantic-engine v4_normalization
cargo test -p semantic-engine analyze_script_routes_manifest_versions
```

```sh
cd refactored
go test ./internal/platform/media/rustexec -run 'SemanticEngine.*V4|SemanticEngineAcceptsNormalized|SemanticEngineRejectsMalformed'
```

# Image catalog visual review (2026-11-10)

This note records what was actually visible in the images available in the
existing databases. It is evidence for the semantic annotations, not a new
catalog and not a biometric identity claim.

## Method

- Inspected 23 unique local image files under
  `refactored/data/media/images` and `refactored/data/cas`.
- Inspected 18 `web_image` candidates that match existing
  `entity_image_catalog_materializations` rows by content SHA-256.
- Downloaded review copies only under the ignored `.tmp/` directory. No
  third-party image bytes were added to the Rust repository.
- Compared repeated news images by eye. `identity_status = "verified_visual"`
  means the depicted person was consistent across the inspected set; it is not
  forensic face verification. `catalog_claimed` means only the existing catalog
  supplied the identity.

## What the local stores actually contain

| Source | Count | Finding |
| --- | ---: | --- |
| CAS JPEGs | 15 | Stock/US context frames: flags, St. Louis arch, fireworks, bridge, boxing ring, landscape, historic train, fire, gauges. None is Garlasco material. They were excluded from the identity seed. |
| `media/images/chiara-poggi` and `person` | 1 unique | A real low-light portrait of Chiara Poggi; duplicate files share SHA-256 `ec51ac88...`. It is not a catalog candidate, so it stays outside the catalog seed. |
| `media/images/searxng/chiara-poggi.jpg` | 1 | **Wrong label.** It shows three adults on a stage at the "Internazionale a Ferrara" festival, not Chiara Poggi. Documented as a mismatch; not imported as a person image. |
| `brainrust_garlasco_illustrative` | 6 | AI-generated illustrative scenes (street, courthouse, forensic lab, courtroom, archive, press). They are not documentary photographs and are not in the entity catalog. They must not be selected as identity images. |
| PostgreSQL `web_image` | 433 | 129 catalog materializations share a SHA-256 with a Postgres asset; 130 annotations were generated, including duplicates. |

## Curated visual annotations

`imagecatalog/fixtures/curated_visual_annotations_20261110.json` contains 20
reviewed annotations. The main corrections:

| Candidate | Visible content | Identities | Status |
| ---: | --- | --- | --- |
| 23256 | Andrea Sempio speaks to reporters outdoors | `person:andrea-sempio` | `verified_visual` |
| 23264 | Composite: Chiara portrait, a man with glasses, the Poggi house, footprints | `person:chiara-poggi` | `verified_visual`, `composite` |
| 23268 | A different brown-haired woman speaks to reporters | none | **`mismatch`** |
| 23294 | Composite: Sempio, a carabiniere, inset of Chiara | Sempio + Chiara | `verified_visual`, `composite` |
| 23296 | Solo Chiara portrait, urban background | Chiara | `verified_visual` (catalog row is semantically rejected) |
| 23298 | Solo Chiara portrait, dark background | Chiara | `verified_visual` (catalog row is semantically rejected) |
| 23302 | Chiara walking outdoors in a striped top, TV footage | Chiara | `verified_visual` (catalog row is semantically rejected) |
| 23304 | Solo Stasi portrait | Stasi | `verified_visual` (catalog row is semantically rejected) |
| 23462 | Solo Stasi portrait, yellow room | Stasi | `verified_visual` |
| 23528 | Composite: Sempio, Chiara, Stasi, house, memorial | all three | `verified_visual`, `composite` |
| 34357 | Triptych: Sempio, Chiara, Stasi | all three | `verified_visual`, `composite` |
| 34464 | Same bytes as 23294 | Sempio + Chiara | `verified_visual`, `composite` |
| 34757 | TV triptych with Sempio, Chiara, Stasi and an overlay | all three | `verified_visual`, `composite` |
| 49875 | Woman in front of a poster wall; identity not confirmed | none | `unverified` |
| 49960 | Solo Chiara portrait with a glass and straw | Chiara | `verified_visual` |
| 54133, 54138, 54143, 54149, 54153 | Courthouse exteriors/interiors, archive boxes | none | **`mismatch`** |

The catalog also contains 130 automatic annotations derived from
`postgres.media_assets.metadata_json.description`. Those are
`catalog_claimed`/`unverified`; they preserve the existing description without
pretending it was visually reviewed.

## Consequences for selection

On the real seed:

- A solo "Chiara Poggi" query selects **49960**, the reviewed solo portrait,
  ahead of the composite collages.
- A scene naming both Chiara and Alberto Stasi promotes **34757**, the triptych
  whose subjects include both identities, through the bounded context bonus.
- The mismatched 23268 and the five courthouse/archive rows are ineligible and
  their reasons say so.
- 49875 remains usable only as an unverified fallback and is demoted.

## Still open

- Only the 20 curated rows are visually reviewed; the remaining automatic
  annotations need review before they can be called correct descriptions.
- Candidates 23256, 23296, 23298, 23302, 23304 and the courthouse rows are
  semantically `rejected` in the owner catalog. The annotations describe what
  is visible, but do not silently recertify them. Recertification, if desired,
  belongs to PipelineGen's existing catalog workflow.
- The local Chiara portrait and the six synthetic Garlasco illustrations are
  not part of the existing entity catalog. They were not turned into a second
  catalog; doing so would require an explicit owner-side import decision.

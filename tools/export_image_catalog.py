#!/usr/bin/env python3
"""Export the existing PipelineGen image catalog as an `imagecatalog` snapshot.

The Rust worker must not open PipelineGen's SQLite database. This tool is the
explicit read-only bridge: it reads `entity_image_catalog_entities`,
`entity_image_catalog_candidates` and `entity_image_catalog_materializations`
from an existing database and writes the JSON `catalog` object expected by
`imagecatalog.v1`.

Semantic annotations are a sidecar, not a second catalog: they are keyed by
the existing `candidate_id` and are merged into the snapshot. `--annotations`
can be passed more than once; later files override earlier ones, so an
automatic metadata export can be followed by a curated visual-review overlay.
`--media-annotations` reads the existing `media_assets` table in the same
database and derives an unverified annotation from its metadata.

No table is created or modified. The database is opened with `mode=ro`.

Usage:
    python3 tools/export_image_catalog.py --db /path/to/pipelinegen.db \
        --annotations annotations.json --out /tmp/image-catalog.json
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from pathlib import Path
from urllib.parse import quote

IDENTITY_STATUSES = {"", "verified_visual", "catalog_claimed", "unverified", "mismatch"}
DEPICTIONS = {
    "",
    "unknown",
    "photograph",
    "composite",
    "illustration",
    "synthetic",
    "generated",
}
REVIEW_STATUSES = {"", "reviewed", "unverified", "pending"}


def table_columns(connection: sqlite3.Connection, table: str) -> set[str]:
    rows = connection.execute(f"PRAGMA table_info({table})").fetchall()
    if not rows:
        raise SystemExit(f"export_image_catalog: missing table {table}")
    return {row[1] for row in rows}


def pick(row: sqlite3.Row, *names: str):
    for name in names:
        try:
            return row[name]
        except (IndexError, KeyError):
            continue
    return None


def decode_text(value) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", "replace")
    return str(value)


def string_value(row: sqlite3.Row, *names: str) -> str:
    return decode_text(pick(row, *names))


def int_value(row: sqlite3.Row, *names: str) -> int:
    value = pick(row, *names)
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return 0


def float_value(row: sqlite3.Row, *names: str) -> float:
    value = pick(row, *names)
    try:
        return float(value or 0.0)
    except (TypeError, ValueError):
        return 0.0


def parse_json_object(value) -> dict:
    if not value:
        return {}
    if isinstance(value, dict):
        return value
    try:
        parsed = json.loads(decode_text(value))
    except (TypeError, ValueError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def parse_tags(value) -> list[str]:
    if not value:
        return []
    if isinstance(value, list):
        return [str(tag).strip() for tag in value if str(tag).strip()]
    text = decode_text(value).strip()
    if not text:
        return []
    try:
        parsed = json.loads(text)
    except (TypeError, ValueError):
        parsed = None
    if isinstance(parsed, list):
        return [str(tag).strip() for tag in parsed if str(tag).strip()]
    return [part.strip() for part in text.split(",") if part.strip()]


def load_annotation_file(path: Path) -> list[dict]:
    try:
        body = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise SystemExit(f"export_image_catalog: cannot read {path}: {exc}") from exc
    if isinstance(body, dict):
        body = body.get("annotations", [])
    if not isinstance(body, list):
        raise SystemExit(f"export_image_catalog: {path} must contain a JSON array")
    return [annotation for annotation in body if isinstance(annotation, dict)]


def normalize_annotation(annotation: dict) -> dict:
    subjects = annotation.get("subjects") or []
    tags = annotation.get("tags") or []
    if not isinstance(subjects, list) or not all(
        isinstance(subject, str) for subject in subjects
    ):
        raise SystemExit("export_image_catalog: annotation subjects must be strings")
    if not isinstance(tags, list) or not all(isinstance(tag, str) for tag in tags):
        raise SystemExit("export_image_catalog: annotation tags must be strings")
    return {
        "candidate_id": int(annotation.get("candidate_id", 0)),
        "description": str(annotation.get("description") or "").strip(),
        "subjects": [subject.strip() for subject in subjects if subject.strip()],
        "tags": [tag.strip() for tag in tags if tag.strip()],
        "depiction": str(annotation.get("depiction") or "").strip().lower(),
        "identity_status": str(annotation.get("identity_status") or "")
        .strip()
        .lower(),
        "review_status": str(annotation.get("review_status") or "").strip().lower(),
        "source": str(annotation.get("source") or "").strip(),
        "evidence": str(annotation.get("evidence") or "").strip(),
    }


def validate_annotations(
    annotations: dict[int, dict], candidate_entity: dict[int, str]
) -> None:
    for candidate_id, annotation in annotations.items():
        if candidate_id not in candidate_entity:
            raise SystemExit(
                f"export_image_catalog: annotation references unknown candidate "
                f"{candidate_id}"
            )
        if annotation["identity_status"] not in IDENTITY_STATUSES:
            raise SystemExit(
                f"export_image_catalog: candidate {candidate_id} has unknown "
                f"identity_status {annotation['identity_status']!r}"
            )
        if annotation["depiction"] not in DEPICTIONS:
            raise SystemExit(
                f"export_image_catalog: candidate {candidate_id} has unknown "
                f"depiction {annotation['depiction']!r}"
            )
        if annotation["review_status"] not in REVIEW_STATUSES:
            raise SystemExit(
                f"export_image_catalog: candidate {candidate_id} has unknown "
                f"review_status {annotation['review_status']!r}"
            )
        primary = candidate_entity[candidate_id]
        if (
            annotation["subjects"]
            and primary
            and primary not in annotation["subjects"]
            and annotation["identity_status"] != "mismatch"
        ):
            raise SystemExit(
                f"export_image_catalog: candidate {candidate_id} annotation "
                f"subjects do not include {primary}; use identity_status "
                f"'mismatch' to document a wrong catalog label"
            )


def auto_media_annotations(
    connection: sqlite3.Connection,
    materializations: list[dict],
    candidate_entity: dict[int, str],
) -> dict[int, dict]:
    if not connection.execute(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='media_assets'"
    ).fetchone():
        return {}
    # `name` is not needed and may contain non-UTF-8 bytes. Read only the
    # annotation columns as BLOBs, decode explicitly, and skip corrupted
    # descriptions rather than emitting replacement garbage.
    media_rows: dict[str, dict[str, str]] = {}
    for row in connection.execute(
        "SELECT CAST(id AS BLOB) AS id, "
        "CAST(metadata_json AS BLOB) AS metadata_json, "
        "CAST(title AS BLOB) AS title, "
        "CAST(search_text AS BLOB) AS search_text, "
        "CAST(tags AS BLOB) AS tags "
        "FROM media_assets"
    ):
        asset_id = decode_text(row["id"]).strip()
        if not asset_id:
            continue
        media_rows[asset_id] = {
            "metadata_json": decode_text(row["metadata_json"]),
            "title": decode_text(row["title"]),
            "search_text": decode_text(row["search_text"]),
            "tags": decode_text(row["tags"]),
        }
    annotations: dict[int, dict] = {}
    for materialization in materializations:
        asset_id = materialization.get("asset_id", "")
        row = media_rows.get(asset_id)
        if row is None:
            continue
        metadata = parse_json_object(row["metadata_json"])
        description = str(metadata.get("description") or "").strip()
        source = "media_assets.metadata_json.description"
        if not description:
            description = row["title"].strip()
            source = "media_assets.title"
        if not description:
            description = row["search_text"].strip()
            source = "media_assets.search_text"
        if not description or "\ufffd" in description:
            continue
        primary = candidate_entity.get(materialization["candidate_id"], "")
        annotations[materialization["candidate_id"]] = normalize_annotation(
            {
                "candidate_id": materialization["candidate_id"],
                "description": description,
                "subjects": [primary] if primary.startswith("person:") else [],
                "tags": parse_tags(row["tags"]),
                "depiction": "unknown",
                "identity_status": "catalog_claimed",
                "review_status": "unverified",
                "source": source,
                "evidence": f"media_asset={asset_id}",
            }
        )
    return annotations


def export(
    db_path: Path,
    annotation_paths: list[Path],
    include_media_annotations: bool,
) -> dict:
    uri = f"file:{quote(str(db_path.resolve()))}?mode=ro"
    connection = sqlite3.connect(uri, uri=True)
    connection.row_factory = sqlite3.Row
    try:
        _entity_columns = table_columns(connection, "entity_image_catalog_entities")
        candidate_columns = table_columns(connection, "entity_image_catalog_candidates")
        materialization_columns = table_columns(
            connection, "entity_image_catalog_materializations"
        )

        entities = []
        for row in connection.execute(
            "SELECT * FROM entity_image_catalog_entities "
            "ORDER BY canonical_entity_id"
        ):
            entities.append(
                {
                    "canonical_entity_id": string_value(
                        row, "canonical_entity_id"
                    ),
                    "entity_type": string_value(row, "entity_type") or "PERSON",
                    "canonical_name": string_value(row, "canonical_name"),
                }
            )

        candidates = []
        candidate_entity: dict[int, str] = {}
        for row in connection.execute(
            "SELECT * FROM entity_image_catalog_candidates "
            "ORDER BY candidate_id"
        ):
            candidate = {
                "candidate_id": int_value(row, "candidate_id"),
                "canonical_entity_id": string_value(
                    row, "canonical_entity_id"
                ),
                "provider": string_value(row, "provider"),
                "rank": int_value(row, "rank") or 1,
                "source_url": string_value(row, "source_url"),
                "thumbnail_url": string_value(row, "thumbnail_url"),
                "width": int_value(row, "width"),
                "height": int_value(row, "height"),
                "status": string_value(row, "status") or "fresh",
                "semantic_status": string_value(row, "semantic_status")
                or "unknown",
                "semantic_score": float_value(row, "semantic_score"),
                "technical_score": float_value(row, "technical_score"),
                "quality_reason": string_value(row, "quality_reason"),
                "last_seen_at": string_value(row, "last_seen_at"),
            }
            if "source_page_url" in candidate_columns:
                candidate["source_page_url"] = string_value(row, "source_page_url")
            candidates.append(candidate)
            candidate_entity[candidate["candidate_id"]] = candidate[
                "canonical_entity_id"
            ]

        materializations = []
        for row in connection.execute(
            "SELECT * FROM entity_image_catalog_materializations "
            "ORDER BY candidate_id"
        ):
            hash_column = next(
                (
                    column
                    for column in ("content_sha256", "file_hash", "legacy_file_md5")
                    if column in materialization_columns
                ),
                None,
            )
            materialization = {
                "candidate_id": int_value(row, "candidate_id"),
                "asset_id": string_value(row, "asset_id"),
                "content_sha256": string_value(row, hash_column)
                if hash_column
                else "",
                "drive_file_id": string_value(row, "drive_file_id"),
                "drive_link": string_value(row, "drive_link"),
                "status": string_value(row, "status"),
                "materialized_at": string_value(row, "materialized_at"),
                "last_verified_at": string_value(row, "last_verified_at"),
            }
            materializations.append(materialization)

        annotations: dict[int, dict] = {}
        if include_media_annotations:
            annotations.update(
                auto_media_annotations(
                    connection, materializations, candidate_entity
                )
            )
        for path in annotation_paths:
            for raw in load_annotation_file(path):
                annotation = normalize_annotation(raw)
                annotations[annotation["candidate_id"]] = annotation
        validate_annotations(annotations, candidate_entity)

        snapshot = {
            "entities": entities,
            "candidates": candidates,
            "materializations": materializations,
        }
        if annotations:
            snapshot["annotations"] = [
                annotations[candidate_id] for candidate_id in sorted(annotations)
            ]
        return snapshot
    finally:
        connection.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--db",
        required=True,
        type=Path,
        help="existing PipelineGen SQLite database (opened read-only)",
    )
    parser.add_argument(
        "--annotations",
        action="append",
        default=[],
        type=Path,
        help="JSON annotation overlay; repeat to apply later files on top",
    )
    parser.add_argument(
        "--media-annotations",
        action="store_true",
        help="also derive unverified annotations from media_assets metadata",
    )
    parser.add_argument(
        "--out",
        type=Path,
        help="write the JSON snapshot here instead of stdout",
    )
    args = parser.parse_args()

    if not args.db.is_file():
        raise SystemExit(f"export_image_catalog: database not found: {args.db}")
    for path in args.annotations:
        if not path.is_file():
            raise SystemExit(f"export_image_catalog: annotations not found: {path}")

    snapshot = export(args.db, args.annotations, args.media_annotations)
    encoded = json.dumps(snapshot, ensure_ascii=False, indent=2) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(encoded, encoding="utf-8")
        print(
            f"exported {len(snapshot['entities'])} entities, "
            f"{len(snapshot['candidates'])} candidates, "
            f"{len(snapshot['materializations'])} materializations, "
            f"{len(snapshot.get('annotations', []))} annotations to {args.out}",
            file=sys.stderr,
        )
    else:
        sys.stdout.write(encoded)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

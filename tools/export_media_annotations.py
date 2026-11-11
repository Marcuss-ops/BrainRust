#!/usr/bin/env python3
"""Derive semantic annotations from an existing PostgreSQL media_assets export.

The Rust worker never opens PostgreSQL. This tool joins a CSV produced by
`psql --csv` against the existing SQLite catalog by content SHA-256, then emits
annotations keyed by `candidate_id`. It does not create a catalog or copy
image bytes.

The resulting description is the one already stored in
`media_assets.metadata_json.description`; it is marked `catalog_claimed` and
`unverified` because it has not necessarily been visually reviewed. A curated
overlay can be applied later with `export_image_catalog.py --annotations`.

Usage:
    docker exec <postgres> psql ... --csv -c "SELECT ..." > /tmp/web_images.csv
    python3 tools/export_media_annotations.py \
        --db /path/to/pipelinegen.db \
        --web-images-csv /tmp/web_images.csv \
        --out annotations.json
"""

from __future__ import annotations

import argparse
import csv
import json
import sqlite3
from pathlib import Path
from urllib.parse import quote


def parse_metadata(value: str) -> dict:
    if not value:
        return {}
    try:
        parsed = json.loads(value)
    except (TypeError, ValueError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def parse_tags(value: str) -> list[str]:
    if not value:
        return []
    try:
        parsed = json.loads(value)
    except (TypeError, ValueError):
        parsed = None
    if isinstance(parsed, list):
        return [str(tag).strip() for tag in parsed if str(tag).strip()]
    return [part.strip() for part in value.split(",") if part.strip()]


def load_web_images(path: Path) -> list[dict]:
    rows = []
    with path.open(newline="", encoding="utf-8") as handle:
        reader = csv.DictReader(handle)
        required = {"id", "content_sha256", "metadata"}
        missing = required - set(reader.fieldnames or [])
        if missing:
            raise SystemExit(
                "export_media_annotations: CSV is missing columns: "
                + ", ".join(sorted(missing))
            )
        for row in reader:
            metadata = parse_metadata(row.get("metadata", ""))
            description = str(metadata.get("description") or "").strip()
            if not description:
                continue
            rows.append(
                {
                    "id": str(row.get("id", "")).strip(),
                    "content_sha256": str(row.get("content_sha256", ""))
                    .strip()
                    .lower(),
                    "name": str(row.get("name", "")).strip(),
                    "description": description,
                    "tags": parse_tags(row.get("tags", "")),
                }
            )
    return rows


def load_catalog_materializations(db_path: Path) -> list[sqlite3.Row]:
    uri = f"file:{quote(str(db_path.resolve()))}?mode=ro"
    connection = sqlite3.connect(uri, uri=True)
    connection.row_factory = sqlite3.Row
    try:
        return connection.execute(
            """
            SELECT m.candidate_id, m.asset_id, m.content_sha256,
                   c.canonical_entity_id
            FROM entity_image_catalog_materializations m
            JOIN entity_image_catalog_candidates c
              ON c.candidate_id = m.candidate_id
            WHERE length(m.content_sha256) = 64
            ORDER BY m.candidate_id
            """
        ).fetchall()
    finally:
        connection.close()


def build_annotations(
    web_images: list[dict], materializations: list[sqlite3.Row]
) -> list[dict]:
    by_sha = {row["content_sha256"]: row for row in web_images}
    annotations: dict[int, dict] = {}
    for materialization in materializations:
        image = by_sha.get(materialization["content_sha256"].lower())
        if image is None:
            continue
        candidate_id = int(materialization["candidate_id"])
        if candidate_id in annotations:
            continue
        primary = str(materialization["canonical_entity_id"]).strip()
        annotations[candidate_id] = {
            "candidate_id": candidate_id,
            "description": image["description"],
            "subjects": [primary] if primary.startswith("person:") else [],
            "tags": image["tags"],
            "depiction": "unknown",
            "identity_status": "catalog_claimed",
            "review_status": "unverified",
            "source": "postgres.media_assets.metadata_json.description",
            "evidence": (
                f"media_asset_id={image['id']}; "
                f"sha256={image['content_sha256']}"
            ),
        }
    return [annotations[candidate_id] for candidate_id in sorted(annotations)]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, type=Path)
    parser.add_argument("--web-images-csv", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument(
        "--generated-at",
        default="2026-11-10T00:00:00Z",
        help="deterministic generation timestamp",
    )
    args = parser.parse_args()

    if not args.db.is_file():
        raise SystemExit(f"export_media_annotations: database not found: {args.db}")
    if not args.web_images_csv.is_file():
        raise SystemExit(
            f"export_media_annotations: CSV not found: {args.web_images_csv}"
        )

    web_images = load_web_images(args.web_images_csv)
    materializations = load_catalog_materializations(args.db)
    annotations = build_annotations(web_images, materializations)
    payload = {
        "schema_version": "image_annotations.v1",
        "generated_at": args.generated_at,
        "source": "postgres.media_assets.metadata_json.description",
        "annotations": annotations,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(
        f"exported {len(annotations)} annotations from "
        f"{len(web_images)} web images and {len(materializations)} materializations "
        f"to {args.out}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Validate the shared semantic_manifest.v4 goldens.

Run from anywhere:
    python3 contract/semantic-manifest.v4/validate.py

The schema validates the normalized envelope; this script also enforces the
cross-references the schema cannot express (unique ids, every reference
resolvable, every identity referenced, mention scene ids real, no per-scene
copy). The v3 golden is checked for the historical shape so a reviewer can see
the delta side by side.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    from jsonschema import Draft7Validator
except ImportError:  # pragma: no cover - exercised by the CI setup path
    sys.exit(
        "jsonschema is required: install it from "
        "contract/mediaexec.v2/requirements.txt"
    )

ROOT = Path(__file__).resolve().parent
V4 = ROOT / "goldens" / "manifest-v4.json"
V3 = ROOT / "goldens" / "manifest-v3.json"


def fail(message: str) -> None:
    print(f"semantic-manifest.v4 contract: {message}", file=sys.stderr)
    raise SystemExit(1)


def load(path: Path) -> dict:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def validate_schema(schema: dict, document: dict) -> None:
    errors = sorted(Draft7Validator(schema).iter_errors(document), key=lambda error: list(error.path))
    if not errors:
        return
    details = []
    for error in errors:
        location = "/".join(str(part) for part in error.path) or "<root>"
        details.append(f"{location}: {error.message}")
    fail("manifest-v4.json does not match the schema:\n  " + "\n  ".join(details))


def validate_v4(document: dict) -> tuple[int, int]:
    if document.get("schema_version") != "semantic_manifest.v4":
        fail("manifest-v4.json: schema_version is not semantic_manifest.v4")
    scenes = document.get("scenes")
    identities = document.get("identities")
    if not isinstance(scenes, list) or not isinstance(identities, list):
        fail("manifest-v4.json: scenes and identities must be arrays")

    scene_ids = set()
    for scene in scenes:
        scene_id = scene.get("scene_id")
        if scene_id in scene_ids:
            fail(f"duplicate scene_id {scene_id!r}")
        scene_ids.add(scene_id)
        if "identities" in scene:
            fail(f"scene {scene_id!r} carries a per-scene identity copy")

    by_id = {}
    for identity in identities:
        identity_id = identity.get("id")
        if identity_id in by_id:
            fail(f"duplicate identity id {identity_id!r}")
        by_id[identity_id] = identity
        mentions = identity.get("mentions") or []
        if not mentions:
            fail(f"identity {identity_id!r} has no mentions")
        for mention in mentions:
            if mention.get("scene_id") not in scene_ids:
                fail(
                    f"identity {identity_id!r} mentions unknown scene "
                    f"{mention.get('scene_id')!r}"
                )
            if mention.get("end_byte", 0) < mention.get("start_byte", 0):
                fail(f"identity {identity_id!r} has an inverted mention span")

    referenced = set()
    for scene in scenes:
        seen = set()
        for identity_id in scene.get("identity_ids", []):
            if identity_id in seen:
                fail(f"scene {scene['scene_id']!r} repeats identity {identity_id!r}")
            seen.add(identity_id)
            if identity_id not in by_id:
                fail(f"scene {scene['scene_id']!r} references unknown identity {identity_id!r}")
            referenced.add(identity_id)
        # The reference list must match the mentions exactly.
        expected = {
            identity_id
            for identity_id, identity in by_id.items()
            if any(mention.get("scene_id") == scene["scene_id"] for mention in identity["mentions"])
        }
        if seen != expected:
            missing = sorted(expected - seen)
            extra = sorted(seen - expected)
            fail(
                f"scene {scene['scene_id']!r} identity_ids drift: "
                f"missing={missing} extra={extra}"
            )

    unreferenced = sorted(set(by_id) - referenced)
    if unreferenced:
        fail(f"identities not referenced by any scene: {unreferenced}")
    return len(scenes), len(by_id)


def validate_v3(document: dict) -> int:
    if document.get("schema_version") != "semantic_manifest.v3":
        fail("manifest-v3.json: schema_version is not semantic_manifest.v3")
    scenes = document.get("scenes") or []
    if not scenes:
        fail("manifest-v3.json: no scenes")
    for scene in scenes:
        if "identities" not in scene:
            fail(f"v3 scene {scene.get('scene_id')!r} omits identities")
        if "identity_ids" in scene:
            fail(f"v3 scene {scene.get('scene_id')!r} carries identity_ids")
    return len(scenes)


def main() -> None:
    schema = load(ROOT / "manifest.schema.json")
    Draft7Validator.check_schema(schema)

    v4 = load(V4)
    validate_schema(schema, v4)
    scene_count, identity_count = validate_v4(v4)
    v3_scene_count = validate_v3(load(V3))

    if scene_count != v3_scene_count:
        fail(
            f"golden scene drift: v4 has {scene_count}, v3 has {v3_scene_count}"
        )

    print(
        f"✅ semantic_manifest.v4 contract: {scene_count} scenes, "
        f"{identity_count} identities, v3 golden checked"
    )


if __name__ == "__main__":
    main()

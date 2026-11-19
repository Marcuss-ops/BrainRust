#!/usr/bin/env python3
"""Validate the curated entity index and emit a deterministic alias collision report."""
import argparse
import hashlib
import json
import re
import sys
import unicodedata
from collections import defaultdict
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INDEX = ROOT / "lexicon/entity_index_v1.json"
SOURCES = ROOT / "lexicon/entity_index_sources_v1.json"
REPORT = ROOT / "lexicon/entity_index_collisions_v1.json"
LIMIT = 5 * 1024 * 1024


def norm(value):
    value = unicodedata.normalize("NFKC", value).casefold()
    return " ".join(re.sub(r"[^\w]+", " ", value).split())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    raw = INDEX.read_bytes()
    index = json.loads(raw)
    sources = json.loads(SOURCES.read_text(encoding="utf-8"))
    errors = []
    if len(raw) > LIMIT:
        errors.append(f"packaged index exceeds {LIMIT} bytes")
    if index.get("version") != "entity_index.v1":
        errors.append("unexpected index version")
    if sources.get("schema_version") != "entity_index_sources.v1":
        errors.append("unexpected source manifest schema")
    try:
        date.fromisoformat(sources["retrieval_date"])
    except (KeyError, ValueError):
        errors.append("manifest retrieval_date must be an ISO date")
    if sources.get("license") != "CC0-1.0" or not sources.get("source_url"):
        errors.append("manifest must identify source URL and CC0-1.0 license")

    ids, aliases = set(), defaultdict(list)
    for row in index.get("entries", []):
        entity_id = row.get("id", "")
        if not entity_id or entity_id in ids:
            errors.append(f"missing or duplicate id: {entity_id!r}")
        ids.add(entity_id)
        for field in ("canonical_name", "entity_type", "gloss"):
            if not isinstance(row.get(field), str) or not row[field].strip():
                errors.append(f"{entity_id}: missing {field}")
        if not isinstance(row.get("aliases"), dict) or not row["aliases"]:
            errors.append(f"{entity_id}: aliases must be a nonempty language map")
        aliases[norm(row.get("canonical_name", ""))].append(
            {"id": entity_id, "canonical_name": row.get("canonical_name", ""), "alias": row.get("canonical_name", "")}
        )
        for lang, values in row.get("aliases", {}).items():
            if not re.fullmatch(r"[a-z]{2,3}(?:-[A-Za-z0-9]{2,8})*", lang):
                errors.append(f"{entity_id}: invalid language tag {lang!r}")
            seen = set()
            for value in values:
                key = norm(value)
                if not key or key in seen:
                    errors.append(f"{entity_id}: empty/duplicate alias {value!r}")
                seen.add(key)
                aliases[key].append({"id": entity_id, "canonical_name": row["canonical_name"], "alias": value})
        grounded_terms = set(row.get("context_terms", [])) | set(row.get("counter_terms", []))
        if len(grounded_terms) < 3:
            errors.append(f"{entity_id}: needs at least three context/counter terms")
        provenance = sources.get("entries", {}).get(entity_id)
        if not provenance or not provenance.get("item_url") or not provenance.get("source"):
            errors.append(f"{entity_id}: missing row provenance")

    if set(sources.get("entries", {})) != ids:
        errors.append("manifest IDs must exactly match packaged index IDs")
    collisions = [{"normalized_surface": key, "intended": key in sources.get("intended_collisions", []),
                   "senses": sorted({r["id"]: r for r in rows}.values(), key=lambda r: r["id"])}
                  for key, rows in sorted(aliases.items()) if len({r["id"] for r in rows}) > 1]
    collision_keys = {row["normalized_surface"] for row in collisions}
    intended_keys = set(sources.get("intended_collisions", []))
    for key in sorted(collision_keys - intended_keys):
        errors.append(f"unreviewed alias collision: {key!r}")
    for key in sorted(intended_keys - collision_keys):
        errors.append(f"manifest declares absent collision: {key!r}")
    document = {
        "schema_version": "entity_index_collisions.v1",
        "index_sha256": hashlib.sha256(raw).hexdigest(),
        "entry_count": len(ids),
        "collision_count": len(collisions),
        "collisions": collisions,
    }
    expected = (json.dumps(document, ensure_ascii=False, indent=2) + "\n").encode()
    if args.check:
        if not REPORT.exists() or REPORT.read_bytes() != expected:
            errors.append("collision report is stale; run build_entity_index.py")
    else:
        REPORT.write_bytes(expected)
    if errors:
        print("\n".join(f"ERROR: {error}" for error in errors), file=sys.stderr)
        return 1
    print(f"entity index valid: {len(ids)} entries, {len(collisions)} alias collisions")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""Build a deterministic multilingual country/capital place index from a frozen Wikidata snapshot.

The checked-in source snapshot records its query, retrieval date and SHA-256.
Wikidata structured data is CC0. Refreshing the source is an explicit operation;
normal builds and --check never access the network.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SOURCE = ROOT / "lexicon/source/wikidata_country_capitals_20261121.json"
DEFAULT_OUTPUT = ROOT / "lexicon/place_index_v1.json"
KINDS = {"country", "city"}
LANGUAGES = ("en", "it", "de", "es", "fr", "id", "nl", "pl", "pt", "ru", "tr")
LABEL_SUFFIX = {
    language: language.upper() if language in {"en", "it"} else language.title()
    for language in LANGUAGES
}
POINT = re.compile(r"^Point\((-?[0-9.]+) (-?[0-9.]+)\)$")


def point(value: str) -> tuple[float, float]:
    match = POINT.fullmatch(value)
    if not match:
        raise ValueError(f"invalid Wikidata coordinate: {value!r}")
    lon, lat = map(float, match.groups())
    return lat, lon


def binding(row: dict, key: str, fallback: str | None = None) -> str:
    value = row.get(key, {}).get("value")
    if value:
        return value
    if fallback:
        return binding(row, fallback)
    raise ValueError(f"missing {key} in Wikidata source row")


def qid(uri: str) -> str:
    value = uri.rsplit("/", 1)[-1]
    if not re.fullmatch(r"Q[1-9][0-9]*", value):
        raise ValueError(f"invalid Wikidata item URI: {uri!r}")
    return f"wd:{value}"


def build(source: dict) -> dict:
    if source.get("schema") != "wikidata_country_capitals.snapshot.v1":
        raise ValueError("source snapshot has the wrong schema")
    if source.get("license") != "CC0-1.0" or not source.get("retrieved_on"):
        raise ValueError("source snapshot must record CC0 license and retrieval date")
    rows = source.get("results", [])
    if not rows:
        raise ValueError("source snapshot has no result rows")
    canonical_results = json.dumps(
        rows, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    result_sha256 = hashlib.sha256(canonical_results).hexdigest()
    if source.get("result_sha256") != result_sha256:
        raise ValueError("source result checksum does not match the frozen rows")
    countries: dict[str, dict] = {}
    cities: dict[str, dict] = {}
    # Sort before first-value selection so duplicate statements are resolved
    # deterministically. Every source country and capital row remains traceable.
    rows = sorted(rows, key=lambda row: (
        binding(row, "iso"), binding(row, "country"), binding(row, "capital"),
        binding(row, "countryCoord"), binding(row, "capitalCoord"),
    ))
    for row in rows:
        country_id = qid(binding(row, "country"))
        capital_id = qid(binding(row, "capital"))
        iso = binding(row, "iso")
        country_lat, country_lon = point(binding(row, "countryCoord"))
        city_lat, city_lon = point(binding(row, "capitalCoord"))
        country_en = binding(row, "countryEN")
        city_en = binding(row, "capitalEN")
        country_aliases = {
            language: [binding(row, f"country{LABEL_SUFFIX[language]}", "countryEN")]
            for language in LANGUAGES
        }
        city_aliases = {
            language: [binding(row, f"capital{LABEL_SUFFIX[language]}", "capitalEN")]
            for language in LANGUAGES
        }
        countries.setdefault(country_id, {
            "id": country_id,
            "name": country_en,
            "kind": "country",
            "lat": country_lat,
            "lon": country_lon,
            "parent_ids": [],
            "aliases": country_aliases,
            "context_terms": [iso],
            "counter_terms": [],
        })
        if capital_id == country_id:
            # Some city-states point to themselves as their capital. Keep the
            # single stable Wikidata identity as a country and preserve the
            # capital spelling as an alias instead of creating a self-parent.
            for language in LANGUAGES:
                countries[country_id]["aliases"][language].extend(city_aliases[language])
            continue
        city = cities.setdefault(capital_id, {
            "id": capital_id,
            "name": city_en,
            "kind": "city",
            "lat": city_lat,
            "lon": city_lon,
            "parent_ids": [],
            "aliases": {language: [] for language in LANGUAGES},
            "context_terms": [],
            "counter_terms": [],
        })
        city["parent_ids"].append(country_id)
        for language in LANGUAGES:
            city["aliases"][language].extend(city_aliases[language])

    entries = list(countries.values()) + list(cities.values())
    for entry in entries:
        entry["parent_ids"] = sorted(set(entry["parent_ids"]))
        for language in entry["aliases"]:
            entry["aliases"][language] = sorted(set(entry["aliases"][language]))
    entries.sort(key=lambda entry: (entry["kind"], entry["id"]))
    collisions: dict[str, set[str]] = defaultdict(set)
    for entry in entries:
        for surfaces in entry["aliases"].values():
            for surface in surfaces:
                collisions[surface.casefold()].add(entry["id"])
    collision_report = [
        {"alias": alias, "ids": sorted(ids)}
        for alias, ids in sorted(collisions.items()) if len(ids) > 1
    ]
    index = {
        "schema": "place_index.v1",
        "version": "place_index.v1-wikidata-country-capitals-11-language-2026-11-21",
        "source": source["source"],
        "license": source["license"],
        "retrieved_on": source["retrieved_on"],
        "query": source["query"],
        "source_sha256": result_sha256,
        "entries": entries,
        "alias_collisions": collision_report,
    }
    validate(index)
    return index


def validate(index: dict) -> None:
    if index.get("schema") != "place_index.v1":
        raise ValueError("expected place_index.v1")
    entries = index.get("entries", [])
    ids = [entry.get("id") for entry in entries]
    if not ids or len(set(ids)) != len(ids):
        raise ValueError("place IDs must be present and unique")
    known = set(ids)
    for entry in entries:
        if entry.get("kind") not in KINDS:
            raise ValueError(f"invalid place kind: {entry.get('kind')!r}")
        lat, lon = entry.get("lat"), entry.get("lon")
        if not isinstance(lat, (int, float)) or not -90 <= lat <= 90:
            raise ValueError(f"latitude out of range for {entry['id']}")
        if not isinstance(lon, (int, float)) or not -180 <= lon <= 180:
            raise ValueError(f"longitude out of range for {entry['id']}")
        if not set(entry.get("parent_ids", [])) <= known:
            raise ValueError(f"unresolved parent for {entry['id']}")
        aliases = entry.get("aliases", {})
        if not set(LANGUAGES) <= set(aliases) or any(
            not aliases.get(language) for language in LANGUAGES
        ):
            raise ValueError(f"missing localized aliases for {entry['id']}")
    if len(json.dumps(index, ensure_ascii=False).encode("utf-8")) > 5 * 1024 * 1024:
        raise ValueError("place index exceeds 5 MiB")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--check", action="store_true", help="verify output; never write")
    args = parser.parse_args()
    source = json.loads(args.source.read_text(encoding="utf-8"))
    expected = json.dumps(build(source), ensure_ascii=False, indent=2) + "\n"
    if args.check:
        if not args.output.is_file() or args.output.read_text(encoding="utf-8") != expected:
            print(f"stale place index: regenerate {args.output}", file=sys.stderr)
            return 1
        print("place index: schema, coordinates, parents, collisions and size valid")
        return 0
    args.output.write_text(expected, encoding="utf-8")
    data = json.loads(expected)
    print(
        f"wrote {args.output}: {len(data['entries'])} entries, "
        f"{len(data['alias_collisions'])} declared alias collisions, "
        f"{args.output.stat().st_size} bytes"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

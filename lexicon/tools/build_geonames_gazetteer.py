#!/usr/bin/env python3
"""Build the real place index behind `lexicon/entity_gazetteer.json`.

Source: GeoNames `cities500.zip` (https://download.geonames.org/export/dump/),
licensed CC BY 4.0. The dump is filtered to populated places (feature class P)
with at least `--min-population` inhabitants, plus national capitals. The
result is merged into the existing gazetteer JSON as `places`, preserving every
hand-curated field (brands, persons, works, ...).

Each place carries its primary name, country, population and the aliases the
extractor can type as LOCATION. Aliases are lowercased and kept for every
script the pipeline writes in (Latin, Greek, Cyrillic, Han, kana, Hangul,
Arabic, Devanagari, Thai); the runtime lookup applies a capitalization guard
only for cased scripts, so a common noun that happens to equal a foreign place
name ("casa" -> Chas) does not become a location while "北京" still does. When `wordfreq` is installed (`pip install wordfreq`),
the generator also rebuilds the per-language `place_ambiguities` table; without
it, the existing ambiguity table is preserved.

Usage:
  python3 build_geonames_gazetteer.py --input cities500.txt \
      --gazetteer ../entity_gazetteer.json
  python3 build_geonames_gazetteer.py            # downloads cities500.zip
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import sys
import urllib.request
import zipfile
from pathlib import Path

try:
    from wordfreq import zipf_frequency
except ImportError:  # optional: preserve the existing ambiguity table
    zipf_frequency = None

HERE = Path(__file__).resolve().parent
DEFAULT_GAZETTEER = HERE.parent / "entity_gazetteer.json"
DUMP_URL = "https://download.geonames.org/export/dump/cities500.zip"
MIN_ALIAS_LENGTH = 4
# Han/kana place names are short: "北京" is a complete name, not a fragment.
CJK_MIN_ALIAS_LENGTH = 2
MAX_ALIASES = 12
# The 12-alias cap used to be filled by Latin spellings alone, so Han/kana/
# Hangul/Arabic/Devanagari/Thai names never reached the index. Each non-cased
# script gets its own slots on top of the unchanged cased set.
NON_CASED_SCRIPT_ALIASES = 4
AMBIGUITY_LANGUAGE = "it"
AMBIGUITY_MAX_POPULATION = 100_000


def is_supported_script(value: str) -> bool:
    """Every script the pipeline can write: Latin (incl. accents), Greek,
    Cyrillic, Han, kana, Hangul, Arabic, Devanagari and Thai, plus name
    punctuation."""
    for char in value:
        if "\u0041" <= char <= "\u024f":
            continue
        if "\u0370" <= char <= "\u03ff" or "\u0400" <= char <= "\u04ff":
            continue
        if "\u3040" <= char <= "\u30ff":
            continue
        if "\u3400" <= char <= "\u4dbf" or "\u4e00" <= char <= "\u9fff":
            continue
        if "\uac00" <= char <= "\ud7af" or "\u1100" <= char <= "\u11ff":
            continue
        if "\u0600" <= char <= "\u06ff" or "\u0750" <= char <= "\u077f":
            continue
        if "\u0900" <= char <= "\u097f" or "\u0e00" <= char <= "\u0e7f":
            continue
        if char in " -'.":
            continue
        return False
    return True


def script_group(value: str) -> str:
    """The script bucket of a name. Latin, Greek and Cyrillic share the
    historical cased pool; every other supported script gets its own.

    The whole string is scanned: a mixed spelling such as "aksrayے" starts
    with a Latin letter but belongs to the Urdu/Arabic bucket, and letting it
    into the cased pool would displace a real ASCII alias."""
    for char in value:
        if "\u3040" <= char <= "\u30ff" or "\u3400" <= char <= "\u4dbf" or "\u4e00" <= char <= "\u9fff":
            return "han"
        if "\uac00" <= char <= "\ud7af" or "\u1100" <= char <= "\u11ff":
            return "hangul"
        if "\u0600" <= char <= "\u06ff" or "\u0750" <= char <= "\u077f":
            return "arabic"
        if "\u0900" <= char <= "\u097f":
            return "devanagari"
        if "\u0e00" <= char <= "\u0e7f":
            return "thai"
    return "cased"


def is_supported_alias(value: str) -> bool:
    """A place alias long enough to be a name in its own script."""
    minimum = (
        CJK_MIN_ALIAS_LENGTH
        if any(
            "\u3040" <= char <= "\u30ff"
            or "\u3400" <= char <= "\u4dbf"
            or "\u4e00" <= char <= "\u9fff"
            or "\uac00" <= char <= "\ud7af"
            for char in value
        )
        else MIN_ALIAS_LENGTH
    )
    return len(value) >= minimum and is_supported_script(value)


def read_rows(input_path: Path | None) -> list[list[str]]:
    if input_path is None:
        with urllib.request.urlopen(DUMP_URL, timeout=120) as response:
            archive = zipfile.ZipFile(io.BytesIO(response.read()))
            with archive.open("cities500.txt") as handle:
                return list(csv.reader(io.TextIOWrapper(handle, encoding="utf-8"), delimiter="\t"))
    with input_path.open(encoding="utf-8") as handle:
        return list(csv.reader(handle, delimiter="\t"))


def place_rows(rows: list[list[str]], min_population: int) -> list[dict]:
    places = []
    for row in rows:
        if len(row) < 15 or row[6] != "P":
            continue
        population = int(row[14] or 0)
        # Capitals stay in even below the threshold: they are the places a
        # news script is most likely to name.
        if population < min_population and row[7] != "PPLC":
            continue
        cased: list[str] = []
        other: list[str] = []
        per_script: dict[str, int] = {}
        for value in [row[1], row[2]] + row[3].split(","):
            alias = value.strip().lower()
            if not alias or alias in cased or alias in other:
                continue
            if not is_supported_alias(alias):
                continue
            group = script_group(alias)
            if group == "cased":
                # Identical to the historical index: the first 12 cased names.
                if len(cased) < MAX_ALIASES:
                    cased.append(alias)
            elif per_script.get(group, 0) < NON_CASED_SCRIPT_ALIASES:
                per_script[group] = per_script.get(group, 0) + 1
                other.append(alias)
        aliases = cased + other
        places.append(
            {
                "name": row[1],
                "country": row[8],
                "population": population,
                "aliases": aliases,
            }
        )
    places.sort(key=lambda place: (place["country"], place["name"].lower(), place["name"]))
    return places


def build_ambiguities(places: list[dict], curated: list[str], threshold: float) -> list[str]:
    """Common Italian words that are also place aliases ("corte", "casa").
    Only small places are considered: a large city keeps its proper-name
    reading even when the lowercase word is frequent (Milano is curated)."""
    max_population: dict[str, int] = {}
    for place in places:
        for alias in [place["name"].lower()] + place["aliases"]:
            max_population[alias] = max(max_population.get(alias, 0), place["population"])
    curated_set = set(curated)
    return sorted(
        alias
        for alias, population in max_population.items()
        if alias not in curated_set
        and population < AMBIGUITY_MAX_POPULATION
        and zipf_frequency(alias, AMBIGUITY_LANGUAGE) >= threshold
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, help="cities500.txt (downloaded when omitted)")
    parser.add_argument("--gazetteer", type=Path, default=DEFAULT_GAZETTEER)
    parser.add_argument("--min-population", type=int, default=5000)
    parser.add_argument("--ambiguity-zipf", type=float, default=5.0)
    args = parser.parse_args()

    rows = read_rows(args.input)
    places = place_rows(rows, args.min_population)
    gazetteer = json.loads(args.gazetteer.read_text(encoding="utf-8"))
    gazetteer["places"] = places
    if zipf_frequency is None:
        print("wordfreq not installed: keeping the existing place_ambiguities table")
    else:
        gazetteer["place_ambiguities"] = {
            AMBIGUITY_LANGUAGE: build_ambiguities(
                places, gazetteer.get("locations", []), args.ambiguity_zipf
            )
        }
    gazetteer["_geonames"] = {
        "source": DUMP_URL,
        "license": "CC BY 4.0",
        "filter": "feature class P, population >= %d, plus PPLC" % args.min_population,
    }
    args.gazetteer.write_text(
        json.dumps(gazetteer, ensure_ascii=False, separators=(",", ":")),
        encoding="utf-8",
    )
    aliases = sum(len(place["aliases"]) for place in places)
    print(
        "wrote %s: %d places, %d aliases, %d bytes"
        % (args.gazetteer, len(places), aliases, args.gazetteer.stat().st_size)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

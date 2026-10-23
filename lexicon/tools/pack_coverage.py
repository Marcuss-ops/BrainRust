#!/usr/bin/env python3
"""Report how far the generated language packs have replaced the legacy table.

The extractor's `is_stop_word` consults, in order:
  1. the language pack (`closed_class`),
  2. the legacy multi-language table `lexicon/stopwords.json`.

Step 2 is the migration bridge, not a per-language copy. This tool measures
that bridge so it can shrink monotonically:

  * coverage      — legacy words that at least one language pack already covers;
  * uncovered     — legacy words no pack covers yet (the remaining debt);
  * per-language  — closed-class size and which legacy words it covers.

Usage:
  python3 lexicon/tools/pack_coverage.py
  python3 lexicon/tools/pack_coverage.py --json
  python3 lexicon/tools/pack_coverage.py --check

`--check` exits 1 when a language directory has no pack or when the committed
packs are stale (same verification the CI runs through
`build_language_packs.py --check`).
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LANGUAGES = ROOT / "lexicon" / "languages"
PACKS = ROOT / "lexicon" / "packs"
LEGACY = ROOT / "lexicon" / "stopwords.json"


def legacy_words() -> set[str]:
    table = json.loads(LEGACY.read_text(encoding="utf-8"))
    return set(table.get("stop_words", [])) | set(table.get("non_ascii_stop_words", []))


def load_packs() -> dict[str, dict]:
    packs: dict[str, dict] = {}
    for path in sorted(PACKS.glob("*.json")):
        if path.name == "index.json":
            continue
        pack = json.loads(path.read_text(encoding="utf-8"))
        packs[pack["language"]] = pack
    return packs


def coverage(packs: dict[str, dict], legacy: set[str]) -> dict:
    per_language = {}
    covered: set[str] = set()
    for language, pack in packs.items():
        closed = set(pack.get("closed_class", []))
        # A pack generated before `closed_class` existed still filters with
        # the union of its older inventories; mirror that here.
        if not closed:
            closed = set(pack.get("stopwords", [])) | set(pack.get("function_words", []))
        hit = closed & legacy
        covered |= hit
        per_language[language] = {
            "closed_class": len(closed),
            "covers_legacy": len(hit),
        }
    return {
        "languages": sorted(packs),
        "language_directories": sorted(
            path.name for path in LANGUAGES.iterdir() if path.is_dir() and path.name != "fallback"
        ),
        "legacy_words": len(legacy),
        "covered": len(covered),
        "uncovered": sorted(legacy - covered),
        "per_language": per_language,
    }


def stale() -> list[str]:
    if str(ROOT / "lexicon" / "tools") not in sys.path:
        sys.path.insert(0, str(ROOT / "lexicon" / "tools"))
    import build_language_packs  # noqa: E402  (same directory, run as a script)

    _languages, files = build_language_packs.build()
    return [
        str(path.relative_to(ROOT))
        for path, content in files.items()
        if not path.is_file() or path.read_text(encoding="utf-8") != content
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail on a missing pack or a stale generated artifact",
    )
    args = parser.parse_args()

    report = coverage(load_packs(), legacy_words())
    missing = sorted(set(report["language_directories"]) - set(report["languages"]))
    report["missing_packs"] = missing

    if args.check:
        problems = []
        problems += [f"missing pack: {language}" for language in missing]
        problems += [f"stale pack: {path}" for path in stale()]
        if problems:
            for problem in problems:
                print(problem, file=sys.stderr)
            return 1

    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0

    print(
        "legacy table: %d words, %d covered by a pack, %d still uncovered"
        % (report["legacy_words"], report["covered"], report["legacy_words"] - report["covered"])
    )
    for language in report["languages"]:
        entry = report["per_language"][language]
        print(
            "  %-3s closed_class=%-4d covers_legacy=%d"
            % (language, entry["closed_class"], entry["covers_legacy"])
        )
    if report["uncovered"]:
        print("uncovered (the bridge that still has to shrink):")
        print("  " + " ".join(report["uncovered"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())

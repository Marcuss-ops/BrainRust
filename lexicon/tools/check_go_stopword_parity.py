#!/usr/bin/env python3
"""Fail-closed parity gate: Go lexicon stopwords vs Rust language packs.

The Go consumer loads `refactored/config/lexicons/<lang>/stopwords.txt`; the
Rust extractor embeds `BrainRust/lexicon/packs/<lang>.json`. For every language
both sides ship, the two sets must be identical: a word added on one side only
is a silent editorial drift. The Go-only `fallback` profile is not compared
because Rust does not ship a fallback language pack; it is the cross-linguistic
set for languages neither side enumerates.

Usage (from the workspace root):
    python3 BrainRust/lexicon/tools/check_go_stopword_parity.py
    python3 BrainRust/lexicon/tools/check_go_stopword_parity.py --repo-root .
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

SHARED_LANGUAGES = ("de", "en", "es", "fr", "it", "pt")


def load_go_stopwords(path: pathlib.Path) -> set[str]:
    words: set[str] = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        word = line.strip().lower()
        if word and not word.startswith("#"):
            words.add(word)
    return words


def load_rust_stopwords(path: pathlib.Path) -> set[str]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    stopwords = payload.get("stopwords", [])
    if not isinstance(stopwords, list):
        raise ValueError(f"{path}: stopwords must be a list")
    return {str(word).strip().lower() for word in stopwords if str(word).strip()}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root",
        type=pathlib.Path,
        default=pathlib.Path(__file__).resolve().parents[3],
        help="workspace root containing BrainRust/ and refactored/",
    )
    args = parser.parse_args()
    root = args.repo_root.resolve()

    failures: list[str] = []
    for language in SHARED_LANGUAGES:
        go_path = root / "refactored" / "config" / "lexicons" / language / "stopwords.txt"
        rust_path = root / "BrainRust" / "lexicon" / "packs" / f"{language}.json"
        if not go_path.is_file():
            failures.append(f"{language}: missing Go stopword file {go_path}")
            continue
        if not rust_path.is_file():
            failures.append(f"{language}: missing Rust pack {rust_path}")
            continue
        go_words = load_go_stopwords(go_path)
        rust_words = load_rust_stopwords(rust_path)
        missing_in_rust = sorted(go_words - rust_words)
        extra_in_rust = sorted(rust_words - go_words)
        print(
            f"{language}: go={len(go_words)} rust={len(rust_words)} "
            f"missing_in_rust={len(missing_in_rust)} extra_in_rust={len(extra_in_rust)}"
        )
        if missing_in_rust:
            failures.append(
                f"{language}: Go-only stopwords missing from Rust pack: {missing_in_rust}"
            )
        if extra_in_rust:
            failures.append(
                f"{language}: Rust-only stopwords missing from Go config: {extra_in_rust}"
            )

    fallback_path = root / "refactored" / "config" / "lexicons" / "fallback" / "stopwords.txt"
    if fallback_path.is_file():
        fallback_words = load_go_stopwords(fallback_path)
        print(
            f"fallback: go={len(fallback_words)} (Go-only by design; "
            "Rust ships no fallback language pack)"
        )

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        return 1
    print(f"PASS: 0 drift across {len(SHARED_LANGUAGES)} shared languages")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

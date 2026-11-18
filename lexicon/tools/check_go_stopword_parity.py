#!/usr/bin/env python3
"""Fail-closed parity gate: Go embedded lexicon vs Rust language packs.

The Go consumer no longer reads `refactored/config/lexicons/<lang>/stopwords.txt`
(that directory was removed by the 2026-11-17 linguistics cutover). It embeds
`refactored/internal/capabilities/linguistics/profiles.generated.json`, which
`lexicon/tools/build_language_packs.py` emits from the Rust language sources.
The Rust extractor embeds `BrainRust/lexicon/packs/<lang>.json`.

For every language both sides ship, the stopword sets must be identical: a word
added on one side only is silent editorial drift. The Go-only `fallback`
profile is reported but not compared, because Rust ships no fallback pack; it
is the cross-linguistic set for languages neither side enumerates.

This is a cross-repo gate: it needs both checkouts. refactored CI runs it in
the `lexicon-parity` job, which checks out BrainRust at the release SHA from
`bin/BRAINRUST_RELEASE.json`; the local agent loop calls
`make verify-lexicon-parity`.

Usage (from the workspace root):
    python3 BrainRust/lexicon/tools/check_go_stopword_parity.py
    python3 BrainRust/lexicon/tools/check_go_stopword_parity.py --repo-root .
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

GO_PROJECTION_SCHEMA = "go_lexicon_profiles.v1"
FALLBACK_LANGUAGE = "fallback"
PACK_INDEX = "index"


def load_go_projection(path: pathlib.Path) -> dict[str, set[str]]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    schema = payload.get("schema")
    if schema != GO_PROJECTION_SCHEMA:
        raise ValueError(
            f"{path}: unexpected schema {schema!r}, want {GO_PROJECTION_SCHEMA!r}"
        )
    profiles = payload.get("profiles")
    if not isinstance(profiles, list):
        raise ValueError(f"{path}: profiles must be a list")
    out: dict[str, set[str]] = {}
    for profile in profiles:
        language = str(profile.get("language", "")).strip().lower()
        if not language:
            raise ValueError(f"{path}: profile without a language")
        stopwords = profile.get("stopwords", [])
        if not isinstance(stopwords, list):
            raise ValueError(f"{path}: {language}: stopwords must be a list")
        out[language] = {
            str(word).strip().lower() for word in stopwords if str(word).strip()
        }
    return out


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

    go_path = (
        root
        / "refactored"
        / "internal"
        / "capabilities"
        / "linguistics"
        / "profiles.generated.json"
    )
    packs_dir = root / "BrainRust" / "lexicon" / "packs"
    failures: list[str] = []

    if not go_path.is_file():
        print(f"FAIL: missing Go projection {go_path}", file=sys.stderr)
        return 1
    if not packs_dir.is_dir():
        print(
            f"FAIL: missing Rust packs directory {packs_dir} "
            "(initialize the BrainRust submodule)",
            file=sys.stderr,
        )
        return 1

    try:
        go_profiles = load_go_projection(go_path)
    except ValueError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1

    rust_packs = {
        path.stem.lower(): path
        for path in sorted(packs_dir.glob("*.json"))
        if path.stem.lower() != PACK_INDEX
    }

    fallback = go_profiles.pop(FALLBACK_LANGUAGE, None)
    shared = sorted(set(go_profiles) & set(rust_packs))
    go_only = sorted(set(go_profiles) - set(rust_packs))
    rust_only = sorted(set(rust_packs) - set(go_profiles))

    if go_only:
        failures.append(f"Go-only languages with no Rust pack: {go_only}")
    if rust_only:
        failures.append(f"Rust-only languages with no Go profile: {rust_only}")
    if not shared:
        failures.append("no shared language between the Go projection and Rust packs")

    for language in shared:
        go_words = go_profiles[language]
        rust_words = load_rust_stopwords(rust_packs[language])
        missing_in_rust = sorted(go_words - rust_words)
        extra_in_rust = sorted(rust_words - go_words)
        print(
            f"{language}: go={len(go_words)} rust={len(rust_words)} "
            f"missing_in_rust={len(missing_in_rust)} extra_in_rust={len(extra_in_rust)}"
        )
        if missing_in_rust:
            failures.append(
                f"{language}: Go stopwords missing from Rust pack: {missing_in_rust}"
            )
        if extra_in_rust:
            failures.append(
                f"{language}: Rust stopwords missing from Go projection: {extra_in_rust}"
            )

    if fallback:
        print(
            f"fallback: go={len(fallback)} (Go-only by design; "
            "Rust ships no fallback language pack)"
        )

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        return 1
    print(f"PASS: 0 drift across {len(shared)} shared languages "
          f"({', '.join(shared)})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

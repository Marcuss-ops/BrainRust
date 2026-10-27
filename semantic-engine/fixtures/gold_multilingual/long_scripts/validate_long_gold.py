#!/usr/bin/env python3
"""Validate the long-form editorial gold corpus before the release scorer runs.

The scorer (`score_long_gold.py`) reports precision/recall; this validator
answers a different question: *is the corpus structurally able to certify
anything at all?* It enforces

  * the corpus shape the scorer reads (schema, scripts, required category
    keys and value types),
  * the **per-language category scope** derived from the shipped contract
    lexicons -- facility facts (`inaugurated_year`, `located_in`, `capacity`)
    are Italian-only because `lexicon/facility_facts.json` carries Italian
    markers only, so a gold that expects them for another language would
    measure the gold, not the engine,
  * absence of placeholder annotations (`TODO`, `...`, empty strings),
  * the annotation provenance: `annotation_policy.authority` must state who
    wrote the labels. Only `human_annotator` can certify a gate;
    `assistant_draft_pending_human_review` is a usable draft and is reported
    as such.

Usage:
    python3 validate_long_gold.py
    python3 validate_long_gold.py --require-release-size
    python3 validate_long_gold.py --require-certified

Exit code 0 when no error is found (warnings do not fail), 1 otherwise.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

BASE = Path(__file__).resolve().parent
REPO = BASE.parents[4]

CORPUS_LANGUAGES = {"it", "en", "es", "pt", "fr", "de", "nl", "pl", "ru", "id"}
NAMED_TYPES = {"PERSON", "LOCATION", "ORG", "ORGANIZATION", "WORK", "BRAND", "PRODUCT", "FACILITY", "STRUCTURE"}
OBJECT_TYPES = {"OBJECT", "VISUAL_CONCEPT"}
AUTHORITIES = {"human_annotator", "assistant_draft_pending_human_review"}
PLACEHOLDERS = ("TODO", "TBD", "FIXME", "XXX", "PLACEHOLDER", "...")
MIN_WORDS = 250

# Contract-backed scope: facility facts come from a single Italian lexicon.
FACILITY_PREDICATES = {"inaugurated_year", "located_in", "capacity", "estimated_saving_percent"}
FACILITY_PREDICATE_LANGUAGES = {"it"}

REQUIRED_CATEGORIES = (
    ("entities", list),
    ("visual_objects", list),
    ("important_phrases", list),
    ("important_segments", list),
    ("facts", list),
    ("positive_charts", list),
    ("negative_chart_evidence", list),
    ("sequence", list),
    ("contextual_image_queries", list),
    ("micro_summary_expectations", list),
)

CATEGORY_KEYS = tuple(name for name, _ in REQUIRED_CATEGORIES)


def object_vocabulary() -> dict[str, set[str]]:
    path = REPO / "lexicon" / "object_vocabulary.json"
    if not path.exists():
        return {}
    terms = json.loads(path.read_text(encoding="utf-8")).get("terms", {})
    return {lang: {" ".join(value.casefold().split()) for value in values} for lang, values in terms.items()}


def load_corpus(path: Path) -> tuple[dict, list[str]]:
    errors: list[str] = []
    try:
        return json.loads(path.read_text(encoding="utf-8")), errors
    except (OSError, json.JSONDecodeError) as error:
        return {}, [f"{path.name}: unreadable ({error})"]


def check_scalar_annotations(script_id: str, spec: dict, errors: list[str], warnings: list[str]) -> None:
    for field in ("important_phrases", "important_segments", "negative_chart_evidence", "sequence"):
        for value in spec.get(field, []):
            if not isinstance(value, str) or not value.strip():
                errors.append(f"{script_id}: {field} contains an empty entry")
                continue
            if any(marker in value for marker in PLACEHOLDERS):
                errors.append(f"{script_id}: {field} contains a placeholder: {value[:60]!r}")

    for field in ("entities", "visual_objects"):
        for entry in spec.get(field, []):
            if not isinstance(entry, dict) or "text" not in entry or "type" not in entry:
                errors.append(f"{script_id}: {field} entry must be an object with text/type")
                continue
            if any(marker in entry["text"] for marker in PLACEHOLDERS):
                errors.append(f"{script_id}: {field} placeholder text {entry['text']!r}")
            allowed = NAMED_TYPES if field == "entities" else OBJECT_TYPES
            if entry["type"] not in allowed:
                errors.append(f"{script_id}: {field} type {entry['type']!r} not in {sorted(allowed)}")

    for fact in spec.get("facts", []):
        if not isinstance(fact, dict) or "subject" not in fact or "predicate" not in fact or "value" not in fact:
            errors.append(f"{script_id}: fact must carry subject/predicate/value")
            continue
        if any(marker in str(fact["subject"]) for marker in PLACEHOLDERS):
            errors.append(f"{script_id}: fact subject placeholder {fact['subject']!r}")
        language = spec.get("language")
        if fact["predicate"] in FACILITY_PREDICATES and language not in FACILITY_PREDICATE_LANGUAGES:
            errors.append(
                f"{script_id}: fact predicate {fact['predicate']!r} is only contract-backed for "
                f"{sorted(FACILITY_PREDICATE_LANGUAGES)} (lexicon/facility_facts.json), not {language!r}"
            )

    for chart in spec.get("positive_charts", []):
        values = chart.get("values") if isinstance(chart, dict) else None
        if not isinstance(values, list) or len(values) < 2 or not all(isinstance(v, (int, float)) for v in values):
            errors.append(f"{script_id}: positive_charts entry needs >=2 numeric values")
        if isinstance(chart, dict) and not isinstance(chart.get("labels"), list):
            errors.append(f"{script_id}: positive_charts entry needs a labels array")

    for query in spec.get("contextual_image_queries", []):
        if not isinstance(query, dict) or not query.get("query") or not query.get("evidence"):
            errors.append(f"{script_id}: contextual_image_queries entry needs query and evidence")
            continue
        if query["evidence"] not in spec.get("_source", ""):
            errors.append(f"{script_id}: contextual query evidence is not a verbatim source span")

    if not spec.get("entities"):
        warnings.append(f"{script_id}: no entities annotated (category not scored)")
    if not spec.get("visual_objects"):
        warnings.append(f"{script_id}: no visual_objects annotated (category not scored)")


def validate(gold: dict, errors: list[str], warnings: list[str]) -> dict:
    if gold.get("schema") != "brainrust.editorial_gold.v1":
        errors.append(f"schema must be brainrust.editorial_gold.v1, found {gold.get('schema')!r}")

    policy = gold.get("annotation_policy")
    if not isinstance(policy, dict):
        errors.append("annotation_policy block is missing")
        policy = {}
    authority = policy.get("authority")
    if authority not in AUTHORITIES:
        errors.append(f"annotation_policy.authority must be one of {sorted(AUTHORITIES)}, found {authority!r}")

    scripts = gold.get("scripts")
    if not isinstance(scripts, dict) or not scripts:
        errors.append("scripts must be a non-empty object")
        return {}

    vocab = object_vocabulary()
    stats = {"scripts": 0, "words": 0, "categories": {key: 0 for key in CATEGORY_KEYS}, "languages": set()}
    for script_id, spec in sorted(scripts.items()):
        if not isinstance(spec, dict):
            errors.append(f"{script_id}: script spec must be an object")
            continue
        language = spec.get("language")
        if language not in CORPUS_LANGUAGES:
            errors.append(f"{script_id}: language {language!r} outside {sorted(CORPUS_LANGUAGES)}")
            continue
        stats["languages"].add(language)

        relative = spec.get("file")
        if not isinstance(relative, str) or not relative:
            errors.append(f"{script_id}: file is missing")
            continue
        source_path = BASE / relative
        if not source_path.exists():
            errors.append(f"{script_id}: source file not found: {relative}")
            continue
        source = source_path.read_text(encoding="utf-8")
        words = len(source.split())
        if words < MIN_WORDS:
            errors.append(f"{script_id}: source has {words} words, minimum for long-form gold is {MIN_WORDS}")
        stats["scripts"] += 1
        stats["words"] += words

        for key, expected_type in REQUIRED_CATEGORIES:
            if not isinstance(spec.get(key), expected_type):
                errors.append(f"{script_id}: category {key!r} must be a {expected_type.__name__}")
            elif spec[key]:
                stats["categories"][key] += 1

        check_scalar_annotations(script_id, {**spec, "_source": source}, errors, warnings)

        for entry in spec.get("visual_objects", []):
            if not isinstance(entry, dict) or entry.get("type") != "OBJECT":
                continue
            term = " ".join(str(entry.get("text", "")).casefold().split())
            if vocab.get(language) and term not in vocab[language]:
                warnings.append(
                    f"{script_id}: OBJECT {term!r} is outside the {language} object vocabulary "
                    "(counts as a deliberate off-vocabulary recall probe)"
                )

    stats["languages"] = sorted(stats["languages"])
    stats["language_count"] = len(stats["languages"])
    stats["release_sized"] = stats["scripts"] >= 20 and stats["language_count"] == 10
    stats["authority"] = authority
    if authority != "human_annotator":
        stats["certification"] = "blocked_pending_human_review"
    elif not stats["release_sized"]:
        stats["certification"] = "blocked_pending_release_size"
    else:
        stats["certification"] = "certified"
    return stats


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--gold", type=Path, default=BASE / "gold.json")
    parser.add_argument("--require-release-size", action="store_true",
                        help="fail unless the corpus has >=20 scripts over all ten languages")
    parser.add_argument("--require-certified", action="store_true",
                        help="fail unless the annotations are human-authored AND release-sized")
    args = parser.parse_args()

    gold, errors = load_corpus(args.gold)
    warnings: list[str] = []
    stats = validate(gold, errors, warnings) if gold else {}

    for warning in warnings:
        print(f"WARN  {warning}")
    for error in errors:
        print(f"ERROR {error}")

    if stats:
        print(json.dumps({k: v for k, v in stats.items() if k != "languages"}, ensure_ascii=False))
        print(f"languages: {', '.join(stats['languages'])}")

    if args.require_release_size and stats and not stats["release_sized"]:
        errors.append("corpus is not release-sized (>=20 scripts over all ten languages required)")
        print("ERROR corpus is not release-sized")
    if args.require_certified:
        certified = bool(stats) and stats["certification"] == "certified"
        if not certified:
            errors.append("corpus is not certified: human-authored annotations over a release-sized corpus required")
            print("ERROR corpus is not certified")
        return 0 if certified else 1
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())

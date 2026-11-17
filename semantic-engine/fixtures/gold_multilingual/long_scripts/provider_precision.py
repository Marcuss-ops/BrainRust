#!/usr/bin/env python3
"""Score a frozen provider run against independent top-10 judgments.

This tool does not call a provider and does not create relevance labels. It
consumes two artifacts produced by the provider owner and an independent
judge, validates them, and reports precision@10 separately for literal and
derived contextual queries.

It deliberately fails closed:

* the 20 long-gold derived queries must match `gold.json` exactly;
* every query must have exactly ten ranked, unique HTTP results;
* every result must carry provider, provenance and downloaded bytes;
* every result must have exactly one judgment from an independent judge;
* the results file must not contain relevance labels.

Without a real provider run and a separate judge, the report status is blocked
and the exit code is 2. A synthetic fixture can exercise this scorer, but it
cannot satisfy TICKET-26.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

BASE = Path(__file__).resolve().parent
RESULTS_SCHEMA = "brainrust.provider_precision.results.v1"
JUDGMENTS_SCHEMA = "brainrust.provider_precision.judgments.v1"
REPORT_SCHEMA = "brainrust.provider_precision.report.v1"
TOP_K = 10
QUERY_CLASSES = {"literal", "derived"}
RELEVANCE_KEYS = {"relevant", "relevance", "judged", "judgment", "label"}
SHA_RE = re.compile(r"^[0-9a-f]{40}$")


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"cannot read {path}: {exc}") from exc


def parse_timestamp(value: Any, where: str, errors: list[str]) -> None:
    if not isinstance(value, str) or not value.strip():
        errors.append(f"{where} must be a non-empty ISO-8601 timestamp")
        return
    try:
        datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        errors.append(f"{where} is not an ISO-8601 timestamp: {value!r}")


def require_string(obj: dict[str, Any], key: str, where: str, errors: list[str]) -> str:
    value = obj.get(key)
    if not isinstance(value, str) or not value.strip():
        errors.append(f"{where}.{key} must be a non-empty string")
        return ""
    return value


def derived_queries(gold: Any, errors: list[str]) -> dict[str, tuple[str, str]]:
    """Return script_id -> (query, evidence) from the long-gold annotations."""
    if not isinstance(gold, dict):
        errors.append("gold must be an object")
        return {}
    scripts = gold.get("scripts")
    if not isinstance(scripts, dict) or not scripts:
        errors.append("gold.scripts must be a non-empty object")
        return {}
    expected: dict[str, tuple[str, str]] = {}
    for script_id, spec in scripts.items():
        if not isinstance(spec, dict):
            errors.append(f"gold.scripts[{script_id}] must be an object")
            continue
        queries = spec.get("contextual_image_queries")
        if not isinstance(queries, list) or len(queries) != 1:
            errors.append(
                f"gold.scripts[{script_id}] must contain exactly one contextual_image_queries entry"
            )
            continue
        query = queries[0]
        if not isinstance(query, dict):
            errors.append(f"gold.scripts[{script_id}].contextual_image_queries[0] must be an object")
            continue
        text = query.get("query")
        evidence = query.get("evidence")
        if not isinstance(text, str) or not text.strip():
            errors.append(f"gold.scripts[{script_id}] derived query is empty")
            continue
        if not isinstance(evidence, str) or not evidence.strip():
            errors.append(f"gold.scripts[{script_id}] derived query evidence is empty")
            continue
        expected[script_id] = (text, evidence)
    return expected


def validate_results(
    payload: Any,
    gold: Any,
    errors: list[str],
) -> tuple[dict[str, Any], list[dict[str, Any]], dict[str, tuple[str, str]]]:
    expected = derived_queries(gold, errors)
    if not isinstance(payload, dict):
        errors.append("results must be an object")
        return {}, [], expected
    if payload.get("schema") != RESULTS_SCHEMA:
        errors.append(f"results.schema must be {RESULTS_SCHEMA!r}")

    run = payload.get("run")
    if not isinstance(run, dict):
        errors.append("results.run must be an object")
        run = {}
    require_string(run, "provider", "results.run", errors)
    require_string(run, "cascade", "results.run", errors)
    parse_timestamp(run.get("started_at"), "results.run.started_at", errors)
    git_sha = run.get("git_sha")
    if not isinstance(git_sha, str) or not SHA_RE.fullmatch(git_sha):
        errors.append("results.run.git_sha must be a 40-character lowercase git SHA")

    queries = payload.get("queries")
    if not isinstance(queries, list) or not queries:
        errors.append("results.queries must be a non-empty array")
        return run, [], expected

    seen_ids: set[str] = set()
    derived_by_script: dict[str, str] = {}
    normalized: list[dict[str, Any]] = []
    for index, query in enumerate(queries):
        where = f"results.queries[{index}]"
        if not isinstance(query, dict):
            errors.append(f"{where} must be an object")
            continue
        if any(key in query for key in RELEVANCE_KEYS):
            errors.append(f"{where} must not contain relevance labels; judgments belong in the judge file")
        if any(key in query for key in ("error", "failure", "provider_error")):
            errors.append(f"{where} records a provider failure; the run is fail-closed")
        query_id = require_string(query, "id", where, errors)
        if query_id in seen_ids:
            errors.append(f"{where}.id is duplicated: {query_id!r}")
        seen_ids.add(query_id)
        script_id = require_string(query, "script_id", where, errors)
        if script_id and script_id not in (gold.get("scripts") or {}):
            errors.append(f"{where}.script_id is not in gold: {script_id!r}")
        query_class = query.get("query_class")
        if query_class not in QUERY_CLASSES:
            errors.append(f"{where}.query_class must be one of {sorted(QUERY_CLASSES)}")
        text = require_string(query, "query", where, errors)
        evidence = query.get("evidence")
        if query_class == "derived":
            if script_id in derived_by_script:
                errors.append(f"{where} is a second derived query for {script_id!r}")
            derived_by_script[script_id] = query_id
            expected_pair = expected.get(script_id)
            if expected_pair is None:
                errors.append(f"{where} derived query has no gold expectation for {script_id!r}")
            else:
                expected_query, expected_evidence = expected_pair
                if text != expected_query:
                    errors.append(
                        f"{where}.query does not match gold for {script_id!r}: "
                        f"{text!r} != {expected_query!r}"
                    )
                if evidence != expected_evidence:
                    errors.append(
                        f"{where}.evidence does not match gold for {script_id!r}"
                    )
        elif evidence is not None and not isinstance(evidence, str):
            errors.append(f"{where}.evidence must be a string when present")

        results = query.get("results")
        if not isinstance(results, list) or len(results) != TOP_K:
            errors.append(f"{where}.results must contain exactly {TOP_K} entries")
            results = []
        ranks: set[int] = set()
        urls: set[str] = set()
        normalized_results: list[dict[str, Any]] = []
        for rank_index, result in enumerate(results):
            result_where = f"{where}.results[{rank_index}]"
            if not isinstance(result, dict):
                errors.append(f"{result_where} must be an object")
                continue
            if any(key in result for key in RELEVANCE_KEYS):
                errors.append(
                    f"{result_where} must not contain relevance labels; "
                    "judgments belong in the judge file"
                )
            rank = result.get("rank")
            if not isinstance(rank, int) or isinstance(rank, bool) or not 1 <= rank <= TOP_K:
                errors.append(f"{result_where}.rank must be an integer in 1..{TOP_K}")
            elif rank in ranks:
                errors.append(f"{result_where}.rank is duplicated: {rank}")
            else:
                ranks.add(rank)
            url = require_string(result, "url", result_where, errors)
            if url and not url.startswith(("http://", "https://")):
                errors.append(f"{result_where}.url must be HTTP(S)")
            if url in urls:
                errors.append(f"{result_where}.url is duplicated: {url!r}")
            urls.add(url)
            require_string(result, "provider", result_where, errors)
            require_string(result, "provenance", result_where, errors)
            downloaded = result.get("downloaded_bytes")
            if (
                not isinstance(downloaded, int)
                or isinstance(downloaded, bool)
                or downloaded <= 0
            ):
                errors.append(f"{result_where}.downloaded_bytes must be a positive integer")
            normalized_results.append(
                {
                    "rank": rank,
                    "url": url,
                    "provider": result.get("provider"),
                    "provenance": result.get("provenance"),
                    "downloaded_bytes": downloaded,
                }
            )
        if ranks != set(range(1, TOP_K + 1)):
            errors.append(f"{where}.results ranks must cover 1..{TOP_K} exactly")
        normalized.append(
            {
                "id": query_id,
                "script_id": script_id,
                "query_class": query_class,
                "query": text,
                "evidence": evidence,
                "results": normalized_results,
            }
        )

    missing = sorted(set(expected) - set(derived_by_script))
    if missing:
        errors.append(f"results are missing derived queries for: {', '.join(missing)}")
    extra = sorted(set(derived_by_script) - set(expected))
    if extra:
        errors.append(f"results contain derived queries outside gold: {', '.join(extra)}")

    total = run.get("downloaded_bytes_total")
    actual_total = sum(
        result["downloaded_bytes"]
        for query in normalized
        for result in query["results"]
        if isinstance(result.get("downloaded_bytes"), int)
    )
    if total is not None and total != actual_total:
        errors.append(
            "results.run.downloaded_bytes_total does not match the sum of result bytes "
            f"({total} != {actual_total})"
        )
    return run, normalized, expected


def validate_judgments(
    payload: Any,
    queries: list[dict[str, Any]],
    errors: list[str],
) -> dict[tuple[str, str], bool]:
    if not isinstance(payload, dict):
        errors.append("judgments must be an object")
        return {}
    if payload.get("schema") != JUDGMENTS_SCHEMA:
        errors.append(f"judgments.schema must be {JUDGMENTS_SCHEMA!r}")
    judge = payload.get("judge")
    if not isinstance(judge, dict):
        errors.append("judgments.judge must be an object")
        judge = {}
    require_string(judge, "id", "judgments.judge", errors)
    if judge.get("independent") is not True:
        errors.append("judgments.judge.independent must be true")
    parse_timestamp(judge.get("judged_at"), "judgments.judge.judged_at", errors)

    pairs = {
        (query["id"], result["url"])
        for query in queries
        for result in query["results"]
    }
    labels: dict[tuple[str, str], bool] = {}
    entries = payload.get("labels")
    if not isinstance(entries, list):
        errors.append("judgments.labels must be an array")
        return labels
    for index, entry in enumerate(entries):
        where = f"judgments.labels[{index}]"
        if not isinstance(entry, dict):
            errors.append(f"{where} must be an object")
            continue
        query_id = require_string(entry, "query_id", where, errors)
        url = require_string(entry, "url", where, errors)
        relevant = entry.get("relevant")
        if not isinstance(relevant, bool):
            errors.append(f"{where}.relevant must be a boolean")
            continue
        key = (query_id, url)
        if key not in pairs:
            errors.append(f"{where} references an unknown query/result pair: {key}")
            continue
        if key in labels:
            errors.append(f"{where} duplicates a judgment for {key}")
            continue
        labels[key] = relevant
    missing = sorted(pairs - set(labels))
    if missing:
        errors.append(f"judgments are missing {len(missing)} of {len(pairs)} result pairs")
    extra = sorted(set(labels) - pairs)
    if extra:
        errors.append(f"judgments contain {len(extra)} unknown result pairs")
    return labels


def aggregate(rows: list[dict[str, Any]]) -> dict[str, Any]:
    results = sum(row["results"] for row in rows)
    relevant = sum(row["relevant"] for row in rows)
    return {
        "queries": len(rows),
        "results": results,
        "relevant": relevant,
        "precision_at_10": (relevant / results) if results else None,
        "macro_precision_at_10": (
            sum(row["precision_at_10"] for row in rows) / len(rows) if rows else None
        ),
    }


def build_report(gold: Any, results: Any, judgments: Any) -> dict[str, Any]:
    errors: list[str] = []
    warnings: list[str] = []
    run, queries, expected = validate_results(results, gold, errors)
    labels = validate_judgments(judgments, queries, errors) if queries else {}

    rows: list[dict[str, Any]] = []
    for query in queries:
        query_labels = [
            labels.get((query["id"], result["url"]))
            for result in query["results"]
        ]
        if any(label is None for label in query_labels):
            continue
        relevant = sum(1 for label in query_labels if label)
        rows.append(
            {
                "id": query["id"],
                "script_id": query["script_id"],
                "query_class": query["query_class"],
                "query": query["query"],
                "results": len(query_labels),
                "relevant": relevant,
                "precision_at_10": relevant / len(query_labels),
            }
        )

    literal = [row for row in rows if row["query_class"] == "literal"]
    derived = [row for row in rows if row["query_class"] == "derived"]
    if not literal:
        warnings.append(
            "no literal queries were scored; criterion 2 cannot be certified by this run"
        )

    if errors:
        status = "blocked_invalid_run"
    elif len(derived) != len(expected):
        status = "blocked_incomplete_derived"
    elif not literal:
        status = "blocked_literal_queries_missing"
    else:
        status = "measured"

    return {
        "schema": REPORT_SCHEMA,
        "status": status,
        "run": run,
        "gold": {
            "derived_expected": len(expected),
            "derived_scored": len(derived),
        },
        "literal": aggregate(literal),
        "derived": aggregate(derived),
        "queries": rows,
        "warnings": warnings,
        "errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, required=True,
                        help="frozen provider results JSON")
    parser.add_argument("--judgments", type=Path, required=True,
                        help="independent judge labels JSON")
    parser.add_argument("--gold", type=Path, default=BASE / "gold.json",
                        help="long-gold gold.json (default: %(default)s)")
    parser.add_argument("--report", type=Path,
                        help="write the JSON report here")
    args = parser.parse_args()

    try:
        gold = load_json(args.gold)
        results = load_json(args.results)
        judgments = load_json(args.judgments)
    except ValueError as exc:
        print(f"provider-precision: {exc}", file=sys.stderr)
        return 2

    report = build_report(gold, results, judgments)
    if args.report:
        args.report.write_text(
            json.dumps(report, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    print(
        "provider-precision:",
        report["status"],
        f"derived={report['derived']['precision_at_10']}",
        f"literal={report['literal']['precision_at_10']}",
        f"errors={len(report['errors'])}",
    )
    for error in report["errors"]:
        print(f"  error: {error}", file=sys.stderr)
    for warning in report["warnings"]:
        print(f"  warning: {warning}", file=sys.stderr)
    return 0 if report["status"] == "measured" else 2


if __name__ == "__main__":
    raise SystemExit(main())

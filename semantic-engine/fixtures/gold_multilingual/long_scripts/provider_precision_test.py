#!/usr/bin/env python3
"""Unit tests for provider_precision.py.

The fixture is synthetic and exists only to exercise validation and arithmetic.
It is not a provider run and does not certify TICKET-26.
"""

from __future__ import annotations

import copy
import unittest

import provider_precision as pp


def gold_fixture() -> dict:
    return {
        "scripts": {
            "s1": {
                "language": "en",
                "contextual_image_queries": [
                    {"query": "Aurora Torino", "evidence": "Aurora is in Torino."}
                ],
            }
        }
    }


def result_entry(query_id: str, rank: int, suffix: str) -> dict:
    return {
        "rank": rank,
        "url": f"https://example.test/{query_id}/{suffix}",
        "provider": "fixture-provider",
        "provenance": "frozen-result",
        "downloaded_bytes": 100 + rank,
    }


def query_entry(
    query_id: str,
    query_class: str,
    text: str,
    relevant_count: int,
    *,
    script_id: str = "s1",
    evidence: str | None = "Aurora is in Torino.",
) -> tuple[dict, list[dict]]:
    results = [result_entry(query_id, rank, f"r{rank}") for rank in range(1, pp.TOP_K + 1)]
    query = {
        "id": query_id,
        "script_id": script_id,
        "query_class": query_class,
        "query": text,
        "results": results,
    }
    if evidence is not None:
        query["evidence"] = evidence
    labels = [
        {
            "query_id": query_id,
            "url": result["url"],
            "relevant": index < relevant_count,
        }
        for index, result in enumerate(results)
    ]
    return query, labels


def inputs(
    *,
    include_literal: bool = True,
    derived_relevant: int = 7,
    literal_relevant: int = 4,
) -> tuple[dict, dict, dict]:
    derived, derived_labels = query_entry(
        "q-derived", "derived", "Aurora Torino", derived_relevant
    )
    queries = [derived]
    labels = list(derived_labels)
    if include_literal:
        literal, literal_labels = query_entry(
            "q-literal", "literal", "Aurora", literal_relevant, evidence=None
        )
        queries.append(literal)
        labels.extend(literal_labels)
    results = {
        "schema": pp.RESULTS_SCHEMA,
        "run": {
            "provider": "fixture-provider",
            "cascade": "fixture-cascade",
            "started_at": "2026-11-17T00:00:00Z",
            "git_sha": "a" * 40,
            "downloaded_bytes_total": sum(
                result["downloaded_bytes"]
                for query in queries
                for result in query["results"]
            ),
        },
        "queries": queries,
    }
    judgments = {
        "schema": pp.JUDGMENTS_SCHEMA,
        "judge": {
            "id": "judge-1",
            "independent": True,
            "judged_at": "2026-11-17T01:00:00Z",
        },
        "labels": labels,
    }
    return results, judgments, gold_fixture()


class ProviderPrecisionTest(unittest.TestCase):
    def test_separates_literal_and_derived_precision(self) -> None:
        results, judgments, gold = inputs()
        report = pp.build_report(gold, results, judgments)
        self.assertEqual(report["status"], "measured")
        self.assertEqual(report["derived"]["precision_at_10"], 0.7)
        self.assertEqual(report["literal"]["precision_at_10"], 0.4)
        self.assertEqual(report["gold"]["derived_scored"], 1)

    def test_missing_literal_queries_blocks_certification(self) -> None:
        results, judgments, gold = inputs(include_literal=False)
        report = pp.build_report(gold, results, judgments)
        self.assertEqual(report["status"], "blocked_literal_queries_missing")
        self.assertIsNone(report["literal"]["precision_at_10"])
        self.assertTrue(any("literal" in warning for warning in report["warnings"]))

    def test_missing_judgment_blocks(self) -> None:
        results, judgments, gold = inputs()
        judgments["labels"].pop()
        report = pp.build_report(gold, results, judgments)
        self.assertEqual(report["status"], "blocked_invalid_run")
        self.assertTrue(any("missing" in error for error in report["errors"]))

    def test_zero_downloaded_bytes_blocks(self) -> None:
        results, judgments, gold = inputs()
        results["queries"][0]["results"][3]["downloaded_bytes"] = 0
        report = pp.build_report(gold, results, judgments)
        self.assertEqual(report["status"], "blocked_invalid_run")
        self.assertTrue(
            any("downloaded_bytes" in error for error in report["errors"])
        )

    def test_relevance_in_results_blocks(self) -> None:
        results, judgments, gold = inputs()
        results["queries"][0]["results"][0]["relevant"] = True
        report = pp.build_report(gold, results, judgments)
        self.assertEqual(report["status"], "blocked_invalid_run")
        self.assertTrue(
            any("relevance" in error for error in report["errors"])
        )

    def test_derived_query_must_match_gold(self) -> None:
        results, judgments, gold = inputs()
        results["queries"][0]["query"] = "Aurora Milano"
        report = pp.build_report(gold, results, judgments)
        self.assertEqual(report["status"], "blocked_invalid_run")
        self.assertTrue(
            any("does not match gold" in error for error in report["errors"])
        )

    def test_duplicate_url_blocks(self) -> None:
        results, judgments, gold = inputs()
        results["queries"][0]["results"][1]["url"] = results["queries"][0]["results"][0]["url"]
        report = pp.build_report(gold, results, judgments)
        self.assertEqual(report["status"], "blocked_invalid_run")
        self.assertTrue(any("duplicated" in error for error in report["errors"]))

    def test_non_independent_judge_blocks(self) -> None:
        results, judgments, gold = inputs()
        judgments["judge"]["independent"] = False
        report = pp.build_report(gold, results, judgments)
        self.assertEqual(report["status"], "blocked_invalid_run")
        self.assertTrue(
            any("independent" in error for error in report["errors"])
        )

    def test_real_gold_has_the_20_derived_queries(self) -> None:
        gold = pp.load_json(pp.BASE / "gold.json")
        errors: list[str] = []
        expected = pp.derived_queries(gold, errors)
        self.assertEqual(errors, [])
        self.assertEqual(len(expected), 20)


if __name__ == "__main__":
    unittest.main()

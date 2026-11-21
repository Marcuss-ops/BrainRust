#!/usr/bin/env python3
"""Run and score the draft 11-language discourse baseline matrix."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path

FIXTURE = Path(__file__).with_name("coref_gold.json")
LANGUAGES = ("de", "en", "es", "fr", "id", "it", "nl", "pl", "pt", "ru", "tr")


def run_language(worker: Path, language: str, cases: list[dict]) -> tuple[dict, list[dict]]:
    # Each gold case is a separate script. Batching them as scenes in one
    # script leaks antecedents between otherwise independent test cases.
    requests = [
        {
            "operation": "analyze_script",
            "language": language,
            "scenes": [{"scene_id": row["case_id"], "text": row["text"]}],
        }
        for row in cases
    ]
    result = subprocess.run(
        [str(worker)], input="".join(json.dumps(request, ensure_ascii=False) + "\n" for request in requests),
        text=True, capture_output=True, check=False,
    )
    if result.returncode:
        raise RuntimeError(f"worker exited {result.returncode} for {language}: {result.stderr}")
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    if len(lines) != len(cases):
        raise RuntimeError(f"worker returned {len(lines)} lines for {language}; expected {len(cases)}")
    manifests = {}
    for row, line in zip(cases, lines, strict=True):
        response = json.loads(line)
        if not response.get("ok"):
            raise RuntimeError(f"worker rejected {language}/{row['case_id']}: {response}")
        manifests[row["case_id"]] = response["manifest"]

    expected_counts = Counter(row["expected_status"] for row in cases)
    observed_counts: Counter[str] = Counter()
    scenario_results: dict[str, dict] = {}
    errors = []
    status_matches = target_matches = confident_wrong = 0
    resolved_tp = resolved_fp = resolved_fn = 0
    for row in cases:
        scene_id = row["case_id"]
        manifest = manifests[scene_id]
        scene = manifest["scenes"][0]
        pronouns = [
            reference for reference in manifest["discourse"]["references"]
            if reference.get("type") == "pronoun" and reference.get("mention", "").casefold() == row["reference"].casefold()
        ]
        reference = pronouns[0] if pronouns else None
        status = reference.get("status", "missing") if reference else "missing"
        observed_counts[status] += 1
        expected_status = row["expected_status"]
        status_matches += status == expected_status

        entity_ids = {
            entity["text"]: entity["id"]
            for entity in scene["entities"]
            if entity.get("type") == "PERSON"
        }
        expected_ids = [entity_ids.get(name) for name in row["expected_targets"]]
        actual_ids = (
            reference.get("target_entity_ids", []) if status == "resolved"
            else reference.get("candidate_entity_ids", []) if status == "ambiguous"
            else []
        ) if reference else []
        target_ok = None not in expected_ids and set(actual_ids) == set(expected_ids)
        target_matches += target_ok
        wrong = (expected_status != "resolved" and status == "resolved") or (
            expected_status == "resolved" and status == "resolved" and not target_ok
        )
        confident_wrong += wrong
        if status == "resolved":
            if expected_status == "resolved" and target_ok:
                resolved_tp += 1
            else:
                resolved_fp += 1
        elif expected_status == "resolved":
            resolved_fn += 1
        if status != expected_status or not target_ok:
            errors.append({
                "case_id": scene_id,
                "scenario": row["scenario"],
                "expected_status": expected_status,
                "actual_status": status,
                "expected_targets": row["expected_targets"],
                "actual_target_or_candidates": actual_ids,
                "missing_gold_target": None in expected_ids,
            })
        bucket = scenario_results.setdefault(row["scenario"], {"cases": 0, "status_matches": 0, "target_matches": 0})
        bucket["cases"] += 1
        bucket["status_matches"] += status == expected_status
        bucket["target_matches"] += target_ok

    summary = {
        "cases": len(cases),
        "expected_status_counts": dict(sorted(expected_counts.items())),
        "observed_status_counts": dict(sorted(observed_counts.items())),
        "status_accuracy": status_matches / len(cases) if cases else 1.0,
        "target_accuracy": target_matches / len(cases) if cases else 1.0,
        "resolved_true_positive": resolved_tp,
        "resolved_false_positive": resolved_fp,
        "resolved_false_negative": resolved_fn,
        "resolved_precision": resolved_tp / (resolved_tp + resolved_fp) if resolved_tp + resolved_fp else 1.0,
        "resolved_recall": resolved_tp / (resolved_tp + resolved_fn) if resolved_tp + resolved_fn else 1.0,
        "confidently_wrong": confident_wrong,
        "scenario_results": scenario_results,
        "failures": errors,
    }
    return summary, errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, default=Path("target/release/semantic-engine"))
    parser.add_argument("--report", type=Path)
    parser.add_argument("--strict", action="store_true", help="exit 1 if any draft label mismatches")
    args = parser.parse_args()
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    if fixture.get("schema") != "brainrust.coref_gold.v1":
        parser.error("unsupported coreference fixture schema")
    if not args.worker.is_file():
        parser.error(f"worker not found: {args.worker}; build semantic-engine first")
    report = {
        "schema": "brainrust.coref_baseline_report.v1",
        "annotation_authority": fixture["annotation_authority"],
        "worker_sha256": hashlib.sha256(args.worker.read_bytes()).hexdigest(),
        "languages": {},
    }
    for language in LANGUAGES:
        cases = fixture["languages"].get(language, [])
        if len(cases) != 40:
            parser.error(f"{language} has {len(cases)} references; expected 40")
        summary, _ = run_language(args.worker, language, cases)
        report["languages"][language] = summary
    report["totals"] = {
        "cases": sum(item["cases"] for item in report["languages"].values()),
        "status_matches": sum(round(item["status_accuracy"] * item["cases"]) for item in report["languages"].values()),
        "target_matches": sum(round(item["target_accuracy"] * item["cases"]) for item in report["languages"].values()),
        "confidently_wrong": sum(item["confidently_wrong"] for item in report["languages"].values()),
        "resolved_true_positive": sum(item["resolved_true_positive"] for item in report["languages"].values()),
        "resolved_false_positive": sum(item["resolved_false_positive"] for item in report["languages"].values()),
        "resolved_false_negative": sum(item["resolved_false_negative"] for item in report["languages"].values()),
    }
    tp = report["totals"]["resolved_true_positive"]
    fp = report["totals"]["resolved_false_positive"]
    fn = report["totals"]["resolved_false_negative"]
    report["totals"]["resolved_precision"] = tp / (tp + fp) if tp + fp else 1.0
    report["totals"]["resolved_recall"] = tp / (tp + fn) if tp + fn else 1.0
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    failures = sum(len(item["failures"]) for item in report["languages"].values())
    return 1 if args.strict and failures else 0


if __name__ == "__main__":
    sys.exit(main())

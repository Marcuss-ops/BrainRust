#!/usr/bin/env python3
"""Score independently authored editorial-gold scripts against BrainRust.

The scorer reports category precision/recall without rewriting annotations.
It deliberately returns exit 2 while the corpus has fewer than 20 scripts or
does not cover all ten target languages: a passing pilot is not a release gate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import select
import subprocess
import tempfile
import time
from pathlib import Path

BASE = Path(__file__).resolve().parent
DEFAULT_WORKER = BASE.parents[3] / "target" / "release" / "semantic-engine"
NAMED_TYPES = {"PERSON", "LOCATION", "ORG", "ORGANIZATION", "WORK", "BRAND", "PRODUCT", "FACILITY", "STRUCTURE"}
OBJECT_TYPES = {"OBJECT", "VISUAL_CONCEPT"}


def normalize(text: str) -> str:
    return " ".join(re.findall(r"[^\W_]+", text.casefold(), flags=re.UNICODE))


def same_surface(left: str, right: str) -> bool:
    a, b = normalize(left).split(), normalize(right).split()
    if not a or not b:
        return False
    short, long = (a, b) if len(a) <= len(b) else (b, a)
    return any(long[index:index + len(short)] == short for index in range(len(long) - len(short) + 1))


def unique_candidates(items: list[dict]) -> list[dict]:
    seen = set()
    output = []
    for item in items:
        key = (normalize(item.get("text", "")), item.get("type"))
        if key not in seen:
            seen.add(key)
            output.append(item)
    return output


def category_score(expected: list[dict], actual: list[dict], *, typed: bool = True) -> dict:
    matches = []
    used = set()
    for gold in expected:
        hit = next((index for index, item in enumerate(actual)
                    if index not in used
                    and same_surface(gold["text"], item["text"])
                    and (not typed or gold.get("type") == item.get("type"))), None)
        if hit is not None:
            used.add(hit)
        matches.append(hit is not None)
    tp = sum(matches)
    return {
        "expected": len(expected),
        "found": tp,
        "extracted": len(actual),
        "precision": tp / len(actual) if actual else (1.0 if not expected else 0.0),
        "recall": tp / len(expected) if expected else 1.0,
        "f1": 2 * tp / (len(actual) + len(expected)) if actual or expected else 1.0,
        "misses": [expected[i] for i, found in enumerate(matches) if not found],
        "false_positives": [item for index, item in enumerate(actual) if index not in used],
    }


def exact_text_score(expected: list[str], actual: list[str]) -> dict:
    """Strict phrase/segment match: complete linguistic unit or a miss."""
    remaining = list(actual)
    found, misses = 0, []
    for phrase in expected:
        index = next((i for i, candidate in enumerate(remaining)
                      if normalize(phrase) == normalize(candidate)), None)
        if index is None:
            misses.append(phrase)
        else:
            found += 1
            remaining.pop(index)
    total = len(expected)
    return {
        "expected": total,
        "found": found,
        "extracted": len(actual),
        "precision": found / len(actual) if actual else (1.0 if not total else 0.0),
        "recall": found / total if total else 1.0,
        "f1": 2 * found / (len(actual) + total) if actual or total else 1.0,
        "misses": misses,
    }


class Worker:
    def __init__(self, binary: Path):
        self.stderr = tempfile.TemporaryFile()
        self.process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr)
        self.buffer = b""

    def request(self, payload: dict, timeout: float = 180.0) -> dict:
        self.process.stdin.write((json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n").encode())
        self.process.stdin.flush()
        deadline = time.monotonic() + timeout
        while True:
            split = self.buffer.find(b"\n")
            if split >= 0:
                line, self.buffer = self.buffer[:split], self.buffer[split + 1:]
                return json.loads(line)
            remaining = deadline - time.monotonic()
            if remaining <= 0 or self.process.poll() is not None:
                raise RuntimeError("worker exited or request timed out")
            ready, _, _ = select.select([self.process.stdout], [], [], remaining)
            if not ready:
                raise TimeoutError("worker request timed out")
            chunk = self.process.stdout.readline()
            if not chunk:
                raise RuntimeError("worker output closed")
            self.buffer += chunk

    def close(self):
        try:
            self.process.stdin.close()
            self.process.wait(timeout=3)
        except Exception:
            self.process.kill()
            self.process.wait()
        self.stderr.close()


def source_grounding(scene: dict, source: str) -> list[str]:
    raw = source.encode("utf-8")
    problems = []
    for field in ("entities", "bullets", "short_phrases", "long_phrases", "important_words", "quotes", "important_segments"):
        for candidate in scene.get(field, []):
            start, end = candidate.get("start_byte"), candidate.get("end_byte")
            surface = candidate.get("text", candidate.get("evidence", ""))
            if field == "important_segments":
                surface = candidate.get("evidence", "")
            if not isinstance(start, int) or not isinstance(end, int) or not 0 <= start < end <= len(raw):
                problems.append(f"{field}: invalid byte range")
                continue
            if raw[start:end].decode("utf-8", errors="replace") != surface:
                problems.append(f"{field}: output span differs from source")
    for query in scene.get("contextual_image_queries", []):
        start, end = query.get("evidence_start_byte"), query.get("evidence_end_byte")
        if not isinstance(start, int) or not isinstance(end, int) or not 0 <= start < end <= len(raw):
            problems.append("contextual query: invalid evidence span")
            continue
        if raw[start:end].decode("utf-8", errors="replace") != query.get("evidence"):
            problems.append("contextual query: evidence is not source grounded")
        for component in query.get("components", []):
            a, b = component.get("start_byte"), component.get("end_byte")
            if not isinstance(a, int) or not isinstance(b, int) or not start <= a < b <= end:
                problems.append("contextual query: component outside evidence")
            elif raw[a:b].decode("utf-8", errors="replace") != component.get("text"):
                problems.append("contextual query: component span differs from source")
    return problems


def score_one(worker: Worker, script_id: str, spec: dict) -> dict:
    source = (BASE / spec["file"]).read_text(encoding="utf-8").strip()
    request = {"operation": "analyze_script", "language": spec["language"],
               "scenes": [{"scene_id": script_id, "text": source}]}
    response = worker.request(request)
    if not response.get("ok"):
        raise RuntimeError(f"worker failed on {script_id}: {response.get('error')}")
    manifest = response["manifest"]
    scene = manifest["scenes"][0]
    entities = unique_candidates([x for x in scene.get("entities", []) if x.get("type") in NAMED_TYPES])
    objects = unique_candidates([x for x in scene.get("entities", []) if x.get("type") in OBJECT_TYPES])
    phrases = []
    for field in ("short_phrases", "long_phrases", "important_segments"):
        for item in scene.get(field, []):
            phrases.append({"text": item.get("evidence", item.get("text", "")), "type": field})

    entity_score = category_score(spec.get("entities", []), entities)
    object_exact = category_score(spec.get("visual_objects", []), objects)
    object_surface = category_score(spec.get("visual_objects", []), objects, typed=False)
    object_verbatim = exact_text_score([x["text"] for x in spec.get("visual_objects", [])],
                                       [x["text"] for x in objects])
    phrase_score = exact_text_score(spec.get("important_phrases", []), [x["text"] for x in phrases])
    segment_score = exact_text_score(spec.get("important_segments", []),
                                     [x["evidence"] for x in scene.get("important_segments", [])])

    fact_results = []
    for expected in spec.get("facts", []):
        hits = [fact for fact in scene.get("entity_facts", [])
                if fact.get("predicate") == expected.get("predicate")
                and (fact.get("status") or "asserted") == expected.get("status")
                and (str(fact.get("normalized_value") or fact.get("normalized_date") or fact.get("value")) == str(expected.get("value"))
                     or same_surface(str(fact.get("value", "")), str(expected.get("value", ""))))
                and expected.get("subject", "").casefold() in fact.get("evidence", "").casefold()]
        fact_results.append({"expected": expected, "found": bool(hits), "evidence": [x.get("evidence") for x in hits]})

    query_expected = spec.get("contextual_image_queries", [])
    actual_queries = scene.get("contextual_image_queries", [])
    query_hits = [next((query for query in actual_queries
                        if query.get("query") == expected["query"]
                        and query.get("evidence") == expected["evidence"]), None)
                  for expected in query_expected]

    chart_candidates = [x for x in scene.get("visual_proposals", []) if x.get("proposal_type") == "chart"]
    positive_chart = []
    for expected in spec.get("positive_charts", []):
        valid = []
        for chart in chart_candidates:
            items = chart.get("chart_items", [])
            values = sorted(item.get("value") for item in items if isinstance(item.get("value"), (int, float)))
            kinds = {item.get("kind") for item in items}
            expected_values = sorted(expected["values"])
            if values == expected_values and len(items) == len(expected_values):
                valid.append({"chart": chart, "kinds": sorted(kinds), "expected_unit": expected["unit"]})
        positive_chart.append({"expected": expected, "value_match": bool(valid), "type_match": any(set(v["kinds"]) == {expected["unit"]} for v in valid)})
    negative_evidence = [x for x in chart_candidates if any(e in x.get("evidence", "") for e in spec.get("negative_chart_evidence", []))]
    micro_candidates = scene.get("micro_summaries", [])
    micro_expectations = spec.get("micro_summary_expectations", [])

    replay = worker.request(request)
    replay_identical = replay.get("manifest") == manifest and replay.get("cached") is True
    return {
        "id": script_id,
        "language": spec["language"],
        "words": len(source.split()),
        "named_entities": entity_score,
        "visual_objects_typed": object_exact,
        "visual_objects_surface_only": object_surface,
        "visual_objects_exact_surface": object_verbatim,
        "important_phrases": phrase_score,
        "important_segments": segment_score,
        "facts": {"expected": len(fact_results), "found": sum(x["found"] for x in fact_results), "details": fact_results},
        "contextual_queries": {"expected": len(query_expected), "found": sum(x is not None for x in query_hits), "actual": actual_queries},
        "charts": {"positive_expected": len(positive_chart), "value_matches": sum(x["value_match"] for x in positive_chart),
                   "typed_matches": sum(x["type_match"] for x in positive_chart), "negative_evidence_matches": len(negative_evidence),
                   "actual_count": len(chart_candidates)},
        "sequence_items_expected": len(spec.get("sequence", [])),
        "sequence_items_exactly_extracted": sum(any(normalize(item) == normalize(candidate.get("text", ""))
                                                     for candidate in scene.get("bullets", []))
                                                  for item in spec.get("sequence", [])),
        "bullet_count": len(scene.get("bullets", [])),
        "micro_summary_count": len(scene.get("micro_summaries", [])),
        "micro_summary_review": {
            "expected": len(micro_expectations),
            "candidates": [x.get("text", x.get("summary", "")) for x in micro_candidates],
            "coverage": min(len(micro_candidates), len(micro_expectations)) / len(micro_expectations)
            if micro_expectations else 1.0,
            "fidelity_status": "pending_human_review",
            "expectations": micro_expectations,
        },
        "critical_invariants_for_editorial_review": {
            "status": "pending_human_review",
            "invariants": spec.get("critical_invariants", []),
        },
        "source_grounding_errors": source_grounding(scene, source),
        "replay_identical": replay_identical,
        "manifest_sha256": hashlib.sha256(json.dumps(manifest, ensure_ascii=False, sort_keys=True).encode()).hexdigest(),
    }


def category_row(rows: list[dict], path: tuple[str, str, str], label: str) -> dict:
    """Aggregate one category over `rows`; `path` is (score field, expected key, found key)."""
    field, expected_key, found_key = path
    expected = sum(row[field][expected_key] for row in rows)
    found = sum(row[field][found_key] for row in rows)
    return {
        label: {
            "annotated_scripts": sum(1 for row in rows if row[field][expected_key]),
            "expected": expected,
            "found": found,
            "recall": (found / expected) if expected else None,
        }
    }


CATEGORY_PATHS = (
    ("named_entities", ("named_entities", "expected", "found")),
    ("visual_objects_surface_only", ("visual_objects_surface_only", "expected", "found")),
    ("visual_objects_typed", ("visual_objects_typed", "expected", "found")),
    ("important_phrases", ("important_phrases", "expected", "found")),
    ("important_segments", ("important_segments", "expected", "found")),
    ("facts", ("facts", "expected", "found")),
    ("contextual_queries", ("contextual_queries", "expected", "found")),
    ("positive_charts", ("charts", "positive_expected", "typed_matches")),
)


def aggregate(rows: list[dict]) -> dict:
    """Per-category aggregation with explicit annotation coverage.

    A category with no annotation in this slice reports `recall: null` instead
    of a fabricated 0.0: an unannotated category is *not measured*, and
    conflating the two is how a gold set starts lying.
    """
    block: dict = {
        "scripts": len(rows),
        "words": sum(row["words"] for row in rows),
        "grounding_errors": sum(len(row["source_grounding_errors"]) for row in rows),
        "replay_identical": all(row["replay_identical"] for row in rows),
    }
    for label, path in CATEGORY_PATHS:
        block.update(category_row(rows, path, label))
    block["annotated_categories"] = sorted(
        label for label, path in CATEGORY_PATHS if block[label]["expected"]
    )
    block["sequences_exactly_extracted"] = {
        "expected": sum(row["sequence_items_expected"] for row in rows),
        "found": sum(row["sequence_items_exactly_extracted"] for row in rows),
    }
    block["bullets"] = sum(row["bullet_count"] for row in rows)
    return block


def per_language(results: list[dict]) -> dict:
    return {
        language: aggregate([row for row in results if row["language"] == language])
        for language in sorted({row["language"] for row in results})
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, default=DEFAULT_WORKER)
    parser.add_argument("--report", type=Path, default=Path("/tmp/brainrust_long_gold_report.json"))
    args = parser.parse_args()
    gold = json.loads((BASE / "gold.json").read_text(encoding="utf-8"))
    specs = gold["scripts"]
    if not args.worker.exists():
        parser.error(f"worker not found: {args.worker}")
    worker = Worker(args.worker)
    try:
        health = worker.request({"operation": "health"}, 30)
        if not health.get("ok"):
            raise RuntimeError(f"worker health check failed: {health}")
        results = [score_one(worker, key, spec) for key, spec in sorted(specs.items())]
    finally:
        worker.close()
    words = sum(row["words"] for row in results)
    languages = sorted({row["language"] for row in results})
    authority = (gold.get("annotation_policy") or {}).get("authority", "unspecified")
    release_sized = len(results) >= 20 and len(languages) == 10
    if authority != "human_annotator":
        certification = "blocked_pending_human_review"
    elif not release_sized:
        certification = "blocked_pending_release_size"
    else:
        certification = "certified"
    report = {
        "schema": "brainrust.editorial_gold_report.v1",
        "scripts": results,
        "corpus": {"script_count": len(results), "language_count": len(languages), "languages": languages,
                   "word_count": words, "is_release_sized": release_sized,
                   "annotation_authority": authority, "certification": certification},
        "overall": aggregate(results),
        "per_language": per_language(results),
    }
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report["corpus"], ensure_ascii=False))
    for language, block in report["per_language"].items():
        def ratio(label: str) -> str:
            row = block[label]
            return "n/a" if row["recall"] is None else f"{row['recall']:.3f} ({row['found']}/{row['expected']})"
        print(f"  {language}: scripts={block['scripts']} words={block['words']} "
              f"entities={ratio('named_entities')} objects={ratio('visual_objects_surface_only')} "
              f"phrases={ratio('important_phrases')} facts={ratio('facts')} charts={ratio('positive_charts')} "
              f"annotated={','.join(block['annotated_categories']) or '-'} "
              f"grounding-errors={block['grounding_errors']} replay-identical={block['replay_identical']}")
    for row in results:
        print(f"{row['id']}: words={row['words']} entity-recall={row['named_entities']['recall']:.3f} "
              f"object-recall={row['visual_objects_surface_only']['recall']:.3f} "
              f"phrase-recall={row['important_phrases']['recall']:.3f} "
              f"facts={row['facts']['found']}/{row['facts']['expected']} "
              f"queries={row['contextual_queries']['found']}/{row['contextual_queries']['expected']} "
              f"charts={row['charts']['typed_matches']}/{row['charts']['positive_expected']} "
              f"grounding-errors={len(row['source_grounding_errors'])}")
    print(f"report: {args.report}")
    return 0 if report["corpus"]["is_release_sized"] else 2


if __name__ == "__main__":
    raise SystemExit(main())

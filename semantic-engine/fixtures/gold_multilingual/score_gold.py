#!/usr/bin/env python3
"""Scorer for the multilingual gold set.

Drives the release worker (`release/semantic-worker`, mode `analyze_script`)
over the 20 annotated scripts and scores the manifest against `gold.json`:

  * recall      - gold entities found by an extracted entity of the same type
                  (case-folded token-set containment, so a head span such as
                  "parquet" finds the gold surface "Parquet de Paris"),
  * precision   - extracted named entities that match some gold entity,
  * noise       - filler phrases and chapter headings must never appear inside
                  a query or an entity; bare pronouns must never BE a query or
                  an entity.

The gold is authored from linguistic judgment (`gold.json` -> `annotation`).
A gold entity the engine misses is reported as a MISS; the scorer never
rewrites the gold.

Usage:
    python3 score_gold.py [--report /tmp/gold_score_report.json] [script_id ...]

Exit code 0 when every threshold in `gold.json` is met, 1 otherwise.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

BASE = Path(__file__).resolve().parent
ROOT = BASE.parents[2]
WORKER = ROOT / "release" / "semantic-worker"

NAMED_TYPES = {"PERSON", "LOCATION", "ORG", "ORGANIZATION", "WORK", "BRAND", "PRODUCT"}
TYPE_ALIAS = {"ORGANIZATION": "ORG"}

PUNCT = re.compile(r"^[^\w\d]+|[^\w\d]+$", re.UNICODE)


def norm(text: str) -> str:
    text = text.casefold().replace("’", "'")
    text = PUNCT.sub("", text)
    return re.sub(r"\s+", " ", text).strip()


def tokens(text: str) -> tuple[str, ...]:
    return tuple(norm(text).split())


def type_of(raw: str) -> str:
    return TYPE_ALIAS.get(raw, raw)


def contains(outer: tuple[str, ...], inner: tuple[str, ...]) -> bool:
    """True when `inner`'s tokens are a contiguous run inside `outer`."""
    if not inner or not outer or len(inner) > len(outer):
        return False
    n, m = len(outer), len(inner)
    return any(outer[i : i + m] == inner for i in range(n - m + 1))


def matches(ex_text: str, ex_type: str, gold_text: str, gold_type: str) -> bool:
    if type_of(ex_type) != type_of(gold_type):
        return False
    a, b = tokens(ex_text), tokens(gold_text)
    return contains(a, b) or contains(b, a)


def matches_any_type(ex_text: str, gold_text: str) -> bool:
    a, b = tokens(ex_text), tokens(gold_text)
    return contains(a, b) or contains(b, a)


def run_worker(script_text: str, language: str) -> dict:
    with tempfile.TemporaryDirectory(prefix="gold_score_") as tmp:
        tmp_path = Path(tmp)
        out = tmp_path / "manifest.json"
        job = {
            "job": "gold_multilingual",
            "job_schema": "worker.v1",
            "output": str(out),
            "mode": "analyze_script",
            "extension": "txt",
            "pipeline": "human",
            "language": language,
            "source_url": "gold://multilingual",
            "vendor": "human",
            "model": "human",
            "codec": "json",
        }
        job_path = tmp_path / "job.json"
        job_path.write_text(json.dumps(job), encoding="utf-8")
        payload = json.dumps({"path": "gold.txt", "text": script_text})
        proc = subprocess.run(
            [str(WORKER), "--job", str(job_path)],
            input=payload,
            capture_output=True,
            text=True,
            timeout=120,
        )
        if proc.returncode != 0:
            raise RuntimeError(f"worker failed: {proc.stderr[-2000:]}")
        return json.loads(out.read_text(encoding="utf-8"))


def collect_queries(manifest: dict) -> list[str]:
    queries: list[str] = []
    for scene in manifest.get("scenes", []):
        for proposal in scene.get("visual_proposals", []):
            queries.extend(proposal.get("visual_query_suggestions", []))
        for composed in scene.get("composed_image_queries", []):
            queries.append(composed.get("query", ""))
    feedback = manifest.get("search_feedback", {})
    for entry in feedback.get("entries", []):
        for query in entry.get("queries", []):
            queries.append(query.get("text", ""))
    return [q for q in queries if q]


def score_script(script_id: str, spec: dict, gold: dict) -> dict:
    language = spec["language"]
    script_text = (BASE / spec["file"]).read_text(encoding="utf-8").strip()
    manifest = run_worker(script_text, language)

    extracted: list[tuple[str, str]] = []
    seen = set()
    for scene in manifest.get("scenes", []):
        for entity in scene.get("entities", []):
            if entity.get("type") not in NAMED_TYPES:
                continue
            key = (norm(entity["text"]), type_of(entity["type"]))
            if key in seen:
                continue
            seen.add(key)
            extracted.append((entity["text"], entity["type"]))

    gold_entities = spec["entities"]
    misses, typed_misses = [], []
    for g in gold_entities:
        hits = [e for e in extracted if matches(e[0], e[1], g["text"], g["type"])]
        if not hits:
            text_hits = [e for e in extracted if matches_any_type(e[0], g["text"])]
            if text_hits:
                typed_misses.append({"gold": g, "extracted": [e[0] for e in text_hits]})
            else:
                misses.append(g)

    false_positives = [
        {"text": e[0], "type": e[1]}
        for e in extracted
        if not any(matches_any_type(e[0], g["text"]) for g in gold_entities)
    ]

    queries = collect_queries(manifest)
    leaks = []
    for noise in spec["noise"]:
        kind, text = noise["kind"], noise["text"]
        nq, ne = norm(text), tokens(text)
        if kind == "pronoun":
            for q in queries:
                if norm(q) == nq:
                    leaks.append({"noise": noise, "where": "query", "text": q})
            for e_text, _ in extracted:
                if norm(e_text) == nq:
                    leaks.append({"noise": noise, "where": "entity", "text": e_text})
        else:
            for q in queries:
                if nq and nq in norm(q):
                    leaks.append({"noise": noise, "where": "query", "text": q})
            for e_text, _ in extracted:
                if nq and nq in norm(e_text):
                    leaks.append({"noise": noise, "where": "entity", "text": e_text})
        del ne

    total_gold = len(gold_entities)
    found = total_gold - len(misses) - len(typed_misses)
    typed_found = total_gold - len(misses) - len(typed_misses)
    return {
        "id": script_id,
        "language": language,
        "gold": total_gold,
        "found": found,
        "typed_found": typed_found,
        "extracted_named": len(extracted),
        "correct": len(extracted) - len(false_positives),
        "misses": misses,
        "typed_misses": typed_misses,
        "false_positives": false_positives,
        "noise_leaks": leaks,
        "queries": len(queries),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", default="/tmp/gold_score_report.json")
    parser.add_argument("scripts", nargs="*")
    args = parser.parse_args()

    if not WORKER.exists():
        print(f"missing {WORKER}; build with: cargo build --release -p semantic-worker", file=sys.stderr)
        return 2

    gold = json.loads((BASE / "gold.json").read_text(encoding="utf-8"))
    specs = gold["scripts"]
    ids = args.scripts or sorted(specs)

    results = [score_script(i, specs[i], gold) for i in ids]

    thresholds = gold["annotation"]["scoring"]["thresholds"]
    total_gold = sum(r["gold"] for r in results)
    total_found = sum(r["found"] for r in results)
    total_extracted = sum(r["extracted_named"] for r in results)
    total_correct = sum(r["correct"] for r in results)
    total_leaks = sum(len(r["noise_leaks"]) for r in results)

    recall = total_found / total_gold if total_gold else 1.0
    precision = total_correct / total_extracted if total_extracted else 1.0

    print(f"{'script':<8}{'lang':<5}{'gold':>5}{'found':>6}{'ext':>5}{'ok':>5}{'FP':>4}{'leaks':>6}")
    for r in results:
        print(
            f"{r['id']:<8}{r['language']:<5}{r['gold']:>5}{r['found']:>6}"
            f"{r['extracted_named']:>5}{r['correct']:>5}{len(r['false_positives']):>4}{len(r['noise_leaks']):>6}"
        )
    print()
    print(f"recall    {recall:.3f}  ({total_found}/{total_gold})  >= {thresholds['recall']}")
    print(f"precision {precision:.3f}  ({total_correct}/{total_extracted})  >= {thresholds['precision']}")
    print(f"noise leaks {total_leaks}  == {thresholds['noise_leaks']}")
    for r in results:
        for m in r["misses"]:
            print(f"  MISS {r['id']}: {m['text']} ({m['type']})")
        for m in r["typed_misses"]:
            gold_text = m["gold"]["text"]
            print(
                f"  TYPED-MISS {r['id']}: {gold_text} ({m['gold']['type']}) "
                f"extracted as {m['extracted']}"
            )
        for f in r["false_positives"]:
            print(f"  FP {r['id']}: {f['text']} ({f['type']})")
        for l in r["noise_leaks"]:
            print(f"  NOISE {r['id']}: {l['noise']['text']} ({l['noise']['kind']}) in {l['where']}: {l['text']}")

    ok = (
        recall >= thresholds["recall"]
        and precision >= thresholds["precision"]
        and total_leaks == thresholds["noise_leaks"]
    )
    Path(args.report).write_text(
        json.dumps({"results": results, "recall": recall, "precision": precision, "leaks": total_leaks, "pass": ok}, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    print(f"\n{'PASS' if ok else 'FAIL'} - report: {args.report}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

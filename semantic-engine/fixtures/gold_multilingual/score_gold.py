#!/usr/bin/env python3
"""Scorer for the multilingual gold set.

Drives the release worker (`target/release/semantic-engine`, operation
`analyze_script`) over the 20 annotated scripts and scores the manifest
against `gold.json`:

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
    python3 score_gold.py [--worker PATH] [--report /tmp/gold_score_report.json] [script_id ...]
    python3 score_gold.py --check-reproducible

Exit code 0 when every threshold in `gold.json` is met, 1 otherwise.
`--check-reproducible` instead runs every script through two cold worker
processes and requires the manifest JSON to be byte-identical.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import select
import subprocess
import sys
import tempfile
import time
from pathlib import Path

BASE = Path(__file__).resolve().parent
ROOT = BASE.parents[2]
DEFAULT_WORKER = ROOT / "target" / "release" / "semantic-engine"

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


class NDJSONWorker:
    """Persistent worker on stdin/stdout (the same protocol as benchmark.py)."""

    def __init__(self, filename: Path):
        self.errlog = tempfile.TemporaryFile()
        self.p = subprocess.Popen(
            [str(filename)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.errlog,
        )
        self.buffer = b""

    def request(self, payload: dict, timeout: float = 120.0) -> dict:
        self.p.stdin.write(
            (json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")
        )
        self.p.stdin.flush()
        deadline = time.monotonic() + timeout
        while True:
            i = self.buffer.find(b"\n")
            if i >= 0:
                result = self.buffer[:i]
                self.buffer = self.buffer[i + 1 :]
                return json.loads(result)
            rem = deadline - time.monotonic()
            if rem <= 0 or self.p.poll() is not None:
                raise RuntimeError("worker exited or request deadline exceeded")
            ready, _, _ = select.select([self.p.stdout], [], [], rem)
            if not ready:
                raise TimeoutError("worker request timeout")
            chunk = os.read(self.p.stdout.fileno(), 1048576)
            if not chunk:
                raise RuntimeError("worker output EOF")
            self.buffer += chunk

    def close(self):
        try:
            self.p.stdin.close()
            self.p.wait(timeout=3)
        except Exception:
            self.p.kill()
            self.p.wait()
        self.errlog.close()


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


def score_script(worker: NDJSONWorker, script_id: str, spec: dict) -> dict:
    language = spec["language"]
    script_text = (BASE / spec["file"]).read_text(encoding="utf-8").strip()
    response = worker.request(
        {
            "operation": "analyze_script",
            "language": language,
            "scenes": [{"scene_id": "s01", "text": script_text}],
        }
    )
    if not response.get("ok"):
        raise RuntimeError(f"worker refused {script_id}: {response.get('error')}")
    manifest = response["manifest"]

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


def canonical_manifest(worker: NDJSONWorker, script_id: str, spec: dict) -> str:
    script_text = (BASE / spec["file"]).read_text(encoding="utf-8").strip()
    response = worker.request(
        {
            "operation": "analyze_script",
            "language": spec["language"],
            "scenes": [{"scene_id": "s01", "text": script_text}],
        }
    )
    if not response.get("ok"):
        raise RuntimeError(f"worker refused {script_id}: {response.get('error')}")
    return json.dumps(response["manifest"], ensure_ascii=False, separators=(",", ":"))


def check_reproducible(worker_path: Path, ids: list[str], specs: dict) -> bool:
    """Two cold worker processes; every manifest must match byte for byte."""
    runs: list[dict[str, str]] = []
    for run in range(2):
        worker = NDJSONWorker(worker_path)
        try:
            health = worker.request({"operation": "health"}, 30.0)
            if not health.get("ok"):
                raise RuntimeError(f"worker health failed: {health}")
            runs.append(
                {i: canonical_manifest(worker, i, specs[i]) for i in ids}
            )
        finally:
            worker.close()
    ok = True
    for script_id in ids:
        if runs[0][script_id] != runs[1][script_id]:
            print(f"  DIFF {script_id}: manifest differs between cold runs")
            ok = False
    print(
        f"reproducibility: {'PASS' if ok else 'FAIL'} "
        f"({len(ids)} scripts × 2 cold runs, byte-identical manifests)"
    )
    return ok


def per_language(results: list[dict]) -> dict[str, dict]:
    """Per-language entity recall/precision and critical errors.

    Gate C asks for per-language precision/recall and critical errors; the
    global thresholds stay untouched, this block only documents them.
    """
    block: dict[str, dict] = {}
    for language in sorted({r["language"] for r in results}):
        rows = [r for r in results if r["language"] == language]
        gold = sum(r["gold"] for r in rows)
        found = sum(r["found"] for r in rows)
        extracted = sum(r["extracted_named"] for r in rows)
        correct = sum(r["correct"] for r in rows)
        typed_misses = [m for r in rows for m in r["typed_misses"]]
        block[language] = {
            "scripts": len(rows),
            "gold_entities": gold,
            "found": found,
            "recall": found / gold if gold else None,
            "extracted": extracted,
            "correct": correct,
            "precision": correct / extracted if extracted else None,
            "critical_errors": {
                "entity_misses": sum(len(r["misses"]) for r in rows),
                "typed_misses": len(typed_misses),
                "noise_leaks": sum(len(r["noise_leaks"]) for r in rows),
            },
            "misses": [m for r in rows for m in r["misses"]],
            "typed_miss_details": typed_misses,
        }
    return block


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", default=str(DEFAULT_WORKER))
    parser.add_argument("--report", default="/tmp/gold_score_report.json")
    parser.add_argument(
        "--check-reproducible",
        action="store_true",
        help="two cold runs per script; manifests must be byte-identical",
    )
    parser.add_argument("scripts", nargs="*")
    args = parser.parse_args()

    worker_path = Path(args.worker)
    if not worker_path.exists():
        print(
            f"missing {worker_path}; build with: cargo build --release -p semantic-engine",
            file=sys.stderr,
        )
        return 2

    gold = json.loads((BASE / "gold.json").read_text(encoding="utf-8"))
    specs = gold["scripts"]
    ids = args.scripts or sorted(specs)

    if args.check_reproducible:
        return 0 if check_reproducible(worker_path, ids, specs) else 1

    worker = NDJSONWorker(worker_path)
    try:
        health = worker.request({"operation": "health"}, 30.0)
        if not health.get("ok"):
            raise RuntimeError(f"worker health failed: {health}")
        results = [score_script(worker, i, specs[i]) for i in ids]
    finally:
        worker.close()

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
    languages = per_language(results)
    Path(args.report).write_text(
        json.dumps(
            {
                "results": results,
                "recall": recall,
                "precision": precision,
                "leaks": total_leaks,
                "pass": ok,
                "annotation_authority": gold["annotation"].get("author", "unspecified"),
                "per_language": languages,
            },
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )
    print()
    for language, row in languages.items():
        recall_text = "n/a" if row["recall"] is None else f"{row['recall']:.3f}"
        precision_text = "n/a" if row["precision"] is None else f"{row['precision']:.3f}"
        print(
            f"  {language}: scripts={row['scripts']} entities={row['found']}/{row['gold_entities']} "
            f"recall={recall_text} precision={precision_text} "
            f"critical(miss={row['critical_errors']['entity_misses']}, "
            f"typed={row['critical_errors']['typed_misses']}, "
            f"noise={row['critical_errors']['noise_leaks']})"
        )
    print(f"\n{'PASS' if ok else 'FAIL'} - report: {args.report}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Compare persistent-worker and one-process-per-scene analysis costs."""

from __future__ import annotations

import argparse
import json
import math
import pathlib
import time

from benchmark import NDJSONWorker, make_scenes

ROOT = pathlib.Path(__file__).resolve().parent


def analyze(worker: NDJSONWorker, request: dict, timeout: float) -> dict:
    response = worker.request(request, timeout)
    if not response.get("ok"):
        raise RuntimeError(f"analysis failed: {response.get('error')}")
    return response["manifest"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", required=True)
    parser.add_argument("--language", default="it")
    parser.add_argument("--limit", type=int, default=5, help="number of scenes")
    parser.add_argument("--scene-words", type=int, default=550)
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--report", default="/tmp/brainrust_worker_performance.json")
    args = parser.parse_args()

    cases = json.loads((ROOT / "cases.json").read_text(encoding="utf-8"))
    case = next((item for item in cases if item["lang"] == args.language), None)
    if case is None:
        parser.error(f"no corpus case for language {args.language!r}")
    source = (ROOT / case["path"]).read_text(encoding="utf-8")
    scenes = make_scenes(source, args.scene_words)[: args.limit]
    requests = [
        {
            "operation": "analyze_script",
            "language": args.language,
            "scenes": [scene],
        }
        for scene in scenes
    ]
    if not requests:
        parser.error("scene limit produced no requests")

    persistent_start = time.monotonic()
    persistent = NDJSONWorker(args.worker)
    try:
        health = persistent.request({"operation": "health"}, args.timeout)
        if not health.get("ok"):
            raise RuntimeError(f"persistent worker health failed: {health}")
        persistent_times: list[float] = []
        persistent_manifests = []
        for request in requests:
            start = time.monotonic()
            persistent_manifests.append(analyze(persistent, request, args.timeout))
            persistent_times.append(time.monotonic() - start)
    finally:
        persistent.close()
    persistent_total = time.monotonic() - persistent_start

    cold_times: list[float] = []
    cold_manifests = []
    for request in requests:
        start = time.monotonic()
        worker = NDJSONWorker(args.worker)
        try:
            health = worker.request({"operation": "health"}, args.timeout)
            if not health.get("ok"):
                raise RuntimeError(f"cold worker health failed: {health}")
            cold_manifests.append(analyze(worker, request, args.timeout))
        finally:
            worker.close()
        cold_times.append(time.monotonic() - start)

    identical = cold_manifests == persistent_manifests
    if not identical:
        raise RuntimeError("cold and persistent worker manifests differ")

    def percentile(values: list[float], p: float) -> float:
        index = max(0, min(len(values) - 1, math.ceil(len(values) * p) - 1))
        return round(sorted(values)[index], 3)
    report = {
        "schema": "brainrust.worker_performance.v1",
        "language": args.language,
        "source_case": case["id"],
        "source_words": len(source.split()),
        "scenes": len(requests),
        "scene_word_budget": args.scene_words,
        "manifests_identical": identical,
        "persistent": {
            "total_seconds": round(persistent_total, 3),
            "request_seconds": [round(value, 3) for value in persistent_times],
            "p50_seconds": percentile(persistent_times, 0.50),
            "p95_seconds": percentile(persistent_times, 0.95),
        },
        "cold_per_scene": {
            "total_seconds": round(sum(cold_times), 3),
            "request_seconds_including_startup": [
                round(value, 3) for value in cold_times
            ],
            "p50_seconds": percentile(cold_times, 0.50),
            "p95_seconds": percentile(cold_times, 0.95),
        },
        "persistent_speedup": round(sum(cold_times) / persistent_total, 3),
    }
    output = pathlib.Path(args.report)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    print(f"REPORT: {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

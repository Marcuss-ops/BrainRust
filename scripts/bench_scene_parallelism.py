#!/usr/bin/env python3
"""DEMOL-10 benchmark: scene parallelism and the per-scene cache.

Drives the release worker (`target/release/semantic-engine`) over the 20
long-form gold scripts (10 languages, 5,669 words) and measures:

  * cold wall-time with RAYON_NUM_THREADS=1 versus N threads (default 4);
  * warm wall-time after changing ONE scene of a multi-scene document, with a
    persistent per-scene cache (`SEMANTIC_CACHE_DIR`).

The worker is a persistent NDJSON process, exactly like `score_gold.py`: model
loading is paid once, so the numbers are comparable with the historical
40.34-51.95 s baseline recorded in
`docs/tickets/ROADMAP-2026-11-19-performance-prodotto.md`.

Usage:
    python3 scripts/bench_scene_parallelism.py \
        --report docs/evidence/demol10-scene-parallelism-2026-11-26.json

Exit code 0 when every measurement completed; the thresholds themselves are
judged in the ticket, never silently "passed" here.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import select
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_WORKER = ROOT / "target" / "release" / "semantic-engine"
DEFAULT_CORPUS = (
    ROOT / "semantic-engine" / "fixtures" / "gold_multilingual" / "long_scripts"
)
DEFAULT_MODELS = ROOT / ".tmp" / "udpipe-models"
REPETITIONS = 2


class NDJSONWorker:
    """One persistent worker process speaking the stdio protocol."""

    def __init__(self, binary: Path, env: dict[str, str]):
        self.errlog = tempfile.TemporaryFile()
        self.p = subprocess.Popen(
            [str(binary)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.errlog,
            env=env,
        )
        self.buffer = b""

    def request(self, payload: dict, timeout: float = 300.0) -> dict:
        assert self.p.stdin is not None and self.p.stdout is not None
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
            remaining = deadline - time.monotonic()
            if remaining <= 0 or self.p.poll() is not None:
                raise RuntimeError("worker exited or request deadline exceeded")
            ready, _, _ = select.select([self.p.stdout], [], [], remaining)
            if not ready:
                raise TimeoutError("worker request timeout")
            chunk = os.read(self.p.stdout.fileno(), 1048576)
            if not chunk:
                raise RuntimeError("worker output EOF")
            self.buffer += chunk

    def close(self) -> None:
        try:
            assert self.p.stdin is not None
            self.p.stdin.close()
            self.p.wait(timeout=5)
        except Exception:
            self.p.kill()
            self.p.wait()
        finally:
            self.errlog.close()


def worker_env(threads: int, cache_dir: Path | None) -> dict[str, str]:
    env = dict(os.environ)
    env["RAYON_NUM_THREADS"] = str(threads)
    if cache_dir is not None:
        env["SEMANTIC_CACHE_DIR"] = str(cache_dir)
    else:
        env.pop("SEMANTIC_CACHE_DIR", None)
    if DEFAULT_MODELS.is_dir() and "VISUALNER_MODEL_DIR" not in env:
        env["VISUALNER_MODEL_DIR"] = str(DEFAULT_MODELS)
    return env


def corpus_scripts(corpus: Path) -> list[tuple[str, str, Path]]:
    """(script_id, language, path) for every long script, in stable order."""
    scripts: list[tuple[str, str, Path]] = []
    for path in sorted(corpus.glob("*.txt")):
        language = path.stem.split("_", 1)[0]
        scripts.append((path.stem, language, path))
    if not scripts:
        raise RuntimeError(f"no long scripts found in {corpus}")
    return scripts


def run_corpus(
    binary: Path, scripts: list[tuple[str, str, Path]], threads: int, cache_dir: Path | None
) -> dict:
    worker = NDJSONWorker(binary, worker_env(threads, cache_dir))
    per_script: list[dict] = []
    started = time.monotonic()
    try:
        for script_id, language, path in scripts:
            text = path.read_text(encoding="utf-8").strip()
            request_started = time.monotonic()
            response = worker.request(
                {
                    "operation": "analyze_script",
                    "language": language,
                    "scenes": [{"scene_id": "s01", "text": text}],
                }
            )
            elapsed = time.monotonic() - request_started
            if not response.get("ok"):
                raise RuntimeError(f"worker refused {script_id}: {response.get('error')}")
            per_script.append(
                {
                    "script_id": script_id,
                    "language": language,
                    "words": len(text.split()),
                    "seconds": round(elapsed, 4),
                    "cached": bool(response.get("cached")),
                }
            )
    finally:
        worker.close()
    total = time.monotonic() - started
    durations = [row["seconds"] for row in per_script]
    return {
        "threads": threads,
        "cache": "off" if cache_dir is None else "on",
        "total_seconds": round(total, 4),
        "wall_seconds_sum": round(sum(durations), 4),
        "words": sum(row["words"] for row in per_script),
        "words_per_second": round(sum(row["words"] for row in per_script) / total, 1),
        "p50_script_seconds": round(statistics.median(durations), 4),
        "p95_script_seconds": round(sorted(durations)[max(0, int(len(durations) * 0.95) - 1)], 4),
        "cached_responses": sum(1 for row in per_script if row["cached"]),
        "scripts": per_script,
    }


def split_sentences(text: str) -> list[str]:
    return [
        sentence.strip()
        for sentence in text.replace("!", ".").replace("?", ".").split(".")
        if sentence.strip()
    ]


def scenes_from_text(text: str, sentences_per_scene: int) -> list[str]:
    """Production-shaped scenes: a fixed number of sentences per scene."""
    sentences = split_sentences(text)
    if not sentences:
        return [text.strip()]
    return [
        ". ".join(sentences[i : i + sentences_per_scene]) + "."
        for i in range(0, len(sentences), sentences_per_scene)
    ]


def multi_scene_request(path: Path, language: str, scenes: int = 6) -> dict:
    """Split one real long script into `scenes` paragraph-sized scenes."""
    text = path.read_text(encoding="utf-8").strip()
    paragraphs = [p.strip() for p in text.split("\n\n") if p.strip()]
    if len(paragraphs) < scenes:
        paragraphs = scenes_from_text(text, max(1, len(split_sentences(text)) // scenes))
    paragraphs = paragraphs[:scenes]
    return {
        "operation": "analyze_script",
        "language": language,
        "scenes": [
            {"scene_id": f"s{index + 1:02d}", "text": body}
            for index, body in enumerate(paragraphs)
        ],
    }


def run_corpus_multi_scene(
    binary: Path,
    scripts: list[tuple[str, str, Path]],
    threads: int,
    sentences_per_scene: int,
    warm_models: bool = False,
) -> dict:
    """The same 5,669-word corpus, split into production-shaped scenes.

    A single-scene request has no scene-local parallelism to expose: the
    40.34-51.95 s baseline in the ROADMAP submitted one script per request, so
    it can only measure the sequential path. This section feeds the identical
    words as multi-scene documents, which is how a real 5,669-word script
    arrives, and is the workload phase 1 parallelises.

    `warm_models` loads every language's UDPipe model before the timed pass.
    Without it the first script of each language pays ~2.5 s of one-time model
    loading inside the measurement, which is startup cost, not scene work.
    """
    worker = NDJSONWorker(binary, worker_env(threads, None))
    per_script: list[dict] = []
    started = 0.0
    try:
        if warm_models:
            for language in sorted({language for _, language, _ in scripts}):
                response = worker.request(
                    {
                        "operation": "analyze_script",
                        "language": language,
                        "scenes": [{"scene_id": "warm", "text": "Il comitato ha approvato il piano."}],
                    }
                )
                if not response.get("ok"):
                    raise RuntimeError(f"model warmup refused for {language}")
        # Time the corpus only: the warmup is one-time startup cost.
        started = time.monotonic()
        for script_id, language, path in scripts:
            text = path.read_text(encoding="utf-8").strip()
            scenes = scenes_from_text(text, sentences_per_scene)
            request_started = time.monotonic()
            response = worker.request(
                {
                    "operation": "analyze_script",
                    "language": language,
                    "scenes": [
                        {"scene_id": f"s{index + 1:02d}", "text": body}
                        for index, body in enumerate(scenes)
                    ],
                }
            )
            elapsed = time.monotonic() - request_started
            if not response.get("ok"):
                raise RuntimeError(f"worker refused {script_id}: {response.get('error')}")
            per_script.append(
                {
                    "script_id": script_id,
                    "language": language,
                    "words": len(text.split()),
                    "scenes": len(scenes),
                    "seconds": round(elapsed, 4),
                }
            )
    finally:
        worker.close()
    total = time.monotonic() - started
    durations = [row["seconds"] for row in per_script]
    return {
        "threads": threads,
        "sentences_per_scene": sentences_per_scene,
        "model_warmup": warm_models,
        "total_seconds": round(total, 4),
        "words": sum(row["words"] for row in per_script),
        "scenes": sum(row["scenes"] for row in per_script),
        "words_per_second": round(sum(row["words"] for row in per_script) / total, 1),
        "p50_script_seconds": round(statistics.median(durations), 4),
        "p95_script_seconds": round(sorted(durations)[max(0, int(len(durations) * 0.95) - 1)], 4),
        "scripts": per_script,
    }


def scene_cache_entries(cache_dir: Path | None) -> int:
    if cache_dir is None:
        return 0
    scenes = cache_dir / "scenes"
    if not scenes.is_dir():
        return 0
    return sum(1 for entry in scenes.iterdir() if entry.name.startswith("scene-"))


def run_warm_single_scene(binary: Path, script: Path, language: str, threads: int) -> dict:
    request = multi_scene_request(script, language)
    cache_dir = Path(tempfile.mkdtemp(prefix="semantic-bench-cache-"))
    worker = NDJSONWorker(binary, worker_env(threads, cache_dir))
    try:
        started = time.monotonic()
        cold = worker.request(request)
        cold_seconds = time.monotonic() - started
        if not cold.get("ok"):
            raise RuntimeError(f"cold multi-scene run failed: {cold.get('error')}")

        # One modified scene: the document fingerprint changes, so phase 2 runs
        # again, while every untouched scene must hit its Level-1 entry.
        changed = json.loads(json.dumps(request))
        changed["scenes"][2]["text"] += (
            " Nel 2027 il nuovo impianto è stato inaugurato a Bologna."
        )
        started = time.monotonic()
        warm = worker.request(changed)
        warm_seconds = time.monotonic() - started
        if not warm.get("ok"):
            raise RuntimeError(f"warm multi-scene run failed: {warm.get('error')}")
        entries = scene_cache_entries(cache_dir)
        baseline_entries = scene_cache_entries(cache_dir)
    finally:
        worker.close()
        shutil.rmtree(cache_dir, ignore_errors=True)
    return {
        "script": script.stem,
        "language": language,
        "scenes": len(request["scenes"]),
        "threads": threads,
        "cold_seconds": round(cold_seconds, 4),
        "warm_one_scene_changed_seconds": round(warm_seconds, 4),
        "warm_over_cold": round(warm_seconds / cold_seconds, 4) if cold_seconds else None,
        "cache_entries": entries,
        "cache_entries_baseline_note": baseline_entries,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, default=DEFAULT_WORKER)
    parser.add_argument("--corpus", type=Path, default=DEFAULT_CORPUS)
    parser.add_argument("--threads", default="1,4")
    parser.add_argument("--sentences-per-scene", type=int, default=4)
    parser.add_argument(
        "--warm-models",
        action="store_true",
        help="load every language's UDPipe model before the multi-scene timed pass",
    )
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()

    if not args.worker.is_file():
        raise SystemExit(f"worker not found: {args.worker} (run `cargo build --release -p semantic-engine`)")
    thread_counts = [int(value) for value in args.threads.split(",") if value.strip()]

    scripts = corpus_scripts(args.corpus)
    corpus_words = sum(len(path.read_text(encoding="utf-8").split()) for _, _, path in scripts)

    runs: list[dict] = []
    for threads in thread_counts:
        for repetition in range(REPETITIONS):
            run = run_corpus(args.worker, scripts, threads, None)
            run["repetition"] = repetition + 1
            runs.append(run)
            print(
                f"threads={threads} rep={repetition + 1}: "
                f"{run['total_seconds']:.2f}s total, "
                f"{run['words_per_second']:.0f} words/s",
                flush=True,
            )

    by_threads: dict[int, list[float]] = {}
    for run in runs:
        by_threads.setdefault(run["threads"], []).append(run["total_seconds"])
    baseline_threads = thread_counts[0]
    parallel_threads = thread_counts[-1]
    baseline = min(by_threads[baseline_threads])
    parallel = min(by_threads[parallel_threads])

    multi_scene_runs: list[dict] = []
    for threads in thread_counts:
        for repetition in range(REPETITIONS):
            run = run_corpus_multi_scene(
                args.worker, scripts, threads, args.sentences_per_scene, args.warm_models
            )
            run["repetition"] = repetition + 1
            multi_scene_runs.append(run)
            print(
                f"multi-scene threads={threads} rep={repetition + 1}: "
                f"{run['total_seconds']:.2f}s total over {run['scenes']} scenes, "
                f"{run['words_per_second']:.0f} words/s",
                flush=True,
            )
    multi_by_threads: dict[int, list[float]] = {}
    for run in multi_scene_runs:
        multi_by_threads.setdefault(run["threads"], []).append(run["total_seconds"])
    multi_baseline = min(multi_by_threads[baseline_threads])
    multi_parallel = min(multi_by_threads[parallel_threads])

    warm_script, warm_language, warm_path = next(
        (row for row in scripts if row[1] == "it"), scripts[0]
    )
    warm = run_warm_single_scene(args.worker, warm_path, warm_language, parallel_threads)
    print(
        f"warm 1/{warm['scenes']} scene changed: {warm['warm_one_scene_changed_seconds']:.2f}s "
        f"({warm['warm_over_cold'] * 100:.0f}% of cold {warm['cold_seconds']:.2f}s)",
        flush=True,
    )

    report = {
        "ticket": "DEMOL-10",
        "date": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "host": {
            "platform": platform.platform(),
            "cpu_count": os.cpu_count(),
            "worker_sha": subprocess.run(
                ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                capture_output=True,
                text=True,
                check=False,
            ).stdout.strip(),
        },
        "corpus": {
            "path": str(args.corpus.relative_to(ROOT)),
            "scripts": len(scripts),
            "words": corpus_words,
        },
        "cold": {
            "repetitions": REPETITIONS,
            "baseline_threads": baseline_threads,
            "parallel_threads": parallel_threads,
            "baseline_min_seconds": round(baseline, 4),
            "parallel_min_seconds": round(parallel, 4),
            "speedup": round(baseline / parallel, 2) if parallel else None,
            "note": "one scene per request, the ROADMAP baseline shape: no scene-local parallelism to expose",
            "runs": runs,
        },
        "cold_multi_scene": {
            "repetitions": REPETITIONS,
            "baseline_threads": baseline_threads,
            "parallel_threads": parallel_threads,
            "sentences_per_scene": args.sentences_per_scene,
            "model_warmup": args.warm_models,
            "baseline_min_seconds": round(multi_baseline, 4),
            "parallel_min_seconds": round(multi_parallel, 4),
            "speedup": round(multi_baseline / multi_parallel, 2) if multi_parallel else None,
            "runs": multi_scene_runs,
        },
        "warm_single_scene": warm,
    }
    rendered = json.dumps(report, indent=2, ensure_ascii=False)
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(rendered + "\n", encoding="utf-8")
        print(f"report: {args.report}", flush=True)
    else:
        print(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main())

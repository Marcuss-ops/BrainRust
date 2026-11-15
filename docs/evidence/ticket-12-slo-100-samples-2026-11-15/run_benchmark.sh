#!/usr/bin/env bash
# Reproduce the TICKET-12 WP4 SLO evidence: 100 warm samples per probe,
# with UDPipe and model-free, on the declared reference runner.
#
# Usage:
#   ./run_benchmark.sh [all|udpipe|model-free]
#
# Environment:
#   VISUALNER_MODEL_DIR  override the model directory (default: <repo>/.tmp/udpipe-models)
#
# The script records the exact source SHA, dirty-tree count, CPU/OS/Rust/ICU,
# model and probe hashes, then writes udpipe/ and model-free/ raw JSON plus
# summary.json and summary.md. It fails closed if any probe did not produce
# exactly 100 samples.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
MODEL_DIR="${VISUALNER_MODEL_DIR:-$ROOT/.tmp/udpipe-models}"
OUT="$HERE"
MODE="${1:-all}"
REAL_HOME="$HOME"
REAL_CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
REAL_RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
MODEL_FREE_HOME="$OUT/model-free/home"
MODEL_FREE_XDG="$OUT/model-free/xdg"
cd "$ROOT"

if [ ! -f "$MODEL_DIR/italian-isdt-ud-2.5-191206.udpipe" ]; then
    echo "missing Italian UDPipe model in $MODEL_DIR" >&2
    echo "run ./visualner/tools/install_udpipe_models.sh first" >&2
    exit 1
fi

mkdir -p "$MODEL_FREE_HOME" "$MODEL_FREE_XDG"
if [ -e "$MODEL_FREE_HOME/.local/share/visualner/models" ] \
    || find "$MODEL_FREE_HOME" "$MODEL_FREE_XDG" -type f -name '*.udpipe' | grep -q .; then
    echo "model-free HOME is not empty: $MODEL_FREE_HOME" >&2
    exit 1
fi

{
    printf 'schema=ticket12.slo_environment.v1\n'
    printf 'generated_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'git_sha=%s\n' "$(git rev-parse HEAD)"
    printf 'git_dirty_entries=%s\n' "$(git status --porcelain | wc -l | tr -d ' ')"
    printf 'git_status_sha256=%s\n' "$(git status --porcelain | sha256sum | awk '{print $1}')"
    printf 'uname=%s\n' "$(uname -a)"
    printf 'os=%s\n' "$(sed -n 's/^PRETTY_NAME=//p' /etc/os-release | tr -d '"')"
    printf 'cpu=%s\n' "$(lscpu | sed -n 's/^Model name:[[:space:]]*//p' | head -n 1)"
    printf 'cpus=%s\n' "$(nproc)"
    printf 'memory_total=%s\n' "$(free -h | awk '/^Mem:/ {print $2}')"
    printf 'loadavg_before=%s\n' "$(cat /proc/loadavg)"
    printf 'rustc=%s\n' "$(rustc -V)"
    printf 'icu=%s\n' "$(pkg-config --modversion icu-i18n 2>/dev/null || echo unavailable)"
    printf 'model_dir=%s\n' "$MODEL_DIR"
    printf 'model_free_home=%s\n' "$MODEL_FREE_HOME"
    printf 'model_free_has_udpipe=0\n'
    printf 'italian_model_sha256=%s\n' \
        "$(sha256sum "$MODEL_DIR/italian-isdt-ud-2.5-191206.udpipe" | awk '{print $1}')"
    printf 'probe_source_sha256=%s\n' \
        "$(sha256sum "$ROOT/semantic-engine/tests/performance_probe.rs" | awk '{print $1}')"
    printf 'cargo_lock_sha256=%s\n' \
        "$(sha256sum "$ROOT/Cargo.lock" | awk '{print $1}')"
} > "$OUT/environment.txt"
cat "$OUT/environment.txt"

run_probe() {
    local label="$1"
    local dir="$OUT/$label"
    mkdir -p "$dir"
    echo "== $label ==" | tee "$dir/probe.log"
    if [ "$label" = "udpipe" ]; then
        VISUALNER_MODEL_DIR="$MODEL_DIR" \
            BRAINRUST_PERF_ARCHIVE="$dir" \
            BRAINRUST_PERF_LABEL="$label" \
            cargo test -p semantic-engine --test performance_probe --release --locked -- \
            --ignored --nocapture --test-threads=1 2>&1 | tee -a "$dir/probe.log"
    else
        env -u VISUALNER_MODEL_DIR \
            HOME="$MODEL_FREE_HOME" \
            XDG_DATA_HOME="$MODEL_FREE_XDG" \
            CARGO_HOME="$REAL_CARGO_HOME" \
            RUSTUP_HOME="$REAL_RUSTUP_HOME" \
            BRAINRUST_PERF_ARCHIVE="$dir" \
            BRAINRUST_PERF_LABEL="$label" \
            cargo test -p semantic-engine --test performance_probe --release --locked -- \
            --ignored --nocapture --test-threads=1 2>&1 | tee -a "$dir/probe.log"
    fi
    printf 'loadavg_after=%s\n' "$(cat /proc/loadavg)" | tee -a "$dir/probe.log"
}

case "$MODE" in
    all)
        run_probe udpipe
        run_probe model-free
        ;;
    udpipe | model-free)
        run_probe "$MODE"
        ;;
    *)
        echo "usage: $0 [all|udpipe|model-free]" >&2
        exit 2
        ;;
esac

python3 - "$OUT" <<'PY'
import json
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
summary = {
    "schema": "ticket12.slo_100_samples.v1",
    "fixture": "20 scenes, Italian, ESA/NASA/SpaceX/Mars; each scene repeats one sentence twice",
    "warmup": "one analysis/extract before sampling; warmup excluded from samples",
    "runs_per_probe": 100,
    "modes": {},
}
for label in ("udpipe", "model-free"):
    paths = [out / label / f"{label}-{probe}.json" for probe in ("twenty-scene", "single-scene")]
    if not all(path.exists() for path in paths):
        continue
    mode = {}
    for probe, path in zip(("twenty-scene", "single-scene"), paths):
        data = json.loads(path.read_text(encoding="utf-8"))
        assert data["warm_runs"] == 100, (path, data["warm_runs"])
        assert len(data["samples_ms"]) == 100, path
        mode[probe] = {
            key: data[key]
            for key in (
                "warm_runs",
                "min_ms",
                "mean_ms",
                "p50_ms",
                "p95_ms",
                "p99_ms",
                "max_ms",
            )
        }
    summary["modes"][label] = mode
assert summary["modes"], "no complete probe pair found"

(out / "summary.json").write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)

lines = [
    "# TICKET-12 SLO evidence — 100 warm samples",
    "",
    "Generated by `run_benchmark.sh`; raw samples are in the per-mode JSON files.",
    "",
    "| Mode | Probe | p50 (ms) | p95 (ms) | p99 (ms) | max (ms) |",
    "| --- | --- | ---: | ---: | ---: | ---: |",
]
for label in ("udpipe", "model-free"):
    for probe in ("twenty-scene", "single-scene"):
        row = summary["modes"][label][probe]
        lines.append(
            f"| {label} | {probe} | {row['p50_ms']:.3f} | {row['p95_ms']:.3f} | "
            f"{row['p99_ms']:.3f} | {row['max_ms']:.3f} |"
        )
(out / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
print(json.dumps(summary, indent=2, sort_keys=True))
PY

echo "✅ evidence in $OUT"

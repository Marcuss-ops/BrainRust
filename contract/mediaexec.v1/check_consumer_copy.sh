#!/usr/bin/env bash
# Check that a consumer's golden copy matches the canonical contract set.
#
# Usage:
#   check_consumer_copy.sh <consumer-goldens-dir>
#
# PipelineGen keeps testdata/mediaexec/v1 because its Go CI checks out the
# monorepo without submodules. The copy must stay byte-identical to
# contract/mediaexec.v1/goldens; this script is the parity gate.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CANONICAL="$HERE/goldens"
CONSUMER="${1:?usage: check_consumer_copy.sh <consumer-goldens-dir>}"

[ -d "$CANONICAL" ] || { echo "canonical goldens missing: $CANONICAL" >&2; exit 1; }
[ -d "$CONSUMER" ] || { echo "consumer goldens directory missing: $CONSUMER" >&2; exit 1; }

status=0
for source in "$CANONICAL"/*.json; do
    name="$(basename "$source")"
    if ! cmp -s "$source" "$CONSUMER/$name"; then
        echo "contract drift: $CONSUMER/$name differs from $source" >&2
        status=1
    fi
done
for copy in "$CONSUMER"/*.json; do
    name="$(basename "$copy")"
    if [ ! -f "$CANONICAL/$name" ]; then
        echo "consumer-only golden: $CONSUMER/$name" >&2
        status=1
    fi
done

[ "$status" -eq 0 ] || exit 1
echo "✅ consumer goldens match contract/mediaexec.v1/goldens"

#!/usr/bin/env bash
# TICKET-12 WP4: prove the analysis path performs no outbound network I/O.
#
# The application has no HTTP client, but absence of application calls is not
# evidence. This runs the real binary inside an isolated network namespace and
# requires a successful analysis response. It exits 2 (not 0) when the host
# cannot create a namespace, so a CI job cannot report a false pass.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${1:-$ROOT/target/release/semantic-engine}"
[ -x "$BIN" ] || BIN="$ROOT/target/debug/semantic-engine"
[ -x "$BIN" ] || {
    echo "check-no-network: build semantic-engine first ($BIN missing)" >&2
    exit 2
}

command -v unshare >/dev/null 2>&1 || {
    echo "check-no-network: unshare is required for OS-level isolation" >&2
    exit 2
}
if ! unshare -n true 2>/dev/null; then
    echo "check-no-network: this host cannot create a network namespace (CAP_SYS_ADMIN)" >&2
    exit 2
fi

REQUEST='{"operation":"analyze_script","language":"it","scenes":[{"scene_id":"s1","text":"ESA e NASA studiano Marte con nuove missioni spaziali."}]}'
RESPONSE="$(printf '%s\n' "$REQUEST" | unshare -n "$BIN")"
printf '%s\n' "$RESPONSE" | grep -q '"ok":true' || {
    echo "check-no-network: analysis failed under network isolation: $RESPONSE" >&2
    exit 1
}
printf '%s\n' "$RESPONSE" | grep -q '"entities"' || {
    echo "check-no-network: v3 entity projection missing: $RESPONSE" >&2
    exit 1
}
echo "check-no-network: analysis completed with zero network access"

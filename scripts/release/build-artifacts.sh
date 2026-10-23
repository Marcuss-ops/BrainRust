#!/usr/bin/env bash
# Build the release assets consumed by PipelineGen's `make rust-fetch`.
#
# Usage:
#   scripts/release/build-artifacts.sh [OUT_DIR]   # default: ./dist
#
# Must run on Debian Bookworm (ICU 72) so the binaries link the same ABI as
# the PipelineGen runtime image. Fails closed when the host ICU disagrees with
# the canonical ICU_MAJOR.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-$ROOT/dist}"
cd "$ROOT"

die() { echo "build-artifacts: $*" >&2; exit 1; }

major="$(tr -dc '0-9' < ICU_MAJOR)"
[ -n "$major" ] || die "ICU_MAJOR is empty"
command -v pkg-config >/dev/null 2>&1 || die "pkg-config is required to verify the ICU pin"
host_major="$(pkg-config --modversion icu-i18n | cut -d. -f1)"
[ "$host_major" = "$major" ] || die "ICU_MAJOR=$major but pkg-config reports ICU $host_major; build on Debian Bookworm (ICU $major)"

export RUST_ICU_MAJOR_VERSION_NUMBER="$major"

cargo build --release --locked --workspace

rm -rf "$OUT"
mkdir -p "$OUT"
for bin in pipelinegen-muscles visualner mediasampler semantic-engine; do
    [ -x "target/release/$bin" ] || die "missing release binary: target/release/$bin"
    install -m 0755 "target/release/$bin" "$OUT/$bin"
done

command -v ldd >/dev/null 2>&1 || die "ldd is required to verify the ICU linkage"
ldd "$OUT/visualner" | grep -q "libicuuc.so.$major" \
    || die "visualner is not linked against libicuuc.so.$major"

(
    cd "$OUT"
    sha256sum pipelinegen-muscles visualner mediasampler semantic-engine > sha256sums.txt
)
rustc -vV > "$OUT/TOOLCHAIN"
printf '%s\n' "$major" > "$OUT/ICU_MAJOR"

echo "✅ release assets in $OUT (ICU $major)"
ls -l "$OUT"

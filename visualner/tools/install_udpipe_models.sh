#!/usr/bin/env bash
# Install the UDPipe 2.5 models declared by language_profiles.json.
#
# The Rust test suite fails closed when a profile declares a treebank but the
# model is missing (visualner/tests/language_profiles.rs). The models are
# 234 MB in total, so they are provisioned by this script instead of being
# committed to the repository. CI caches the destination directory.
#
# Usage:
#   visualner/tools/install_udpipe_models.sh [LANGUAGE ...]
#
# Environment:
#   VISUALNER_MODEL_DIR   destination directory
#                         (default: $HOME/.local/share/visualner/models)
#   UDPIPE_MODEL_BASE     LINDAT bitstream base URL
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROFILES="$(cd "$HERE/../.." && pwd)/lexicon/language_profiles.json"
DEST="${VISUALNER_MODEL_DIR:-$HOME/.local/share/visualner/models}"
BASE="${UDPIPE_MODEL_BASE:-https://lindat.mff.cuni.cz/repository/xmlui/bitstream/handle/11234/1-3131}"

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required to read $PROFILES" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required to download the UDPipe models" >&2
  exit 1
fi
if [ ! -f "$PROFILES" ]; then
  echo "language profile table not found: $PROFILES" >&2
  exit 1
fi

mkdir -p "$DEST"

mapfile -t MODELS < <(python3 - "$PROFILES" "$@" <<'PY'
import json
import sys

path, *wanted = sys.argv[1:]
with open(path, encoding="utf-8") as handle:
    profiles = json.load(handle)["profiles"]
selected = set(wanted)
for code, profile in sorted(profiles.items()):
    treebank = profile.get("treebank")
    if not treebank:
        continue
    if selected and code not in selected and treebank not in selected:
        continue
    print(f"{treebank}-ud-2.5-191206.udpipe")
PY
)

if [ "${#MODELS[@]}" -eq 0 ]; then
  echo "no UDPipe model matches the requested languages" >&2
  exit 1
fi

for model in "${MODELS[@]}"; do
  target="$DEST/$model"
  if [ -s "$target" ]; then
    echo "present  $model"
    continue
  fi
  echo "download $model"
  tmp="$target.part"
  rm -f "$tmp"
  curl -fsSL --retry 3 --retry-delay 2 -o "$tmp" "$BASE/$model?sequence=1&isAllowed=y"
  if [ ! -s "$tmp" ]; then
    echo "downloaded model is empty: $model" >&2
    rm -f "$tmp"
    exit 1
  fi
  mv "$tmp" "$target"
done

missing=0
for model in "${MODELS[@]}"; do
  if [ ! -s "$DEST/$model" ]; then
    echo "missing $model" >&2
    missing=1
  fi
done
if [ "$missing" -ne 0 ]; then
  exit 1
fi
echo "UDPipe models ready in $DEST (${#MODELS[@]} declared)"

#!/usr/bin/env bash
# Install the UDPipe 2.5 models declared by language_profiles.json.
#
# The Rust test suite fails closed when a profile declares a treebank but the
# model is missing or unusable (visualner/tests/language_profiles.rs). The
# models are ~234 MB in total, so they are provisioned here instead of being
# committed to the repository. CI caches the destination directory.
#
# LINDAT migrated to DSpace 7: the old
# `/repository/xmlui/bitstream/handle/11234/1-3131/<file>` URL now redirects to
# the Angular UI and returns HTML with HTTP 200. This installer resolves the
# real bitstream through the DSpace API, then verifies its byte size and MD5
# before installing it. A repository error can no longer masquerade as a model.
#
# Usage:
#   visualner/tools/install_udpipe_models.sh [LANGUAGE ...]
#
# Environment:
#   VISUALNER_MODEL_DIR   destination directory
#                         (default: $HOME/.local/share/visualner/models)
#   UDPIPE_MODEL_BASE     optional direct-download base URL (mirror/offline);
#                         when set, the DSpace API is not queried
#   UDPIPE_MODEL_API      DSpace REST base
#                         (default: https://lindat.mff.cuni.cz/repository/server/api)
#   UDPIPE_HANDLE         LINDAT handle of the 2.5 model collection
#                         (default: 11234/1-3131)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROFILES="$(cd "$HERE/../.." && pwd)/lexicon/language_profiles.json"
DEST="${VISUALNER_MODEL_DIR:-$HOME/.local/share/visualner/models}"
BASE="${UDPIPE_MODEL_BASE:-}"
API="${UDPIPE_MODEL_API:-https://lindat.mff.cuni.cz/repository/server/api}"
HANDLE="${UDPIPE_HANDLE:-11234/1-3131}"
MIN_MODEL_BYTES=$((1024 * 1024))

die() { echo "$*" >&2; exit 1; }

# A UDPipe model is a multi-megabyte morphodita archive. The DSpace UI answers
# with a small HTML/JSON document; treat anything that is not a binary blob of
# plausible size as invalid, so a bad cache is repaired instead of trusted.
model_looks_valid() {
  local file=$1
  [ -s "$file" ] || return 1
  [ "$(wc -c < "$file")" -ge "$MIN_MODEL_BYTES" ] || return 1
  if head -c 512 "$file" | LC_ALL=C grep -aqE '<!DOCTYPE|<html|\{"'; then
    return 1
  fi
  return 0
}

command -v python3 >/dev/null 2>&1 || die "python3 is required to read $PROFILES"
command -v curl >/dev/null 2>&1 || die "curl is required to download the UDPipe models"
[ -f "$PROFILES" ] || die "language profile table not found: $PROFILES"

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

[ "${#MODELS[@]}" -gt 0 ] || die "no UDPipe model matches the requested languages"

# Offline/no-op path: every requested model is already a plausible local
# binary. Skip the repository lookup (the old installer did the same) while
# still rejecting the HTML error page that used to be cached as a model.
all_present=1
for model in "${MODELS[@]}"; do
  if ! model_looks_valid "$DEST/$model"; then
    all_present=0
    break
  fi
done
if [ "$all_present" -eq 1 ]; then
  echo "UDPipe models ready in $DEST (${#MODELS[@]} declared, local files valid)"
  exit 0
fi

resolve_bitstreams() {
  local item bundle
  item="$(curl -fsSL --retry 3 --retry-delay 2 \
    "$API/discover/search/objects?query=handle:%22${HANDLE}%22" \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["_embedded"]["searchResult"]["_embedded"]["objects"][0]["_embedded"]["indexableObject"]["id"])')" \
    || die "cannot resolve the UDPipe model item for handle $HANDLE"
  bundle="$(curl -fsSL --retry 3 --retry-delay 2 "$API/core/items/$item/bundles" \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next(b["uuid"] for b in d["_embedded"]["bundles"] if b["name"] == "ORIGINAL"))')" \
    || die "cannot resolve the ORIGINAL bundle for handle $HANDLE"
  curl -fsSL --retry 3 --retry-delay 2 "$API/core/bundles/$bundle/bitstreams?size=500"
}

download_verified() {
  local model=$1 url=$2 expected_size=$3 expected_md5=$4
  local tmp="$DEST/$model.part" actual_size
  rm -f "$tmp"
  echo "download $model"
  curl -fsSL --retry 3 --retry-delay 2 -o "$tmp" "$url" \
    || { rm -f "$tmp"; die "download failed: $model"; }
  actual_size="$(wc -c < "$tmp")"
  [ "$actual_size" = "$expected_size" ] \
    || { rm -f "$tmp"; die "size mismatch for $model: got $actual_size, expected $expected_size (repository returned an error page?)"; }
  if [ -n "$expected_md5" ]; then
    command -v md5sum >/dev/null 2>&1 || die "md5sum is required to verify $model"
    printf '%s  %s\n' "$expected_md5" "$tmp" | md5sum -c - >/dev/null \
      || { rm -f "$tmp"; die "MD5 mismatch for $model"; }
  fi
  model_looks_valid "$tmp" \
    || { rm -f "$tmp"; die "downloaded file is not a UDPipe model: $model"; }
  mv "$tmp" "$DEST/$model"
}

if [ -n "$BASE" ]; then
  # Mirror/offline mode: no metadata, only the local content sanity check.
  for model in "${MODELS[@]}"; do
    if model_looks_valid "$DEST/$model"; then
      echo "present  $model"
      continue
    fi
    echo "download $model (direct base)"
    tmp="$DEST/$model.part"
    rm -f "$tmp"
    curl -fsSL --retry 3 --retry-delay 2 -o "$tmp" "${BASE%/}/$model" \
      || { rm -f "$tmp"; die "download failed: $model"; }
    model_looks_valid "$tmp" \
      || { rm -f "$tmp"; die "downloaded file is not a UDPipe model: $model"; }
    mv "$tmp" "$DEST/$model"
  done
else
  bitstreams="$(mktemp)"
  trap 'rm -f "$bitstreams"' EXIT
  resolve_bitstreams > "$bitstreams"
  metadata="$(python3 - "$bitstreams" "${MODELS[@]}" <<'PY'
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))["_embedded"]["bitstreams"]
by_name = {item["name"]: item for item in data}
for name in sys.argv[2:]:
    item = by_name.get(name)
    if item is None:
        print("\t".join(["MISSING", name, "", ""]))
        continue
    checksum = item.get("checkSum") or {}
    print("\t".join([
        name,
        item["uuid"],
        str(item["sizeBytes"]),
        checksum.get("value", ""),
    ]))
PY
)"

  while IFS=$'\t' read -r model uuid size md5; do
    [ -n "$model" ] || continue
    if [ "$uuid" = "MISSING" ]; then
      die "model not published under handle $HANDLE: $model"
    fi
    target="$DEST/$model"
    if model_looks_valid "$target" && [ "$(wc -c < "$target")" = "$size" ]; then
      echo "present  $model"
      continue
    fi
    download_verified "$model" "$API/core/bitstreams/$uuid/content" "$size" "$md5"
  done <<< "$metadata"
fi

missing=0
for model in "${MODELS[@]}"; do
  if ! model_looks_valid "$DEST/$model"; then
    echo "missing or invalid $model" >&2
    missing=1
  fi
done
[ "$missing" -eq 0 ] || exit 1
echo "UDPipe models ready in $DEST (${#MODELS[@]} declared)"

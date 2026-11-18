#!/usr/bin/env bash
# Publish the prepared BrainRust v0.3.7 release.
#
#   ./scripts/release/publish-v0.3.7.sh                # dry run (default)
#   ./scripts/release/publish-v0.3.7.sh --publish      # push the tag; the
#                                                      # release workflow builds
#                                                      # and attaches dist/*
#   ./scripts/release/publish-v0.3.7.sh --publish-local # push the tag, then
#                                                      # attach the local
#                                                      # dist-v0.3.7 bundle
#
# The dry run proves the tag/manifest/checksum chain offline. `--publish`
# needs an authenticated `gh` host; without credentials nothing is pushed.
#
# Do NOT attach dist-icu72/*: it carries the same basenames as
# dist-v0.3.7/*, so `gh release create` would fail on duplicate assets. The
# v0.3.7 assets are the eight files of dist-v0.3.7/.
set -euo pipefail

VERSION="${BRAINRUST_VERSION:-v0.3.7}"
EXPECTED_SHA="${BRAINRUST_COMMIT:-2ca360344f51dc3d941c853baf8d3dc3ac1811af}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIST="$ROOT/dist-$VERSION"
NOTES="$ROOT/docs/releases/$VERSION.md"
MODE=dry-run

case "${1:-}" in
  --publish) MODE=publish ;;
  --publish-local) MODE=publish-local ;;
  "") ;;
  *) echo "usage: $0 [--publish|--publish-local]" >&2; exit 2 ;;
esac

fail() { echo "error: $*" >&2; exit 1; }

git -C "$ROOT" rev-parse -q --verify "refs/tags/$VERSION" >/dev/null \
  || fail "tag $VERSION does not exist locally"
tag_sha="$(git -C "$ROOT" rev-list -n1 "$VERSION")"
[ "$tag_sha" = "$EXPECTED_SHA" ] \
  || fail "tag $VERSION points at $tag_sha, expected $EXPECTED_SHA"
echo "tag:      $VERSION -> $tag_sha"

[ -f "$DIST/BRAINRUST_RELEASE.json" ] \
  || fail "missing $DIST/BRAINRUST_RELEASE.json"
python3 - "$DIST/BRAINRUST_RELEASE.json" "$VERSION" "$EXPECTED_SHA" <<'PY'
import json, sys
manifest = json.load(open(sys.argv[1]))
version, sha = sys.argv[2], sys.argv[3]
if manifest.get("version") != version:
    raise SystemExit(f"manifest version {manifest.get('version')!r} != {version}")
if manifest.get("git_sha") != sha:
    raise SystemExit(f"manifest git_sha {manifest.get('git_sha')!r} != {sha}")
if manifest.get("icu_major") != 72:
    raise SystemExit(f"manifest icu_major {manifest.get('icu_major')!r} != 72")
print(f"manifest: {version} {sha[:7]} icu {manifest['icu_major']}")
PY

( cd "$DIST" && sha256sum -c sha256sums.txt > /dev/null ) \
  || fail "asset checksums in $DIST do not match sha256sums.txt"
echo "checksums: ok"

shopt -s nullglob
assets=("$DIST"/*)
[ "${#assets[@]}" -eq 8 ] \
  || fail "expected 8 release assets in $DIST, found ${#assets[@]}"
[ -f "$NOTES" ] || fail "missing release notes $NOTES"
echo "assets:"
for f in "${assets[@]}"; do printf '  %s\n' "$(basename "$f")"; done

if [ "$MODE" = dry-run ]; then
  echo "dry run: nothing pushed. Re-run with --publish when credentials exist."
  exit 0
fi

if git -C "$ROOT" ls-remote --exit-code --tags origin "$VERSION" > /dev/null 2>&1; then
  fail "origin already has tag $VERSION; refusing to rewrite it"
fi
command -v gh > /dev/null || fail "gh is not installed"
gh auth status > /dev/null 2>&1 || fail "no authenticated gh host (gh auth login)"

git -C "$ROOT" push origin "$VERSION"
if [ "$MODE" = publish ]; then
  echo "tag pushed: the Release artifacts workflow builds and attaches dist/*."
  echo "watch:  gh run list --workflow=release.yml --limit 1"
  exit 0
fi

if gh release view "$VERSION" > /dev/null 2>&1; then
  fail "release $VERSION already exists; the workflow owns it"
fi
gh release create "$VERSION" --title "$VERSION" --notes-file "$NOTES" "${assets[@]}"
echo "release created from the local bundle: $(gh release view "$VERSION" --json url -q .url)"

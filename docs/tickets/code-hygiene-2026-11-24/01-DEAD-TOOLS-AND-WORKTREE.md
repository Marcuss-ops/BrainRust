# BR-HYG-01 — Dead `tools/`, imagecatalog residue and abandoned worktree artifacts

- **Status:** PARTIAL (2026-11-24) — `tools/`, `dist-icu72/`, `dist-v0.3.7/`,
  `.tmp-build/` and `.vn-release-build.log` are gone. `target/` is still present
  because two other profiles are actively compiling in it; deleting it now would
  break their work. It is git-ignored and will be removed in closeout when no
  cargo process is active.
- **Priority:** P2
- **Owner:** BrainRust workspace
- **Evidence date:** 2026-11-24

## Problem

The `imagecatalog` crate was retired on 2026-11-18 (TICKET-28), but its
offline exporters and ignore rules are still tracked:

| Artifact | Size / state | Why it is dead |
| --- | --- | --- |
| `tools/export_image_catalog.py` | 15 KB, tracked | Only consumer was the retired `imagecatalog` crate |
| `tools/export_media_annotations.py` | 6 KB, tracked | Same; documented as an `export_image_catalog.py` companion |
| `.gitignore:21-23` | tracked | `/imagecatalog/target/` and `/imagecatalog/Cargo.lock` for a directory that no longer exists |
| `dist-icu72/` | 47 MB, ignored, `root:root` | Release-only bundle; contains the retired `imagecatalog` binary |
| `dist-v0.3.7/` | 42 MB, ignored | Historical release drop, not a source input |
| `.tmp-build/` | empty, untracked | Build scratch |
| `.vn-release-build.log` | 0 bytes, ignored | Empty log |
| `target/` | 17 GB, ignored | Build cache; safe to remove after the final gate |

## Acceptance criteria

- [ ] `git ls-files tools` is empty and `tools/` is gone.
- [ ] `.gitignore` has no `imagecatalog` rules.
- [ ] `dist-icu72/`, `dist-v0.3.7/`, `.tmp-build/` and
      `.vn-release-build.log` are gone.
- [ ] No source file references those paths; historical docs may keep the
      release links.
- [ ] `target/` is removed only after the final test/clippy gate, so the gate
      still runs against a real build.
- [ ] `git status --short` contains only the intended deletions/modifications;
      the 14 files from the concurrent workstream are untouched.

## Checks

```sh
cd BrainRust
git ls-files tools
test ! -e tools
test ! -e dist-icu72
test ! -e dist-v0.3.7
test ! -e .tmp-build
test ! -e .vn-release-build.log
rg -n "imagecatalog" .gitignore Cargo.toml README.md --glob '!target/**'
```

`dist-v0.3.7` is referenced by historical release docs and
`scripts/release/publish-v0.3.7.sh`; those are documentation of a past release,
not a runtime dependency. The ticket removes the local bundle, not the release
notes or the publish script.

## Evidence (2026-11-24)

```text
$ git rm -r tools
rm 'tools/export_image_catalog.py'
rm 'tools/export_media_annotations.py'
$ for p in tools dist-icu72 dist-v0.3.7 .tmp-build .vn-release-build.log; do ...
GONE tools
GONE dist-icu72
GONE dist-v0.3.7
GONE .tmp-build
GONE .vn-release-build.log
```

`.gitignore` no longer contains `/imagecatalog/target/` or
`/imagecatalog/Cargo.lock`; it now ignores `/.tmp-build/`.

## Out of scope

- Deleting historical release documentation.
- Rewriting `scripts/release/publish-v0.3.7.sh`.
- Touching `target/` before the final workspace gate.

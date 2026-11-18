# TICKET-27 — Publish v0.3.7: the tag and the assets exist, the release does not

- Priority: P0
- Status: BLOCKED (2026-11-18) — no GitHub credentials in this workspace. Assets re-verified on 2026-11-18 (checksums, manifest, tag chain) and `scripts/release/publish-v0.3.7.sh` is ready: one command publishes once credentials exist.
- Owner: BrainRust release
- Source: audit 2026-11-18; `docs/releases/v0.3.7.md`; `docs/stato/brainrust.md`
- Blocks: `refactored` cutover (`TICKET-BRAINRUST-V0.3.7-CUTOVER-2026-11-18`)
  and TICKET-22 (`DONE` only after the cutover)

## Problem

The release candidate is complete in the working copy but not published:

- `v0.3.7` is a **local tag** at `2ca360344f51dc3d941c853baf8d3dc3ac1811af`
  (`chore(release): prepare v0.3.7 candidate`, 2026-11-17). It has never been
  pushed, so no CI run and no GitHub Release exist for it.
- `dist-v0.3.7/` (35 MB) contains the four binaries and `BRAINRUST_RELEASE.json`
  with `version: v0.3.7` and `git_sha: 2ca3603…`; `dist-icu72/` (47 MB) holds
  the ICU 72 runtime. The eight release assets documented in the notes are
  therefore buildable but not uploaded.
- `HEAD` is `57b47b2` (`v0.3.7-2`). The two commits after the tag are
  documentation/tooling only: `94eddd5` (provider precision harness) and
  `57b47b2` (long-script review packet). They change no engine behaviour and
  do not invalidate the binaries, but they are **not** covered by the tag.
- `refactored` now pins `v0.3.7` (`make/build.mk:44`). The installed
  `bin/BRAINRUST_RELEASE.json` is a local, untracked artifact that records
  `version: v0.3.7`, `git_sha: 2ca3603…`, and all four SHA-256 digests match
  `dist-v0.3.7/sha256sums.txt` after
  `make rust-fetch RUST_ARTIFACTS_DIR=…/BrainRust/dist-v0.3.7`. The `v0.3.6`
  install (`git_sha: 527b756`) is replaced, so the installed worker serves
  `quality_gate`.

The only operation that production needs from the tagged commit is
`quality_gate` (introduced by `d3b77d6`). The Go gate is deleted and the tree
pins `v0.3.7`; what remains is publishing the tag so a fresh checkout can fetch
the same binaries without the local `RUST_ARTIFACTS_DIR` bundle.

## Decision to record

Choose one and write it here:

1. **Publish `2ca3603` as-is** (recommended): the two later commits are
   docs/tools, the assets are already built from this commit, and the tag has
   never been pushed, so nothing is rewritten.
2. **Re-tag `v0.3.7` at `57b47b2`**: only if the review packet and harness are
   meant to ship with the tag. This requires rebuilding `dist-v0.3.7/` and
   re-running the release gates first.

**Decisione registrata (2026-11-18): opzione 1 — si pubblica `2ca3603` così
com'è.** `94eddd5` e `57b47b2` sono docs/tooling e non cambiano l'engine; il
tag non è mai stato pushato, quindi non si riscrive nulla e i binari in
`dist-v0.3.7/` restano validi (manifest `git_sha` = `2ca3603…`, checksum
verificati).

## Acceptance criteria

- [ ] The chosen commit is recorded in this ticket; if it is not `2ca3603`,
      `dist-v0.3.7/` is rebuilt and its manifest matches the tag.
- [ ] `git push origin v0.3.7` completes and the tag points at the chosen SHA
      on the remote.
- [ ] The release workflow runs on that SHA and is green: `cargo fmt --all --
      --check`, `cargo clippy --workspace --all-targets --locked -- -D
      warnings`, `cargo test --workspace --locked`, and the release-mode
      contract tests listed in `docs/releases/v0.3.7.md`.
- [ ] The eight ICU 72 assets are attached to the GitHub Release; the four
      `sha256sums.txt` entries match the uploaded files.
- [ ] A clean checkout can run `make rust-fetch` for `v0.3.7` and gets the
      documented `git_sha`, without local `dist-*` directories.
- [ ] `docs/releases/v0.3.7.md` changes `Status: release candidate prepared
      locally — NOT published` to the published status with the date and the
      release URL.

## Verification

```sh
git rev-list -n1 v0.3.7
git ls-remote --tags origin v0.3.7
gh release view v0.3.7 --json tagName,assets,url
(cd dist-v0.3.7 && sha256sum -c sha256sums.txt)
```

## Out of scope

- The `refactored` pin bump and the quality-gate default flip: those belong to
  `TICKET-BRAINRUST-V0.3.7-CUTOVER-2026-11-18`.
- TICKET-22 acceptance: it closes in `refactored` after the cutover, not here.

## Blocker — 2026-11-18

`gh auth status` reports no logged-in host and `git ls-remote --tags origin
v0.3.7` is empty: the tag is local only. Publishing needs an authenticated
operator and one command:

```sh
cd BrainRust
./scripts/release/publish-v0.3.7.sh            # dry run: tag + manifest + checksums
./scripts/release/publish-v0.3.7.sh --publish  # push the tag; release.yml builds and attaches dist/*
```

`--publish-local` exists as a fallback (create the release from the local
`dist-v0.3.7/` bundle instead of waiting for CI). The script deliberately does
**not** upload `dist-icu72/*`: it carries the same basenames as
`dist-v0.3.7/*` (the eight release assets), so passing both makes `gh` fail on
duplicate asset names. The earlier version of this runbook did exactly that.

Verified offline on 2026-11-18: local tag `v0.3.7` → `2ca3603…`, manifest
`version: v0.3.7` / `git_sha: 2ca3603…` / `icu_major: 72`, `sha256sum -c`
passes on all eight assets, release notes present. The script's dry run
(`./scripts/release/publish-v0.3.7.sh`, default mode) exits 0 and reports
`manifest: v0.3.7 2ca3603 icu 72`, `checksums: ok` and the eight asset names.
No push or release was attempted without credentials; the ticket stays BLOCKED
until an authenticated operator runs `--publish`.

# TICKET-27 — Publish v0.3.7: the tag and the assets exist, the release does not

- Priority: P0
- Status: CLOSED (2026-11-19) — `v0.3.8` è pubblicata con manifest, binari e checksum coerenti; `refactored` pinna la release e `make rust-fetch` l'ha installata.
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

## Recheck 2026-11-19 — provenienza rotta, credenziali presenti

Il 2026-11-19 il working tree è stato ricontrollato dopo che una sessione
concorrente ha ricostruito i binari da HEAD:

- `dist-v0.3.7/BRAINRUST_RELEASE.json` (mtime 08:06) dichiara
  `git_sha 8dda410…` mantenendo `version v0.3.7`; `visualner` è una build HEAD
  (`a0c79fcd…`), non l'artefatto del tag (`cc0089b4…`).
- `refactored/bin/semantic-engine` è una build HEAD (`8db3932e…`) contro il
  manifest installato (`0e1d10ab…`); `refactored/bin/visualner` è `a0c79fcd…`
  contro `cc0089b4…`. Solo `mediasampler` e `pipelinegen-muscles` corrispondono.
- Il `visualner` originale del tag non è presente in nessuna copia su disco.
- `bash scripts/release/publish-v0.3.7.sh` (dry run) esce **1**:
  `manifest git_sha '8dda410…' != 2ca360344f51dc3d941c853baf8d3dc3ac1811af`.
- `git push --dry-run origin refs/tags/v0.3.7` riesce (`* [new tag]`), quindi
  l'host ha credenziali git valide anche se `gh auth status` non è autenticato.

Pubblicare in questo stato significherebbe o allegare una build HEAD etichettata
`v0.3.7` (provenienza falsa), o pushare il tag con un bundle che non passa il
proprio controllo. Prima di `--publish` va scelta una delle due strade, con il
tree fermo:

1. ricostruire i quattro binari dal tag `2ca3603` e rigenerare
   `BRAINRUST_RELEASE.json` + `sha256sums.txt`; oppure
2. tagliare `v0.3.8` da HEAD (include il segnale lingua di `local_score.rs` e i
   fix grouping/chapter-bullet), pubblicarla e ripinare `BRAINRUST_VERSION`.

L'opzione 2 è quella che serve davvero alla migrazione: HEAD è già avanti
rispetto al tag e una release nuova attiva il segnale lingua. In entrambi i casi
il push del tag è l'ultimo passo, non il primo.

### Aggiornamento 2026-11-19 (planner consolidato; stato prima della pubblicazione)

Il commit `359d29e` (`feat: strengthen multilingual overlay planning`, 2026-11-19) ha consolidato su `HEAD` le modifiche di `TICKET-29`, `TICKET-31`, `TICKET-37`, `TICKET-38`, `TICKET-39`, `lexicon`, `visualner` e gran parte di `overlay-compiler`. Il planner residuo e il fix Clippy sono stati consolidati nel commit `7012c62` (`fix: clear overlay planner release gates`); `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` e `VISUALNER_MODEL_DIR=$PWD/.tmp/udpipe-models cargo test --workspace --tests --locked` passano. Il working tree BrainRust è pulito. Per chiudere il blocco release:

1. Tagliare `v0.3.8` sul commit pulito, rigenerare `dist-v0.3.8/` con `BRAINRUST_RELEASE.json` e `sha256sums.txt` coerenti con il tag, e pubblicare la GitHub Release (`gh auth login` + push del tag / `release.yml`).
2. Aggiornare `BRAINRUST_VERSION ?= v0.3.8` in `refactored/make/build.mk` ed eseguire `make rust-fetch` per riallineare `refactored/bin/BRAINRUST_RELEASE.json` ai binari installati.

### Chiusura 2026-11-19

Il tag annotato `v0.3.8` punta al commit `a4ab4cb65af0fffd93966f27738ae58626fce2a9`. La [GitHub Release v0.3.8](https://github.com/Marcuss-ops/BrainRust/releases/tag/v0.3.8) è stata pubblicata dopo il successo dei gate completi: fmt, Clippy, test workspace, semantic performance/RSS, contratto `mediaexec.v2` e build. Gli otto asset includono `BRAINRUST_RELEASE.json` e `sha256sums.txt`; il manifest dichiara lo stesso SHA, `mediaexec.v2` e ICU 72. In `refactored`, `BRAINRUST_VERSION` è `v0.3.8` e `make rust-fetch` ha verificato e installato i quattro binari.

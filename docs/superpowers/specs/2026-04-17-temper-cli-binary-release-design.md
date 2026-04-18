# Temper CLI Binary Release — Design

**Date:** 2026-04-17
**Task:** `2026-04-17-temper-cli-binary-release`
**Mode / effort:** build / medium
**Status:** Design — pending user review

## Goal

Ship prebuilt `temper` CLI binaries for macOS (Apple Silicon), Linux x86_64, and Windows x86_64 as GitHub Release artifacts, with a `curl | sh` (and PowerShell equivalent) installer so users can install without a Rust toolchain. No crates.io, npm, PyPI, or other registry publishing in scope.

## Non-goals

- Publishing to crates.io or any package registry.
- Intel Mac (`x86_64-apple-darwin`) support. Apple Silicon only. Intel Mac users can still build from source.
- Linux arm64 support. Deferred until a concrete user request; requires cross-compile toolchain and a second onnxruntime bundle source.
- Windows ARM64 support.
- Code-signing binaries (cert cost + review burden). Documented as a follow-up.
- Auto-release on every merge to main. Releases are intentional, PR-reviewed events.
- Conventional-commit auto-versioning. Human picks bump level during `release-prepare`.

## Platform matrix

| Target triple | Runner | Archive format | ONNX Runtime source |
|---|---|---|---|
| `aarch64-apple-darwin` | `macos-14` (native) | `.tar.gz` | `onnxruntime-osx-arm64-<ver>.tgz` |
| `x86_64-unknown-linux-gnu` | `ubuntu-22.04` (native) | `.tar.gz` | `onnxruntime-linux-x64-<ver>.tgz` |
| `x86_64-pc-windows-msvc` | `windows-2022` (native) | `.zip` | `onnxruntime-win-x64-<ver>.zip` |

ONNX Runtime version pinned by the release workflow — tracks the version already checked in at `crates/temper-ingest/lib/x86_64-unknown-linux-gnu/libonnxruntime.so` (currently 1.24.2). A single constant in the workflow.

**Windows stance for v1:** Included but labeled "experimental" in the README. If the implementation run uncovers a blocker we can't resolve in-session (rustls issue, DLL search weirdness, PowerShell SmartScreen quirks that can't be worked around without cert signing), Windows is cut from v1 and a follow-up task `windows-cli-binary-build-bring-up` is created with a full writeup of what we learned. Pete's two Windows-using collaborators can pick it up from there.

## Archive contents

Each archive has a flat layout (no versioned wrapper directory — the archive filename carries the version):

```
temper[.exe]                      # the CLI binary
lib/libonnxruntime.{dylib,so}     # bundled ORT (mac/linux)
onnxruntime.dll                   # bundled ORT (windows, flat next to exe)
LICENSE
README-INSTALL.txt                # minimal install notes, points at docs
```

On mac/linux, the installer extracts this layout directly into `~/.local/share/temper/`, yielding `~/.local/share/temper/temper` and `~/.local/share/temper/lib/libonnxruntime.{dylib,so}` — the layout matched by the new `embed.rs` fallback chain entries. On Windows, `Expand-Archive` into `%LOCALAPPDATA%\Programs\temper\` gives `temper.exe` + `onnxruntime.dll` as siblings, which satisfies the standard Windows DLL search order — no env var required.

### `embed.rs` fallback chain (new step added)

Current chain in `crates/temper-ingest/src/embed.rs:87-97`:

1. `ORT_DYLIB_PATH` env var (explicit override)
2. `/opt/homebrew/lib/libonnxruntime.dylib`
3. `/usr/local/lib/libonnxruntime.dylib`
4. `/usr/lib/libonnxruntime.so`

New chain (insert two new steps after the env var, renumber):

1. `ORT_DYLIB_PATH` env var (explicit override) — unchanged
2. **New:** binary-adjacent — `<exe_dir>/lib/libonnxruntime.{dylib,so}` (mac/linux installed layout) and `<exe_dir>/onnxruntime.dll` (Windows flat layout). Resolved from `std::env::current_exe()`.
3. **New:** `~/.local/share/temper/lib/libonnxruntime.{dylib,so}` — covers the installer-placed location even when the binary is symlinked onto `PATH` (Linux/macOS only). Resolved from `dirs::data_local_dir()` (already a dependency).
4. `/opt/homebrew/lib/libonnxruntime.dylib`
5. `/usr/local/lib/libonnxruntime.dylib`
6. `/usr/lib/libonnxruntime.so`

Steps 2 and 3 are what make the bundled-runtime installer UX work. Step 2 is cheap (one filesystem stat per candidate path); Step 3 is the fallback for when users move the binary around. Both new steps only apply when the `embed` feature is compiled in.

## Version management

**Source of truth:** repo-root `VERSION` file, single scalar like `0.1.0`. Matches tasker-core convention.

`temper-cli/Cargo.toml`'s `package.version` is kept in sync by `update-version.sh` (same mechanism tasker-core uses for its core crate). Sync direction is `VERSION` → `Cargo.toml`.

## Developer release flow

```
$ cargo make release-prepare
```

This command (via `tools/cargo-make/release-tasks.toml`):

1. Calls `tools/scripts/release/detect-changes.sh` — has anything in `temper-cli/` or its workspace deps (`temper-core`, `temper-client`, `temper-ingest`, `temper-llm`) changed since the last `v*` tag? If nothing has, exit with "nothing to release" and stop.
2. Calls `tools/scripts/release/calculate-version.sh` — reads current `VERSION`, prompts with suggested bump (patch by default; user can override `patch|minor|major` or type an explicit version).
3. Creates a release branch `release/v<N.N.N>`.
4. Calls `tools/scripts/release/update-version.sh` — writes new VERSION + bumps `temper-cli/Cargo.toml` `package.version`.
5. Commits with message `release: v<N.N.N>`.
6. Pushes the branch and, if `gh` is present, opens a PR titled `release: v<N.N.N>` with a body autofilled from `git log <prev-tag>..HEAD --oneline`.

The PR is not merged by the script — it's a human review step. CI (existing `test-rust.yml`, `test-typescript.yml`, `code-quality.yml`) runs on the PR like any other. Merge = blessed release.

## Post-merge → tag → build

On merge of the `release/v*` branch into `main`, a new lightweight workflow `release-tag.yml` fires:

- Trigger: `push` to `main` where `VERSION` changed (path filter)
- Reads new `VERSION`, creates + pushes annotated tag `v<N.N.N>`
- That's it. Short and focused.

The tag push triggers the main `release.yml` workflow.

## `release.yml` (adapted from tasker-core)

Trigger: `push` on tags `v*`, plus `workflow_dispatch` for manual invocation.

Job DAG (stripped down from tasker-core's 8-job version):

```
pre-flight-check          (fmt, clippy, release build smoke)
         |
build-cli-binaries        (3 matrix jobs: darwin-arm64, linux-x64, windows-x64)
         |
release-summary           (creates GitHub Release, attaches tarballs/zip)
```

No `detect-and-read` stage — the tag itself is the source-of-truth version. No publish-crates / publish-ruby / publish-python / publish-typescript / publish-containers — not in scope.

**Concurrency:** `group: release`, `cancel-in-progress: false` (same as tasker-core; avoid stomping on an in-flight release).

**No dry-run override:** tasker-core's `RELEASE_DRY_RUN: 'false'` top-of-workflow guard protected against accidental registry pushes during testing. We have no registries. Dropped.

## `build-cli-binaries.yml` (new, adapted)

`workflow_call`-style reusable workflow. Inputs: `version` (string).

Matrix (3 targets listed in the platform matrix table above).

Steps per job:

1. `actions/checkout@v6`
2. `dtolnay/rust-toolchain@stable` with `targets: ${{ matrix.target.triple }}`
3. `Swatinem/rust-cache@v2` — build cache
4. Set `SQLX_OFFLINE=true`
5. `cargo build --release --package temper-cli --target ${{ matrix.target.triple }} --features embed,extract,hnsw` — same feature set as `[features] default` in `temper-cli/Cargo.toml`
6. **Download ONNX Runtime** for this target (URL built from a pinned version constant). Verify SHA256 against a checksum committed to the repo at `scripts/release/onnxruntime-checksums.txt`. Extract the runtime library file(s).
7. **Assemble archive staging dir**:
   - `temper-v<ver>-<target>/temper[.exe]`
   - `temper-v<ver>-<target>/lib/libonnxruntime.{dylib,so}` (mac/linux) OR `temper-v<ver>-<target>/onnxruntime.dll` (windows)
   - `temper-v<ver>-<target>/LICENSE`
   - `temper-v<ver>-<target>/README-INSTALL.txt`
8. Create archive:
   - mac/linux: `tar -czf temper-v<ver>-<target>.tar.gz -C staging temper-v<ver>-<target>`
   - windows: `Compress-Archive` → `temper-v<ver>-<target>.zip`
9. Compute SHA256 of archive → `temper-v<ver>-<target>.tar.gz.sha256` (or `.zip.sha256`). Installer uses these for integrity check.
10. `actions/upload-artifact@v7` — upload archive + checksum

## `release-summary` job

Single job, runs after `build-cli-binaries` (needs: it). Runs even if build jobs partially fail so we get visibility.

Steps:

1. Download all cli-binary artifacts (archives + `.sha256` files)
2. If all three matrix jobs succeeded:
   - `gh release create v<ver>` with autogenerated release notes (`gh release create --generate-notes`) OR, if the release was opened from a release-prepare PR, use that PR's body as notes
   - `gh release upload v<ver> temper-v<ver>-*.tar.gz temper-v<ver>-*.zip *.sha256`
3. If any matrix job failed: open a GitHub Issue titled `release v<ver> failed` with the failure details, skip the release creation. Surface in the workflow summary.

## Installer scripts

Committed under `scripts/install/install.sh` (POSIX sh, not bash) and `scripts/install/install.ps1` (PowerShell 5.1+ compatible).

Both are served to users via `raw.githubusercontent.com` pointing at the `main` branch. When we fix an installer bug, the fix is live as soon as it's merged; users don't need to re-download.

### `install.sh` (macOS + Linux)

POSIX sh for maximum portability (some containers lack bash). Workflow:

1. `set -e` and a `trap` for cleanup
2. Detect OS: `uname -s` → `darwin` or `linux`
3. Detect arch: `uname -m` → if not `arm64` on Darwin or `x86_64` on Linux, bail with a clear "no prebuilt binary, install from source" message and a link to the README
4. Query GitHub API: `curl -fsSL https://api.github.com/repos/tasker-systems/temper/releases/latest` → parse `tag_name` with sed (no jq dependency — POSIX-only)
5. Build archive URL: `https://github.com/tasker-systems/temper/releases/download/v<ver>/temper-v<ver>-<target>.tar.gz`
6. Download archive + `.sha256` to a mktemp'd dir
7. Verify SHA256 (`shasum -a 256` on Darwin, `sha256sum` on Linux)
8. Extract to `${XDG_DATA_HOME:-$HOME/.local/share}/temper/` — overwrites any previous install
9. Create `${XDG_BIN_HOME:-$HOME/.local/bin}/temper` as a symlink to the extracted binary. If `.local/bin` isn't on PATH, print a warning with the one-line fix for common shells.
10. Print version + `temper --help` hint

### `install.ps1` (Windows)

1. `$ErrorActionPreference = 'Stop'`
2. Verify PowerShell 5.1+ (`$PSVersionTable.PSVersion.Major -ge 5`)
3. Check architecture — `$env:PROCESSOR_ARCHITECTURE -eq 'AMD64'`. Bail otherwise with install-from-source message.
4. Query `https://api.github.com/repos/tasker-systems/temper/releases/latest` via `Invoke-RestMethod`
5. Download archive + `.sha256` to `$env:TEMP`
6. Verify SHA256 via `Get-FileHash`
7. Extract with `Expand-Archive` to `$env:LOCALAPPDATA\Programs\temper\`
8. Append install dir to user PATH (`[Environment]::SetEnvironmentVariable('Path', ..., 'User')`) if not already present. Print warning about needing to restart the shell.
9. Print version + `temper --help` hint

### Installer README doc

`docs/guides/install.md` explains what the installer does, lists the one-liners for all platforms, gives explicit "don't want to pipe curl to sh?" alternatives (download-and-inspect-then-run), and documents the uninstall story (`rm -rf ~/.local/share/temper ~/.local/bin/temper` on \*nix; `Remove-Item` snippet for Windows).

The README.md at repo root gets a new "Install" section pointing at `docs/guides/install.md`, replacing the current `cargo install --path crates/temper-cli` instructions (which stay documented in the install doc as the "build from source" option).

## cargo-make integration

New file `tools/cargo-make/release-tasks.toml` with these tasks:

| Task | Description |
|---|---|
| `release-prepare` | Top-level entrypoint — detect changes, calc version, open release branch+PR |
| `release-check` | Dry-run detect-changes, print what would happen, no mutations |
| `release-version-bump` | Wrapped `update-version.sh` for manual use |

`tools/cargo-make/main.toml` gets extended to load `release-tasks.toml` (matches the pattern currently used for `base-tasks.toml`).

## Supporting scripts

Adapted from tasker-core into `tools/scripts/release/`:

| Script | Source | Changes |
|---|---|---|
| `lib/common.sh` | tasker-core | Copy verbatim |
| `read-version.sh` | `read-versions.sh` | Single version only — drops ruby/python/ts reads |
| `detect-changes.sh` | same | Drops ruby/python/ts/container detection. Detects changes in `temper-cli/` or any of its workspace deps (`temper-core`, `temper-client`, `temper-ingest`, `temper-llm`) since last `v*` tag. |
| `calculate-version.sh` | `calculate-versions.sh` | Single-version version of the logic |
| `update-version.sh` | `update-versions.sh` | Updates `VERSION` + `temper-cli/Cargo.toml` only |
| `release-prepare.sh` | same | Reduced scope — no registry/FFI handling |

And under `.github/scripts/release/`:

| Script | Source | Changes |
|---|---|---|
| `create-github-release.sh` | tasker-core | Simplified to CLI-only artifact set |
| `generate-summary.sh` | same | Reduced to single-component summary |
| `check-failures.sh` | same | Only CLI results |
| `tag-component.sh` | same | Single `v<N.N.N>` tag, no component-prefixed tags |

## Testing strategy

- **Installer scripts:** Authored under `scripts/install/`. Manual test pass on macOS before merge. Linux + Windows tested via the first "test release" produced against a throwaway tag (`v0.0.1-test`) that we let the real pipeline build. If the installer works end-to-end against that release, merge. Automated installer CI is a follow-up (would want a matrix of docker images simulating various user environments).
- **Release scripts:** Unit-test-level bash checks are overkill for this scope. Tested by doing a dry-run release-prepare locally against main.
- **`embed.rs` fallback chain:** Add an integration-style test to `temper-ingest` that creates a temp dir with a fake `libonnxruntime.so`, points the binary-adjacent and `~/.local/share/temper/lib/` paths at it, and asserts that `init_ort()` picks it up via the new steps 2 and 3.
- **First real release:** Cut `v0.1.0` against a throwaway test branch. Verify archive contents on each platform (extract, check layout, run binary, run `temper graph build`). Only then merge the release-prepare PR for real.

## Rollout plan

1. **PR 1 — `embed.rs` fallback chain + tests.** Additive, no behavior change for existing users. Small, reviewable.
2. **PR 2 — installer scripts + docs.** `scripts/install/install.sh` and `install.ps1`. Lands before any release workflow exists; installer is non-functional until first release but docs land with it.
3. **PR 3 — cargo-make release-tasks + supporting scripts.** Local tooling — `cargo make release-check` works immediately against main.
4. **PR 4 — GitHub Actions workflows.** `build-cli-binaries.yml`, `release-tag.yml`, `release.yml`, `.github/scripts/release/*`. Test by running the workflow against a throwaway tag.
5. **PR 5 — README.md update.** Rewrite the install section, update the project elevator pitch. Standalone so it's easy to review and iterate on.
6. **Tag `v0.1.0`** as the first real release. Celebrate.

## Risks and open questions

| Risk | Mitigation |
|---|---|
| Windows `load-dynamic` ort behavior differs from docs | Validate on `windows-2022` runner during implementation. If blocker, cut Windows from v1 per the plan above. |
| SmartScreen / Defender blocks unsigned `temper.exe` | Documented in install guide. Flag code-signing as follow-up task. |
| `sqlx` offline prepare misses queries in a workspace-feature-aware build | Use `SQLX_OFFLINE=true` + committed `.sqlx/` cache per existing project convention. Release workflow fails loudly if the cache is stale. |
| ONNX Runtime release cadence outpaces the pinned version | Pinned version is a constant in the workflow — easy bump. First time ort's URL scheme changes we'll notice immediately. |
| GitHub API rate limits on the installer's `releases/latest` call | 60 unauth'd requests per hour per IP. Plenty for a per-user installer. If this ever becomes a problem, move to a CDN-fronted redirect. |
| Installer chooses wrong architecture on macOS under Rosetta | `uname -m` reports `arm64` on native, `x86_64` under Rosetta. We only ship arm64, so an Intel-Mac user running Rosetta would get a wrong-arch error. Clearly communicated in install doc. |

## Summary

Three prebuilt platforms, bundled ONNX Runtime, human-driven SemVer via `cargo make release-prepare` → release PR → merge → tag → build → GitHub Release. Installer scripts (`curl | sh` and `irm | iex`) live on `main` so fixes are instant. Heavily adapted from tasker-core's release pipeline — keeps the parts that apply (version math, change detection, release DAG structure, cargo-make wiring), drops the registry-publishing complexity that doesn't apply to a binary-only distribution.

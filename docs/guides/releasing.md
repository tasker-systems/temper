# Releasing Temper

This guide is for maintainers cutting a new `temper` CLI release. End users looking for install instructions should read [install.md](install.md) instead.

## The Release Pipeline in One Picture

```
   developer machine            main branch            tag push            releases
┌──────────────────────┐     ┌────────────────┐     ┌──────────┐     ┌────────────────┐
│ cargo make release-  │     │ release/v0.1.0 │     │  v0.1.0  │     │ GitHub Release │
│   prepare --bump X   │──▶  │     branch     │──▶  │   tag    │──▶  │ + 3 archives   │
│                      │ PR  │  (merged)      │     │          │     │ + 3 sha256     │
└──────────────────────┘     └────────────────┘     └──────────┘     └────────────────┘
                                       │                  ▲                  ▲
                                       │                  │                  │
                               release-tag.yml       release.yml        install.sh /
                              (fires on VERSION    (fires on v*          install.ps1
                                  change)           tag push)          (fetches latest)
```

Three GitHub Actions workflows coordinate the release:

1. **`release-tag.yml`** — fires when `main` receives a commit that changes `VERSION`. Reads the new version, creates and pushes an annotated `v<X.Y.Z>` tag. That's it.
2. **`release.yml`** — fires on `v*` tag push. Runs pre-flight validation (fmt, clippy, release smoke build), then calls the reusable `build-cli-binaries.yml` to produce 3 platform archives, then creates a GitHub Release with attached artifacts.
3. **`build-cli-binaries.yml`** — a reusable workflow called by `release.yml`. Builds `temper` for macOS arm64, Linux x86_64, and Windows x86_64, bundles the matching ONNX Runtime library, and uploads per-platform archives plus SHA256 checksums.

## Cutting a Release

The primary entry point is:

```sh
cargo make release-prepare
```

This:

1. Verifies preconditions — clean working tree, on `main`, up-to-date with `origin/main`, `gh` CLI present.
2. Detects whether `temper-cli` or any of its workspace deps (`temper-core`, `temper-client`, `temper-ingest`, `temper-llm`) or release/installer tooling changed since the last `v*` tag. If nothing changed, it exits cleanly — no release needed.
3. Calculates the next version based on the current `VERSION` file and the bump level (`patch` by default). Bump variants:
   ```sh
   cargo make release-prepare           # patch: 0.1.0 → 0.1.1
   cargo make release-prepare-minor     # minor: 0.1.x → 0.2.0
   cargo make release-prepare-major     # major: 0.x.y → 1.0.0
   ```
4. Prints a summary and asks for confirmation.
5. Creates a `release/v<X.Y.Z>` branch, writes the new version into `VERSION` and `crates/temper-cli/Cargo.toml`, runs `cargo check` as a sanity gate, commits, pushes, and opens a PR via `gh`.

The PR runs through normal CI (fmt, clippy, tests, etc.) like any other change. Review it, then merge.

## What happens on merge

Merging the `release/v<X.Y.Z>` PR into `main` lands a commit that modifies `VERSION`. That triggers `release-tag.yml`, which creates and pushes `v<X.Y.Z>`. That tag push triggers `release.yml`, which builds binaries and creates the GitHub Release.

Depending on CI load this typically completes within 15-25 minutes. Watch it at [github.com/tasker-systems/temper/actions](https://github.com/tasker-systems/temper/actions).

## Release Artifact Layout

Each release has three archives, each paired with a SHA256 checksum file:

| Platform | Archive | Checksum |
|---|---|---|
| macOS (Apple Silicon) | `temper-v<X.Y.Z>-aarch64-apple-darwin.tar.gz` | `...tar.gz.sha256` |
| Linux (x86_64) | `temper-v<X.Y.Z>-x86_64-unknown-linux-gnu.tar.gz` | `...tar.gz.sha256` |
| Windows (x86_64) | `temper-v<X.Y.Z>-x86_64-pc-windows-msvc.zip` | `...zip.sha256` |

Archive contents (flat layout — no versioned top-level directory):

- `temper` or `temper.exe` — the CLI binary
- `lib/libonnxruntime.dylib` or `lib/libonnxruntime.so` (mac/linux) OR `onnxruntime.dll` (Windows, flat)
- `LICENSE`
- `README-INSTALL.txt` — brief pointer at the installer

The installer scripts in [scripts/install/](../../scripts/install/) fetch the latest release via the GitHub API, download the matching archive plus checksum, verify, extract into `~/.local/share/temper/` (mac/linux) or `%LOCALAPPDATA%\Programs\temper\` (Windows), and symlink or PATH-update as appropriate.

## ONNX Runtime Versioning

The release workflow pins the bundled ONNX Runtime version via an env var at the top of `.github/workflows/build-cli-binaries.yml`:

```yaml
env:
  ONNX_RUNTIME_VERSION: '1.24.2'
```

This must match the version used by `ort` in `crates/temper-ingest/Cargo.toml` — specifically, the `api-XX` feature. When upgrading `ort`:

1. Update `ort` and its `api-XX` feature in `crates/temper-ingest/Cargo.toml`.
2. Update `ONNX_RUNTIME_VERSION` in `build-cli-binaries.yml`.
3. Replace the checked-in Linux `.so` in `crates/temper-ingest/lib/x86_64-unknown-linux-gnu/` (this is used by the Vercel `temper-api` deploy).
4. Cut a new release.

The release workflow downloads the runtime from `github.com/microsoft/onnxruntime/releases` per platform. The four per-platform archives differ in packaging (`.tgz` vs `.zip`) and library name (`libonnxruntime.{dylib,so}` vs `onnxruntime.dll`), all handled in the workflow's matrix.

## Skipping a Release

If `detect-changes.sh` finds no changes to `temper-cli`, its deps, installer scripts, release tooling, or release workflows, `release-prepare` exits cleanly with:

```
[warn] No changes to temper-cli or its deps since <base-ref> — nothing to release
```

Nothing is created — no branch, no PR, no tag. This is the intended behavior: releases track meaningful CLI changes, not merely the passage of time.

## Troubleshooting

### Pre-flight fails with "must be on main"

You're on a feature branch. Switch to main first:

```sh
git checkout main
git pull
cargo make release-prepare
```

### Pre-flight fails with "uncommitted changes"

Commit, stash, or discard your working tree changes first:

```sh
git status                         # see what's uncommitted
git stash                          # or: git commit -am "wip"
cargo make release-prepare
git stash pop                      # restore after
```

### `release.yml` fails on a single platform

The v1 pipeline uses an aggregate `needs.build-cli-binaries.result` — a failure on any single platform marks the whole release as failed and skips GitHub Release creation. To investigate:

1. Open the failed workflow run in GitHub Actions.
2. Expand the `build-cli-binaries` job for the failing platform.
3. Fix the root cause on `main` in a normal PR.
4. Either re-trigger the release via `workflow_dispatch` on the `release.yml` workflow (input: the existing tag), or delete the tag and `cargo make release-prepare` again.

### A release was created with corrupt artifacts

You can delete the release and re-trigger:

```sh
gh release delete v<X.Y.Z> --yes --cleanup-tag
git push --delete origin v<X.Y.Z>
# fix whatever broke it, then
cargo make release-prepare
```

Be careful with this on a release that's been public for any length of time — users may have already pulled the archives. Prefer cutting a new patch release unless the broken one is fresh and unannounced.

### Upgrading to a Windows ARM64 runner / adding platforms

The per-platform matrix entries in `build-cli-binaries.yml` are self-documenting. To add a new target:

1. Add a new entry to the `matrix.target` list with `name`, `runner`, `triple`, `ort_archive`, `ort_archive_ext`, `lib_name`, `lib_dest_dir`, `archive_ext`.
2. Update `install.sh` (or `install.ps1` for a Windows variant) with the new OS/arch detection branch.
3. Update `docs/guides/install.md`'s platform list.
4. Cut a release to test the new target.

## Related files

- [`docs/guides/install.md`](install.md) — user-facing install instructions
- [`docs/superpowers/specs/2026-04-17-temper-cli-binary-release-design.md`](../superpowers/specs/2026-04-17-temper-cli-binary-release-design.md) — original design doc
- [`tools/scripts/release/`](../../tools/scripts/release/) — the shell scripts driving `release-prepare`
- [`.github/workflows/release.yml`](../../.github/workflows/release.yml) — the tag-driven release workflow
- [`.github/workflows/build-cli-binaries.yml`](../../.github/workflows/build-cli-binaries.yml) — the reusable build matrix

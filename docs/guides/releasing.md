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
2. **`release.yml`** — fires on `v*` tag push (or via `workflow_call` from `release-tag.yml`, or manual `workflow_dispatch`). Calls the reusable `build-cli-binaries.yml` to produce 3 platform archives, then creates a GitHub Release with attached artifacts. There's no separate pre-flight validation step — the release PR's normal CI (fmt, clippy, tests) is the gate.
3. **`build-cli-binaries.yml`** — a reusable workflow called by `release.yml`. Builds `temper` for macOS arm64, Linux x86_64, and Windows x86_64, bundles the matching ONNX Runtime library, and uploads per-platform archives plus SHA256 checksums.

## Cutting a Release

The primary entry point is:

```sh
cargo make release-prepare
```

This:

1. Verifies preconditions — clean working tree, on `main`, up-to-date with `origin/main`, `gh` CLI present.
2. Detects whether `temper-cli` or any of its workspace deps (`temper-core`, `temper-client`, `temper-ingest`) or release/installer tooling changed since the last `v*` tag. If nothing changed, it exits cleanly — no release needed.
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

## Standing obligation: Sigstore root rotation

`temper` verifies release attestations against a Sigstore trust root **pinned
at build time and compiled into the binary** (`crates/temper-cli/trust/sigstore-public-good-trusted-root.json`,
embedded via `include_str!` in `attest.rs`) — deliberately not fetched live
over TUF at verify time. See
[docs/superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md](../superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md)
for why: the Rust TUF ecosystem is unsettled, and pinning converts an open
ecosystem problem into a closed, auditable release-engineering one — the same
`EXPECTED_MODEL_SHA256` doctrine (`crates/temper-ingest/build.rs`) applied to
the trust root itself.

**The cost of that choice is a standing release obligation, not a one-time
decision: when Sigstore rotates its trust root, cut a release promptly.** A
binary's pinned root is fixed at the moment it was built. It cannot verify an
attestation signed under a *newer* root than the one baked in — so once
Sigstore rotates, every `temper` built before that rotation will fail
`--verify --online` and `temper update`'s (mandatory, no-bypass) attestation
check against any release built *after* the rotation, until that older
`temper` is itself replaced by a build carrying the new root.

Three things bound how bad this is in practice:

- **Updates chain.** vN's pinned root verifies vN+1's attestation as long as
  no rotation lands between them; vN+1 ships whatever root was current when
  *it* was built. Only a rotation landing strictly between the version
  installed and the version being verified against actually bites.
- **Fulcio's public-good root is long-lived and rotations are rare and
  pre-announced** — this is not a weekly fire drill.
- **The failure is loud and distinguishable, never a silent downgrade.**
  `attest.rs`'s `AttestError::TrustRootUnusable` fires specifically for an
  unusable/stale pinned root, distinct from `NotOurs` (a bad signature or
  wrong identity) — the two recoveries do not overlap, and the code never
  degrades either to a warning that reads as "verified anyway."

**The escape hatch, always available:** re-running `install.sh` fetches a
fresh archive and verifies it against the archive-level SHA256 sidecar (hash
verification does not depend on the pinned attestation root at all), so a
user stuck behind a stale pinned root can always recover a working install
without waiting for `temper update`'s attestation path to catch up.

**Maintainer action:** when you learn Sigstore has rotated (or is scheduled
to), treat it the same as any other change that forces a release — update
`crates/temper-cli/trust/sigstore-public-good-trusted-root.json` from a fresh
`gh attestation trusted-root`, verify it still contains the public-good root
(not GitHub's own, no-transparency-log root — see `attest.rs`'s module docs
for how to tell them apart), and run `cargo make release-prepare` promptly.
Sitting on a rotation is what turns this bounded, well-understood cost into an
unbounded one for anyone who hasn't updated recently.

## ONNX Runtime Versioning

The release workflow pins the bundled ONNX Runtime version via an env var at the top of `.github/workflows/build-cli-binaries.yml`:

```yaml
env:
  ONNX_RUNTIME_VERSION: '1.24.2'
```

This must match the version used by `ort` in `crates/temper-ingest/Cargo.toml` — specifically, the `api-XX` feature. When upgrading `ort`:

1. Update `ort` and its `api-XX` feature in `crates/temper-ingest/Cargo.toml`.
2. Update `ONNX_RUNTIME_VERSION` in `build-cli-binaries.yml`.
3. **Recompute all three `ort_sha256` matrix values** — see the standing obligation below. This is not optional; the build fails closed without it.
4. Replace the checked-in Linux `.so` in `crates/temper-ingest/lib/x86_64-unknown-linux-gnu/` (this is used by the Vercel `temper-api` deploy).
5. Cut a new release.

The release workflow downloads the runtime from `github.com/microsoft/onnxruntime/releases` per platform. The four per-platform archives differ in packaging (`.tgz` vs `.zip`) and library name (`libonnxruntime.{dylib,so}` vs `onnxruntime.dll`), all handled in the workflow's matrix.

## Standing obligation: ONNX Runtime digest pinning

Each matrix target in `.github/workflows/build-cli-binaries.yml` carries an
`ort_sha256` beside its `ort_archive`/`ort_archive_ext`, and the "Download ONNX
Runtime" step verifies the fetched archive against it before extracting
anything. The reason is the same `EXPECTED_MODEL_SHA256` doctrine
(`crates/temper-ingest/build.rs`) applied one layer out: the native library
extracted from that archive is copied into staging, hashed into the per-file
manifest, **attested**, and then `dlopen`'d by the shipped binary. An unpinned
fetch means we faithfully sign whatever the network handed us — and every
downstream verdict, including a signature-backed `--verify --online`, comes
back `verified` over it. The model riding in the same archive was already
pinned; this closes the other half.

**The cost of that choice is a standing obligation, not a one-time decision:
bumping `ONNX_RUNTIME_VERSION` requires recomputing all three digests in the
same commit.** The pin is per-version by construction — the digests name
`onnxruntime-{osx-arm64,linux-x64,win-x64}-<version>` archives, so a version
bump invalidates every one of them.

Two things bound how bad this is in practice:

- **The failure is loud, immediate, and fails closed.** A stale digest fails
  the download step on all three runners before a single byte is extracted,
  staged, hashed, or attested. There is no path where a mismatched archive
  reaches the signing step, and no variant of this that degrades to a warning.
- **It fires only on a deliberate version bump.** The digests are stable for
  the life of an `ONNX_RUNTIME_VERSION` — Microsoft's release assets are
  immutable — so this is not maintenance that accrues on its own.

**Maintainer action:** recompute all three in the same commit that bumps
`ONNX_RUNTIME_VERSION`, and paste the real output — never a placeholder. These
are load-bearing constants.

```bash
ORT_VER=1.24.2  # the NEW version you are bumping to
for n in onnxruntime-osx-arm64:tgz onnxruntime-linux-x64:tgz onnxruntime-win-x64:zip; do
  name="${n%%:*}"; ext="${n##*:}"
  url="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VER}/${name}-${ORT_VER}.${ext}"
  printf '%-28s ' "$name"
  curl -fsSL "$url" | shasum -a 256 | awk '{print $1}'
done
```

Map each line to the matrix entry whose `ort_archive` matches the name printed
beside it. Getting that mapping wrong fails the build rather than weakening
it — every target checks its own archive against its own pin.

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
- [`docs/superpowers/specs/2026-07-29-binary-attestation-and-manifest-verification-design.md`](../superpowers/specs/2026-07-29-binary-attestation-and-manifest-verification-design.md) — per-file manifest + attestation design
- [`docs/superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md`](../superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md) — why the trust root is pinned, and which crate/root
- [`tools/scripts/release/`](../../tools/scripts/release/) — the shell scripts driving `release-prepare`
- [`.github/workflows/release.yml`](../../.github/workflows/release.yml) — the tag-driven release workflow
- [`.github/workflows/build-cli-binaries.yml`](../../.github/workflows/build-cli-binaries.yml) — the reusable build matrix

# Releasing Temper

This guide is for maintainers cutting a new `temper` CLI release. End users looking for install instructions should read [install-temper.md](../../docs/playbooks/install-temper.md) instead.

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

### The Agent Skill bundle

Alongside the three CLI archives, each release publishes **one** architecture-independent artifact:

| Artifact | Checksum |
|---|---|
| `temper-skill-v<X.Y.Z>.zip` | `...zip.sha256` |

It is the MCP packaging of the temper skill, uploaded by a user to Claude Desktop / claude.ai via
**Customize → Skills → +**. Built by the `build-skill-bundle` job from the committed
`agent-skills/temper-knowledge-base/` tree, attested like every other published artifact, and
reproducible locally with `cargo make skill-package`. See
[connect-claude-desktop.md](../../docs/playbooks/connect-claude-desktop.md) for the user-facing side.

**Two things about it are deliberate and easy to "tidy" into breakage.**

*It carries no per-file manifest.* Manifests exist so `install.sh` can verify each extracted file
before an atomic swap. Nothing installs this bundle — a human uploads it — so there is no swap to
gate, and the sha256 sidecar plus the provenance attestation are the whole integrity story.

*Its name is `temper-skill-v<ver>.zip`, not `temper-v<ver>-skill.zip`.* `create-github-release.sh`
derives the set of archives that **must** ship a manifest from the glob
`temper-v<ver>-*.{tar.gz,zip}`. The second spelling matches that glob and would fail the release for
lacking a manifest it should not have; the first misses it while still matching the upload globs
(`temper-*.zip`, `temper-*.sha256`), so the bundle rides the existing publish loop untouched. Both
halves are pinned by a case in `test-create-github-release.sh` — the failure direction is safe
(a rename fails the release loudly rather than publishing something wrong), but it fails at release
time, which is late.

## What the attestation does and does not prove

A successful attestation check establishes something precise, and it is worth
stating plainly before stating its boundary. The artifact in hand is
byte-for-byte the one `build-cli-binaries.yml` produced, running in GitHub
Actions on `main`, **triggered by the release-tag chain** (`release-tag.yml`):
signed by a Fulcio certificate whose SAN is
`https://github.com/tasker-systems/temper/.github/workflows/build-cli-binaries.yml@refs/heads/main`
(`attest.rs`'s `expected_identity` — that string *is* the property), and whose
SLSA predicate pins
`predicate.buildDefinition.externalParameters.workflow.path` to
`.github/workflows/release-tag.yml` (the chain's entry workflow — this closes
the direct-`workflow_dispatch` door on `build-cli-binaries.yml`, which would
otherwise carry the same SAN). Issued under GitHub Actions' OIDC issuer,
chained to the pinned public-good Sigstore root, and present in Rekor's
transparency log (`skip_tlog()` is never called). Both online paths check that
signature over the digest of the **exact object each one just compared** — the
manifest's for `--verify --online`, the archive's for `temper update`. That is
a real property, correctly enforced.

> **Why `refs/heads/main`, not `refs/tags/{tag}`:** the release chain is
> branch-triggered by construction. `release-tag.yml` fires on a `VERSION`-file
> push to `main`, creates and pushes the tag with `GITHUB_TOKEN`, then calls
> `release.yml` → `build-cli-binaries.yml` via `workflow_call`. A tag pushed
> with `GITHUB_TOKEN` does not trigger workflows, so `release.yml`'s own
> `on: push: tags: v*` never fires (`release-tag.yml:53-56` documents this).
> A reusable workflow called via `workflow_call` inherits the caller's
> `github.ref`, which is `refs/heads/main` throughout the chain — so the OIDC
> token's `ref` claim, and therefore the Fulcio cert SAN, carries
> `@refs/heads/main` for every release. The tag is carried by the archive
> filename and the manifest's `version` field, not by the cert SAN. The digest
> match is what binds to a specific release artifact.

**The attestation binds the builder and the chain. It never binds the source.**
Anyone with write access to this repo can push a commit to `main` whose
workflow builds a backdoor, and it will verify perfectly on every path `temper`
offers — correct signature, correct identity, correct Rekor inclusion proof,
correct digest. This is inherent to build provenance, not a defect in this
implementation: SLSA provenance attests *the build*, and a build is only ever
as trustworthy as the commit it ran against. No verification code closes this;
what closes it is process — who holds repo write, and what review the release
PR gets on its way to `main`.

Two things bound how bad this is in practice:

- **The claim is falsifiable by anyone, not just by us.** The attestation names
  the workflow file, the chain's entry workflow, and the ref (`refs/heads/main`),
  so a reader can go read exactly what that workflow did at that commit, and
  confirm the commit is one that went through review. The provenance makes the
  build *auditable*; it does not make the audit unnecessary.
- **The boundary is the same for the out-of-band check.** `gh attestation
  verify` (see [install-temper.md](../../docs/playbooks/install-temper.md))
  removes any dependence on `temper`'s own verification code and on our pinned
  root — a genuinely stronger position — but it verifies the same predicate
  over the same subject, so it inherits the same limit. Nothing in the
  ecosystem upgrades "this build" into "this source."

Two residual trusts sit outside the signature chain, and are named here rather
than left implicit:

- **The bootstrap installer is unsigned.** The
  `curl -fsSL …/main/scripts/install/install.sh | sh` line in
  [install-temper.md](../../docs/playbooks/install-temper.md#quick-install) fetches over HTTPS from
  `raw.githubusercontent.com` and is authenticated by TLS and GitHub's control
  of that host — nothing more. That matters more than it first looks, because
  it is the recovery path every trust-root failure names: both
  `AttestError::TrustRootUnusable` and
  `AttestationVerifyError::TrustRootUnusable` render *"cut a new release or
  re-run install.sh to get a binary with a current one"*, and a test in each
  module asserts that the message names `install.sh`. So the escape hatch from
  a stale pinned root is the one path with no attestation over it. It is still
  the right hatch — a hatch that depended on the thing that broke would be no
  hatch — but it is a hatch, not a chain link.
- **The signing job's action pins do not move on their own.** The three actions
  the signing job runs (`attest-build-provenance`, `checkout`,
  `upload-artifact`) are pinned by full commit SHA, because a moving tag on a
  job holding `attestations: write` is a signing oracle waiting to be
  repointed. The cost is that a pin can rot arbitrarily far behind upstream,
  including past security fixes. Dependabot **alerts** and **security
  updates** — the org-level toggles —
  do not cover workflow `uses:` pins; only **version updates** with the
  `github-actions` ecosystem do, and those run solely from a committed config.
  That config is `.github/dependabot.yml`, and it is the whole bump path: delete
  it and the pins go stale silently.
- **The pinned trust root is a hand-committed blob.** `crates/temper-cli/trust/sigstore-public-good-trusted-root.json`
  reaches the binary through `include_str!` and nothing else. There is no
  `build.rs` in `temper-cli`, no digest pin over it, and no freshness check —
  unlike the ONNX Runtime archive and the embedding model, both of which *are*
  digest-pinned. Its integrity rests on the same review that gates any other
  committed file, and on the maintainer step below fetching it from
  `gh attestation trusted-root` rather than anywhere else.

**Maintainer action:** none recurring — this section is a statement of scope,
not an obligation. Treat it as the thing to keep true: if a future change makes
a verification path sound stronger than "builder and tag," amend this section
in the same commit rather than letting the docs drift ahead of the mechanism.

## Standing obligation: Sigstore root rotation

`temper` verifies release attestations against a Sigstore trust root **pinned
at build time and compiled into the binary** (`crates/temper-cli/trust/sigstore-public-good-trusted-root.json`,
embedded via `include_str!` in `attest.rs`) — deliberately not fetched live
over TUF at verify time. See
`internal/superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md`
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
without waiting for `temper update`'s attestation path to catch up. Note what
that hatch does and does not carry: the sidecar is an integrity check fetched
from the same release URL base as the archive, and the bootstrap script itself
is unsigned — so this path recovers a working install, it does not
independently establish provenance. That is deliberate (a hatch that depended
on the thing that broke would be no hatch) and is recorded in
[What the attestation does and does not prove](#what-the-attestation-does-and-does-not-prove).

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
3. Update `docs/playbooks/install-temper.md`'s platform list.
4. Cut a release to test the new target.

## Related files

- [`docs/playbooks/install-temper.md`](../../docs/playbooks/install-temper.md) — user-facing install instructions
- `internal/superpowers/specs/2026-04-17-temper-cli-binary-release-design.md` — original design doc
- `internal/superpowers/specs/2026-07-29-binary-attestation-and-manifest-verification-design.md` — per-file manifest + attestation design
- `internal/superpowers/spikes/2026-07-29-sigstore-crate-evaluation.md` — why the trust root is pinned, and which crate/root
- [`tools/scripts/release/`](../../tools/scripts/release/) — the shell scripts driving `release-prepare`
- [`.github/workflows/release.yml`](../../.github/workflows/release.yml) — the tag-driven release workflow
- [`.github/workflows/build-cli-binaries.yml`](../../.github/workflows/build-cli-binaries.yml) — the reusable build matrix

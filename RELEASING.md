# Releasing temper

Cutting a release is an **OSS-commitment-level**, target-agnostic act: it produces
the versioned source, the cross-platform `temper` CLI binaries, and a GitHub Release.
It does **not** deploy any running site. A release is the artifact that the world and
every deployment target consume; how a release reaches a running site is a separate,
per-target concern — see [DEPLOYING.md](DEPLOYING.md).

## What a release produces

A `v*` tag invokes [`.github/workflows/release.yml`](.github/workflows/release.yml):

`determine-version` → `build-cli-binaries` (darwin-arm64 / linux-x64 / windows-x64) →
`release-summary` (publishes the GitHub Release with the CLI binaries attached).

No Vercel deploy, no schema migration, no production side effects. Releasing and
deploying are decoupled by design (see
[docs/superpowers/specs/2026-06-25-multi-target-deployment-model-design.md](docs/superpowers/specs/2026-06-25-multi-target-deployment-model-design.md)).

## Release checklist

1. **Merge to `main`.** Per-PR CI validates the change; per-target preview deploys
   (Vercel) validate it on each deployment target before it can reach that target's
   production.

2. **Bump `VERSION` on `main`.** [`release-tag.yml`](.github/workflows/release-tag.yml)
   derives and pushes the `v<VERSION>` tag, which invokes `release.yml`.

3. **Verify the GitHub Release.** The Actions run should be green and the Release
   should list the three CLI binaries. That's the whole release.

A release can also be (re-)run manually via **Actions → Release → Run workflow** with
an explicit `tag` input — useful to re-cut binaries for an existing tag.

## Deploying a release

Releasing does not ship a running site. Each deployment target (temperkb.io, an
enterprise self-hosted instance) is an independent Vercel project that consumes the
repo on its own cadence, with its own Neon DB and env. See **[DEPLOYING.md](DEPLOYING.md)**
for the per-target model, the additive-only-on-`main` invariant, and how schema
changes are applied per target.

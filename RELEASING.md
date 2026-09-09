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
`temper-artifacts:specs/2026-06-25-multi-target-deployment-model-design.md`).

## Release checklist

1. **Merge to `main`.** Per-PR CI validates the change; per-target preview deploys
   (Vercel) validate it on each deployment target before it can reach that target's
   production.

2. **Consult the register.** [RELEASE_REGISTER.md](RELEASE_REGISTER.md) is the
   release-verdict register: a release whose surface class has an open gated entry
   waits. `tools/scripts/release/calculate-versions.sh` reads it — a shape-breaking
   window refuses to compute a patch next (re-run with `--minor`; M is a decision,
   never an accident), and a blocked row refuses a client-skin release unless the
   verdict owner's decision is passed explicitly.

3. **Bump `VERSION` on `main`.** The release spine in
   [`tools/scripts/release/`](tools/scripts/release/) does the arithmetic from the
   declared classes: `detect-changes.sh` (where changed) → `calculate-versions.sh`
   (bump + register gates) → `update-versions.sh` (one writer for every version
   site) → `release-prepare.sh` (pre-flight through release PR).
   [`release-tag.yml`](.github/workflows/release-tag.yml) derives and pushes the
   `v<VERSION>` tag, which invokes `release.yml`.

4. **Verify the GitHub Release.** The Actions run should be green and the Release
   should list the three CLI binaries. That's the whole release.

A release can also be (re-)run manually via **Actions → Release → Run workflow** with
an explicit `tag` input — useful to re-cut binaries for an existing tag.

## Versioning: 0.M.P and the declared-class gate

Every wire surface shares one contract number, anchored on the repo-root `VERSION`
file. The scheme is **0.M.P** (spec of record:
`temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md`):

- **M moves only for non-additive shape-breaking changes** — deliberate, signaled,
  carrying the migration path for every impacted client. M is the break valve; it is
  always a decision, never an accident.
- **P is the floor bump for every wire-contract change**: "an old client carrying
  0.M.p₁ keeps working against a server at 0.M.p₂, in both skew directions."
- Hard semver is not claimed. 0.x is the honest statement: no stability guarantee is
  offered yet.

What a bump promises (spec §2, verbatim):

| Bump | Promise |
|---|---|
| shared 0.M.\* | every crate, package, and client at ≥ 0.M interoperates with the server at 0.M.\* — compatible-forward within the M |
| P (anywhere) | additive evolution only; no client action required to keep working |
| leaf P float | a leaf (client skin, npm package) may fix or grow additively ahead of the fleet without touching the shared number |
| M | a non-additive wire change; every client whose contract predates it owes a deliberate, signaled response (§3 stale-client rule) |

What shares the number, what releases independently (spec §3):

- **Crates never float.** All workspace crates ride `VERSION` together — if any crate
  floated P, `temper --version` would diverge from the shared number.
- **Leaf packages float P above the floor, never below it**: the client skins
  (temper-rb, temper-py, temper-ts) and the npm packages (temper-ui, temper-cloud).
  When the floor's M moves, a leaf below the new floor re-bases to it (its float P
  count restarts); a leaf already above stays.
- **One contract number for the server wire.** `openapi.json` `info.version` derives
  from `VERSION` at emit time; HTTP and MCP share the number. CLI stdout rides
  `VERSION` too — its stdout/exit contract is consumed by external parsers, so its
  compat class is declared per release like any other wire surface.
- **Skins keep their own semver** (`Temper::VERSION`, `temper.__version__`) — a
  skin's number answers the skin's question. `CONTRACT_VERSION` names the contract
  the skin carries, and **a client whose `CONTRACT_VERSION` is behind main's version
  is stale**, so releasing it is a deliberate, signaled act: the release notes name
  the contract delta it closes. Staleness between releases is the steady state, not
  a blocker; the obligation tightens only when the lag crosses an **M** boundary.

Every PR that touches a wire surface (`crates/temper-api/`, `crates/temper-mcp/`,
`crates/temper-client/`, `openapi.json`, `clients/`, `packages/`) declares its compat
class in [RELEASE_REGISTER.md](RELEASE_REGISTER.md): `additive` (floor P bump at the
next release), `shape-breaking` (M bump required; changelog, release notes, and a
client-release plan before merge), or `behavioral` (meaning behind an unchanged shape
changed — a register entry, never silent; no version bump). CI verifies row presence
and shape-diff honesty; it is structurally barred from certifying the behavioral
class — that half is owned by review ("does this change the meaning behind an
unchanged shape for any existing client?") and the merge decision.

## Deploying a release

Releasing does not ship a running site. Each deployment target (temperkb.io, an
enterprise self-hosted instance) is an independent Vercel project that consumes the
repo on its own cadence, with its own Neon DB and env. See **[DEPLOYING.md](DEPLOYING.md)**
for the per-target model, the additive-only-on-`main` invariant, and how schema
changes are applied per target.

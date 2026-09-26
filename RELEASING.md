# Releasing temper

Cutting a release is an **OSS-commitment-level**, target-agnostic act: it produces
the versioned source, the cross-platform `temper` CLI binaries, the four client
packages, and a GitHub Release. It does **not** deploy any running site. A release
is the artifact that the world and every deployment target consume; how a release
reaches a running site is a separate, per-target concern — see
[DEPLOYING.md](DEPLOYING.md).

## What a release produces

A `v*` tag invokes [`.github/workflows/release.yml`](.github/workflows/release.yml):

`determine-version` → `build-cli-binaries` (darwin-arm64 / linux-x64 / windows-x64)
· `build-skill-bundle` · `publish-npm-clients` (@tasker-systems/temper-ts,
@tasker-systems/temper-telemetry-ts → registry.npmjs.org) · `publish-ruby-client`
(temper-rb → rubygems.org) · `publish-py-client` (temperkb-py → pypi.org) →
`release-summary` (publishes the GitHub Release with the CLI binaries and skill
bundle attached; the release is created only when every producer succeeded, and
the summary table reports each lane).

No Vercel deploy, no schema migration, no production side effects. Releasing and
deploying are decoupled by design (see
`temper-artifacts:specs/2026-06-25-multi-target-deployment-model-design.md`).

## Installing the clients

Client packages publish to the **public registries** — rubygems.org for the gem,
registry.npmjs.org for the TypeScript packages, pypi.org for the Python client.
All three paths are token-free for consumers; no `read:packages` PAT, no
`.npmrc` stanza, no Bundler credentials, no direct URL.

**temper-ts / temper-telemetry-ts (npm)** — `npm install @tasker-systems/temper-ts`
(telemetry-ts likewise), or pinned to the release being cut:
`npm install @tasker-systems/temper-ts@<VERSION>` — `<VERSION>` is the repo-root
`VERSION` file's value, as in the `v<VERSION>` tag below.

**temper-rb (RubyGems)** — `gem install temper-rb`, or a plain `Gemfile` line pinned
to the release being cut:

```
gem "temper-rb", "<VERSION>"
```

**temperkb-py (PyPI)** — `pip install temperkb-py`, or pinned to the release
being cut: `pip install temperkb-py==<VERSION>`. The distribution is
`temperkb-py`; the import
package stays `temper`. PyPI has no scopes, and both natural distribution names
were taken by unrelated projects (a TEMPer USB-device reader under `temper-py`,
an HTML DSL under `temper`), so the distribution carries the kb.

(The npmjs/RubyGems examples above are token-free; the GitHub Packages copies
published before the public flip remain on their hosts but are no longer the
documented path — and no new versions land there.)

**Publishing-side auth** (maintainers): every registry lane authenticates via
OIDC trusted publishing — RubyGems through
`rubygems/configure-rubygems-credentials`, npm through `npm publish
--provenance`, and PyPI through `uv publish --trusted-publishing automatic` —
with the trusted publisher on each host registered against **`release.yml`**
(the workflow whose job performs the push — the identity claim names the job's
own workflow file, even when that workflow was called from the `release-tag.yml`
chain; a publisher registered against the entry workflow is silently
unauthorized at push: "You are not allowed to push this gem"). No registry API
key exists as a repo secret. The first publish of a NEW package name on npm cannot
be OIDC — npmjs.com only attaches trusted publishers to existing packages — so
a new name is claimed once locally (`npm login`, then `npm publish --access
public` in the package directory) and the trusted publisher is attached
immediately after. PyPI has no such gap: a pending publisher can be attached to
a not-yet-existing project, so a new name is pre-registered on pypi.org and the
first release claims it — no local bootstrap upload.

## Release checklist

1. **Merge to `main`.** Per-PR CI validates the change; per-target preview deploys
   (Vercel) validate it on each deployment target before it can reach that target's
   production.

2. **Consult the register — and roll it.** [RELEASE_REGISTER.md](RELEASE_REGISTER.md) is the
   release-verdict register: a release whose surface class has an open gated entry
   waits. `tools/scripts/release/calculate-versions.sh` reads it — a retirement-train
   row refuses to compute a patch next (re-run with `--minor`; M is the reserved era
   level, moved only by the retirement release Pete calls), and a blocked row refuses
   a client-skin release unless the verdict owner's decision is passed explicitly.
   **The release is not done until the register is rolled**: move the shipped rows
   under a `## Shipped in <V>` section and retitle the window to `## Since <V> —
   unreleased`. An unrolled window double-counts — the calculator reads the whole
   `## Since` section, so rows that already shipped inflate the next release's
   roll-up and its class arithmetic.

3. **Bump `VERSION` on `main`.** The release spine in
   [`tools/scripts/release/`](tools/scripts/release/) does the arithmetic from the
   declared classes: `detect-changes.sh` (where changed) → `calculate-versions.sh`
   (bump + register gates) → `update-versions.sh` (one writer for every version
   site: the anchor, the temperkb-* closure specs, and the floats) →
   `release-prepare.sh` (pre-flight through release PR).
   [`release-tag.yml`](.github/workflows/release-tag.yml) derives and pushes the
   `v<VERSION>` tag, which invokes `release.yml`.
   **When the bump is an M (a retirement release), the same release PR cuts the
   era's next pin**: copy the release candidate's `openapi.json` to
   `schemas/versions/<M.m>/openapi.json` with a provenance README beside it (the
   0.5 pin's README is the form). Until that lands, the previous pin stays current
   and the retirement movement correctly reads red on
   `check-openapi-pin.sh` — the pin gate is what turns green with the era release.
   The release chain also feeds the [homebrew tap](https://github.com/tasker-systems/homebrew-tap)
   automatically (the `update-homebrew-tap` job renders `temper@<M>` from the
   release's own digests; a new minor adds the formula and moves the alias).
   The tap README's inventory-table row for the new minor is the one manual
   edit — include it in the release PR.

4. **Verify the GitHub Release.** The Actions run should be green and the
   Release should list the three CLI binaries and the skill bundle; the npm,
   Ruby, Python, and crates.io publish lanes report in the run's summary
   table. That's the whole release.

   The crates.io lane publishes the `temperkb-*` client closure (see
   `tools/scripts/release/publish-crates.sh`): trusted publishers are
   configured on crates.io for all six names against this workflow, and the
   per-crate versions-API probe makes re-runs and re-cuts idempotent — a
   version already published skips loudly. Bumping the workspace anchor moves
   the closure's `[workspace.dependencies]` specs with it, so every release
   publishes the closure at the new version; the bootstrap 0.5.3 versions were
   the one token-publishing exception (crates.io attaches publishers only to
   existing crates), already spent.

A release can also be (re-)run manually via **Actions → Release → Run workflow** with
an explicit `tag` input — useful to re-cut binaries for an existing tag.

## Versioning: 0.M.P, the era level, and the declared-class gate

Every wire surface shares one contract number, anchored on the repo-root `VERSION`
file. The scheme is **0.M.P** (spec of record:
`temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md`, as amended by
the compat-deprecation regime,
`temper-artifacts/specs/2026-09-22-compat-semver-policy-amendment-design.md`):

- **M is the reserved era level** — it moves only in a retirement release Pete
  deliberately calls: announced by the deprecation records whose horizons it
  honors, removing the retired shapes, cutting the era's pin, carrying one
  changelog with the migration path for every impacted client. It is always a
  decision, never an accident. Between era releases, every release is
  additive-only against the current pin.
- **P promises additive evolution within the era**: a client built at any pin of
  the current era interoperates with every later release of that era, in both
  skew directions.
- Hard semver is not claimed. 0.x is the honest statement: no stability guarantee is
  offered yet.

What a bump promises (spec §2, as amended by D-C1):

| Bump | Promise |
|---|---|
| shared 0.M.\* | every crate, package, and client at ≥ 0.M interoperates with the server at 0.M.\* — compatible-forward within the era |
| P (anywhere) | additive evolution only; no client action required to keep working |
| leaf P float | a leaf (client skin, npm package) may fix or grow additively ahead of the fleet without touching the shared number |
| M | the era boundary — retirements executed against their deprecation records' horizons, migrations named, one changelog; every client still reading a retired shape owes the migration its record carried (§3 stale-client rule) |

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
next release), `shape-breaking` (below the era level this routes — D-C2: convert the
movement to the deprecation path and re-declare the row `additive` + `behavioral`, or
wait for the retirement release train and name `retirement-train` in the row; a routed
row requires `--minor`, the era release), or `behavioral` (meaning behind an unchanged
shape changed — a register entry, never silent; no version bump). CI verifies row presence
and shape-diff honesty; it is structurally barred from certifying the behavioral
class — that half is owned by review ("does this change the meaning behind an
unchanged shape for any existing client?") and the merge decision.

### The pinned contract (additive-only within the era)

Since 2026-09-16 each released minor also **pins** its contract:
`schemas/versions/<M.m>/openapi.json` plus a provenance README (the 0.5 pin,
cut from `v0.5.1`, is the founding one). Within the era the committed
`openapi.json` may only GROW — a client built at the pin (the deploy base's
widest adoption: enterprise fleets current as of v0.5.1) interoperates with
every later release of the era, in both skew directions.

The gate is [`check-openapi-pin.sh`](.github/scripts/check-openapi-pin.sh)
(`cargo make openapi-pin-check`, and the Guard-Tests CI step), verdict from the
same comparator the declared-class gate uses (`wire-shape-lib.jq` — one
definition). A `moved` verdict names each movement and is a **shape break**:
below the era level it has two routes (D-C2) — convert to the deprecation path
(keep serving the existing rendering under a record, land the corrected signal
additively, re-declare the register row), or wait for the retirement release
train, which moves M and cuts the next pin in its own release PR, turning the
gate green (D-C4). A retirement release is therefore expected to read red
against the current pin until its own PR cuts the new one — that red is the
discipline saying the movement cannot merge to `main` and strand the pin's
clients.

## Deploying a release

Releasing does not ship a running site. Each deployment target (temperkb.io, an
enterprise self-hosted instance) is an independent Vercel project that consumes the
repo on its own cadence, with its own Neon DB and env. See **[DEPLOYING.md](DEPLOYING.md)**
for the per-target model, the additive-only-on-`main` invariant, and how schema
changes are applied per target.

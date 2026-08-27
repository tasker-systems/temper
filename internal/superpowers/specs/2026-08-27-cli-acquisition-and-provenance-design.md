# CLI acquisition and provenance — design

**Date:** 2026-08-27
**Status:** Draft, ungrounded in two places — see *Open questions*
**Goal:** [The CLI is acquired and trusted without the source that built it](./01a04315-39c3-75e2-a7f0-78effee802c2)

## What this designs

How someone obtains the `temper` CLI, and how they establish that what they obtained is what was
published — in a way that holds regardless of whether they can read the source that built it.

The goal register states the invariants and deliberately names no mechanism. This spec picks the
mechanisms and shows the grounding for each choice.

## Grounding

Evidence first, proposal after. Every claim below was executed against `main` at `be901412`.

### G1 — the CLI's dependency closure is eight crates, and none is server-side

```
$ cargo tree -p temper-cli --prefix none --no-dedupe | grep -oE "^temper-[a-z-]+" | sort -u
temper-auth  temper-cli  temper-client  temper-core
temper-ingest  temper-principal  temper-telemetry  temper-workflow
```

Absent from that closure: `temper-api`, `temper-services`, `temper-substrate`, `temper-mcp`,
`temper-migrate`, `temper-agents`, `temper-macros` — and `migrations/`.

### G2 — the dependency direction across the proposed boundary is one-way

Reading each server-side crate's `[dependencies]`:

```
temper-api      -> auth, principal, core, workflow, telemetry  (+ services, substrate, migrate)
temper-services -> auth, core, principal, workflow, ingest     (+ macros, substrate, migrate)
temper-substrate-> core, ingest                                (+ migrate)
temper-mcp      -> core, workflow, telemetry                   (+ services, substrate)
```

The server stack consumes seven of the eight; **no crate in the closure consumes a server-side
crate.** That directionality is what makes the boundary viable, and it is the property to protect.

### G3 — a source build alone does not produce a working CLI

`crates/temper-cli/Cargo.toml` sets `default = ["embed", "extract"]`, and `embed` resolves to
`temper-ingest/embed-download`, whose own comment states the arrangement:

> Resolve the model from disk at runtime (next to the binary) instead of baking it in. […] Keeps the
> CLI binary ~18 MB instead of ~128 MB.

Without the feature, five call sites refuse rather than degrade — e.g. `actions/search.rs`:

```rust
#[cfg(not(feature = "embed"))]
pub fn embed_query(_text: &str) -> Result<Vec<f32>> {
    Err(TemperError::Config(
        "search requires the 'embed' feature — rebuild with --features embed".into(),
    ))
}
```

So a compile-from-source acquisition yields a CLI whose search does not work, and the model file is
acquired separately or not at all. **This is the constraint that decides the design**, and it is why
the goal's `an-acquisition-yields-a-working-cli` clause names "without a further acquisition step".

### G4 — the release archive already carries what a source build lacks

Published `v0.3.6` assets include per-platform archives, per-platform `manifest.json`, `.sha256`
sidecars, and the skill zip. The archive — not the binary — is the unit that works.

### G5 — the registry name `temper` is unavailable

```
$ curl -s https://crates.io/api/v1/crates/temper | ...
EXISTS: temper 0.2.0
```

Unrelated crate. The published binary crate is therefore `temper-cli`; the *binary it installs* is
still named `temper`, which `[[bin]] name = "temper"` already sets.

### G6 — every crate in the closure is publishable as-is

None carries `publish = false`. Versions today: `temper-cli` at `0.3.6`, the other seven at `0.1.0`.

## The design

### D1 — the published surface is a projection, not a fork

**CONFORM.** The private workspace remains the source of truth and the place all development
happens. A release job mirrors the eight crates (G1) into a public repository, which publishes to
the registry and cuts the signed release.

The alternative — a genuine two-repository split with the public side consumed as a versioned
dependency — is rejected rather than deferred. G2 shows the server stack consumes seven of the eight
crates, and `temper-core` carries the shared wire types, so every DTO or `ts-rs` change would become
publish-bump-rebuild. That also puts the OpenAPI→gem→`schema.ts` chain and the `ts-rs` drift gates
across a repository boundary, and those gates hold today *because* it is one workspace. The cost
lands on the most-churned crate in the tree.

The projection satisfies `a-change-that-spans-both-sides-stays-one-reviewable-act` structurally:
there is only ever one act, because there is only one source.

**What it costs, stated:** a contribution arrives against the projection and is replayed inward by
hand. That is real friction and it is the honest price of the arrangement.

### D2 — acquisition has three paths, and the archive is the unit

**EXTEND**, authorized by the goal's `what-a-consumer-may-fetch-is-not-assumed` — no single origin
may be required.

| Path | Role | Yields |
|---|---|---|
| Prebuilt archive via the existing script | The path in use today; repointed at the public projection | A working CLI (G4) |
| Binary-install from the registry | Fetches the same archive rather than compiling | A working CLI |
| Platform package manager (own tap) | Third-party trust model, for consumers whose policy prefers one | A working CLI |
| `cargo install temper-cli` | Namespace and discovery only | **A degraded CLI** (G3) |

The last row is the one to be honest about. Publishing to the registry is worth doing for the name,
for discovery, and because the binary-install path reads its metadata — but a from-source install
does not satisfy `an-acquisition-yields-a-working-cli`, and the crate's documentation should say so
rather than let a user discover it at first search.

### D3 — provenance rides on what already exists

**CONFORM.** Build-provenance attestation against a pinned trust root, with `.sha256` sidecars and
per-platform manifests, is already produced and already verified by a shipped verb. Nothing here
changes the verification model; the projection inherits it.

`provenance-is-checkable-without-trusting-us` is the clause this serves, and the existing
three-verdict distinction — verified, contradicted, unverifiable — is exactly the refusal face the
register asks for.

## Out of scope

**Rejected** — considered and declined, not deferred:

- **A true two-repository split.** Rejected per D1 on coupling cost.
- **Baking the model into the binary.** The `~18 MB vs ~128 MB` note (G3) records this as already
  decided; re-opening it is not this design's business.
- **A releases-only public repository holding no source.** It would serve acquisition while serving
  none of the contribution intent, and the marginal cost over D1 is small.

**Deferred** — wanted, not now:

- Submission to a platform package manager's central index. Notability thresholds are not met today;
  an own tap is the current answer and the central index is a later one.
- Reproducible builds. A stronger property than attestation and a separate piece of work.

## Open questions — ungrounded, and blocking

Both must be answered before D1 is committed to. Neither is answerable by reading; both need the
release path exercised.

1. **Can the release and signing job run from the projection without private build inputs?** The job
   builds the archive, which requires the model artifact and the native runtime library. Whether
   those are reachable from a tree containing only the eight crates is not established here.
2. **Does the verification verb assume the current repository identity?** Attestation binds a builder
   and a tag. If the pinned trust root or the expected subject encodes the present repository, a
   projection publishing under a different identity changes what verification asserts — and doing
   that silently would weaken the clause D3 claims to serve.

**These are the first task.** Nothing else in this design is safe to build on top of them.

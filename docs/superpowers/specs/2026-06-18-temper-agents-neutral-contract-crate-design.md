# temper-agents — neutral-contract crate (design)

**Date:** 2026-06-18
**Status:** Design — approved, pending implementation plan
**Goal:** `substrate-kernel-to-cognitive-map`, Workstream 7 (Agent surface)
**Companion research:** [Vercel Eve & Claude Managed Agents investigation](../../research/2026-06-18-vercel-eve-and-claude-managed-agents-investigation.md); vault research doc `2026-06-18-agentic-workflows-on-temper-via-vercel-eve` (the WS7 charter — search the vault)

---

## Summary

Create `crates/temper-agents` — the **neutral-contract package** for temper's agent surface (WS7 decision #6). It is a deliberately **thin layer**: it re-exports the invocation-envelope and agent-authorship contract types already shipped in `temper-next` (PR #148), and *owns* exactly one new thing — the `DeploymentProfile` policy object plus its ts-rs wire export. It replaces the dead `temper-llm` / `temper-llm-smoke` crates, which are culled. The thin MCP/HTTP client named in decision #6 is **deferred** until a real agent binding needs it.

This crate is **parallel-safe** with the in-flight WS6 chunk-5 flip: it adds a new crate and culls two dead ones, and depends on `temper-next` types additively without touching the backend/MCP cutover surface.

## Motivation

The runtime investigation (companion doc) found that Vercel Eve and Claude Managed Agents are nearly isomorphic at the deployment-config level, so the neutral binding seam is **thin**: most agent config (skills, MCP tool list, persona, HITL policy) is portable and lives in the agent definition, not in temper. What temper *must* model is (a) the accountability grain — the invocation envelope + authorship metadata, already shipped in `temper-next` — and (b) the small set of genuinely-divergent deployment knobs — the `DeploymentProfile`. `temper-agents` is the crate that gives both a single, runtime-neutral import home, decoupled from `temper-next`'s internal location so that future bindings don't break when the convergence lift relocates the envelope types.

## Ownership model (the central decision)

The envelope/authorship types already exist, in `temper-next`, and are "parity-shaped for the temper-core lift at convergence." `temper-agents` is **option B — a thin layer above the existing types**, not their owner:

- It **re-exports** the contract types from `temper-next` (their definitional home stays `temper-next` now → `temper-core` at the convergence lift).
- It **owns** only what is genuinely new and agent-specific and missing today: `DeploymentProfile`.
- Dependency direction: `temper-agents → temper-next`. Never the reverse. The convergence lift later moves the *definitions*; `temper-agents` updates its re-export path and consumers are unaffected.

## What the crate contains

### 1. Re-export surface (data types only)

Re-export the **pure data** contract types from `temper-next` (these are the typed event payloads + ids + enums; they carry `serde` derives and are `kb_events`-replayable):

| Type | Origin (`temper-next`) | Role |
|---|---|---|
| `InvocationId` | `src/ids.rs` | Envelope id newtype (UUIDv7) |
| `DelegatedLaunch` | `src/payloads.rs` | Invocation-open payload = the **delegation binding** (originating + parent cogmap, scoped entity, trigger kind) |
| `InvocationClosed` | `src/payloads.rs` | Invocation-close payload = terminal **outcome** |
| `Disposition` | `src/payloads.rs` | `Completed` / `Failed` / `Abandoned` |
| `AgentAuthorship` | `src/payloads.rs` | Attribution metadata (reasoning, confidence, provenance) |
| `ConfidenceBand` | `src/payloads.rs` | `Tentative` / `Probable` / `Confident` |

**Explicitly NOT re-exported:** `EventContext` and `fire_with` (`src/events.rs`). These are substrate-side **runtime helpers** bound to `sqlx::PgConnection` — they belong to the write path inside `temper-next`, not to a neutral contract a remote (Claude-managed) binding consumes over MCP. The contract is the *data*, not the substrate write mechanism.

The re-export is a thin `pub use temper_next::{ids::InvocationId, payloads::{...}}` surface in `temper-agents`'s `lib.rs`, behind a small module (e.g. `pub mod envelope`) so the import path reads as a contract (`temper_agents::envelope::DelegatedLaunch`).

### 2. `DeploymentProfile` (the one owned type)

The policy object the **runtime-binding layer** reads; the substrate **never** reads it (decision #3: kernel never branches on stratum). Three fields, each grounded in an observed runtime divergence (see companion doc Part 3):

```rust
/// Which agent runtime this deployment binds to. (Decision #1.)
pub enum RuntimeBinding {
    /// Vercel Eve durable agents.
    Eve,
    /// Claude Managed Agents (`/v1/agents` + `/v1/sessions`).
    ClaudeManaged,
}

/// Where tool execution runs. Orthogonal to `RuntimeBinding` — both runtimes
/// offer both (Eve: Vercel-managed vs docker/self-deploy; CMA: cloud vs self_hosted).
/// This is WS7's "stratum" made concrete.
pub enum Residency {
    /// Runtime-operator-hosted sandbox (Vercel sandbox / CMA cloud env).
    Managed,
    /// Customer-infrastructure execution (Eve docker/self-deploy / CMA self_hosted worker).
    SelfHosted,
}

/// How an agent binding is deployed and paced. Carried by the binding layer;
/// never read by the substrate.
pub struct DeploymentProfile {
    pub runtime: RuntimeBinding,
    pub residency: Residency,
    /// Token-denominated budget — neither runtime exposes a managed $ budget,
    /// so spend governance is expressed in tokens. `None` = runtime default.
    pub token_budget: Option<u32>,
}
```

Derives: `Debug` (workspace rule: all public types implement `Debug`), `Clone`, `PartialEq`, `Eq`, `serde::{Serialize, Deserialize}`, and `ts_rs::TS` behind the `typescript` feature (below). Enums serialize snake_case to match the rest of the wire model.

**Deliberately absent:** durability/checkpointing, triggering/channels, delegation mechanics, skills, MCP tool list, HITL policy. These are *convergent* across runtimes (companion doc Part 3) and therefore portable — they live in the agent definition (markdown skills + the `temper-mcp` surface), not in this profile. Decision #7's wording ("the profile absorbs the divergence") holds, but the divergence turned out to be these three fields, not the larger set the design guessed.

### 3. ts-rs wire export (owned types only)

`temper-agents` carries a `typescript` feature mirroring `temper-core`'s (`ts-rs = "10"`, optional; `typescript = ["ts-rs"]`). It derives `ts_rs::TS` on `DeploymentProfile`, `RuntimeBinding`, `Residency` and drives their generation into the repo's existing generated-TS location (matching `temper-core`'s `generate-ts-types` flow), so the TS runtimes (Eve agent config, temper-ui) share the exact type.

ts-rs derives must sit **on the type definition**, so `temper-agents` can only ts-rs-export the types it *defines* — i.e. `DeploymentProfile` and its enums. The **re-exported envelope types are NOT ts-rs-exported here**; their TS lands at the temper-core convergence lift (when their definitions move and gain `typescript`-gated derives). This is correct for now: no TypeScript runtime consumes the envelope types yet (Eve agents blocked behind the MCP-surface work; temper-cloud/temper-ui don't touch invocation types), and it keeps this crate from reaching into `temper-next` to add derives during the chunk-5 flip.

### 4. Cull `temper-llm` + `temper-llm-smoke`

Both crates have no production callers (`temper-llm-smoke` is `temper-llm`'s only dependent; it is `publish = false`). The cull:

- Delete `crates/temper-llm/` and `crates/temper-llm-smoke/`. The workspace `members` is the glob `["crates/*", "tests/e2e"]`, so removal is automatic — no `members` edit.
- `temper-llm` is **not** in root `[workspace.dependencies]` (only a direct path-dep inside `temper-llm-smoke`), so there is no workspace-dep entry to remove.
- Update `docs/guides/releasing.md:38` — remove `temper-llm` from the "workspace deps changed" detection list.
- Grep the release/installer tooling (`xtask`, release CI, install scripts) for `temper-llm` / `temper_llm` and drop any stale references — release flag sets are not exercised by regular CI (prior regression class: release-only build flags escaping CI).

## Crate layout

```
crates/temper-agents/
├── Cargo.toml          # deps: temper-next (path); optional ts-rs behind `typescript`
└── src/
    ├── lib.rs          # crate docs + re-exports + module wiring
    ├── envelope.rs     # pub use of the temper-next contract types (data only)
    └── profile.rs      # DeploymentProfile + RuntimeBinding + Residency (owned, ts-rs-derived)
```

`Cargo.toml` essentials: `publish = false` (matches the workspace posture for internal crates pre-alpha); `[features] typescript = ["ts-rs"]`; `ts-rs` optional. No heavy deps — this is a types crate.

## Testing

- A ts-rs generation/snapshot test for `DeploymentProfile` (mirrors how `temper-core` validates its generated TS), so wire-shape drift is caught.
- A trivial round-trip serde test on `DeploymentProfile` and the enums (snake_case wire form).
- A compile-level assertion that the re-export surface resolves (e.g. a doc-test or `use` in a test) — guards the `temper-next` import path so a future relocation is caught here, not by a downstream consumer.
- No DB/ONNX features needed; this crate stays in the CI default tier.

## Out of scope

### Rejected (load-bearing — resist scope creep)
- **`temper-agents` owning the envelope types.** Rejected in favor of option B (thin re-export). The definitional home stays `temper-next` → `temper-core` at convergence; `temper-agents` is a consumer, never the owner. Inverting this would make `temper-next` depend on `temper-agents` and pre-empt the convergence lift's placement decision.
- **Re-exporting `EventContext` / `fire_with`.** These are substrate-side sqlx-bound write helpers, not neutral contract. A remote Claude-managed binding reaches the substrate over MCP and cannot use them; exposing them would leak the local write path into the neutral surface.
- **ts-rs-exporting the envelope types from here.** The derive must live on the definition; doing it here would force `temper-agents` to wrap/mirror `temper-next` types (a parallel-type smell) or reach into `temper-next` to add derives mid-cutover. Deferred to the convergence lift instead.
- **Materialization cadence as a profile field.** Region re-materialization is a deterministic, threshold-triggered substrate-maintenance job (re-cluster when changes since last materialization exceed a threshold; cheap post-WS5 Lance-Williams ~13.9×), system-wide and not agent-facing. Modeling it as an agent deployment knob would contradict the determinism reframe (agents tend structure, never cluster).

### Deferred (in scope elsewhere / later)
- **Thin MCP/HTTP client wrapper** (decision #6). No consumer yet; both candidate bindings are downstream. Lands with the first agent binding, shaped by an actual caller, wrapping `temper-client`. By then the chunk-5 flip will have settled the MCP surface it would talk to.
- **Envelope-type TS exports.** Land at the temper-core convergence lift, when the definitions move and gain `typescript`-gated derives.
- **The convergence lift itself** — relocating envelope/authorship types from `temper-next` to `temper-core`. Separate thread; `temper-agents`'s re-export path absorbs the move when it happens.
- **The agent bindings** (Eve steward / charter-bootstrapper; Claude-managed binding), the sweeper design, and cross-cogmap promotion-translation — all downstream WS7 threads.

## Connections

- Builds on PR #148 (invocation envelope + agent-authorship metadata in `temper-next`).
- Realizes WS7 decision #6 (`temper-llm → temper-agents`, neutral contract) and grounds decision #7 (deployment-profile shape) in the runtime investigation.
- Parallel-safe with the WS6 chunk-5 flip (sibling session) — additive crate + dead-crate cull, no backend/MCP cutover surface touched.

# temper-agents Neutral-Contract Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create `crates/temper-agents` — the runtime-neutral contract crate for temper's agent surface — and cull the dead `temper-llm` / `temper-llm-smoke` crates.

**Architecture:** A deliberately thin types crate (WS7 decision #6). It re-exports the invocation-envelope + agent-authorship data types already shipped in `temper-next` (PR #148) and owns exactly one new type — `DeploymentProfile` — with a ts-rs wire export. Dependency direction is `temper-agents → temper-next`, never the reverse; the convergence lift later relocates the envelope definitions and only the re-export paths change.

**Tech Stack:** Rust, serde, ts-rs (v10, behind a `typescript` feature), cargo-make, cargo-nextest.

**Spec:** `docs/superpowers/specs/2026-06-18-temper-agents-neutral-contract-crate-design.md`

## Global Constraints

- **Edition `2021`**, `version = "0.0.0"`, `publish = false` — matches the sibling internal crate `temper-next`.
- **`--all-features`** is required for all builds and clippy (`cargo make check` does this).
- **All public types implement `Debug`** (workspace rule).
- **Lint suppression uses `#[expect(lint_name, reason = "...")]`**, never `#[allow]`.
- **Tests run via cargo-nextest**; `cargo make` tasks force `SQLX_OFFLINE=true`.
- **Typed structs over inline JSON** — never `serde_json::json!()` for known-shape data (test assertions may use `serde_json::json!` / `to_value` for comparison).
- **No new heavy deps.** This is a types crate. Note: depending on `temper-next` transitively pulls its dep graph (incl. `temper-ingest` with ONNX via `ort`). This is expected and **compile-safe** — `ort` uses load-dynamic, so `cargo make check`/`build`/`clippy` need no ONNX runtime installed (only runtime embed-test *execution* would, and this crate has no such tests). The convergence lift later slims this by moving the envelope types to `temper-core`.
- **Enum wire form is snake_case** to match the rest of the model.

---

### Task 1: Scaffold the crate + `DeploymentProfile`

**Files:**
- Create: `crates/temper-agents/Cargo.toml`
- Create: `crates/temper-agents/src/lib.rs`
- Create: `crates/temper-agents/src/profile.rs`
- Test: inline `#[cfg(test)]` module in `crates/temper-agents/src/profile.rs`

**Interfaces:**
- Consumes: nothing (new crate).
- Produces: `temper_agents::profile::{DeploymentProfile, RuntimeBinding, Residency}` — `DeploymentProfile { runtime: RuntimeBinding, residency: Residency, token_budget: Option<u64> }`; `RuntimeBinding::{Eve, ClaudeManaged}`; `Residency::{Managed, SelfHosted}`. All derive `Debug, Clone, PartialEq, Eq, Serialize, Deserialize` (struct omits `Copy`).

The workspace `members` is the glob `["crates/*", "tests/e2e"]`, so creating `crates/temper-agents/` automatically enrolls it — no root `Cargo.toml` edit.

- [ ] **Step 1: Write the crate manifest**

Create `crates/temper-agents/Cargo.toml`:

```toml
[package]
name = "temper-agents"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
temper-next = { path = "../temper-next" }
serde = { version = "1", features = ["derive"] }
ts-rs = { version = "10", features = [
  "chrono-impl",
  "serde-json-impl",
  "uuid-impl",
], optional = true }

[features]
typescript = ["ts-rs"]

[dev-dependencies]
serde_json = "1"
```

- [ ] **Step 2: Write the crate root**

Create `crates/temper-agents/src/lib.rs`:

```rust
//! `temper-agents` — the runtime-neutral contract for temper's agent surface.
//!
//! A deliberately thin layer (WS7 decision #6). Owns the
//! [`profile::DeploymentProfile`] policy object. See the design spec under
//! `docs/superpowers/specs/2026-06-18-temper-agents-neutral-contract-crate-design.md`.

pub mod profile;
```

- [ ] **Step 3: Write the failing test**

Create `crates/temper-agents/src/profile.rs` with *only* the test module (no types yet, so it fails to compile — that is the failing state):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_profile_round_trips_snake_case() {
        let profile = DeploymentProfile {
            runtime: RuntimeBinding::ClaudeManaged,
            residency: Residency::SelfHosted,
            token_budget: Some(50_000),
        };
        let value = serde_json::to_value(&profile).unwrap();
        assert_eq!(value["runtime"], "claude_managed");
        assert_eq!(value["residency"], "self_hosted");
        assert_eq!(value["token_budget"], 50_000);

        let back: DeploymentProfile = serde_json::from_value(value).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn absent_token_budget_serializes_as_null() {
        let profile = DeploymentProfile {
            runtime: RuntimeBinding::Eve,
            residency: Residency::Managed,
            token_budget: None,
        };
        let value = serde_json::to_value(&profile).unwrap();
        assert_eq!(value["token_budget"], serde_json::Value::Null);
    }
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo nextest run -p temper-agents 2>&1 | tail -20`
Expected: FAIL — compile error, `cannot find type DeploymentProfile / RuntimeBinding / Residency in this scope`.

- [ ] **Step 5: Write the minimal implementation**

Prepend the types above the test module in `crates/temper-agents/src/profile.rs`:

```rust
//! Deployment-profile policy object: how and where an agent binding is deployed.
//!
//! Read by the runtime-binding layer; the substrate never reads it
//! (WS7 decision #3 — the kernel never branches on stratum).

use serde::{Deserialize, Serialize};

/// Which agent runtime this deployment binds to. (WS7 decision #1.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBinding {
    /// Vercel Eve durable agents.
    Eve,
    /// Claude Managed Agents (`/v1/agents` + `/v1/sessions`).
    ClaudeManaged,
}

/// Where tool execution runs. Orthogonal to [`RuntimeBinding`] — both runtimes
/// offer both (Eve: Vercel-managed vs docker/self-deploy; CMA: cloud vs
/// self-hosted). This is WS7's "stratum" made concrete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Residency {
    /// Runtime-operator-hosted sandbox (Vercel sandbox / CMA cloud env).
    Managed,
    /// Customer-infrastructure execution (Eve docker/self-deploy / CMA self-hosted worker).
    SelfHosted,
}

/// How an agent binding is deployed and paced. Carried by the binding layer;
/// never read by the substrate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentProfile {
    pub runtime: RuntimeBinding,
    pub residency: Residency,
    /// Token-denominated budget — neither runtime exposes a managed dollar
    /// budget, so spend governance is expressed in tokens. `None` = runtime default.
    pub token_budget: Option<u64>,
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo nextest run -p temper-agents 2>&1 | tail -20`
Expected: PASS — `2 tests run: 2 passed`.

- [ ] **Step 7: Run the workspace quality gate**

Run: `cargo make check 2>&1 | tail -20`
Expected: clippy + fmt + docs clean (no warnings; `-D warnings` enforced).

- [ ] **Step 8: Commit**

```bash
git add crates/temper-agents/Cargo.toml crates/temper-agents/src/lib.rs crates/temper-agents/src/profile.rs Cargo.lock
git commit -m "feat(temper-agents): scaffold crate + DeploymentProfile

New neutral-contract crate (WS7 decision #6). DeploymentProfile policy
object with runtime / residency / token-denominated budget, snake_case
wire form. Re-export surface and cull follow.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: ts-rs wire export for the owned types

**Files:**
- Modify: `crates/temper-agents/src/profile.rs` (add `typescript`-gated ts-rs derives)
- Modify: `tools/cargo-make/main.toml:210` (extend `generate-ts-types` to include `temper-agents`)
- Generated: `packages/temper-ui/src/lib/types/generated/deployment_profile.ts`

**Interfaces:**
- Consumes: `temper_agents::profile::{DeploymentProfile, RuntimeBinding, Residency}` from Task 1.
- Produces: a generated `deployment_profile.ts` with TypeScript types `DeploymentProfile`, `RuntimeBinding`, `Residency`.

ts-rs derives must sit on the type definition, so only the *owned* types are exported here. The re-exported envelope types (Task 3) are **not** ts-rs-exported — their TS lands at the temper-core convergence lift. Generation is driven by `cargo test --features typescript` with `TS_RS_EXPORT_DIR` set (the existing `temper-core` mechanism).

- [ ] **Step 1: Add `typescript`-gated ts-rs derives**

In `crates/temper-agents/src/profile.rs`, add two `cfg_attr` lines to **each** of the three types (immediately below the existing `#[derive(...)]`, above any `#[serde(...)]`). The result for each type looks like:

```rust
/// Which agent runtime this deployment binds to. (WS7 decision #1.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "deployment_profile.ts"))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBinding {
```

```rust
/// Where tool execution runs. ...
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "deployment_profile.ts"))]
#[serde(rename_all = "snake_case")]
pub enum Residency {
```

```rust
/// How an agent binding is deployed and paced. ...
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "deployment_profile.ts"))]
pub struct DeploymentProfile {
```

- [ ] **Step 2: Verify the crate still builds with the feature**

Run: `cargo build -p temper-agents --features typescript 2>&1 | tail -10`
Expected: builds clean. (`ts_rs::TS` derive resolves; `Option<u64>` maps to `number | null`.)

- [ ] **Step 3: Extend the `generate-ts-types` task**

In `tools/cargo-make/main.toml`, find the `temper-core` generation line (line ~210) and add a sibling line for `temper-agents`. Replace:

```toml
  "TS_RS_EXPORT_DIR=${CARGO_MAKE_WORKING_DIRECTORY}/packages/temper-ui/src/lib/types/generated cargo test -p temper-core --features typescript",
```

with:

```toml
  "TS_RS_EXPORT_DIR=${CARGO_MAKE_WORKING_DIRECTORY}/packages/temper-ui/src/lib/types/generated cargo test -p temper-core --features typescript",
  "TS_RS_EXPORT_DIR=${CARGO_MAKE_WORKING_DIRECTORY}/packages/temper-ui/src/lib/types/generated cargo test -p temper-agents --features typescript",
```

- [ ] **Step 4: Generate and verify the TypeScript**

Run: `cargo make generate-ts-types 2>&1 | tail -20`
Then: `cat packages/temper-ui/src/lib/types/generated/deployment_profile.ts`
Expected: a generated file declaring `DeploymentProfile`, `RuntimeBinding`, and `Residency`, e.g.:

```typescript
export type DeploymentProfile = { runtime: RuntimeBinding, residency: Residency, token_budget: number | null, };
export type Residency = "managed" | "self_hosted";
export type RuntimeBinding = "eve" | "claude_managed";
```

(Exact formatting/line order is ts-rs's; the three type names and the snake_case literals are what matters.)

- [ ] **Step 5: Run the workspace quality gate**

Run: `cargo make check 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-agents/src/profile.rs tools/cargo-make/main.toml packages/temper-ui/src/lib/types/generated/deployment_profile.ts
git commit -m "feat(temper-agents): ts-rs wire export for DeploymentProfile

Export the owned profile types to TypeScript via the existing
generate-ts-types flow. Envelope-type TS stays deferred to the
convergence lift.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Re-export the envelope contract

**Files:**
- Create: `crates/temper-agents/src/envelope.rs`
- Modify: `crates/temper-agents/src/lib.rs` (add `pub mod envelope;` + doc update)
- Test: inline `#[cfg(test)]` module in `crates/temper-agents/src/envelope.rs`

**Interfaces:**
- Consumes: `temper_next::ids::InvocationId`; `temper_next::payloads::{AgentAuthorship, ConfidenceBand, DelegatedLaunch, Disposition, InvocationClosed}` (these are `pub` in `temper-next`, confirmed: `payloads.rs` lines 481/492/508/517/529, `ids.rs`).
- Produces: `temper_agents::envelope::{InvocationId, AgentAuthorship, ConfidenceBand, DelegatedLaunch, Disposition, InvocationClosed}`.

Data types only. The substrate-side write helpers `EventContext` / `fire_with` (`temper_next::events`) are deliberately NOT re-exported — they are sqlx-bound and not part of a neutral contract a remote binding consumes over MCP.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-agents/src/envelope.rs` with *only* the test module (the `use super::*` resolves nothing yet → fails to compile, the failing state):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Compile-only guard: the re-export path resolves for every contract type.
    #[expect(dead_code, reason = "compile-only guard that the re-export paths resolve")]
    fn contract_resolves(
        _id: InvocationId,
        _launch: DelegatedLaunch,
        _closed: InvocationClosed,
        _authorship: AgentAuthorship,
        _band: ConfidenceBand,
    ) {
    }

    #[test]
    fn disposition_round_trips_via_reexport() {
        let value = serde_json::to_value(Disposition::Completed).unwrap();
        let back: Disposition = serde_json::from_value(value).unwrap();
        assert!(matches!(back, Disposition::Completed));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-agents 2>&1 | tail -20`
Expected: FAIL — compile error, the envelope types are unresolved (the module has no re-exports yet).

- [ ] **Step 3: Write the re-export surface**

Prepend the re-exports above the test module in `crates/temper-agents/src/envelope.rs`:

```rust
//! The runtime-neutral agent accountability contract.
//!
//! Re-exports the invocation-envelope + agent-authorship data types that
//! currently live in `temper-next` (shipped in PR #148). This crate is a thin
//! *consumer* of those types, never their owner — the definitional home stays
//! `temper-next` now and moves to `temper-core` at the convergence lift, at
//! which point only the `pub use` paths below change.
//!
//! Data types only: the substrate-side write helpers (`EventContext`,
//! `fire_with`) are deliberately NOT re-exported — a remote (Claude-managed)
//! binding reaches the substrate over MCP and cannot use sqlx-bound helpers.

pub use temper_next::ids::InvocationId;
pub use temper_next::payloads::{
    AgentAuthorship, ConfidenceBand, DelegatedLaunch, Disposition, InvocationClosed,
};
```

- [ ] **Step 4: Wire the module into the crate root**

Replace the contents of `crates/temper-agents/src/lib.rs` with:

```rust
//! `temper-agents` — the runtime-neutral contract for temper's agent surface.
//!
//! A deliberately thin layer (WS7 decision #6): owns the
//! [`profile::DeploymentProfile`] policy object and re-exports the
//! invocation-envelope + agent-authorship contract from `temper-next`
//! ([`envelope`]). See the design spec under
//! `docs/superpowers/specs/2026-06-18-temper-agents-neutral-contract-crate-design.md`.

pub mod envelope;
pub mod profile;
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo nextest run -p temper-agents 2>&1 | tail -20`
Expected: PASS — the profile tests plus `disposition_round_trips_via_reexport` all pass.

- [ ] **Step 6: Run the workspace quality gate**

Run: `cargo make check 2>&1 | tail -20`
Expected: clean (the `[`envelope`]` intra-doc link now resolves).

- [ ] **Step 7: Commit**

```bash
git add crates/temper-agents/src/envelope.rs crates/temper-agents/src/lib.rs
git commit -m "feat(temper-agents): re-export the temper-next envelope contract

Thin pub-use surface for the invocation-envelope + authorship data types
(InvocationId, DelegatedLaunch, InvocationClosed, Disposition,
AgentAuthorship, ConfidenceBand). Write helpers stay in temper-next.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Cull `temper-llm` + `temper-llm-smoke`

**Files:**
- Delete: `crates/temper-llm/` (whole directory)
- Delete: `crates/temper-llm-smoke/` (whole directory)
- Modify: `tools/scripts/release/detect-changes.sh:52` (release-tooling regex)
- Modify: `docs/guides/releasing.md:38` (prose dep list)
- Modify: `crates/temper-client/src/http.rs:19-20` (stale doc-comment reference)
- Regenerate: `Cargo.lock`

**Interfaces:** none — pure removal.

**⚠️ Do NOT touch the `temper-llm-model` / `temper-llm-run` *managed-meta frontmatter fields*** in `temper-core` (`schemas/base.schema.json`, `src/frontmatter/fields.rs`, `src/types/managed_meta.rs`, `src/frontmatter/document.rs`) or `temper-next` (`src/synthesis/key_fate.rs`). Those are unrelated to the crate and must stay. **Do NOT rewrite historical plan/spec docs** under `docs/superpowers/` that mention `temper-llm` — they are point-in-time records. The only references to *the crate* that change are the three files listed above (plus the auto-regenerated `Cargo.lock`).

- [ ] **Step 1: Delete the two crate directories**

```bash
git rm -r crates/temper-llm crates/temper-llm-smoke
```

- [ ] **Step 2: Update the release change-detection regex**

In `tools/scripts/release/detect-changes.sh` (line ~52), replace:

```sh
if changes_match '^crates/(temper-cli|temper-core|temper-client|temper-ingest|temper-llm)/'; then
```

with:

```sh
if changes_match '^crates/(temper-cli|temper-core|temper-client|temper-ingest)/'; then
```

- [ ] **Step 3: Update the releasing guide prose**

In `docs/guides/releasing.md` (line ~38), replace:

```markdown
2. Detects whether `temper-cli` or any of its workspace deps (`temper-core`, `temper-client`, `temper-ingest`, `temper-llm`) or release/installer tooling changed since the last `v*` tag. If nothing changed, it exits cleanly — no release needed.
```

with:

```markdown
2. Detects whether `temper-cli` or any of its workspace deps (`temper-core`, `temper-client`, `temper-ingest`) or release/installer tooling changed since the last `v*` tag. If nothing changed, it exits cleanly — no release needed.
```

- [ ] **Step 4: Fix the stale doc-comment in temper-client**

In `crates/temper-client/src/http.rs` (lines 19-20), the `MAX_ATTEMPTS` doc comment references the deleted crate. Replace:

```rust
/// Total attempts (initial request + retries) for safe, idempotent requests
/// that fail transiently. Mirrors the retry convention in `temper-llm`'s
/// providers. Absorbs a Vercel cold-start / Neon compute-resume blip — the
```

with:

```rust
/// Total attempts (initial request + retries) for safe, idempotent requests
/// that fail transiently. Absorbs a Vercel cold-start / Neon compute-resume blip — the
```

- [ ] **Step 5: Regenerate `Cargo.lock` and verify the workspace builds without the crates**

Run: `cargo build --workspace --all-features 2>&1 | tail -20`
Expected: builds clean; `Cargo.lock` no longer lists `temper-llm` / `temper-llm-smoke` (cargo rewrites it). Confirm:

Run: `grep -c 'name = "temper-llm' Cargo.lock`
Expected: `0`.

- [ ] **Step 6: Run the workspace quality gate**

Run: `cargo make check 2>&1 | tail -20`
Expected: clean — no references to the deleted crates anywhere in the build graph.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "chore: cull dead temper-llm + temper-llm-smoke crates

Replaced by temper-agents. No production callers (temper-llm-smoke was
the only dependent). Updates the release change-detection regex, the
releasing guide, and a stale temper-client doc comment. The
temper-llm-model / temper-llm-run managed-meta fields are unrelated and
untouched.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final Verification

After all four tasks, confirm the whole thing is green end-to-end:

- [ ] `cargo make check` — clippy + fmt + docs + machete clean across the workspace.
- [ ] `cargo nextest run -p temper-agents` — all crate tests pass.
- [ ] `cargo make test` — workspace unit tests pass (no DB needed).
- [ ] `cargo make generate-ts-types` — regenerates cleanly; `deployment_profile.ts` present.
- [ ] `grep -rc 'name = "temper-llm' Cargo.lock` returns `0`.

---

## Self-Review (plan author)

**Spec coverage:**
- Re-export surface (data types only, not `EventContext`/`fire_with`) → Task 3. ✓
- `DeploymentProfile` (3 fields + enums, snake_case) → Task 1. ✓
- ts-rs export for owned types only → Task 2. ✓
- Cull `temper-llm` + `temper-llm-smoke` + releasing.md + release tooling → Task 4. ✓
- Crate layout (`lib.rs` / `envelope.rs` / `profile.rs`), `publish = false`, testing → Tasks 1–3. ✓
- Deferred items (thin client, envelope-type TS, convergence lift, bindings) → not implemented, by design. ✓

**Type consistency:** `DeploymentProfile`/`RuntimeBinding`/`Residency` field and variant names are identical across Tasks 1–2; the six re-exported envelope type names match `temper-next`'s `pub` decls (verified by grep) and are used identically in Task 3's test.

**Placeholder scan:** no TBD/TODO/"handle edge cases"/"similar to Task N"; every code step shows complete content; every referenced type is defined in a task or grounded in a cited `temper-next` path.

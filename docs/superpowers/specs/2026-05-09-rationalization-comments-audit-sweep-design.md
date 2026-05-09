# Rationalization-Comments Audit Sweep — Design

**Date:** 2026-05-09
**Status:** Design
**Predecessor task:** `audit-followups--rationalization-comments-hiding-incomplete-implementations` (vault, context: temper)
**Sub-task closure:** `deprecate-resource-service--create-after-phase-3b`

## Summary

The 2026-05-08 rationalization-comments audit surfaced five findings (A.1, A.2, A.3, B.1, B.2) where in-code soft-fail comments hid deferred or incomplete implementations. After the Wave 1 Phase 3b/3c PR landed (commit `6ded483`), three of those findings need re-triage and one (B.1, UI) is intentionally deferred. This spec resolves the rest.

The work is mechanical except for one design-bearing piece (A.3 expiry warning). Total surface: ~5 commits, ~80–150 LOC across `temper-client`, `temper-cli`, and `temper-api`, plus a vault-task-body refresh and one task closure.

## Background

### What 3b/3c already retired
- **A.1 — `translators.rs:54-57` dark-launch comment.** Phase 3b Task 10 extended `BodyUpdate` with `content_hash` and `chunks_packed`, removed the dark-launch rationalization, and replaced it with a forward-looking docstring describing the short-circuit behavior. **No further action.**
- **`resource_service::create` deletion.** Phase 3b Task 12 deleted the function (commit `f483453`); zero callers remain. **No further action; close the deprecate-resource-service vault task.**

### What 3b/3c-introduced code looks like
A grep across `crates/temper-api/src/backend/`, `handlers/`, and `crates/temper-mcp/src/tools/` for the audit's hunting shapes (`for now`, `acceptable tradeoff`, `deferred`, `intentionally`, `temporarily`, `once X lands`, `future work will`) returned three hits:

1. `handlers/resources.rs:230` — *"Wire-supplied content_hash and chunks_packed are intentionally ignored — server is single source of truth (Contract tightening from Phase 3b)"*. **Documents a deliberate API contract**, not a deferral. Keep as-is.
2. `tools/resources.rs:102` — *"the open tier is intentionally untyped"*. **Documents a design rationale** for `open_meta`. Keep as-is.
3. `handlers/graph.rs:41` — already in audit as A.2 (see Component 2).

**Sweep result:** Phase 3b/3c introduced no new soft-fail rationalization comments. The audit's pattern detection held up cleanly.

### What's still load-bearing from the audit
- **A.2 — `handlers/graph.rs:41` cross-owner deferral comment.** Doc nudge.
- **A.3 — `auth.rs:553-569` env-var refresh-less rationalization.** Doc rewrite + runtime guard.
- **B.2 — `actions/ingest.rs:398-400` `@me` hardcode TODO.** Code change (wider than initially estimated; see Component 3).
- **B.1 — `KnowledgeGraph.svelte:46-49` meta-doc mode stub.** Out of scope (UI work deferred per `feedback_ui_last`).

## Components

### Component 1 — A.3: env-var auth doc rewrite + expiry warning

**Why this is doc-not-architecture.** Reading `docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md` (Unit B.4) reframes A.3:

- The B.4 spec recommends **W1 — access-token-only export, no refresh-token export**.
- Reason: exporting a user's RT to a cloud session would break Auth0's refresh-token rotation. Local CLI and cloud session sharing the same RT means whichever one refreshes first invalidates the other; on the second's next refresh, Auth0 flags a breach and kills the entire grant family.
- Therefore **env-var auth is intentionally refresh-less by design**, not a deferral. The 24h Auth0-default AT TTL is the contract; users re-export to renew.
- The current docstring's "Unit B.4 research work, not B.1" framing is *stale*: B.1 has shipped, B.4's research spec exists, and W1 is the architectural answer.

The audit's three suggested mitigations resolve as:
- **(a) gate behind `TEMPER_CLOUD_MODE=ephemeral`** — already exists structurally as `TEMPER_VAULT_STATE=cloud` + `MemoryTokenStore::from_env_required()` (errors with a clear message if `TEMPER_TOKEN` unset). No new gate needed.
- **(b) document TTL contract** — the doc rewrite. Below.
- **(c) add refresh-token handling** — actively conflicts with W1. Reject.

**Redundancy verdict.** `stored_auth_from_env()` is correctly load-bearing as the parsing primitive shared by `MemoryTokenStore::from_env()` (line 122), `DiskTokenStore::load()`'s env fallback (line 70), and the legacy `load_auth()` free function (line 462). No deletion. The comment is what's stale, not the function.

#### Doc rewrite

Replace the docstring on `pub fn stored_auth_from_env()` at `crates/temper-client/src/auth.rs:553-569` to:

- Drop the "Unit B.4 research work, not B.1" sentence entirely.
- State the W1 contract: refresh-less env-var auth is **intentional architectural choice** (Auth0 RT-rotation security), not a deferral. 24h Auth0-default AT TTL is the ceiling.
- Reference `docs/superpowers/specs/2026-04-19-cloud-mode-auth0-design.md` §Q1/W1 so the rationale is findable.
- Note the function's role as a parsing primitive (used by both `MemoryTokenStore` and `DiskTokenStore`), not a user-facing API. The user-facing surface is `MemoryTokenStore::from_env_required()`.
- Note the recovery path: when an env-var token approaches expiry, the cloud-mode bootstrap warns (Component 1's expiry warning, below); on actual expiry, callers receive `ClientError::TokenExpired` and must re-export.

#### Expiry warning

**Site:** `crates/temper-cli/src/actions/runtime.rs:46`, immediately after the `MemoryTokenStore::from_env_required()` call.

**Behavior:**
- Read `expires_at` from the loaded `StoredAuth`.
- If `expires_at - now() < Duration::from_secs(3600)` (1 hour), emit a stderr warning:
  ```
  warning: TEMPER_TOKEN expires at <RFC3339-ts> (~<N> minutes).
           Re-run `temper auth export-token` and re-set TEMPER_TOKEN to renew.
  ```
- If the token has already expired (`expires_at < now()`), emit a stronger error-shaped warning but allow the call to proceed — the next API call will surface `ClientError::TokenExpired` cleanly.

**Threshold:** 1 hour. Rationale: long enough that ignoring is plausible, short enough to be actionable mid-task. Tunable via constant if telemetry shows the threshold is wrong.

**Scope guard:** only fires for the cloud-mode bootstrap path (`TEMPER_VAULT_STATE=cloud`). Disk-backed CLI auto-refreshes via the RT and doesn't need this. Achieved naturally by placing the warning at the cloud-mode bootstrap site.

**Rough size:** ~25 LOC including a small helper `time_until_expiry(stored: &StoredAuth) -> Duration` and a unit test. The helper is justified because both the threshold check and the human-readable "~N minutes" formatting reuse it.

**Tests:**
- Unit test in `runtime.rs`: token with expiry 30 min out → warning fires; token with expiry 2h out → no warning; expired token → expired-shaped warning.
- Use a fake-clock parameter on the helper (or pass `expires_at` and `now` separately) so the test isn't time-of-day flaky.

### Component 2 — A.2: graph cross-owner doc nudge

**Site:** `crates/temper-api/src/handlers/graph.rs:41`.

**Current comment:**
```rust
// Resolve `owner` — v1 only supports "@me" (caller's own vault).
// Cross-owner querying is deferred; handles are left as a later migration.
// A client-supplied handle other than "@me" is an invalid query parameter,
// so we return 400 Bad Request rather than 404.
```

**Change:** drop the middle sentence ("Cross-owner querying is deferred…"). The first sentence already says "v1 only supports" without the soft-fail framing. Replace it with a single line that says: *cross-owner is a v1-scope boundary; expanding it requires permission-model design (out of scope here)*.

**Net:** -1 line of "deferred-with-no-tracking-link" framing, +1 line of "v1-scope boundary, design-bounded if extended."

**No code change.** No tests touched.

### Component 3 — B.2: thread owner through `build_vault_path`

**Why this is wider than the audit estimated.**

The audit's recommendation was "thread owner through `build_vault_path`," but the call graph is wider:

- `build_vault_path` (def at `actions/ingest.rs:397`) takes `(vault_root, context, doc_type, slug)` and hardcodes `@me` via `Vault::new(...).doc_file("@me", ...)`.
- `dedup_vault_slug` (`actions/ingest.rs:405`) calls `build_vault_path` twice; signature also lacks owner.
- `write_vault_file_and_register` (`actions/ingest.rs:583`) takes 8 args (already with `#[expect(clippy::too_many_arguments)]`) and calls `build_vault_path` at line 593; signature also lacks owner.

Caller sites (verified via grep):

| Site | File | Has Config? |
|------|------|-------------|
| L148, L154 | `commands/add.rs` | yes |
| L259, L268 | `commands/add.rs` | yes |
| L356, L358 | `commands/add.rs` | yes |
| L421, L423 | `commands/add.rs` | yes |
| L719, L725 | `commands/add.rs` | yes |
| L910, L918 | `commands/add.rs` | yes |
| L3271 | `actions/sync.rs` (test only) | n/a — pass `"@me"` literal |

Plus 2 unit tests in `actions/ingest.rs` (lines 792, 799).

**Approach.** Add `owner: &str` as the second param of `build_vault_path`, propagate through `dedup_vault_slug` and `write_vault_file_and_register`. At each `commands/add.rs` site, the Config is already in scope; replace the implicit `@me` with `config.owner_for_context(ctx)`.

**Param-count smell.** `write_vault_file_and_register` becomes 9 args. The `#[expect(clippy::too_many_arguments)]` was already there before this change; B.2 strictly worsens it. **This spec does NOT take on the params-struct refactor** — that's a separate code-quality concern surfaced by this work and captured in the audit task body as a follow-up. Reasoning: keeping B.2 mechanical is the YAGNI choice; introducing `VaultWritePlan` mid-sweep mixes concerns and inflates review cost.

**Tests.**
- Update the 2 existing unit tests in `actions/ingest.rs` to pass `"@me"` as owner.
- Add one new unit test asserting `build_vault_path("/v", "@petetaylor", "work", "note", "doc")` produces `/v/@petetaylor/work/note/doc.md` (vs the implicit `@me` shape).
- The sync.rs test at L3271 takes `"@me"` literal — preserves existing behavior.

**TODO comment removed** at `actions/ingest.rs:398-400`.

### Component 4 — Audit refresh + task closure

**Audit task body update** (`audit-followups--rationalization-comments-hiding-incomplete-implementations`, type=task, context=temper):

Rewrite §Findings to mark current state:
- A.1 ✅ resolved by 3b/3c (commit `6ded483`).
- A.2 ✅ resolved by this sweep (commit ref TBD at finish).
- A.3 ✅ resolved by this sweep — doc rewrite + expiry warning + redundancy verdict (commit refs TBD).
- B.1 ⏸ still UI-deferred (`feedback_ui_last`); intentionally not in this sweep.
- B.2 ✅ resolved by this sweep (commit ref TBD).

Add a new section "Follow-ups surfaced":
- `write_vault_file_and_register` has 9 args after B.2; candidate for `VaultWritePlan` params-struct refactor (see CLAUDE.md "Params structs" rule).
- New rationalization-shape sweep across 3b/3c-introduced code returned **zero new soft-fail comments** — the two `intentionally`-tagged comments document deliberate contract decisions, not deferrals.

**Task closure:** mark `deprecate-resource-service--create-after-phase-3b` as `done` — its acceptance ("`pub async fn create` no longer defined in `resource_service.rs`") is verified.

## Out of scope

- **A.3 refresh-token wiring.** Conflicts with W1 design.
- **A.2 cross-owner implementation.** Permission-model design needed; not in this sweep.
- **B.1 UI meta-doc mode.** UI-deferred.
- **`VaultWritePlan` params struct.** Captured as follow-up in audit task body.
- **Service-layer redundancy audit.** Pete picked "rationalization comments first" — service-layer redundancy is a separate session.

## Verification

- `cargo make check` clean (Rust fmt + clippy + machete).
- `cargo nextest run --workspace` clean.
- `cargo nextest run -p temper-api --features test-db` clean (no API surface changed but adjacent code touched).
- Manual: set `TEMPER_TOKEN` to a JWT with `exp` ~30 min out, run any cloud-mode CLI command, confirm warning fires; set to one ~2h out, confirm no warning; set to expired, confirm expired-warning shape.

## Commit shape (proposed, in order)

1. **doc(client): rewrite stored_auth_from_env docstring to reflect W1 contract** — A.3 doc rewrite only.
2. **feat(cli): warn when env-var TEMPER_TOKEN approaches expiry** — A.3 expiry warning + helper + unit test.
3. **doc(api): tighten graph subgraph cross-owner comment** — A.2 nudge.
4. **refactor(cli): thread owner through vault-path helpers** — B.2; one commit covering signature change + caller updates + tests + TODO removal.
5. **chore(vault): refresh audit-followups task body and close deprecate-resource-service** — Component 4 (vault commit, no code change).

## Risks

- **B.2 caller sites that turn out to lack Config access.** Verified via grep that all `add.rs` callers have Config in scope; the only exception is the sync.rs test which can take a literal. Low risk but check at implementation time.
- **Expiry warning false positives in tests.** Mitigated by passing `now: SystemTime` as a parameter to the helper and using a fixed timestamp in tests.
- **Spec drift if Pete wants `VaultWritePlan` after all.** Captured as follow-up; if Pete prefers it in this sweep, B.2 becomes a 2-commit subseries (signature change first, params struct second). No design change needed; just a pacing decision.

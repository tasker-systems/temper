# Offboarding: surface residual owned-resource reach on member removal — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a team member is removed (or self-leaves), report the resources they still own in the team's contexts so an admin can hand them off deliberately — no auto-reassign.

**Architecture:** The bulk handoff mechanism (`reassign_team_resources`, Lever B) already exists end-to-end. This plan (1) extracts its scope query into one shared read, then (2) has `remove_member` run that read after a successful removal and return a new typed outcome carrying the residual reach, (3) threads the outcome through API → client → CLI (a nudge toward the existing `team reassign`). MCP is untouched. It is the D3 "surface, don't sweep" sibling.

**Tech Stack:** Rust (axum, sqlx, utoipa), temper-core DTOs (ts-rs + utoipa derives), temper-client, temper-cli, PostgreSQL.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-07-15-offboarding-owned-resource-handoff-design.md` — read it; carry its invariants verbatim.
- **No auto-reassign, no pre-check block, no MCP surface, reassign stays UUID-only.** Surfacing only.
- **Auth before writes** — the residual read is read-only and runs *after* the existing auth+delete; it never reorders the guard.
- **One shared scope definition** — the residual set the warning reports MUST be the exact set `reassign_team_resources` moves. One query, no second copy (Fundamentals: extract shared predicate sets).
- **Typed structs over inline JSON** — wire types live in `temper-core` with `web-api` + `typescript` derives.
- **SQL macros** — `sqlx::query!` / `query_scalar!`; regenerate the cache after SQL changes (`cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services` / `prepare-api` / `prepare-e2e` for test-target queries).
- **Wire-contract regen** — a new response DTO restales `openapi.json` + temper-rb gem + temper-ts `schema.ts` (`cargo make openapi`) and ts-rs types (`cargo make generate-ts-types`). Stage the output; the drift gates diff against git.
- **Skew-safety** — the response changes from `204 No Content` to `200 + body`; additive, the existing client discards bodies. Must not hard-fail across version skew.
- **Gate before done** — `cargo make check` + the affected test targets green before claiming complete.

---

## File Structure

- **Modify** `crates/temper-services/src/services/reassign_service.rs` — add `ScopedOwnedRow` + `team_scoped_owned`; refactor `reassign_team_resources` to consume it.
- **Modify** `crates/temper-core/src/types/reassign.rs` — add `RemoveMemberOutcome`, `ResidualOwnedReach`, `ResidualContext`.
- **Modify** `crates/temper-services/src/services/team_service.rs` — `remove_member` returns `RemoveMemberOutcome`; new residual-surfacing tests.
- **Modify** `crates/temper-api/src/handlers/teams.rs` — `remove_member` handler returns `200 + Json<RemoveMemberOutcome>`; update utoipa.
- **Modify** `crates/temper-client/src/teams.rs` — `remove_member` returns `RemoveMemberOutcome` (`send_json`).
- **Modify** `crates/temper-cli/src/commands/team.rs` — `remove_member_remote` + `leave_remote` print the nudge on `count > 0`.
- **Modify** `crates/temper-api/tests/team_lifecycle_test.rs` — API integration: removal returns the outcome body.
- **Regen (staged):** `openapi.json`, `clients/temper-rb/lib/temper/generated/**`, `clients/temper-ts/src/generated/schema.ts`, `crates/temper-core/.../generated/*.ts`, `.sqlx/` caches.

---

## Task 1: Extract the shared scope read (`team_scoped_owned`)

**Files:**
- Modify: `crates/temper-services/src/services/reassign_service.rs` (scope query at `:172-190`; bulk fn `reassign_team_resources` at `:137`)

**Interfaces:**
- Produces:
  - `struct ScopedOwnedRow { pub resource_id: Uuid, pub context_id: Uuid, pub context_ref: String }`
  - `async fn team_scoped_owned(pool: &PgPool, team_id: Uuid, profile_id: Uuid) -> ApiResult<Vec<ScopedOwnedRow>>` — resources owned by `profile_id` **and** homed in a context shared to `team_id`; `context_ref` is the decorated `{owner_ref}/{slug}` (owner_ref = `@handle` for a profile-owned context, `+slug` for a team-owned one), ordered by `(slug, resource_id)`.

- [ ] **Step 1: Write the failing test** (append to the `tests` module in `reassign_service.rs`)

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn team_scoped_owned_matches_bulk_scope(pool: PgPool) {
    // Same fixture shape as `bulk_reassigns_only_owned_and_scoped`.
    let leaver = mk_profile(&pool, "leaver").await;
    let other = mk_profile(&pool, "other").await;
    let team = mk_team(&pool, "acme").await;

    let shared = mk_context(&pool, "shared", leaver).await;
    share_ctx(&pool, shared, team).await;
    let private = mk_context(&pool, "private", leaver).await; // NOT shared to team

    let in_scope = mk_homed_resource(&pool, shared, leaver).await; // owned + scoped → listed
    let _out_scope = mk_homed_resource(&pool, private, leaver).await; // owned, not scoped → excluded
    let _not_leaver = mk_homed_resource(&pool, shared, other).await; // scoped, other owner → excluded

    let rows = team_scoped_owned(&pool, team, *leaver).await.expect("scope read");
    let ids: Vec<uuid::Uuid> = rows.iter().map(|r| r.resource_id).collect();
    assert_eq!(ids, vec![in_scope]);
    assert!(rows[0].context_ref.ends_with("/shared"), "decorated ref: {}", rows[0].context_ref);
}
```

- [ ] **Step 2: Run it — expect FAIL (unresolved `team_scoped_owned`)**

Run: `cargo nextest run -p temper-services --features test-db team_scoped_owned_matches_bulk_scope`
Expected: FAIL — `cannot find function team_scoped_owned`.

- [ ] **Step 3: Add `ScopedOwnedRow` + `team_scoped_owned`** (place above `reassign_team_resources`)

```rust
/// A resource owned by a given profile and homed in a context shared to a given
/// team. The single definition of "what a departing member still owns in this
/// team" — consumed by both the bulk handoff (the move set) and remove_member's
/// residual surfacing (the count + per-context breakdown), so the two can never drift.
pub struct ScopedOwnedRow {
    pub resource_id: Uuid,
    pub context_id: Uuid,
    /// Decorated context ref `{owner_ref}/{slug}` (owner_ref: `@handle` | `+slug`).
    pub context_ref: String,
}

/// Resources owned by `profile_id` and homed in a context shared to `team_id`.
pub async fn team_scoped_owned(
    pool: &PgPool,
    team_id: Uuid,
    profile_id: Uuid,
) -> ApiResult<Vec<ScopedOwnedRow>> {
    let rows = sqlx::query!(
        r#"
        SELECT h.resource_id AS "resource_id!",
               c.id          AS "context_id!",
               c.slug        AS "slug!",
               CASE c.owner_table
                 WHEN 'kb_teams'    THEN '+' || t.slug
                 WHEN 'kb_profiles' THEN '@' || p.handle
               END AS "owner_ref!"
        FROM kb_team_contexts tc
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
        JOIN kb_contexts c ON c.id = tc.context_id
        LEFT JOIN kb_teams    t ON c.owner_table = 'kb_teams'    AND t.id = c.owner_id
        LEFT JOIN kb_profiles p ON c.owner_table = 'kb_profiles' AND p.id = c.owner_id
        WHERE tc.team_id = $1 AND h.owner_profile_id = $2
        ORDER BY c.slug, h.resource_id
        "#,
        team_id,
        profile_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ScopedOwnedRow {
            resource_id: r.resource_id,
            context_id: r.context_id,
            context_ref: format!("{}/{}", r.owner_ref, r.slug),
        })
        .collect())
}
```

- [ ] **Step 4: Refactor `reassign_team_resources` to consume it** — replace the inline `targets` query (`reassign_service.rs:173-189`) with:

```rust
    // Scope read: the shared definition (owned by `from` ∩ homed in a team-shared context).
    let targets: Vec<Uuid> = team_scoped_owned(pool, team_id, from_profile_id)
        .await?
        .into_iter()
        .map(|r| r.resource_id)
        .collect();
    if targets.is_empty() {
        return Ok(Vec::new());
    }
```

The rest of `reassign_team_resources` (emitter, tx loop over `&targets`, commit, `Ok(targets)`) is unchanged.

- [ ] **Step 5: Regenerate the services sqlx cache**

Run: `cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services`
(Requires Docker Postgres on 5437 + `DATABASE_URL` exported.)

- [ ] **Step 6: Run the new test + the existing bulk tests — expect PASS**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(team_scoped_owned_matches_bulk_scope) or test(bulk_reassigns_only_owned_and_scoped) or test(bulk_empty_match_is_ok)'`
Expected: PASS. The unchanged bulk tests passing proves the refactor is behavior-preserving.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-services/src/services/reassign_service.rs crates/temper-services/.sqlx .sqlx
git commit -m "refactor(reassign): extract team_scoped_owned as the shared scope read"
```

---

## Task 2: `remove_member` returns the residual reach

**Files:**
- Modify: `crates/temper-core/src/types/reassign.rs`
- Modify: `crates/temper-services/src/services/team_service.rs` (`remove_member` at `:386`; tests from `:591`)

**Interfaces:**
- Consumes: `reassign_service::team_scoped_owned` (Task 1).
- Produces:
  - `RemoveMemberOutcome { residual_owned: ResidualOwnedReach }`
  - `ResidualOwnedReach { count: usize, contexts: Vec<ResidualContext> }`
  - `ResidualContext { context_ref: String, count: usize }`
  - `team_service::remove_member(...) -> ApiResult<RemoveMemberOutcome>` (was `ApiResult<()>`)

- [ ] **Step 1: Add the DTOs** to `crates/temper-core/src/types/reassign.rs` (mirror the derive stack already on `BulkReassignAck` in that file)

```rust
/// One team context a removed member still owns resources in, with the count.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResidualContext {
    /// Decorated context ref `{owner_ref}/{slug}`.
    pub context_ref: String,
    pub count: usize,
}

/// The resources a removed member still OWNS in the team's contexts — the reach
/// an admin should hand off via `team reassign`. `count == 0` is the clean case.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResidualOwnedReach {
    pub count: usize,
    pub contexts: Vec<ResidualContext>,
}

/// Response to a member removal (or self-leave): the removal happened; this
/// reports the residual owned-resource reach so the caller can hand it off.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveMemberOutcome {
    pub residual_owned: ResidualOwnedReach,
}
```

- [ ] **Step 2: Write the failing test** (append to the `tests` module in `team_service.rs`; reuse that module's existing helpers — check their names, e.g. member/context/resource builders — before writing)

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn remove_member_surfaces_residual_owned_reach(pool: PgPool) {
    // owner removes `leaver`, who owns 2 resources in a shared context + 0 elsewhere.
    let owner = mk_profile(&pool, "owner").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, owner, "owner").await;
    add_member(&pool, team, leaver, "member").await;

    let shared = mk_context(&pool, "shared", leaver).await;
    share_ctx(&pool, shared, team).await;
    let _r1 = mk_homed_resource(&pool, shared, leaver).await;
    let _r2 = mk_homed_resource(&pool, shared, leaver).await;

    let outcome = remove_member(&pool, ProfileId::from(*owner), team, *leaver)
        .await
        .expect("removal ok");
    assert_eq!(outcome.residual_owned.count, 2);
    assert_eq!(outcome.residual_owned.contexts.len(), 1);
    assert_eq!(outcome.residual_owned.contexts[0].count, 2);
    assert!(outcome.residual_owned.contexts[0].context_ref.ends_with("/shared"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn remove_member_residual_is_empty_when_nothing_owned(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let leaver = mk_profile(&pool, "leaver").await;
    let team = mk_team(&pool, "acme").await;
    add_member(&pool, team, owner, "owner").await;
    add_member(&pool, team, leaver, "member").await;

    let outcome = remove_member(&pool, ProfileId::from(*owner), team, *leaver)
        .await
        .expect("removal ok");
    assert_eq!(outcome.residual_owned.count, 0);
    assert!(outcome.residual_owned.contexts.is_empty());
}
```

> ⚠️ **Reuse the module's real helpers.** `team_service.rs`'s test module may not define `mk_context`/`mk_homed_resource`/`share_ctx` (those live in `reassign_service.rs`). Before writing, grep this module's `tests` block for its actual fixture helpers; if the context/resource builders are absent, add minimal local copies (mirror `reassign_service.rs:234-320`) or import them. Do not invent helper names.

- [ ] **Step 3: Run — expect FAIL** (return type is `()`, no `.residual_owned`)

Run: `cargo nextest run -p temper-services --features test-db remove_member_surfaces_residual_owned_reach`
Expected: FAIL — compile error on `.residual_owned` / mismatched return type.

- [ ] **Step 4: Change `remove_member` to compute + return the outcome.** Add the import `use temper_core::types::reassign::{RemoveMemberOutcome, ResidualOwnedReach, ResidualContext};` and change the signature to `-> ApiResult<RemoveMemberOutcome>`. Replace the trailing `Ok(())` (after the last-owner-guard block) with:

```rust
    // Removal succeeded. Surface (read-only) the reach the removed member still
    // OWNS in this team's contexts, so the caller can hand it off deliberately.
    // The scope query is membership-independent, so it is correct post-delete.
    let owned = crate::services::reassign_service::team_scoped_owned(pool, team_id, target).await?;
    let mut contexts: Vec<ResidualContext> = Vec::new();
    for row in &owned {
        match contexts.last_mut() {
            Some(last) if last.context_ref == row.context_ref => last.count += 1,
            _ => contexts.push(ResidualContext { context_ref: row.context_ref.clone(), count: 1 }),
        }
    }
    Ok(RemoveMemberOutcome {
        residual_owned: ResidualOwnedReach { count: owned.len(), contexts },
    })
```

(The fold relies on `team_scoped_owned`'s `ORDER BY c.slug` grouping same-context rows adjacently.)

- [ ] **Step 5: Fix the existing `remove_member` call sites in this module's tests.** The success-path callers (`team_service.rs:627, 650, 769`) use `remove_member(...).await` then `.unwrap()`/`.expect()` — they now yield a `RemoveMemberOutcome` they can ignore (append `;` / bind `let _ =`). The error-path callers (`:647, 661, 673`) bind to `denied` and assert `.is_err()` / matches — unaffected (only the `Ok` type changed). Confirm each compiles; adjust only if a caller pattern-matched on `Ok(())`.

- [ ] **Step 6: Regenerate ts-rs types + services sqlx cache**

Run: `cargo make generate-ts-types && cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services`

- [ ] **Step 7: Run the new tests + existing removal tests — expect PASS**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(remove_member) or test(owner_removes_member) or test(maintainer_can_remove_a_member) or test(cannot_remove_last_owner)'`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/types/reassign.rs crates/temper-services/src/services/team_service.rs crates/temper-services/.sqlx .sqlx crates/temper-core/bindings 2>/dev/null; git add -A
git commit -m "feat(offboarding): remove_member returns residual owned-resource reach"
```

---

## Task 3: Thread the outcome through API + client + CLI

**Files:**
- Modify: `crates/temper-api/src/handlers/teams.rs` (`remove_member` at `:178`, utoipa at `:162`)
- Modify: `crates/temper-client/src/teams.rs` (`remove_member` at `:119`)
- Modify: `crates/temper-cli/src/commands/team.rs` (`remove_member_remote` at `:273`, `leave_remote` at `~:250`)
- Test: `crates/temper-api/tests/team_lifecycle_test.rs`

**Interfaces:**
- Consumes: `RemoveMemberOutcome` (Task 2); `team_service::remove_member` now returns it.
- Produces: API `200 + Json<RemoveMemberOutcome>`; client `remove_member(...) -> Result<RemoveMemberOutcome>`; CLI prints a handoff nudge when `count > 0`.

- [ ] **Step 1: Update the API handler** (`handlers/teams.rs:178`). Change return to `ApiResult<Json<RemoveMemberOutcome>>`, add `use axum::Json;` / `use temper_core::types::reassign::RemoveMemberOutcome;` if absent, and:

```rust
pub async fn remove_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, profile_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<RemoveMemberOutcome>> {
    let outcome = team_service::remove_member(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        profile_id,
    )
    .await?;
    Ok(Json(outcome))
}
```

Update the utoipa `responses` for it: replace `(status = 204, description = "Member removed")` with `(status = 200, description = "Member removed; residual owned-resource reach reported", body = RemoveMemberOutcome)`.

- [ ] **Step 2: Update the client** (`temper-client/src/teams.rs:119`). Switch from `send` to `send_json` and return the outcome:

```rust
    /// DELETE /api/teams/{id}/members/{profile_id} — remove a member (or self-leave).
    /// Returns the residual owned-resource reach the removed member retains.
    pub async fn remove_member(
        &self,
        team_id: Uuid,
        profile_id: Uuid,
    ) -> Result<RemoveMemberOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/teams/{team_id}/members/{profile_id}");
        let req = self.http.delete(&path);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }
```

Add `RemoveMemberOutcome` to the `temper_core::types::reassign::{...}` import at the top of the file.

- [ ] **Step 3: Update the CLI** (`commands/team.rs`). In `remove_member_remote`, replace the `output::success("Member removed.")` tail with:

```rust
    let outcome = client
        .teams()
        .remove_member(team_id, profile_id)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("Member removed.");
    print_residual_nudge(team, profile, &outcome.residual_owned);
    Ok(())
```

In `leave_remote`, after `output::success("You have left the team.")`, capture the returned outcome (bind the `remove_member` result instead of discarding it) and call `print_residual_nudge(team, &me.id.to_string(), &outcome.residual_owned)`. Add the helper to `team.rs`:

```rust
/// On a non-empty residual reach, nudge the admin toward the existing handoff.
fn print_residual_nudge(
    team: &str,
    from: &str,
    reach: &temper_core::types::reassign::ResidualOwnedReach,
) {
    if reach.count == 0 {
        return;
    }
    let ctxs: Vec<&str> = reach.contexts.iter().map(|c| c.context_ref.as_str()).collect();
    output::warning(format!(
        "{} still owns {} resource(s) in: {}. Hand them off with:\n  \
         temper team reassign {} --from {} --to <member-uuid>",
        from,
        reach.count,
        ctxs.join(", "),
        team,
        from,
    ));
}
```

- [ ] **Step 4: Write the API integration test** (append to `crates/temper-api/tests/team_lifecycle_test.rs`; mirror an existing removal case in that file for harness/auth setup)

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn remove_member_returns_residual_reach_body(pool: PgPool) {
    // Build a team with an owner + a leaver who owns a resource in a shared context,
    // then DELETE the member and assert a 200 body with count == 1.
    // (Follow the auth/harness pattern of the existing remove-member test in this file.)
    // ... arrange ...
    let resp = /* DELETE /api/teams/{team}/members/{leaver} */;
    assert_eq!(resp.status(), 200);
    let body: temper_core::types::reassign::RemoveMemberOutcome = resp.json().await.unwrap();
    assert_eq!(body.residual_owned.count, 1);
}
```

> ⚠️ **Plan/reality gap to close at implementation time:** open `team_lifecycle_test.rs` and copy its actual request-building + auth helpers (JWT fixture, app spawn) — do not invent a client. The body above is the assertion that matters; the arrange half must match the file's established pattern.

- [ ] **Step 5: Regenerate the API sqlx cache** (new test-target query)

Run: `cargo make prepare-api`

- [ ] **Step 6: Build + run the surface tests — expect PASS**

Run: `cargo nextest run -p temper-api --features test-db --test team_lifecycle_test`
Expected: PASS. Also confirm the workspace still builds: `cargo build -p temper-cli -p temper-client -p temper-api`.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api crates/temper-client crates/temper-cli
git commit -m "feat(offboarding): surface residual reach through API/client/CLI (204→200)"
```

---

## Task 4: Wire-contract regen + full gate

**Files:**
- Regen (staged): `openapi.json`, `clients/temper-rb/lib/temper/generated/**`, `clients/temper-ts/src/generated/schema.ts`, ts-rs bindings.

- [ ] **Step 1: Regenerate all router products**

Run: `cargo make openapi` (openapi.json + temper-rb gem + temper-ts schema; gem regen needs Docker) and `cargo make generate-ts-types`.

- [ ] **Step 2: Stage the regenerated artifacts** (the drift gates diff against git — freshly-regenerated-but-unstaged still reds)

```bash
git add openapi.json clients/temper-rb clients/temper-ts crates/temper-core/bindings 2>/dev/null; git add -A
```

- [ ] **Step 3: Run the full gate — expect green**

Run: `cargo make check`
Expected: PASS — including `openapi-check`, `openapi-ts-drift` (never skips), and (Docker present) `openapi-rb-drift`.

- [ ] **Step 4: Run the DB-backed suites for the touched crates**

Run: `cargo nextest run -p temper-services -p temper-api --features test-db -E 'test(remove_member) or test(reassign) or test(team_scoped_owned)'`
Expected: PASS.

- [ ] **Step 5: Commit the regen**

```bash
git commit -m "chore(offboarding): regenerate openapi + temper-rb + temper-ts for RemoveMemberOutcome"
```

- [ ] **Step 6 (optional e2e): extend `tests/e2e/tests/team_member_lifecycle_test.rs`** to drive `temper team remove-member` through the real CLI and assert the residual nudge appears on stderr/stdout when the removed member owns a scoped resource. Run with `cargo make test-e2e` (rebuild the CLI bin first: `cargo build -p temper-cli --bin temper`). Fold in only if the e2e harness makes ownership setup cheap; otherwise the service + API integration tests are the sufficient guard.

---

## Self-Review

**Spec coverage:**
- Component 1 (shared read) → Task 1. ✅
- Component 2 (`remove_member` returns outcome) → Task 2. ✅
- Component 3 (thread CLI + API; MCP untouched) → Task 3. ✅ (MCP: no task, by design.)
- Component 4 (regen + tests) → Task 2/3 tests + Task 4 regen. ✅
- Non-goals (no auto-reassign / no pre-check / UUID-only / no MCP) → honored; no task adds them. ✅
- Open questions: context-ref rendering → resolved in Task 1 (the `CASE owner_table` pattern from `context_service.rs:591-646`); `200`-vs-`204` skew → pinned by Task 3 Step 4 body-decode test + the client already discarding bodies. ✅

**Placeholder scan:** The only non-literal blocks are the two ⚠️ plan/reality-gap markers (Task 2 Step 2 helpers, Task 3 Step 4 arrange) — these are deliberate "verify the real helper names on disk" instructions, not unfilled logic; the assertions they guard are concrete.

**Type consistency:** `RemoveMemberOutcome.residual_owned: ResidualOwnedReach`; `ResidualOwnedReach.{count, contexts}`; `ResidualContext.{context_ref, count}`; `ScopedOwnedRow.{resource_id, context_id, context_ref}` — used identically in Tasks 1–3. `team_scoped_owned` and `remove_member -> ApiResult<RemoveMemberOutcome>` signatures match across producer/consumer. ✅

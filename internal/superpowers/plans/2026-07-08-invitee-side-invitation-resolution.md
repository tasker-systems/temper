# Invitee-Side Team Invitation Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an authenticated caller list the pending team invitations addressed to *them* (email-correlated, safely) over CLI + MCP + the API read behind them, and round out the MCP surface with accept/decline — so the invitation loop closes without an out-of-band token hand-off.

**Architecture:** A new service-direct read `invitation_service::list_for_profile` matches `kb_team_invitations.invited_email` against the caller's `kb_profile_auth_links` emails, guarded so an email mapping to more than one profile is discounted (Option B — no schema change, never leaks). It is surfaced as `GET /api/invitations/mine` on the **un-gated** router (so a gating-team invitee can discover invites pre-access), a `temper invitations` CLI command, and MCP tools. Accept/decline are unchanged (token-bearer), now also exposed over MCP.

**Tech Stack:** Rust workspace — temper-core (types, ts-rs), temper-services (sqlx service layer), temper-api (Axum + utoipa), temper-client (reqwest), temper-cli (clap), temper-mcp (rmcp). PostgreSQL + pgvector via sqlx compile-time macros.

## Global Constraints

- **Compile-time SQL:** production queries use `sqlx::query_as!`; after adding/altering any macro query, regenerate the cache: `cargo sqlx prepare --workspace -- --all-features` then `cargo make prepare-services`. `cargo make check` forces `SQLX_OFFLINE=true`, so it is the honest probe of the committed cache. `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.
- **Typed structs over inline JSON** — define types in temper-core; never `serde_json::json!()` for known-shape data.
- **Reads stay service-direct** on both API and MCP surfaces — no `DbBackend`, no operations command. This whole feature is a read (plus the two pre-existing token writes exposed over MCP).
- **Auth before any logic.** All new surfaces require authentication; `GET /api/invitations/mine` is un-gated on *system access* only.
- **No `TeamId` newtype** — team ids are raw `Uuid`. Only `ProfileId` (from `temper_core::types::ids`) is a newtype; deref via `*caller`.
- **Full surface parity is intended** — CLI + MCP + API all land in this plan.
- **Docker Postgres must be up** for db/e2e tests: `cargo make docker-up`.
- **Frequent commits** — one per task minimum.

---

## File Structure

- `crates/temper-core/src/types/invitation.rs` — **modify**: add `InviteeInvitation` struct.
- `crates/temper-services/src/services/invitation_service.rs` — **modify**: add `list_for_profile`.
- `crates/temper-api/src/handlers/invitations.rs` — **modify**: add `list_mine` handler.
- `crates/temper-api/src/routes.rs` — **modify**: add un-gated `/api/invitations/mine` route.
- `crates/temper-api/src/openapi.rs` — **modify**: register path + schema.
- `crates/temper-client/src/teams.rs` — **modify**: add `list_my_invitations`.
- `crates/temper-cli/src/cli.rs` — **modify**: add top-level `Commands::Invitations`.
- `crates/temper-cli/src/main.rs` — **modify**: dispatch arm.
- `crates/temper-cli/src/commands/invitations.rs` — **create**: `list_mine` command fn.
- `crates/temper-cli/src/commands/mod.rs` — **modify**: `pub mod invitations;`.
- `crates/temper-mcp/src/tools/invitations.rs` — **create**: three tool delegate fns + input structs.
- `crates/temper-mcp/src/tools/mod.rs` — **modify**: `pub mod invitations;`.
- `crates/temper-mcp/src/service.rs` — **modify**: three `#[tool]` methods.
- `docs/guides/teams.md` — **create**: human guide.
- `crates/temper-cli/skill-content/teams.md` — **create**: agent skill guidance.
- `crates/temper-cli/src/commands/skill.rs` — **modify**: wire `teams.md`.
- `crates/temper-cli/templates/skill.md` — **modify**: Supporting Files line.
- `tests/e2e/tests/` — **create/extend**: e2e for `temper invitations`.

---

### Task 1: Core type `InviteeInvitation`

**Files:**
- Modify: `crates/temper-core/src/types/invitation.rs`
- Test: same file (`#[cfg(test)]` mod) or `crates/temper-core/tests/` — inline unit test.

**Interfaces:**
- Produces: `temper_core::types::invitation::InviteeInvitation { id: Uuid, team_id: Uuid, team_slug: String, team_name: String, invited_email: String, invited_by_profile_id: Uuid, role: TeamRole, token: String, status: InvitationStatus, expires_at: DateTime<Utc>, created: DateTime<Utc> }`. Consumed by Tasks 2–5.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/temper-core/src/types/invitation.rs` (create a `#[cfg(test)] mod tests` if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invitee_invitation_serde_roundtrip() {
        let json = serde_json::json!({
            "id": "019f41f3-74ab-7ec0-8b0d-cb21662c51cb",
            "team_id": "019f25d6-e1a9-7360-8a35-6bdf8ef53940",
            "team_slug": "platform",
            "team_name": "Platform",
            "invited_email": "person@x.com",
            "invited_by_profile_id": "019d4add-f49d-7c43-a87d-dda470e5dd9c",
            "role": "member",
            "token": "abc123",
            "status": "pending",
            "expires_at": "2026-07-15T00:00:00Z",
            "created": "2026-07-08T00:00:00Z"
        });
        let inv: InviteeInvitation = serde_json::from_value(json).unwrap();
        assert_eq!(inv.team_slug, "platform");
        assert_eq!(inv.role, TeamRole::Member);
        assert_eq!(inv.status, InvitationStatus::Pending);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core invitee_invitation_serde_roundtrip`
Expected: FAIL — `cannot find type InviteeInvitation`.

- [ ] **Step 3: Add the struct**

Insert after the `TeamInvitation` struct (around line 49), mirroring its exact derive stack (no `mcp`/schemars — this is a return type, not a tool input):

```rust
/// A pending invitation resolved to the *invitee's* view — the `TeamInvitation`
/// fields plus the team's slug/name for display. Returned by
/// `GET /api/invitations/mine`; the caller is authorized to redeem these, so the
/// `token` is legitimately theirs to see.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "invitation.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, FromRow, serde::Serialize, serde::Deserialize)]
pub struct InviteeInvitation {
    pub id: Uuid,
    pub team_id: Uuid,
    pub team_slug: String,
    pub team_name: String,
    pub invited_email: String,
    pub invited_by_profile_id: Uuid,
    pub role: TeamRole,
    pub token: String,
    pub status: InvitationStatus,
    pub expires_at: DateTime<Utc>,
    pub created: DateTime<Utc>,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-core invitee_invitation_serde_roundtrip`
Expected: PASS.

- [ ] **Step 5: Regenerate TypeScript bindings**

Run: `cargo make generate-ts-types`
Expected: `packages/temper-ui/src/lib/types/invitation.ts` (or the ts-rs output dir) now contains `InviteeInvitation`. Stage the regenerated file.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/invitation.rs
git add $(git diff --name-only | rg '\.ts$' || true)
git commit -m "feat(core): add InviteeInvitation type for invitee-side invitation list"
```

---

### Task 2: Service `list_for_profile` (the Option B resolver)

**Files:**
- Modify: `crates/temper-services/src/services/invitation_service.rs`
- Test: same file, `#[cfg(test)] mod` using `#[sqlx::test]`. Ensure the file has `#![cfg(...)]` test-db gating consistent with the crate's other `#[sqlx::test]` files (check the top of `invitation_service.rs` / sibling test modules; if the crate gates sqlx tests behind `feature = "test-db"`, gate this `mod tests` the same way).

**Interfaces:**
- Consumes: `InviteeInvitation` (Task 1); `ProfileId`, `ApiResult` (already imported).
- Produces: `pub async fn list_for_profile(pool: &PgPool, caller: ProfileId) -> ApiResult<Vec<InviteeInvitation>>`. Consumed by Tasks 3 and 5.

- [ ] **Step 1: Write the failing integration test**

Add a `#[sqlx::test]` test. It seeds two profiles each with a verified-shaped auth-link email, a team owned by inviter, and invitations, then asserts the Option B behaviors. Use the crate's existing seed helpers if present (grep `invitation_service.rs` and sibling service tests for how they insert `kb_profiles` / `kb_team_members` / `kb_teams`); otherwise insert directly as below.

```rust
#[cfg(test)]
mod list_for_profile_tests {
    use super::*;
    use sqlx::PgPool;
    use uuid::Uuid;

    // Minimal seed helpers — insert a profile + one auth-link email, and a team.
    async fn seed_profile(pool: &PgPool, handle: &str, email: Option<&str>) -> Uuid {
        let pid = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
             VALUES ($1, $2, $2, $3, '{}')",
            pid, handle, email as Option<&str>,
        ).execute(pool).await.unwrap();
        sqlx::query!(
            "INSERT INTO kb_profile_auth_links \
               (id, profile_id, auth_provider, auth_provider_user_id, email, is_default) \
             VALUES ($1, $2, 'test', $3, $4, true)",
            Uuid::now_v7(), pid, handle, email as Option<&str>,
        ).execute(pool).await.unwrap();
        pid
    }

    async fn seed_team(pool: &PgPool, slug: &str, owner: Uuid) -> Uuid {
        let tid = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_teams (id, slug, name, is_active) VALUES ($1, $2, $2, true)",
            tid, slug,
        ).execute(pool).await.unwrap();
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')",
            tid, owner,
        ).execute(pool).await.unwrap();
        tid
    }

    async fn seed_invite(pool: &PgPool, team: Uuid, email: &str, by: Uuid) -> String {
        let token = format!("tok-{}", Uuid::now_v7());
        sqlx::query!(
            "INSERT INTO kb_team_invitations \
               (id, team_id, invited_email, invited_by_profile_id, role, token, status, expires_at) \
             VALUES ($1, $2, $3, $4, 'member', $5, 'pending', now() + interval '7 days')",
            Uuid::now_v7(), team, email, by, token,
        ).execute(pool).await.unwrap();
        token
    }

    #[sqlx::test]
    async fn resolves_unambiguous_email_only(pool: PgPool) {
        let inviter = seed_profile(&pool, "inviter", Some("owner@x.com")).await;
        let invitee = seed_profile(&pool, "invitee", Some("invitee@x.com")).await;
        let team = seed_team(&pool, "platform", inviter).await;
        seed_invite(&pool, team, "invitee@x.com", inviter).await;

        let got = list_for_profile(&pool, ProfileId::from(invitee)).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].team_slug, "platform");
        assert_eq!(got[0].invited_email, "invitee@x.com");
    }

    #[sqlx::test]
    async fn case_insensitive_match(pool: PgPool) {
        let inviter = seed_profile(&pool, "inviter", Some("owner@x.com")).await;
        let invitee = seed_profile(&pool, "invitee", Some("invitee@x.com")).await;
        let team = seed_team(&pool, "platform", inviter).await;
        seed_invite(&pool, team, "Invitee@X.com", inviter).await; // mixed case

        let got = list_for_profile(&pool, ProfileId::from(invitee)).await.unwrap();
        assert_eq!(got.len(), 1);
    }

    #[sqlx::test]
    async fn discounts_ambiguous_email(pool: PgPool) {
        // Two profiles both hold invitee@x.com — ambiguous, must be discounted for the caller.
        let inviter = seed_profile(&pool, "inviter", Some("owner@x.com")).await;
        let invitee = seed_profile(&pool, "invitee", Some("dup@x.com")).await;
        let _other = seed_profile(&pool, "other", Some("dup@x.com")).await;
        let team = seed_team(&pool, "platform", inviter).await;
        seed_invite(&pool, team, "dup@x.com", inviter).await;

        let got = list_for_profile(&pool, ProfileId::from(invitee)).await.unwrap();
        assert!(got.is_empty(), "ambiguous email must not resolve");
    }

    #[sqlx::test]
    async fn excludes_declined_expired_and_softdeleted_team(pool: PgPool) {
        let inviter = seed_profile(&pool, "inviter", Some("owner@x.com")).await;
        let invitee = seed_profile(&pool, "invitee", Some("invitee@x.com")).await;
        let team = seed_team(&pool, "platform", inviter).await;

        // declined
        sqlx::query!(
            "INSERT INTO kb_team_invitations \
               (id, team_id, invited_email, invited_by_profile_id, role, token, status, expires_at) \
             VALUES ($1, $2, 'invitee@x.com', $3, 'member', $4, 'declined', now() + interval '7 days')",
            Uuid::now_v7(), team, inviter, format!("d-{}", Uuid::now_v7()),
        ).execute(&pool).await.unwrap();
        // expired
        sqlx::query!(
            "INSERT INTO kb_team_invitations \
               (id, team_id, invited_email, invited_by_profile_id, role, token, status, expires_at) \
             VALUES ($1, $2, 'invitee@x.com', $3, 'member', $4, 'pending', now() - interval '1 day')",
            Uuid::now_v7(), team, inviter, format!("e-{}", Uuid::now_v7()),
        ).execute(&pool).await.unwrap();
        // pending on a soft-deleted team
        let dead = seed_team(&pool, "dead", inviter).await;
        sqlx::query!("UPDATE kb_teams SET is_active = false WHERE id = $1", dead)
            .execute(&pool).await.unwrap();
        seed_invite(&pool, dead, "invitee@x.com", inviter).await;

        let got = list_for_profile(&pool, ProfileId::from(invitee)).await.unwrap();
        assert!(got.is_empty());
    }

    #[sqlx::test]
    async fn null_email_caller_gets_empty(pool: PgPool) {
        let agent = seed_profile(&pool, "agent", None).await;
        let got = list_for_profile(&pool, ProfileId::from(agent)).await.unwrap();
        assert!(got.is_empty());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-services --features test-db list_for_profile_tests`
Expected: FAIL — `cannot find function list_for_profile`.

- [ ] **Step 3: Add the import and the function**

At the top of `invitation_service.rs`, extend the invitation import:

```rust
use temper_core::types::invitation::{
    AcceptInvitationResponse, InvitationStatus, InviteeInvitation, TeamInvitation,
};
```

Add the function (mirrors `list_invitations` but matches on the caller's auth-link emails with the Option B uniqueness guard):

```rust
/// List a caller's own pending, non-expired invitations. Resolution matches
/// `invited_email` (case-insensitively) against the caller's `kb_profile_auth_links`
/// emails, guarded so an email that maps to more than one profile is discounted
/// (Option B — never leaks another profile's invite; falls back to token hand-off).
/// Auth: any authenticated caller (their own invitations only).
pub async fn list_for_profile(
    pool: &PgPool,
    caller: ProfileId,
) -> ApiResult<Vec<InviteeInvitation>> {
    let rows = sqlx::query_as!(
        InviteeInvitation,
        r#"
        SELECT i.id, i.team_id, t.slug AS team_slug, t.name AS team_name,
               i.invited_email, i.invited_by_profile_id,
               i.role AS "role: TeamRole", i.token,
               i.status AS "status: InvitationStatus", i.expires_at, i.created
          FROM kb_team_invitations i
          JOIN kb_teams t ON t.id = i.team_id
         WHERE i.status = 'pending'
           AND i.expires_at > now()
           AND t.is_active
           AND lower(i.invited_email) IN (
                 SELECT lower(al.email)
                   FROM kb_profile_auth_links al
                  WHERE al.profile_id = $1
                    AND al.email IS NOT NULL
                    AND (SELECT COUNT(DISTINCT al2.profile_id)
                           FROM kb_profile_auth_links al2
                          WHERE lower(al2.email) = lower(al.email)) = 1
               )
         ORDER BY i.created DESC
        "#,
        *caller,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
```

- [ ] **Step 4: Regenerate the sqlx cache, then run the tests**

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo nextest run -p temper-services --features test-db list_for_profile_tests
```
Expected: all five tests PASS.

- [ ] **Step 5: Guard the full service suite + offline check**

```bash
cargo nextest run -p temper-services --features test-db
cargo make check
```
Expected: green (offline check proves the committed `.sqlx` cache is complete).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-services/src/services/invitation_service.rs .sqlx crates/temper-services/.sqlx
git commit -m "feat(services): list_for_profile — invitee-side invitation resolution (Option B guard)"
```

---

### Task 3: API handler + un-gated route + OpenAPI

**Files:**
- Modify: `crates/temper-api/src/handlers/invitations.rs`
- Modify: `crates/temper-api/src/routes.rs`
- Modify: `crates/temper-api/src/openapi.rs`

**Interfaces:**
- Consumes: `invitation_service::list_for_profile` (Task 2), `InviteeInvitation` (Task 1).
- Produces: `GET /api/invitations/mine` returning `Json<Vec<InviteeInvitation>>`.

- [ ] **Step 1: Write the failing OpenAPI-presence test**

The existing `#[cfg(test)] mod tests` in `openapi.rs` (around lines 208–262) asserts each invitation path string appears in the generated spec JSON. Add an assertion next to the existing invitation-path asserts (~line 241):

```rust
    assert!(json.contains("/api/invitations/mine"), "list_mine path missing from OpenAPI");
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db --test <the openapi test target>` — or if the test is a lib unit test: `cargo nextest run -p temper-api openapi`
Expected: FAIL — spec does not contain `/api/invitations/mine`.

- [ ] **Step 3: Add the handler**

In `crates/temper-api/src/handlers/invitations.rs`, add the import and handler. Extend the invitation import to include `InviteeInvitation`:

```rust
use temper_core::types::invitation::{
    AcceptInvitationResponse, CreateInvitationRequest, InviteeInvitation, TeamInvitation,
};
```

```rust
#[utoipa::path(
    get,
    path = "/api/invitations/mine",
    tag = "Invitations",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The caller's pending invitations", body = Vec<InviteeInvitation>),
    )
)]
pub async fn list_mine(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<InviteeInvitation>>> {
    invitation_service::list_for_profile(&state.pool, ProfileId::from(auth.0.profile.id))
        .await
        .map(Json)
}
```

- [ ] **Step 4: Register the un-gated route**

In `crates/temper-api/src/routes.rs`, inside the `auth_only` router block (next to accept/decline, around lines 39–46), add:

```rust
        .route(
            "/api/invitations/mine",
            get(handlers::invitations::list_mine),
        )
```
(`get` is already imported at line 16.)

- [ ] **Step 5: Register in OpenAPI**

In `crates/temper-api/src/openapi.rs`, add to the `paths(...)` macro (near line 71):

```rust
        crate::handlers::invitations::list_mine,
```
and to `components(schemas(...))` (near line 159):

```rust
        temper_core::types::invitation::InviteeInvitation,
```

- [ ] **Step 6: Run the test + build**

```bash
cargo nextest run -p temper-api --features test-db openapi
cargo build -p temper-api
```
Expected: PASS + clean build.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/handlers/invitations.rs crates/temper-api/src/routes.rs crates/temper-api/src/openapi.rs
git commit -m "feat(api): GET /api/invitations/mine (un-gated invitee invitation list)"
```

---

### Task 4: Client method + CLI `temper invitations` + e2e

**Files:**
- Modify: `crates/temper-client/src/teams.rs`
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/main.rs`
- Create: `crates/temper-cli/src/commands/invitations.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`
- Create/extend: `tests/e2e/tests/invitee_invitations_test.rs`

**Interfaces:**
- Consumes: `GET /api/invitations/mine` (Task 3), `InviteeInvitation` (Task 1).
- Produces: `TeamsClient::list_my_invitations() -> Result<Vec<InviteeInvitation>>`; `temper invitations` command.

- [ ] **Step 1: Add the client method**

In `crates/temper-client/src/teams.rs`, extend the invitation import to add `InviteeInvitation`, then add (mirroring `list_invitations`, no path param):

```rust
    /// GET /api/invitations/mine — the caller's own pending invitations.
    pub async fn list_my_invitations(&self) -> Result<Vec<InviteeInvitation>> {
        let token = self.http.resolve_token()?;
        let path = "/api/invitations/mine";
        let req = self.http.get(path);
        self.http
            .send_json(&Method::GET, path, req, Some(&token))
            .await
    }
```

- [ ] **Step 2: Add the CLI command fn**

Create `crates/temper-cli/src/commands/invitations.rs`:

```rust
//! Invitee-side invitation commands: list the pending invitations addressed to
//! the authenticated caller (across all teams), resolved by email correlation.

use crate::error::Result;

/// List the caller's own pending team invitations.
pub async fn list_mine(
    client: &temper_client::TemperClient,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let invitations = client
        .teams()
        .list_my_invitations()
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&invitations, fmt)?);
    Ok(())
}
```

Add to `crates/temper-cli/src/commands/mod.rs`:

```rust
pub mod invitations;
```

- [ ] **Step 3: Add the clap variant + dispatch**

In `crates/temper-cli/src/cli.rs`, add a top-level unit variant to the `Commands` enum (near the other top-level commands):

```rust
    /// List the pending team invitations addressed to you
    Invitations,
```

In `crates/temper-cli/src/main.rs`, add a dispatch arm (mirror the `Commands::Warmup`/`TeamAction::List` client pattern):

```rust
        Commands::Invitations => temper_cli::actions::runtime::with_client(|client| {
            Box::pin(async move {
                temper_cli::commands::invitations::list_mine(client, output_format).await
            })
        }),
```

- [ ] **Step 4: Build the CLI + confirm the command exists**

```bash
cargo build -p temper-cli --bin temper
./target/debug/temper invitations --help
```
Expected: clean build; help shows the `invitations` command.

- [ ] **Step 5: Write the e2e test (production-caller level)**

Create `tests/e2e/tests/invitee_invitations_test.rs`. **Mirror the existing invitation e2e** (grep `tests/e2e/tests` for the PR #251 invitation test — likely `team_invitations_*` — and copy its harness setup: spawning the app, JWKS fixtures, provisioning two profiles with distinct auth emails, minting bearer tokens). The test must:

1. Provision profile **A** (inviter, email `owner@x.com`) and create a team owned by A.
2. As A, `POST /api/teams/{id}/invite` with `invited_email = "invitee@x.com"`, role `member`.
3. Provision profile **B** with auth email `invitee@x.com` (so provisioning correlates).
4. Drive the **CLI** as B: run the built `temper invitations` against the spawned server (the e2e harness runs the real `temper` binary — follow the sibling test's CLI-invocation helper), OR call `client.teams().list_my_invitations()` through `temper-client` if that is the sibling test's convention.
5. Assert the returned list contains exactly one invitation, `team_slug` matches, `token` is present.

Assertion skeleton (adapt the harness lines to match the sibling test):

```rust
let invites = client_b.teams().list_my_invitations().await.unwrap();
assert_eq!(invites.len(), 1);
assert_eq!(invites[0].invited_email, "invitee@x.com");
assert!(!invites[0].token.is_empty());
```

- [ ] **Step 6: Rebuild the CLI bin (e2e uses a stale bin otherwise), then run e2e**

```bash
cargo build -p temper-cli --bin temper
cargo make test-e2e
```
Expected: the new test passes. **Do not run concurrent `test-e2e` invocations.** If the e2e added a macro query, run `cargo make prepare-e2e` and re-check `cargo make check`.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-client/src/teams.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/src/commands/ tests/e2e/tests/invitee_invitations_test.rs
git add tests/e2e/.sqlx 2>/dev/null || true
git commit -m "feat(cli): temper invitations — list your own pending invites (+ e2e)"
```

---

### Task 5: MCP tools — list_my_invitations, accept, decline

**Files:**
- Create: `crates/temper-mcp/src/tools/invitations.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`
- Modify: `crates/temper-mcp/src/service.rs`

**Interfaces:**
- Consumes: `invitation_service::{list_for_profile, accept_invitation, decline_invitation}`; `InviteeInvitation`.
- Produces: three MCP tools. Delegates call the service directly via `svc.api_state.pool` (confirmed accessor — `service.rs:34`, mirrors `tools::contexts::list_contexts`).

- [ ] **Step 1: Write the failing input-struct test**

Create `crates/temper-mcp/src/tools/invitations.rs` with a deserialize test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_input_deserializes() {
        let input: AcceptInvitationInput =
            serde_json::from_value(serde_json::json!({ "token": "abc" })).unwrap();
        assert_eq!(input.token, "abc");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-mcp accept_input_deserializes`
Expected: FAIL — module/type not found.

- [ ] **Step 3: Write the delegate fns + input structs**

Fill in `crates/temper-mcp/src/tools/invitations.rs`. Match the caller-resolution pattern from `tools/profiles.rs` (`svc.require_profile().await?`), the service-error mapping from `tools/contexts.rs::list_contexts`, and the input-struct derives from `tools/contexts.rs` (`#[derive(Debug, Deserialize, JsonSchema)]`).

```rust
//! Invitation tools — list your own pending invitations, and accept/decline by token.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::service::TemperMcpService;
use temper_core::types::ids::ProfileId;
use temper_services::services::invitation_service;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AcceptInvitationInput {
    /// The invitation token to redeem.
    pub token: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeclineInvitationInput {
    /// The invitation token to decline.
    pub token: String,
}

pub async fn list_my_invitations(
    svc: &TemperMcpService,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let invites =
        invitation_service::list_for_profile(&svc.api_state.pool, ProfileId::from(profile.id))
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to list invitations: {e}"), None)
            })?;
    let text = serde_json::to_string_pretty(&invites).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
}

pub async fn accept_invitation(
    svc: &TemperMcpService,
    input: AcceptInvitationInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let resp = invitation_service::accept_invitation(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        &input.token,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to accept invitation: {e}"), None))?;
    let text = serde_json::to_string_pretty(&resp).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
}

pub async fn decline_invitation(
    svc: &TemperMcpService,
    input: DeclineInvitationInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    invitation_service::decline_invitation(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        &input.token,
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to decline invitation: {e}"), None)
    })?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        "Invitation declined.".to_string(),
    )]))
}
```

The error-mapping convention above is the **confirmed** house pattern — see `tools::contexts::list_contexts`, which maps service errors with `.map_err(|e| rmcp::ErrorData::internal_error(format!("…: {e}"), None))?` (there is no blanket `From<ApiError>` impl, so bare `?` on a service call will NOT compile). Pool accessor `svc.api_state.pool` is confirmed (`service.rs:34`, field `api_state: AppState`).

- [ ] **Step 4: Register the module + tools**

In `crates/temper-mcp/src/tools/mod.rs` add (alphabetical): `pub mod invitations;`.

In `crates/temper-mcp/src/service.rs`, add three methods inside the `#[tool_router] impl TemperMcpService` block (before its closing brace ~line 589). `list_my_invitations` mirrors `get_profile` (no input); the other two mirror `get_context` (typed `Parameters`):

```rust
    #[tool(description = "List the pending team invitations addressed to you (across all teams).")]
    async fn list_my_invitations(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invitations::list_my_invitations(self).await
    }

    #[tool(description = "Accept a team invitation by its token.")]
    async fn accept_invitation(
        &self,
        Parameters(input): Parameters<tools::invitations::AcceptInvitationInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invitations::accept_invitation(self, input).await
    }

    #[tool(description = "Decline a team invitation by its token.")]
    async fn decline_invitation(
        &self,
        Parameters(input): Parameters<tools::invitations::DeclineInvitationInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invitations::decline_invitation(self, input).await
    }
```

- [ ] **Step 5: Run the test + build**

```bash
cargo nextest run -p temper-mcp accept_input_deserializes
cargo build -p temper-mcp
```
Expected: PASS + clean build (the `#[tool_router]` macro auto-registers the three tools for listing/dispatch — no manual table).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-mcp/src/tools/invitations.rs crates/temper-mcp/src/tools/mod.rs crates/temper-mcp/src/service.rs
git commit -m "feat(mcp): first invitation tools — list_my_invitations, accept, decline"
```

---

### Task 6: Docs — human guide + generated-skill guidance

**Files:**
- Create: `docs/guides/teams.md`
- Create: `crates/temper-cli/skill-content/teams.md`
- Modify: `crates/temper-cli/src/commands/skill.rs`
- Modify: `crates/temper-cli/templates/skill.md`

**Interfaces:** none (docs + skill wiring).

- [ ] **Step 1: Write the human guide**

Create `docs/guides/teams.md`. Cover, truthfully (grounded in this feature): what a team is and the four roles (owner/maintainer/member/watcher); `temper team create`; adding an existing member by profile UUID (`team add-member`); the **invite → self-serve resolve → join** loop — invite by email (`team invite --team <id> --email <e> --role <r>`), that **no email is sent** (email is a correlator; OAuth/SAML self-serve join provisions the profile), the invitee runs **`temper invitations`** to see and copy their token, then `temper team join <token>`; the ambiguous-email fallback (token hand-off) in one honest sentence; team-owned contexts (`context share`/`unshare`, `resource grant --to-team`); offboarding via `team reassign`; soft-delete (`team delete`). Keep it partner-register prose (see the other files in `docs/guides/`).

- [ ] **Step 2: Write the skill-content guidance (self-contained)**

Create `crates/temper-cli/skill-content/teams.md` — terse, agent-oriented, self-contained (skill consumers do not have the repo). Same content spine as the guide but action-first: when to reach for teams, the exact command sequence for invite/list-mine/join, roles table, and the "email is a correlator, `temper invitations` is how the invitee pulls their token" fact. Do not reference repo paths.

- [ ] **Step 3: Wire the skill content into the generator**

In `crates/temper-cli/src/commands/skill.rs`:

Add the static (near line 18, beside `COGNITIVE_MAPS_MD`):
```rust
static TEAMS_MD: &str = include_str!("../../skill-content/teams.md");
```
Add to the `files` map in `generate_skill_files_with_hash` (near lines 510–513):
```rust
    files.insert("teams.md".to_string(), TEAMS_MD.to_string());
```
Add to the `check_expected_files` array (near lines 390–403):
```rust
        "teams.md",
```

- [ ] **Step 4: Add the router line to the template**

In `crates/temper-cli/templates/skill.md`, in the "Supporting Files" list (lines 22–27), add:
```markdown
- `teams.md` — Working with teams: create, invite (email as correlator), list your invitations, join, roles, offboarding
```

- [ ] **Step 5: Update the generator test + run it**

In `skill.rs`'s `test_generate_skill_files_contains_expected_keys` (~lines 601–619), add:
```rust
        assert!(files.contains_key("teams.md"));
```
Run: `cargo nextest run -p temper-cli test_generate_skill_files_contains_expected_keys`
Expected: PASS.

- [ ] **Step 6: Rebuild the binary and reinstall the skill (REQUIRED — content is compiled in)**

```bash
cargo install --path crates/temper-cli
temper skill install
```
Then **verify the content actually shipped**:
```bash
cat ~/.claude/skills/temper/teams.md | head -20
```
Expected: the file exists and shows your guidance. (Remember: `temper skill check` will NOT flag stale binary content — the eyeball is the verification.)

- [ ] **Step 7: Commit**

```bash
git add docs/guides/teams.md crates/temper-cli/skill-content/teams.md crates/temper-cli/src/commands/skill.rs crates/temper-cli/templates/skill.md
git commit -m "docs(teams): human guide + generated-skill teams guidance"
```

---

### Task 7: File the deferred follow-up + final verification

**Files:** none (temper task creation + full-suite guard).

- [ ] **Step 1: File the Option A + account-merge follow-up task**

```bash
cat <<'EOF' | temper resource create --type task \
  --title "Persist email_verified on auth-links + account-merge (Option A)" \
  --context @me/temper --mode build --effort medium
# Persist email_verified + account-merge

Robust end-state for invitation resolution, deferred from the invitee-side
resolution round (task 019f41f3, which shipped Option B — a query-time
uniqueness guard, no schema change).

- Add `email_verified BOOLEAN NOT NULL DEFAULT false` to kb_profile_auth_links
  (additive, main-safe); populate from `claims.email_verified` at provisioning
  (create_new_profile_and_link + create_link_for_existing_profile).
- Decide backfill: existing rows default false → invites won't auto-resolve until
  next sign-in; or backfill more liberally.
- Switch the resolver to match verified-only.
- Add an account-merge story to collapse pre-existing unverified duplicate profiles
  (the residual ambiguity Option B discounts).
EOF
```
Then edge it under the goal (mirror the goal→task link):
```bash
# copy the new task's ref from the create output / `temper resource list --type task --context @me/temper`
temper edge assert --kind leads-to --polarity forward --label advances \
  teams-in-temper-usable-multi-user-collaboration-surface-019f25d6-e1a9-7360-8a35-6bdf8ef53940 \
  <new-task-ref>
```

- [ ] **Step 2: Full workspace guard (branch-end)**

```bash
cargo make check
cargo make test
cargo make test-db
```
Expected: all green. (E2E already run in Task 4; do not run concurrent e2e.)

- [ ] **Step 3: Mark the implementation task done + finish the branch**

Update the task stage and open the PR:
```bash
temper resource update invitee-side-team-invitation-resolution-cli-mcp-019f41f3-74ab-7ec0-8b0d-cb21662c51cb --stage done
```
Then follow superpowers:finishing-a-development-branch for the PR.

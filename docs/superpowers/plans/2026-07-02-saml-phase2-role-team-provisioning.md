# SAML Phase 2 — role + team provisioning — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reconcile Temper team memberships from SAML-asserted groups at login — adding, updating, and revoking only IdP-managed memberships, never touching native ones.

**Architecture:** Seam C. The temper-cloud AS (TS) extracts asserted groups from the SAML profile and calls a shared-secret-gated internal endpoint on temper-api (Rust) before minting the token. That endpoint resolves the profile, looks up an operator-maintained `(idp_key, group) → (team, role)` mapping, and reconciles `kb_team_members` rows where `source='idp'` in a Rust service (native-wins-skip, fail-open).

**Tech Stack:** Rust (axum, sqlx, temper-services/temper-api/temper-core), TypeScript (temper-cloud serverless, Neon serverless Postgres, Vitest, pino), sqlx migrations, ts-rs.

**Design spec:** [docs/superpowers/specs/2026-07-01-saml-phase2-role-team-provisioning-design.md](../specs/2026-07-01-saml-phase2-role-team-provisioning-design.md)

## Global Constraints

- **Additive-only on `main`** — the migration only adds an enum, a column (with default), and a table. No destructive DDL.
- **Provenance is load-bearing** — reconcile touches ONLY `source='idp'` rows. `source='native'` rows are never inserted-over, updated, or deleted by reconcile. Native-wins-skip on `(team, user)` overlap.
- **Identity provider string comes from server config** — the reconcile endpoint builds `AuthClaims.provider` from `state.config.auth_provider_name` (identical to `middleware/auth.rs`), NOT from the AS payload, so it resolves the *same* profile the minted token later resolves to. On a SAML instance `auth_provider_name` == `saml:<idp_key>`.
- **Fail-open** — a reconcile failure (network, 401, 5xx, DB error) logs and lets login proceed; it never blocks authn.
- **Typed structs over inline JSON** — the AS→API wire type is `ReconcileRequest`, defined once in `temper-core` with `ts-rs` derives; the TS side imports the generated type. No `serde_json::json!()`, no hand-written TS mirror.
- **Compile-time SQL** — new queries use `sqlx::query!`/`query_as!`. After adding them: `cargo sqlx prepare --workspace -- --all-features` then `cargo make prepare-services` (and `cargo make prepare-e2e` if e2e gains macro queries).
- **No unbounded channels; all public types derive `Debug`; `#[expect(..., reason=...)]` over `#[allow]`.**
- **DATABASE_URL for local Rust tests:** `postgresql://temper:temper@localhost:5437/temper_development`.

---

### Task 1: Migration — provenance column, mapping table, groups_attr

**Files:**
- Create: `migrations/20260702000001_saml_group_provisioning.sql`

**Interfaces:**
- Produces: `team_member_source` enum (`native|idp`); `kb_team_members.source` column (default `native`); `kb_saml_group_mappings(idp_key, group_value, team_id, role, created)` PK `(idp_key, group_value, team_id)`; `kb_saml_idp.groups_attr TEXT NULL`.

- [ ] **Step 1: Write the migration**

Create `migrations/20260702000001_saml_group_provisioning.sql`:

```sql
-- SAML Phase 2: role + team provisioning (reconcile-on-login).
-- Additive-only. See docs/superpowers/specs/2026-07-01-saml-phase2-role-team-provisioning-design.md §5.

-- 1. Provenance on team membership. Existing rows are native by definition (the DEFAULT backfills them).
CREATE TYPE team_member_source AS ENUM ('native', 'idp');
ALTER TABLE kb_team_members
    ADD COLUMN source team_member_source NOT NULL DEFAULT 'native';

-- 2. The group -> (team, role) mapping, per-IdP. Operator-maintained via SQL in v1.
CREATE TABLE kb_saml_group_mappings (
    idp_key      TEXT      NOT NULL REFERENCES kb_saml_idp(idp_key) ON DELETE CASCADE,
    group_value  TEXT      NOT NULL,
    team_id      UUID      NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    role         team_role NOT NULL,
    created      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (idp_key, group_value, team_id)
);
CREATE INDEX idx_kb_saml_group_mappings_idp ON kb_saml_group_mappings(idp_key);

-- 3. Which assertion attribute carries the group list. NULL => pure authn (no reconcile).
ALTER TABLE kb_saml_idp ADD COLUMN groups_attr TEXT;
```

- [ ] **Step 2: Apply the migration to the dev DB**

Run:
```bash
cargo make docker-up
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run
```
Expected: the new migration applies clean, listed after `20260701000006`.

- [ ] **Step 3: Verify the schema objects exist**

Run:
```bash
PGPASSWORD=temper psql -h localhost -p 5437 -U temper -d temper_development -c "\d kb_team_members" -c "\d kb_saml_group_mappings" -c "SELECT column_name FROM information_schema.columns WHERE table_name='kb_saml_idp' AND column_name='groups_attr';"
```
Expected: `kb_team_members` shows a `source` column of type `team_member_source`; `kb_saml_group_mappings` exists with the four columns + PK; `groups_attr` row returned.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260702000001_saml_group_provisioning.sql
git commit -m "feat(db): SAML group provisioning schema (provenance + mapping table)"
```

---

### Task 2: `ReconcileRequest` boundary type in temper-core (ts-rs)

**Files:**
- Modify: `crates/temper-core/src/types/auth.rs` (append the struct)
- Modify: `crates/temper-core/src/types/mod.rs` or the auth re-export (only if `ReconcileRequest` needs adding to the public re-export list — check how `AuthClaims` is exported and mirror it)
- Test: `crates/temper-core/tests/` (ts-rs export is verified by the generate step, not a unit test)

**Interfaces:**
- Produces: `temper_core::types::ReconcileRequest { provider: Option<String>, external_user_id: String, email: String, email_verified: Option<bool>, idp_key: String, groups: Vec<String> }`. `provider` is advisory only (the API ignores it for identity; see Global Constraints) — included so the wire shape is self-describing and future multi-IdP can use it. Generated TS type: `packages/temper-cloud/src/generated/ReconcileRequest.ts` (match the existing ts-rs output dir — verify with the next step).

- [ ] **Step 1: Find where ts-rs types are generated and their output path**

Run:
```bash
grep -rn "ts(export" crates/temper-core/src/types/team.rs | head -1
grep -rn "TS_RS_EXPORT_DIR\|export_to\|generate-ts-types" Makefile.toml crates/temper-core 2>/dev/null | head
```
Expected: shows the `#[ts(export, export_to = "...")]` pattern and the output directory the workspace uses. Use the SAME `export_to` target as `TeamRole` so the generated file lands beside the others.

- [ ] **Step 2: Add the struct**

Append to `crates/temper-core/src/types/auth.rs` (mirror the derive/cfg_attr pattern used by `AddMemberRequest` in `team.rs` — adjust `export_to` to the path found in Step 1):

```rust
/// Wire payload for the internal SAML membership-reconcile call (AS -> temper-api).
///
/// `provider` is advisory: the API derives the authoritative provider from its own
/// config (`auth_provider_name`) so the resolved profile matches the one the minted
/// token resolves to. `idp_key` selects the `kb_saml_group_mappings` rows to apply.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "ReconcileRequest.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconcileRequest {
    /// Advisory provider label (e.g. "saml:acme-okta"); the API ignores it for identity.
    pub provider: Option<String>,
    /// Stable NameID — the same value minted as the token `sub`.
    pub external_user_id: String,
    /// Email attribute from the assertion.
    pub email: String,
    /// Verified flag (a signed trusted-IdP assertion is treated as verified).
    pub email_verified: Option<bool>,
    /// Which IdP's group mappings to apply.
    pub idp_key: String,
    /// Asserted group values (possibly empty).
    pub groups: Vec<String>,
}
```

Ensure `use serde::{Serialize, Deserialize};` is present at the top of `auth.rs` (add it if `AuthClaims` didn't need it — it currently does not derive Serialize, so this import is likely new).

- [ ] **Step 3: Re-export it (match how `AuthClaims` is surfaced)**

Run:
```bash
grep -rn "AuthClaims" crates/temper-core/src/types/mod.rs crates/temper-core/src/lib.rs 2>/dev/null | head
```
If `AuthClaims` is re-exported via a `pub use ...auth::{...}` list, add `ReconcileRequest` to it. If the module is glob-exported, nothing to do.

- [ ] **Step 4: Generate the TS type and verify it compiles**

Run:
```bash
cargo make generate-ts-types
git status --short packages/temper-cloud
```
Expected: a new/updated `ReconcileRequest.ts` under the generated types dir. Then:
```bash
cd packages/temper-cloud && bun run typecheck && cd -
```
Expected: PASS.

- [ ] **Step 5: Verify the Rust workspace still checks**

Run: `SQLX_OFFLINE=true cargo check -p temper-core --all-features`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core packages/temper-cloud
git commit -m "feat(core): ReconcileRequest wire type (ts-rs) for SAML membership reconcile"
```

---

### Task 3: `saml_provisioning_service` — reconcile logic (Rust)

**Files:**
- Create: `crates/temper-services/src/services/saml_provisioning_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs` (add `pub mod saml_provisioning_service;`)
- Test: unit tests inline (`#[cfg(test)]`) for the pure helpers; integration test `crates/temper-services/tests/saml_provisioning_test.rs` (feature `test-db`)

**Interfaces:**
- Consumes: `kb_saml_group_mappings`, `kb_team_members` (Task 1); `temper_core::types::TeamRole`.
- Produces: `pub async fn reconcile_idp_memberships(pool: &PgPool, profile_id: Uuid, idp_key: &str, groups: &[String]) -> ApiResult<ReconcileOutcome>` where `pub struct ReconcileOutcome { pub added: usize, pub updated: usize, pub revoked: usize, pub skipped_native: usize }`. Pure helper `fn max_role(a: TeamRole, b: TeamRole) -> TeamRole`.

- [ ] **Step 1: Write the failing unit test for `max_role`**

Create `crates/temper-services/src/services/saml_provisioning_service.rs` with just the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::TeamRole;

    #[test]
    fn max_role_picks_the_stronger_role() {
        assert_eq!(max_role(TeamRole::Member, TeamRole::Maintainer), TeamRole::Maintainer);
        assert_eq!(max_role(TeamRole::Owner, TeamRole::Maintainer), TeamRole::Owner);
        assert_eq!(max_role(TeamRole::Watcher, TeamRole::Member), TeamRole::Member);
        assert_eq!(max_role(TeamRole::Owner, TeamRole::Owner), TeamRole::Owner);
    }
}
```

Add `pub mod saml_provisioning_service;` to `crates/temper-services/src/services/mod.rs`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `SQLX_OFFLINE=true cargo test -p temper-services --lib saml_provisioning_service 2>&1 | head -20`
Expected: FAIL — `max_role` not found.

- [ ] **Step 3: Implement `max_role` and the rank helper**

Prepend to the file (above the test module):

```rust
//! SAML-driven team-membership reconciliation (Phase 2). Applies an operator-maintained
//! `(idp_key, group) -> (team, role)` mapping to `kb_team_members` rows tagged `source='idp'`,
//! leaving `source='native'` rows untouched (native-wins-skip). See the Phase 2 design spec.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::TeamRole;

use crate::error::ApiResult;

/// Numeric rank for the strict hierarchy Owner > Maintainer > Member > Watcher.
/// TeamRole is not `Ord` (its derive order would rank Owner lowest), so rank explicitly.
fn role_rank(role: TeamRole) -> u8 {
    match role {
        TeamRole::Owner => 3,
        TeamRole::Maintainer => 2,
        TeamRole::Member => 1,
        TeamRole::Watcher => 0,
    }
}

/// The stronger of two roles (used when two asserted groups map to the same team).
fn max_role(a: TeamRole, b: TeamRole) -> TeamRole {
    if role_rank(a) >= role_rank(b) { a } else { b }
}
```

If `TeamRole` has only `Owner`/`Maintainer` visible from the earlier grep, confirm the four variants are `Owner, Maintainer, Member, Watcher` (spec §2.3) before writing the match.

- [ ] **Step 4: Run the unit test to verify it passes**

Run: `SQLX_OFFLINE=true cargo test -p temper-services --lib saml_provisioning_service`
Expected: PASS.

- [ ] **Step 5: Write the failing integration test for reconcile**

Create `crates/temper-services/tests/saml_provisioning_test.rs`:

```rust
#![cfg(feature = "test-db")]
//! Integration tests for SAML membership reconcile. Each test runs on an isolated
//! `#[sqlx::test]` database with the workspace migrations applied.

use sqlx::PgPool;
use temper_core::types::TeamRole;
use temper_services::services::saml_provisioning_service::reconcile_idp_memberships;
use uuid::Uuid;

/// Minimal fixtures: a profile, two teams, one IdP, and mappings. Returns (profile, team_a, team_b).
async fn seed(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("user-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();

    let team_a: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("eng-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();

    let team_b: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("ops-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO kb_saml_idp (idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, groups_attr)
         VALUES ('acme', true, 'x', 'https://idp/sso', 'idp', 'sp', 'https://sp/acs', 'persistent', 'email', 'uid', 'groups')",
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) VALUES
         ('acme', 'engineering', $1, 'member'),
         ('acme', 'eng-leads',   $1, 'maintainer'),
         ('acme', 'operations',  $2, 'member')",
    )
    .bind(team_a)
    .bind(team_b)
    .execute(pool)
    .await
    .unwrap();

    (profile, team_a, team_b)
}

async fn membership(pool: &PgPool, team: Uuid, profile: Uuid) -> Option<(String, String)> {
    sqlx::query_as::<_, (TeamRole, String)>(
        "SELECT role, source::text FROM kb_team_members WHERE team_id=$1 AND profile_id=$2",
    )
    .bind(team)
    .bind(profile)
    .fetch_optional(pool)
    .await
    .unwrap()
    .map(|(r, s)| (format!("{r:?}"), s))
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn adds_idp_memberships_and_picks_max_role(pool: PgPool) {
    let (profile, team_a, team_b) = seed(&pool).await;

    let out = reconcile_idp_memberships(
        &pool,
        profile,
        "acme",
        &["engineering".into(), "eng-leads".into(), "operations".into()],
    )
    .await
    .unwrap();

    assert_eq!(out.added, 2);
    // engineering(member) + eng-leads(maintainer) collapse to Maintainer on team_a.
    assert_eq!(membership(&pool, team_a, profile).await, Some(("Maintainer".into(), "idp".into())));
    assert_eq!(membership(&pool, team_b, profile).await, Some(("Member".into(), "idp".into())));
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn revokes_idp_memberships_no_longer_asserted(pool: PgPool) {
    let (profile, team_a, _team_b) = seed(&pool).await;
    reconcile_idp_memberships(&pool, profile, "acme", &["engineering".into()]).await.unwrap();
    assert!(membership(&pool, team_a, profile).await.is_some());

    // Second login: no groups asserted -> the idp row is revoked.
    let out = reconcile_idp_memberships(&pool, profile, "acme", &[]).await.unwrap();
    assert_eq!(out.revoked, 1);
    assert_eq!(membership(&pool, team_a, profile).await, None);
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn native_membership_is_never_touched(pool: PgPool) {
    let (profile, team_a, _team_b) = seed(&pool).await;
    // A native membership on team_a (e.g. a join request approval).
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role, source) VALUES ($1,$2,'owner','native')")
        .bind(team_a).bind(profile).execute(&pool).await.unwrap();

    // IdP asserts engineering (maps to team_a member) -> must skip; native owner survives.
    let out = reconcile_idp_memberships(&pool, profile, "acme", &["engineering".into()]).await.unwrap();
    assert_eq!(out.skipped_native, 1);
    assert_eq!(out.added, 0);
    assert_eq!(membership(&pool, team_a, profile).await, Some(("Owner".into(), "native".into())));
}
```

**temper-services has no `MIGRATOR` export yet (this is its first `#[sqlx::test]`).** Before running, add one to `crates/temper-services/src/lib.rs` (mirroring temper-api's `pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");`):

```rust
/// Embedded workspace migrations, for `#[sqlx::test(migrator = "temper_services::MIGRATOR")]`.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");
```

`sqlx` is already a dependency; the `migrate` feature it needs is enabled transitively via the `sqlx::test` macro path used elsewhere in the workspace. If `cargo check` complains the `migrate` feature is missing, add `"migrate"` to temper-services' `sqlx` feature list in `crates/temper-services/Cargo.toml`.

- [ ] **Step 6: Run the integration test to verify it fails**

Run:
```bash
cargo make docker-up
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test saml_provisioning_test 2>&1 | tail -20
```
Expected: FAIL — `reconcile_idp_memberships` not found (won't compile).

- [ ] **Step 7: Implement `reconcile_idp_memberships` and `ReconcileOutcome`**

Add to `saml_provisioning_service.rs` (below the helpers):

```rust
/// Counts of what a reconcile pass changed. Returned for logging/observability.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReconcileOutcome {
    pub added: usize,
    pub updated: usize,
    pub revoked: usize,
    pub skipped_native: usize,
}

/// A single mapping row after filtering to asserted groups, collapsed per team.
struct DesiredMembership {
    team_id: Uuid,
    role: TeamRole,
}

/// Reconcile the profile's `source='idp'` team memberships to match the asserted groups.
///
/// Native memberships (`source='native'`) are sacred: if one exists for a `(team, profile)`
/// pair, that team is skipped entirely (native-wins-skip). Runs in one transaction so a
/// failure leaves membership state unchanged (fail-open at the caller).
pub async fn reconcile_idp_memberships(
    pool: &PgPool,
    profile_id: Uuid,
    idp_key: &str,
    groups: &[String],
) -> ApiResult<ReconcileOutcome> {
    // 1. Desired set: mapping rows whose group is asserted, collapsed to one max role per team.
    let mut desired: HashMap<Uuid, TeamRole> = HashMap::new();
    if !groups.is_empty() {
        let rows = sqlx::query!(
            r#"SELECT team_id, role AS "role: TeamRole"
               FROM kb_saml_group_mappings
               WHERE idp_key = $1 AND group_value = ANY($2)"#,
            idp_key,
            groups,
        )
        .fetch_all(pool)
        .await?;
        for r in rows {
            desired
                .entry(r.team_id)
                .and_modify(|cur| *cur = max_role(*cur, r.role))
                .or_insert(r.role);
        }
    }

    let mut tx = pool.begin().await?;

    // 2. Current state for this profile: role + source per team.
    let current = sqlx::query!(
        r#"SELECT team_id, role AS "role: TeamRole", source::text AS "source: String"
           FROM kb_team_members WHERE profile_id = $1"#,
        profile_id,
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut native_teams: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut idp_current: HashMap<Uuid, TeamRole> = HashMap::new();
    for c in current {
        if c.source.as_deref() == Some("native") {
            native_teams.insert(c.team_id);
        } else {
            idp_current.insert(c.team_id, c.role);
        }
    }

    let mut out = ReconcileOutcome::default();

    // 3. Add / update desired teams (skipping any team the user is native in).
    for m in desired.iter().map(|(&team_id, &role)| DesiredMembership { team_id, role }) {
        if native_teams.contains(&m.team_id) {
            out.skipped_native += 1;
            continue;
        }
        match idp_current.get(&m.team_id) {
            Some(&existing) if existing == m.role => {}
            Some(_) => {
                sqlx::query!(
                    "UPDATE kb_team_members SET role = $3 WHERE team_id = $1 AND profile_id = $2",
                    m.team_id,
                    profile_id,
                    m.role as TeamRole,
                )
                .execute(&mut *tx)
                .await?;
                out.updated += 1;
            }
            None => {
                sqlx::query!(
                    "INSERT INTO kb_team_members (team_id, profile_id, role, source) VALUES ($1, $2, $3, 'idp')",
                    m.team_id,
                    profile_id,
                    m.role as TeamRole,
                )
                .execute(&mut *tx)
                .await?;
                out.added += 1;
            }
        }
    }

    // 4. Revoke idp memberships no longer desired.
    for (&team_id, _) in idp_current.iter().filter(|(t, _)| !desired.contains_key(t)) {
        sqlx::query!(
            "DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2 AND source = 'idp'",
            team_id,
            profile_id,
        )
        .execute(&mut *tx)
        .await?;
        out.revoked += 1;
    }

    tx.commit().await?;
    Ok(out)
}
```

- [ ] **Step 8: Regenerate the sqlx cache for the new service queries**

Run:
```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
```
Expected: `.sqlx` entries added; no error.

- [ ] **Step 9: Run the integration test to verify it passes**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test saml_provisioning_test
```
Expected: all three tests PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/temper-services
git commit -m "feat(services): saml_provisioning_service — reconcile idp team memberships (native-wins-skip)"
```

---

### Task 4: Internal shared-secret auth middleware + config field (Rust)

**Files:**
- Modify: `crates/temper-services/src/config.rs` (add `internal_reconcile_secret: Option<String>` + read it in `from_env`)
- Create: `crates/temper-api/src/middleware/internal_auth.rs`
- Modify: `crates/temper-api/src/middleware/mod.rs` (add `pub mod internal_auth;`)
- Test: unit test inline in `internal_auth.rs`

**Interfaces:**
- Consumes: `AppState` / `ApiConfig.internal_reconcile_secret`.
- Produces: `pub async fn require_internal_secret(State<AppState>, Request<Body>, Next) -> Result<Response, ApiError>`; constant-time compares the `X-Temper-Internal-Secret` header against the configured secret; 401 on mismatch or when unconfigured. Header name constant `pub const INTERNAL_SECRET_HEADER: &str = "X-Temper-Internal-Secret";`.

- [ ] **Step 1: Add the config field**

In `crates/temper-services/src/config.rs`, add to `ApiConfig`:
```rust
    /// Shared secret gating the internal SAML reconcile endpoint. `None` disables the endpoint.
    pub internal_reconcile_secret: Option<String>,
```
And in `from_env()` (use `.ok()` so it's optional — mirror how `auth_audience` optionality is handled in this file):
```rust
        internal_reconcile_secret: env::var("INTERNAL_RECONCILE_SECRET").ok(),
```

**This breaks every `ApiConfig { .. }` struct literal in the codebase** (a non-`Default` field). Update them:
- `crates/temper-api/tests/common/mod.rs` — `setup_test_app`'s `ApiConfig { .. }` literal: add `internal_reconcile_secret: None,`.
- Any other `ApiConfig { .. }` literal — find them: `grep -rn "ApiConfig {" crates/ tests/ api/`. Add `internal_reconcile_secret: None,` to each (or `Some(...)` where a test needs the endpoint live). `from_env()` is the only non-literal constructor.

- [ ] **Step 2: Write the failing unit test**

Create `crates/temper-api/src/middleware/internal_auth.rs`:
```rust
//! Shared-secret gate for the internal SAML reconcile endpoint (AS -> temper-api).
//! Not a JWT path: the caller is the co-deployed Authorization Server, trusted by a
//! constant-time-compared shared secret from `INTERNAL_RECONCILE_SECRET`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_only_identical_secrets() {
        assert!(secret_matches("hunter2", Some("hunter2")));
        assert!(!secret_matches("hunter2", Some("Hunter2")));
        assert!(!secret_matches("hunter2", Some("")));
        assert!(!secret_matches("hunter2", None)); // endpoint unconfigured
        assert!(!secret_matches("", Some("hunter2")));
    }
}
```
Add `pub mod internal_auth;` to `crates/temper-api/src/middleware/mod.rs`.

- [ ] **Step 3: Run the test to verify it fails**

Run: `SQLX_OFFLINE=true cargo test -p temper-api --lib internal_auth 2>&1 | head -20`
Expected: FAIL — `secret_matches` not found.

- [ ] **Step 4: Implement the middleware**

Prepend to `internal_auth.rs` (above the test module). Use `subtle` if it's already a workspace dep; otherwise a length-checked byte compare is acceptable here (the secret is not a password hash) — grep first: `grep -rn "subtle" Cargo.toml crates/*/Cargo.toml`.

```rust
use axum::body::Body;
use axum::extract::State;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use temper_services::error::ApiError;
use temper_services::state::AppState;

pub const INTERNAL_SECRET_HEADER: &str = "X-Temper-Internal-Secret";

/// Constant-time-ish comparison: equal length AND equal bytes, no early return on content.
/// `configured == None` means the endpoint is disabled and never matches.
fn secret_matches(presented: &str, configured: Option<&str>) -> bool {
    let Some(expected) = configured else { return false };
    if expected.is_empty() || presented.len() != expected.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (a, b) in presented.bytes().zip(expected.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Rejects the request unless it carries the correct `X-Temper-Internal-Secret` header.
pub async fn require_internal_secret(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let presented = request
        .headers()
        .get(INTERNAL_SECRET_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !secret_matches(presented, state.config.internal_reconcile_secret.as_deref()) {
        tracing::warn!("internal reconcile: rejected (bad or missing shared secret)");
        return Err(ApiError::Unauthorized("invalid internal secret".to_string()));
    }
    Ok(next.run(request).await)
}
```

Note: `secret_matches` must be visible to the test — it's a private fn in the same module, so `use super::*;` covers it.

- [ ] **Step 5: Run the test to verify it passes**

Run: `SQLX_OFFLINE=true cargo test -p temper-api --lib internal_auth`
Expected: PASS.

- [ ] **Step 6: Regenerate api sqlx cache if config change touched macro queries**

(No SQL added here, but config is a compile dependency.) Run: `SQLX_OFFLINE=true cargo check -p temper-api --all-features`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-services/src/config.rs crates/temper-api/src/middleware
git commit -m "feat(api): internal shared-secret middleware + INTERNAL_RECONCILE_SECRET config"
```

---

### Task 5: Internal reconcile handler + route wiring (Rust)

**Files:**
- Create: `crates/temper-api/src/handlers/internal_saml.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs` (add `pub mod internal_saml;`)
- Modify: `crates/temper-api/src/routes.rs` (add an `internal` router gated by `require_internal_secret`, merged into `app`)
- Test: `crates/temper-api/tests/internal_saml_test.rs` (feature `test-db`)

**Interfaces:**
- Consumes: `ReconcileRequest` (Task 2); `saml_provisioning_service::reconcile_idp_memberships` (Task 3); `profile_service::resolve_from_claims`; `require_internal_secret` + `INTERNAL_SECRET_HEADER` (Task 4).
- Produces: `POST /internal/saml/reconcile` → `204 No Content` on success (fail-open lives in the AS caller; the endpoint itself returns real errors).

- [ ] **Step 1: Add a config-parametrized harness helper**

The existing `common::setup_test_app` (in `crates/temper-api/tests/common/mod.rs`) hardcodes `ApiConfig`. Add a variant that lets a test override the config (mirror `setup_test_app` exactly — the only change is the `configure` closure applied before `AppState::new`). Insert into `common/mod.rs`:

```rust
/// Like [`setup_test_app`] but lets the caller mutate the `ApiConfig` before the app is built
/// (e.g. to set `internal_reconcile_secret` / `auth_provider_name` for a specific test).
pub async fn setup_test_app_with_config(
    pool: PgPool,
    configure: impl FnOnce(&mut ApiConfig),
) -> TestApp {
    fixtures::clean_and_seed(&pool).await;

    let decoding_key = jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("test_rsa.pub"))
        .expect("Failed to load test RSA public key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, Algorithm::RS256);

    let mut config = ApiConfig {
        database_url: "unused".to_string(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
    };
    configure(&mut config);

    let state = AppState::new(pool.clone(), jwks_store, config);
    let app = create_app(state);

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test listener");
    let addr = listener.local_addr().expect("Failed to get local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("Test server failed");
    });

    TestApp { addr, pool, client: reqwest::Client::new() }
}
```

(`ApiConfig` is already imported in `common/mod.rs`. Note the `internal_reconcile_secret: None` field — added by Task 4.)

- [ ] **Step 2: Write the failing integration test (complete)**

Create `crates/temper-api/tests/internal_saml_test.rs`:

```rust
#![cfg(feature = "test-db")]
//! HTTP-layer integration tests for the internal SAML reconcile endpoint.
//! The endpoint is gated by a shared secret (not JWT). We build the app with a known
//! `internal_reconcile_secret` and `auth_provider_name = "saml:acme"` so the JIT'd profile's
//! auth link matches what the minted token would later resolve to.

mod common;

use sqlx::PgPool;
use temper_core::types::ReconcileRequest;
use uuid::Uuid;

/// Seed an active IdP 'acme', a team, and a mapping engineering -> team (member). Returns team_id.
async fn seed(pool: &PgPool) -> Uuid {
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(format!("eng-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("seed team");

    sqlx::query(
        "INSERT INTO kb_saml_idp (idp_key, is_active, idp_cert, idp_sso_url, idp_entity_id, sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, groups_attr)
         VALUES ('acme', true, 'x', 'https://idp/sso', 'idp', 'sp', 'https://sp/acs', 'persistent', 'email', 'uid', 'groups')",
    )
    .execute(pool)
    .await
    .expect("seed idp");

    sqlx::query("INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) VALUES ('acme', 'engineering', $1, 'member')")
        .bind(team_id)
        .execute(pool)
        .await
        .expect("seed mapping");

    team_id
}

fn reconcile_body() -> ReconcileRequest {
    ReconcileRequest {
        provider: Some("saml:acme".to_string()),
        external_user_id: "nid-1".to_string(),
        email: "a@corp.io".to_string(),
        email_verified: Some(true),
        idp_key: "acme".to_string(),
        groups: vec!["engineering".to_string()],
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_endpoint_provisions_idp_membership(pool: PgPool) {
    let team_id = seed(&pool).await;
    let app = common::setup_test_app_with_config(pool.clone(), |c| {
        c.auth_provider_name = "saml:acme".to_string();
        c.internal_reconcile_secret = Some("s3cr3t".to_string());
    })
    .await;

    let resp = app
        .client
        .post(app.url("/internal/saml/reconcile"))
        .header("X-Temper-Internal-Secret", "s3cr3t")
        .json(&reconcile_body())
        .send()
        .await
        .expect("request failed");
    assert_eq!(
        resp.status().as_u16(),
        204,
        "correct secret should return 204; body: {}",
        resp.text().await.unwrap_or_default()
    );

    // The profile was JIT-created with provider 'saml:acme', external id 'nid-1'.
    let profile_id: Uuid = sqlx::query_scalar(
        "SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider = $1 AND auth_provider_user_id = $2",
    )
    .bind("saml:acme")
    .bind("nid-1")
    .fetch_one(&pool)
    .await
    .expect("JIT auth link must exist");

    let (role, source): (String, String) = sqlx::query_as(
        "SELECT role::text, source::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(team_id)
    .bind(profile_id)
    .fetch_one(&pool)
    .await
    .expect("idp membership must exist");
    assert_eq!(role, "member");
    assert_eq!(source, "idp");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reconcile_endpoint_rejects_wrong_secret(pool: PgPool) {
    seed(&pool).await;
    let app = common::setup_test_app_with_config(pool.clone(), |c| {
        c.auth_provider_name = "saml:acme".to_string();
        c.internal_reconcile_secret = Some("s3cr3t".to_string());
    })
    .await;

    let resp = app
        .client
        .post(app.url("/internal/saml/reconcile"))
        .header("X-Temper-Internal-Secret", "wrong")
        .json(&reconcile_body())
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 401);

    // No profile/link/membership was created.
    let links: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_profile_auth_links WHERE auth_provider_user_id = 'nid-1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(links, 0);
}
```

Note: `role::text`/`source::text` casts keep the fixture assertions as plain `String` (no `TeamRole`/enum FromRow needed in the test).

- [ ] **Step 3: Run the test to verify it fails**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-api --features test-db --test internal_saml_test 2>&1 | tail -20
```
Expected: FAIL — route/handler not found (won't compile / 404).

- [ ] **Step 4: Implement the handler**

Create `crates/temper-api/src/handlers/internal_saml.rs`:
```rust
//! Internal SAML membership-reconcile endpoint. Called server-to-server by the co-deployed
//! Authorization Server after it validates an assertion, BEFORE it mints the token. Gated by
//! `require_internal_secret` (not JWT). See the Phase 2 design spec §7.2.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

use temper_core::types::{AuthClaims, ReconcileRequest};
use temper_services::error::ApiError;
use temper_services::services::{profile_service, saml_provisioning_service};
use temper_services::state::AppState;

/// `POST /internal/saml/reconcile` — resolve/JIT the profile, then reconcile its idp memberships.
pub async fn reconcile(
    State(state): State<AppState>,
    Json(req): Json<ReconcileRequest>,
) -> Result<StatusCode, ApiError> {
    // Identity provider string is authoritative from server config, NOT the payload — this MUST
    // match middleware/auth.rs so the resolved profile is the same one the minted token resolves to.
    let claims = AuthClaims {
        provider: state.config.auth_provider_name.clone(),
        external_user_id: req.external_user_id.clone(),
        email: req.email.clone(),
        email_verified: req.email_verified,
        // exp/iat are unused by resolve_from_claims; supply zero rather than inventing a clock.
        exp: 0,
        iat: 0,
    };

    let profile = profile_service::resolve_from_claims(&state.pool, &claims).await?;

    let outcome = saml_provisioning_service::reconcile_idp_memberships(
        &state.pool,
        profile.id,
        &req.idp_key,
        &req.groups,
    )
    .await?;

    tracing::info!(
        profile_id = %profile.id,
        idp_key = %req.idp_key,
        added = outcome.added,
        updated = outcome.updated,
        revoked = outcome.revoked,
        skipped_native = outcome.skipped_native,
        "saml reconcile complete",
    );

    Ok(StatusCode::NO_CONTENT)
}
```
Add `pub mod internal_saml;` to `crates/temper-api/src/handlers/mod.rs`.

- [ ] **Step 5: Wire the route into `routes.rs`**

In `create_app`, add a new router after `gated` and merge it. It carries the internal-secret layer, NOT `require_auth`:
```rust
    let internal = Router::new()
        .route(
            "/internal/saml/reconcile",
            post(handlers::internal_saml::reconcile),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::internal_auth::require_internal_secret,
        ));
```
And extend the merge line:
```rust
    let mut app = Router::new().merge(public).merge(auth_only).merge(gated).merge(internal);
```

- [ ] **Step 6: Regenerate the api test sqlx cache**

Run:
```bash
cargo make prepare-api
```
Expected: `crates/temper-api/.sqlx` updated for the new test queries (if the test uses macro queries; runtime `sqlx::query()` fixtures don't need it — but run it to be safe).

- [ ] **Step 7: Run the integration test to verify it passes**

Run:
```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-api --features test-db --test internal_saml_test
```
Expected: PASS (204 + membership for correct secret; 401 for wrong secret).

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api
git commit -m "feat(api): POST /internal/saml/reconcile — JIT profile + reconcile idp memberships"
```

---

### Task 6: AS group extraction + `groups_attr` config (TypeScript)

**Files:**
- Modify: `packages/temper-cloud/src/saml/config.ts` (add `groups_attr` to `SamlIdpRow` + `loadActiveIdp` SELECT)
- Modify: `packages/temper-cloud/src/saml/sp.ts` (add `extractGroups`)
- Test: `packages/temper-cloud/tests/saml/sp.test.ts` (add cases; create the file if the SAML unit tests live elsewhere — grep first)

**Interfaces:**
- Produces: `export function extractGroups(profile: Profile, idp: SamlIdpRow): string[]` — reads the multi-valued attribute named by `idp.groups_attr`; returns `[]` when `groups_attr` is null or the attribute is absent.
- Modifies: `SamlIdpRow` gains `groups_attr: string | null`.

- [ ] **Step 1: Locate the existing SAML unit tests**

Run: `ls packages/temper-cloud/tests/saml/ 2>/dev/null; grep -rln "mapProfileToClaims" packages/temper-cloud/tests`
Use whichever file already tests `sp.ts`; if none, create `packages/temper-cloud/tests/saml/sp.test.ts`.

- [ ] **Step 2: Write the failing test**

Add to the SAML sp test file (Vitest):
```ts
import { describe, expect, it } from "vitest";
import type { Profile } from "@node-saml/node-saml";
import { extractGroups } from "../../src/saml/sp.js";
import type { SamlIdpRow } from "../../src/saml/config.js";

const idp = (groups_attr: string | null): SamlIdpRow => ({
  idp_key: "acme", is_active: true, idp_cert: "x", idp_sso_url: "u", idp_entity_id: "e",
  sp_entity_id: "sp", acs_url: "a", nameid_format: "persistent", email_attr: "email",
  stable_id_attr: "uid", groups_attr, created: "", updated: "",
});

const profileWith = (attrs: Record<string, unknown>): Profile =>
  ({ attributes: attrs } as unknown as Profile);

describe("extractGroups", () => {
  it("returns [] when groups_attr is null", () => {
    expect(extractGroups(profileWith({ groups: ["a"] }), idp(null))).toEqual([]);
  });
  it("reads a multi-valued attribute", () => {
    expect(extractGroups(profileWith({ groups: ["a", "b"] }), idp("groups"))).toEqual(["a", "b"]);
  });
  it("coerces a single-valued attribute to a one-element array", () => {
    expect(extractGroups(profileWith({ groups: "solo" }), idp("groups"))).toEqual(["solo"]);
  });
  it("returns [] when the named attribute is absent", () => {
    expect(extractGroups(profileWith({ other: ["a"] }), idp("groups"))).toEqual([]);
  });
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cd packages/temper-cloud && bun run test saml/sp && cd -`
Expected: FAIL — `extractGroups` is not exported.

- [ ] **Step 4: Implement `extractGroups` + config plumbing**

In `packages/temper-cloud/src/saml/config.ts`, add to `SamlIdpRow`:
```ts
  groups_attr: string | null;
```
and add `groups_attr` to the `loadActiveIdp` SELECT column list:
```ts
    sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr, groups_attr, created, updated
```

In `packages/temper-cloud/src/saml/sp.ts`, add (reusing the file's `readAttr` narrowing style):
```ts
/**
 * Reads the multi-valued group attribute named by `idp.groups_attr` from a validated assertion.
 * Returns [] when no groups attribute is configured or the attribute is absent — either way the
 * reconcile call will assert "no IdP-driven memberships".
 */
export function extractGroups(profile: Profile, idp: SamlIdpRow): string[] {
  if (!idp.groups_attr) {
    return [];
  }
  const attrs = (profile.attributes ?? {}) as Record<string, unknown>;
  const value = attrs[idp.groups_attr];
  if (value === undefined || value === null) {
    return [];
  }
  const arr = Array.isArray(value) ? value : [value];
  return arr.map((v) => String(v)).filter((s) => s.length > 0);
}
```
Ensure `SamlIdpRow` is imported in `sp.ts` (it already imports from `./config.js`).

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd packages/temper-cloud && bun run test saml/sp && bun run typecheck && cd -`
Expected: PASS + typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add packages/temper-cloud/src/saml packages/temper-cloud/tests/saml
git commit -m "feat(saml): extractGroups + groups_attr on the active IdP config"
```

---

### Task 7: AS reconcile client + ACS wiring (fail-open) (TypeScript)

**Files:**
- Create: `packages/temper-cloud/src/oauth/reconcile.ts`
- Modify: `packages/temper-cloud/src/oauth/endpoints.ts` (call reconcile in `handleSamlAcs`)
- Test: `packages/temper-cloud/tests/oauth/reconcile.test.ts`; extend the ACS test to prove fail-open

**Interfaces:**
- Consumes: `extractGroups` (Task 6); the generated `ReconcileRequest` TS type (Task 2); env `INTERNAL_RECONCILE_URL`, `INTERNAL_RECONCILE_SECRET`.
- Produces: `export async function reconcileMemberships(payload: ReconcileRequest): Promise<void>` — POSTs to `INTERNAL_RECONCILE_URL` with the secret header; throws on non-2xx or fetch error (the ACS caller catches and proceeds — fail-open).

- [ ] **Step 1: Write the failing test for the reconcile client**

Create `packages/temper-cloud/tests/oauth/reconcile.test.ts`:
```ts
import { afterEach, describe, expect, it, vi } from "vitest";
import { reconcileMemberships } from "../../src/oauth/reconcile.js";

const payload = {
  provider: "saml:acme", external_user_id: "nid-1", email: "a@corp.io",
  email_verified: true, idp_key: "acme", groups: ["engineering"],
};

afterEach(() => vi.unstubAllGlobals());

describe("reconcileMemberships", () => {
  it("POSTs to INTERNAL_RECONCILE_URL with the secret header", async () => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", "https://api.internal/internal/saml/reconcile");
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    const fetchMock = vi.fn(async () => new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await reconcileMemberships(payload);

    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("https://api.internal/internal/saml/reconcile");
    expect((init as RequestInit).method).toBe("POST");
    expect((init as RequestInit).headers).toMatchObject({
      "content-type": "application/json",
      "X-Temper-Internal-Secret": "s3cr3t",
    });
    expect(JSON.parse((init as RequestInit).body as string)).toMatchObject({ idp_key: "acme" });
  });

  it("throws on a non-2xx response", async () => {
    vi.stubEnv("INTERNAL_RECONCILE_URL", "https://api.internal/internal/saml/reconcile");
    vi.stubEnv("INTERNAL_RECONCILE_SECRET", "s3cr3t");
    vi.stubGlobal("fetch", vi.fn(async () => new Response("nope", { status: 500 })));
    await expect(reconcileMemberships(payload)).rejects.toThrow();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd packages/temper-cloud && bun run test oauth/reconcile && cd -`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the reconcile client**

Create `packages/temper-cloud/src/oauth/reconcile.ts`:
```ts
import { logger } from "../logger.js";
import { requireEnv } from "./env.js";
import type { ReconcileRequest } from "../generated/ReconcileRequest.js";

/**
 * Calls the internal temper-api reconcile endpoint (server-to-server) with the shared secret.
 * Throws on transport error or non-2xx — the ACS handler catches and proceeds (fail-open), so a
 * provisioning hiccup never blocks login. The header name matches temper-api's INTERNAL_SECRET_HEADER.
 */
export async function reconcileMemberships(payload: ReconcileRequest): Promise<void> {
  const url = requireEnv("INTERNAL_RECONCILE_URL");
  const secret = requireEnv("INTERNAL_RECONCILE_SECRET");
  const res = await fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "X-Temper-Internal-Secret": secret,
    },
    body: JSON.stringify(payload),
  });
  if (!res.ok) {
    throw new Error(`reconcile endpoint returned ${res.status}`);
  }
  logger.info({ idp_key: payload.idp_key, groups: payload.groups.length }, "saml reconcile ok");
}
```
Adjust the `ReconcileRequest` import path to the actual generated location from Task 2 Step 4.

- [ ] **Step 4: Run the reconcile client test to verify it passes**

Run: `cd packages/temper-cloud && bun run test oauth/reconcile && cd -`
Expected: PASS.

- [ ] **Step 5: Wire reconcile into the ACS handler (fail-open)**

In `packages/temper-cloud/src/oauth/endpoints.ts`, inside `handleSamlAcs`'s `try` block, after `const claims = mapProfileToClaims(profile, idp);` and before `bindCodeToFlow`, insert a self-contained fail-open reconcile:
```ts
    // Phase 2: reconcile IdP-driven team memberships before minting. Fail-open — a provisioning
    // error must never block authentication (design spec §3.8). Its own try/catch so a reconcile
    // failure is NOT misreported as an assertion rejection by the outer catch.
    try {
      await reconcileMemberships({
        provider: `saml:${idp.idp_key}`,
        external_user_id: claims.sub,
        email: claims.email,
        email_verified: claims.email_verified,
        idp_key: idp.idp_key,
        groups: extractGroups(profile, idp),
      });
    } catch (reconcileErr) {
      logger.error(
        { err: reconcileErr instanceof Error ? reconcileErr.message : String(reconcileErr) },
        "SAML ACS: membership reconcile failed (fail-open, login proceeds)",
      );
    }
```
Add the imports at the top of `endpoints.ts`:
```ts
import { extractGroups, mapProfileToClaims, validateAssertion } from "../saml/sp.js";
import { reconcileMemberships } from "./reconcile.js";
```
(`mapProfileToClaims`/`validateAssertion` are already imported — merge `extractGroups` into that existing import line rather than duplicating.)

- [ ] **Step 6: Write the ACS fail-open test**

In the ACS test file (grep `handleSamlAcs` under `tests/`), add a case: stub `reconcileMemberships` (or `fetch`) to reject, drive a valid assertion, and assert the handler still returns a redirect with a `code` (login proceeds). If the ACS tests are integration-style (real DB), assert the flow still issues a code despite a rejecting `fetch` stub.

- [ ] **Step 7: Run TS tests + typecheck + lint**

Run:
```bash
cd packages/temper-cloud && bun run test && bun run typecheck && bun run check && cd -
```
Expected: all PASS.

- [ ] **Step 8: Commit**

```bash
git add packages/temper-cloud
git commit -m "feat(saml): AS calls internal reconcile before minting (fail-open)"
```

---

### Task 8: End-to-end — SAML login with groups drives reconcile

**Files:**
- Modify: `packages/temper-cloud/test-fixtures/saml.ts` (add multi-valued attribute support to `makeSignedSamlResponse`)
- Test: extend `packages/temper-cloud/tests/integration/oauth/e2e.saml.test.ts`.

**Interfaces:**
- The temper-cloud integration e2e drives the AS in-process against a real local PG (via the `postgres` pkg). The Rust `/internal/saml/reconcile` endpoint is a different process, so this TS e2e stubs `fetch` and asserts the ACS issues a reconcile POST carrying the asserted groups. (The Rust reconcile behavior is proven by Tasks 3 & 5.)

- [ ] **Step 1: Extend the fixture to emit multi-valued attributes**

In `packages/temper-cloud/test-fixtures/saml.ts`, add a `multiValuedAttributes` param. Add to `MakeSignedSamlResponseParams`:
```ts
  /** Attributes emitted with multiple <AttributeValue> children (e.g. group membership). */
  multiValuedAttributes?: Record<string, string[]>;
```
Destructure it with a default in the `makeSignedSamlResponse` signature (beside `attributes = {}`):
```ts
  multiValuedAttributes = {},
```
Then, right after the existing `const attributeXml = ...` block, build the multi-valued XML and combine — and use the combined value in the `AttributeStatement` guard:
```ts
  const multiValuedAttributeXml = Object.entries(multiValuedAttributes)
    .map(
      ([name, values]) =>
        `<saml:Attribute Name="${name}" NameFormat="urn:oasis:names:tc:SAML:2.0:attrname-format:basic">` +
        values
          .map((v) => `<saml:AttributeValue xsi:type="xs:string">${v}</saml:AttributeValue>`)
          .join("") +
        `</saml:Attribute>`,
    )
    .join("");
  const allAttributeXml = attributeXml + multiValuedAttributeXml;
```
Change the assertion-body line from `(attributeXml ? \`<saml:AttributeStatement>${attributeXml}...` to use `allAttributeXml`:
```ts
    (allAttributeXml ? `<saml:AttributeStatement>${allAttributeXml}</saml:AttributeStatement>` : "") +
```
(The signing logic is untouched — only the pre-signing XML string changes, so the byte-for-byte-verified signer still applies.)

- [ ] **Step 2: Write the failing e2e assertion**

In `packages/temper-cloud/tests/integration/oauth/e2e.saml.test.ts`, add `vi` to the vitest import:
```ts
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
```
Add this test inside the `describe` block (it reuses `db`, `rs` flow, `idpKeyPem`/`idpCertPem`, and the module constants):
```ts
  it("ACS issues a reconcile call carrying the asserted groups (fail-open)", async () => {
    // Configure the seeded IdP for group provisioning + point the reconcile client at a stub.
    await sql`UPDATE kb_saml_idp SET groups_attr = 'groups' WHERE idp_key = 'test'`;
    process.env.INTERNAL_RECONCILE_URL = "https://api.internal/internal/saml/reconcile";
    process.env.INTERNAL_RECONCILE_SECRET = "s3cr3t";

    const reconcileCalls: Array<{ url: string; body: unknown; secret: string | null }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init: RequestInit) => {
        const headers = new Headers(init.headers);
        reconcileCalls.push({
          url,
          body: JSON.parse(init.body as string),
          secret: headers.get("X-Temper-Internal-Secret"),
        });
        return new Response(null, { status: 204 });
      }),
    );

    try {
      // authorize -> relay state
      const verifier = `e2e-grp-verifier-${"a".repeat(50)}`;
      const challenge = createHash("sha256").update(verifier).digest("base64url");
      const authRes = await handleAuthorize(
        new Request(
          "https://as.example.com/oauth/authorize?response_type=code&client_id=cli&redirect_uri=" +
            encodeURIComponent(REDIRECT_URI) +
            "&code_challenge=" +
            challenge +
            "&code_challenge_method=S256&state=grp-state",
        ),
        db,
      );
      const rs = new URLSearchParams(
        new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
      ).get("rs");

      // signed assertion carrying a multi-valued 'groups' attribute
      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID,
        acsUrl: ACS_URL,
        nameId: "grp-user-1",
        attributes: { email: "grp@example.com", uid: "grp-user-1" },
        multiValuedAttributes: { groups: ["engineering", "eng-leads"] },
        idpKeyPem,
        idpCertPem,
      });

      const acsRes = await handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
        }),
        db,
      );

      // login still completes (fail-open is irrelevant here since the stub returns 204)
      expect(acsRes.status).toBe(302);
      expect(new URL(acsRes.headers.get("location") as string).searchParams.get("code")).toBeTruthy();

      // the reconcile POST fired with the asserted groups + secret header
      expect(reconcileCalls).toHaveLength(1);
      expect(reconcileCalls[0].url).toBe("https://api.internal/internal/saml/reconcile");
      expect(reconcileCalls[0].secret).toBe("s3cr3t");
      expect(reconcileCalls[0].body).toMatchObject({
        idp_key: "test",
        external_user_id: "grp-user-1",
        groups: ["engineering", "eng-leads"],
      });
    } finally {
      vi.unstubAllGlobals();
      delete process.env.INTERNAL_RECONCILE_URL;
      delete process.env.INTERNAL_RECONCILE_SECRET;
    }
  });
```

- [ ] **Step 3: Run the e2e to verify it fails, then passes**

Run: `cd packages/temper-cloud && bun run test:integration oauth/e2e.saml && cd -`
Expected: FAIL before Task 6/7 land (no reconcile call) and before Step 1 (groups not emitted); PASS once the ACS wiring (Task 7) + fixture change (Step 1) are in place.

- [ ] **Step 4: Verify fail-open — a rejecting reconcile still logs in**

Add a second test that stubs `fetch` to reject and asserts the ACS still returns a 302 with a `code` (login proceeds despite reconcile failure):
```ts
  it("ACS completes login even when reconcile fails (fail-open)", async () => {
    await sql`UPDATE kb_saml_idp SET groups_attr = 'groups' WHERE idp_key = 'test'`;
    process.env.INTERNAL_RECONCILE_URL = "https://api.internal/internal/saml/reconcile";
    process.env.INTERNAL_RECONCILE_SECRET = "s3cr3t";
    vi.stubGlobal("fetch", vi.fn(async () => new Response("boom", { status: 500 })));
    try {
      const verifier = `e2e-fo-verifier-${"a".repeat(50)}`;
      const challenge = createHash("sha256").update(verifier).digest("base64url");
      const authRes = await handleAuthorize(
        new Request(
          "https://as.example.com/oauth/authorize?response_type=code&client_id=cli&redirect_uri=" +
            encodeURIComponent(REDIRECT_URI) +
            "&code_challenge=" + challenge + "&code_challenge_method=S256&state=fo-state",
        ),
        db,
      );
      const rs = new URLSearchParams(
        new URL(authRes.headers.get("location") as string, "https://as.example.com").search,
      ).get("rs");
      const { samlResponseB64 } = makeSignedSamlResponse({
        spEntityId: SP_ENTITY_ID, acsUrl: ACS_URL, nameId: "fo-user-1",
        attributes: { email: "fo@example.com", uid: "fo-user-1" },
        multiValuedAttributes: { groups: ["engineering"] },
        idpKeyPem, idpCertPem,
      });
      const acsRes = await handleSamlAcs(
        new Request("https://sp.example.com/saml/acs", {
          method: "POST",
          body: new URLSearchParams({ SAMLResponse: samlResponseB64, RelayState: rs as string }),
        }),
        db,
      );
      expect(acsRes.status).toBe(302);
      expect(new URL(acsRes.headers.get("location") as string).searchParams.get("code")).toBeTruthy();
    } finally {
      vi.unstubAllGlobals();
      delete process.env.INTERNAL_RECONCILE_URL;
      delete process.env.INTERNAL_RECONCILE_SECRET;
    }
  });
```
Run again: `cd packages/temper-cloud && bun run test:integration oauth/e2e.saml && cd -` → PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud
git commit -m "test(saml): e2e — asserted groups drive the reconcile call; fail-open login"
```

> **Carry-forward (not a task):** a true cross-process Rust e2e (spawn temper-api, POST a real `ReconcileRequest`, log in with the minted token, assert memberships) would exercise the live endpoint end-to-end. Task 5's `#[sqlx::test]` already covers the endpoint against a real DB, so this is deferred rather than built — note it in the session/issue as a possible hardening follow-up.

---

### Task 9: Docs + env surface

**Files:**
- Modify: `docs/guides/self-hosting-saml.md` (add a "Group → team/role mapping" section + the new env vars)

**Interfaces:**
- Consumes: everything. Documents the operator SQL config surface and the two new env vars.

- [ ] **Step 1: Document the mapping config surface**

In `docs/guides/self-hosting-saml.md`, after the `kb_saml_idp` INSERT section, add:

````markdown
## 4. Map IdP groups to Temper teams/roles (Phase 2)

Temper reconciles team membership from SAML-asserted groups **on each login**. This is
eventual, not immediate: a user removed from a group keeps access until their session
expires and they next log in. For immediate deprovisioning use SCIM (not yet available).

**Reconcile only ever manages `source='idp'` memberships. Native memberships (added in-app or
by join-request approval) are never touched — if a user is already a native member of a team,
the IdP reconcile skips that team for them entirely.**

1. Tell the SP which assertion attribute carries the group list:

   ```sql
   UPDATE kb_saml_idp SET groups_attr = 'groups' WHERE idp_key = 'acme-okta';
   ```
   Leave `groups_attr` NULL to keep authentication-only behavior (no membership changes).

2. Map groups to `(team, role)`. Teams must already exist. Two groups mapping to the same
   team collapse to the strongest role (owner > maintainer > member > watcher):

   ```sql
   INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) VALUES
     ('acme-okta', 'engineering',   '<team-uuid>', 'member'),
     ('acme-okta', 'eng-leads',     '<team-uuid>', 'maintainer'),
     ('acme-okta', 'temper-admins', '<gating-team-uuid>', 'owner');
   ```
   The last row is "admin via group" — it makes members of `temper-admins` owners of the gating
   team. Note: the **first** admin still requires the SQL bootstrap step (`org-bootstrap.md`);
   SAML does not bootstrap the system.

Unmapped asserted groups are ignored. Removing a group from the assertion revokes the
corresponding `idp` membership on the next login.
````

- [ ] **Step 2: Document the new env vars**

In the env-var table of `self-hosting-saml.md`, add:

````markdown
| Var | Where | Purpose |
|-----|-------|---------|
| `INTERNAL_RECONCILE_SECRET` | AS + API (shared) | Shared secret gating the internal reconcile call. Set the SAME value on both the Authorization Server and the temper-api function. If unset, the reconcile endpoint is disabled and no group provisioning occurs. |
| `INTERNAL_RECONCILE_URL` | AS | Full URL of the temper-api `/internal/saml/reconcile` endpoint the AS calls before minting a token (e.g. `https://<your-api-origin>/internal/saml/reconcile`). |
````

- [ ] **Step 3: Verify docs lint (if the repo lints markdown) and commit**

Run: `git add docs/guides/self-hosting-saml.md && git commit -m "docs(saml): group->team/role mapping + reconcile env vars"`

---

## Final verification (run after all tasks)

- [ ] `cargo make check` (fmt + clippy + docs + machete; honest offline sqlx probe)
- [ ] `DATABASE_URL=… cargo make test-db` (Rust integration incl. Tasks 3 & 5)
- [ ] `cargo make test-e2e-embed` (SAML e2e tier — Task 8 Step 4 if added)
- [ ] `cd packages/temper-cloud && bun run test && bun run test:integration && bun run check && bun run typecheck`
- [ ] Confirm `.sqlx` caches are committed (workspace + `crates/temper-services/.sqlx` + `crates/temper-api/.sqlx`) and no orphaned entries remain.
- [ ] Update issue #224: Phase 2 delivered; Phase 3 (SCIM) remains.

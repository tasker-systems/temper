//! Persistence for `kb_connections` — temper's authed link to a remote system.
//!
//! A connection is a machine principal wearing an integration's clothes, so authorization is
//! `machine_authz::authorize` **verbatim**: a system admin, or the OWNER of the team that owns
//! the connection, with a teamless connection failing closed. Calling the machine gate rather
//! than restating it is deliberate — tighten that predicate and this surface tightens with it.
//! There is no second copy of the policy to drift.
//!
//! Writes are admin-driven and rare. Nothing here emits a ledger event: an admin creating a
//! connection is not a receipt of anything external, and this goal's own invariant is that *the
//! ledger records receipt, never elaboration*. This follows the shipped `kb_machine_clients`
//! precedent, which fires no event either.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::connection::{Connection, ProvisionConnectionRequest};
use temper_core::types::ids::ProfileId;
use temper_workflow::operations::sluggify;

use crate::error::{ApiError, ApiResult};
use crate::services::{machine_authz, profile_service};

/// The emitter marker for a connection's entity (`<handle>@webhook`).
///
/// Deliberately **not** a `Surface` variant. `Surface::ALL` drives
/// `profile_service::provision_profile_entities`, so adding `Webhook` there would provision a
/// webhook emitter onto every human profile — and oblige a backfill migration for every profile
/// that already exists (see `temper_workflow::operations::surface`). A connection's entity is
/// created directly instead, and intake resolves the emitter from `connection.emitter_entity_id`
/// rather than from a surface marker.
const WEBHOOK_EMITTER_MARKER: &str = "webhook";

/// Load one connection by its own id. Unauthorized: the internal primitive the post-insert
/// readback uses. Surface callers want [`get_for_caller`].
pub async fn get(pool: &PgPool, id: Uuid) -> ApiResult<Connection> {
    sqlx::query_as!(
        Connection,
        r#"SELECT id, provider, slug, name, owner_team_id, registered_by_profile_id,
                  profile_id, emitter_entity_id, home_context_id, credential,
                  webhook_events, tool_manifest, reach_granularity, reach_covers,
                  created, revoked_at, revoked_by_profile_id
             FROM kb_connections WHERE id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// [`get`], gated on the *existing row's* owning team.
pub async fn get_for_caller(pool: &PgPool, caller: ProfileId, id: Uuid) -> ApiResult<Connection> {
    let connection = get(pool, id).await?;
    machine_authz::authorize(pool, caller, connection.owner_team_id).await?;
    Ok(connection)
}

/// List connections visible to `caller`, newest first. Revoked rows are hidden unless asked for.
/// A system admin sees every row, including teamless ones; a team owner sees only connections
/// owned by a team they own.
///
/// `EXISTS`, not `array_agg` — an empty scope must DENY, and an aggregate over an empty scope
/// yields NULL, which falls open.
pub async fn list(
    pool: &PgPool,
    caller: ProfileId,
    include_revoked: bool,
) -> ApiResult<Vec<Connection>> {
    let is_admin = crate::services::access_service::is_system_admin(pool, caller).await?;

    let rows = sqlx::query_as!(
        Connection,
        r#"SELECT id, provider, slug, name, owner_team_id, registered_by_profile_id,
                  profile_id, emitter_entity_id, home_context_id, credential,
                  webhook_events, tool_manifest, reach_granularity, reach_covers,
                  created, revoked_at, revoked_by_profile_id
             FROM kb_connections c
            WHERE ($1 OR c.revoked_at IS NULL)
              AND ( $2
                    OR ( c.owner_team_id IS NOT NULL
                         AND EXISTS (
                             SELECT 1
                               FROM kb_team_members tm
                              WHERE tm.team_id = c.owner_team_id
                                AND tm.profile_id = $3
                                AND tm.role = 'owner'
                         ) ) )
            ORDER BY created DESC"#,
        include_revoked,
        is_admin,
        *caller,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Provision a connection, creating the profile and emitter entity that let a remote system emit
/// into the ledger, and the context that homes it. One transaction.
///
/// The connection is born **`needs_credential`** (`credential IS NULL`) and with both capability
/// tiers empty. It becomes ledger-capable and/or reach-capable only as those are provisioned, so
/// it never silently pretends to be more than it is.
pub async fn provision(
    pool: &PgPool,
    caller: ProfileId,
    req: &ProvisionConnectionRequest,
) -> ApiResult<Connection> {
    // Auth before writes: a rejected provisioning must leave the DB completely unchanged — no
    // orphaned profile, no orphaned entity, no orphaned context. Resolving before the
    // transaction opens is what makes that assertable.
    machine_authz::authorize(pool, caller, req.owner_team_id).await?;

    if req.provider.trim().is_empty() {
        return Err(ApiError::BadRequest("provider must not be empty".into()));
    }
    if req.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".into()));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to begin transaction: {e}")))?;

    let slug = next_unique_connection_slug(&mut tx, &req.name).await?;

    let (profile_id, handle) =
        profile_service::create_connection_profile(&mut tx, &req.provider, &slug).await?;

    // The emitter. `kb_events.emitter_entity_id` is NOT NULL, so without this row the connection
    // could not emit at all — every intake would fail on a missing emitter.
    let emitter_entity_id = sqlx::query_scalar!(
        r#"INSERT INTO kb_entities (profile_id, name, metadata)
           VALUES ($1, $2, '{}'::jsonb)
           RETURNING id"#,
        profile_id,
        format!("{handle}@{WEBHOOK_EMITTER_MARKER}"),
    )
    .fetch_one(&mut *tx)
    .await?;

    // The home. Owned by the owning TEAM where there is one, so read authz inherits for free —
    // the team that owns the connection can read what it receives, with no new predicate. A
    // teamless connection homes on its own profile, which (like the rest of the teamless path)
    // is admin-only to reach.
    let (owner_table, owner_id) = match req.owner_team_id {
        Some(team_id) => ("kb_teams", team_id),
        None => ("kb_profiles", profile_id),
    };
    let home_context_id = sqlx::query_scalar!(
        r#"INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id"#,
        Uuid::now_v7(),
        owner_table,
        owner_id,
        &slug,
        &req.name,
    )
    .fetch_one(&mut *tx)
    .await?;

    let id = sqlx::query_scalar!(
        r#"INSERT INTO kb_connections
               (provider, slug, name, owner_team_id, registered_by_profile_id,
                profile_id, emitter_entity_id, home_context_id,
                reach_granularity, reach_covers)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id"#,
        req.provider,
        slug,
        req.name,
        req.owner_team_id,
        *caller,
        profile_id,
        emitter_entity_id,
        home_context_id,
        req.reach_granularity,
        req.reach_covers,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| map_duplicate(e, &slug))?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to commit transaction: {e}")))?;

    get(pool, id).await
}

/// Mark a connection dead. Idempotent in effect but not in record: a second revoke of an
/// already-revoked row is a no-op returning the existing row (the first revoker and first
/// timestamp are the truth). Rows are never deleted; reactivation is a new provisioning.
///
/// The profile, entity, and home context are deliberately left in place — events already
/// attributed to this emitter must keep resolving, and `kb_events` is append-only.
pub async fn revoke(pool: &PgPool, id: Uuid, revoker: ProfileId) -> ApiResult<Connection> {
    // Auth before writes, keyed on the existing row's owning team.
    let existing = get(pool, id).await?;
    machine_authz::authorize(pool, revoker, existing.owner_team_id).await?;

    sqlx::query!(
        r#"UPDATE kb_connections
              SET revoked_at = now(), revoked_by_profile_id = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        *revoker,
    )
    .execute(pool)
    .await?;
    get(pool, id).await
}

/// A connection slug, unique across the instance. Bases it on the name; on collision (two
/// distinct names can sluggify to the same string) suffixes `-2`, `-3`, … The UNIQUE constraint
/// remains the actual race guard — this only keeps the common case friendly.
async fn next_unique_connection_slug(
    conn: &mut sqlx::PgConnection,
    name: &str,
) -> ApiResult<String> {
    let base = sluggify(name);
    if base.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "name '{name}' has no sluggable characters"
        )));
    }

    for suffix in 1..=100 {
        let candidate = if suffix == 1 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        let taken = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM kb_connections WHERE slug = $1) AS "taken!: bool""#,
            &candidate,
        )
        .fetch_one(&mut *conn)
        .await?;
        if !taken {
            return Ok(candidate);
        }
    }

    Err(ApiError::Conflict(format!(
        "could not derive a free slug from name '{name}'"
    )))
}

/// Name the slug in a duplicate-provisioning conflict. `From<sqlx::Error> for ApiError` already
/// maps SQLSTATE 23505 to a bare `Conflict`, so this is purely about the message being useful.
fn map_duplicate(err: sqlx::Error, slug: &str) -> ApiError {
    if let sqlx::Error::Database(ref db) = err {
        if db.constraint() == Some("kb_connections_slug_key") {
            return ApiError::Conflict(format!("connection '{slug}' already exists"));
        }
    }
    ApiError::from(err)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use sqlx::PgPool;
    use uuid::Uuid;

    use temper_core::types::connection::ProvisionConnectionRequest;
    use temper_core::types::ids::ProfileId;
    use temper_core::types::team::TeamRole;

    use crate::error::ApiError;
    use crate::services::connection_service as svc;

    fn req(name: &str, owner_team_id: Option<Uuid>) -> ProvisionConnectionRequest {
        ProvisionConnectionRequest {
            provider: "github".into(),
            name: name.into(),
            owner_team_id,
            reach_granularity: Some("repo-set".into()),
            reach_covers: Some("acme/temper".into()),
        }
    }

    /// Seed a caller who is genuinely a system admin. `is_system_admin` IS ownership of the
    /// gating team, so being an admin means being seeded as one. The `temper-system` root team
    /// already exists in a migrated database (the L0 kernel migration creates it), so the team
    /// write is an upsert, not an insert.
    async fn seed_admin(pool: &PgPool) -> ProfileId {
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, 'conn-admin', 'conn-admin')",
            id,
        )
        .execute(pool)
        .await
        .expect("seed admin profile");

        let team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System') \
             ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name \
             RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("gating team");

        sqlx::query!("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
            .execute(pool)
            .await
            .expect("configure gating team");

        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) \
             VALUES ($1, $2, 'owner'::team_role) \
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
            team,
            id,
        )
        .execute(pool)
        .await
        .expect("join gating team as owner");

        ProfileId::from(id)
    }

    /// Seed a profile holding `role` on a fresh team, who is NOT a system admin.
    ///
    /// `invite_only` FIRST: `temper-system` carries `auto_join_role = 'watcher'` and
    /// `trg_sync_system_membership` fires on profile INSERT, so under the default `open` mode
    /// every new profile is auto-joined to the gating team. In open mode this fixture would be
    /// asserting about the trigger, not about our gate.
    async fn seed_team_member(
        pool: &PgPool,
        handle: &str,
        team_slug: &str,
        role: TeamRole,
    ) -> (ProfileId, Uuid) {
        sqlx::query!(
            "UPDATE kb_system_settings \
                SET access_mode = 'invite_only', gating_team_slug = 'temper-system'"
        )
        .execute(pool)
        .await
        .expect("invite_only with a configured gating team");

        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)",
            id,
            handle,
        )
        .execute(pool)
        .await
        .expect("seed profile");

        let team: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) \
             ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
            team_slug,
        )
        .fetch_one(pool)
        .await
        .expect("seed team");

        sqlx::query!(
            r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, $3)"#,
            team,
            id,
            role as TeamRole,
        )
        .execute(pool)
        .await
        .expect("seed membership");

        sqlx::query!(
            "DELETE FROM kb_team_members m USING kb_teams t \
              WHERE m.team_id = t.id AND t.slug = 'temper-system' AND m.profile_id = $1",
            id,
        )
        .execute(pool)
        .await
        .expect("no gating-team membership");

        (ProfileId::from(id), team)
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn provision_creates_profile_entity_context_and_row(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        assert_eq!(c.provider, "github");
        assert_eq!(c.slug, "acme-github");
        assert_eq!(c.registered_by_profile_id, *admin);

        // Born needs_credential, both capability tiers empty. It never silently pretends to be
        // more than it is.
        assert!(c.needs_credential(), "born needs_credential");
        assert!(!c.is_ledger_capable(), "no webhook events registered yet");
        assert!(!c.is_reach_capable(), "no tool manifest yet");

        // The emitter exists and is named `<handle>@webhook`.
        let entity_name = sqlx::query_scalar!(
            "SELECT name FROM kb_entities WHERE id = $1",
            c.emitter_entity_id
        )
        .fetch_one(&pool)
        .await
        .expect("emitter entity");
        assert!(
            entity_name.ends_with("@webhook"),
            "emitter is the webhook entity, got {entity_name}"
        );

        // The home context exists and is owned by the connection's own profile (teamless here).
        let (owner_table, owner_id) = sqlx::query!(
            r#"SELECT owner_table, owner_id FROM kb_contexts WHERE id = $1"#,
            c.home_context_id
        )
        .fetch_one(&pool)
        .await
        .map(|r| (r.owner_table, r.owner_id))
        .expect("home context");
        assert_eq!(owner_table, "kb_profiles");
        assert_eq!(owner_id, c.profile_id);
    }

    /// The direction-of-credential assertion, and the single easiest thing to get wrong here.
    /// `kb_machine_clients` answers "who may authenticate TO temper". A connection is the
    /// opposite: temper authenticating to a REMOTE system. GitHub holds no temper token, so a
    /// connection must have NO auth link and NO machine-client row.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_connection_has_no_auth_link_and_no_machine_client_row(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        let auth_links = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_profile_auth_links WHERE profile_id = $1",
            c.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("count auth links")
        .unwrap_or(0);
        assert_eq!(auth_links, 0, "a connection never authenticates TO temper");

        let machine_rows = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_machine_clients WHERE profile_id = $1",
            c.profile_id,
        )
        .fetch_one(&pool)
        .await
        .expect("count machine clients")
        .unwrap_or(0);
        assert_eq!(machine_rows, 0, "a connection is not a machine client");
    }

    /// The connection's profile gets exactly ONE entity — the webhook emitter — and none of the
    /// four `Surface::ALL` emitters. `webhook` is deliberately not a Surface variant: making it
    /// one would provision a webhook emitter onto every human profile.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_connection_profile_gets_only_the_webhook_emitter(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        let names: Vec<String> = sqlx::query_scalar!(
            "SELECT name FROM kb_entities WHERE profile_id = $1 ORDER BY name",
            c.profile_id,
        )
        .fetch_all(&pool)
        .await
        .expect("entities");

        assert_eq!(names.len(), 1, "exactly one emitter, got {names:?}");
        assert!(names[0].ends_with("@webhook"));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_team_owner_may_provision_for_their_own_team(pool: PgPool) {
        let (owner, team) = seed_team_member(&pool, "conn-owner", "acme", TeamRole::Owner).await;

        let c = svc::provision(&pool, owner, &req("Acme Linear", Some(team)))
            .await
            .expect("a team owner runs their own connections");
        assert_eq!(c.owner_team_id, Some(team));

        // The home context is owned by the TEAM, so read authz inherits for free.
        let owner_table = sqlx::query_scalar!(
            "SELECT owner_table FROM kb_contexts WHERE id = $1",
            c.home_context_id
        )
        .fetch_one(&pool)
        .await
        .expect("home context");
        assert_eq!(owner_table, "kb_teams");
    }

    /// Owner, not maintainer. `machine_authz::authorize` admits `Some(TeamRole::Owner)` only.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_mere_maintainer_cannot_provision_for_the_team(pool: PgPool) {
        let (maintainer, team) =
            seed_team_member(&pool, "conn-maintainer", "acme", TeamRole::Maintainer).await;

        let err = svc::provision(&pool, maintainer, &req("Acme Linear", Some(team)))
            .await
            .expect_err("a maintainer is not an owner");
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
    }

    /// Teamless fails closed. "No team to check" must never mean "nothing to deny".
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_teamless_connection_is_admin_only(pool: PgPool) {
        let (outsider, _team) =
            seed_team_member(&pool, "conn-outsider", "acme", TeamRole::Owner).await;

        let err = svc::provision(&pool, outsider, &req("Rogue GitHub", None))
            .await
            .expect_err("teamless is admin-only");
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
    }

    /// Auth before writes: a rejected provisioning must leave the DB completely unchanged — no
    /// orphaned profile, no orphaned entity, no orphaned context. Resolving authority *before*
    /// the transaction opens is what makes this assertable.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_rejected_provisioning_writes_nothing(pool: PgPool) {
        let (outsider, _team) =
            seed_team_member(&pool, "conn-outsider", "acme", TeamRole::Owner).await;

        let before = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
            .fetch_one(&pool)
            .await
            .expect("count")
            .unwrap_or(0);

        svc::provision(&pool, outsider, &req("Rogue GitHub", None))
            .await
            .expect_err("denied");

        let after = sqlx::query_scalar!("SELECT count(*) FROM kb_profiles")
            .fetch_one(&pool)
            .await
            .expect("count")
            .unwrap_or(0);
        assert_eq!(before, after, "a denied provisioning left a profile behind");

        let connections = sqlx::query_scalar!("SELECT count(*) FROM kb_connections")
            .fetch_one(&pool)
            .await
            .expect("count")
            .unwrap_or(0);
        assert_eq!(connections, 0);
    }

    /// Two connections can share a name; their slugs must still be distinct.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_slug_collision_suffixes(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        let a = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("first");
        let b = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("second");

        assert_eq!(a.slug, "acme-github");
        assert_eq!(b.slug, "acme-github-2");
    }

    /// Revocation denies the connection, and nothing else: the profile, emitter, and home context
    /// survive, because events already attributed to that emitter must keep resolving.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoke_marks_dead_but_keeps_the_emitter_resolvable(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        let revoked = svc::revoke(&pool, c.id, admin).await.expect("revoke");
        assert!(revoked.revoked_at.is_some());
        assert_eq!(revoked.revoked_by_profile_id, Some(*admin));

        let entity_still_there = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM kb_entities WHERE id = $1) AS "e!: bool""#,
            c.emitter_entity_id,
        )
        .fetch_one(&pool)
        .await
        .expect("entity");
        assert!(entity_still_there, "the emitter must keep resolving");

        // Hidden from the default list, visible when asked for.
        let visible = svc::list(&pool, admin, false).await.expect("list");
        assert!(visible.is_empty());
        let all = svc::list(&pool, admin, true).await.expect("list revoked");
        assert_eq!(all.len(), 1);
    }

    /// An empty scope must DENY. A team owner sees only connections owned by a team they own —
    /// never a teamless one, which is admin-only.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn list_scopes_to_teams_the_caller_owns(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let (owner, team) = seed_team_member(&pool, "conn-owner", "acme", TeamRole::Owner).await;

        svc::provision(&pool, admin, &req("Teamless GitHub", None))
            .await
            .expect("teamless");
        svc::provision(&pool, owner, &req("Acme Linear", Some(team)))
            .await
            .expect("team-owned");

        let seen = svc::list(&pool, owner, false).await.expect("list");
        assert_eq!(seen.len(), 1, "the teamless connection must not be visible");
        assert_eq!(seen[0].owner_team_id, Some(team));

        let admin_sees = svc::list(&pool, admin, false).await.expect("list");
        assert_eq!(admin_sees.len(), 2, "an admin sees every row");
    }
}

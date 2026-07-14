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

use temper_core::types::connection::{
    Connection, ConnectionCredential, ProvisionConnectionRequest,
};
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

/// Gate a mutation on an existing, **live** connection.
///
/// The machine gate keyed on the row's own owning team, plus the revoked check. A revoked
/// connection is dead — reactivation is a new provisioning, never an UPDATE — so a mutator refuses
/// one outright rather than issuing an UPDATE that silently matches no rows and reports success.
async fn authorize_live(pool: &PgPool, caller: ProfileId, id: Uuid) -> ApiResult<Connection> {
    let existing = get(pool, id).await?;
    machine_authz::authorize(pool, caller, existing.owner_team_id).await?;
    if existing.revoked_at.is_some() {
        return Err(ApiError::Conflict(format!(
            "connection '{}' is revoked; reactivation is a new provisioning",
            existing.slug
        )));
    }
    Ok(existing)
}

/// Attach the credential. This is what flips `needs_credential` off — and it flips off because
/// the column became non-NULL, not because a status was written. There is no status to write.
///
/// The stored value holds **no secret**: it names a broker and a connector the broker holds the
/// secret for. Nothing dispatches on `broker` yet; the adapter that does is a later chunk, which
/// is precisely why this one only records it.
pub async fn attach_credential(
    pool: &PgPool,
    caller: ProfileId,
    id: Uuid,
    credential: &ConnectionCredential,
) -> ApiResult<Connection> {
    authorize_live(pool, caller, id).await?;

    if credential.broker.trim().is_empty() {
        return Err(ApiError::BadRequest("broker must not be empty".into()));
    }
    if credential.connector.trim().is_empty() {
        return Err(ApiError::BadRequest("connector must not be empty".into()));
    }

    let value = serde_json::to_value(credential)
        .map_err(|e| ApiError::Internal(format!("failed to serialize credential: {e}")))?;

    sqlx::query!(
        r#"UPDATE kb_connections
              SET credential = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        value,
    )
    .execute(pool)
    .await?;

    get(pool, id).await
}

/// Register the remote event types. Non-empty ⇒ **ledger-capable**: events land, facts accrue.
///
/// Replaces the set wholesale. The registered set mirrors what the remote is actually configured
/// to send, so a merge would let a stale entry outlive the webhook it names — and a capability we
/// claim but do not have is exactly the silence invariant 6 forbids.
pub async fn set_webhook_events(
    pool: &PgPool,
    caller: ProfileId,
    id: Uuid,
    events: &[String],
) -> ApiResult<Connection> {
    authorize_live(pool, caller, id).await?;

    sqlx::query!(
        r#"UPDATE kb_connections
              SET webhook_events = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        events,
    )
    .execute(pool)
    .await?;

    get(pool, id).await
}

/// Declare the read-only remote tools. Non-empty ⇒ **reach-capable**: agents can read the remote
/// back, so judgment becomes possible.
///
/// The manifest is the evidence the provider is admissible at all — an empty one means judgment is
/// *impossible*, not merely unconfigured. Stored as a JSON array of tool names, which is what
/// `Connection::is_reach_capable` reads.
pub async fn set_tool_manifest(
    pool: &PgPool,
    caller: ProfileId,
    id: Uuid,
    tools: &[String],
) -> ApiResult<Connection> {
    authorize_live(pool, caller, id).await?;

    let value = serde_json::to_value(tools)
        .map_err(|e| ApiError::Internal(format!("failed to serialize tool manifest: {e}")))?;

    sqlx::query!(
        r#"UPDATE kb_connections
              SET tool_manifest = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        value,
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

    use temper_core::types::connection::{ConnectionCredential, ProvisionConnectionRequest};
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

    fn credential() -> ConnectionCredential {
        ConnectionCredential {
            broker: "vercel-connect".to_string(),
            connector: "conn_abc123".to_string(),
            installation: Some("inst_42".to_string()),
        }
    }

    /// The round-trip is the whole point of typing the column: what we write must come back as the
    /// same struct, or the broker seam cannot dispatch on `broker`. Asserted through
    /// `credential_typed`, not by eyeballing the raw JSON.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn attaching_a_credential_flips_needs_credential_and_round_trips(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");
        assert!(c.needs_credential(), "born needs_credential");

        let attached = svc::attach_credential(&pool, admin, c.id, &credential())
            .await
            .expect("attach");

        // It flips off because the COLUMN is non-NULL — there is no status to set.
        assert!(!attached.needs_credential());
        assert_eq!(
            attached
                .credential_typed()
                .expect("credential present")
                .expect("credential parses"),
            credential(),
            "the stored credential must round-trip through ConnectionCredential"
        );

        // Attaching a credential confers NEITHER capability tier. They are separately provisioned.
        assert!(!attached.is_ledger_capable());
        assert!(!attached.is_reach_capable());
    }

    /// Ledger-capable and reach-capable are independent. A connection may legitimately be
    /// ledger-only: events land, judgment is simply not yet possible — and it says so.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_two_capability_tiers_are_independently_settable(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        let events = vec!["pull_request".to_string(), "push".to_string()];
        let ledger_only = svc::set_webhook_events(&pool, admin, c.id, &events)
            .await
            .expect("set webhooks");
        assert!(ledger_only.is_ledger_capable(), "events land");
        assert!(
            !ledger_only.is_reach_capable(),
            "ledger-capable must NOT imply reach-capable — a ledger-only connection is legal, \
             and inert for judgment"
        );
        assert_eq!(ledger_only.webhook_events, events);

        let tools = vec!["get_pull_request".to_string()];
        let both = svc::set_tool_manifest(&pool, admin, c.id, &tools)
            .await
            .expect("set tools");
        assert!(both.is_reach_capable(), "judgment becomes possible");
        assert!(both.is_ledger_capable(), "and the webhook set survives");
        assert_eq!(
            both.tool_manifest,
            serde_json::json!(["get_pull_request"]),
            "the manifest is a JSON array of tool names — what is_reach_capable reads"
        );
    }

    /// The registered set MIRRORS what the remote is configured to send, so it replaces rather
    /// than merges. A merge would let a stale entry outlive the webhook it names — a claimed
    /// capability we do not have.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn setting_webhook_events_replaces_rather_than_merges(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        svc::set_webhook_events(&pool, admin, c.id, &["push".to_string()])
            .await
            .expect("first");
        let second = svc::set_webhook_events(&pool, admin, c.id, &["pull_request".to_string()])
            .await
            .expect("second");

        assert_eq!(
            second.webhook_events,
            vec!["pull_request".to_string()],
            "`push` must be gone, not merged"
        );
    }

    /// A revoked connection is dead: reactivation is a new provisioning, never an UPDATE. Every
    /// mutator must REFUSE one rather than issue an UPDATE that matches no rows and reports success.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_revoked_connection_refuses_every_mutation(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");
        svc::revoke(&pool, c.id, admin).await.expect("revoke");

        assert!(matches!(
            svc::attach_credential(&pool, admin, c.id, &credential()).await,
            Err(ApiError::Conflict(_))
        ));
        assert!(matches!(
            svc::set_webhook_events(&pool, admin, c.id, &["push".to_string()]).await,
            Err(ApiError::Conflict(_))
        ));
        assert!(matches!(
            svc::set_tool_manifest(&pool, admin, c.id, &["t".to_string()]).await,
            Err(ApiError::Conflict(_))
        ));

        // And nothing was written despite three attempts.
        let after = svc::get(&pool, c.id).await.expect("get");
        assert!(after.needs_credential());
        assert!(!after.is_ledger_capable());
        assert!(!after.is_reach_capable());
    }

    /// The mutators call `machine_authz::authorize` — the same gate as provisioning, keyed on the
    /// EXISTING row's owning team. A maintainer is not an owner.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_mere_maintainer_cannot_attach_a_credential(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let (maintainer, team) =
            seed_team_member(&pool, "conn-maint", "acme", TeamRole::Maintainer).await;

        let c = svc::provision(&pool, admin, &req("Acme GitHub", Some(team)))
            .await
            .expect("provision");

        assert!(matches!(
            svc::attach_credential(&pool, maintainer, c.id, &credential()).await,
            Err(ApiError::Forbidden)
        ));
        assert!(
            svc::get(&pool, c.id).await.expect("get").needs_credential(),
            "a rejected attach writes nothing"
        );
    }

    /// A credential naming no broker cannot be dispatched on, so it is not a credential. Reject it
    /// rather than storing a row that flips `needs_credential` off while being unusable.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_empty_broker_or_connector_is_rejected(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        let no_broker = ConnectionCredential {
            broker: "  ".to_string(),
            ..credential()
        };
        assert!(matches!(
            svc::attach_credential(&pool, admin, c.id, &no_broker).await,
            Err(ApiError::BadRequest(_))
        ));

        let no_connector = ConnectionCredential {
            connector: String::new(),
            ..credential()
        };
        assert!(matches!(
            svc::attach_credential(&pool, admin, c.id, &no_connector).await,
            Err(ApiError::BadRequest(_))
        ));

        assert!(
            svc::get(&pool, c.id).await.expect("get").needs_credential(),
            "neither rejection may leave the connection looking credentialed"
        );
    }
}

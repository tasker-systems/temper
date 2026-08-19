//! Persistence for `kb_subscriptions` — a team/context/cogmap subscribes to an aspect of a
//! connection.
//!
//! Authorization is a **two-leg gate, composed from existing seams** — no new predicate is
//! invented:
//!
//! 1. **The caller manages the authoring team.** `team_service::require_manage_on_team` —
//!    owner or maintainer, composed with the system-admin path at the authority layer. The
//!    authoring team is the team the caller names, NOT derived from the subscriber: `kb_cogmaps`
//!    has no owner team (only `kb_team_cogmaps` links), and `kb_contexts.owner_table` can be
//!    `kb_profiles` (no team). The caller names the team; the gate checks against it.
//!
//! 2. **The authoring team holds a read-reach grant on the connection.** A `kb_access_grants`
//!    row with `subject_table = 'kb_connections'`, `subject_id = <connection_id>`,
//!    `principal_table = 'kb_teams'`, `principal_id = <authoring_team_id>`, `can_read = true`.
//!    This consults the grant that B2's widening
//!    (`migrations/20260714000020_connection_reach_grants.sql`) made possible; it does not
//!    create a parallel permission. The connection's owning team is NOT consulted — owner ≠
//!    reach (`migrations/20260714000010_connections.sql:60-61`). Cross-team subscription is
//!    legal when reach was granted.
//!
//! Writes are admin-driven and rare. Nothing here emits a ledger event: declaring a
//! subscription is internal infra, not a receipt of anything external — the same discipline
//! as `connection_service` and `kb_machine_clients`.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::subscription::{
    CreateSubscriptionRequest, Subscription, SubscriptionSelector,
};

use crate::error::{ApiError, ApiResult};
use crate::services::{connection_service, team_service};

/// The admissible subscriber tables. Kept here, not in the migration's CHECK, so the service
/// layer can match on them without re-parsing strings. The migration's CHECK is the
/// enforcement; this const is the mirror the service layer dispatches on.
const SUBSCRIBER_TABLES: &[&str] = &["kb_contexts", "kb_cogmaps", "kb_teams"];

/// Load one subscription by its own id. Unauthorized: the internal primitive the post-insert
/// readback uses. Surface callers want [`get_for_caller`].
pub async fn get(pool: &PgPool, id: Uuid) -> ApiResult<Subscription> {
    sqlx::query_as!(
        Subscription,
        r#"SELECT id, subscriber_table, subscriber_id, authoring_team_id,
                  connection_id, selector,
                  created_by_profile_id, created, revoked_at, revoked_by_profile_id
             FROM kb_subscriptions WHERE id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("subscription not found or not readable".to_string()))
}

/// [`get`], gated on the authoring team (the caller must manage it).
pub async fn get_for_caller(pool: &PgPool, caller: ProfileId, id: Uuid) -> ApiResult<Subscription> {
    let sub = get(pool, id).await?;
    team_service::require_manage_on_team(pool, sub.authoring_team_id, caller).await?;
    Ok(sub)
}

/// List subscriptions visible to `caller`, newest first. Revoked rows are hidden unless asked
/// for. A system admin sees every row; a team manager sees only subscriptions whose authoring
/// team they manage. Optional `connection_id` filter.
pub async fn list(
    pool: &PgPool,
    caller: ProfileId,
    include_revoked: bool,
    connection_id: Option<Uuid>,
) -> ApiResult<Vec<Subscription>> {
    let is_admin = crate::services::access_service::is_system_admin(pool, caller).await?;

    sqlx::query_as!(
        Subscription,
        r#"SELECT id, subscriber_table, subscriber_id, authoring_team_id,
                  connection_id, selector,
                  created_by_profile_id, created, revoked_at, revoked_by_profile_id
             FROM kb_subscriptions s
            WHERE ($1 OR s.revoked_at IS NULL)
              AND ( $2
                    OR EXISTS (
                        SELECT 1 FROM kb_team_members tm
                         WHERE tm.team_id = s.authoring_team_id
                           AND tm.profile_id = $3
                           AND tm.role IN ('owner', 'maintainer')
                    ) )
              AND ( $4::uuid IS NULL OR s.connection_id = $4 )
            ORDER BY s.created DESC"#,
        include_revoked,
        is_admin,
        *caller,
        connection_id,
    )
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

/// Create a subscription. The two-leg gate runs before the INSERT; a rejected create writes
/// nothing.
///
/// The selector is deserialized into a `temper_core::types::subscription::SubscriptionSelector`
/// before storage, so an unknown `kind` or a malformed payload is a 400, not a silent untyped
/// JSON write. The typed selector is then re-serialized to JSONB for storage — the column is the
/// storage, the enum is the shape.
pub async fn create(
    pool: &PgPool,
    caller: ProfileId,
    req: &CreateSubscriptionRequest,
) -> ApiResult<Subscription> {
    // Validate the subscriber_table against the admissible set before anything else — a bad
    // table name is a 400, not a 403 (which is what the authz gate would return).
    if !SUBSCRIBER_TABLES.contains(&req.subscriber_table.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "subscriber_table must be one of {:?}, got '{}'",
            SUBSCRIBER_TABLES, req.subscriber_table
        )));
    }

    // For kb_teams subscribers, authoring_team_id must equal subscriber_id — a team subscribes
    // for itself, not on behalf of another team.
    if req.subscriber_table == "kb_teams" && req.authoring_team_id != req.subscriber_id {
        return Err(ApiError::BadRequest(
            "for kb_teams subscribers, authoring_team_id must equal subscriber_id".into(),
        ));
    }

    // Leg 1: the caller manages the authoring team. require_manage_on_team refuses a
    // nonexistent team as Forbidden (role_on_team returns None for a team_id no row carries),
    // so a bogus UUID is denied and never reaches the INSERT.
    team_service::require_manage_on_team(pool, req.authoring_team_id, caller).await?;

    // Leg 2: the authoring team holds a read-reach grant on the connection. This is the
    // authorization surface B2 made possible; it does not create a parallel permission.
    if !reach_grant_held(pool, req.connection_id, req.authoring_team_id).await? {
        return Err(ApiError::Forbidden);
    }

    // Validate the subscriber row exists and is linked to the authoring team (for
    // contexts/cogmaps). For kb_teams, the equality check above is sufficient (a team row that
    // exists is its own subscriber). This is a read, not an authz gate — it is a precondition
    // check that turns a bogus subscriber_id into a 400, not a silent FK-less write.
    validate_subscriber_link(
        pool,
        &req.subscriber_table,
        req.subscriber_id,
        req.authoring_team_id,
    )
    .await?;

    // A declaration that can never match is refused here, not disclosed forever after. See
    // `refuse_inert_declaration` for what it will and will not conclude.
    let conn = connection_service::get(pool, req.connection_id).await?;
    refuse_inert_declaration(&conn, &req.selector)?;

    // Serialize the typed selector to JSONB for storage. The `?` bound is needed because
    // sqlx's `Json<T>` bind maps to JSONB but the column is `serde_json::Value`-shaped in the
    // query_as! macro. We store the re-serialized value so the column carries the canonical
    // form.
    let selector_json = serde_json::to_value(&req.selector)
        .map_err(|e| ApiError::BadRequest(format!("selector serialization failed: {e}")))?;

    let id = sqlx::query_scalar!(
        r#"INSERT INTO kb_subscriptions
               (subscriber_table, subscriber_id, authoring_team_id,
                connection_id, selector, created_by_profile_id)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id"#,
        req.subscriber_table,
        req.subscriber_id,
        req.authoring_team_id,
        req.connection_id,
        selector_json,
        *caller,
    )
    .fetch_one(pool)
    .await
    .map_err(map_duplicate)?;

    get(pool, id).await
}

/// Refuse a declaration that cannot ever match.
///
/// A selector waiting on an event kind the connection is not registered to receive is inert by
/// construction, and temper knows it at declaration time with **no payload required**. Refusing is
/// the honest answer where disclosing-forever-after is not: saying no once beats explaining
/// inertness every time someone reads the subscription. This is the register's refusal face — a
/// well-formed act the system said no to — rather than a case of clause C12
/// (`a-silent-declaration-is-distinguishable-from-a-quiet-source`), which covers declarations that
/// *could* match and have not.
///
/// **It only concludes inertness where it can prove it, and the bar is deliberately high.**
/// `webhook_events` is temper's *record* of what was registered remotely, and that record can lag
/// the provider. So:
///
/// - An **empty** `webhook_events` proves nothing — it is the not-yet-ledger-capable state, not a
///   statement that the connection receives nothing forever. A connection may legitimately be
///   provisioned before its webhook registration lands. Never refuse on it.
/// - A selector naming **no** event types matches all of them by definition
///   ([`SubscriptionSelector::GitHubRepository`]'s empty `event_types` = match all), so there is no
///   intersection to be empty.
/// - Only when the connection declares what it receives AND the selector declares what it waits
///   for AND the two are disjoint is inertness proven.
///
/// The refusal **cites what it compared**, so a maintainer who knows the registration record is
/// stale can see exactly why it said no and go fix the record rather than guess.
fn refuse_inert_declaration(
    conn: &temper_core::types::connection::Connection,
    selector: &SubscriptionSelector,
) -> ApiResult<()> {
    // Only variants that name event types can be proven inert this way. The other variants match
    // on repo or project and are indifferent to the event kind, so the connection's registered
    // set says nothing about whether they can match.
    let SubscriptionSelector::GitHubRepository { event_types, .. } = selector else {
        return Ok(());
    };
    if conn.webhook_events.is_empty() || event_types.is_empty() {
        return Ok(());
    }
    if event_types
        .iter()
        .any(|wanted| conn.webhook_events.iter().any(|got| got == wanted))
    {
        return Ok(());
    }
    Err(ApiError::BadRequest(format!(
        "this declaration can never match: the selector waits for {:?}, and connection '{}' \
         is registered to receive {:?}. Nothing in the first set is in the second, so no \
         payload would ever reach this subscription. If the connection's registered set is \
         stale, update it first — this refusal compared the two sets and nothing else.",
        event_types, conn.slug, conn.webhook_events
    )))
}

/// Revoke a subscription. Idempotent in effect but not in record: a second revoke of an
/// already-revoked row is a no-op returning the existing row. Rows are never deleted — mirrors
/// `kb_connections`. A revoked subscription stops matching (chunk B's query filters
/// `revoked_at IS NULL`); the history stays, so a subscription that existed at intake is
/// resolvable at disposition time (the delivery row's research-corpus property).
pub async fn revoke(pool: &PgPool, caller: ProfileId, id: Uuid) -> ApiResult<Subscription> {
    // Auth before writes, keyed on the existing row's authoring team.
    let existing = get(pool, id).await?;
    team_service::require_manage_on_team(pool, existing.authoring_team_id, caller).await?;

    sqlx::query!(
        r#"UPDATE kb_subscriptions
              SET revoked_at = now(), revoked_by_profile_id = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        *caller,
    )
    .execute(pool)
    .await?;
    get(pool, id).await
}

/// Does `team_id` hold a read-reach grant on `connection_id`? The leg-2 check. Reads
/// `kb_access_grants` for the row B2's widening made possible; does not create a parallel
/// permission.
async fn reach_grant_held(pool: &PgPool, connection_id: Uuid, team_id: Uuid) -> ApiResult<bool> {
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(
             SELECT 1 FROM kb_access_grants
              WHERE subject_table = 'kb_connections' AND subject_id = $1
                AND principal_table = 'kb_teams' AND principal_id = $2
                AND can_read
           ) AS "e!: bool""#,
        connection_id,
        team_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

/// Validate that the subscriber row exists and (for contexts/cogmaps) is linked to the
/// authoring team. For `kb_teams`, the equality check in [`create`] already ensured
/// `authoring_team_id = subscriber_id`, and a team row that exists is its own subscriber —
/// no extra query needed. For `kb_contexts`/`kb_cogmaps`, the link is through
/// `kb_team_contexts`/`kb_team_cogmaps`.
async fn validate_subscriber_link(
    pool: &PgPool,
    subscriber_table: &str,
    subscriber_id: Uuid,
    authoring_team_id: Uuid,
) -> ApiResult<()> {
    match subscriber_table {
        "kb_teams" => {
            // The equality check in `create` already ensured authoring_team_id = subscriber_id.
            // A team row that exists is its own subscriber; a nonexistent team was already
            // refused by require_manage_on_team. Nothing to do.
            Ok(())
        }
        "kb_contexts" => {
            let linked = sqlx::query_scalar!(
                r#"SELECT EXISTS(
                     SELECT 1 FROM kb_team_contexts
                      WHERE context_id = $1 AND team_id = $2
                   ) AS "e!: bool""#,
                subscriber_id,
                authoring_team_id,
            )
            .fetch_one(pool)
            .await?;
            if linked {
                Ok(())
            } else {
                Err(ApiError::BadRequest(
                    "authoring_team_id is not linked to this context (kb_team_contexts)".into(),
                ))
            }
        }
        "kb_cogmaps" => {
            let linked = sqlx::query_scalar!(
                r#"SELECT EXISTS(
                     SELECT 1 FROM kb_team_cogmaps
                      WHERE cogmap_id = $1 AND team_id = $2
                   ) AS "e!: bool""#,
                subscriber_id,
                authoring_team_id,
            )
            .fetch_one(pool)
            .await?;
            if linked {
                Ok(())
            } else {
                Err(ApiError::BadRequest(
                    "authoring_team_id is not linked to this cogmap (kb_team_cogmaps)".into(),
                ))
            }
        }
        _ => Err(ApiError::BadRequest(format!(
            "subscriber_table must be one of {:?}, got '{}'",
            SUBSCRIBER_TABLES, subscriber_table
        ))),
    }
}

/// Map a unique-violation to a conflict error. The unique constraint is on
/// `(authoring_team_id, connection_id, selector)` — re-declaring the same selector is a
/// conflict, not a new subscription.
fn map_duplicate(err: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(ref db_err) = err {
        if db_err.is_unique_violation() {
            return ApiError::Conflict(
                "a live or revoked subscription with this (authoring_team, connection, selector) \
                 already exists; re-declaring is a conflict, not a new subscription"
                    .into(),
            );
        }
    }
    ApiError::from(err)
}

// ── tests ───────────────────────────────────────────────────────────────────
//
// Run the tests you wrote or changed, the neighbouring ones in the same file/crate, and
// anything that regenerates a committed artifact. CI owns the broad suites.
// (`cargo nextest run -p temper-services --features test-db subscription_service`)

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use temper_core::types::connection::ProvisionConnectionRequest;
    use temper_core::types::ids::ProfileId;
    use temper_core::types::subscription::SubscriptionSelector;

    const GITHUB_REPO: &str = "acme/temper";

    /// Seed a system admin and return its profile id. Follows the connection_service::seed_admin
    /// pattern: gating-team owner + approved standing + governance grant (the D11 model).
    async fn seed_admin(pool: &PgPool) -> ProfileId {
        let id = Uuid::now_v7();
        let handle = format!("admin-{id}");
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)",
            id,
            &handle,
            &handle,
        )
        .execute(pool)
        .await
        .expect("seed admin profile");

        // The caller must carry its `<handle>@web` emitter — provision via the production path.
        let mut conn = pool.acquire().await.expect("acquire");
        crate::services::profile_service::provision_profile_entities(&mut conn, id, &handle)
            .await
            .expect("provision caller emitters");
        drop(conn);

        // gating team + owner (is_system_admin reads governance, but the gating team still
        // needs to exist and the owner row is what the existing fixture pattern writes).
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

        // What actually confers admin-ness: approved standing + governance grant.
        crate::test_support::approved_admin(pool, id).await;

        ProfileId::from(id)
    }

    /// Seed a team with the given owner and return its id.
    async fn seed_team(pool: &PgPool, owner: ProfileId) -> Uuid {
        let team_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO kb_teams (id, slug, name)
               VALUES ($1, $2, $3)"#,
            team_id,
            format!("team-{team_id}"),
            format!("Team {team_id}"),
        )
        .execute(pool)
        .await
        .expect("seed team");
        sqlx::query!(
            r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, 'owner'::team_role)"#,
            team_id,
            *owner,
        )
        .execute(pool)
        .await
        .expect("seed team owner");
        team_id
    }

    /// Seed a second team and add `member` as a maintainer (manage-capable but not owner).
    async fn seed_team_with_maintainer(pool: &PgPool, maintainer: ProfileId) -> Uuid {
        let team_id = Uuid::now_v7();
        let slug = format!("team-{team_id}");
        sqlx::query!(
            r#"INSERT INTO kb_teams (id, slug, name)
               VALUES ($1, $2, $3)"#,
            team_id,
            &slug,
            &slug,
        )
        .execute(pool)
        .await
        .expect("seed team");
        // The admin is owner (so the team has one), and `maintainer` is maintainer.
        let admin = seed_admin(pool).await;
        sqlx::query!(
            r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, 'owner'::team_role)"#,
            team_id,
            *admin,
        )
        .execute(pool)
        .await
        .expect("seed team owner");
        sqlx::query!(
            r#"INSERT INTO kb_team_members (team_id, profile_id, role)
               VALUES ($1, $2, 'maintainer'::team_role)"#,
            team_id,
            *maintainer,
        )
        .execute(pool)
        .await
        .expect("seed team maintainer");
        team_id
    }

    /// Seed a context owned by `team_id` and return its id.
    async fn seed_context_owned_by_team(pool: &PgPool, team_id: Uuid) -> Uuid {
        let context_id = Uuid::now_v7();
        let slug = format!("ctx-{context_id}");
        sqlx::query!(
            r#"INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
               VALUES ($1, 'kb_teams', $2, $3, $4)"#,
            context_id,
            team_id,
            &slug,
            &slug,
        )
        .execute(pool)
        .await
        .expect("seed context");
        // Link the context to the team via kb_team_contexts.
        sqlx::query!(
            r#"INSERT INTO kb_team_contexts (context_id, team_id)
               VALUES ($1, $2)"#,
            context_id,
            team_id,
        )
        .execute(pool)
        .await
        .expect("seed team_contexts");
        context_id
    }

    /// Seed a cogmap and link it to `team_id` via kb_team_cogmaps; return the cogmap id.
    /// Creating a cogmap requires a telos_resource, which requires a resource row — `kb_resources`
    /// has `title` + `origin_uri` (no context_id, no slug).
    async fn seed_cogmap_linked_to_team(pool: &PgPool, team_id: Uuid) -> Uuid {
        let _context_id = seed_context_owned_by_team(pool, team_id).await;
        // kb_resources has title + origin_uri (NOT context_id/slug). Let the DB generate the id
        // and capture it via RETURNING.
        let resource_id: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id",
            "telos",
            "",
        )
        .fetch_one(pool)
        .await
        .expect("seed resource for cogmap telos");

        let cogmap_id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO kb_cogmaps (id, name, telos_resource_id)
               VALUES ($1, $2, $3)"#,
            cogmap_id,
            format!("Cogmap {cogmap_id}"),
            resource_id,
        )
        .execute(pool)
        .await
        .expect("seed cogmap");

        sqlx::query!(
            r#"INSERT INTO kb_team_cogmaps (cogmap_id, team_id)
               VALUES ($1, $2)"#,
            cogmap_id,
            team_id,
        )
        .execute(pool)
        .await
        .expect("seed team_cogmaps");
        cogmap_id
    }

    /// Seed a connection owned by `team_id` and return its id. Uses the connection_service
    /// provision path so the profile, entity, and home context are all created correctly.
    async fn seed_connection(
        pool: &PgPool,
        owner_team_id: Option<Uuid>,
        caller: ProfileId,
    ) -> Uuid {
        let req = ProvisionConnectionRequest {
            provider: "github".into(),
            name: format!("test-conn-{}", Uuid::now_v7()),
            owner_team_id,
            reach_granularity: None,
            reach_covers: None,
        };
        let conn = crate::services::connection_service::provision(pool, caller, &req)
            .await
            .expect("seed connection");
        conn.id
    }

    /// Grant read-reach on `connection_id` to `team_id`. Writes the kb_access_grants row B2 made
    /// possible. No affirmation needed — the connection declares no reach.
    async fn grant_reach(pool: &PgPool, caller: ProfileId, connection_id: Uuid, team_id: Uuid) {
        crate::services::connection_service::grant_reach(
            pool,
            caller,
            connection_id,
            team_id,
            None,
        )
        .await
        .expect("grant reach");
    }

    fn github_repo_selector() -> SubscriptionSelector {
        SubscriptionSelector::GitHubRepository {
            repo: GITHUB_REPO.into(),
            event_types: vec!["pull_request.merged".into()],
        }
    }

    fn req(
        subscriber_table: &str,
        subscriber_id: Uuid,
        authoring_team_id: Uuid,
        connection_id: Uuid,
    ) -> CreateSubscriptionRequest {
        CreateSubscriptionRequest {
            subscriber_table: subscriber_table.into(),
            subscriber_id,
            authoring_team_id,
            connection_id,
            selector: github_repo_selector(),
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn create_and_revoke_a_team_subscription_round_trips(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        // A team subscribes for itself: subscriber_table = kb_teams, subscriber_id = team,
        // authoring_team_id = team.
        let created = create(&pool, admin, &req("kb_teams", team, team, conn))
            .await
            .expect("create subscription");

        assert_eq!(created.subscriber_table, "kb_teams");
        assert_eq!(created.subscriber_id, team);
        assert_eq!(created.authoring_team_id, team);
        assert_eq!(created.connection_id, conn);
        assert!(created.revoked_at.is_none());

        // Revoke.
        let revoked = revoke(&pool, admin, created.id).await.expect("revoke");
        assert!(revoked.revoked_at.is_some());
        assert_eq!(revoked.revoked_by_profile_id, Some(*admin));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn create_refused_without_reach_grant(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        // NOTE: no grant_reach — the team does NOT hold a reach grant on the connection.

        let err = create(&pool, admin, &req("kb_teams", team, team, conn))
            .await
            .expect_err("should be forbidden");
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn create_refused_without_manage_role(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        // A stranger with no role on the team.
        let stranger = ProfileId::from(Uuid::now_v7());
        let stranger_handle = format!("stranger-{}", *stranger);
        sqlx::query!(
            r#"INSERT INTO kb_profiles (id, handle, display_name)
               VALUES ($1, $2, $3)"#,
            *stranger,
            &stranger_handle,
            &stranger_handle,
        )
        .execute(&pool)
        .await
        .expect("seed stranger profile");
        let mut acquired = pool.acquire().await.expect("acquire");
        crate::services::profile_service::provision_profile_entities(
            &mut acquired,
            *stranger,
            &stranger_handle,
        )
        .await
        .expect("provision stranger emitters");
        drop(acquired);
        crate::test_support::approve(&pool, *stranger).await;

        let err = create(&pool, stranger, &req("kb_teams", team, team, conn))
            .await
            .expect_err("should be forbidden");
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn cross_team_subscription_legal_when_reach_granted(pool: PgPool) {
        // The platform team owns the connection; a different team (the subscriber) is granted
        // reach, then subscribes. The connection's owning team is NOT consulted for the
        // subscription — owner ≠ reach.
        let admin = seed_admin(&pool).await;
        let platform_team = seed_team(&pool, admin).await;
        let subscriber_team = seed_team_with_maintainer(&pool, admin).await;
        let conn = seed_connection(&pool, Some(platform_team), admin).await;

        // The platform team grants reach to the subscriber team.
        grant_reach(&pool, admin, conn, subscriber_team).await;

        // A maintainer of the subscriber team creates the subscription.
        let created = create(
            &pool,
            admin,
            &req("kb_teams", subscriber_team, subscriber_team, conn),
        )
        .await
        .expect("cross-team subscription should succeed");

        assert_eq!(created.authoring_team_id, subscriber_team);
        assert_eq!(created.connection_id, conn);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn subscription_against_ledger_only_connection_is_legal(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        // A connection with no webhook_events and no tool_manifest — ledger-only (in fact,
        // born needs_credential and both tiers empty). A subscription against it is legal and
        // stored — it says "inert for judgment."
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        let created = create(&pool, admin, &req("kb_teams", team, team, conn))
            .await
            .expect("ledger-only subscription should be legal");

        assert_eq!(created.connection_id, conn);
        assert!(created.revoked_at.is_none());
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoke_but_never_delete(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        let created = create(&pool, admin, &req("kb_teams", team, team, conn))
            .await
            .expect("create");

        let revoked = revoke(&pool, admin, created.id).await.expect("revoke");
        assert!(revoked.revoked_at.is_some());

        // The row is still there — never deleted.
        let still_there = get(&pool, created.id)
            .await
            .expect("row should still exist");
        assert_eq!(still_there.id, created.id);
        assert!(still_there.revoked_at.is_some());

        // Double revoke is a no-op returning the existing row (first revoker is the truth).
        let double = revoke(&pool, admin, created.id)
            .await
            .expect("double revoke");
        assert_eq!(double.revoked_by_profile_id, revoked.revoked_by_profile_id);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn unique_constraint_rejects_duplicate_selector(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        let _first = create(&pool, admin, &req("kb_teams", team, team, conn))
            .await
            .expect("first create");

        let err = create(&pool, admin, &req("kb_teams", team, team, conn))
            .await
            .expect_err("duplicate should conflict");
        assert!(matches!(err, ApiError::Conflict(_)), "got {err:?}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn distinct_selectors_against_same_connection_are_distinct(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        let first = create(&pool, admin, &req("kb_teams", team, team, conn))
            .await
            .expect("first create");

        // Same connection, same team, different selector (different repo).
        let second_req = CreateSubscriptionRequest {
            subscriber_table: "kb_teams".into(),
            subscriber_id: team,
            authoring_team_id: team,
            connection_id: conn,
            selector: SubscriptionSelector::GitHubRepository {
                repo: "acme/other-repo".into(),
                event_types: vec![],
            },
        };
        let second = create(&pool, admin, &second_req)
            .await
            .expect("second create with different selector");

        assert_ne!(first.id, second.id);
        assert_eq!(first.connection_id, second.connection_id);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn kb_teams_subscriber_requires_authoring_team_equals_subscriber(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team_a = seed_team(&pool, admin).await;
        let team_b = seed_team_with_maintainer(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team_a), admin).await;
        grant_reach(&pool, admin, conn, team_b).await;

        // Try to subscribe as team_b but with authoring_team_id = team_a — should be a 400.
        let err = create(&pool, admin, &req("kb_teams", team_b, team_a, conn))
            .await
            .expect_err("authoring_team != subscriber for kb_teams should be 400");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn kb_contexts_subscriber_validates_team_link(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        let context_id = seed_context_owned_by_team(&pool, team).await;

        let created = create(&pool, admin, &req("kb_contexts", context_id, team, conn))
            .await
            .expect("context subscription should succeed");

        assert_eq!(created.subscriber_table, "kb_contexts");
        assert_eq!(created.subscriber_id, context_id);
        assert_eq!(created.authoring_team_id, team);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn kb_contexts_subscriber_refused_when_team_not_linked(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let other_team = seed_team_with_maintainer(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, other_team).await;

        // Create a context owned by `team`, but try to subscribe with `other_team` as the
        // authoring team — other_team is NOT linked to this context.
        let context_id = seed_context_owned_by_team(&pool, team).await;

        let err = create(
            &pool,
            admin,
            &req("kb_contexts", context_id, other_team, conn),
        )
        .await
        .expect_err("unlinked team should be 400");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn kb_cogmaps_subscriber_validates_team_link(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        let cogmap_id = seed_cogmap_linked_to_team(&pool, team).await;

        let created = create(&pool, admin, &req("kb_cogmaps", cogmap_id, team, conn))
            .await
            .expect("cogmap subscription should succeed");

        assert_eq!(created.subscriber_table, "kb_cogmaps");
        assert_eq!(created.subscriber_id, cogmap_id);
        assert_eq!(created.authoring_team_id, team);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn list_scopes_to_teams_the_caller_manages(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team_a = seed_team(&pool, admin).await;
        let team_b = seed_team_with_maintainer(&pool, admin).await;
        let conn_a = seed_connection(&pool, Some(team_a), admin).await;
        let conn_b = seed_connection(&pool, Some(team_b), admin).await;
        grant_reach(&pool, admin, conn_a, team_a).await;
        grant_reach(&pool, admin, conn_b, team_b).await;

        let _sub_a = create(&pool, admin, &req("kb_teams", team_a, team_a, conn_a))
            .await
            .expect("sub a");
        let _sub_b = create(&pool, admin, &req("kb_teams", team_b, team_b, conn_b))
            .await
            .expect("sub b");

        // admin is system admin → sees all.
        let all = list(&pool, admin, false, None).await.expect("admin list");
        assert_eq!(all.len(), 2);

        // A stranger with no role on either team sees none. Seed a stranger with system access
        // (so they can call list at all) but no team role.
        let stranger = ProfileId::from(Uuid::now_v7());
        let stranger_handle = format!("stranger-{}", *stranger);
        sqlx::query!(
            r#"INSERT INTO kb_profiles (id, handle, display_name)
               VALUES ($1, $2, $3)"#,
            *stranger,
            &stranger_handle,
            &stranger_handle,
        )
        .execute(&pool)
        .await
        .expect("seed stranger");
        let mut acquired = pool.acquire().await.expect("acquire");
        crate::services::profile_service::provision_profile_entities(
            &mut acquired,
            *stranger,
            &stranger_handle,
        )
        .await
        .expect("provision stranger emitters");
        drop(acquired);
        crate::test_support::approve(&pool, *stranger).await;

        let none = list(&pool, stranger, false, None)
            .await
            .expect("stranger list");
        assert!(none.is_empty(), "stranger should see no subscriptions");
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn bad_subscriber_table_is_bad_request(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let team = seed_team(&pool, admin).await;
        let conn = seed_connection(&pool, Some(team), admin).await;
        grant_reach(&pool, admin, conn, team).await;

        let err = create(
            &pool,
            admin,
            &CreateSubscriptionRequest {
                subscriber_table: "kb_resources".into(), // not admissible
                subscriber_id: team,
                authoring_team_id: team,
                connection_id: conn,
                selector: github_repo_selector(),
            },
        )
        .await
        .expect_err("bad subscriber_table should be 400");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }
}

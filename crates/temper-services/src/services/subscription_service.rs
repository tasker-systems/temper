//! Persistence for `kb_subscriptions` — a team/context/cogmap subscribes to an aspect of a
//! connection.
//!
//! Authorization is a **three-leg gate, composed from existing seams** — no new predicate is
//! invented:
//!
//! 1. **The caller manages the authoring team, or is a system admin.**
//!    `crate::authz::SubscriptionAuthority` — `team_service::can_manage` (owner or maintainer)
//!    composed with `access_service::is_system_admin`, at the authority layer where every other
//!    scoped gate in this crate lives. The
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
//! 3. **The caller authors the subscriber** (a context or map). Deliveries stamp `touches` onto the
//!    subscriber, so declaring one writes onto that object, and the object's own write regime
//!    decides: `context_authorable_by_profile` / `cogmap_authorable_by_profile`. A link to the team
//!    is not enough, because sharing or binding into a team can confer read only.
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

use crate::auth::HumanPrincipal;
use crate::authz::{authorize, Principal, SubscriptionAuthority, SubscriptionControlAuthority};
use crate::error::{ApiError, ApiResult};
use crate::services::{connection_service, context_service, team_service};

/// The admissible subscriber tables. Kept here, not in the migration's CHECK, so the service
/// layer can match on them without re-parsing strings. The migration's CHECK is the
/// enforcement; this const is the mirror the service layer dispatches on.
const SUBSCRIBER_TABLES: &[&str] = &["kb_contexts", "kb_cogmaps", "kb_teams"];

/// The refusal for a subscription the caller cannot see — absent, or present but outside the
/// caller's authority. One sentence for both, on purpose: `get` renders it for a missing row and
/// `authz::SubscriptionControlAuthority` for a denied one, so a caller probing ids cannot tell the
/// two apart.
pub(crate) const SUBSCRIPTION_REFUSAL: &str = "subscription not found or not readable";

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
    .ok_or_else(|| ApiError::NotFound(SUBSCRIPTION_REFUSAL.to_string()))
}

/// [`get`], gated on the authoring team: the caller manages it, or is a system admin. A caller
/// outside that authority is refused exactly as a missing id is (`SUBSCRIPTION_REFUSAL`) — see
/// `authz::SubscriptionControlAuthority`.
pub async fn get_for_caller(
    pool: &PgPool,
    authed: &HumanPrincipal,
    id: Uuid,
) -> ApiResult<Subscription> {
    authorize::<SubscriptionControlAuthority>(pool, Principal::Human(authed), id).await?;
    get(pool, id).await
}

/// List subscriptions visible to `caller`, newest first. Revoked rows are hidden unless asked
/// for. A system admin sees every row; a team manager sees only subscriptions whose authoring
/// team they manage. Optional `connection_id` filter.
///
/// **Deliberately not a `SubscriptionAuthority` gate.** A `ScopedAuthority` answers "may this
/// caller act on *this* subject"; a listing has no single subject to be scoped to — it is a
/// visibility predicate over many authoring teams, evaluated in SQL. The two arms below are the
/// same two the authority names, which is the point: this function and that type must agree, and
/// they agree by being the same disjunction, not by sharing a call.
pub async fn list(
    pool: &PgPool,
    authed: &HumanPrincipal,
    include_revoked: bool,
    connection_id: Option<Uuid>,
) -> ApiResult<Vec<Subscription>> {
    let caller = ProfileId::from(authed.profile().id);
    let is_admin = crate::authz::Principal::Human(authed)
        .system_admin(pool)
        .await?
        .is_some();

    sqlx::query_as!(
        Subscription,
        r#"SELECT id, subscriber_table, subscriber_id, authoring_team_id,
                  connection_id, selector,
                  created_by_profile_id, created, revoked_at, revoked_by_profile_id
             FROM kb_subscriptions s
            WHERE ($1 OR s.revoked_at IS NULL)
              -- The admin arm stops at personal space (task `01a191b4`): a subscription a
              -- personal team authors, or one delivering into a personal context, is its owner's.
              AND ( ($2 AND NOT EXISTS (SELECT 1 FROM kb_teams pt
                                         WHERE pt.id = s.authoring_team_id
                                           AND profile_is_person(pt.personal_of)
                                           AND pt.personal_of <> $3)
                        AND NOT (s.subscriber_table = 'kb_contexts'
                                 AND COALESCE(context_personal_owner(s.subscriber_id) <> $3,
                                              false)))
                    OR EXISTS (
                        SELECT 1 FROM kb_team_members tm
                          JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
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

/// Create a subscription. The three-leg gate runs before the INSERT; a rejected create writes
/// nothing.
///
/// The selector arrives typed (`temper_core::types::subscription::SubscriptionSelector`), so an
/// unknown `kind` or a malformed payload is refused by the HTTP surface's JSON extractor (a 422)
/// before this runs — never a silent untyped JSON write. The typed selector is re-serialized to
/// JSONB for storage — the column is the storage, the enum is the shape.
pub async fn create(
    pool: &PgPool,
    authed: &HumanPrincipal,
    req: &CreateSubscriptionRequest,
) -> ApiResult<Subscription> {
    let caller = ProfileId::from(authed.profile().id);
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

    // Leg 1: the caller manages the authoring team, or is a system admin. A nonexistent team
    // resolves to the denial arm (role_on_team returns None for a team_id no row carries), so a
    // bogus UUID is refused here and never reaches the INSERT.
    let authorized =
        authorize::<SubscriptionAuthority>(pool, Principal::Human(authed), req.authoring_team_id)
            .await?;
    let authoring_team = authorized.subject();
    let on_admin_arm = authorized.authority() == SubscriptionAuthority::SystemAdmin;
    // A subscription delivering into a personal context writes into personal space, which only its
    // owner acts on (task `01a191b4`): the admin arm above does not carry it. A manager of the
    // authoring team the context was shared into passes this check, but the share alone is not
    // enough: leg 3 below also requires them to author the context, which for someone else's
    // personal context means a write grant its owner gave.
    let into_others_personal = req.subscriber_table == "kb_contexts"
        && context_service::is_others_personal_context(pool, req.subscriber_id, caller).await?;
    if into_others_personal
        && !matches!(
            team_service::role_on_team(pool, authoring_team, caller).await?,
            Some(role) if team_service::can_manage(role)
        )
    {
        return Err(ApiError::Forbidden);
    }

    // Leg 2: the authoring team holds a read-reach grant on the connection. This is the
    // authorization surface B2 made possible; it does not create a parallel permission.
    //
    // **Leg 2 binds a system admin too.** The admin leg above widens WHO may declare on behalf of
    // a team; it does not exempt the team from needing reach. An admin declaring a subscription
    // against a connection the authoring team cannot reach would create a row that is inert by
    // construction — the thing `refuse_inert_declaration` below exists to prevent from the other
    // direction.
    if !reach_grant_held(pool, req.connection_id, authoring_team).await? {
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
        authoring_team,
    )
    .await?;

    // Leg 3: the caller authors the subscriber. Every delivery appends an event that `touches` the
    // subscriber, so declaring a subscription writes onto that object, and the bar is the object's
    // own write regime, not the caller's role on a team it is linked to. A context or map shared
    // into the authoring team for reading is linked, and still refused here.
    refuse_unless_subscriber_authorable(pool, caller, &req.subscriber_table, req.subscriber_id)
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

    // The caller's admin standing, asked again under the governance lock on the insert's
    // transaction and held to its commit. Two answers above may have rested on it: leg 1's admin
    // arm, and leg 3, whose regime for an admin-only map (or an admin-only context) is
    // `is_system_admin` itself, including for a manager of the authoring team who passed leg 1 on
    // the team arm. A demoted admin on leg 1's admin arm is refused with the gate's answer to them
    // as they now stand.
    let mut tx = pool.begin().await?;
    // The team-manager arm, likewise read on the pool, is asked again under the authoring team's
    // row lock (the lock every membership writer takes first), ahead of the governance lock, in the
    // order the team acts take the two. Deliveries carry what the authoring team reaches on the
    // connection into a subscriber the caller authors, so a manager removed from the team mid-act
    // would keep reading it there. A removal or demotion committed since refuses; one in flight
    // makes this wait, then refuses. A caller the pool gate now refuses gets its answer; an admin
    // who also managed the team (leg 1 resolves the team arm first) is refused too, and a retry
    // decides on the admin arm. It is
    // asked wherever the pool decision rested on managing the team: leg 1's team arm, and the
    // personal-context bar above, which an admin passes only as a manager.
    if (!on_admin_arm || into_others_personal)
        && !matches!(
            crate::authz::team_membership::role_under_lock(&mut tx, authoring_team, caller, &[])
                .await?,
            Some(role) if team_service::can_manage(role)
        )
    {
        drop(tx);
        authorize::<SubscriptionAuthority>(pool, Principal::Human(authed), req.authoring_team_id)
            .await?;
        return Err(ApiError::Forbidden);
    }
    let admin_now = crate::auth::is_system_admin_locked(&mut tx, caller).await?;
    if on_admin_arm && !admin_now {
        drop(tx);
        authorize::<SubscriptionAuthority>(pool, Principal::Human(authed), req.authoring_team_id)
            .await?;
        return Err(ApiError::Forbidden);
    }
    // Leg 3 again, on this transaction: its admin-regime answer now reads the governance row the
    // lock above holds (or found absent), so a demotion cannot commit between it and the insert.
    refuse_unless_subscriber_authorable(&mut *tx, caller, &req.subscriber_table, req.subscriber_id)
        .await?;
    let id = sqlx::query_scalar!(
        r#"INSERT INTO kb_subscriptions
               (subscriber_table, subscriber_id, authoring_team_id,
                connection_id, selector, created_by_profile_id)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id"#,
        req.subscriber_table,
        req.subscriber_id,
        authoring_team,
        req.connection_id,
        selector_json,
        *caller,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(map_duplicate)?;
    tx.commit().await?;

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
pub async fn revoke(pool: &PgPool, authed: &HumanPrincipal, id: Uuid) -> ApiResult<Subscription> {
    let caller = ProfileId::from(authed.profile().id);
    // Auth before writes, keyed on the existing row's authoring team. A refusal is
    // indistinguishable from a missing id.
    let on_admin_arm =
        authorize::<SubscriptionControlAuthority>(pool, Principal::Human(authed), id)
            .await?
            .authority()
            == SubscriptionControlAuthority::SystemAdmin;

    // The admin arm asked again under the governance lock, on the update's transaction. A demoted
    // admin is refused as a caller with no authority over the row is: the missing-id answer.
    let mut tx = pool.begin().await?;
    if on_admin_arm && !crate::auth::is_system_admin_locked(&mut tx, caller).await? {
        drop(tx);
        authorize::<SubscriptionControlAuthority>(pool, Principal::Human(authed), id).await?;
        return Err(ApiError::NotFound(SUBSCRIPTION_REFUSAL.to_string()));
    }
    sqlx::query!(
        r#"UPDATE kb_subscriptions
              SET revoked_at = now(), revoked_by_profile_id = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        *caller,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
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
            // refused by the SubscriptionAuthority gate. Nothing to do.
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

/// Refuse unless `caller` authors the subscriber, by that object's own regime:
/// `context_authorable_by_profile` for a context, `cogmap_authorable_by_profile` for a map (which
/// leaves a map in the admin-only regime to a system admin). There is no system-admin exemption:
/// the admin leg of [`SubscriptionAuthority`] widens who may act for a team, and a system admin
/// authors an ordinary map or context the way everyone else does. A team subscriber is the
/// authoring team itself, which leg 1 already proved the caller manages.
///
/// Runs after [`validate_subscriber_link`], so the subscriber is linked to a team the caller
/// manages, and the caller can therefore read it: a `403` here discloses nothing they cannot see.
async fn refuse_unless_subscriber_authorable(
    conn: impl sqlx::PgExecutor<'_>,
    caller: ProfileId,
    subscriber_table: &str,
    subscriber_id: Uuid,
) -> ApiResult<()> {
    let authorable = match subscriber_table {
        "kb_teams" => return Ok(()),
        "kb_contexts" => {
            sqlx::query_scalar!(
                r#"SELECT context_authorable_by_profile($1, $2) AS "a!: bool""#,
                *caller,
                subscriber_id,
            )
            .fetch_one(conn)
            .await?
        }
        "kb_cogmaps" => {
            sqlx::query_scalar!(
                r#"SELECT cogmap_authorable_by_profile($1, $2) AS "a!: bool""#,
                *caller,
                subscriber_id,
            )
            .fetch_one(conn)
            .await?
        }
        _ => false,
    };
    if authorable {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
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
mod tests;

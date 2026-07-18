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
    AttachCredentialResponse, Connection, ConnectionCredential, CredentialVerification,
    ProvisionConnectionRequest,
};
use temper_core::types::ids::ProfileId;
use temper_workflow::operations::sluggify;

use crate::broker::{BrokerError, CredentialBroker, MintRequest, MintSubject};
use crate::error::{ApiError, ApiResult};
use crate::services::access_service::InsertGrantParams;
use crate::services::{access_service, machine_authz, profile_service};

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
                  reach_affirmed_by, reach_affirmed_at, reach_affirmation,
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
                  reach_affirmed_by, reach_affirmed_at, reach_affirmation,
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
///
/// **Revocation is a temper-side act; it does NOT reach the provider.** Setting `revoked_at`
/// stops temper from minting *new* tokens for this connection, but any token *already* minted
/// stays valid at the remote until it expires — the broker's revoke, where it exists at all, is
/// best-effort (Vercel Connect's own CLI warns that provider-side token revocation may be
/// unsupported). So this is not "the remote can no longer be reached"; it is "temper will not mint
/// for it again." Callers surfacing revocation must say so rather than imply an instantaneous cutoff
/// (invariant 6: absence of a capability — here, immediate remote revocation — must never be
/// silently assumed present).
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

/// Attach the credential, minting once to prove the connector is live. This flips
/// `needs_credential` off — and it flips off because the column became non-NULL,
/// not because a status was written. There is no status to write.
///
/// The stored value holds **no secret**: it names a broker and a connector the
/// broker holds the secret for.
///
/// **The mint is the seam's real caller.** A connector that a broker *proves* bad
/// (`Unauthorized`) fails the attach loudly here, rather than silently at S3.
/// Every other outcome — consent pending, no broker configured, a transient
/// failure — persists the credential with a visible `note` (invariant 6: an
/// absent capability is stated, never silent). The mint response's `metadata` is
/// surfaced as `observed_reach` next to the connection's *declared* reach, so a
/// reviewer sees the gap; there is no computed `exceeds` bool (B3 acknowledges).
pub async fn attach_credential(
    pool: &PgPool,
    broker: &dyn CredentialBroker,
    caller: ProfileId,
    id: Uuid,
    credential: &ConnectionCredential,
) -> ApiResult<AttachCredentialResponse> {
    authorize_live(pool, caller, id).await?;

    if credential.broker.trim().is_empty() {
        return Err(ApiError::BadRequest("broker must not be empty".into()));
    }
    if credential.connector.trim().is_empty() {
        return Err(ApiError::BadRequest("connector must not be empty".into()));
    }

    // Mint once to verify. Auth is already checked; this is a read against the
    // broker, before any persistence. Only a proven-bad credential blocks.
    let verification = verify_by_minting(broker, credential).await?;

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

    let connection = get(pool, id).await?;
    Ok(AttachCredentialResponse {
        connection,
        verification,
    })
}

/// Mint once and classify the outcome. `Err` here means the attach must be
/// rejected (the credential was proven bad); `Ok(verification)` means persist,
/// carrying what was (or could not be) observed.
async fn verify_by_minting(
    broker: &dyn CredentialBroker,
    credential: &ConnectionCredential,
) -> ApiResult<CredentialVerification> {
    let outcome = broker
        .mint(MintRequest {
            credential,
            subject: MintSubject::App,
            scopes: vec![],
        })
        .await;

    let verification = match outcome {
        Ok(minted) => CredentialVerification {
            verified: true,
            observed_reach: Some(minted.reach.raw),
            note: None,
        },
        // The ONE blocking case: the broker proved the credential bad.
        Err(BrokerError::Unauthorized(msg)) => {
            return Err(ApiError::BadRequest(format!(
                "connector rejected the credential: {msg}"
            )));
        }
        // Everything else records the credential but says, out loud, that it is
        // not verified and why.
        Err(BrokerError::NeedsConsent { authorize_url }) => CredentialVerification {
            verified: false,
            observed_reach: None,
            note: Some(match authorize_url {
                Some(url) => format!("consent pending — authorize at {url}"),
                None => "consent pending — the connector needs an OAuth consent".into(),
            }),
        },
        Err(BrokerError::NotConfigured) => CredentialVerification {
            verified: false,
            observed_reach: None,
            note: Some("not verified — no credential broker is configured".into()),
        },
        Err(e) => CredentialVerification {
            verified: false,
            observed_reach: None,
            note: Some(format!("could not verify — {e}")),
        },
    };
    Ok(verification)
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

/// Grant a TEAM read-reach on a connection. Owning a connection is not reaching it — this writes a
/// `kb_access_grants` row (`subject_table = 'kb_connections'`) so the named team's members inherit
/// READ on what the connection receives. Reach is read-only: no write, delete, or grant is conferred.
///
/// Auth before writes, keyed on the connection's own owning team: `machine_authz::authorize` — a
/// system admin, or the OWNER of the owning team; a teamless connection fails closed. This is the
/// SAME policy as every other connection mutator, CALLED not restated. It deliberately does NOT
/// route through `access_service::can_administer_grant`, whose `can_grant` seam has no bootstrap
/// holder for a connection subject — the whole point is one policy, called once.
///
/// A connection whose reach is *declared* (`Connection::declares_reach`) carries a coarse remote
/// reach that may exceed the granted team's temper reach — remote and temper scope are
/// incommensurable, so there is no computed `exceeds` bool, only the honest declaration. Binding
/// such a connection to a team is therefore not allowed to proceed silently: `affirm_reach` must
/// carry the stated reason, or the grant FAILS. Affirming records who/when/why on the connection —
/// it makes the asymmetry declared and reviewable; it does NOT resolve it or narrow the remote
/// reach. The four cases:
///
/// - declares reach, no affirmation → `Conflict`, writes nothing (naming the declared reach and the fix).
/// - declares reach, affirmation → one transaction: stamp the affirmation, then insert the grant.
/// - declares no reach, affirmation → `BadRequest` (the flag is inapplicable — honest over lenient).
/// - declares no reach, no affirmation → the plain grant.
///
/// Auth stays FIRST — affirmation never bypasses authorization.
pub async fn grant_reach(
    pool: &PgPool,
    caller: ProfileId,
    connection_id: Uuid,
    team_id: Uuid,
    affirm_reach: Option<String>,
) -> ApiResult<Connection> {
    let connection = get(pool, connection_id).await?;
    machine_authz::authorize(pool, caller, connection.owner_team_id).await?;

    match (connection.declares_reach(), affirm_reach) {
        // Declared reach, unaffirmed: refuse, naming the declared reach and the fix. Write nothing.
        (true, None) => Err(ApiError::Conflict(format!(
            "connection '{}' declares remote reach ({}) that may exceed the team's temper reach; \
             binding it to a team must be affirmed as intentional — re-run with \
             --affirm-reach \"<why this is intended>\"",
            connection.slug,
            reach_descriptor(&connection),
        ))),
        // Declared reach, affirmed: stamp the affirmation and insert the grant atomically —
        // never affirmation-without-grant or grant-without-affirmation.
        (true, Some(rationale)) => {
            let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let mut tx = pool.begin().await?;
            sqlx::query!(
                r#"UPDATE kb_connections
                      SET reach_affirmed_by = $2, reach_affirmed_at = now(), reach_affirmation = $3
                    WHERE id = $1"#,
                connection_id,
                *caller,
                rationale,
            )
            .execute(&mut *tx)
            .await?;
            access_service::insert_grant(
                &mut tx,
                &reach_grant_params(connection_id, team_id, caller),
                emitter,
            )
            .await?;
            // (`&mut tx` coerces to `&mut PgConnection` via Transaction's DerefMut.)
            tx.commit().await?;
            // The top-of-fn `connection` is stale post-UPDATE; re-read so the returned struct
            // carries the populated affirmation fields.
            get(pool, connection_id).await
        }
        // No declared reach, but an affirmation was passed: the flag is inapplicable. Honest over
        // lenient — do not silently ignore it. Write nothing.
        (false, Some(_)) => Err(ApiError::BadRequest(
            "this connection declares no reach; --affirm-reach is not applicable".into(),
        )),
        // No declared reach, no affirmation: the plain grant.
        (false, None) => {
            let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let mut conn = pool.acquire().await?;
            access_service::insert_grant(
                &mut conn,
                &reach_grant_params(connection_id, team_id, caller),
                emitter,
            )
            .await?;
            Ok(connection)
        }
    }
}

/// The declared-reach descriptor for the affirmation-required message. Built from whichever of
/// `reach_granularity` / `reach_covers` is populated (at least one is, given `declares_reach`).
fn reach_descriptor(connection: &Connection) -> String {
    match (
        connection.reach_granularity.as_deref(),
        connection.reach_covers.as_deref(),
    ) {
        (Some(g), Some(c)) => format!("{g}: {c}"),
        (Some(g), None) => g.to_string(),
        (None, Some(c)) => c.to_string(),
        (None, None) => "declared".to_string(),
    }
}

/// The columns of a team's read-reach grant on a connection — read-only, conferring no write,
/// delete, or grant. Built in one place so the two grant paths (affirmed, unaffirmed) cannot drift.
fn reach_grant_params(
    connection_id: Uuid,
    team_id: Uuid,
    granted_by: ProfileId,
) -> InsertGrantParams {
    InsertGrantParams {
        subject_table: "kb_connections".into(),
        subject_id: connection_id,
        principal_table: "kb_teams".into(),
        principal_id: team_id,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
        granted_by_profile_id: *granted_by,
    }
}

/// Revoke a team's read-reach on a connection. Same gate as [`grant_reach`]; deletes the
/// `kb_access_grants` row by its 4-tuple (absent ⇒ idempotent no-op).
pub async fn revoke_reach(
    pool: &PgPool,
    caller: ProfileId,
    connection_id: Uuid,
    team_id: Uuid,
) -> ApiResult<Connection> {
    let connection = get(pool, connection_id).await?;
    machine_authz::authorize(pool, caller, connection.owner_team_id).await?;

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut conn = pool.acquire().await?;
    access_service::delete_grant(
        &mut conn,
        "kb_connections",
        connection_id,
        "kb_teams",
        team_id,
        caller,
        emitter,
    )
    .await?;

    Ok(connection)
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

    /// A provision request that declares NO reach fidelity — neither granularity nor covers.
    fn req_no_reach(name: &str, owner_team_id: Option<Uuid>) -> ProvisionConnectionRequest {
        ProvisionConnectionRequest {
            provider: "github".into(),
            name: name.into(),
            owner_team_id,
            reach_granularity: None,
            reach_covers: None,
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

        // The caller must carry its `<handle>@web` emitter — grant_reach now resolves one to author
        // the grant_created event, exactly as production does (a fixture that skips this passes while
        // production 500s). Provision via the same production path.
        let mut conn = pool.acquire().await.expect("acquire");
        crate::services::profile_service::provision_profile_entities(&mut conn, id, "conn-admin")
            .await
            .expect("provision caller emitters");
        drop(conn);

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

        // Emitters, so this profile can author a grant event if it ends up calling grant_reach.
        let mut conn = pool.acquire().await.expect("acquire");
        crate::services::profile_service::provision_profile_entities(&mut conn, id, handle)
            .await
            .expect("provision caller emitters");
        drop(conn);

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

    /// The three excess-reach affirmation columns are born NULL (unaffirmed) and round-trip
    /// through `get`. NO grant requiring affirmation has happened, so nothing is stamped — the
    /// honest "never affirmed" state. Read-side only; enforcement is Beat 2.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn reach_affirmation_is_born_null_and_round_trips(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        assert!(c.reach_affirmed_by.is_none(), "unaffirmed by default");
        assert!(c.reach_affirmed_at.is_none(), "unaffirmed by default");
        assert!(c.reach_affirmation.is_none(), "unaffirmed by default");

        // Round-trip through a fresh get — the columns exist and read back NULL.
        let reloaded = svc::get(&pool, c.id).await.expect("get");
        assert!(reloaded.reach_affirmed_by.is_none());
        assert!(reloaded.reach_affirmed_at.is_none());
        assert!(reloaded.reach_affirmation.is_none());
    }

    /// `declares_reach()` is the honest, non-computing signal: TRUE when the connection was
    /// provisioned WITH a declared reach fidelity, FALSE when it declares none. No enforcement —
    /// this only reads whether an affirmation would be required (Beat 2 acts on it).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn declares_reach_reflects_the_declared_fidelity(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        let with_reach = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision with reach");
        assert!(
            with_reach.declares_reach(),
            "a connection provisioned with reach_granularity/reach_covers declares reach"
        );

        let no_reach = svc::provision(&pool, admin, &req_no_reach("Bare GitHub", None))
            .await
            .expect("provision without reach");
        assert!(
            !no_reach.declares_reach(),
            "a connection provisioned with no reach fidelity declares no reach"
        );
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

    /// A broker that mints successfully — the default for tests where the mint
    /// outcome is not what is under test.
    fn granting_broker() -> crate::broker::FakeBroker {
        crate::broker::FakeBroker::granting(serde_json::json!({"repository_selection": "all"}))
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

        let attached =
            svc::attach_credential(&pool, &granting_broker(), admin, c.id, &credential())
                .await
                .expect("attach")
                .connection;

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

    /// A broker that mints successfully records what it observed, and the observed
    /// reach rides back next to the connection's declared reach.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_granting_broker_records_the_observed_reach(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        let out = svc::attach_credential(&pool, &granting_broker(), admin, c.id, &credential())
            .await
            .expect("attach");

        assert!(out.verification.verified, "a successful mint is verified");
        assert_eq!(
            out.verification
                .observed_reach
                .as_ref()
                .and_then(|r| r.get("repository_selection"))
                .and_then(|v| v.as_str()),
            Some("all"),
            "the mint metadata is surfaced as observed_reach"
        );
        assert!(!out.connection.needs_credential());
    }

    /// A broker that *proves* the credential bad blocks the attach and writes
    /// nothing — the seam failing loudly at attach, not silently at S3.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_rejecting_broker_blocks_the_attach_and_writes_nothing(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Acme GitHub", None))
            .await
            .expect("provision");

        let broker = crate::broker::FakeBroker::rejecting();
        assert!(
            svc::attach_credential(&pool, &broker, admin, c.id, &credential())
                .await
                .is_err(),
            "a proven-bad credential must be rejected"
        );

        let reloaded = svc::get(&pool, c.id).await.expect("still there");
        assert!(
            reloaded.needs_credential(),
            "a rejected attach must write nothing — the credential stays NULL"
        );
    }

    /// A connector that needs consent, or a deployment with no broker, still
    /// attaches — but says, out loud, that it is not verified (invariant 6).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn an_unverifiable_but_not_rejected_credential_attaches_with_a_note(pool: PgPool) {
        let admin = seed_admin(&pool).await;

        // needs-consent: persisted, flagged.
        let c1 = svc::provision(&pool, admin, &req("Consent Pending", None))
            .await
            .expect("provision");
        let consent = crate::broker::FakeBroker::needs_consent();
        let out = svc::attach_credential(&pool, &consent, admin, c1.id, &credential())
            .await
            .expect("attach still succeeds");
        assert!(
            !out.connection.needs_credential(),
            "the credential is recorded"
        );
        assert!(!out.verification.verified);
        assert!(
            out.verification.note.is_some(),
            "an unverified attach must carry a note saying why"
        );

        // no broker configured: same shape.
        let c2 = svc::provision(&pool, admin, &req("No Broker", None))
            .await
            .expect("provision");
        let out2 = svc::attach_credential(
            &pool,
            &crate::broker::NullBroker,
            admin,
            c2.id,
            &credential(),
        )
        .await
        .expect("attach still succeeds without a broker");
        assert!(!out2.verification.verified);
        assert!(out2.verification.note.is_some());
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
            svc::attach_credential(&pool, &granting_broker(), admin, c.id, &credential()).await,
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
            svc::attach_credential(&pool, &granting_broker(), maintainer, c.id, &credential())
                .await,
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
            svc::attach_credential(&pool, &granting_broker(), admin, c.id, &no_broker).await,
            Err(ApiError::BadRequest(_))
        ));

        let no_connector = ConnectionCredential {
            connector: String::new(),
            ..credential()
        };
        assert!(matches!(
            svc::attach_credential(&pool, &granting_broker(), admin, c.id, &no_connector).await,
            Err(ApiError::BadRequest(_))
        ));

        assert!(
            svc::get(&pool, c.id).await.expect("get").needs_credential(),
            "neither rejection may leave the connection looking credentialed"
        );
    }

    // -----------------------------------------------------------------------
    // Reach grants — a TEAM's read-reach on a connection. Same gate as every
    // other mutator (`machine_authz::authorize`, keyed on the connection's
    // owning team; teamless fails closed), writing a `kb_access_grants` row.
    // -----------------------------------------------------------------------

    /// A bare team, so a reach grant has a real principal to point at.
    async fn seed_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) \
             ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
            slug,
        )
        .fetch_one(pool)
        .await
        .expect("seed team")
    }

    /// Does a read-reach grant row exist for `team_id` on `connection_id`?
    async fn reach_grant_exists(pool: &PgPool, connection_id: Uuid, team_id: Uuid) -> bool {
        sqlx::query_scalar!(
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
        .await
        .expect("grant existence check")
    }

    /// The owner of the connection's owning team may grant a team read-reach; the grant lands.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_team_owner_may_grant_reach(pool: PgPool) {
        let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
        let c = svc::provision(&pool, owner, &req("Acme GitHub", Some(team)))
            .await
            .expect("provision");
        let beta = seed_team(&pool, "beta").await;

        svc::grant_reach(
            &pool,
            owner,
            c.id,
            beta,
            Some("beta reviews acme CI".into()),
        )
        .await
        .expect("a team owner may grant reach on their own connection");

        assert!(
            reach_grant_exists(&pool, c.id, beta).await,
            "the read-reach grant must be visible after granting"
        );
    }

    /// A system admin may grant reach — even on a teamless connection (the admin bypass).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_system_admin_may_grant_reach(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Teamless GitHub", None))
            .await
            .expect("provision");
        let beta = seed_team(&pool, "beta").await;

        svc::grant_reach(
            &pool,
            admin,
            c.id,
            beta,
            Some("admin binds teamless reach".into()),
        )
        .await
        .expect("a system admin may grant reach");

        assert!(reach_grant_exists(&pool, c.id, beta).await);
    }

    /// Neither admin nor owner-of-the-owning-team is denied — read-reach is not self-service.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_non_owner_non_admin_cannot_grant_reach(pool: PgPool) {
        let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
        let c = svc::provision(&pool, owner, &req("Acme GitHub", Some(team)))
            .await
            .expect("provision");
        let (outsider, _other) =
            seed_team_member(&pool, "reach-outsider", "other", TeamRole::Owner).await;

        let err = svc::grant_reach(&pool, outsider, c.id, team, None)
            .await
            .expect_err("an owner of a different team is not authorized here");
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
        assert!(
            !reach_grant_exists(&pool, c.id, team).await,
            "a denied grant must write nothing"
        );
    }

    /// A teamless connection fails closed for a non-admin: "no team to check" ≠ "nothing to deny".
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_teamless_connection_grant_reach_fails_closed(pool: PgPool) {
        let admin = seed_admin(&pool).await;
        let c = svc::provision(&pool, admin, &req("Teamless GitHub", None))
            .await
            .expect("provision");
        let (outsider, _other) =
            seed_team_member(&pool, "reach-outsider", "other", TeamRole::Owner).await;
        let beta = seed_team(&pool, "beta").await;

        let err = svc::grant_reach(&pool, outsider, c.id, beta, None)
            .await
            .expect_err("teamless is admin-only");
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
    }

    /// `revoke_reach` removes a previously granted row, gated the same way.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn revoke_reach_removes_a_previously_granted_row(pool: PgPool) {
        let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
        let c = svc::provision(&pool, owner, &req("Acme GitHub", Some(team)))
            .await
            .expect("provision");
        let beta = seed_team(&pool, "beta").await;

        svc::grant_reach(
            &pool,
            owner,
            c.id,
            beta,
            Some("beta reviews acme CI".into()),
        )
        .await
        .expect("grant");
        assert!(reach_grant_exists(&pool, c.id, beta).await, "granted");

        svc::revoke_reach(&pool, owner, c.id, beta)
            .await
            .expect("revoke");
        assert!(
            !reach_grant_exists(&pool, c.id, beta).await,
            "the grant must be gone after revoke"
        );
    }

    // -----------------------------------------------------------------------
    // Excess-reach affirmation (B3 beat 2). Granting a team reach on a
    // connection that DECLARES reach must be affirmed as intentional or it
    // FAILS — the trigger is purely `declares_reach()`, never a computed
    // comparison against the team. Affirming records who/when/why; it does not
    // narrow the remote reach.
    // -----------------------------------------------------------------------

    /// Declared reach, no affirmation: the grant is refused, and nothing is written — no grant row,
    /// and the affirmation stamp stays NULL.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn grant_on_a_reach_declaring_connection_without_affirmation_is_refused(pool: PgPool) {
        let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
        let c = svc::provision(&pool, owner, &req("Acme GitHub", Some(team)))
            .await
            .expect("provision");
        assert!(c.declares_reach(), "req declares reach");
        let beta = seed_team(&pool, "beta").await;

        let err = svc::grant_reach(&pool, owner, c.id, beta, None)
            .await
            .expect_err("a reach-declaring grant must be affirmed");
        assert!(matches!(err, ApiError::Conflict(_)), "got {err:?}");

        assert!(
            !reach_grant_exists(&pool, c.id, beta).await,
            "a refused grant must write no grant row"
        );
        let reloaded = svc::get(&pool, c.id).await.expect("get");
        assert!(
            reloaded.reach_affirmed_at.is_none(),
            "a refused grant must stamp no affirmation"
        );
    }

    /// Declared reach, affirmed: the affirmation records who/when/why AND the grant lands. The
    /// returned connection (and a fresh get) carry the populated affirmation fields.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn affirming_a_reach_declaring_grant_records_who_when_why_and_writes_the_grant(
        pool: PgPool,
    ) {
        let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
        let c = svc::provision(&pool, owner, &req("Acme GitHub", Some(team)))
            .await
            .expect("provision");
        let beta = seed_team(&pool, "beta").await;

        let out = svc::grant_reach(
            &pool,
            owner,
            c.id,
            beta,
            Some("beta reviews acme CI".into()),
        )
        .await
        .expect("an affirmed grant lands");

        assert!(
            reach_grant_exists(&pool, c.id, beta).await,
            "the grant must land alongside the affirmation"
        );
        // The returned struct is re-read post-UPDATE, so it carries the stamp.
        assert_eq!(out.reach_affirmed_by, Some(*owner), "who");
        assert_eq!(
            out.reach_affirmation.as_deref(),
            Some("beta reviews acme CI"),
            "why"
        );
        assert!(out.reach_affirmed_at.is_some(), "when");

        let reloaded = svc::get(&pool, c.id).await.expect("get");
        assert_eq!(reloaded.reach_affirmed_by, Some(*owner));
        assert_eq!(
            reloaded.reach_affirmation.as_deref(),
            Some("beta reviews acme CI")
        );
        assert!(reloaded.reach_affirmed_at.is_some());
    }

    /// A connection that declares NO reach grants without any affirmation — the affirmation stamp
    /// stays NULL because no affirmation was required or made.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn a_connection_that_declares_no_reach_grants_without_affirmation(pool: PgPool) {
        let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
        let c = svc::provision(&pool, owner, &req_no_reach("Bare GitHub", Some(team)))
            .await
            .expect("provision");
        assert!(!c.declares_reach(), "req_no_reach declares no reach");
        let beta = seed_team(&pool, "beta").await;

        svc::grant_reach(&pool, owner, c.id, beta, None)
            .await
            .expect("a connection with no declared reach grants freely");

        assert!(
            reach_grant_exists(&pool, c.id, beta).await,
            "the grant lands"
        );
        let reloaded = svc::get(&pool, c.id).await.expect("get");
        assert!(
            reloaded.reach_affirmed_at.is_none(),
            "no affirmation is required or recorded"
        );
    }

    /// Affirming a connection that declares NO reach is refused — the flag is inapplicable. Honest
    /// over lenient: it is not silently ignored, and nothing is written.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn affirming_a_connection_that_declares_no_reach_is_refused(pool: PgPool) {
        let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
        let c = svc::provision(&pool, owner, &req_no_reach("Bare GitHub", Some(team)))
            .await
            .expect("provision");
        let beta = seed_team(&pool, "beta").await;

        let err = svc::grant_reach(&pool, owner, c.id, beta, Some("but I insist".into()))
            .await
            .expect_err("--affirm-reach is inapplicable when no reach is declared");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");

        assert!(
            !reach_grant_exists(&pool, c.id, beta).await,
            "a refused grant must write nothing"
        );
    }

    /// Authorization precedes affirmation: a non-owner/non-admin caller passing `affirm_reach` on a
    /// reach-declaring connection is still `Forbidden` (the gate runs FIRST), and nothing is written.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn authorization_precedes_affirmation(pool: PgPool) {
        let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
        let c = svc::provision(&pool, owner, &req("Acme GitHub", Some(team)))
            .await
            .expect("provision");
        let (outsider, _other) =
            seed_team_member(&pool, "reach-outsider", "other", TeamRole::Owner).await;
        let beta = seed_team(&pool, "beta").await;

        let err = svc::grant_reach(&pool, outsider, c.id, beta, Some("I insist".into()))
            .await
            .expect_err("auth runs before affirmation");
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");

        assert!(
            !reach_grant_exists(&pool, c.id, beta).await,
            "an auth-denied grant writes no grant row"
        );
        let reloaded = svc::get(&pool, c.id).await.expect("get");
        assert!(
            reloaded.reach_affirmed_at.is_none(),
            "an auth-denied grant stamps no affirmation"
        );
    }

    /// Re-affirming overwrites the prior affirmation (last-writer). The second rationale and
    /// affirmer win.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn re_affirming_overwrites_the_prior_affirmation(pool: PgPool) {
        let (owner, team) = seed_team_member(&pool, "reach-owner", "acme", TeamRole::Owner).await;
        let c = svc::provision(&pool, owner, &req("Acme GitHub", Some(team)))
            .await
            .expect("provision");
        let beta = seed_team(&pool, "beta").await;
        let gamma = seed_team(&pool, "gamma").await;

        svc::grant_reach(&pool, owner, c.id, beta, Some("first reason".into()))
            .await
            .expect("first affirmation");
        let second = svc::grant_reach(&pool, owner, c.id, gamma, Some("second reason".into()))
            .await
            .expect("second affirmation");

        assert_eq!(
            second.reach_affirmation.as_deref(),
            Some("second reason"),
            "the last writer's rationale wins"
        );
        assert_eq!(second.reach_affirmed_by, Some(*owner));
    }
}

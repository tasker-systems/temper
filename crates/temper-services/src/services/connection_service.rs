//! Persistence for `kb_connections` — temper's authed link to a remote system.
//!
//! A connection is a machine principal wearing an integration's clothes, so authorization is
//! `MachineRegistrationAuthority`'s policy **verbatim**: a system admin, or the OWNER of the team
//! that owns the connection, with a teamless connection failing closed. Provisioning asks it
//! directly (`machine_authz::authorize`, keyed on the team the caller names); every act on an
//! existing connection asks it through `authz::ConnectionControlAuthority`, keyed on the row, whose
//! refusal is indistinguishable from a missing id. Calling the machine gate rather than restating
//! it is deliberate — tighten that predicate and this surface tightens with it. There is no second
//! copy of the policy to drift.
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

use crate::auth::HumanPrincipal;
use crate::authz::{ConnectionAuthority, ConnectionControlAuthority, ConnectionScope, Principal};
use crate::broker::{BrokerError, CredentialBroker, MintRequest, MintSubject};
use crate::error::{ApiError, ApiResult};
use crate::services::access_service::InsertGrantParams;
use crate::services::{access_service, context_service, machine_authz, profile_service};

/// The emitter marker for a connection's entity (`<handle>@webhook`).
///
/// Deliberately **not** a `Surface` variant. `Surface::ALL` drives
/// `profile_service::provision_profile_entities`, so adding `Webhook` there would provision a
/// webhook emitter onto every human profile — and oblige a backfill migration for every profile
/// that already exists (see `temper_workflow::operations::surface`). A connection's entity is
/// created directly instead, and intake resolves the emitter from `connection.emitter_entity_id`
/// rather than from a surface marker.
const WEBHOOK_EMITTER_MARKER: &str = "webhook";

/// The refusal for a connection the caller cannot see — absent, or present but outside the caller's
/// authority. One sentence for both, on purpose: `get` renders it for a missing row and
/// `authz::ConnectionControlAuthority` for a denied one, so a caller probing ids cannot tell the
/// two apart.
pub(crate) const CONNECTION_REFUSAL: &str = "connection not found or not readable";

/// Load one connection by its own id. Unauthorized: the internal primitive the post-insert
/// readback uses. Surface callers want [`get_for_caller`].
pub async fn get(pool: &PgPool, id: Uuid) -> ApiResult<Connection> {
    sqlx::query_as!(
        Connection,
        r#"SELECT id, provider, slug, name, owner_team_id, registered_by_profile_id,
                  profile_id, emitter_entity_id, home_context_id, credential,
                  webhook_events, tool_manifest, reach_granularity, reach_covers,
                  reach_affirmed_by, reach_affirmed_at, reach_affirmation,
                  observed_reach,
                  created, revoked_at, revoked_by_profile_id
             FROM kb_connections WHERE id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(CONNECTION_REFUSAL.to_string()))
}

/// Resolve a **verified inbound attestation** to the connection that receives it.
///
/// `CredentialBroker::verify_inbound` stops at the signed `trigger` claim on purpose — its doc:
/// *"resolving it to a `kb_connections` row is the caller's job (the broker stays DB-free, hence
/// swappable)"*. This is that job. Both arguments come from the **signed** claim, never from the
/// unsigned `x-trigger-*` mirror headers.
///
/// Keys on `credential->>'connector'`, which holds the connector **uid** — the same value
/// `connect_token_url` percent-encodes for the mint hop. Both partial-index predicates are
/// repeated here deliberately: without them the planner cannot use `idx_kb_connections_connector`
/// (witnessed — the query plans as a `Seq Scan` even with `enable_seqscan=off`), and each is also
/// load-bearing on its own. A NULL credential is the `needs_credential` birth state and can
/// receive nothing; a revoked connection must not receive at all.
///
/// **No match returns the uniform pre-identity refusal ([`ApiError::Unauthenticated`]), not
/// `NotFound`.** It renders byte-identically to a failed attestation, so a probe cannot learn
/// whether a connector is provisioned here by comparing responses.
///
/// **Two matches are a FAILURE, not a choice.** `credential->>'connector'` is not unique and two
/// teams may legitimately register the same org-wide connector, so ambiguity is reachable — and
/// temper genuinely cannot know which connection the payload was for. Picking one would route a
/// real event to the wrong subscribers and read, downstream, exactly like a correct routing.
pub async fn resolve_inbound(
    pool: &PgPool,
    provider: &str,
    connector_uid: &str,
) -> ApiResult<Connection> {
    let rows = sqlx::query_as!(
        Connection,
        r#"SELECT id, provider, slug, name, owner_team_id, registered_by_profile_id,
                  profile_id, emitter_entity_id, home_context_id, credential,
                  webhook_events, tool_manifest, reach_granularity, reach_covers,
                  reach_affirmed_by, reach_affirmed_at, reach_affirmation,
                  observed_reach,
                  created, revoked_at, revoked_by_profile_id
             FROM kb_connections
            WHERE credential->>'connector' = $1
              AND provider = $2
              AND credential IS NOT NULL
              AND revoked_at IS NULL"#,
        connector_uid,
        provider,
    )
    .fetch_all(pool)
    .await?;

    match rows.len() {
        1 => Ok(rows.into_iter().next().expect("length checked")),
        0 => Err(ApiError::Unauthenticated(
            crate::error::UnauthCause::IntakeUnattested,
        )),
        n => Err(ApiError::Internal(format!(
            "inbound connector resolves to {n} connections; temper cannot know which one this              payload was for. Exactly one live, credentialed connection must carry this connector."
        ))),
    }
}

/// [`get`], gated on the *existing row's* owning team. A caller outside that authority is refused
/// exactly as a missing id is (`CONNECTION_REFUSAL`) — see `authz::ConnectionControlAuthority`.
pub async fn get_for_caller(
    pool: &PgPool,
    authed: &HumanPrincipal,
    id: Uuid,
) -> ApiResult<Connection> {
    authorize_control(pool, authed, id).await?;
    get(pool, id).await
}

/// The per-row gate every act on an existing connection passes first: a system admin, or the owner
/// of the connection's owning team, keyed on the row. A refusal is indistinguishable from a missing
/// id.
///
/// Answers whether it resolved on the system-admin arm, which a writing act asks again on its
/// transaction ([`write_tx`]).
async fn authorize_control(pool: &PgPool, authed: &HumanPrincipal, id: Uuid) -> ApiResult<bool> {
    Ok(
        crate::authz::authorize::<ConnectionControlAuthority>(pool, Principal::Human(authed), id)
            .await?
            .authority()
            == ConnectionControlAuthority::SystemAdmin,
    )
}

/// The transaction an act on an existing connection writes on, opened with the admin re-check
/// first. A decision resolved on the system-admin arm was a pool read of governance; it is asked
/// again here under the governance lock (`auth::admin_arm_holds`) before the act's first write, so
/// an admin demoted since (or while this runs) changes nothing. The refusal is
/// `CONNECTION_REFUSAL`, the sentence a caller with no authority over the row gets, the same as
/// for a missing id. A demoted admin who owns the connection's team is admitted by a retry on the
/// owner arm. Lock order: governance first, then the connection and grant rows the act writes; a
/// demotion locks none of those.
async fn write_tx(
    pool: &PgPool,
    authed: &HumanPrincipal,
    on_admin_arm: bool,
) -> ApiResult<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    if !crate::auth::admin_arm_holds(&mut tx, authed.profile_id(), on_admin_arm).await? {
        return Err(ApiError::NotFound(CONNECTION_REFUSAL.to_string()));
    }
    Ok(tx)
}

/// [`write_tx`] for [`grant_reach`], with the owner arm asked again too. A reach grant hands READ on
/// what the connection receives to a team the caller manages, so an owner removed from the
/// connection's owning team mid-act would keep reaching it through that team. Off the admin arm,
/// the caller's ownership of the owning team is therefore decided again under that team's row lock
/// (the lock every membership writer takes first): a removal or demotion committed since refuses,
/// one in flight makes this wait, then refuses. The refusal is `CONNECTION_REFUSAL`, what a caller
/// with no authority over the connection gets. Lock order: governance (admin arm only), then the
/// owning team, then the grant row.
async fn reach_tx(
    pool: &PgPool,
    authed: &HumanPrincipal,
    on_admin_arm: bool,
    owning_team: Option<Uuid>,
) -> ApiResult<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut tx = write_tx(pool, authed, on_admin_arm).await?;
    if !on_admin_arm {
        let owns = match owning_team {
            Some(team) => {
                crate::authz::team_membership::role_under_lock(
                    &mut tx,
                    team,
                    authed.profile_id(),
                    &[],
                )
                .await?
                    == Some(temper_core::types::team::TeamRole::Owner)
            }
            None => false,
        };
        if !owns {
            return Err(ApiError::NotFound(CONNECTION_REFUSAL.to_string()));
        }
    }
    Ok(tx)
}

/// List connections visible to `caller`, newest first. Revoked rows are hidden unless asked for.
/// A system admin sees every row but those of someone else's personal team (teamless ones
/// included); a team owner sees only connections owned by a team they own.
///
/// `EXISTS`, not `array_agg` — an empty scope must DENY, and an aggregate over an empty scope
/// yields NULL, which falls open.
pub async fn list(
    pool: &PgPool,
    authed: &HumanPrincipal,
    include_revoked: bool,
) -> ApiResult<Vec<Connection>> {
    let caller = ProfileId::from(authed.profile().id);
    let is_admin = crate::authz::Principal::Human(authed)
        .system_admin(pool)
        .await?
        .is_some();

    let rows = sqlx::query_as!(
        Connection,
        r#"SELECT id, provider, slug, name, owner_team_id, registered_by_profile_id,
                  profile_id, emitter_entity_id, home_context_id, credential,
                  webhook_events, tool_manifest, reach_granularity, reach_covers,
                  reach_affirmed_by, reach_affirmed_at, reach_affirmation,
                  observed_reach,
                  created, revoked_at, revoked_by_profile_id
             FROM kb_connections c
            WHERE ($1 OR c.revoked_at IS NULL)
              -- The admin arm stops at someone else's personal team (task `01a191b4`), as the
              -- per-row gate does; its roster stays readable through the team's members.
              AND ( ($2 AND NOT EXISTS (SELECT 1 FROM kb_teams pt
                                         WHERE pt.id = c.owner_team_id
                                           AND profile_is_person(pt.personal_of)
                                           AND pt.personal_of <> $3))
                    OR ( c.owner_team_id IS NOT NULL
                         AND EXISTS (
                             SELECT 1
                               FROM kb_team_members tm
                               JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
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
    authed: &HumanPrincipal,
    req: &ProvisionConnectionRequest,
) -> ApiResult<Connection> {
    let caller = ProfileId::from(authed.profile().id);
    // Auth before writes: a rejected provisioning must leave the DB completely unchanged — no
    // orphaned profile, no orphaned entity, no orphaned context. Resolving before the
    // transaction opens is what makes that assertable.
    let authority =
        machine_authz::authorize(pool, Principal::Human(authed), req.owner_team_id).await?;

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
    // The admin arm was decided on the pool; ask again under the governance lock before the first
    // write. A demoted admin is refused as a caller with no authority is (`403`); one who owns the
    // team is admitted by a retry on the owner arm.
    let on_admin_arm = authority == machine_authz::MachineRegistrationAuthority::SystemAdmin;
    if !crate::auth::admin_arm_holds(&mut tx, caller, on_admin_arm).await? {
        return Err(ApiError::Forbidden);
    }

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
    // The name is canonicalized on the way in, as `context_service::create` and `rename` do. This
    // is the third path that stores a context name, and a canonical-form invariant honoured by two
    // of three write paths is no more an invariant than one of two. The slug is unaffected —
    // `sluggify` already collapses whitespace runs — so this changes the stored `name` only.
    let home_context_name = context_service::canonical_name(&req.name);
    let home_context_id = sqlx::query_scalar!(
        r#"INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id"#,
        Uuid::now_v7(),
        owner_table,
        owner_id,
        &slug,
        &home_context_name,
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
pub async fn revoke(pool: &PgPool, id: Uuid, authed: &HumanPrincipal) -> ApiResult<Connection> {
    let revoker = ProfileId::from(authed.profile().id);
    // Auth before writes, keyed on the existing row's owning team.
    let on_admin_arm = authorize_control(pool, authed, id).await?;

    let mut tx = write_tx(pool, authed, on_admin_arm).await?;
    sqlx::query!(
        r#"UPDATE kb_connections
              SET revoked_at = now(), revoked_by_profile_id = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        *revoker,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get(pool, id).await
}

/// Gate a mutation on an existing, **live** connection.
///
/// The per-row gate keyed on the row's own owning team, plus the revoked check. A revoked
/// connection is dead — reactivation is a new provisioning, never an UPDATE — so a mutator refuses
/// one outright rather than issuing an UPDATE that silently matches no rows and reports success.
///
/// The revoked check runs **after** authorization, so only a caller who controls the connection
/// learns that it is revoked.
///
/// Answers the row and whether the decision resolved on the system-admin arm ([`write_tx`]).
async fn authorize_live(
    pool: &PgPool,
    authed: &HumanPrincipal,
    id: Uuid,
) -> ApiResult<(Connection, bool)> {
    let on_admin_arm = authorize_control(pool, authed, id).await?;
    let existing = get(pool, id).await?;
    if existing.revoked_at.is_some() {
        return Err(ApiError::Conflict(format!(
            "connection '{}' is revoked; reactivation is a new provisioning",
            existing.slug
        )));
    }
    Ok((existing, on_admin_arm))
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
///
/// **The observed reach is persisted** onto `kb_connections.observed_reach` so the
/// grant path can consult it later — the mint is the witness, and the witness
/// must outlive the attach response. `grant_reach` reads it back to decide
/// whether affirmation is required (the gate fires on observed-vs-declared drift
/// within the remote domain, not on the declaration alone).
pub async fn attach_credential(
    pool: &PgPool,
    broker: &dyn CredentialBroker,
    authed: &HumanPrincipal,
    id: Uuid,
    credential: &ConnectionCredential,
) -> ApiResult<AttachCredentialResponse> {
    let (_, on_admin_arm) = authorize_live(pool, authed, id).await?;

    if credential.broker.trim().is_empty() {
        return Err(ApiError::BadRequest("broker must not be empty".into()));
    }
    if credential.connector.trim().is_empty() {
        return Err(ApiError::BadRequest("connector must not be empty".into()));
    }

    // The admin arm asked again before the broker is called: the mint below reaches the provider,
    // so an admin demoted since the pool read (or one whose demotion is in flight, which this
    // waits for) reaches nothing. The lock is released before the round trip; the write's own
    // transaction asks again below, and that is the check that holds to the write.
    write_tx(pool, authed, on_admin_arm).await?.commit().await?;

    // Mint once to verify. Auth is already checked; this is a read against the
    // broker, before any persistence. Only a proven-bad credential blocks.
    let verification = verify_by_minting(broker, id, credential).await?;

    let value = serde_json::to_value(credential)
        .map_err(|e| ApiError::Internal(format!("failed to serialize credential: {e}")))?;

    // After the broker round trip, so no transaction is held open across it.
    let mut tx = write_tx(pool, authed, on_admin_arm).await?;
    sqlx::query!(
        r#"UPDATE kb_connections
              SET credential = $2,
                  observed_reach = $3
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        value,
        verification.observed_reach.as_ref(),
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let connection = get(pool, id).await?;
    Ok(AttachCredentialResponse {
        connection,
        verification,
    })
}

/// The `400` an attach answers when the broker rejects the credential.
const REJECTED_CREDENTIAL: &str = "connector rejected the credential";

/// The note an attach carries when the broker failed for a reason that says nothing about the
/// credential.
const UNVERIFIED_NOTE: &str = "not verified — the broker's error is in the server log";

/// Mint once and classify the outcome. `Err` here means the attach must be
/// rejected (the credential was proven bad); `Ok(verification)` means persist,
/// carrying what was (or could not be) observed.
///
/// The caller is told which outcome, in a fixed sentence. The broker's own error text (for Vercel
/// Connect, its response body about a mint made with this deployment's token) goes to the server
/// log, never to the response: [`REJECTED_CREDENTIAL`], [`UNVERIFIED_NOTE`].
async fn verify_by_minting(
    broker: &dyn CredentialBroker,
    connection: Uuid,
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
        Err(e @ BrokerError::Unauthorized(_)) => {
            tracing::warn!(
                %connection,
                connector = %credential.connector,
                error = %e,
                "credential attach: the broker rejected the credential"
            );
            return Err(ApiError::BadRequest(REJECTED_CREDENTIAL.to_string()));
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
        Err(e) => {
            tracing::warn!(
                %connection,
                connector = %credential.connector,
                error = %e,
                "credential attach: the credential could not be verified"
            );
            CredentialVerification {
                verified: false,
                observed_reach: None,
                note: Some(UNVERIFIED_NOTE.to_string()),
            }
        }
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
    authed: &HumanPrincipal,
    id: Uuid,
    events: &[String],
) -> ApiResult<Connection> {
    let (_, on_admin_arm) = authorize_live(pool, authed, id).await?;

    let mut tx = write_tx(pool, authed, on_admin_arm).await?;
    sqlx::query!(
        r#"UPDATE kb_connections
              SET webhook_events = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        events,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

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
    authed: &HumanPrincipal,
    id: Uuid,
    tools: &[String],
) -> ApiResult<Connection> {
    let (_, on_admin_arm) = authorize_live(pool, authed, id).await?;

    let value = serde_json::to_value(tools)
        .map_err(|e| ApiError::Internal(format!("failed to serialize tool manifest: {e}")))?;

    let mut tx = write_tx(pool, authed, on_admin_arm).await?;
    sqlx::query!(
        r#"UPDATE kb_connections
              SET tool_manifest = $2
            WHERE id = $1 AND revoked_at IS NULL"#,
        id,
        value,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    get(pool, id).await
}

/// Grant a TEAM read-reach on a connection. Owning a connection is not reaching it — this writes a
/// `kb_access_grants` row (`subject_table = 'kb_connections'`) so the named team's members inherit
/// READ on what the connection receives. Reach is read-only: no write, delete, or grant is conferred.
///
/// Auth before writes, on **both** teams in play — they are different questions, and they are now
/// one gate: `crate::authz::ConnectionAuthority`. It composes, rather than restates, the two:
///
/// 1. *May you act on this connection?* — `MachineRegistrationAuthority`, keyed on the
///    connection's own owning team: a system admin, or the OWNER of that team; a teamless
///    connection fails closed. The SAME policy as every other connection mutator, CALLED not
///    restated. It deliberately does NOT route
///    through `access_service::can_administer_grant`, whose `can_grant` seam has no bootstrap
///    holder for a connection subject.
/// 2. *May you hand read-reach to THAT team?* — `machine_authz::contain_target_team`: a
///    manage-capable role on the RECEIVING team, system admins exempt. Without it, the owner of
///    one team could bind their connection's reach to any team UUID in the instance, including
///    teams they have no relationship with — and since `kb_access_grants.principal_id` carries no
///    foreign key, a bogus UUID would persist a dangling grant row. This mirrors the machine
///    path's `machine_authz::contain_reach`, through the same `require_manage_on_team` seam so the
///    two cannot drift.
///
/// The connection's OWNING team is read from the row inside the gate, not passed in — so the pair
/// the proof is about is (this connection, that team), and nothing the caller supplies decides
/// which team its authority gets checked against.
///
/// A connection whose reach is *declared* (`Connection::declares_reach`) carries a coarse remote
/// reach that may exceed the granted team's temper reach — remote and temper scope are
/// incommensurable, so there is no computed `exceeds` bool, only the honest declaration. Binding
/// such a connection to a team is therefore not allowed to proceed silently: `affirm_reach` must
/// carry the stated reason, or the grant FAILS. Affirming records who/when/why on the connection —
/// it makes the asymmetry declared and reviewable; it does NOT resolve it or narrow the remote
/// reach.
///
/// **The gate consults the mint's witness, not just the declaration.** A declared reach is a
/// promise; the `observed_reach` column is what the attach-time mint actually saw. The gate fires
/// (`affirm_reach` required) when the connection declares reach AND either the credential was never
/// minted (`observed_reach` is `None` — the declaration is unwitnessed) or the mint's observed
/// scope is broader than the declared covers (the commensurable, remote-domain gap the mint can
/// detect — e.g. GitHub's `repository_selection: "all"` against `reach_covers: "acme/temper"`).
/// Remote-vs-temper scope stays incommensurable and uncomputed; only the remote-domain
/// observed-vs-declared drift is compared, and only to decide whether affirmation is required —
/// never to auto-deny. The four cases:
///
/// - declares reach, gate fires, no affirmation → `Conflict`, writes nothing (naming the declared
///   reach, the observed reach, and the fix).
/// - declares reach, gate fires, affirmation → one transaction: stamp the affirmation, then insert
///   the grant.
/// - declares no reach, affirmation → `BadRequest` (the flag is inapplicable — honest over lenient).
/// - declares no reach, no affirmation → the plain grant.
/// - declares reach, gate does NOT fire (observed agrees with declared), affirmation → `BadRequest`
///   (the flag is inapplicable — there is no gap to acknowledge). Honest over lenient, same as the
///   no-declared-reach case: do not silently accept a spurious affirmation.
/// - declares reach, gate does NOT fire, no affirmation → the plain grant (the mint witnessed the
///   declaration; no gap to acknowledge).
///
/// Auth stays FIRST — affirmation never bypasses authorization.
pub async fn grant_reach(
    pool: &PgPool,
    authed: &HumanPrincipal,
    connection_id: Uuid,
    team_id: Uuid,
    affirm_reach: Option<String>,
) -> ApiResult<Connection> {
    let caller = ProfileId::from(authed.profile().id);
    let connection = get(pool, connection_id).await?;
    let proof = crate::authz::authorize::<ConnectionAuthority>(
        pool,
        Principal::Human(authed),
        ConnectionScope::new(connection_id, team_id),
    )
    .await?;
    let on_admin_arm = proof.authority() == ConnectionAuthority::SystemAdmin;

    // The gate: declared reach alone is the declaration; the mint's observed reach is the witness.
    // Affirmation is required when the connection declares reach AND the gate fires (observed is
    // missing, or observed scope is broader than declared). A declared reach whose mint witnessed
    // no gap grants freely — the declaration was honest.
    let gate_fires = connection.declares_reach() && reach_gate_fires(&connection);

    match (gate_fires, affirm_reach) {
        // Declared reach, gate fires, unaffirmed: refuse, naming the declared reach, the observed
        // reach, and the fix. Write nothing.
        (true, None) => Err(ApiError::Conflict(format!(
            "connection '{}' declares remote reach ({}) that may exceed the team's temper reach; \
             the credential's observed reach is {} — binding it to a team must be affirmed as \
             intentional — re-run with --affirm-reach \"<why this is intended>\"",
            connection.slug,
            reach_descriptor(&connection),
            observed_reach_descriptor(&connection),
        ))),
        // Declared reach, gate fires, affirmed: stamp the affirmation and insert the grant
        // atomically — never affirmation-without-grant or grant-without-affirmation.
        (true, Some(rationale)) => {
            let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let mut tx = reach_tx(pool, authed, on_admin_arm, connection.owner_team_id).await?;
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
                &crate::authz::GrantWarrant::ConnectionReach(&proof),
                &reach_grant_params(team_id, caller),
                emitter,
            )
            .await?;
            // (`&mut tx` coerces to `&mut PgConnection` via Transaction's DerefMut.)
            tx.commit().await?;
            // The top-of-fn `connection` is stale post-UPDATE; re-read so the returned struct
            // carries the populated affirmation fields.
            get(pool, connection_id).await
        }
        // No gate (declares no reach, OR declares reach with no observed gap), but an affirmation
        // was passed: the flag is inapplicable. Honest over lenient — do not silently ignore it.
        // Write nothing.
        (false, Some(_)) => Err(ApiError::BadRequest(
            "this connection has no reach gap to acknowledge; --affirm-reach is not applicable"
                .into(),
        )),
        // No gate, no affirmation: the plain grant.
        (false, None) => {
            let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let mut tx = reach_tx(pool, authed, on_admin_arm, connection.owner_team_id).await?;
            access_service::insert_grant(
                &mut tx,
                &crate::authz::GrantWarrant::ConnectionReach(&proof),
                &reach_grant_params(team_id, caller),
                emitter,
            )
            .await?;
            tx.commit().await?;
            Ok(connection)
        }
    }
}

/// Does the grant gate fire for this connection? The gate fires when the connection declares reach
/// AND the mint's witness does not confirm the declaration. Two cases, both commensurable within
/// the remote domain (never the incommensurable remote-vs-temper axis):
///
/// - `observed_reach` is `None` — the credential was never minted, or the mint returned no reach
///   metadata. The declaration is unwitnessed; affirmation is required.
/// - The observed scope is broader than the declared covers. Today this catches the one
///   commensurable signal the mint exposes: GitHub's `repository_selection: "all"` against a
///   `reach_covers` naming a specific repo or repo-set. Richer per-provider comparison (a
///   `repositories` list broader than `reach_covers`) is deferred — this is the honest minimum,
///   not a computed `exceeds` bool.
///
/// A declared reach whose mint witnessed `repository_selection: "selected"` (or any shape this
/// function does not recognize as broader) does NOT fire the gate: the declaration was honest, and
/// the grant proceeds without affirmation. The mint's `permissions` map is not compared against
/// `reach_covers` — permissions are about *what* (read/write), `reach_covers` is about *which*
/// (which repos), and conflating them would be a false comparison.
fn reach_gate_fires(connection: &Connection) -> bool {
    let Some(observed) = &connection.observed_reach else {
        // Unwitnessed declaration: the gate fires. A declared reach that no mint has confirmed is
        // exactly the case where affirmation is the honest substitute for evidence.
        return true;
    };
    // The commensurable signal: GitHub's `repository_selection`. `"all"` is broader than any
    // specific `reach_covers`; `"selected"` (or absent) is not. This is provider-aware but not
    // provider-specific in its *shape* — `repository_selection` is the field the mint returns, and
    // `"all"` is the value that says "the credential sees everything." Other providers that expose
    // a comparable "all vs selected" signal could be added here; a provider with no such signal
    // leaves the observed reach uninterpreted, and the gate does not fire on it alone (the
    // unwitnessed case above is the catch-all for "we cannot tell").
    let selection_all = observed
        .get("repository_selection")
        .and_then(|v| v.as_str())
        .map(|s| s == "all")
        .unwrap_or(false);
    selection_all
}

/// The observed-reach descriptor for the affirmation-required message. Human-readable, never
/// panic-prone: `None` says "never minted" out loud rather than encoding silence as absence.
fn observed_reach_descriptor(connection: &Connection) -> String {
    match &connection.observed_reach {
        Some(reach) => reach.to_string(),
        None => "never minted (no observed reach on record)".to_string(),
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
/// The non-subject columns of a reach grant. The connection is absent on purpose — it is the
/// warrant's subject, and naming it here as well is the second spelling PR 3 removes.
fn reach_grant_params(team_id: Uuid, granted_by: ProfileId) -> InsertGrantParams {
    InsertGrantParams {
        principal_table: "kb_teams".into(),
        principal_id: team_id,
        can_read: true,
        can_write: false,
        can_delete: false,
        can_grant: false,
        granted_by_profile_id: *granted_by,
    }
}

/// Revoke a team's read-reach on a connection. Deletes the `kb_access_grants` row by its 4-tuple
/// (absent ⇒ idempotent no-op).
///
/// **Deliberately asymmetric with [`grant_reach`]: this checks the connection's owning team ONLY,
/// never the target team.** That is not an oversight, and it must not be "fixed" into symmetry.
/// De-escalation may never be harder than escalation. A grant is legitimately made while the
/// caller holds a manage role on the target team; roles change. If revoke demanded that same role,
/// the grant would become permanently unrevokable the moment the caller lost it — stranding
/// access on a connection whose own owner can see the grant and cannot withdraw it. Handing reach
/// out is gated; taking it back stays open to whoever controls the connection.
///
/// The target-team existence check is likewise absent by design: a `DELETE` on a 4-tuple that
/// matches nothing writes nothing, so there is no dangling row to prevent — and if a dangling row
/// already exists (granted before this gate shipped), this is precisely the path that cleans it up.
pub async fn revoke_reach(
    pool: &PgPool,
    authed: &HumanPrincipal,
    connection_id: Uuid,
    team_id: Uuid,
) -> ApiResult<Connection> {
    let caller = ProfileId::from(authed.profile().id);
    let connection = get(pool, connection_id).await?;
    // `ConnectionControlAuthority`, NOT `ConnectionAuthority` — the whole asymmetry in one line.
    // This asks only "may you act on this connection?"; it deliberately does not ask whether the
    // caller still manages the team losing the reach, which is what
    // `revoke_reach_survives_losing_the_target_team_role` exists to hold in place.
    let proof = crate::authz::authorize::<crate::authz::ConnectionControlAuthority>(
        pool,
        Principal::Human(authed),
        connection_id,
    )
    .await?;

    let on_admin_arm = proof.authority() == crate::authz::ConnectionControlAuthority::SystemAdmin;

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut tx = write_tx(pool, authed, on_admin_arm).await?;
    access_service::delete_grant(
        &mut tx,
        &crate::authz::RevokeWarrant::ConnectionControl(&proof),
        "kb_teams",
        team_id,
        caller,
        emitter,
    )
    .await?;
    tx.commit().await?;

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
mod tests;

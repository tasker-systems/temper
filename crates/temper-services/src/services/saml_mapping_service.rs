//! The SAML group-mapping doors: create and remove a `kb_saml_group_mappings` row, as a system
//! admin's recorded act.
//!
//! A mapping turns an IdP's asserted group into a role on a team, at every login's reconcile
//! (`saml_provisioning_service`), for everyone the IdP puts in the group. So a mapping confers
//! retained access, and both acts follow the admin re-check pattern: the [`SystemAdmin`] proof is
//! minted on the pool by the caller, and asked again under the governance lock on the write's own
//! transaction before the write (`auth::is_system_admin_locked`). A demotion committed since the
//! proof refuses; one in flight waits for the act.
//!
//! Removal only takes access away, and only at each member's next reconcile. It takes the same
//! re-check anyway, so the ledger never records an admin act committed after its actor's demotion.
//!
//! The row change, the ceiling and the confirmation are decided in `_admin_saml_group_mapping_write`
//! (migrations `20261125000010`, `20261126000010`) under the team and gating locks, so the decision
//! holds to commit. Onto temper-system or the gating team a mapping is `watcher` only, and the IdP
//! reconcile clamps any row there to `watcher` (Pete, 2026-11-12). Elsewhere an `owner` role, or an
//! authoring role (owner, maintainer, member) onto a team whose subtree holds a pool (dead teams
//! included), needs the admin's confirmation; a watcher mapping never does. This module maps its named outcomes
//! to the wire.
//!
//! It also holds the one elevation scan every structural team act runs (`elevation_before` and
//! `record_elevations`): an act that puts a pool beneath a team already holding an authoring-role
//! mapping answers each mapping it elevated and records it, act `elevated`.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::saml_mapping::{
    CreateSamlGroupMappingOutcome, CreateSamlGroupMappingRequest, RemoveSamlGroupMappingOutcome,
    RemoveSamlGroupMappingParams, SamlGroupMapping, MAPPING_REMOVAL_LEAVES_MEMBERSHIPS,
};
use temper_core::types::team::TeamRole;

use crate::auth::SystemAdmin;
use crate::error::{ApiError, ApiResult};

/// What the writer answers. Only `changed` wrote anything.
#[derive(serde::Deserialize)]
struct WriteAnswer {
    outcome: String,
    #[serde(default)]
    role: Option<TeamRole>,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    idp_memberships: Option<i64>,
}

/// The refusal a create without confirmation gets, naming what needed it.
fn needs_confirmation(needs: &[String], role: TeamRole) -> ApiError {
    let mut why = Vec::new();
    if needs.iter().any(|n| n == "owner_role") {
        why.push(format!(
            "the role `{}` is above the `maintainer` ceiling",
            role.as_str()
        ));
    }
    if needs.iter().any(|n| n == "pool_beneath") {
        why.push(format!(
            "the role `{}` authors onto a team with a pool beneath it (the gating team or an \
             auto-join team, live or deleted), whose members read everything the team holds",
            role.as_str()
        ));
    }
    ApiError::BadRequest(format!(
        "confirmation required: {}. Re-send with `confirm_elevated: true` to confirm; nothing was \
         written",
        why.join(", and ")
    ))
}

fn refuse_blank(idp_key: &str, group: &str) -> ApiResult<()> {
    if idp_key.trim().is_empty() || group.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "idp_key and group must be non-empty".to_string(),
        ));
    }
    Ok(())
}

async fn write(
    pool: &PgPool,
    admin: &SystemAdmin,
    act: &str,
    key: (&str, &str, Uuid),
    role: Option<TeamRole>,
    confirmed: bool,
) -> ApiResult<(WriteAnswer, sqlx::Transaction<'static, sqlx::Postgres>)> {
    let (idp_key, group, team_id) = key;
    let mut tx = pool.begin().await?;
    if !crate::auth::is_system_admin_locked(&mut tx, admin.actor()).await? {
        return Err(ApiError::Forbidden);
    }
    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, admin.actor(), "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let raw = sqlx::query_scalar!(
        r#"SELECT _admin_saml_group_mapping_write(
                  p_emitter => $1, p_actor => $2, p_act => $3, p_idp_key => $4, p_group => $5,
                  p_team => $6, p_role => $7, p_confirmed => $8) AS "w!""#,
        emitter.uuid(),
        *admin.actor(),
        act,
        idp_key,
        group,
        team_id,
        role as Option<TeamRole>,
        confirmed,
    )
    .fetch_one(&mut *tx)
    .await?;
    let answer: WriteAnswer = serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("_admin_saml_group_mapping_write: {e}")))?;
    Ok((answer, tx))
}

/// Create a mapping. Onto `temper-system` or the gating team, `watcher` only: any other role is
/// refused (`400`, nothing written) with or without `confirm_elevated`, since a higher role there
/// is a system admin's native act. Elsewhere, refused (`400`, naming why, nothing written) without
/// `confirm_elevated` when the role is `owner`, or is an authoring role (owner, maintainer, member)
/// and the team's subtree, dead teams included, holds a pool (the gating team or an auto-join team
/// beneath it). A `watcher` mapping never needs it. `404` for an unknown IdP
/// or a team that is not live; `400` for a personal team; `409` when the key already maps to
/// another role, or when the team's subtree changed while the act waited for its locks (retry).
/// The same mapping again is answered `created: false` and records nothing.
pub async fn create(
    pool: &PgPool,
    admin: &SystemAdmin,
    req: &CreateSamlGroupMappingRequest,
) -> ApiResult<CreateSamlGroupMappingOutcome> {
    // Stored exactly as given: the reconcile matches the asserted value byte for byte.
    let (idp_key, group) = (req.idp_key.as_str(), req.group.as_str());
    refuse_blank(idp_key, group)?;
    let (answer, tx) = write(
        pool,
        admin,
        "created",
        (idp_key, group, req.team_id),
        Some(req.role),
        req.confirm_elevated,
    )
    .await?;
    let mapping = SamlGroupMapping {
        idp_key: idp_key.to_string(),
        group: group.to_string(),
        team_id: req.team_id,
        role: req.role,
    };
    let created = match answer.outcome.as_str() {
        "changed" => true,
        "noop" => false,
        "needs_confirmation" => return Err(needs_confirmation(&answer.needs, req.role)),
        "absent_idp" => {
            return Err(ApiError::NotFound(format!(
                "no SAML IdP is configured with idp_key `{idp_key}`"
            )))
        }
        "absent_team" => {
            return Err(ApiError::NotFound(
                "team not found or soft-deleted".to_string(),
            ))
        }
        "personal_team" => {
            return Err(ApiError::BadRequest(
                "a personal team takes no other member, so no SAML mapping can target one"
                    .to_string(),
            ))
        }
        "instance_team_watcher_only" => {
            return Err(ApiError::BadRequest(format!(
                "an identity-provider group confers at most `watcher` on temper-system or the \
                 gating team; `{}` there is a system admin's native act (add the member directly), \
                 with or without `confirm_elevated`. Nothing was written",
                req.role.as_str()
            )))
        }
        "subtree_changed" => {
            return Err(ApiError::Conflict(
                "the team tree changed while this act was being checked; retry".to_string(),
            ))
        }
        "conflict" => {
            let held = answer.role.map(|r| r.as_str()).unwrap_or("another role");
            return Err(ApiError::Conflict(format!(
                "this group already maps to the team as `{held}`; remove that mapping first to \
                 change its role"
            )));
        }
        other => {
            return Err(ApiError::Internal(format!(
                "_admin_saml_group_mapping_write answered {other}"
            )))
        }
    };
    tx.commit().await?;
    Ok(CreateSamlGroupMappingOutcome {
        mapping,
        created,
        confirmed: answer.confirmed,
    })
}

/// Remove a mapping. `404` when no such mapping exists. Deletes no membership: the answer counts the
/// team's IdP memberships it left in place and says what governs them.
pub async fn remove(
    pool: &PgPool,
    admin: &SystemAdmin,
    params: &RemoveSamlGroupMappingParams,
) -> ApiResult<RemoveSamlGroupMappingOutcome> {
    refuse_blank(&params.idp_key, &params.group)?;
    let (answer, tx) = write(
        pool,
        admin,
        "removed",
        (&params.idp_key, &params.group, params.team_id),
        None,
        false,
    )
    .await?;
    match answer.outcome.as_str() {
        "changed" => {}
        "absent" => return Err(ApiError::NotFound("no such SAML group mapping".to_string())),
        other => {
            return Err(ApiError::Internal(format!(
                "_admin_saml_group_mapping_write answered {other}"
            )))
        }
    }
    tx.commit().await?;
    let role = answer.role.ok_or_else(|| {
        ApiError::Internal("_admin_saml_group_mapping_write removed without a role".to_string())
    })?;
    Ok(RemoveSamlGroupMappingOutcome {
        removed: SamlGroupMapping {
            idp_key: params.idp_key.clone(),
            group: params.group.clone(),
            team_id: params.team_id,
            role,
        },
        idp_memberships_left_in_place: answer.idp_memberships.unwrap_or(0),
        memberships: MAPPING_REMOVAL_LEAVES_MEMBERSHIPS.to_string(),
    })
}

/// The authoring-role mappings at or above an act's seed teams whose subtree holds a pool, read by
/// a structural team act on its own transaction after its row locks and before its write. Hand it
/// to [`record_elevations`] after the write.
pub(crate) struct ElevationBefore {
    seeds: Vec<Uuid>,
    before: serde_json::Value,
}

/// Read [`ElevationBefore`] on `conn` (`_saml_mappings_over_pool`) for `seeds`: the teams the act
/// writes at (the parent it links under, the team it flips, the new gating team), the only teams
/// at or above which it can put a pool. Takes `_public_within_effects_lock` first, which every
/// elevating act holds to commit, so two acts never both record one mapping; it is re-entrant, and
/// the act would take it later in its write anyway.
pub(crate) async fn elevation_before(
    conn: &mut sqlx::PgConnection,
    seeds: &[Uuid],
) -> ApiResult<ElevationBefore> {
    sqlx::query_scalar!(r#"SELECT 1 AS "one!" FROM (SELECT _public_within_effects_lock()) l"#)
        .fetch_one(&mut *conn)
        .await?;
    let before = sqlx::query_scalar!(r#"SELECT _saml_mappings_over_pool($1) AS "m!""#, seeds)
        .fetch_one(&mut *conn)
        .await?;
    Ok(ElevationBefore {
        seeds: seeds.to_vec(),
        before,
    })
}

/// After a structural act's write, on its transaction: every authoring-role mapping at or above the
/// act's seeds now over a pool that `before` did not hold. Each gets one `saml_group_mapping_changed`
/// event, act `elevated`, naming `actor` (`_saml_mapping_elevations_record`), and is answered,
/// ordered by idp_key, group and team. The one scan of every door that can put a pool beneath a
/// team, so the doors cannot drift apart. `None` (an act on an arm that cannot put a pool beneath
/// a team, which is every non-admin arm) answers none and records nothing.
pub(crate) async fn record_elevations(
    conn: &mut sqlx::PgConnection,
    before: Option<ElevationBefore>,
    emitter: Uuid,
    actor: temper_core::types::ids::ProfileId,
    correlation: Option<Uuid>,
) -> ApiResult<Vec<SamlGroupMapping>> {
    let Some(before) = before else {
        return Ok(Vec::new());
    };
    let raw = sqlx::query_scalar!(
        r#"SELECT _saml_mapping_elevations_record($1, $2, $3, $4, $5) AS "e!""#,
        emitter,
        *actor,
        &before.seeds,
        before.before,
        correlation,
    )
    .fetch_one(&mut *conn)
    .await?;
    serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("_saml_mapping_elevations_record: {e}")))
}

/// After a gating repoint, on its transaction: each mapping above `watcher` onto the new gating team
/// gets one `saml_group_mapping_changed` event, act `clamped`, naming `actor`
/// (`_saml_mappings_clamped_record`): the IdP reconcile confers `watcher` there from now on. Answered,
/// ordered by idp_key, group and team.
pub(crate) async fn record_clamps(
    conn: &mut sqlx::PgConnection,
    teams: &[Uuid],
    emitter: Uuid,
    actor: temper_core::types::ids::ProfileId,
    correlation: Option<Uuid>,
) -> ApiResult<Vec<SamlGroupMapping>> {
    let raw = sqlx::query_scalar!(
        r#"SELECT _saml_mappings_clamped_record($1, $2, $3, NULL, $4) AS "c!""#,
        emitter,
        *actor,
        teams,
        correlation,
    )
    .fetch_one(&mut *conn)
    .await?;
    serde_json::from_value(raw)
        .map_err(|e| ApiError::Internal(format!("_saml_mappings_clamped_record: {e}")))
}

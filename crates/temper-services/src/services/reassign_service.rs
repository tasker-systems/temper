//! Resource ownership reassignment over `kb_resource_homes`, event-sourced via
//! `writes::reassign_resource_with`. Service-direct (no `DbBackend` command),
//! same precedent as `invitation_service` / `team_service`. Authorization
//! precedes every write.
//!
//! Two authorized paths: the current owner may reassign their own resource to
//! anyone (mis-attribution self-fix); a team admin may reassign a resource
//! homed in a context their team owns, owned by a current or former member of it, to a member
//! of that team (offboarding +
//! admin-assisted mis-attribution). Only **context-homed** resources are
//! reassignable — a cogmap interior is team-resource-derived, not personally
//! owned, and is refused here (and by the `resource_reassign` SQL backstop).

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::auth::HumanPrincipal;
use crate::backend::write_floor;
use crate::error::{ApiError, ApiResult};
use crate::services::team_service::{can_manage, role_on_team};
use temper_core::error::TemperError;
use temper_core::types::ids::ProfileId;

/// A resource's home owner + the anchor table it's homed under.
struct HomeRow {
    owner: Uuid,
    anchor_table: String,
}

async fn home_of(conn: &mut PgConnection, resource: Uuid) -> ApiResult<Option<HomeRow>> {
    Ok(sqlx::query_as!(
        HomeRow,
        "SELECT owner_profile_id AS owner, anchor_table \
           FROM kb_resource_homes WHERE resource_id = $1",
        resource,
    )
    .fetch_optional(&mut *conn)
    .await?)
}

/// Is there a team T where caller manages T, `resource` is homed in a context **owned by T**, `to` is a
/// member of T, and the resource's current owner is a current or former member of T? T is never the
/// root or the gating team. (admin-path reach: from-scope + into-scope.)
///
/// Both the team and the context must be live. A retired context closes this arm the
/// same way it closes the team-share read arm in `contexts_readable_by_teams` — without
/// that floor, an admin whose only reach was the team share could reassign a retired
/// context's resource to themselves and then read it through the owner arm of
/// `resources_visible_to`, which has no context floor. Witness:
/// `admin_cannot_reassign_via_retired_context`.
///
/// **Reassign acts only inside T's own containers** (team-authority fixes D5). A read-only share of
/// someone's context into T used to let T's managers take ownership of the resources in it. A
/// share confers read and nothing more, so only contexts T owns are in scope: the one place T's
/// managers already govern the container. Membership is no bar on its own — a manager can add
/// anyone to T — so it is kept as an extra check, not as the authority.
async fn admin_reach(
    conn: &mut PgConnection,
    caller: ProfileId,
    resource: Uuid,
    to: Uuid,
) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM kb_resource_homes h
            JOIN kb_contexts c
              ON c.id = h.anchor_id AND c.is_active
            JOIN kb_teams t
              ON t.is_active
             AND c.owner_table = 'kb_teams' AND c.owner_id = t.id
             AND t.slug <> 'temper-system'
             AND t.slug IS DISTINCT FROM
                 (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1)
            JOIN kb_team_members cm
              ON cm.team_id = t.id AND cm.profile_id = $2
                 AND cm.role IN ('owner', 'maintainer')
            JOIN kb_team_members tm
              ON tm.team_id = t.id AND tm.profile_id = $3
            WHERE h.resource_id = $1
              AND h.anchor_table = 'kb_contexts'
              AND ( EXISTS (SELECT 1 FROM kb_team_members om
                             WHERE om.team_id = t.id AND om.profile_id = h.owner_profile_id)
                 OR EXISTS (SELECT 1 FROM kb_events e
                              JOIN kb_event_types et ON et.id = e.event_type_id
                             WHERE et.name = 'team_membership_changed'
                               AND e.payload @> jsonb_build_object(
                                     'team_id', t.id, 'profile_id', h.owner_profile_id)) )
        ) AS "exists!: bool"
        "#,
        resource,
        *caller,
        to,
    )
    .fetch_one(&mut *conn)
    .await?)
}

/// Is `profile_id` a current member of `team_id`, or a former one — a `team_membership_changed`
/// admin event names them on it? The "former" half is what offboarding needs: the handoff runs
/// after the member is gone. A removal that predates the ledger (`20261101200000`) left no record,
/// so that leaver is reassigned with the admin path instead.
async fn is_or_was_member(pool: &PgPool, team_id: Uuid, profile_id: Uuid) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT ( EXISTS (SELECT 1 FROM kb_team_members
                             WHERE team_id = $1 AND profile_id = $2)
                 OR EXISTS (SELECT 1 FROM kb_events e
                              JOIN kb_event_types et ON et.id = e.event_type_id
                             WHERE et.name = 'team_membership_changed'
                               AND e.payload @> jsonb_build_object(
                                     'team_id', $1::uuid, 'profile_id', $2::uuid)) ) AS "e!: bool""#,
        team_id,
        profile_id,
    )
    .fetch_one(pool)
    .await?)
}

/// The single reassign's authority, decided against `resource`'s home as `conn` reads it now:
/// the caller owns it, or has [`admin_reach`] over it and `to`; and it is context-homed. Answers
/// the home the decision was made against. An id with no home is refused as a resource the caller
/// has no authority over (`403`, the same body).
async fn authorize_single(
    conn: &mut PgConnection,
    caller: ProfileId,
    resource_id: Uuid,
    to_profile_id: Uuid,
) -> ApiResult<HomeRow> {
    let Some(home) = home_of(conn, resource_id).await? else {
        return Err(ApiError::Forbidden);
    };
    let authorized =
        home.owner == *caller || admin_reach(conn, caller, resource_id, to_profile_id).await?;
    if !authorized {
        return Err(ApiError::Forbidden);
    }
    // Only context-homed resources are reassignable. The owner path would otherwise
    // let a cogmap-node owner flip it — guard here for BOTH paths (the admin path's
    // reach query already excludes non-context homes structurally, so only an owner
    // reaches this refusal).
    if home.anchor_table != "kb_contexts" {
        return Err(ApiError::BadRequest(
            "cannot reassign a cogmap-homed resource; map interiors are not personally owned"
                .to_string(),
        ));
    }
    Ok(home)
}

/// Reassign a resource's owner to `to_profile_id`. Auth: current owner (any target)
/// OR team-admin over a team the resource is scoped to, to a member of that team.
/// Reassigning to the current owner is an idempotent no-op. Cogmap-homed resources
/// are rejected (map interiors are not personally owned).
///
/// Authority is decided FIRST, and an id with no home is refused exactly as an existing resource
/// the caller has no authority over: `403`, the same body. Nothing the door answers before the
/// authority check may depend on the id — otherwise the status says whether the id is a resource,
/// and what kind of home it has, to any caller.
pub async fn reassign_resource(
    pool: &PgPool,
    caller: &HumanPrincipal,
    resource_id: Uuid,
    to_profile_id: Uuid,
) -> ApiResult<()> {
    let caller = caller.profile_id();
    let resource = temper_substrate::ids::ResourceId::from(resource_id);
    // Auth before writes, unlocked: current owner, or an admin with reach over the
    // resource+target. A caller refused here never takes a row lock (the write floor's "a refused
    // caller takes no lock"). It binds nothing; the decision under the lock below does.
    {
        let mut conn = pool.acquire().await?;
        authorize_single(&mut conn, caller, resource_id, to_profile_id).await?;
    }

    // The liveness floor (resource erasure spec D13; plan 2c, controller ruling 5), at the head of
    // the reassign's own transaction: the resource must be live, under the `FOR KEY SHARE` row
    // lock the erasure act's `FOR UPDATE` conflicts with, held until the reassign commits. Liveness
    // only — the authority stays the gate above. A refusal is classified as every write floor's
    // is: 410 `RESOURCE_ERASED` to a holder of an erased husk, 403 otherwise (a soft-deleted
    // resource always 403). It runs ahead of the no-op return so a husk is never answered `200`.
    let mut tx = pool.begin().await?;
    if let Err(refusal) = write_floor::liveness_floor_in_tx(&mut tx, caller, resource).await {
        return Err(write_floor::rollback_with(tx, ApiError::from(refusal)).await);
    }
    // R's writer lock, the one a move takes, after the floor (a move's order: floor, then this),
    // then the authority again under it. A move of R that was in flight has committed by now, so
    // the home read here is the one the event will anchor on, and the authority is decided
    // against it: an admin whose reach came through the home R just left has none over the one it
    // is in. Witnesses: `reassign_move_race_test.rs`.
    if let Err(e) = temper_substrate::writes::lock_resource_for_write(&mut tx, resource).await {
        return Err(write_floor::rollback_with(tx, ApiError::from_write(e)).await);
    }
    let home = match authorize_single(&mut tx, caller, resource_id, to_profile_id).await {
        Ok(home) => home,
        Err(refusal) => return Err(write_floor::rollback_with(tx, refusal).await),
    };
    if home.owner == to_profile_id {
        return Ok(()); // idempotent no-op
    }

    // The owner path admits any target; verify it's a real profile so a bad UUID is a
    // clean 400 rather than an FK-violation 500 in the projector. (The admin path's
    // membership join already guarantees existence, but this covers both uniformly.) This and the
    // emitter below run on the reassign's own transaction, never the pool: the transaction already
    // holds a pool connection, and a second acquire while it is open is how concurrent writers
    // stall the pool.
    let to_exists = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_profiles WHERE id = $1) AS "e!: bool""#,
        to_profile_id,
    )
    .fetch_one(&mut *tx)
    .await?;
    if !to_exists {
        return Err(ApiError::BadRequest(
            "target profile does not exist".to_string(),
        ));
    }

    let emitter = temper_substrate::writes::resolve_emitter(&mut *tx, caller, "web")
        .await
        .map_err(ApiError::from_write)?;
    temper_substrate::writes::reassign_resource_in_tx(
        &mut tx,
        resource,
        home.owner.into(),
        to_profile_id.into(),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(ApiError::from_write)?;
    tx.commit().await?;
    Ok(())
}

/// A resource owned by a given profile and homed in a context owned by a given team. The single definition of "what a departing member still owns in this
/// team" — consumed by both the bulk handoff (the move set) and `remove_member`'s
/// residual surfacing (the count + per-context breakdown), so the two can never drift.
pub struct ScopedOwnedRow {
    pub resource_id: Uuid,
    pub context_id: Uuid,
    /// Decorated context ref `{owner_ref}/{slug}` (owner_ref: `@handle` | `+slug`).
    pub context_ref: String,
}

/// *Live* resources owned by `profile_id` and homed in a *live* context OWNED by `team_id`. A context
/// merely shared to the team is out of scope: a share confers read, never authority over the
/// container's contents (team-authority fixes D5).
/// A retired context drops out of this scope, and so does a soft-deleted or erased resource
/// (`kb_resources.is_active`), so the bulk handoff and `remove_member`'s residual surfacing
/// both stop counting either and agree on what is left to hand off. Witnesses:
/// `bulk_excludes_a_retired_contexts_resources`, and in `resource_husk_write_test.rs`
/// `remove_member_and_team_reassign_leave_out_a_husk_and_a_tombstone`.
///
/// Ordered by `(context slug, resource_id)` for stable output only — consumers
/// that group by context MUST key on `context_id` (a slug is unique only per-owner,
/// so two distinct contexts can share a slug and are NOT guaranteed to be adjacent).
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
        FROM kb_contexts c
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
        JOIN kb_resources r ON r.id = h.resource_id AND r.is_active
        LEFT JOIN kb_teams    t ON c.owner_table = 'kb_teams'    AND t.id = c.owner_id
        LEFT JOIN kb_profiles p ON c.owner_table = 'kb_profiles' AND p.id = c.owner_id
        WHERE c.is_active
          AND h.owner_profile_id = $2
          -- Only the team's OWN contexts (team-authority fixes D5): a share confers read, never
          -- authority over what is in the shared container.
          AND c.owner_table = 'kb_teams' AND c.owner_id = $1
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

/// Is `resource` still in [`team_scoped_owned`]'s scope for `team_id` and `profile_id`: live,
/// owned by `profile_id`, and homed in a live context owned by `team_id`? The bulk handoff asks it
/// per resource under R's writer lock, so the scope it read on the pool is re-decided against the
/// home the event will anchor on.
async fn still_in_team_scope(
    conn: &mut PgConnection,
    team_id: Uuid,
    profile_id: Uuid,
    resource: Uuid,
) -> ApiResult<bool> {
    Ok(sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1
               FROM kb_resource_homes h
               JOIN kb_resources r ON r.id = h.resource_id AND r.is_active
               JOIN kb_contexts c
                 ON h.anchor_table = 'kb_contexts' AND c.id = h.anchor_id AND c.is_active
                AND c.owner_table = 'kb_teams' AND c.owner_id = $1
              WHERE h.resource_id = $3 AND h.owner_profile_id = $2
           ) AS "e!: bool""#,
        team_id,
        profile_id,
        resource,
    )
    .fetch_one(&mut *conn)
    .await?)
}

/// Bulk-reassign, from `from_profile_id` to `to_profile_id`, every LIVE resource owned by
/// `from` and homed in a context owned by `team_id`. Auth: caller manages the team (never the root
/// or the gating team), `to` is a member of it, and `from` is or was a member of it. One transaction; returns the reassigned resource ids. A soft-deleted or
/// erased resource is outside the scope; one soft-deleted or erased after the scope read fails the
/// liveness floor, and one moved out of the scope after it fails `still_in_team_scope`: either is
/// skipped and is not among them.
pub async fn reassign_team_resources(
    pool: &PgPool,
    caller: &HumanPrincipal,
    team_id: Uuid,
    from_profile_id: Uuid,
    to_profile_id: Uuid,
) -> ApiResult<Vec<Uuid>> {
    let caller = caller.profile_id();
    // A soft-deleted team is inert — it confers no reassignment authority. `role_on_team` is
    // live-only too; this check stays because it answers `Forbidden` before any lookup by role.
    let team_active = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_teams WHERE id = $1 AND is_active) AS "e!: bool""#,
        team_id,
    )
    .fetch_one(pool)
    .await?;
    if !team_active {
        return Err(ApiError::Forbidden);
    }

    // Auth before writes: caller manages the team, and `to` is a member of it. Never on the root or
    // the gating team, where a leftover managing row (a demoted admin's) confers nothing.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }
    let root_or_gating =
        crate::services::access_service::is_root_or_gating_team(pool, team_id).await?;
    if root_or_gating {
        return Err(ApiError::Forbidden);
    }
    let to_is_member = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_team_members WHERE team_id=$1 AND profile_id=$2) AS "e!: bool""#,
        team_id,
        to_profile_id,
    )
    .fetch_one(pool)
    .await?;
    if !to_is_member {
        return Err(ApiError::Forbidden);
    }
    // ...and `from` is, or was, on the team: reassign hands off a member's work, never a sharer's
    // (team-authority fixes D5).
    if !is_or_was_member(pool, team_id, from_profile_id).await? {
        return Err(ApiError::Forbidden);
    }

    // Scope read: the shared definition (owned by `from` ∩ homed in a team-shared context).
    let targets: Vec<Uuid> = team_scoped_owned(pool, team_id, from_profile_id)
        .await?
        .into_iter()
        .map(|r| r.resource_id)
        .collect();
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(ApiError::from_write)?;

    let mut tx = pool.begin().await?;
    let mut reassigned = Vec::with_capacity(targets.len());
    for &rid in &targets {
        let resource = temper_substrate::ids::ResourceId::from(rid);
        // The liveness floor per resource (controller ruling 5), in the run's transaction, ahead
        // of its write. `team_scoped_owned` enumerates live resources only, so the floor refuses
        // one erased or deleted after the scope read. It is SKIPPED — left with its owner and
        // absent from the returned ids — never a whole-run failure. Only the floor's two refusals
        // skip; a fault aborts the run as before.
        match write_floor::liveness_floor_bulk_in_tx(&mut tx, caller, resource).await {
            Ok(()) => {}
            Err(TemperError::Forbidden | TemperError::ResourceErased(_)) => continue,
            Err(e) => return Err(e.into()),
        }
        // R's writer lock, the one a move takes, then the scope again under it: a resource moved
        // out of the team's contexts, or reassigned away from `from`, after the scope read is
        // skipped like one the floor refuses. Without it the run would hand off a resource in a
        // context the team does not own, and anchor its event on the home the move left.
        temper_substrate::writes::lock_resource_for_write(&mut tx, resource)
            .await
            .map_err(ApiError::from_write)?;
        if !still_in_team_scope(&mut tx, team_id, from_profile_id, rid).await? {
            continue;
        }
        temper_substrate::writes::reassign_resource_in_tx(
            &mut tx,
            resource,
            from_profile_id.into(),
            to_profile_id.into(),
            emitter,
            temper_substrate::events::EventContext::default(),
        )
        .await
        .map_err(ApiError::from_write)?;
        reassigned.push(rid);
    }
    tx.commit().await?;
    Ok(reassigned)
}

#[cfg(all(test, feature = "test-db"))]
mod tests;

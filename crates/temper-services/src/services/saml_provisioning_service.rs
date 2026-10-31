//! SAML-driven team-membership reconciliation (Phase 2). Applies an operator-maintained
//! `(idp_key, group) -> (team, role)` mapping to `kb_team_members` rows tagged `source='idp'`,
//! leaving `source='native'` rows untouched (native-wins-skip). See the Phase 2 design spec.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::team::TeamMemberSource;
use temper_core::types::TeamRole;

use crate::authz::team_membership;
use crate::error::{ApiError, ApiResult};

/// Proof that a reconcile transaction is writing: defined here with a private field and
/// constructed only in this module's reconcile flow, so the `idp` constructors in
/// `authz::team_membership` cannot be called from anywhere else (plan P16).
#[derive(Debug, Clone, Copy)]
pub(crate) struct IdpReconcileToken {
    _private: (),
}

/// The stronger of two roles (used when two asserted groups map to the same team).
fn max_role(a: TeamRole, b: TeamRole) -> TeamRole {
    if a.rank() >= b.rank() {
        a
    } else {
        b
    }
}

/// Counts of what a reconcile pass changed. Returned for logging/observability.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReconcileCounts {
    pub added: usize,
    pub updated: usize,
    pub revoked: usize,
    pub skipped_native: usize,
    /// Teams whose last owner the reconcile removed or demoted (R8: the identity provider
    /// wins over the last-owner guard). The event carries `left_ownerless`; a system admin
    /// repairs the team.
    pub left_ownerless: usize,
}

/// What a call to [`reconcile_idp_memberships`] did — an enum, so that "declined to try" cannot be
/// read as "tried and changed nothing".
///
/// Those two were the same value until this type split: a skip returned nothing at all and a
/// reconcile that found everything already in agreement returned `ReconcileCounts::default()`, so
/// every count was zero in both cases and the caller's log line was identical. The distinction
/// matters more than most: a reconcile with all-zero counts means the principal's reach IS in
/// agreement, and a skip means nothing was compared. Only one of them is evidence about reach.
#[derive(Debug, Clone, Copy)]
pub enum ReconcileOutcome {
    /// No group signal was presented, so no comparison was made and no membership was touched.
    /// Recorded as `last_skipped_at` — see [`reconcile_idp_memberships`].
    SignalMissing,
    /// The asserted group set was applied. Reach is in agreement as of now.
    Reconciled(ReconcileCounts),
}

/// A single mapping row after filtering to asserted groups, collapsed per team.
struct DesiredMembership {
    team_id: Uuid,
    role: TeamRole,
}

/// Reconcile the profile's `source='idp'` team memberships to match the asserted groups.
///
/// Native memberships (`source='native'`) are sacred: if one exists for a `(team, profile)`
/// pair, that team is skipped entirely (native-wins-skip). Every write is a recorded act through
/// the membership writer (`via = idp_reconcile`, plan C4): the touched teams are locked in
/// ascending id order FIRST and `current` is re-read under the locks, so a native row inserted
/// mid-reconcile is skipped rather than written over (S1), and the identity provider wins over
/// the last-owner guard (R8) — a reconcile that leaves a team ownerless records
/// `left_ownerless` on the event and in the counts. Each pass runs in one transaction, so a
/// failure leaves membership state unchanged (fail-open at the caller).
///
/// The set to lock comes from an unlocked pre-read, so an idp row on a team outside it can commit
/// before the locked re-read. The pass then rolls back and the next one locks the widened set from
/// the start, rather than lock that team late and out of order. After two passes it gives up.
///
/// # `groups` is an `Option`, and that is the guard
///
/// `None` means the assertion carried **no group signal** — the named attribute was absent from
/// this particular assertion (`packages/temper-cloud/src/saml/sp.ts`). It is NOT a provider saying
/// "this principal is in no groups", and it must never revoke anything. `Some(&[])` IS that second
/// statement, and does revoke.
///
/// The caller reaches this function only for an IdP that has group provisioning configured
/// (`groupProvisioningConfigured`), so a `None` arriving here carries the actionable reading: it
/// was expected to arrive and did not. An authentication-only IdP has no reconcile to perform and
/// makes no call, which is why the record this writes stays free of a whole deployment class that
/// has no de-provisioning to suspend.
///
/// The distinction is carried by the type rather than by a convention about empty slices, and it
/// is carried *here* rather than at the caller, because this is the function that holds the
/// `DELETE`. A caller cannot express "no signal" in a form this function might act on, and the
/// early return below is the only path a `None` has.
///
/// Both paths leave a record in `kb_saml_principal_reconcile` (`20260827000030`), because a
/// de-provisioning that was declined and one that agreed are different facts and neither is
/// visible in the membership rows themselves.
pub async fn reconcile_idp_memberships(
    pool: &PgPool,
    profile_id: Uuid,
    idp_key: &str,
    groups: Option<&[String]>,
) -> ApiResult<ReconcileOutcome> {
    // The guard, and the whole of it: no group signal, so nothing is compared and nothing is
    // touched. Recorded so that the silence is a fact someone can query rather than an absence
    // indistinguishable from "this principal never authenticated".
    let Some(groups) = groups else {
        record_signal_missing(pool, profile_id, idp_key).await?;
        return Ok(ReconcileOutcome::SignalMissing);
    };

    // 0. Discovery capture: record EVERY asserted group (mapped or not) so operators can see
    //    what the IdP sends and add mappings reactively. Autonomous (not in the reconcile tx
    //    below) so discovery data survives even if the reconcile fails. No-op when no groups.
    if !groups.is_empty() {
        sqlx::query!(
            r#"INSERT INTO kb_saml_seen_groups (idp_key, group_value)
               SELECT $1, g FROM UNNEST($2::text[]) AS g
               ON CONFLICT (idp_key, group_value) DO UPDATE SET last_seen = now()"#,
            idp_key,
            groups,
        )
        .execute(pool)
        .await?;
    }

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

    // 2–5, in passes (task `01a14592-708c`). A pass locks every team it will touch, in ascending
    // id order, BEFORE any write. If its locked re-read finds a row on a team it did not lock, it
    // rolls back and the next pass locks the widened set from the start, so no team lock is ever
    // taken out of order — and none after `try_membership_write` has locked and revoked invitation
    // rows on an earlier team (task `01a14591`). `to_lock` only grows across passes.
    let mut to_lock: Vec<Uuid> = desired.keys().copied().collect();
    for _ in 0..RECONCILE_PASSES {
        if let Some(counts) =
            reconcile_pass(pool, profile_id, idp_key, &desired, &mut to_lock).await?
        {
            return Ok(ReconcileOutcome::Reconciled(counts));
        }
    }
    // A row appeared on an unlocked team in every pass. Membership is left as it was — the same
    // fail-open a deadlock abort gave before the passes — and the next login reconciles again.
    Err(ApiError::Conflict(RECONCILE_RACED.to_string()))
}

/// How many passes [`reconcile_idp_memberships`] makes before it gives up. The second pass locks
/// every team the first one found, so only a row appearing on yet another team, in the gap
/// between the passes, makes it fail.
const RECONCILE_PASSES: usize = 2;

/// The refusal when a row kept appearing on a team the reconcile had not locked.
const RECONCILE_RACED: &str =
    "team membership changed during the identity-provider reconcile; retry";

/// One pass of [`reconcile_idp_memberships`]' steps 2–5, in its own transaction. `None` means the
/// pass found a row on a team outside `to_lock`, widened `to_lock` with it and rolled back, having
/// written nothing; `Some` is a committed reconcile.
async fn reconcile_pass(
    pool: &PgPool,
    profile_id: Uuid,
    idp_key: &str,
    desired: &HashMap<Uuid, TeamRole>,
    to_lock: &mut Vec<Uuid>,
) -> ApiResult<Option<ReconcileCounts>> {
    let mut tx = pool.begin().await?;

    // 2. The pre-read: the team set to lock. It is not trusted for DECISIONS — that is the
    //    locked re-read below (S1). Today's hole was deciding on this read: a native row
    //    inserted between it and the write was invisible, and reconcile updated or revoked a
    //    row that was no longer there.
    let pre = sqlx::query!(
        r#"SELECT team_id, role AS "role: TeamRole", source::text AS "source: String"
           FROM kb_team_members WHERE profile_id = $1"#,
        profile_id,
    )
    .fetch_all(&mut *tx)
    .await?;

    // 2a. LOCK FIRST (plan C4/S1): every recorded membership writer takes the team row lock, so
    //     taking it for every touched team — single-row writes included — serializes reconcile
    //     with them and makes the re-read below current state. Ascending id, the multi-team
    //     lock order; one statement locks in output order.
    to_lock.extend(pre.iter().map(|c| c.team_id));
    to_lock.sort();
    to_lock.dedup();
    let locked = sqlx::query!(
        r#"SELECT id AS "id!", is_active AS "is_active!"
             FROM kb_teams WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE"#,
        &to_lock[..],
    )
    .fetch_all(&mut *tx)
    .await?;
    // Nothing is written onto a soft-deleted team (soft-delete design D4): a group mapped to one
    // is skipped, in both directions, rather than failing the whole login. Its idp rows confer
    // nothing while the team is dead.
    let dead: std::collections::HashSet<Uuid> = locked
        .iter()
        .filter(|t| !t.is_active)
        .map(|t| t.id)
        .collect();

    // 2b. Current state under the locks: role + source per team, as it NOW stands.
    let current = sqlx::query!(
        r#"SELECT team_id, role AS "role: TeamRole", source::text AS "source: String"
           FROM kb_team_members WHERE profile_id = $1"#,
        profile_id,
    )
    .fetch_all(&mut *tx)
    .await?;

    // 2c. An idp row on a team the pre-read did not see — a concurrent reconcile's — committed
    //     before the re-read. Step 3 writes only `desired` teams, all locked, and step 4 only idp
    //     rows, so this is the only way a write could need a lock taken after the others. Widen
    //     and start again instead. A native row on an unlocked team is never written, so it never
    //     restarts a pass: a user creating teams mid-login cannot run the passes out and keep
    //     their idp rows past a revocation.
    let unlocked: Vec<Uuid> = current
        .iter()
        .filter(|c| c.source.as_deref() != Some("native"))
        .map(|c| c.team_id)
        .filter(|t| to_lock.binary_search(t).is_err())
        .collect();
    if !unlocked.is_empty() {
        to_lock.extend(unlocked);
        tx.rollback().await?;
        return Ok(None);
    }

    let mut native_teams: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut idp_current: HashMap<Uuid, TeamRole> = HashMap::new();
    for c in &current {
        if c.source.as_deref() == Some("native") {
            native_teams.insert(c.team_id);
        } else {
            idp_current.insert(c.team_id, c.role);
        }
    }

    let token = IdpReconcileToken { _private: () };
    let mut out = ReconcileCounts::default();

    // 3. Add / update desired teams (skipping any team the profile is native in).
    for m in desired
        .iter()
        .map(|(&team_id, &role)| DesiredMembership { team_id, role })
    {
        if dead.contains(&m.team_id) {
            continue;
        }
        if native_teams.contains(&m.team_id) {
            out.skipped_native += 1;
            continue;
        }
        match idp_current.get(&m.team_id) {
            Some(&existing) if existing == m.role => {}
            _ => {
                let target =
                    team_membership::locked(&mut tx, m.team_id, ProfileId::from(profile_id))
                        .await?;
                // The team lock serializes every routed writer, but not the derived ones (R13:
                // auto-join, personal-team sync), which can still insert a native row between
                // the re-read above and this row lock. Native wins (C7): skip, never write.
                if let Some((_, TeamMemberSource::Native)) = target.current() {
                    out.skipped_native += 1;
                    continue;
                }
                let lowers_owner = matches!(target.current(), Some((TeamRole::Owner, _)));
                let change =
                    team_membership::AuthorizedMembershipChange::idp(token, &target, m.role)?;
                match team_membership::try_membership_write(&mut tx, None, change).await? {
                    Ok(team_membership::MembershipOutcome::Added) => out.added += 1,
                    Ok(team_membership::MembershipOutcome::Changed) => {
                        out.updated += 1;
                        if lowers_owner {
                            count_ownerless(&mut tx, m.team_id, &mut out).await?;
                        }
                    }
                    Ok(_) => {}
                    // An add found a row: a derived writer inserted a native one after the
                    // member-row lock above, which locks no row that does not exist yet (C7).
                    Err(team_membership::NotWritten::Exists) => out.skipped_native += 1,
                    Err(other) => return Err(idp_write_refused(other)),
                }
            }
        }
    }

    // 4. Revoke idp memberships no longer desired.
    // NOTE: filter's predicate takes `&Self::Item`, so `t` binds as `&&Uuid` under match
    // ergonomics — deref once so `contains_key` sees `&Uuid` (HashMap<Uuid, _> has no
    // `Borrow<&Uuid>` impl, so passing `t` directly would fail to type-check).
    for (&team_id, _) in idp_current
        .iter()
        .filter(|(t, _)| !desired.contains_key(*t) && !dead.contains(*t))
    {
        let target = team_membership::locked(&mut tx, team_id, ProfileId::from(profile_id)).await?;
        let lowers_owner = matches!(target.current(), Some((TeamRole::Owner, _)));
        let change = team_membership::AuthorizedMembershipChange::idp_removal(token, &target)?;
        match team_membership::try_membership_write(&mut tx, None, change).await? {
            Ok(team_membership::MembershipOutcome::Removed) => {
                out.revoked += 1;
                if lowers_owner {
                    count_ownerless(&mut tx, team_id, &mut out).await?;
                }
            }
            Ok(_) => {}
            Err(other) => return Err(idp_write_refused(other)),
        }
    }

    // 5. The agreement is now a fact, so record when it was reached — INSIDE the transaction.
    //
    // Deliberately unlike the discovery capture at the top of this function, which is autonomous so
    // that discovery data survives a failed reconcile. The reason inverts here: discovery data is
    // meaningful on its own, whereas `last_reconciled_at` is a CLAIM ABOUT THE MEMBERSHIP ROWS this
    // transaction is writing. Written outside, a rollback would leave the claim standing over state
    // it does not describe. Written here, the claim and the state it describes commit together or
    // neither does, and the column's guarantee is the strong one: true whenever present.
    record_reconciled(&mut tx, profile_id, idp_key).await?;

    tx.commit().await?;
    Ok(Some(out))
}

/// A refusal the writer cannot give a reconcile write, answered as an error rather than counted.
/// The write holds the team lock and, for a change or a removal, the row's lock (`locked`), so the
/// row cannot be absent or move from what was read (`absent`, `stale`), and the last-owner guard
/// is off (R8). One of these means the invariant broke; the reconcile fails open — membership
/// unchanged, the caller logs — rather than misreport it as a native skip.
fn idp_write_refused(refusal: team_membership::NotWritten) -> ApiError {
    ApiError::Internal(format!(
        "identity-provider reconcile write refused under its locks: {refusal:?}"
    ))
}

/// Record that a reconcile was declined for want of a group signal the IdP was configured to send.
///
/// Autonomous (not in any transaction) because there is no membership work to be atomic with — the
/// point of this path is that none was done. Touches only `last_skipped_at`: any
/// `last_reconciled_at` already on the row is a true statement about a past agreement and survives,
/// which is what lets a reader see "agreed on the 1st, and every login since has carried no
/// signal".
async fn record_signal_missing(pool: &PgPool, profile_id: Uuid, idp_key: &str) -> ApiResult<()> {
    sqlx::query!(
        r#"INSERT INTO kb_saml_principal_reconcile (profile_id, idp_key, last_skipped_at)
           VALUES ($1, $2, now())
           ON CONFLICT (profile_id, idp_key) DO UPDATE SET last_skipped_at = now()"#,
        profile_id,
        idp_key,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// After a reconcile write that removed or demoted an owner, under the team lock the write holds:
/// was the team left with no owner? R8 lets the identity provider remove or demote the last owner;
/// the event carries `left_ownerless` and this count surfaces it to the operator log. Called only
/// for a write that lowered an owner, so a team that was already ownerless is not counted.
async fn count_ownerless(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    team_id: Uuid,
    out: &mut ReconcileCounts,
) -> ApiResult<()> {
    let owners: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_team_members WHERE team_id = $1 AND role = 'owner'",
        team_id,
    )
    .fetch_one(&mut **tx)
    .await?
    .unwrap_or(0);
    if owners == 0 {
        out.left_ownerless += 1;
    }
    Ok(())
}

/// Record that this principal's idp reach was brought into agreement, in the caller's transaction.
///
/// Touches only `last_reconciled_at`, for the mirror of the reason above: a `last_skipped_at`
/// already on the row records a login that really did carry no signal, and a later successful
/// reconcile does not un-happen it. The two columns are only ever written apart, which is what
/// makes `last_skipped_at > last_reconciled_at` a meaningful comparison.
async fn record_reconciled(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: Uuid,
    idp_key: &str,
) -> ApiResult<()> {
    sqlx::query!(
        r#"INSERT INTO kb_saml_principal_reconcile (profile_id, idp_key, last_reconciled_at)
           VALUES ($1, $2, now())
           ON CONFLICT (profile_id, idp_key) DO UPDATE SET last_reconciled_at = now()"#,
        profile_id,
        idp_key,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::TeamRole;

    #[test]
    fn max_role_picks_the_stronger_role() {
        assert_eq!(
            max_role(TeamRole::Member, TeamRole::Maintainer),
            TeamRole::Maintainer
        );
        assert_eq!(
            max_role(TeamRole::Owner, TeamRole::Maintainer),
            TeamRole::Owner
        );
        assert_eq!(
            max_role(TeamRole::Watcher, TeamRole::Member),
            TeamRole::Member
        );
        assert_eq!(max_role(TeamRole::Owner, TeamRole::Owner), TeamRole::Owner);
    }
}

//! SAML-driven team-membership reconciliation (Phase 2). Applies an operator-maintained
//! `(idp_key, group) -> (team, role)` mapping to `kb_team_members` rows tagged `source='idp'`,
//! leaving `source='native'` rows untouched (native-wins-skip). See the Phase 2 design spec.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::TeamRole;

use crate::error::ApiResult;

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
/// pair, that team is skipped entirely (native-wins-skip). Runs in one transaction so a
/// failure leaves membership state unchanged (fail-open at the caller).
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

    let mut out = ReconcileCounts::default();

    // 3. Add / update desired teams (skipping any team the user is native in).
    for m in desired
        .iter()
        .map(|(&team_id, &role)| DesiredMembership { team_id, role })
    {
        if native_teams.contains(&m.team_id) {
            out.skipped_native += 1;
            continue;
        }
        match idp_current.get(&m.team_id) {
            Some(&existing) if existing == m.role => {}
            Some(_) => {
                sqlx::query!(
                    "UPDATE kb_team_members SET role = $3 WHERE team_id = $1 AND profile_id = $2 AND source = 'idp'",
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
    // NOTE: filter's predicate takes `&Self::Item`, so `t` binds as `&&Uuid` under match
    // ergonomics — deref once so `contains_key` sees `&Uuid` (HashMap<Uuid, _> has no
    // `Borrow<&Uuid>` impl, so passing `t` directly would fail to type-check).
    for (&team_id, _) in idp_current
        .iter()
        .filter(|(t, _)| !desired.contains_key(*t))
    {
        sqlx::query!(
            "DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2 AND source = 'idp'",
            team_id,
            profile_id,
        )
        .execute(&mut *tx)
        .await?;
        out.revoked += 1;
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
    Ok(ReconcileOutcome::Reconciled(out))
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

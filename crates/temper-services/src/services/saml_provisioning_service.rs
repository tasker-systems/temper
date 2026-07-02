//! SAML-driven team-membership reconciliation (Phase 2). Applies an operator-maintained
//! `(idp_key, group) -> (team, role)` mapping to `kb_team_members` rows tagged `source='idp'`,
//! leaving `source='native'` rows untouched (native-wins-skip). See the Phase 2 design spec.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::TeamRole;

use crate::error::ApiResult;

/// Numeric rank for the strict hierarchy Owner > Maintainer > Member > Watcher.
/// TeamRole is not `Ord` (its derive order would rank Owner lowest), so rank explicitly.
fn role_rank(role: TeamRole) -> u8 {
    match role {
        TeamRole::Owner => 3,
        TeamRole::Maintainer => 2,
        TeamRole::Member => 1,
        TeamRole::Watcher => 0,
    }
}

/// The stronger of two roles (used when two asserted groups map to the same team).
fn max_role(a: TeamRole, b: TeamRole) -> TeamRole {
    if role_rank(a) >= role_rank(b) {
        a
    } else {
        b
    }
}

/// Counts of what a reconcile pass changed. Returned for logging/observability.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReconcileOutcome {
    pub added: usize,
    pub updated: usize,
    pub revoked: usize,
    pub skipped_native: usize,
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
pub async fn reconcile_idp_memberships(
    pool: &PgPool,
    profile_id: Uuid,
    idp_key: &str,
    groups: &[String],
) -> ApiResult<ReconcileOutcome> {
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

    let mut out = ReconcileOutcome::default();

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

    tx.commit().await?;
    Ok(out)
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

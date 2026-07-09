//! `temper cogmap shape` business logic — thin wrapper over the cognitive-maps client. Cloud-only.

use temper_core::types::cognitive_maps::{
    BindTeamOutcome, BindTeamRequest, CogmapAnalyticsRow, CogmapGrantBody, CogmapRegionMetricsRow,
    CogmapRegionRow, CogmapRevokeBody, GrantOutcome, RevokeOutcome, UnbindTeamOutcome,
};

use crate::error::{Result, TemperError};

/// A principal for a grant/revoke: exactly one of a profile or a team, both raw UUIDs.
pub struct Principal {
    pub table: String,
    pub id: uuid::Uuid,
}

/// Resolve exactly one of (profile, team) into a `(principal_table, principal_id)` pair.
pub fn resolve_principal(
    profile: Option<uuid::Uuid>,
    team: Option<uuid::Uuid>,
) -> Result<Principal> {
    match (profile, team) {
        (Some(id), None) => Ok(Principal {
            table: "kb_profiles".to_string(),
            id,
        }),
        (None, Some(id)) => Ok(Principal {
            table: "kb_teams".to_string(),
            id,
        }),
        (Some(_), Some(_)) => Err(TemperError::Api(
            "supply exactly one principal, not both a profile and a team".to_string(),
        )),
        (None, None) => Err(TemperError::Api(
            "no principal — supply exactly one of --to-profile/--to-team (or --from-*)".to_string(),
        )),
    }
}

/// Call the shape API for the given cogmap (and optional lens), both already resolved to UUIDs.
pub async fn shape_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> Result<Vec<CogmapRegionRow>> {
    client
        .cognitive_maps()
        .shape(cogmap_id, lens_id)
        .await
        .map_err(crate::commands::client_err)
}

/// Call the region-metrics API for the given cogmap (and optional lens), both resolved to UUIDs.
pub async fn region_metrics_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> Result<Vec<CogmapRegionMetricsRow>> {
    client
        .cognitive_maps()
        .region_metrics(cogmap_id, lens_id)
        .await
        .map_err(crate::commands::client_err)
}

/// Call the analytics API for the given cogmap (resolved to a UUID).
pub async fn analytics_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
) -> Result<CogmapAnalyticsRow> {
    client
        .cognitive_maps()
        .analytics(cogmap_id)
        .await
        .map_err(crate::commands::client_err)
}

/// Call the materialize API for the given cogmap (resolved to a UUID) — recompute regions when
/// the event delta clears the threshold. A no-op below threshold (`materialized: false`), not
/// an error.
pub async fn materialize_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    threshold: Option<i64>,
) -> Result<temper_core::types::materialize::MaterializeAck> {
    client
        .cognitive_maps()
        .materialize(cogmap_id, threshold)
        .await
        .map_err(crate::commands::client_err)
}

/// Strip an optional leading `+` sigil from a team ref, yielding the bare team token.
///
/// Teams are addressed by their global-unique slug (`team_service` strips the same `+`
/// on the server). Unlike a context ref (`+team/slug`), a team has no `/slug` half — so
/// `parse_context_ref` is the wrong tool here; we strip the sigil directly.
fn strip_team_sigil(team: &str) -> &str {
    team.strip_prefix('+').unwrap_or(team)
}

/// Resolve a team ref (a slug, optionally `+`-prefixed, or a bare UUID) to its team id.
///
/// A UUID is used directly. Otherwise the slug is matched against the teams the caller is a
/// member of (`TeamsClient::list`) — the admin who provisions a map is a member (owner) of
/// the teams they bind it to. Returns a clear error when the slug does not resolve.
pub async fn resolve_team_id(
    client: &temper_client::TemperClient,
    team: &str,
) -> Result<uuid::Uuid> {
    let token = strip_team_sigil(team);
    if let Ok(id) = uuid::Uuid::parse_str(token) {
        return Ok(id);
    }
    let teams = client
        .teams()
        .list()
        .await
        .map_err(crate::commands::client_err)?;
    teams
        .into_iter()
        .find(|t| t.slug == token)
        .map(|t| t.id)
        .ok_or_else(|| {
            TemperError::Api(format!(
                "team '{token}' not found among the teams you are a member of"
            ))
        })
}

/// Bind the cogmap (already resolved to a UUID) to a team (resolved from `team`).
pub async fn bind_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    team: &str,
) -> Result<BindTeamOutcome> {
    let team_id = resolve_team_id(client, team).await?;
    client
        .cognitive_maps()
        .bind_team(cogmap_id, &BindTeamRequest { team_id })
        .await
        .map_err(crate::commands::client_err)
}

/// Unbind the cogmap (already resolved to a UUID) from a team (resolved from `team`).
pub async fn unbind_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    team: &str,
) -> Result<UnbindTeamOutcome> {
    let team_id = resolve_team_id(client, team).await?;
    client
        .cognitive_maps()
        .unbind_team(cogmap_id, team_id)
        .await
        .map_err(crate::commands::client_err)
}

/// Grant a capability on the cogmap. `read` is forced on when `write`/`grant` is set (coherence).
pub async fn grant_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    principal: &Principal,
    read: bool,
    write: bool,
    grant: bool,
) -> Result<GrantOutcome> {
    let body = CogmapGrantBody {
        principal_table: principal.table.clone(),
        principal_id: principal.id,
        can_read: read || write || grant,
        can_write: write,
        can_delete: false,
        can_grant: grant,
    };
    client
        .cognitive_maps()
        .grant(cogmap_id, &body)
        .await
        .map_err(crate::commands::client_err)
}

/// Revoke a capability grant on the cogmap.
pub async fn revoke_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    principal: &Principal,
) -> Result<RevokeOutcome> {
    let body = CogmapRevokeBody {
        principal_table: principal.table.clone(),
        principal_id: principal.id,
    };
    client
        .cognitive_maps()
        .revoke(cogmap_id, &body)
        .await
        .map_err(crate::commands::client_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn strip_team_sigil_handles_prefix_and_bare() {
        assert_eq!(strip_team_sigil("+my-team"), "my-team");
        assert_eq!(strip_team_sigil("my-team"), "my-team");
    }

    #[test]
    fn render_region_metrics_rows_json_is_passthrough_array() {
        use temper_core::types::cognitive_maps::CogmapRegionMetricsRow;
        let rows: Vec<CogmapRegionMetricsRow> = vec![CogmapRegionMetricsRow {
            region_id: Uuid::from_u128(1).into(),
            lens_id: Uuid::from_u128(2).into(),
            centrality: Some(4.0),
            content_cohesion: None,
            internal_tension: Some(1.5),
            reference_standing: Some(7.0),
            telos_alignment: None,
        }];
        let out =
            crate::format::render(&rows, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.starts_with('['), "json should be an array: {out}");
        assert!(out.contains("\"internal_tension\""), "json: {out}");
    }

    #[test]
    fn render_shape_rows_json_is_passthrough_array() {
        let rows: Vec<CogmapRegionRow> = vec![CogmapRegionRow {
            region_id: Uuid::from_u128(1).into(),
            lens_id: Uuid::from_u128(2).into(),
            salience: 0.5,
            content_cohesion: None,
            label: Some("region".to_string()),
            member_count: 2,
        }];
        let out =
            crate::format::render(&rows, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.starts_with('['), "json should be an array: {out}");
        assert!(out.contains("\"region_id\""), "json: {out}");
        assert!(out.contains("\"member_count\""), "json: {out}");
    }
}

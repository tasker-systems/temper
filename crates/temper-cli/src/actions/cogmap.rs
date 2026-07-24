//! `temper cogmap shape` business logic — thin wrapper over the cognitive-maps client. Cloud-only.

use temper_core::types::cognitive_maps::{
    BindTeamOutcome, BindTeamRequest, CogmapAnalyticsRow, CogmapDetail, CogmapGrantBody,
    CogmapRegionMetricsRow, CogmapRegionRow, CogmapRevokeBody, CogmapRow, GrantOutcome,
    RevokeOutcome, UnbindTeamOutcome,
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

/// Fetch the caller's visible cognitive maps (identity + charter statement). Self-scoped
/// server-side — an empty vec means the caller can see no maps, never an error.
pub async fn list_api(client: &temper_client::TemperClient) -> Result<Vec<CogmapRow>> {
    client
        .cognitive_maps()
        .list()
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
}

/// Fetch one map's full orientation — identity + charter + foundational resources. 404 (NotFound)
/// when the map is not readable by the caller.
pub async fn show_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
) -> Result<CogmapDetail> {
    client
        .cognitive_maps()
        .show(cogmap_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
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
        .map_err(crate::actions::runtime::client_err_to_temper)
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
        .map_err(crate::actions::runtime::client_err_to_temper)
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
        .map_err(crate::actions::runtime::client_err_to_temper)
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
        .map_err(crate::actions::runtime::client_err_to_temper)
}

/// Strip an optional leading `+` sigil from a team ref, yielding the bare team token.
///
/// Teams are addressed by their global-unique slug (`team_service` strips the same `+`
/// on the server). Unlike a context ref (`+team/slug`), a team has no `/slug` half — so
/// `parse_context_ref` is the wrong tool here; we strip the sigil directly.
fn strip_team_sigil(team: &str) -> &str {
    team.strip_prefix('+').unwrap_or(team)
}

/// Resolve a team ref to its team id — the single team resolver shared by every team-typed
/// CLI argument (`team show`, `team invite`, `context share`, `cogmap bind`, `resource grant
/// --to-team`, …). Accepts, in order:
///
/// 1. a team UUID, or the decorated `slug-<uuid>` form (trailing UUID extracted, slug half
///    ignored — the same decorated addressing resources use). `parse_ref` covers both.
/// 2. a bare team slug (unique across the instance), matched against the teams the caller is
///    a member of (`TeamsClient::list`).
///
/// The error names the *team* argument and its accepted forms — never "resource ref" (issue
/// #366): `--to-team`/`--from-team` are team-typed, so a bad value is a team error, not a
/// resource-ref error.
pub async fn resolve_team_id(
    client: &temper_client::TemperClient,
    team: &str,
) -> Result<uuid::Uuid> {
    let token = strip_team_sigil(team);
    // A bare UUID or a decorated `slug-<uuid>` ref both carry the id directly. On a bare slug
    // `parse_ref` errors (its message is resource-oriented) — we swallow it and fall through to
    // the slug lookup, so the only error a caller ever sees is the team-shaped one below.
    if let Ok(id) = temper_workflow::operations::parse_ref(token) {
        return Ok(uuid::Uuid::from(id));
    }
    let teams = client
        .teams()
        .list()
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    teams
        .into_iter()
        .find(|t| t.slug == token)
        .map(|t| t.id)
        .ok_or_else(|| {
            TemperError::Api(format!(
                "not a team: {token:?} (expected a team slug, a decorated `slug-<uuid>` ref, \
                 or a team UUID; {token:?} matched no team you are a member of)"
            ))
        })
}

/// Map a `cogmap bind`/`unbind` client error, enriching the message-less 403 with the
/// actual two-sided requirement + escalation path — symmetric with `context share` (issue
/// #367). "instance administrator" is spelled out so it is never read as the per-team
/// `admin`/`owner` role.
fn map_bind_err(action: &str, e: temper_client::error::ClientError) -> TemperError {
    match e {
        temper_client::error::ClientError::Forbidden => TemperError::Api(format!(
            "not authorized: `cogmap {action}` requires that you administer the map \
             (hold a grant on it) AND manage the target team (owner/maintainer) — or that \
             you are an instance administrator."
        )),
        other => crate::actions::runtime::client_err_to_temper(other),
    }
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
        .map_err(|e| map_bind_err("bind", e))
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
        .map_err(|e| map_bind_err("unbind", e))
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
        .map_err(crate::actions::runtime::client_err_to_temper)
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
        .map_err(crate::actions::runtime::client_err_to_temper)
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

    fn sample_cogmap_row(name: &str, statement: Option<&str>) -> CogmapRow {
        CogmapRow {
            id: Uuid::now_v7(),
            name: name.to_string(),
            owner_ref: "+platform".to_string(),
            team_ids: vec![],
            region_count: 3,
            resource_count: 12,
            telos_resource_id: Uuid::now_v7(),
            charter_statement: statement.map(str::to_owned),
        }
    }

    #[test]
    fn cogmap_list_row_injects_ref_that_resolves_to_the_id() {
        let row = sample_cogmap_row(
            "My Architecture Map",
            Some("What holds the platform together."),
        );
        let id = row.id;
        let mut value = serde_json::to_value(&row).expect("serialize");
        crate::commands::resource::inject_cogmap_ref(&mut value);
        let got = value
            .get("ref")
            .and_then(|v| v.as_str())
            .expect("ref injected");
        // The contract is trailing-UUID resolution, not an exact slug — assert the ref round-trips.
        let parsed = temper_workflow::operations::parse_ref(got).expect("ref parses");
        assert_eq!(
            Uuid::from(parsed),
            id,
            "decorated ref must resolve to the map id"
        );
    }

    #[test]
    fn cogmap_row_omits_null_charter_statement() {
        let row = sample_cogmap_row("Empty", None);
        let out = crate::format::render(&row, crate::format::OutputFormat::Json).expect("json");
        assert!(
            !out.contains("charter_statement"),
            "a null statement is omitted from JSON: {out}"
        );
    }

    #[test]
    fn cogmap_row_renders_charter_statement_when_present() {
        let row = sample_cogmap_row("Architecture", Some("The load-bearing shape."));
        let out = crate::format::render(&row, crate::format::OutputFormat::Json).expect("json");
        assert!(
            out.contains("charter_statement"),
            "statement present: {out}"
        );
        assert!(
            out.contains("The load-bearing shape."),
            "statement body: {out}"
        );
    }

    #[test]
    fn cogmap_foundation_injects_resource_ref_and_keeps_telos_flag() {
        use temper_core::types::cognitive_maps::CogmapFoundationRow;
        let rid = Uuid::now_v7();
        let row = CogmapFoundationRow {
            resource_id: rid,
            title: "Charter".to_string(),
            doc_type: "cogmap_charter".to_string(),
            is_telos: true,
        };
        let mut v = serde_json::to_value(&row).expect("serialize");
        // `show` injects a resource ref on each foundation via inject_ref (resource_id + title).
        crate::commands::resource::inject_ref(&mut v);
        let got = v.get("ref").and_then(|x| x.as_str()).expect("ref injected");
        let parsed = temper_workflow::operations::parse_ref(got).expect("ref parses");
        assert_eq!(
            Uuid::from(parsed),
            rid,
            "foundation ref resolves to resource id"
        );
        assert_eq!(v.get("is_telos").and_then(|x| x.as_bool()), Some(true));
    }

    #[test]
    fn cogmap_detail_renders_identity_charter_and_foundations() {
        use temper_core::types::cognitive_maps::{CharterBlock, CogmapDetail, CogmapFoundationRow};
        let detail = CogmapDetail {
            cogmap: sample_cogmap_row("Arch", Some("purpose")),
            charter: vec![CharterBlock {
                seq: 0,
                role: "statement".to_string(),
                body: "why the map exists".to_string(),
            }],
            foundations: vec![CogmapFoundationRow {
                resource_id: Uuid::now_v7(),
                title: "Telos".to_string(),
                doc_type: "cogmap_charter".to_string(),
                is_telos: true,
            }],
        };
        let out = crate::format::render(&detail, crate::format::OutputFormat::Json).expect("json");
        for key in ["cogmap", "charter", "foundations", "is_telos"] {
            assert!(out.contains(key), "detail JSON missing `{key}`: {out}");
        }
    }
}

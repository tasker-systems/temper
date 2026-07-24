//! `temper cogmap reconcile` — operator command to reconcile a cognitive map's content to a
//! committed manifest.
//!
//! Reads the authored manifest, embeds each entry CLIENT-SIDE (`compute_body_chunks`), builds a
//! pre-embedded `ReconcileCogmapRequest`, and PUTs it to `/api/cognitive-maps/{id}` (admin-gated,
//! idempotent). Prints the run outcome (`created`/`updated`/`folded`/`unchanged`/`charter`).
//!
//! The whole path needs the `embed` feature (it runs ONNX). A non-embed build still compiles and
//! returns a clear "requires --features embed" error, mirroring `actions::search::embed_query`.
//!
//! Also provides `temper cogmap shape <ref>` — read a map's materialized regions (surface tier).

use crate::error::Result;
use crate::format::OutputFormat;

/// `temper cogmap list [--name-contains <s>] [--team <ref>]` — list the maps you can see, each with
/// its decorated ref, held-by scope, region/resource counts, and charter statement.
///
/// The listing itself is self-scoped server-side (`cogmap_visible_maps`); `--name-contains` /
/// `--team` are client-side narrowings over the returned set (the set is small — tens of maps). Each
/// row is decorated with a `ref` (`sluggify(name)-<uuid>`) the way `context list` rows are.
pub fn list(name_contains: Option<&str>, team: Option<&str>, fmt: OutputFormat) -> Result<()> {
    let name_needle = name_contains.map(str::to_lowercase);
    let team = team.map(str::to_owned);

    let rows = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            // Resolve `--team` to a team id first (needs the client), then narrow. A team ref that
            // matches no team you belong to is a team error, surfaced here rather than silently
            // yielding an empty list.
            let team_id = match team.as_deref() {
                Some(t) => Some(crate::actions::cogmap::resolve_team_id(client, t).await?),
                None => None,
            };
            let mut rows = crate::actions::cogmap::list_api(client).await?;
            if let Some(needle) = name_needle {
                rows.retain(|r| r.name.to_lowercase().contains(&needle));
            }
            if let Some(tid) = team_id {
                rows.retain(|r| r.team_ids.contains(&tid));
            }
            Ok(rows)
        })
    })?;

    // Identity-out: decorate each row with its `ref`, as list/show/search rows carry one.
    let mut value = serde_json::to_value(&rows)
        .map_err(|e| crate::error::TemperError::Api(format!("cogmap list serialize: {e}")))?;
    if let Some(arr) = value.as_array_mut() {
        for row in arr.iter_mut() {
            crate::commands::resource::inject_cogmap_ref(row);
        }
    }
    let rendered = crate::format::render(&value, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap show <cogmap_ref>` — one map's orientation: identity, charter blocks, and the
/// foundational (homed) resources it is built on, telos flagged. 404 when the map is not readable.
///
/// The identity carries a decorated cogmap `ref`; each foundation carries its own resource `ref`
/// (and `context_ref` where resolvable), the way `resource show`/`list` rows do.
pub fn show(cogmap_ref: &str, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let detail = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::cogmap::show_api(client, cogmap_id).await })
    })?;

    let mut value = serde_json::to_value(&detail)
        .map_err(|e| crate::error::TemperError::Api(format!("cogmap show serialize: {e}")))?;
    if let Some(cog) = value.get_mut("cogmap") {
        crate::commands::resource::inject_cogmap_ref(cog);
    }
    if let Some(founds) = value.get_mut("foundations").and_then(|v| v.as_array_mut()) {
        // `inject_ref` reads `resource_id` + `title`, so each foundation gets a resource ref.
        for f in founds.iter_mut() {
            crate::commands::resource::inject_ref(f);
        }
    }
    let rendered = crate::format::render(&value, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap shape <cogmap_ref> [--lens <ref>]` — read the map's materialized regions.
pub fn shape(cogmap_ref: &str, lens_ref: Option<&str>, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;
    let lens_id = lens_ref
        .map(|l| temper_workflow::operations::parse_ref(l).map(|p| p.0))
        .transpose()?;

    let rows = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::cogmap::shape_api(client, cogmap_id, lens_id).await })
    })?;

    let rendered = crate::format::render(&rows, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap region-metrics <cogmap_ref> [--lens <ref>]` — read the per-region analytics metrics.
pub fn region_metrics(cogmap_ref: &str, lens_ref: Option<&str>, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;
    let lens_id = lens_ref
        .map(|l| temper_workflow::operations::parse_ref(l).map(|p| p.0))
        .transpose()?;

    let rows = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            crate::actions::cogmap::region_metrics_api(client, cogmap_id, lens_id).await
        })
    })?;

    let rendered = crate::format::render(&rows, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap analytics <cogmap_ref>` — read the map-level analytics (telos, staleness, regulation).
pub fn analytics(cogmap_ref: &str, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::cogmap::analytics_api(client, cogmap_id).await })
    })?;

    let rendered = crate::format::render(&row, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap materialize <cogmap_ref> [--threshold N]` — recompute the map's regions.
///
/// The write-side counterpart to `shape`/`region-metrics`/`analytics`: regions only exist
/// after a materialize, so an authoring pass that creates nodes and asserts edges must
/// materialize before the read tier reflects them.
pub fn materialize(cogmap_ref: &str, threshold: Option<i64>, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let ack = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            crate::actions::cogmap::materialize_api(client, cogmap_id, threshold).await
        })
    })?;

    let rendered = crate::format::render(&ack, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap bind <cogmap_ref> <team>` — bind the map to a team (system-admin, or a team
/// manager who administers the map).
///
/// The team is resolved to its id CLIENT-SIDE (slug → id via the teams list, or a bare UUID); the
/// wire shape is id-based (mirroring `team add-member`).
pub fn bind(cogmap_ref: &str, team: &str, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;
    let team = team.to_owned();

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::cogmap::bind_api(client, cogmap_id, &team).await })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap unbind <cogmap_ref> <team>` — unbind the map from a team (same authority
/// as `bind`: system-admin, or a team manager who administers the map).
pub fn unbind(cogmap_ref: &str, team: &str, fmt: OutputFormat) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;
    let team = team.to_owned();

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move { crate::actions::cogmap::unbind_api(client, cogmap_id, &team).await })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap grant <cogmap_ref> --to-profile|--to-team <ref> [--read] [--write] [--grant]`.
#[allow(clippy::too_many_arguments)]
pub fn grant(
    cogmap_ref: &str,
    to_profile: Option<uuid::Uuid>,
    to_team: Option<String>,
    read: bool,
    write: bool,
    grant_cap: bool,
    fmt: OutputFormat,
) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            // `--to-team` is a team ref (slug / decorated / UUID), resolved like the rest of the
            // team surface — not the resource-ref parser (issue #366).
            let to_team_id = match to_team.as_deref() {
                Some(team) => Some(crate::actions::cogmap::resolve_team_id(client, team).await?),
                None => None,
            };
            let principal = crate::actions::cogmap::resolve_principal(to_profile, to_team_id)?;
            crate::actions::cogmap::grant_api(client, cogmap_id, &principal, read, write, grant_cap)
                .await
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper cogmap revoke <cogmap_ref> --from-profile|--from-team <ref>`.
pub fn revoke(
    cogmap_ref: &str,
    from_profile: Option<uuid::Uuid>,
    from_team: Option<String>,
    fmt: OutputFormat,
) -> Result<()> {
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let from_team_id = match from_team.as_deref() {
                Some(team) => Some(crate::actions::cogmap::resolve_team_id(client, team).await?),
                None => None,
            };
            let principal = crate::actions::cogmap::resolve_principal(from_profile, from_team_id)?;
            crate::actions::cogmap::revoke_api(client, cogmap_id, &principal).await
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// Reconcile the cognitive map addressed by `cogmap_ref` to the manifest at `manifest_path`.
#[cfg(feature = "embed")]
pub fn reconcile(
    cogmap_ref: &str,
    manifest_path: &str,
    act: temper_core::types::ActInput,
    fmt: OutputFormat,
) -> Result<()> {
    use crate::actions::reconcile as reconcile_action;
    use crate::error::TemperError;

    // Resolve the cogmap ref → UUID (trailing-UUID-only; the slug half is ignored).
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let yaml = std::fs::read_to_string(manifest_path)
        .map_err(|e| TemperError::Config(format!("reading manifest {manifest_path}: {e}")))?;
    let doc = reconcile_action::parse_manifest(&yaml)?;
    let req = reconcile_action::manifest_to_request(&doc)?;

    // The manifest body stays pure; authorship rides query params (discrete ActInput shape).
    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .cognitive_maps()
                .reconcile_cognitive_map(cogmap_id, &req, &act)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// Non-embed build: the command cannot run (no ONNX to embed the manifest).
#[cfg(not(feature = "embed"))]
pub fn reconcile(
    _cogmap_ref: &str,
    _manifest_path: &str,
    _act: temper_core::types::ActInput,
    _fmt: OutputFormat,
) -> Result<()> {
    Err(crate::error::TemperError::Config(
        "cogmap reconcile requires the 'embed' feature — rebuild with --features embed".into(),
    ))
}

/// Genesis (create) a new cognitive map from the manifest at `manifest_path`. `--id` / `--name`
/// override the manifest's `cogmap_id` / `name`. A supplied id is honored only for a system-admin;
/// a non-admin always receives a server-minted id (reserved-id hardening).
#[cfg(feature = "embed")]
pub fn create(
    manifest_path: &str,
    name: Option<&str>,
    id: Option<&str>,
    fmt: OutputFormat,
) -> Result<()> {
    use crate::actions::genesis as genesis_action;
    use crate::error::TemperError;

    // Resolve the optional --id flag (trailing-UUID-only; the slug half is ignored), if supplied.
    let id_override = id
        .map(|r| temper_workflow::operations::parse_ref(r).map(|p| p.0))
        .transpose()?;

    let yaml = std::fs::read_to_string(manifest_path)
        .map_err(|e| TemperError::Config(format!("reading manifest {manifest_path}: {e}")))?;
    let doc = genesis_action::parse_manifest(&yaml)?;
    let req = genesis_action::manifest_to_request(&doc, id_override, name)?;

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .cognitive_maps()
                .create_cognitive_map(&req)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// Non-embed build: the command cannot run (no ONNX to embed the charter manifest).
#[cfg(not(feature = "embed"))]
pub fn create(
    _manifest_path: &str,
    _name: Option<&str>,
    _id: Option<&str>,
    _fmt: OutputFormat,
) -> Result<()> {
    Err(crate::error::TemperError::Config(
        "cogmap create requires the 'embed' feature — rebuild with --features embed".into(),
    ))
}

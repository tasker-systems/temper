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

/// `temper cogmap bind <cogmap_ref> <team>` — bind the map to a team (admin-only).
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

/// `temper cogmap unbind <cogmap_ref> <team>` — unbind the map from a team (admin-only).
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

/// Reconcile the cognitive map addressed by `cogmap_ref` to the manifest at `manifest_path`.
#[cfg(feature = "embed")]
pub fn reconcile(cogmap_ref: &str, manifest_path: &str, fmt: OutputFormat) -> Result<()> {
    use crate::actions::reconcile as reconcile_action;
    use crate::error::TemperError;

    // Resolve the cogmap ref → UUID (trailing-UUID-only; the slug half is ignored).
    let cogmap_id = temper_workflow::operations::parse_ref(cogmap_ref)?.0;

    let yaml = std::fs::read_to_string(manifest_path)
        .map_err(|e| TemperError::Config(format!("reading manifest {manifest_path}: {e}")))?;
    let doc = reconcile_action::parse_manifest(&yaml)?;
    let req = reconcile_action::manifest_to_request(&doc)?;

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .cognitive_maps()
                .reconcile_cognitive_map(cogmap_id, &req)
                .await
                .map_err(crate::commands::client_err)
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// Non-embed build: the command cannot run (no ONNX to embed the manifest).
#[cfg(not(feature = "embed"))]
pub fn reconcile(_cogmap_ref: &str, _manifest_path: &str, _fmt: OutputFormat) -> Result<()> {
    Err(crate::error::TemperError::Config(
        "cogmap reconcile requires the 'embed' feature — rebuild with --features embed".into(),
    ))
}

/// Genesis (create) a new cognitive map from the manifest at `manifest_path`. `--id` / `--name`
/// override the manifest's `cogmap_id` / `name`; absent ids are minted client-side (stable, reproducible).
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
                .map_err(crate::commands::client_err)
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

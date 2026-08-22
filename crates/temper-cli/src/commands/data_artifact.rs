//! `temper data-artifact list/show/commit` — CLI surface for data artifact reads and writes.
//!
//! The `schema` subgroup (`schema list/show/declare`) reaches the shape registry — the
//! enforcement surface that governs artifact families within a context home.

use crate::actions::body_source;
use crate::actions::runtime;
use crate::format::OutputFormat;
use crate::output;
use temper_core::types::data_artifact::{ArtifactCommitRequest, ArtifactListParams};
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest, ShapeView};
use temper_core::types::ids::DataArtifactId;
use temper_workflow::operations::parse_ref;

pub fn list(_config: &crate::config::Config, params: ListParams<'_>) -> crate::error::Result<()> {
    let id = parse_ref(params.r#ref)?;

    let api_params = ArtifactListParams {
        kind: params.kind.map(|s| s.to_string()),
        intent: params.intent.map(|s| s.to_string()),
        include_folded: params.include_folded.then_some(true),
        counts: params.counts.then_some(true),
    };

    let response = runtime::with_client(move |client| {
        Box::pin(async move {
            client
                .data_artifacts()
                .list(uuid::Uuid::from(id), &api_params)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&response, params.format)?;
    output::plain(rendered);
    Ok(())
}

pub fn show(_config: &crate::config::Config, params: ShowParams<'_>) -> crate::error::Result<()> {
    let resource_id = parse_ref(params.r#ref)?;
    let artifact_id = parse_ref(params.artifact_id)?;

    let artifact = runtime::with_client(move |client| {
        Box::pin(async move {
            client
                .data_artifacts()
                .get(uuid::Uuid::from(resource_id), uuid::Uuid::from(artifact_id))
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&artifact, params.format)?;
    output::plain(rendered);
    Ok(())
}

pub struct ListParams<'a> {
    pub r#ref: &'a str,
    pub kind: Option<&'a str>,
    pub intent: Option<&'a str>,
    pub include_folded: bool,
    pub counts: bool,
    pub format: OutputFormat,
}

pub struct ShowParams<'a> {
    pub r#ref: &'a str,
    pub artifact_id: &'a str,
    pub format: OutputFormat,
}

pub struct CommitParams<'a> {
    pub r#ref: &'a str,
    pub kind: &'a str,
    pub intent: &'a str,
    pub precedence: f64,
    pub content_flag: Option<&'a str>,
    pub supersedes: &'a [String],
    pub act: temper_core::types::ActInput,
    pub format: OutputFormat,
}

pub fn commit(
    _config: &crate::config::Config,
    params: CommitParams<'_>,
) -> crate::error::Result<()> {
    use std::io::IsTerminal;

    let resource_id = parse_ref(params.r#ref)?;

    let stdin_is_tty = std::io::stdin().is_terminal();
    let body_opt = body_source::resolve_body_source(
        params.content_flag,
        stdin_is_tty,
        std::io::stdin(),
        body_source::stdin_has_input_within,
    )?;

    let content_str = body_opt.ok_or_else(|| {
        crate::error::TemperError::Project(
            "data-artifact commit requires content (--content @<path>, --content -, or piped stdin)"
                .to_string(),
        )
    })?;
    let content: serde_json::Value = serde_json::from_str(&content_str).map_err(|e| {
        crate::error::TemperError::Project(format!("content is not valid JSON: {e}"))
    })?;

    let supersedes: Vec<DataArtifactId> = params
        .supersedes
        .iter()
        .map(|s| parse_ref(s).map(|id| DataArtifactId::from(uuid::Uuid::from(id))))
        .collect::<Result<Vec<_>, _>>()?;

    let request = ArtifactCommitRequest {
        kind: params.kind.to_string(),
        kind_owner: None,
        intent: params.intent.to_string(),
        precedence: params.precedence,
        content,
        supersedes,
        act: params.act,
    };

    let response = runtime::with_client(move |client| {
        Box::pin(async move {
            client
                .data_artifacts()
                .commit(uuid::Uuid::from(resource_id), &request)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&response, params.format)?;
    output::plain(rendered);
    Ok(())
}

// ---------------------------------------------------------------------------
// schema subgroup — shape registry reads and writes
// ---------------------------------------------------------------------------

/// `temper data-artifact schema list --context <ref>` — list live shapes for a context home.
pub async fn schema_list_remote(
    client: &temper_client::TemperClient,
    context: &str,
    fmt: OutputFormat,
) -> crate::error::Result<()> {
    let context_id =
        crate::commands::context_cmd::resolve_context_id_for_read(client, context).await?;
    let shapes: Vec<ShapeView> = client
        .data_artifacts()
        .list_shapes(context_id)
        .await
        .map_err(runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&shapes, fmt)?;
    output::plain(rendered);
    Ok(())
}

/// `temper data-artifact schema show <ref>` — show a single shape by ID.
pub async fn schema_show_remote(
    client: &temper_client::TemperClient,
    shape_ref: &str,
    fmt: OutputFormat,
) -> crate::error::Result<()> {
    let shape_id = parse_ref(shape_ref)?;
    let shape: ShapeView = client
        .data_artifacts()
        .get_shape(uuid::Uuid::from(shape_id))
        .await
        .map_err(runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&shape, fmt)?;
    output::plain(rendered);
    Ok(())
}

/// `temper data-artifact schema declare <ref> --kind <k>` — declare a shape for a context home.
pub async fn schema_declare_remote(
    client: &temper_client::TemperClient,
    context: &str,
    kind: &str,
    enforcement: EnforcementMode,
    content_flag: Option<&str>,
    act: temper_core::types::ActInput,
    fmt: OutputFormat,
) -> crate::error::Result<()> {
    use std::io::IsTerminal;

    let context_id =
        crate::commands::context_cmd::resolve_context_id_for_read(client, context).await?;

    let stdin_is_tty = std::io::stdin().is_terminal();
    let body_opt = body_source::resolve_body_source(
        content_flag,
        stdin_is_tty,
        std::io::stdin(),
        body_source::stdin_has_input_within,
    )?;
    let content_str = body_opt.ok_or_else(|| {
        crate::error::TemperError::Project(
            "data-artifact schema declare requires content (--content @<path>, --content -, or piped stdin)"
                .to_string(),
        )
    })?;
    let schema: serde_json::Value = serde_json::from_str(&content_str).map_err(|e| {
        crate::error::TemperError::Project(format!("content is not valid JSON: {e}"))
    })?;

    let request = ShapeDeclareRequest {
        kind: kind.to_string(),
        kind_owner: None,
        schema,
        enforcement,
        act,
    };

    let shape: ShapeView = client
        .data_artifacts()
        .declare_shape(context_id, &request)
        .await
        .map_err(runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&shape, fmt)?;
    output::plain(rendered);
    Ok(())
}

//! `temper data-artifact list/show` — CLI surface for data artifact reads.

use crate::actions::runtime;
use crate::format::OutputFormat;
use crate::output;
use temper_core::types::data_artifact::ArtifactListParams;
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

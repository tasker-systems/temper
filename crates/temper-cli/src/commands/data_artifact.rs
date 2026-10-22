//! `temper data-artifact list/show/commit` — CLI surface for data artifact reads and writes.
//!
//! The `schema` subgroup (`schema list/show/declare`) reaches the shape registry — the
//! enforcement surface that governs artifact families within a context home.

use crate::actions::body_source;
use crate::actions::runtime;
use crate::format::OutputFormat;
use crate::output;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactListParams, ArtifactPageRequest, KindOwnerInput,
};
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest, ShapeView};
use temper_core::types::ids::DataArtifactId;
use temper_core::types::resource::SortOrder;
use temper_workflow::operations::parse_ref;

/// What one `data-artifact list` call asks the server for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListAsk {
    /// Per-family counts: one row per family, not paged.
    Counts,
    /// No `--limit`: every artifact, as a bare array. The unbounded default, which retires later.
    Every,
    /// `--limit`: one bounded page, answered as the `{artifacts, next}` envelope.
    Page(ArtifactPageRequest),
}

/// The filters and the [`ListAsk`] these flags ask for.
///
/// `--limit` is the demarcation, as on the route: the paging flags are checked by
/// [`ArtifactListParams::page_request`], so this command and the server refuse the same
/// combinations in the same words (`--after-created` without `--after-id`, either without
/// `--limit`, `--order` without `--limit`, a `--limit` of 0 or above 200). `--counts` with a
/// paging flag never reaches here: clap refuses it.
fn list_ask(params: &ListParams<'_>) -> crate::error::Result<(ArtifactListParams, ListAsk)> {
    let api_params = ArtifactListParams {
        kind: params.kind.map(str::to_string),
        intent: params.intent.map(str::to_string),
        include_folded: params.include_folded.then_some(true),
        counts: params.counts.then_some(true),
        limit: params.limit,
        after_created: params.after_created,
        after_id: params.after_id,
        order: params.order,
    };
    if params.counts {
        return Ok((api_params, ListAsk::Counts));
    }
    let ask = match api_params
        .page_request()
        .map_err(|refusal| crate::error::TemperError::BadRequest(refusal.to_string()))?
    {
        Some(request) => ListAsk::Page(request),
        None => ListAsk::Every,
    };
    Ok((api_params, ask))
}

pub fn list(_config: &crate::config::Config, params: ListParams<'_>) -> crate::error::Result<()> {
    let id = uuid::Uuid::from(parse_ref(params.r#ref)?);
    let (api_params, ask) = list_ask(&params)?;

    let rendered = match ask {
        ListAsk::Counts => {
            let response = runtime::with_client(move |client| {
                Box::pin(async move {
                    client
                        .data_artifacts()
                        .list(id, &api_params)
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)
                })
            })?;
            crate::format::render(&response, params.format)?
        }
        // No `--limit`: drain the bounded list, so no single response is unbounded, and answer
        // the bare array of every artifact, as this command always has.
        ListAsk::Every => {
            let artifacts = runtime::with_client(move |client| {
                Box::pin(async move {
                    client
                        .data_artifacts()
                        .list_all(id, &api_params)
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)
                })
            })?;
            crate::format::render(&artifacts, params.format)?
        }
        // `--limit`: one page, and the envelope carries where to continue (`next`, `null` on
        // the last page), since stdout has no header to put it in.
        ListAsk::Page(request) => {
            let page = runtime::with_client(move |client| {
                Box::pin(async move {
                    client
                        .data_artifacts()
                        .list_page(id, &api_params, request)
                        .await
                        .map_err(crate::actions::runtime::client_err_to_temper)
                })
            })?;
            crate::format::render(&page, params.format)?
        }
    };
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
    /// One bounded page of at most this many; absent asks for every artifact.
    pub limit: Option<u32>,
    pub order: Option<SortOrder>,
    pub after_created: Option<chrono::DateTime<chrono::Utc>>,
    pub after_id: Option<uuid::Uuid>,
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
    pub kind_owner: Option<&'a str>,
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
        kind_owner: params.kind_owner.map(parse_kind_owner).transpose()?,
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

/// `temper data-artifact get <artifact_ref>` — the flat artifact read
/// (`GET /api/data-artifacts/{id}`): no owning-resource address needed.
pub fn get_by_id(artifact_ref: &str, fmt: OutputFormat) -> crate::error::Result<()> {
    let artifact_id = parse_ref(artifact_ref)?;

    let artifact = runtime::with_client(move |client| {
        Box::pin(async move {
            client
                .data_artifacts()
                .get_by_id(uuid::Uuid::from(artifact_id))
                .await
                .map_err(runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&artifact, fmt)?;
    output::plain(rendered);
    Ok(())
}

// ---------------------------------------------------------------------------
// schema subgroup — shape registry reads and writes
// ---------------------------------------------------------------------------

/// `temper data-artifact schema list --context <ref> | --cogmap <ref>` — list live shapes
/// for a home anchor (context or cognitive map; exactly one).
pub async fn schema_list_remote(
    client: &temper_client::TemperClient,
    context: Option<&str>,
    cogmap: Option<&str>,
    fmt: OutputFormat,
) -> crate::error::Result<()> {
    if context.is_some() && cogmap.is_some() {
        return Err(crate::error::TemperError::Project(
            "schema list takes exactly one home: --context <ref> or --cogmap <ref>, not both"
                .to_string(),
        ));
    }
    let shapes: Vec<ShapeView> = if let Some(cogmap_ref) = cogmap {
        let cogmap_id = parse_ref(cogmap_ref)?;
        client
            .data_artifacts()
            .list_cogmap_shapes(uuid::Uuid::from(cogmap_id))
            .await
            .map_err(runtime::client_err_to_temper)?
    } else {
        let context = context.ok_or_else(|| {
            crate::error::TemperError::Project(
                "schema list needs exactly one home: --context <ref> or --cogmap <ref>".to_string(),
            )
        })?;
        let context_id =
            crate::commands::context_cmd::resolve_context_id_for_read(client, context).await?;
        client
            .data_artifacts()
            .list_shapes(context_id)
            .await
            .map_err(runtime::client_err_to_temper)?
    };
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

/// Parameters for [`schema_declare_remote`] — the CLI `schema declare` invocation.
pub struct SchemaDeclareParams<'a> {
    pub context: Option<&'a str>,
    pub cogmap: Option<&'a str>,
    pub kind: &'a str,
    pub kind_owner: Option<&'a str>,
    pub enforcement: EnforcementMode,
    pub content_flag: Option<&'a str>,
    pub act: temper_core::types::ActInput,
    pub format: OutputFormat,
}

/// `temper data-artifact schema declare [<context_ref> | --cogmap <ref>] --kind <k>` —
/// declare a shape for a family within a context or cognitive-map home (exactly one).
pub async fn schema_declare_remote(
    client: &temper_client::TemperClient,
    params: SchemaDeclareParams<'_>,
) -> crate::error::Result<()> {
    let SchemaDeclareParams {
        context,
        cogmap,
        kind,
        kind_owner,
        enforcement,
        content_flag,
        act,
        format: fmt,
    } = params;
    use std::io::IsTerminal;

    match (context, cogmap) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(crate::error::TemperError::Project(
                "schema declare needs exactly one home: a positional context ref, or --cogmap <ref>"
                    .to_string(),
            ));
        }
        _ => {}
    }

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
        kind_owner: kind_owner.map(parse_kind_owner).transpose()?,
        schema,
        enforcement,
        act,
    };

    // The home kind picks the route arm: a cogmap home declares through
    // POST /api/cognitive-maps/{id}/shapes, a context through POST /api/contexts/{id}/shapes.
    let shape: ShapeView = if let Some(cogmap_ref) = cogmap {
        let cogmap_id = parse_ref(cogmap_ref)?;
        client
            .data_artifacts()
            .declare_cogmap_shape(uuid::Uuid::from(cogmap_id), &request)
            .await
            .map_err(runtime::client_err_to_temper)?
    } else {
        let context = context.expect("validated: exactly one home");
        let context_id =
            crate::commands::context_cmd::resolve_context_id_for_read(client, context).await?;
        client
            .data_artifacts()
            .declare_shape(context_id, &request)
            .await
            .map_err(runtime::client_err_to_temper)?
    };
    let rendered = crate::format::render(&shape, fmt)?;
    output::plain(rendered);
    Ok(())
}

/// Parse the `--kind-owner` value: `kb_profiles:<uuid>` or `kb_teams:<uuid>` — the same
/// wire vocabulary [`KindOwnerInput`] carries, with the table named explicitly rather than
/// inferred from shape.
fn parse_kind_owner(s: &str) -> crate::error::Result<KindOwnerInput> {
    let (table, id) = s.split_once(':').ok_or_else(|| {
        crate::error::TemperError::Project(
            "--kind-owner expects `kb_profiles:<uuid>` or `kb_teams:<uuid>`".to_string(),
        )
    })?;
    let id = parse_ref(id)?;
    match table {
        "kb_profiles" => Ok(KindOwnerInput::Profile(uuid::Uuid::from(id))),
        "kb_teams" => Ok(KindOwnerInput::Team(uuid::Uuid::from(id))),
        other => Err(crate::error::TemperError::Project(format!(
            "unrecognized kind-owner table '{other}'; expected kb_profiles or kb_teams"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::data_artifact::{
        ArtifactCursor, ArtifactPageRefusal, MAX_ARTIFACT_PAGE_ROWS,
    };

    fn created() -> chrono::DateTime<chrono::Utc> {
        // Sub-microsecond precision: a position that does not round-trip exactly is a defect.
        chrono::DateTime::parse_from_rfc3339("2026-10-19T12:30:45.123456789Z")
            .expect("literal parses")
            .with_timezone(&chrono::Utc)
    }

    fn id() -> uuid::Uuid {
        uuid::Uuid::from_u128(0x0123_4567_89ab_cdef)
    }

    fn params() -> ListParams<'static> {
        ListParams {
            r#ref: "unused",
            kind: Some("measurement"),
            intent: Some("member"),
            include_folded: true,
            counts: false,
            limit: None,
            order: None,
            after_created: None,
            after_id: None,
            format: OutputFormat::Json,
        }
    }

    fn refusal_of(params: &ListParams<'_>) -> String {
        match list_ask(params) {
            Err(crate::error::TemperError::BadRequest(said)) => said,
            other => panic!("expected a BadRequest refusal, got {other:?}"),
        }
    }

    /// No `--limit` is the unchanged answer: every artifact. FAILS IF the command starts paging
    /// by default (a default `limit` would make this a `Page`).
    #[test]
    fn without_limit_the_command_asks_for_every_artifact() {
        let (api, ask) = list_ask(&params()).expect("no paging flags is valid");
        assert_eq!(ask, ListAsk::Every);
        assert_eq!(api.limit, None, "the drain is not given a page size here");
        assert_eq!(api.kind.as_deref(), Some("measurement"));
        assert_eq!(api.intent.as_deref(), Some("member"));
        assert_eq!(api.include_folded, Some(true));
    }

    /// `--limit` alone is the first page, oldest first. FAILS IF `--limit` is dropped, or an
    /// absent `--order` reads as `SortOrder::default()` (desc).
    #[test]
    fn limit_alone_asks_for_the_first_page_oldest_first() {
        let (_, ask) = list_ask(&ListParams {
            limit: Some(5),
            ..params()
        })
        .expect("valid");
        assert_eq!(
            ask,
            ListAsk::Page(ArtifactPageRequest {
                limit: 5,
                after: None,
                order: SortOrder::Asc,
            })
        );
    }

    /// The continuation flags carry the exact position and the direction. FAILS IF either half
    /// of the position or `--order` is lost on the way to the request.
    #[test]
    fn the_continuation_flags_carry_the_exact_position_and_direction() {
        let (api, ask) = list_ask(&ListParams {
            limit: Some(7),
            order: Some(SortOrder::Desc),
            after_created: Some(created()),
            after_id: Some(id()),
            ..params()
        })
        .expect("valid");
        assert_eq!(
            ask,
            ListAsk::Page(ArtifactPageRequest {
                limit: 7,
                after: Some(ArtifactCursor {
                    created: created(),
                    artifact_id: DataArtifactId::from(id()),
                }),
                order: SortOrder::Desc,
            })
        );
        assert_eq!(
            api.kind.as_deref(),
            Some("measurement"),
            "filters ride along"
        );
    }

    /// The refusals are `page_request`'s own, word for word. FAILS IF a combination is ignored
    /// instead of refused, or refused in other words than the server's.
    #[test]
    fn the_paging_refusals_are_the_servers_own() {
        let cases = [
            (
                ListParams {
                    limit: Some(0),
                    ..params()
                },
                ArtifactPageRefusal::LimitZero,
            ),
            (
                ListParams {
                    limit: Some(MAX_ARTIFACT_PAGE_ROWS + 1),
                    ..params()
                },
                ArtifactPageRefusal::LimitAboveCeiling {
                    limit: MAX_ARTIFACT_PAGE_ROWS + 1,
                },
            ),
            (
                ListParams {
                    limit: Some(2),
                    after_created: Some(created()),
                    ..params()
                },
                ArtifactPageRefusal::AfterIncomplete,
            ),
            (
                ListParams {
                    limit: Some(2),
                    after_id: Some(id()),
                    ..params()
                },
                ArtifactPageRefusal::AfterIncomplete,
            ),
            (
                ListParams {
                    after_created: Some(created()),
                    after_id: Some(id()),
                    ..params()
                },
                ArtifactPageRefusal::AfterWithoutLimit,
            ),
            (
                ListParams {
                    order: Some(SortOrder::Desc),
                    ..params()
                },
                ArtifactPageRefusal::OrderWithoutLimit,
            ),
        ];
        for (asked, refusal) in cases {
            assert_eq!(refusal_of(&asked), refusal.to_string());
        }
    }

    /// The ceiling itself is a page, not a refusal. FAILS IF the range is off by one.
    #[test]
    fn the_ceiling_is_a_page() {
        let (_, ask) = list_ask(&ListParams {
            limit: Some(MAX_ARTIFACT_PAGE_ROWS),
            ..params()
        })
        .expect("the ceiling is in range");
        assert!(matches!(ask, ListAsk::Page(r) if r.limit == MAX_ARTIFACT_PAGE_ROWS));
    }

    /// `--counts` is the unpaged counts read. FAILS IF counts is routed through the page path.
    #[test]
    fn counts_asks_for_counts() {
        let (api, ask) = list_ask(&ListParams {
            counts: true,
            ..params()
        })
        .expect("valid");
        assert_eq!(ask, ListAsk::Counts);
        assert_eq!(api.counts, Some(true));
        assert_eq!(api.limit, None);
    }
}

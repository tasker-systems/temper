//! Query tool — the MCP door onto the composition contract.
//!
//! One tool taking a whole `Composition` as its input, calling the same service-direct read path
//! the API handler calls (`query_read::prepare` → `run_composition`). The composition schema IS
//! the tool's input schema — every struct already carries `#[cfg_attr(feature = "mcp",
//! derive(schemars::JsonSchema))]`, so the vocabulary an agent needs to compose is the schema it
//! reads, not a second description of it.
//!
//! # The door, declared
//!
//! `subject-decides-the-door` (the door goal, `019fa618`) says knowledge subjects reach all three
//! doors. A composition's subject is resources and edges, so the MCP absence was a gap, not a
//! declared scope decision — and because nothing stated the absence, it also failed
//! `a-doors-scope-is-readable-before-it-is-called`. This tool IS the readable declaration: its
//! schema tells a caller what the door offers, before they knock.
//!
//! # Three decisions, settled with Pete `[2026-08-16]`
//!
//! 1. **One tool, not two.** No `query_check` sibling. On MCP both `query` and a hypothetical
//!    `query_check` are round trips — the CLI's `--check` is free because it touches no network,
//!    and that advantage does not transfer. Worse, `query_check` runs `validate_shape` only, so a
//!    cautious agent that checks first gets a false "clean" and then discovers capability refusals
//!    on the real call. `query`'s `prepare` already gates shape before embed, so a shape-invalid
//!    plan refuses at the shape gate and returns every fault (shape + capability) in one response.
//!    The refusal path IS the check.
//!
//! 2. **`trace: bool`, default `true`.** The trace is the composition's legibility — without it an
//!    intermediate stage is a black box with no answer at the end. The default matches the CLI so
//!    the doors do not diverge silently; the knob exists because MCP is not 1:1 with the CLI's
//!    jq-able trace use case, and an agent iterating on a failed composition may not want the trace
//!    on every retry. The schema shows the parameter, so the difference is declared, not discovered.
//!
//! 3. **No interim skill-file declaration.** The tool ships in one PR, and the tool IS the
//!    declaration. An interim prose line in the skill file has a shelf life of one PR and creates a
//!    "remember to remove this" burden.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use temper_core::types::ids::ProfileId;
use temper_core::types::query::Composition;
use temper_services::backend::query_read;
use temper_services::error::ApiError;

use crate::service::TemperMcpService;

/// MCP input for `run_query`: a composition plan and a trace flag.
///
/// The `plan` field IS the composition wire type — the same struct `/api/query` deserializes. Its
/// `JsonSchema` derive is already active under the `mcp` feature, so the tool's input schema is the
/// contract, not a restatement of it.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryInput {
    /// The composition plan: an ordered DAG of act invocations and set combinations, plus a
    /// declaration of which stages' rows come back. The schema for this object IS the composition
    /// contract — every stage, act, filter, and combinator is described there.
    pub plan: Composition,

    /// Whether to include the full stage trace in the response. Default `true`.
    ///
    /// The trace covers EVERY stage — including intermediates whose rows were not returned — and
    /// carries per-stage disposition, refusal, input counts, produced counts, and narrowing
    /// disclosures. It is the composition's legibility: without it, a multi-stage plan is a black
    /// box with an answer at the end. Set `false` to omit the trace and receive only the returned
    /// arms, useful when iterating on a plan and the intermediate legibility is not needed.
    #[serde(default = "default_trace")]
    pub trace: bool,
}

fn default_trace() -> bool {
    true
}

/// Run a composition query against the knowledge base.
///
/// Sends a composition plan to the server, which validates it (returning every refusal at once if
/// the plan is malformed), embeds any missing query vectors, compiles and executes the DAG, and
/// returns the requested stages' hydrated rows plus a trace covering every stage.
///
/// A refused plan returns an `invalid_params` error carrying every refusal — each names its stage
/// and its reason — so the plan can be repaired in one round trip, not one refusal per call.
pub async fn run_query(
    svc: &TemperMcpService,
    input: QueryInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let validated = query_read::prepare(input.plan)
        .await
        .map_err(|refusals| map_query_error("run_query", ApiError::PlanRefused { refusals }))?;

    let response =
        query_read::run_composition(&svc.api_state.pool, ProfileId::from(profile.id), &validated)
            .await
            .map_err(|e| map_query_error("run_query", e))?;

    let body = if input.trace {
        serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string())
    } else {
        let without_trace = serde_json::to_value(&response)
            .map(|mut v| {
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("trace");
                }
                v
            })
            .map(|v| serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|_| "{}".to_string());
        without_trace
    };

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        body,
    )]))
}

/// Map a query-path error onto an MCP error.
///
/// `PlanRefused` is the one error shape this door produces that an agent can act on: it carries
/// every static refusal, each naming its stage and reason. Rendering it as `invalid_params` with
/// the joined refusal details lets the agent repair the plan in one round trip — the same care
/// `contexts.rs::map_api_error` gives `BadRequest`, because `PlanRefused` IS a `BadRequest` variant
/// at the HTTP layer. Everything else stays opaque, matching the established pattern.
fn map_query_error(context: &str, err: ApiError) -> rmcp::ErrorData {
    match err {
        ApiError::PlanRefused { refusals } => {
            let details = refusals
                .iter()
                .map(|r| {
                    let stage = r
                        .stage
                        .as_ref()
                        .map(|s| format!("stage '{}': ", s.as_str()))
                        .unwrap_or_default();
                    format!("{}{:?} — {}", stage, r.reason, r.detail)
                })
                .collect::<Vec<_>>()
                .join("\n");
            rmcp::ErrorData::invalid_params(format!("{context}: plan refused — {details}"), None)
        }
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::query::PlanRefusal;
    use temper_core::types::query::RefusalReason;
    use temper_core::types::query::StageName;

    /// `PlanRefused` renders as `invalid_params` carrying every refusal, each with its stage and
    /// reason — the property that lets an agent repair a plan in one round trip.
    #[test]
    fn plan_refused_renders_every_refusal_as_invalid_params() {
        let refusals = vec![
            PlanRefusal {
                stage: Some(StageName::parse("hits").unwrap()),
                reason: RefusalReason::MissingIntention,
                detail: "find-exact needs a query".to_string(),
            },
            PlanRefusal {
                stage: Some(StageName::parse("wide").unwrap()),
                reason: RefusalReason::FilterNotApplicable,
                detail: "find-about-anywhere does not accept an edge filter".to_string(),
            },
        ];
        let err = map_query_error("run_query", ApiError::PlanRefused { refusals });
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);

        let msg = err.message.as_ref();
        assert!(msg.contains("stage 'hits'"), "names the first stage: {msg}");
        assert!(
            msg.contains("stage 'wide'"),
            "names the second stage: {msg}"
        );
        assert!(
            msg.contains("MissingIntention") || msg.contains("missing_intention"),
            "carries the reason: {msg}"
        );
        assert!(
            msg.contains("find-exact needs a query"),
            "carries the detail: {msg}"
        );
        assert!(
            msg.contains("does not accept an edge filter"),
            "carries both refusals, not just the first: {msg}"
        );
    }

    /// A composition-level refusal (no stage) still renders, without an empty "stage ''" prefix.
    #[test]
    fn a_composition_level_refusal_omits_the_stage_prefix() {
        let refusals = vec![PlanRefusal {
            stage: None,
            reason: RefusalReason::NoReturns,
            detail: "outcome.returns is empty".to_string(),
        }];
        let err = map_query_error("run_query", ApiError::PlanRefused { refusals });
        let msg = err.message.as_ref();
        assert!(!msg.contains("stage ''"), "no empty stage prefix: {msg}");
        assert!(msg.contains("NoReturns") || msg.contains("no_returns"));
        assert!(msg.contains("outcome.returns is empty"));
    }

    /// A non-refusal error stays opaque — the established pattern for everything an agent cannot
    /// act on. It renders as `internal_error`, not `invalid_params`, so an agent does not mistake a
    /// server fault for a repairable plan.
    #[test]
    fn a_non_refusal_error_stays_opaque() {
        let err = map_query_error("run_query", ApiError::Internal("db down".to_string()));
        assert_eq!(err.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        // The opaque path carries the context tag, not the refusal-rendering shape — an agent
        // reading this knows it is not a plan refusal, not which stage to repair.
        let msg = err.message.as_ref();
        assert!(!msg.contains("plan refused"), "not a refusal shape: {msg}");
    }
}

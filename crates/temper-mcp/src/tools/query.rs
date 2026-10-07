//! Query tool — the MCP door onto the composition contract.
//!
//! One tool taking a whole `Composition` as its input, forwarded to `POST /api/query`
//! through the network door (beat G3b) — the same route, shape-gate, embed, and
//! validate pipeline the HTTP door serves, on the caller's own bearer. The composition
//! schema IS the tool's input schema — every struct already carries
//! `#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]`, so the vocabulary an
//! agent needs to compose is the schema it reads, not a second description of it.
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
//!    on the real call. `query`'s `prepare` already gates on the full `validate` before embed
//!    `[was shape alone — 2026-08-28]`, so an invalid
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
use serde::{Deserialize, Deserializer};

use temper_client::error::ClientError;
use temper_core::types::query::Composition;

use crate::service::{AcrossAuth, TemperMcpService};

/// MCP input for `run_query`: a composition plan and a trace flag.
///
/// **The plan is declared as a composition and relayed unread.** Its schema is `Composition`'s own
/// (`schemars(with)`), so the tool's input schema is the contract, not a restatement of it, and an
/// agent composes against exactly what `/api/query` reads. But this door never deserializes it:
/// whether a plan is readable — its structure, its sizes, every name in it — is `/api/query`'s to
/// decide, once, for both doors. A plan the API cannot read comes back as its own
/// `UNREADABLE_PLAN` sentence, bounded there, rather than as this door's deserializer quoting the
/// caller's input back whole.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryInput {
    /// The composition plan: an ordered DAG of act invocations and set combinations, plus a
    /// declaration of which stages' rows come back. The schema for this object IS the composition
    /// contract — every stage, act, filter, and combinator is described there.
    #[schemars(with = "Composition")]
    pub plan: serde_json::Value,

    /// Whether to include the full stage trace in the response. Default `true`.
    ///
    /// The trace covers EVERY stage — including intermediates whose rows were not returned — and
    /// carries per-stage disposition, refusal, input counts, produced counts, and narrowing
    /// disclosures. It is the composition's legibility: without it, a multi-stage plan is a black
    /// box with an answer at the end. Set `false` to omit the trace and receive only the returned
    /// arms, useful when iterating on a plan and the intermediate legibility is not needed.
    #[serde(default = "default_trace", deserialize_with = "trace_flag")]
    pub trace: bool,
}

/// `trace` as a boolean, refused in a fixed sentence: serde's own `invalid type` message would
/// quote a non-boolean whole, and this field is read by this door, not relayed.
fn trace_flag<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Bool(b) => Ok(b),
        _ => Err(serde::de::Error::custom("`trace` must be a boolean")),
    }
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
    parts: &http::request::Parts,
    input: QueryInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    // The client is constructed BEFORE the act measures: an arrival with no bearer, or a
    // deployment missing its relay config, is refused by the constructor and never enters
    // the distribution — the direct binding's gate-first ordering excluded unauthenticated
    // arrivals the same way. Measuring before the send (not after) keeps
    // the property `CompositionShape` requires: the act is counted before the server
    // decides whether to answer it. The `mcp` door is measured separately from `http`
    // because `embeddings_supplied` is structurally zero here — this door cannot run the
    // model, which is why the server embeds on its behalf — so any bound on what the
    // server must embed binds this door alone, and its distribution is the one that
    // decides it. The API skips its own `door=http` event when the act arrives relayed,
    // so one act measures once, on the door it arrived on.
    let client = svc.relay_client(parts)?;
    // No shape measurement here: this door holds no composition, only the caller's JSON. The API
    // measures the plan once it reads it, labelled `mcp` by the relay's `RelayedSurface`.

    let response = client
        .query()
        .run(&input.plan)
        .await
        .across_auth(|e| map_query_error("run_query", e))?;

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

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(body),
    ]))
}

/// Map a query-path error onto an MCP error.
///
/// `PlanRefused` is the one error shape this door produces that an agent can act on: it carries
/// every static refusal, each naming its stage and reason. The 400 arrives under the wire code
/// `PLAN_REFUSED` and temper-client reconstructs the refusal list
/// (`ClientError::PlanRefused`), so the rendering is the direct binding's, arm for arm —
/// `invalid_params` with the joined refusal details lets the agent repair the plan in one round
/// trip, the same care `contexts.rs::map_api_error` gives `BadRequest`, because `PlanRefused` IS
/// a `BadRequest` variant at the HTTP layer. Everything else stays opaque, matching the
/// established pattern.
fn map_query_error(context: &str, err: ClientError) -> rmcp::ErrorData {
    match err {
        ClientError::PlanRefused { refusals } => {
            // `[added — 2026-08-28, found in review]` The HTTP door logs every refusal's reason in
            // `ApiError`'s `IntoResponse`; this door never reaches that impl, so an MCP refusal
            // emitted NOTHING and the agent-facing surface — the one carrying the most automated
            // traffic — was the one with no operator signal. Reasons only, never `detail`, which
            // quotes the caller's own composition back.
            tracing::warn!(
                context,
                refusal_count = refusals.len(),
                reasons = ?refusals.iter().map(|r| &r.reason).collect::<Vec<_>>(),
                "plan refused"
            );
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
        // The API could not read the plan this door relayed unread. A caller error, in the API's
        // own words (bounded there), so the agent can repair it.
        ClientError::UnreadablePlan { message } => rmcp::ErrorData::invalid_params(
            format!("{context}: plan could not be read — {message}"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The edge reads `trace` and nothing of `plan`: any plan passes its parse, so whether a
    /// plan is readable is the API's to say, and a non-boolean `trace` is refused in a fixed
    /// sentence rather than serde's, which would quote it back whole.
    #[test]
    fn the_edge_parses_no_plan_and_quotes_no_trace() {
        let input: QueryInput =
            serde_json::from_value(serde_json::json!({ "plan": "not a composition" }))
                .expect("any plan passes the edge");
        assert_eq!(input.plan, serde_json::json!("not a composition"));
        assert!(input.trace, "trace defaults to true");

        let huge = "z".repeat(100_000);
        let err =
            serde_json::from_value::<QueryInput>(serde_json::json!({ "plan": {}, "trace": huge }))
                .expect_err("a non-boolean trace is refused");
        let msg = err.to_string();
        assert!(
            msg.contains("`trace` must be a boolean") && msg.len() < 256,
            "a fixed sentence, not the caller's string: {} bytes",
            msg.len()
        );
    }

    /// An unreadable plan is a caller error in the API's own words, not an opaque fault.
    #[test]
    fn an_unreadable_plan_renders_as_invalid_params_with_the_apis_sentence() {
        let err = map_query_error(
            "run_query",
            ClientError::UnreadablePlan {
                message: "stages: invalid type".to_string(),
            },
        );
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("stages: invalid type"),
            "{}",
            err.message
        );
    }
    use temper_client::error::ClientError;
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
        let err = map_query_error("run_query", ClientError::PlanRefused { refusals });
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
        let err = map_query_error("run_query", ClientError::PlanRefused { refusals });
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
        let err = map_query_error(
            "run_query",
            ClientError::Server {
                status: 503,
                message: "db down".to_string(),
            },
        );
        assert_eq!(err.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        // The opaque path carries the context tag, not the refusal-rendering shape — an agent
        // reading this knows it is not a plan refusal, not which stage to repair.
        let msg = err.message.as_ref();
        assert!(!msg.contains("plan refused"), "not a refusal shape: {msg}");
    }
}

//! How a door that relays its body unread renders what the API refused.
//!
//! Two tools send the API a body they never read — `run_query` a plan, `ask_data_artifacts` an ask
//! — and the API answers both with the same two caller errors: `PLAN_REFUSED`, carrying every
//! refusal at once, and `UNREADABLE_PLAN`, its bounded account of a body it could not read.
//! temper-client types them as [`ClientError::PlanRefused`] and [`ClientError::UnreadablePlan`].
//! Both tools render them here, so the two doors cannot disagree about what a refusal looks like.

use temper_client::error::ClientError;
use temper_core::types::error_details::PlanRefusalDetails;

/// What the API refused: the noun the prose names, so an ask's caller is never told about a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// A composition, relayed by `run_query`.
    Plan,
    /// An artifact ask, relayed by `ask_data_artifacts`.
    Ask,
}

impl Refused {
    fn noun(self) -> &'static str {
        match self {
            Refused::Plan => "plan",
            Refused::Ask => "ask",
        }
    }
}

/// Map an error from a relayed, unread body onto an MCP error.
///
/// Two error shapes are ones an agent can act on. `UnreadablePlan` carries the server's bounded
/// account of what in the body it could not read. `PlanRefused` carries every refusal, each naming
/// its stage (a plan's) and its reason; `invalid_params` with the joined refusal details lets the
/// agent repair the body in one round trip. Everything else stays opaque, as `internal_error`.
///
/// **A refusal reaches the agent twice, for two readers.** The message is the prose line per
/// refusal, which a reader that only shows `message` still gets. The error's `data` slot carries
/// the same list as [`PlanRefusalDetails`] — the type `error.details` carries on the HTTP door —
/// so `reason` and `teaches` arrive as fields an agent reads without parsing English. Without the
/// `data`, a refusal's `teaches` would survive the API and the client and be dropped here, at the
/// last hop.
pub(crate) fn map_refusable_error(
    context: &str,
    refused: Refused,
    err: ClientError,
) -> rmcp::ErrorData {
    let noun = refused.noun();
    match err {
        ClientError::PlanRefused { refusals } => {
            // The HTTP door logs every refusal's reason in `ApiError`'s `IntoResponse`; this door
            // never reaches that impl, so without this an MCP refusal would emit nothing on the
            // agent-facing surface, which carries the most automated traffic. Reasons only, never
            // `detail`, which quotes the caller's own body back.
            let reasons: Vec<_> = refusals.iter().map(|r| &r.reason).collect();
            match refused {
                Refused::Plan => tracing::warn!(
                    context,
                    refusal_count = refusals.len(),
                    reasons = ?reasons,
                    "plan refused"
                ),
                Refused::Ask => tracing::warn!(
                    context,
                    refusal_count = refusals.len(),
                    reasons = ?reasons,
                    "ask refused"
                ),
            }
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
            // Typed, not `json!`: the same struct the HTTP door serializes into `error.details`,
            // so the doors cannot disagree about the shape of a refusal list.
            let data = serde_json::to_value(PlanRefusalDetails { refusals })
                .inspect_err(|error| {
                    tracing::error!(context, %error, "refusals did not serialize");
                })
                .ok();
            rmcp::ErrorData::invalid_params(format!("{context}: {noun} refused — {details}"), data)
        }
        // The API could not read the body this door relayed unread. A caller error, in the API's
        // own words (bounded there), so the agent can repair it.
        ClientError::UnreadablePlan { message } => rmcp::ErrorData::invalid_params(
            format!("{context}: {noun} could not be read — {message}"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::query::{PlanRefusal, RefusalReason};

    fn refusal() -> PlanRefusal {
        PlanRefusal {
            stage: None,
            reason: RefusalReason::PageLimitOutOfRange,
            detail: "`limit` must be at least 1".to_string(),
            teaches: None,
        }
    }

    /// **An ask's refusal and an unreadable ask name the ask, never a plan**, and the refusal
    /// carries its list as `data`. Bites: `Refused::Ask` mapped to `"plan"`, or `data` passed as
    /// `None`.
    #[test]
    fn an_ask_is_named_as_an_ask() {
        let err = map_refusable_error(
            "ask_data_artifacts",
            Refused::Ask,
            ClientError::PlanRefused {
                refusals: vec![refusal()],
            },
        );
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("ask refused"), "{}", err.message);
        assert!(!err.message.contains("plan"), "{}", err.message);
        let back: PlanRefusalDetails =
            serde_json::from_value(err.data.expect("the refusals ride data")).expect("typed");
        assert_eq!(back.refusals, vec![refusal()]);

        let unreadable = map_refusable_error(
            "ask_data_artifacts",
            Refused::Ask,
            ClientError::UnreadablePlan {
                message: "limit: invalid type".to_string(),
            },
        );
        assert_eq!(unreadable.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            unreadable.message.contains("ask could not be read")
                && unreadable.message.contains("limit: invalid type")
                && !unreadable.message.contains("plan"),
            "{}",
            unreadable.message
        );
    }
}

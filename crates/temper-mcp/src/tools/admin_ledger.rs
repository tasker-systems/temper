//! The admin ledger's MCP read tool — "who granted what, to whom, and when".
//!
//! A **service-direct read**, like the other read tools: the gate lives in
//! `admin_ledger_service`, which dispatches per act family rather than running a prelude, so
//! there is nothing for this layer to check and nothing it could usefully add.
//!
//! Deny is **404-shaped** on every surface (here: "not found"), never "forbidden". On this
//! surface "you may not read that" and "there is nothing there" are made deliberately
//! indistinguishable — a forbidden would confirm the ledger holds something about the subject,
//! which is itself the disclosure.

use rmcp::model::CallToolResult;

use temper_core::types::admin::AdminLedgerInput;
use temper_core::types::ids::ProfileId;
use temper_services::error::ApiError;
use temper_services::services::admin_ledger_service;

use crate::service::TemperMcpService;

/// Page size when the agent does not ask, and the ceiling when it asks for too much. Kept equal
/// to the HTTP handler's — a surface that pages differently is a surface that answers a different
/// question.
const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

fn map_err(e: ApiError) -> rmcp::ErrorData {
    match e {
        // The deny-split invariant, carried onto MCP: refusal is indistinguishable from absence.
        ApiError::NotFound => {
            rmcp::ErrorData::invalid_params("admin_ledger: nothing readable here".to_string(), None)
        }
        ApiError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => rmcp::ErrorData::internal_error(format!("admin_ledger: {other}"), None),
    }
}

/// Which axis the agent asked for. Resolved BEFORE any DB work so the refusal paths are pure and
/// therefore testable — this logic previously lived inline in `admin_ledger`, where it needed a
/// live `TemperMcpService` to reach and so had no test at all. An adversarial review demonstrated
/// the cost: deleting the "not both" arm compiled and passed every test in the repo.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Axis<'a> {
    /// The `<kind>:<uuid>` spelling, still unparsed — `admin_ledger_service` owns that grammar.
    Subject(&'a str),
    Actor(uuid::Uuid),
}

/// The two axes gate differently, so naming both is refused rather than resolved — picking one
/// would answer a question the agent did not ask, under a gate it did not expect.
pub(crate) fn resolve_axis(input: &AdminLedgerInput) -> Result<Axis<'_>, rmcp::ErrorData> {
    match (input.subject.as_deref(), input.actor.as_deref()) {
        (Some(_), Some(_)) => Err(rmcp::ErrorData::invalid_params(
            "pass either subject or actor, not both".to_string(),
            None,
        )),
        (None, None) => Err(rmcp::ErrorData::invalid_params(
            "pass either subject ('<kind>:<uuid>') or actor (a profile uuid)".to_string(),
            None,
        )),
        (Some(spec), None) => Ok(Axis::Subject(spec)),
        (None, Some(actor)) => Ok(Axis::Actor(uuid::Uuid::parse_str(actor).map_err(|e| {
            rmcp::ErrorData::invalid_params(format!("invalid actor id '{actor}': {e}"), None)
        })?)),
    }
}

/// Page bounds, clamped identically to the HTTP handler. Extracted for the same reason as
/// `resolve_axis`: a bound that is only asserted by a comment is not asserted.
pub(crate) fn page_bounds(input: &AdminLedgerInput) -> (i64, i64) {
    (
        input.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
        input.offset.unwrap_or(0).max(0),
    )
}

pub async fn admin_ledger(
    svc: &TemperMcpService,
    input: AdminLedgerInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let caller = ProfileId::from(profile.id);

    let (limit, offset) = page_bounds(&input);

    let entries = match resolve_axis(&input)? {
        Axis::Subject(spec) => admin_ledger_service::list_by_subject(
            &svc.api_state.pool,
            caller,
            admin_ledger_service::parse_subject_spec(spec).map_err(map_err)?,
            limit,
            offset,
        )
        .await
        .map_err(map_err)?,
        Axis::Actor(actor) => admin_ledger_service::list_by_actor(
            &svc.api_state.pool,
            caller,
            ProfileId::from(actor),
            limit,
            offset,
        )
        .await
        .map_err(map_err)?,
    };

    // Only read once the service has authorized above. The epoch is what stops an empty list from
    // lying: "nothing since T", never "nothing ever".
    let epoch = admin_ledger_service::ledger_epoch(&svc.api_state.pool)
        .await
        .map_err(map_err)?;

    // The same projection temper-api uses — typed, and shared so the surfaces cannot drift.
    let page = admin_ledger_service::to_wire_page(entries, epoch).map_err(map_err)?;
    let text = serde_json::to_string_pretty(&page).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(subject: Option<&str>, actor: Option<&str>) -> AdminLedgerInput {
        AdminLedgerInput {
            subject: subject.map(str::to_string),
            actor: actor.map(str::to_string),
            ..Default::default()
        }
    }

    /// The arm an adversarial review proved was unprotected: deleting it compiled and passed the
    /// whole repo. The axes gate differently, so resolving the ambiguity would answer a question
    /// the agent did not ask.
    #[test]
    fn naming_both_axes_is_refused() {
        let err = resolve_axis(&input(
            Some("kb_resources:0199c3f1-0000-7000-8000-000000000001"),
            Some("0199c3f1-0000-7000-8000-000000000002"),
        ))
        .expect_err("both axes must be refused, never resolved");
        assert!(
            format!("{err:?}").contains("not both"),
            "the refusal must say why: {err:?}"
        );
    }

    #[test]
    fn naming_no_axis_is_refused() {
        resolve_axis(&input(None, None)).expect_err("an axis-less request has no meaning");
    }

    #[test]
    fn each_axis_resolves_to_itself() {
        assert_eq!(
            resolve_axis(&input(Some("kb_resources:abc"), None)).expect("subject axis"),
            // Deliberately unparsed here: `admin_ledger_service` owns the `<kind>:<uuid>` grammar,
            // and a second parser in this layer would be a second grammar.
            Axis::Subject("kb_resources:abc")
        );

        let actor = uuid::Uuid::now_v7();
        assert_eq!(
            resolve_axis(&input(None, Some(&actor.to_string()))).expect("actor axis"),
            Axis::Actor(actor)
        );
    }

    #[test]
    fn an_unparseable_actor_is_a_bad_request_not_a_panic() {
        resolve_axis(&input(None, Some("not-a-uuid"))).expect_err("must reject, not unwrap");
    }

    /// The comment says these are "kept equal to the HTTP handler's". A comment is not a test.
    #[test]
    fn page_bounds_clamp_every_hostile_input() {
        let bounds = |limit, offset| {
            page_bounds(&AdminLedgerInput {
                limit,
                offset,
                ..Default::default()
            })
        };

        assert_eq!(bounds(None, None), (DEFAULT_LIMIT, 0), "defaults");
        assert_eq!(bounds(Some(0), None).0, 1, "zero is raised to one");
        assert_eq!(bounds(Some(-5), None).0, 1, "negative is raised to one");
        assert_eq!(bounds(Some(i64::MIN), None).0, 1, "i64::MIN is raised");
        assert_eq!(
            bounds(Some(10_000), None).0,
            MAX_LIMIT,
            "over-ask is capped"
        );
        assert_eq!(
            bounds(Some(i64::MAX), None).0,
            MAX_LIMIT,
            "i64::MAX is capped"
        );
        assert_eq!(bounds(None, Some(-1)).1, 0, "negative offset floors at 0");
        assert_eq!(bounds(None, Some(i64::MIN)).1, 0, "i64::MIN offset floors");
        assert_eq!(
            bounds(Some(25), Some(75)),
            (25, 75),
            "in-range passes through"
        );
    }

    /// Surface parity is claimed by a comment; assert it against the HTTP handler's real constants
    /// so a change to one side fails here rather than shipping two pagers.
    #[test]
    fn page_bounds_match_the_http_surface() {
        assert_eq!(DEFAULT_LIMIT, 50, "HTTP handler's DEFAULT_LIMIT");
        assert_eq!(MAX_LIMIT, 200, "HTTP handler's MAX_LIMIT");
    }
}

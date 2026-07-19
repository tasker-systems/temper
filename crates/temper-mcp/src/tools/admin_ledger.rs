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

pub async fn admin_ledger(
    svc: &TemperMcpService,
    input: AdminLedgerInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let caller = ProfileId::from(profile.id);

    let limit = input.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = input.offset.unwrap_or(0).max(0);

    // The two axes gate differently, so naming both is refused rather than resolved — picking one
    // would answer a question the agent did not ask, under a gate it did not expect.
    let entries = match (input.subject.as_deref(), input.actor.as_deref()) {
        (Some(_), Some(_)) => {
            return Err(rmcp::ErrorData::invalid_params(
                "pass either subject or actor, not both".to_string(),
                None,
            ))
        }
        (None, None) => {
            return Err(rmcp::ErrorData::invalid_params(
                "pass either subject ('<kind>:<uuid>') or actor (a profile uuid)".to_string(),
                None,
            ))
        }
        (Some(spec), None) => admin_ledger_service::list_by_subject(
            &svc.api_state.pool,
            caller,
            admin_ledger_service::parse_subject_spec(spec).map_err(map_err)?,
            limit,
            offset,
        )
        .await
        .map_err(map_err)?,
        (None, Some(actor)) => {
            let actor = uuid::Uuid::parse_str(actor).map_err(|e| {
                rmcp::ErrorData::invalid_params(format!("invalid actor id '{actor}': {e}"), None)
            })?;
            admin_ledger_service::list_by_actor(
                &svc.api_state.pool,
                caller,
                ProfileId::from(actor),
                limit,
                offset,
            )
            .await
            .map_err(map_err)?
        }
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
